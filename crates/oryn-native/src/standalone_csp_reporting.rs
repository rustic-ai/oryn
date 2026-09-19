//! Bounded document-owned report delivery for standalone native brokers.
use crate::{
    csp::{CspPolicySet, CspViolation},
    network::{NetworkRequest, SharedNetworkBroker, collect_operation_bounded_with_cancel},
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
static PENDING: AtomicUsize = AtomicUsize::new(0);
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1u64 << 61);
#[cfg(test)]
thread_local! {static FAIL_SUBMIT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
fn submit_report(job: impl FnOnce() + Send + 'static) -> Result<(), String> {
    #[cfg(test)]
    if FAIL_SUBMIT.with(|failure| failure.replace(false)) {
        return Err("injected report executor admission".into());
    }
    crate::executable::resource_io_executor::submit(job)
}

#[derive(Default)]
struct State {
    pending: usize,
    endpoints: BTreeMap<String, VecDeque<Instant>>,
    outcomes: VecDeque<String>,
}
pub(crate) struct Sender {
    broker: SharedNetworkBroker,
    top_level: String,
    cancel: AtomicBool,
    state: Mutex<State>,
}
pub(crate) struct Owner(Arc<Sender>);
impl Drop for Owner {
    fn drop(&mut self) {
        self.0.cancel.store(true, Ordering::Release);
    }
}
struct Permit(Arc<Sender>);
impl Drop for Permit {
    fn drop(&mut self) {
        PENDING.fetch_sub(1, Ordering::AcqRel);
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.pending = state.pending.saturating_sub(1);
    }
}
impl Sender {
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub(crate) fn new(broker: SharedNetworkBroker, top_level: String) -> (Owner, Arc<Self>) {
        let sender = Arc::new(Self {
            broker,
            top_level,
            cancel: AtomicBool::new(false),
            state: Mutex::new(State::default()),
        });
        (Owner(sender.clone()), sender)
    }
    fn outcome(&self, value: String) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.outcomes.len() == 128 {
            state.outcomes.pop_front();
        }
        state.outcomes.push_back(value);
    }
    pub(crate) fn queue(
        self: &Arc<Self>,
        policy: &CspPolicySet,
        violation: &CspViolation,
    ) -> Result<(), String> {
        if self.cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        let source_bytes = violation
            .original_policy
            .len()
            .saturating_add(violation.document_uri.len())
            .saturating_add(violation.source.len())
            .saturating_add(violation.source_file.len())
            .saturating_add(violation.blocked_uri.len())
            .saturating_add(violation.sample.len())
            .saturating_add(violation.referrer.len())
            .saturating_add(violation.effective_directive.len());
        if source_bytes > 32 * 1024 {
            self.outcome("size_limited".into());
            return Ok(());
        }
        for target in policy.report_targets(violation) {
            let (content_type, body) =
                crate::runtime::serialize_csp_report(target.format, violation);
            self.queue_payload(
                target.url,
                violation.document_uri.clone(),
                content_type.into(),
                body,
            )?;
        }
        Ok(())
    }
    pub(crate) fn queue_payload(
        self: &Arc<Self>,
        endpoint_url: String,
        document_url: String,
        content_type: String,
        body: Vec<u8>,
    ) -> Result<(), String> {
        if self.cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        if body.len() > 32 * 1024 {
            self.outcome("size_limited".into());
            return Ok(());
        }
        let Ok(endpoint) = url::Url::parse(&endpoint_url) else {
            self.outcome("invalid_endpoint".into());
            return Ok(());
        };
        if !matches!(endpoint.scheme(), "http" | "https")
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
        {
            self.outcome("invalid_endpoint".into());
            return Ok(());
        }
        let now = Instant::now();
        {
            let mut state = match self.state.lock() {
                Ok(state) => state,
                Err(error) => {
                    // Preserve poison: never admit more IO against uncertain
                    // accounting. Delivery failure does not undo a CSP event.
                    drop(error.into_inner());
                    self.outcome("queue_poisoned".into());
                    return Ok(());
                }
            };
            if state.pending >= 16 {
                drop(state);
                self.outcome("queue_limited".into());
                return Ok(());
            }
            state.endpoints.retain(|_, times| {
                while times
                    .front()
                    .is_some_and(|time| now.duration_since(*time) >= Duration::from_secs(60))
                {
                    times.pop_front();
                }
                !times.is_empty()
            });
            if !state.endpoints.contains_key(&endpoint_url) && state.endpoints.len() >= 128 {
                drop(state);
                self.outcome("endpoint_limit".into());
                return Ok(());
            }
            let times = state.endpoints.entry(endpoint_url.clone()).or_default();
            if times.len() >= 10 {
                drop(state);
                self.outcome("rate_limited".into());
                return Ok(());
            }
            if PENDING
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    (count < 128).then_some(count + 1)
                })
                .is_err()
            {
                drop(state);
                self.outcome("process_queue_limited".into());
                return Ok(());
            }
            times.push_back(now);
            state.pending += 1;
        }
        let permit = Permit(self.clone());
        if body.len() > 32 * 1024 {
            self.outcome("size_limited".into());
            drop(permit);
            return Ok(());
        }
        let request_id =
            match NEXT_REQUEST
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
            {
                Ok(id) => id,
                Err(_) => {
                    self.outcome("identity_exhausted".into());
                    return Ok(());
                }
            };
        let request = NetworkRequest {
            request_id,
            method: "POST".into(),
            url: endpoint_url,
            headers: vec![
                ("content-type".into(), content_type.into()),
                ("cache-control".into(), "no-store".into()),
            ],
            body,
            credentials: false,
            security: crate::cookies::CookieRequestContext {
                fetch_metadata: crate::fetch_metadata::Metadata::resource(
                    "no-cors", "report", false,
                ),
                dns_timing: None,
                credentials: crate::cookies::CookieCredentials::Omit,
                kind: crate::cookies::CookieRequestKind::Other,
                initiator_url: Some(document_url),
                top_level_url: Some(self.top_level.clone()),
                ..Default::default()
            },
            resolved_addrs: Vec::new(),
        };
        let sender = self.clone();
        let deadline = now + Duration::from_secs(10);
        let submitted = submit_report(move || {
            let _permit = permit;
            let result = if sender.cancel.load(Ordering::Acquire) {
                "cancelled".into()
            } else if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                match collect_operation_bounded_with_cancel(
                    &sender.broker,
                    request,
                    remaining,
                    64 * 1024,
                    &sender.cancel,
                ) {
                    Ok(response) if (200..300).contains(&response.status) => "delivered".into(),
                    Ok(response) => format!("http_{}", response.status),
                    Err(error) => format!("failed:{error}"),
                }
            } else {
                "timeout".into()
            };
            sender.outcome(result);
        });
        // Report delivery is best effort, like queue/rate limiting above.
        // Once the violation event is admitted, executor saturation must
        // not turn its policy decision into a retry of that same event.
        if let Err(error) = submitted {
            self.outcome(format!("queue_failed:{error}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{NetworkBroker, NetworkError, NetworkResponse};
    struct Reply(Arc<Mutex<Vec<NetworkRequest>>>);
    impl NetworkBroker for Reply {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let request_id = request.request_id;
            self.0.lock().unwrap().push(request);
            Ok(NetworkResponse {
                timing: None,
                request_id,
                status: 204,
                headers: Vec::new(),
                body: Vec::new(),
            })
        }
    }
    fn violation(policy: &CspPolicySet) -> CspViolation {
        policy
            .authorize(&crate::csp::CspRequest {
                destination: crate::csp::CspDestination::Script,
                url: "https://source.test/blocked.js".into(),
                nonce: None,
                inline_source: None,
                parser_inserted: true,
                initiated_by_trusted_script: false,
            })
            .violations
            .remove(0)
    }
    #[test]
    fn coep_report_only_endpoint_uses_native_bounded_credentialless_sender() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (owner, sender) = Sender::new(
            SharedNetworkBroker::new(Reply(requests.clone())),
            "https://top.test/".into(),
        );
        let policy = crate::coep_reporting::Policy::new(
            "https://source.test/document",
            &[
                (
                    "Cross-Origin-Embedder-Policy-Report-Only".into(),
                    "require-corp; report-to=\"coep\"".into(),
                ),
                ("Reporting-Endpoints".into(), "coep=\"/coep-report\"".into()),
            ],
        );
        let target = url::Url::parse("https://foreign.test/image.png").unwrap();
        let report = policy
            .response(
                "https://source.test",
                false,
                &target,
                false,
                true,
                &[],
                "image",
            )
            .remove(0);
        sender
            .queue_payload(
                report.endpoint,
                report.document_url,
                "application/reports+json".into(),
                report.body,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while sender.state.lock().unwrap().pending != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://source.test/coep-report");
        assert!(!requests[0].credentials);
        assert_eq!(
            requests[0].security.credentials,
            crate::cookies::CookieCredentials::Omit
        );
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body[0]["type"], "coep");
        assert_eq!(body[0]["body"]["disposition"], "reporting");
        drop(owner);
    }

    #[test]
    fn executor_rejection_records_outcome_and_releases_report_permit() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (_owner, sender) = Sender::new(
            SharedNetworkBroker::new(Reply(requests.clone())),
            "https://top.test/".into(),
        );
        let policy = CspPolicySet::from_headers(
            "https://source.test/",
            &[(
                "Content-Security-Policy".into(),
                "script-src 'none'; report-uri /report".into(),
            )],
        );
        FAIL_SUBMIT.with(|failure| failure.set(true));
        sender.queue(&policy, &violation(&policy)).unwrap();
        let state = sender.state.lock().unwrap();
        assert_eq!(state.pending, 0);
        assert!(
            state
                .outcomes
                .iter()
                .any(|outcome| outcome == "queue_failed:injected report executor admission")
        );
        assert!(requests.lock().unwrap().is_empty());
    }
    #[test]
    fn poisoned_report_accounting_does_not_retry_policy_or_admit_io() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (_owner, sender) = Sender::new(
            SharedNetworkBroker::new(Reply(requests.clone())),
            "https://top.test/".into(),
        );
        // Model an already admitted job whose accounting lock was poisoned.
        PENDING.fetch_add(1, Ordering::AcqRel);
        sender.state.lock().unwrap().pending = 1;
        let permit = Permit(sender.clone());
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _state = sender.state.lock().unwrap();
            panic!("injected report-state panic");
        }));
        assert!(poisoned.is_err());
        drop(permit);
        let policy = CspPolicySet::from_headers(
            "https://source.test/",
            &[(
                "Content-Security-Policy".into(),
                "script-src 'none'; report-uri /report /second-report".into(),
            )],
        );
        sender.queue(&policy, &violation(&policy)).unwrap();
        assert!(sender.state.is_poisoned());
        let state = sender
            .state
            .lock()
            .err()
            .expect("poison preserved")
            .into_inner();
        assert_eq!(state.pending, 0);
        assert_eq!(
            state
                .outcomes
                .iter()
                .filter(|outcome| outcome.as_str() == "queue_poisoned")
                .count(),
            2
        );
        assert!(requests.lock().unwrap().is_empty());
    }

    #[test]
    fn malformed_report_endpoint_is_a_delivery_outcome() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (_owner, sender) = Sender::new(
            SharedNetworkBroker::new(Reply(requests.clone())),
            "https://top.test/".into(),
        );
        sender
            .queue_payload(
                "not a URL".into(),
                "https://source.test/".into(),
                "application/reports+json".into(),
                vec![],
            )
            .unwrap();
        assert_eq!(
            sender
                .state
                .lock()
                .unwrap()
                .outcomes
                .back()
                .map(String::as_str),
            Some("invalid_endpoint")
        );
        assert!(requests.lock().unwrap().is_empty());
    }

    #[test]
    fn report_delivery_uses_captured_endpoint_and_never_includes_credentials() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let broker = SharedNetworkBroker::new(Reply(requests.clone()));
        let (owner, sender) = Sender::new(broker, "https://top.test/".into());
        let mut policy = CspPolicySet::from_headers(
            "https://source.test/",
            &[(
                "Content-Security-Policy".into(),
                "script-src 'none'; report-uri /original-report".into(),
            )],
        );
        let violation = violation(&policy);
        sender.queue(&policy, &violation).unwrap();
        policy.install_meta("script-src 'none'; report-uri /later-report");
        let deadline = Instant::now() + Duration::from_secs(2);
        while sender.state.lock().unwrap().pending != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(sender.state.lock().unwrap().pending, 0);
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://source.test/original-report");
        assert_eq!(requests[0].method, "POST");
        assert!(!requests[0].credentials);
        assert_eq!(
            requests[0].security.credentials,
            crate::cookies::CookieCredentials::Omit
        );
        assert_eq!(
            requests[0].security.top_level_url.as_deref(),
            Some("https://top.test/")
        );
        assert!(requests[0].body.len() <= 32 * 1024);
        drop(owner);
    }
    #[test]
    fn retiring_owner_cancels_active_response_and_releases_report_admission() {
        struct Pending {
            started: Arc<AtomicBool>,
            cancelled: Arc<AtomicBool>,
            stream: Mutex<Option<tokio::sync::mpsc::Sender<crate::network::NetworkEvent>>>,
        }
        impl NetworkBroker for Pending {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                panic!("report must use native operation")
            }
            fn start_operation(
                &self,
                request: NetworkRequest,
            ) -> Result<crate::network::NetworkOperation, NetworkError> {
                let (sender, events) = tokio::sync::mpsc::channel(1);
                *self.stream.lock().unwrap() = Some(sender);
                self.started.store(true, Ordering::Release);
                Ok(crate::network::NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id: request.request_id,
                    events,
                    demand: None,
                })
            }
            fn cancel_operation(&self, _: u64) -> Result<(), NetworkError> {
                self.cancelled.store(true, Ordering::Release);
                self.stream.lock().unwrap().take();
                Ok(())
            }
        }
        let started = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        let (owner, sender) = Sender::new(
            SharedNetworkBroker::new(Pending {
                started: started.clone(),
                cancelled: cancelled.clone(),
                stream: Mutex::new(None),
            }),
            "https://top.test/".into(),
        );
        let policy = CspPolicySet::from_headers(
            "https://source.test/",
            &[(
                "Content-Security-Policy".into(),
                "script-src 'none'; report-uri /report".into(),
            )],
        );
        sender.queue(&policy, &violation(&policy)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !started.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(started.load(Ordering::Acquire));
        drop(owner);
        while sender.state.lock().unwrap().pending != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(sender.state.lock().unwrap().pending, 0);
        assert!(cancelled.load(Ordering::Acquire));
    }

    #[test]
    fn retired_owner_rejects_new_report_io_and_queue_keeps_bounded_outcomes() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (owner, sender) = Sender::new(
            SharedNetworkBroker::new(Reply(requests.clone())),
            "https://top.test/".into(),
        );
        let policy = CspPolicySet::from_headers(
            "https://source.test/",
            &[(
                "Content-Security-Policy".into(),
                "script-src 'none'; report-uri /report".into(),
            )],
        );
        drop(owner);
        sender.queue(&policy, &violation(&policy)).unwrap();
        assert_eq!(sender.state.lock().unwrap().pending, 0);
        assert!(requests.lock().unwrap().is_empty());
        for _ in 0..1024 {
            sender.outcome("limited".into());
        }
        assert_eq!(sender.state.lock().unwrap().outcomes.len(), 128);
    }
}
