//! Capacity is reserved before requesting a parent script token. Dropping a
//! token or unreceived network-result owner only queues cancellation; fixed IO lanes retain it until ACK.
#[cfg(any(test, feature = "v8-host"))]
use super::ContextServiceResponse;
use super::{ContextServiceRequest, SharedContextClient};
use std::{
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

struct State {
    activity: Option<super::ResourceJob>,
    request: Option<ContextServiceRequest>,
    #[cfg(any(test, feature = "v8-host"))]
    target_client: Option<SharedContextClient>,
    abandoned: bool,
    #[cfg(any(test, feature = "v8-host"))]
    running: bool,
    done: bool,
    #[cfg(any(test, feature = "v8-host"))]
    retry: Instant,
}
struct Job {
    client: SharedContextClient,
    page_id: oryn_common::v3::PageId,
    state: Mutex<State>,
}
struct Queue {
    jobs: Vec<Arc<Job>>,
    #[cfg(any(test, feature = "v8-host"))]
    stopping: bool,
}
struct Executor {
    queue: Mutex<Queue>,
    changed: Condvar,
    #[cfg(any(test, feature = "v8-host"))]
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}
static EXECUTOR: OnceLock<Result<Arc<Executor>, String>> = OnceLock::new();

#[cfg(any(test, feature = "v8-host"))]
fn executor() -> Result<&'static Arc<Executor>, String> {
    EXECUTOR
        .get_or_init(|| {
            let pool = Arc::new(Executor {
                queue: Mutex::new(Queue {
                    jobs: Vec::new(),
                    stopping: false,
                }),
                changed: Condvar::new(),
                threads: Mutex::new(Vec::new()),
            });
            for index in 0..4 {
                let worker = pool.clone();
                match std::thread::Builder::new()
                    .name(format!("oryn-script-cleanup-{index}"))
                    .spawn(move || worker.run())
                {
                    Ok(thread) => pool
                        .threads
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(thread),
                    Err(error) => {
                        pool.queue
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .stopping = true;
                        pool.changed.notify_all();
                        let threads = std::mem::take(
                            &mut *pool.threads.lock().unwrap_or_else(|e| e.into_inner()),
                        );
                        for thread in threads {
                            let _ = thread.join();
                        }
                        return Err(error.to_string());
                    }
                }
            }
            Ok(pool)
        })
        .as_ref()
        .map_err(Clone::clone)
}

impl Executor {
    #[cfg(any(test, feature = "v8-host"))]
    fn run(&self) {
        loop {
            let job = {
                let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if queue.stopping {
                        return;
                    }
                    let next = queue.jobs.iter().find_map(|job| {
                        let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
                        if state.abandoned
                            && !state.running
                            && !state.done
                            && state.retry <= Instant::now()
                        {
                            state.running = true;
                            Some(job.clone())
                        } else {
                            None
                        }
                    });
                    if let Some(job) = next {
                        break job;
                    }
                    queue = self
                        .changed
                        .wait_timeout(queue, Duration::from_millis(100))
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            };
            let request = job
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .request
                .clone();
            let acknowledged = request.is_none() || std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let request=request.unwrap();
                let kind=match &request{ContextServiceRequest::RollbackAuxiliaryRoot{..}=>12,ContextServiceRequest::DiscardNavigationNetworkSource{..}=>11,ContextServiceRequest::DiscardNavigationResponsePolicy{..}=>10,ContextServiceRequest::ReleaseOrdinaryKeepalive{..}=>9,ContextServiceRequest::ReleaseDeferredContainer{..}=>8,ContextServiceRequest::CancelBlankPrerender{..}|ContextServiceRequest::CancelPrerender{..}=>7,ContextServiceRequest::FinishHyperlinkAudit{..}=>6,ContextServiceRequest::ParserEncodingPolicy{action:super::ParserEncodingPolicyAction::Release,..}=>5,ContextServiceRequest::ReleaseNetworkResult{..}|ContextServiceRequest::ReleaseFrameResponseStream{..}=>1,ContextServiceRequest::HttpRequestAbort{..}=>2,ContextServiceRequest::DiscardPreparedLocalNavigation{..}=>3,ContextServiceRequest::NavigationEvent{event:crate::navigation_control::NavigationEvent::Stopped(_),..}=>4,_=>0};
                let client=job.state.lock().unwrap_or_else(|e|e.into_inner()).target_client.clone().unwrap_or_else(||job.client.clone());
                matches!((kind,client.request(request)),(12,Ok(ContextServiceResponse::PageClosed)|Err(crate::runtime::RuntimeError::Closed))|(11,Ok(ContextServiceResponse::NavigationNetworkSourceDiscarded)|Err(crate::runtime::RuntimeError::Closed))|(10,Ok(ContextServiceResponse::NavigationResponsePolicyDiscarded)|Err(crate::runtime::RuntimeError::Closed))|(9,Ok(ContextServiceResponse::OrdinaryKeepaliveReleased)|Err(crate::runtime::RuntimeError::Closed))|(8,Ok(ContextServiceResponse::DeferredFetchResult{result:Ok(())})|Err(crate::runtime::RuntimeError::Closed))|(7,Ok(ContextServiceResponse::PrerenderCancelled))|(6,Ok(ContextServiceResponse::HyperlinkAuditFinished)|Err(crate::runtime::RuntimeError::Closed))|(5,Ok(ContextServiceResponse::DocumentSecurityInstalled))|(0,Ok(ContextServiceResponse::ScriptLoadCancelled))|(1,Ok(ContextServiceResponse::NetworkResultReleased))|(2,Ok(ContextServiceResponse::HttpRequestAborted))|(3,Ok(ContextServiceResponse::LocalNavigationDiscarded))|(4,Ok(ContextServiceResponse::NavigationControl{result:Ok(())|Err(crate::navigation_control::NavigationStop::Superseded|crate::navigation_control::NavigationStop::Retired)})))
            })).unwrap_or(false);
            {
                let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
                state.running = false;
                state.done = acknowledged;
                if acknowledged {
                    state.activity.take();
                }
                state.retry = Instant::now() + Duration::from_millis(100);
            }
            if acknowledged {
                self.queue
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .jobs
                    .retain(|entry| !Arc::ptr_eq(entry, &job));
            }
            self.changed.notify_all();
        }
    }
}

pub(super) struct Ticket {
    job: Arc<Job>,
}
impl Ticket {
    #[cfg(all(test, feature = "v8-host"))]
    pub(super) fn status(&self) -> (bool, bool) {
        let state = self
            .job
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (state.abandoned, state.done)
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(super) fn reserve(
        client: SharedContextClient,
        page_id: oryn_common::v3::PageId,
    ) -> Result<Self, String> {
        let pool = executor()?;
        let mut queue = pool.queue.lock().unwrap_or_else(|e| e.into_inner());
        if queue.jobs.len() >= 1024 {
            return Err("script cancellation custody capacity exceeded".into());
        }
        let job = Arc::new(Job {
            client,
            page_id,
            state: Mutex::new(State {
                activity: None,
                request: None,
                target_client: None,
                abandoned: false,
                running: false,
                done: false,
                retry: Instant::now(),
            }),
        });
        queue.jobs.push(job.clone());
        Ok(Self { job })
    }

    #[cfg(feature = "v8-host")]
    pub(super) fn bind_auxiliary(
        &self,
        client: SharedContextClient,
        page_id: oryn_common::v3::PageId,
        expected_generation: Option<u64>,
    ) {
        let mut state = self.job.state.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            !state.abandoned
                && !state.done
                && (state.request.is_none()
                    || matches!(
                        state.request,
                        Some(ContextServiceRequest::RollbackAuxiliaryRoot {
                            expected_generation: None,
                            ..
                        })
                    )),
            "one auxiliary identity per cleanup reservation"
        );
        state.target_client = Some(client);
        state.request = Some(ContextServiceRequest::RollbackAuxiliaryRoot {
            page_id,
            expected_generation,
        });
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(super) fn track_network_activity(&self) -> Result<(), String> {
        let activity = self
            .job
            .client
            .track_resource_job()
            .map_err(|error| error.to_string())?;
        let mut state = self
            .job
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert!(
            state.activity.is_none() && !state.done && !state.abandoned,
            "one activity owner per live cleanup ticket"
        );
        state.activity = Some(activity);
        Ok(())
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(super) fn attach_request(&self, request: ContextServiceRequest) {
        assert!(matches!(
            &request,
            ContextServiceRequest::ReleaseOrdinaryKeepalive { .. }
                | ContextServiceRequest::ReleaseDeferredContainer { .. }
                | ContextServiceRequest::CancelBlankPrerender { .. }
                | ContextServiceRequest::CancelPrerender { .. }
                | ContextServiceRequest::FinishHyperlinkAudit { .. }
                | ContextServiceRequest::ParserEncodingPolicy {
                    action: super::ParserEncodingPolicyAction::Release,
                    ..
                }
                | ContextServiceRequest::ReleaseNetworkResult { .. }
                | ContextServiceRequest::ReleaseFrameResponseStream { .. }
                | ContextServiceRequest::HttpRequestAbort { .. }
                | ContextServiceRequest::DiscardPreparedLocalNavigation { .. }
                | ContextServiceRequest::DiscardNavigationNetworkSource { .. }
                | ContextServiceRequest::DiscardNavigationResponsePolicy { .. }
                | ContextServiceRequest::NavigationEvent {
                    event: crate::navigation_control::NavigationEvent::Stopped(_),
                    ..
                }
        ));
        let mut state = self.job.state.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            state.request.is_none() && !state.abandoned,
            "one response per native cleanup owner"
        );
        state.request = Some(request);
    }
    #[cfg(feature = "v8-host")]
    pub(super) fn accept_hyperlink_audit(&self) {
        let mut state = self
            .job
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert!(!state.abandoned && !state.done, "live audit capability");
        match state.request.as_mut() {
            Some(ContextServiceRequest::FinishHyperlinkAudit { commit, .. }) => *commit = true,
            _ => panic!("audit capability completion owner"),
        }
    }
    pub(super) fn cancel(&self) {
        let mut state = self.job.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.done {
            return;
        }
        state.abandoned = true;
        drop(state);
        if let Some(Ok(pool)) = EXECUTOR.get() {
            pool.changed.notify_one();
        }
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(super) fn finish_if_not_cancelled(&self) {
        let mut state = self.job.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.abandoned {
            return;
        }
        state.done = true;
        state.activity.take();
        drop(state);
        if let Some(Ok(pool)) = EXECUTOR.get() {
            pool.queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .jobs
                .retain(|job| !Arc::ptr_eq(job, &self.job));
            pool.changed.notify_all();
        }
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(super) fn attach(&self, token: u64) {
        let mut state = self.job.state.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            state.request.is_none() && !state.abandoned,
            "one token per native cleanup owner"
        );
        state.request = Some(ContextServiceRequest::CancelScriptLoad {
            page_id: self.job.page_id,
            token,
        });
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// A buffered script response proves the parent has released its claimed
/// request. Retained prepared-source handles must not keep network activity.
pub(super) fn finish_script(
    client: &SharedContextClient,
    page: oryn_common::v3::PageId,
    token: u64,
) {
    settle_script(client, page, token, true);
}
pub(super) fn cancel_script(
    client: &SharedContextClient,
    page: oryn_common::v3::PageId,
    token: u64,
) {
    settle_script(client, page, token, false);
}
fn settle_script(
    client: &SharedContextClient,
    page: oryn_common::v3::PageId,
    token: u64,
    completed: bool,
) {
    let Some(Ok(pool)) = EXECUTOR.get() else {
        return;
    };
    let mut queue = pool.queue.lock().unwrap_or_else(|error| error.into_inner());
    queue.jobs.retain(|job|{
        if !Arc::ptr_eq(&job.client.0,&client.0)||job.client.1!=client.1||job.page_id!=page{return true;}
        let mut state=job.state.lock().unwrap_or_else(|error|error.into_inner());
        if !matches!(&state.request,Some(ContextServiceRequest::CancelScriptLoad{token:owned,..}) if *owned==token)||state.abandoned{return true;}
        if completed {
            state.done=true;
            state.activity.take();
            false
        }else{
            state.abandoned=true;
            true
        }
    });
    pool.changed.notify_all();
}

/// Terminal client teardown releases browsing-context leases even while their
/// V8 lifetime objects remain allocated. Ordinary drain never revokes them.
pub(super) fn release_deferred_containers(client: &SharedContextClient) {
    if let Some(Ok(pool)) = EXECUTOR.get() {
        let queue = pool.queue.lock().unwrap_or_else(|error| error.into_inner());
        for job in &queue.jobs {
            if Arc::ptr_eq(&job.client.0, &client.0) && job.client.1 == client.1 {
                let mut state = job.state.lock().unwrap_or_else(|error| error.into_inner());
                if matches!(
                    &state.request,
                    Some(ContextServiceRequest::ReleaseDeferredContainer { .. })
                ) {
                    state.abandoned = true;
                }
            }
        }
        pool.changed.notify_all();
    }
}

pub(super) fn drain(client: &SharedContextClient, budget: Duration) -> Result<(), String> {
    let Some(Ok(pool)) = EXECUTOR.get() else {
        return Ok(());
    };
    let deadline = Instant::now()
        .checked_add(budget)
        .unwrap_or_else(Instant::now);
    let mut queue = pool.queue.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        // An admission RPC can still own a ticket without a token. A teardown
        // barrier must include those tickets and live token owners, not only
        // cancellations that have already reached their Drop implementation.
        let pending = queue
            .jobs
            .iter()
            .any(|job| Arc::ptr_eq(&job.client.0, &client.0) && job.client.1 == client.1);
        if !pending {
            return Ok(());
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err("script cancellation acknowledgements remain pending".into());
        };
        queue = pool
            .changed
            .wait_timeout(queue, remaining.min(Duration::from_millis(100)))
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_service::{ContextTransport, IpcContextClient, serve_context_transport};
    use std::sync::mpsc;

    #[test]
    fn navigation_source_cleanup_is_prearmed_and_acknowledged_after_drop() {
        struct Service {
            sent: mpsc::Sender<String>,
        }
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                let ContextServiceRequest::DiscardNavigationNetworkSource { page_id, nonce } =
                    request
                else {
                    panic!("unexpected cleanup request");
                };
                assert_eq!(page_id, oryn_common::v3::PageId(1));
                self.sent.send(nonce).unwrap();
                Ok(ContextServiceResponse::NavigationNetworkSourceDiscarded)
            }
        }
        let (sent, received) = mpsc::channel();
        let client = SharedContextClient(
            Arc::new(Service { sent }),
            None,
            Default::default(),
            Default::default(),
            #[cfg(feature = "v8-host")]
            None,
        );
        #[cfg(feature = "v8-host")]
        let (owner, nonce) = {
            let lease = client
                .reserve_navigation_source(oryn_common::v3::PageId(1))
                .unwrap();
            let nonce = lease.nonce.clone();
            (lease, nonce)
        };
        #[cfg(not(feature = "v8-host"))]
        let (owner, nonce) = {
            let ticket = Ticket::reserve(client.clone(), oryn_common::v3::PageId(1)).unwrap();
            let nonce = "a".repeat(64);
            ticket.attach_request(ContextServiceRequest::DiscardNavigationNetworkSource {
                page_id: oryn_common::v3::PageId(1),
                nonce: nonce.clone(),
            });
            (ticket, nonce)
        };
        assert!(received.try_recv().is_err());
        assert!(drain(&client, Duration::ZERO).is_err());
        drop(owner);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(5)).unwrap(),
            nonce
        );
        drain(&client, Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn terminal_client_teardown_releases_live_frame_lease_before_drain() {
        struct Service;
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                assert!(matches!(
                    request,
                    ContextServiceRequest::ReleaseDeferredContainer { .. }
                ));
                Ok(ContextServiceResponse::DeferredFetchResult { result: Ok(()) })
            }
        }
        let client = SharedContextClient(
            Arc::new(Service),
            None,
            Default::default(),
            Default::default(),
            #[cfg(feature = "v8-host")]
            None,
        );
        let lease = client
            .reserve_deferred_container_cleanup(oryn_common::v3::PageId(1), "b".repeat(64))
            .unwrap();
        assert!(client.drain_script_cleanup(Duration::ZERO).is_err());
        client
            .drain_resource_teardown(Duration::from_secs(2))
            .unwrap();
        drop(lease);
        client.drain_script_cleanup(Duration::ZERO).unwrap();
    }
    #[test]
    fn deferred_container_release_retains_custody_until_typed_ack() {
        struct Service {
            attempt: Mutex<u32>,
            entered: std::sync::mpsc::Sender<u32>,
            release: Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                assert!(matches!(
                    request,
                    ContextServiceRequest::ReleaseDeferredContainer { .. }
                ));
                let attempt = {
                    let mut attempt = self.attempt.lock().unwrap();
                    *attempt += 1;
                    *attempt
                };
                self.entered.send(attempt).unwrap();
                if attempt == 1 {
                    return Ok(ContextServiceResponse::ScriptLoadCancelled);
                }
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
                Ok(ContextServiceResponse::DeferredFetchResult { result: Ok(()) })
            }
        }
        let (entered, observed) = std::sync::mpsc::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let client = SharedContextClient(
            Arc::new(Service {
                attempt: Mutex::new(0),
                entered,
                release: Mutex::new(gate),
            }),
            None,
            Default::default(),
            Default::default(),
            #[cfg(feature = "v8-host")]
            None,
        );
        let lease = client
            .reserve_deferred_container_cleanup(oryn_common::v3::PageId(1), "a".repeat(64))
            .unwrap();
        drop(lease);
        assert_eq!(observed.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
        assert_eq!(observed.recv_timeout(Duration::from_secs(2)).unwrap(), 2);
        assert!(client.drain_script_cleanup(Duration::ZERO).is_err());
        release.send(()).unwrap();
        client.drain_script_cleanup(Duration::from_secs(2)).unwrap();
    }
    #[test]
    fn buffered_page_resources_finish_but_streamed_responses_retain_activity() {
        struct Service(Mutex<Option<ContextServiceResponse>>);
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                match request {
                    ContextServiceRequest::RunScriptLoad { .. } => {
                        Ok(self.0.lock().unwrap().take().unwrap())
                    }
                    ContextServiceRequest::CancelScriptLoad { .. } => {
                        Ok(ContextServiceResponse::ScriptLoadCancelled)
                    }
                    _ => panic!("unexpected resource request"),
                }
            }
        }
        let page_response = |stream| {
            serde_json::from_value::<ContextServiceResponse>(serde_json::json!({
            "result":"page_resource","url":"https://example.test/frame","source":"<p>body</p>","encoding":"UTF-8","destination":"frame","stream_id":stream
        })).unwrap()
        };
        let cases = vec![
            (page_response(None::<u64>), 0),
            (page_response(Some(17)), 1),
            (
                ContextServiceResponse::PageBinaryResource {
                    timing: None,
                    url: "https://example.test/image".into(),
                    body_base64: String::new(),
                    content_type: Some("image/png".into()),
                    destination: "image".into(),
                },
                0,
            ),
            (
                ContextServiceResponse::PageResourceNoCommit {
                    reason: "HTTP 204".into(),
                },
                0,
            ),
        ];
        for (response, remaining) in cases {
            let client = SharedContextClient(
                Arc::new(Service(Mutex::new(Some(response)))),
                None,
                Default::default(),
                Default::default(),
                #[cfg(feature = "v8-host")]
                None,
            );
            let page = oryn_common::v3::PageId(1);
            let load = super::super::ContextScriptLoad::from_reserved(client.clone(), page, 17);
            let creator = oryn_js::worker_wire::Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            };
            let request = ContextServiceRequest::LoadPageResource {
                page_id: page,
                download: None,
                script_authority: None,
                initiator: Box::new(super::super::ResourceInitiator {
                    embedded_type: None,
                    request_state: None,
                    navigation: Default::default(),
                    stylesheet: None,
                    creator,
                    prepared_policy: None,
                }),
                form_authority: None,
                url: "https://example.test/frame".into(),
                destination: "frame".into(),
                parent_encoding: None,
                initiator_url: None,
                referrer_policy: None,
                post: None,
            };
            load.run(request).unwrap();
            assert_eq!(client.resource_job_activity().0, remaining);
            drop(load);
            client.drain_script_cleanup(Duration::from_secs(2)).unwrap();
            assert_eq!(client.resource_job_activity().0, 0);
        }
    }

    #[test]
    fn pending_script_ack_retries_without_releasing_network_activity() {
        struct Service {
            attempt: Mutex<usize>,
            entered: mpsc::Sender<usize>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                assert!(matches!(
                    request,
                    ContextServiceRequest::CancelScriptLoad { token: 17, .. }
                ));
                let attempt = {
                    let mut count = self.attempt.lock().unwrap();
                    *count += 1;
                    *count
                };
                self.entered.send(attempt).unwrap();
                if attempt == 1 {
                    return Ok(ContextServiceResponse::ScriptLoadCancellationPending);
                }
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
                Ok(ContextServiceResponse::ScriptLoadCancelled)
            }
        }
        let (entered, observed) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let client = SharedContextClient(
            Arc::new(Service {
                attempt: Mutex::new(0),
                entered,
                release: Mutex::new(gate),
            }),
            None,
            Default::default(),
            Default::default(),
            #[cfg(feature = "v8-host")]
            None,
        );
        let load = super::super::ContextScriptLoad::from_reserved(
            client.clone(),
            oryn_common::v3::PageId(1),
            17,
        );
        drop(load);
        assert_eq!(observed.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
        assert_eq!(observed.recv_timeout(Duration::from_secs(2)).unwrap(), 2);
        assert_eq!(client.resource_job_activity().0, 1);
        assert!(client.drain_script_cleanup(Duration::ZERO).is_err());
        release.send(()).unwrap();
        client.drain_script_cleanup(Duration::from_secs(2)).unwrap();
        assert_eq!(client.resource_job_activity().0, 0);
    }

    #[test]
    fn failed_or_mismatched_load_cancels_before_retained_handle_is_dropped() {
        struct Service {
            mismatched: bool,
            entered: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                match request {
                    ContextServiceRequest::RunScriptLoad { .. } => {
                        if self.mismatched {
                            Ok(ContextServiceResponse::PageResourceNoCommit {
                                reason: "wrong response kind".into(),
                            })
                        } else {
                            Err(crate::RuntimeError::ContextService(
                                "transport interrupted".into(),
                            ))
                        }
                    }
                    ContextServiceRequest::CancelScriptLoad { token: 17, .. } => {
                        self.entered.send(()).unwrap();
                        self.release
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                        Ok(ContextServiceResponse::ScriptLoadCancelled)
                    }
                    _ => panic!("unexpected load operation"),
                }
            }
        }
        for mismatched in [false, true] {
            let (entered, observed) = mpsc::channel();
            let (release, gate) = mpsc::channel();
            let client = SharedContextClient(
                Arc::new(Service {
                    mismatched,
                    entered,
                    release: Mutex::new(gate),
                }),
                None,
                Default::default(),
                Default::default(),
                #[cfg(feature = "v8-host")]
                None,
            );
            let page = oryn_common::v3::PageId(1);
            let load = super::super::ContextScriptLoad::from_reserved(client.clone(), page, 17);
            let creator = oryn_js::worker_wire::Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            };
            let request = ContextServiceRequest::LoadScript {
                fallback_encoding: None,
                page_id: page,
                creator,
                url: "https://example.test/script.js".into(),
                module: false,
                nonce: None,
                integrity: None,
                cross_origin: None,
                initiator_url: None,
                referrer_policy: None,
                referrer_source: None,
                script_policy: None,
                parser_inserted: false,
            };
            let result = load.run(request);
            assert_eq!(
                result.is_ok(),
                mismatched,
                "caller retains its original transport result"
            );
            observed.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(client.resource_job_activity().0, 1);
            release.send(()).unwrap();
            client.drain_script_cleanup(Duration::from_secs(2)).unwrap();
            assert_eq!(client.resource_job_activity().0, 0);
            drop(load);
            client.drain_script_cleanup(Duration::ZERO).unwrap();
        }
    }

    #[test]
    fn buffered_script_response_finishes_only_its_page_token_with_owner_retained() {
        struct Service;
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                assert!(matches!(
                    request,
                    ContextServiceRequest::RunScriptLoad { token: 17, .. }
                ));
                Ok(ContextServiceResponse::ScriptSource {
                    timing: None,
                    url: "https://example.test/script.js".into(),
                    source: "void 0".into(),
                    module: false,
                    muted_errors: false,
                    referrer_policy: "strict-origin-when-cross-origin".into(),
                })
            }
        }
        let client = SharedContextClient(
            Arc::new(Service),
            None,
            Default::default(),
            Default::default(),
            #[cfg(feature = "v8-host")]
            None,
        );
        let first = super::super::ContextScriptLoad::from_reserved(
            client.clone(),
            oryn_common::v3::PageId(1),
            17,
        );
        let second = super::super::ContextScriptLoad::from_reserved(
            client.clone(),
            oryn_common::v3::PageId(2),
            17,
        );
        assert_eq!(client.resource_job_activity().0, 2);
        for (load, remaining) in [(&first, 1), (&second, 0)] {
            let creator = oryn_js::worker_wire::Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            };
            let request = ContextServiceRequest::LoadScript {
                fallback_encoding: None,
                page_id: load.page_id,
                creator,
                url: "https://example.test/script.js".into(),
                module: false,
                nonce: None,
                integrity: None,
                cross_origin: None,
                initiator_url: None,
                referrer_policy: None,
                referrer_source: None,
                script_policy: None,
                parser_inserted: false,
            };
            assert!(matches!(
                load.run(request),
                Ok(ContextServiceResponse::ScriptSource { .. })
            ));
            assert_eq!(client.resource_job_activity().0, remaining);
        }
        // Prepared-source handles remain alive; they no longer represent IO.
        drain(&client, Duration::ZERO).unwrap();
        drop((first, second));
        drain(&client, Duration::ZERO).unwrap();
    }

    #[test]
    fn cancelled_http_activity_survives_owner_drop_until_cleanup_ack() {
        struct Service {
            entered: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                assert!(matches!(
                    request,
                    ContextServiceRequest::ReleaseNetworkResult {
                        stream_id: Some(17),
                        ..
                    }
                ));
                self.entered.send(()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
                Ok(ContextServiceResponse::NetworkResultReleased)
            }
        }
        let (entered, observed) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let client = SharedContextClient(
            Arc::new(Service {
                entered,
                release: Mutex::new(gate),
            }),
            None,
            Default::default(),
            Default::default(),
            #[cfg(feature = "v8-host")]
            None,
        );
        let document = client.for_document_activity();
        let replacement = client.for_document_activity();
        let reply = document
            .reserve_http_network_reply(oryn_common::v3::PageId(71), None)
            .unwrap();
        reply.attach_stream(17);
        assert_eq!(client.resource_job_activity().0, 1);
        assert_eq!(document.resource_job_activity().0, 1);
        assert_eq!(replacement.resource_job_activity(), (0, None));
        reply.cancel();
        observed.recv_timeout(Duration::from_secs(2)).unwrap();
        // A late successful completion must not override pending cancellation.
        reply.finish();
        drop(reply);
        assert_eq!(client.resource_job_activity().0, 1);
        assert_eq!(document.resource_job_activity().0, 1);
        assert_eq!(replacement.resource_job_activity(), (0, None));
        assert!(client.retire_resource_jobs(Duration::ZERO).is_err());
        let acknowledged = Instant::now();
        release.send(()).unwrap();
        client
            .drain_resource_teardown(Duration::from_secs(2))
            .unwrap();
        let (active, changed) = client.resource_job_activity();
        assert_eq!(active, 0);
        assert!(changed.is_some_and(|at| at >= acknowledged));
        assert_eq!(document.resource_job_activity().0, 0);
        assert!(
            document
                .resource_job_activity()
                .1
                .is_some_and(|at| at >= acknowledged)
        );
        assert_eq!(replacement.resource_job_activity(), (0, None));
    }

    #[test]
    fn cancelled_frame_activity_survives_owner_drop_until_cleanup_ack() {
        struct Service {
            entered: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                assert!(matches!(
                    request,
                    ContextServiceRequest::ReleaseFrameResponseStream { stream_id: 17, .. }
                ));
                self.entered.send(()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
                Ok(ContextServiceResponse::NetworkResultReleased)
            }
        }
        let (entered, observed) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let client = SharedContextClient(
            Arc::new(Service {
                entered,
                release: Mutex::new(gate),
            }),
            None,
            Default::default(),
            Default::default(),
            #[cfg(feature = "v8-host")]
            None,
        );
        let reply = client
            .reserve_frame_response(oryn_common::v3::PageId(71))
            .unwrap();
        reply.attach(17, "prepared-frame-policy");
        assert_eq!(client.resource_job_activity().0, 1);
        reply.cancel();
        observed.recv_timeout(Duration::from_secs(2)).unwrap();
        // A late successful completion must not override pending cancellation.
        reply.finish();
        drop(reply);
        assert_eq!(client.resource_job_activity().0, 1);
        assert!(client.retire_resource_jobs(Duration::ZERO).is_err());
        let acknowledged = Instant::now();
        release.send(()).unwrap();
        client
            .drain_resource_teardown(Duration::from_secs(2))
            .unwrap();
        let (active, changed) = client.resource_job_activity();
        assert_eq!(active, 0);
        assert!(changed.is_some_and(|at| at >= acknowledged));
    }

    #[test]
    fn abandoned_navigation_keeps_exact_stop_until_terminal_parent_ack() {
        use crate::navigation_control::{NavigationEvent, NavigationId, NavigationStop};
        struct Service {
            seen: mpsc::Sender<usize>,
            release: Mutex<mpsc::Receiver<()>>,
            attempt: Mutex<usize>,
        }
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                let ContextServiceRequest::NavigationEvent {
                    page_id,
                    id,
                    event: NavigationEvent::Stopped(NavigationStop::Failed(_)),
                } = request
                else {
                    panic!("unexpected navigation cleanup")
                };
                assert_eq!(page_id, oryn_common::v3::PageId(71));
                assert_eq!(id, NavigationId(29));
                let mut attempt = self.attempt.lock().unwrap();
                *attempt += 1;
                let current = *attempt;
                drop(attempt);
                self.seen.send(current).unwrap();
                if current == 1 {
                    self.release
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                }
                Ok(ContextServiceResponse::NavigationControl {
                    result: if current == 1 {
                        Err(NavigationStop::Poisoned)
                    } else {
                        Err(NavigationStop::Superseded)
                    },
                })
            }
        }
        let (worker, parent) = std::os::unix::net::UnixStream::pair().unwrap();
        let shutdown = parent.try_clone().unwrap();
        let (seen, observed) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let server = std::thread::spawn(move || {
            serve_context_transport(
                parent,
                Arc::new(Service {
                    seen,
                    release: Mutex::new(gate),
                    attempt: Mutex::new(0),
                }),
            )
        });
        let client = SharedContextClient::new(IpcContextClient::new(worker));
        let completed = client
            .reserve_navigation_stop(oryn_common::v3::PageId(71), NavigationId(28))
            .unwrap();
        completed.finish();
        drop(completed);
        drain(&client, Duration::ZERO).unwrap();
        assert!(observed.try_recv().is_err());
        let abandoned = client
            .reserve_navigation_stop(oryn_common::v3::PageId(71), NavigationId(29))
            .unwrap();
        drop(abandoned);
        assert_eq!(observed.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
        assert!(drain(&client, Duration::ZERO).is_err());
        release.send(()).unwrap();
        assert_eq!(observed.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
        drain(&client, Duration::from_secs(5)).unwrap();
        shutdown.shutdown(std::net::Shutdown::Both).unwrap();
        let _ = server.join().unwrap();
    }

    #[test]
    fn network_result_guard_retains_original_route_until_release_ack_and_eof_finishes() {
        struct Service {
            requests: mpsc::Sender<u64>,
            release: Mutex<mpsc::Receiver<()>>,
            first: Mutex<bool>,
        }
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                let ContextServiceRequest::ReleaseNetworkResult {
                    stream_id: Some(id),
                    socket_id: None,
                    ..
                } = request
                else {
                    panic!("unexpected network cleanup")
                };
                self.requests.send(id).unwrap();
                let mut first = self.first.lock().unwrap();
                if *first {
                    *first = false;
                    self.release.lock().unwrap().recv().unwrap();
                    return Err(crate::RuntimeError::ContextService(
                        "injected release failure".into(),
                    ));
                }
                Ok(ContextServiceResponse::NetworkResultReleased)
            }
        }
        let (worker, parent) = std::os::unix::net::UnixStream::pair().unwrap();
        let shutdown = parent.try_clone().unwrap();
        let (requests, observed) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let server = std::thread::spawn(move || {
            serve_context_transport(
                parent,
                Arc::new(Service {
                    requests,
                    release: Mutex::new(gate),
                    first: Mutex::new(true),
                }),
            )
        });
        let client = SharedContextClient::new(IpcContextClient::new(worker));
        let completed = client
            .reserve_network_reply(oryn_common::v3::PageId(1), None)
            .unwrap();
        completed.attach_stream(18);
        completed.finish();
        drop(completed);
        drain(&client, Duration::ZERO).unwrap();
        assert!(observed.try_recv().is_err());
        let discarded = client
            .reserve_network_reply(oryn_common::v3::PageId(1), None)
            .unwrap();
        discarded.attach_stream(19);
        drop(discarded);
        assert_eq!(observed.recv_timeout(Duration::from_secs(5)).unwrap(), 19);
        assert!(drain(&client, Duration::ZERO).is_err());
        release.send(()).unwrap();
        assert_eq!(observed.recv_timeout(Duration::from_secs(5)).unwrap(), 19);
        drain(&client, Duration::from_secs(5)).unwrap();
        shutdown.shutdown(std::net::Shutdown::Both).unwrap();
        let _ = server.join().unwrap();
    }

    #[test]
    fn dropped_owner_never_waits_for_cancel_reply_and_failed_cleanup_is_retried() {
        struct Service {
            entered: mpsc::Sender<u64>,
            release: Mutex<mpsc::Receiver<()>>,
            first: Mutex<bool>,
        }
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                let ContextServiceRequest::CancelScriptLoad { token, .. } = request else {
                    panic!("unexpected cleanup request");
                };
                self.entered.send(token).unwrap();
                let mut first = self.first.lock().unwrap();
                if *first {
                    *first = false;
                    self.release.lock().unwrap().recv().unwrap();
                    Err(crate::RuntimeError::ContextService(
                        "transient cancellation failure".into(),
                    ))
                } else {
                    Ok(ContextServiceResponse::ScriptLoadCancelled)
                }
            }
        }
        let (worker, parent) = std::os::unix::net::UnixStream::pair().unwrap();
        let shutdown = parent.try_clone().unwrap();
        let (entered, requests) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let service = Arc::new(Service {
            entered,
            release: Mutex::new(blocked),
            first: Mutex::new(true),
        });
        let server = std::thread::spawn(move || serve_context_transport(parent, service));
        let client = SharedContextClient::new(IpcContextClient::new(worker));
        let ticket = Ticket::reserve(client.clone(), oryn_common::v3::PageId(1)).unwrap();
        ticket.track_network_activity().unwrap();
        assert_eq!(client.resource_job_activity().0, 1);
        assert!(
            drain(&client, Duration::ZERO).is_err(),
            "pending admission is part of the barrier"
        );
        ticket.attach(71);
        assert!(
            drain(&client, Duration::ZERO).is_err(),
            "live token owners are part of the barrier"
        );
        let owner = ticket.job.clone();
        drop(ticket);
        assert_eq!(requests.recv_timeout(Duration::from_secs(5)).unwrap(), 71);
        assert!(drain(&client, Duration::ZERO).is_err());
        assert!(!owner.state.lock().unwrap().done);
        assert_eq!(
            client.resource_job_activity().0,
            1,
            "failed cleanup retains network activity"
        );
        let acknowledged = Instant::now();
        release.send(()).unwrap();
        assert_eq!(requests.recv_timeout(Duration::from_secs(5)).unwrap(), 71);
        drain(&client, Duration::from_secs(5)).unwrap();
        assert!(owner.state.lock().unwrap().done);
        let (active, changed) = client.resource_job_activity();
        assert_eq!(active, 0);
        assert!(changed.is_some_and(|at| at >= acknowledged));
        shutdown.shutdown(std::net::Shutdown::Both).unwrap();
        let _ = server.join().unwrap();
    }
}
