//! Frozen, bounded hyperlink audit plans. Navigation retirement does not revoke
//! an accepted keepalive request; page/context closure still owns cancellation.

/// Inputs retained together for prepare standalone audit.
#[cfg(feature = "v8-host")]
pub(crate) struct StandaloneAuditInput<'a> {
    pub(crate) broker: SharedNetworkBroker,
    pub(crate) source: Url,
    pub(crate) origin: String,
    pub(crate) top: String,
    pub(crate) ancestors: Vec<String>,
    pub(crate) policy: crate::csp::CspPolicySet,
    pub(crate) reports: Arc<crate::executable::module_graph_load::ScriptCspReports>,
    pub(crate) destination: &'a str,
    pub(crate) urls: &'a [String],
}
#[cfg(test)]
type FixtureResponses = Arc<Mutex<VecDeque<(u16, Vec<(String, String)>)>>>;

use super::*;
use crate::context_service::http_cleanup_custody::{Scope, Ticket};
use oryn_js::navigation::{HyperlinkAuditToken, NavigationExecutionToken};
const MAX_PLANS: usize = 128;
const MAX_BYTES: usize = 64 * 1024;
const LIFETIME: Duration = Duration::from_secs(30);
struct Plan {
    page: PageId,
    destination: Url,
    source: Url,
    origin: String,
    top: String,
    ancestors: Vec<String>,
    urls: Vec<Url>,
    policy: crate::csp::CspPolicySet,
    deadline: Instant,
}
#[derive(Default)]
pub(super) struct Registry {
    pending: BTreeMap<Vec<u8>, Plan>,
    bound: BTreeMap<(PageId, Vec<u8>), HyperlinkAuditToken>,
    completed: VecDeque<Vec<u8>>,
    scopes: Vec<(PageId, Arc<Scope>)>,
}
pub(crate) struct Pending {
    context: BrowserContext,
    token: Option<HyperlinkAuditToken>,
}
impl Pending {
    pub(crate) fn new(context: BrowserContext, token: Option<HyperlinkAuditToken>) -> Self {
        Self { context, token }
    }
    pub(crate) fn take(&mut self) -> Option<HyperlinkAuditToken> {
        self.token.take()
    }
    pub(crate) fn bind(
        &mut self,
        page: PageId,
        receipt: &NavigationExecutionToken,
        destination: &str,
    ) -> Result<(), RuntimeError> {
        if let Some(token) = self.token.as_ref() {
            self.context
                .bind_hyperlink_audit(token.clone(), page, receipt, destination)?;
            self.token = None;
        }
        Ok(())
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            let _ = self
                .context
                .finish_hyperlink_audit(PageId(token.page_id), &token, false);
        }
    }
}
impl Registry {
    fn prune(&mut self) {
        let now = Instant::now();
        self.pending.retain(|_, plan| plan.deadline > now);
        self.bound
            .retain(|_, token| self.pending.contains_key(&token.capability));
        self.scopes
            .retain(|(_, scope)| !scope.is_drained().unwrap_or(false));
    }
    fn consume(&mut self, token: &HyperlinkAuditToken) -> Result<Option<Plan>, RuntimeError> {
        self.prune();
        if token.capability.len() != 32 {
            return Err(RuntimeError::ProtocolViolation(
                "invalid audit capability".into(),
            ));
        }
        if self
            .pending
            .get(&token.capability)
            .is_some_and(|plan| plan.page.0 != token.page_id)
        {
            return Err(RuntimeError::ProtocolViolation(
                "foreign audit owner".into(),
            ));
        }
        let plan = self.pending.remove(&token.capability);
        self.bound.retain(|_, other| other != token);
        if plan.is_some() {
            if self.completed.len() == 256 {
                self.completed.pop_front();
            }
            self.completed.push_back(token.capability.clone());
        }
        Ok(plan)
    }
    fn consume_reserved(
        &mut self,
        token: &HyperlinkAuditToken,
        tickets: Vec<Ticket>,
    ) -> Result<Option<(Plan, Vec<Ticket>)>, RuntimeError> {
        // Reservation can outlive the plan deadline. Expiry owns no network
        // work; dropping the unused tickets releases their cleanup capacity.
        Ok(self.consume(token)?.map(|plan| (plan, tickets)))
    }
    pub(super) fn retire(&mut self, page: PageId) -> Vec<Arc<Scope>> {
        self.pending.retain(|_, plan| plan.page != page);
        self.bound.retain(|_, token| token.page_id != page.0);
        let scopes = self
            .scopes
            .iter()
            .filter(|(owner, _)| *owner == page)
            .map(|(_, scope)| scope.clone())
            .collect::<Vec<_>>();
        for scope in &scopes {
            scope.cancel(None);
        }
        scopes
    }
}
impl Drop for Registry {
    fn drop(&mut self) {
        for (_, scope) in &self.scopes {
            scope.cancel(None);
        }
    }
}
impl BrowserContext {
    pub(crate) fn prepare_hyperlink_audit(
        &self,
        page: PageId,
        creator: &oryn_js::worker_wire::Creator,
        token: HyperlinkAuditToken,
        destination: &str,
        urls: Vec<String>,
    ) -> Result<(), RuntimeError> {
        let oryn_js::worker_wire::Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "audit requires Window owner".into(),
            ));
        };
        let (owner, source, origin, mut top) = self.with_window_worker_creator(
            page,
            target,
            *document_generation,
            *stream_epoch,
            true,
            |(owner, _partition, origin), source, top| {
                let source = source.ok_or_else(|| {
                    RuntimeError::ProtocolViolation(
                        "audit source document URL is not committed".into(),
                    )
                })?;
                let top = top.ok_or_else(|| {
                    RuntimeError::ProtocolViolation(
                        "audit top document URL is not committed".into(),
                    )
                })?;
                Ok((owner, source, origin, top))
            },
        )?;
        let policy =
            self.window_worker_policy(page, target, *document_generation, *stream_epoch)?;
        if token.page_id != page.0
            || token.capability.len() != 32
            || urls.is_empty()
            || urls.len() > 16
        {
            return Err(RuntimeError::ProtocolViolation(
                "invalid audit preparation".into(),
            ));
        }
        let policy_bytes = policy.headers.iter().fold(0usize, |bytes, (name, value)| {
            bytes.saturating_add(name.len()).saturating_add(value.len())
        });
        let bytes = urls.iter().fold(
            destination
                .len()
                .saturating_add(source.len())
                .saturating_add(policy_bytes),
            |bytes, url| bytes.saturating_add(url.len()),
        );
        if bytes > MAX_BYTES {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "hyperlink audit bytes".into(),
            });
        }
        let source =
            Url::parse(&source).map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let destination = Url::parse(destination)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let urls = urls
            .into_iter()
            .map(|url| {
                Url::parse(&url).map_err(|error| RuntimeError::ContextService(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if urls.iter().any(|url| {
            !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
        }) {
            return Err(RuntimeError::ProtocolViolation(
                "invalid audit endpoint".into(),
            ));
        }
        if Url::parse(&top).is_ok_and(|url| !matches!(url.scheme(), "http" | "https")) {
            top = self.security_origin(page)?.serialize();
        }
        let ancestors = self.window_ancestor_urls(&owner)?;
        if ancestors
            .iter()
            .fold(bytes.saturating_add(top.len()), |bytes, url| {
                bytes.saturating_add(url.len())
            })
            > MAX_BYTES
        {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "hyperlink audit ancestry bytes".into(),
            });
        }
        // Reauthenticate after all source policy reads; only this frozen plan
        // survives source retirement. No creator lookup occurs during commit.
        self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        let mut registry = self
            .inner
            .hyperlink_audits
            .lock()
            .map_err(|_| RuntimeError::ContextService("audit registry poisoned".into()))?;
        registry.prune();
        if registry.pending.contains_key(&token.capability)
            || registry.completed.contains(&token.capability)
        {
            return Err(RuntimeError::ProtocolViolation(
                "reused audit preparation".into(),
            ));
        }
        if registry.pending.len() + registry.scopes.len() >= MAX_PLANS
            || registry
                .pending
                .values()
                .filter(|plan| plan.page == page)
                .count()
                >= 32
        {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "hyperlink audit plans".into(),
            });
        }
        registry.pending.insert(
            token.capability,
            Plan {
                page,
                destination,
                source,
                origin: origin.serialized,
                top,
                ancestors,
                urls,
                policy: crate::csp::CspPolicySet::from_agent_policy(&policy),
                deadline: Instant::now() + LIFETIME,
            },
        );
        Ok(())
    }
    pub(crate) fn bind_hyperlink_audit(
        &self,
        token: HyperlinkAuditToken,
        page: PageId,
        receipt: &NavigationExecutionToken,
        destination: &str,
    ) -> Result<(), RuntimeError> {
        let mut registry = self
            .inner
            .hyperlink_audits
            .lock()
            .map_err(|_| RuntimeError::ContextService("audit registry poisoned".into()))?;
        registry.prune();
        let Some(plan) = registry.pending.get(&token.capability) else {
            return Ok(());
        };
        if plan.page.0 != token.page_id || plan.destination.as_str() != destination {
            return Err(RuntimeError::ProtocolViolation(
                "audit navigation binding mismatch".into(),
            ));
        }
        if registry.bound.values().any(|value| value == &token) {
            return Err(RuntimeError::ProtocolViolation(
                "audit already bound".into(),
            ));
        }
        registry.bound.insert((page, receipt.0.clone()), token);
        Ok(())
    }
    pub(crate) fn accept_hyperlink_audit_receipt(
        &self,
        page: PageId,
        receipt: &NavigationExecutionToken,
    ) -> Result<(), RuntimeError> {
        let token = self
            .inner
            .hyperlink_audits
            .lock()
            .map_err(|_| RuntimeError::ContextService("audit registry poisoned".into()))?
            .bound
            .get(&(page, receipt.0.clone()))
            .cloned();
        if let Some(token) = token {
            self.finish_hyperlink_audit(PageId(token.page_id), &token, true)?;
        }
        Ok(())
    }
    pub(crate) fn discard_hyperlink_audit_receipt(
        &self,
        page: PageId,
        receipt: &NavigationExecutionToken,
    ) {
        let token = self
            .inner
            .hyperlink_audits
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bound
            .get(&(page, receipt.0.clone()))
            .cloned();
        if let Some(token) = token {
            let _ = self.finish_hyperlink_audit(PageId(token.page_id), &token, false);
        }
    }
    pub(crate) fn finish_hyperlink_audit(
        &self,
        page: PageId,
        token: &HyperlinkAuditToken,
        commit: bool,
    ) -> Result<(), RuntimeError> {
        if token.page_id != page.0 {
            return Err(RuntimeError::ProtocolViolation(
                "foreign audit completion".into(),
            ));
        }
        let mut registry = self
            .inner
            .hyperlink_audits
            .lock()
            .map_err(|_| RuntimeError::ContextService("audit registry poisoned".into()))?;
        if !commit {
            registry.consume(token)?;
            return Ok(());
        }
        registry.prune();
        let Some(plan) = registry.pending.get(&token.capability) else {
            return Ok(());
        };
        if plan.page != page {
            return Err(RuntimeError::ProtocolViolation(
                "foreign audit capability".into(),
            ));
        }
        let Some(broker) = self.inner.broker.clone() else {
            registry.consume(token)?;
            return Ok(());
        };
        let broker = broker.new_request_scope();
        let scope = Arc::new(Scope::default());
        // Reserve all cancellation owners before consuming the capability.
        let tickets = (0..plan.urls.len())
            .map(|index| Ticket::reserve(&scope, index as u64, broker.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        let Some((plan, tickets)) = registry.consume_reserved(token, tickets)? else {
            return Ok(());
        };
        registry.scopes.push((page, scope.clone()));
        drop(registry);
        let context = Arc::downgrade(&self.inner);
        crate::resource_io_executor::submit(move || {
            for (index, (url, ticket)) in plan.urls.iter().cloned().zip(tickets).enumerate() {
                if ticket.cancelled() || Instant::now() >= plan.deadline {
                    break;
                }
                let _ = deliver(
                    &broker,
                    &scope,
                    &plan,
                    url,
                    index as u64 * 11,
                    &ticket,
                    |violation| {
                        if let Some(inner) = context.upgrade() {
                            let context = BrowserContext { inner };
                            for target in plan.policy.report_targets(&violation) {
                                let _ = context.queue_native_csp_report(
                                    Some(page),
                                    plan.top.clone(),
                                    target,
                                    violation.clone(),
                                );
                            }
                        }
                    },
                );
            }
        })
        .map_err(RuntimeError::ContextService)
    }
}
fn audit_headers(plan: &Plan, url: &Url) -> Vec<(String, String)> {
    let mut headers = vec![
        ("content-type".into(), "text/ping".into()),
        ("ping-to".into(), plan.destination.to_string()),
        ("cache-control".into(), "no-store".into()),
        ("origin".into(), plan.origin.clone()),
    ];
    if url.origin().ascii_serialization() == plan.origin || plan.source.scheme() != "https" {
        let mut source = plan.source.clone();
        source.set_fragment(None);
        let _ = source.set_username("");
        let _ = source.set_password(None);
        headers.push(("ping-from".into(), source.into()));
    }
    headers
}
fn deliver(
    broker: &SharedNetworkBroker,
    scope: &Scope,
    plan: &Plan,
    mut url: Url,
    first_id: u64,
    ticket: &Ticket,
    mut report: impl FnMut(crate::csp::CspViolation),
) -> Result<(), String> {
    let mut metadata = crate::fetch_metadata::Metadata::resource("no-cors", "empty", false);
    let mut post = true;
    let mut headers = audit_headers(plan, &url);
    let initial_site = crate::cookies::SchemefulSite::from_url(&url);
    let mut crossed = false;
    for redirect in 0..=10 {
        if ticket.cancelled() {
            return Err("audit canceled".into());
        }
        if let Some(upgraded) = plan.policy.upgraded_url(&url) {
            url = upgraded;
        }
        let decision = plan.policy.authorize(&crate::csp::CspRequest {
            destination: crate::csp::CspDestination::Connect,
            url: url.to_string(),
            nonce: None,
            inline_source: None,
            parser_inserted: false,
            initiated_by_trusted_script: false,
        });
        for violation in decision.violations {
            report(violation);
        }
        if !decision.allowed {
            return Err("audit CSP denied".into());
        }
        crate::context_service::enforce_mixed_content(
            if plan.source.scheme() == "https" {
                plan.source.as_str()
            } else {
                &plan.origin
            },
            &url,
        )
        .map_err(|error| error.to_string())?;
        metadata.observe(Some(&plan.origin), &url);
        // Each redirect has its own native cancellation owner; the initial
        // ticket protects queued admission and page-close cancellation.
        let request_id = first_id + redirect as u64;
        let hop = Ticket::reserve(scope, request_id, broker.clone())
            .map_err(|error| error.to_string())?;
        hop.attach(request_id);
        let response = crate::network::collect_operation_bounded_with_cancel(
            broker,
            NetworkRequest {
                request_id,
                method: if post { "POST" } else { "GET" }.into(),
                url: url.to_string(),
                headers: headers.clone(),
                body: if post { b"PING".to_vec() } else { Vec::new() },
                credentials: true,
                resolved_addrs: Vec::new(),
                security: crate::cookies::CookieRequestContext {
                    fetch_metadata: metadata.clone(),
                    fetch_cache: oryn_js::platform::FetchCacheMode::NoStore,
                    credentials: crate::cookies::CookieCredentials::Include,
                    kind: crate::cookies::CookieRequestKind::Other,
                    initiator_url: Some(if plan.origin == "null" {
                        "about:blank".into()
                    } else if plan.source.origin().ascii_serialization() == plan.origin {
                        plan.source.to_string()
                    } else {
                        plan.origin.clone()
                    }),
                    top_level_url: Some(plan.top.clone()),
                    ancestor_urls: plan.ancestors.clone(),
                    redirect_chain_cross_site: crossed,
                    ..Default::default()
                },
            },
            plan.deadline
                .checked_duration_since(Instant::now())
                .ok_or("audit deadline exceeded")?,
            64 * 1024,
            ticket.cancellation(),
        )
        .map_err(|error| error.to_string())?;
        if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            return Ok(());
        }
        if redirect == 10 {
            return Err("audit redirect limit".into());
        }
        if post && matches!(response.status, 301..=303) {
            post = false;
            headers.retain(|(name, _)| name != "content-type" && name != "origin");
        }
        let location = response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("location"))
            .map(|(_, value)| value)
            .ok_or("audit redirect lacks Location")?;
        let next = url.join(location).map_err(|error| error.to_string())?;
        if post
            && next.origin() != url.origin()
            && let Some((_, value)) = headers.iter_mut().find(|(name, _)| name == "origin")
        {
            *value = "null".into();
        }
        url = next;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("invalid audit redirect".into());
        }
        crossed |= crate::cookies::SchemefulSite::from_url(&url) != initial_site;
    }
    unreachable!()
}

/// Isolate-owned custody for standalone pages. It is independent of a retiring
/// source Document and canceled when the hosting page/isolate is destroyed.
#[cfg(feature = "v8-host")]
#[derive(Default)]
pub(crate) struct StandaloneOwner(Mutex<Registry>);
#[cfg(feature = "v8-host")]
struct StandaloneAudit {
    token: HyperlinkAuditToken,
    job: Mutex<Option<(Plan, Vec<Ticket>)>>,
    broker: SharedNetworkBroker,
    scope: Arc<Scope>,
    reports: Arc<crate::executable::module_graph_load::ScriptCspReports>,
}
#[cfg(feature = "v8-host")]
impl oryn_js::navigation::HyperlinkAudit for StandaloneAudit {
    fn token(&self) -> HyperlinkAuditToken {
        self.token.clone()
    }
    fn accepted(&self) {
        let Some((plan, tickets)) = self
            .job
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        else {
            return;
        };
        let broker = self.broker.clone();
        let scope = self.scope.clone();
        let reports = self.reports.clone();
        let _ = crate::resource_io_executor::submit(move || {
            for (index, (url, ticket)) in plan.urls.iter().cloned().zip(tickets).enumerate() {
                if ticket.cancelled() || Instant::now() >= plan.deadline {
                    break;
                }
                let _ = deliver(
                    &broker,
                    &scope,
                    &plan,
                    url,
                    index as u64 * 11,
                    &ticket,
                    |violation| {
                        let _ = reports.push_with_policy(&plan.policy, vec![violation]);
                    },
                );
            }
        });
    }
    fn discarded(&self) {
        self.job
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }
    fn transferred(&self) {
        self.discarded();
    }
}
#[cfg(feature = "v8-host")]
impl StandaloneOwner {
    pub(crate) fn prepare(
        &self,
        params: StandaloneAuditInput<'_>,
    ) -> Result<Arc<dyn oryn_js::navigation::HyperlinkAudit>, String> {
        let StandaloneAuditInput {
            broker,
            source,
            origin,
            top,
            ancestors,
            policy,
            reports,
            destination,
            urls,
        } = params;

        let retained = policy.worker_snapshot().headers.iter().fold(
            source
                .as_str()
                .len()
                .saturating_add(top.len())
                .saturating_add(destination.len()),
            |bytes, (name, value)| bytes.saturating_add(name.len()).saturating_add(value.len()),
        );
        let retained = ancestors
            .iter()
            .fold(retained, |bytes, url| bytes.saturating_add(url.len()));
        if urls.is_empty()
            || urls.len() > 16
            || urls
                .iter()
                .fold(retained, |bytes, url| bytes.saturating_add(url.len()))
                > MAX_BYTES
        {
            return Err("audit preparation bounds".into());
        }
        let destination = Url::parse(destination).map_err(|error| error.to_string())?;
        let urls = urls
            .iter()
            .map(|url| Url::parse(url).map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        if urls.iter().any(|url| {
            !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
        }) {
            return Err("invalid audit endpoint".into());
        }
        let mut registry = self.0.lock().map_err(|_| "audit registry poisoned")?;
        registry.prune();
        if registry.scopes.len() >= MAX_PLANS {
            return Err("audit capacity exceeded".into());
        }
        let scope = Arc::new(Scope::default());
        let broker = broker.new_request_scope();
        let tickets = (0..urls.len())
            .map(|index| {
                Ticket::reserve(&scope, index as u64, broker.clone())
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut capability = vec![0; 32];
        getrandom::fill(&mut capability).map_err(|_| "audit entropy unavailable")?;
        let token = HyperlinkAuditToken {
            page_id: 0,
            capability,
        };
        registry.scopes.push((PageId(0), scope.clone()));
        Ok(Arc::new(StandaloneAudit {
            token,
            job: Mutex::new(Some((
                Plan {
                    page: PageId(0),
                    destination,
                    source,
                    origin,
                    top,
                    ancestors,
                    urls,
                    policy,
                    deadline: Instant::now() + LIFETIME,
                },
                tickets,
            ))),
            broker,
            scope,
            reports,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{NetworkBroker, NetworkError, NetworkResponse};
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        worker_wire::Creator,
    };
    fn plan(source: &str, endpoint: &str, policy: &str) -> Plan {
        let source = Url::parse(source).unwrap();
        let origin = source.origin().ascii_serialization();
        Plan {
            page: PageId(1),
            destination: Url::parse("https://destination.test/next").unwrap(),
            top: source.to_string(),
            ancestors: vec![],
            policy: crate::csp::CspPolicySet::from_headers(
                source.as_str(),
                &[("content-security-policy".into(), policy.into())],
            ),
            source,
            origin,
            urls: vec![Url::parse(endpoint).unwrap()],
            deadline: Instant::now() + LIFETIME,
        }
    }
    #[derive(Clone)]
    struct Broker {
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
        responses: FixtureResponses,
    }
    impl NetworkBroker for Broker {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let request_id = request.request_id;
            self.requests.lock().unwrap().push(request);
            let (status, headers) = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected audit request");
            Ok(NetworkResponse {
                response_policy: None,
                request_id,
                status,
                headers,
                body: vec![],
                timing: None,
            })
        }
    }
    fn broker(
        responses: Vec<(u16, Vec<(String, String)>)>,
    ) -> (SharedNetworkBroker, Arc<Mutex<Vec<NetworkRequest>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        (
            SharedNetworkBroker::new(Broker {
                requests: requests.clone(),
                responses: Arc::new(Mutex::new(responses.into())),
            }),
            requests,
        )
    }
    #[test]
    fn native_ping_body_headers_redirect_method_and_identity() {
        let plan = plan(
            "http://source.test/page#fragment",
            "http://audit.test/first",
            "",
        );
        let (network, requests) = broker(vec![
            (307, vec![("location".into(), "/second".into())]),
            (303, vec![("location".into(), "/final".into())]),
            (204, vec![]),
        ]);
        let scope = Scope::default();
        let ticket = Ticket::reserve(&scope, 900, network.clone()).unwrap();
        deliver(
            &network,
            &scope,
            &plan,
            plan.urls[0].clone(),
            30,
            &ticket,
            |_| {},
        )
        .unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests
                .iter()
                .map(|request| request.request_id)
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
        for request in &requests[..2] {
            assert_eq!(request.method, "POST");
            assert_eq!(request.body, b"PING");
            assert!(
                request
                    .headers
                    .contains(&("content-type".into(), "text/ping".into()))
            );
            assert!(
                request
                    .headers
                    .contains(&("ping-from".into(), "http://source.test/page".into()))
            );
            assert!(
                request
                    .headers
                    .contains(&("ping-to".into(), "https://destination.test/next".into()))
            );
            assert!(request.credentials);
        }
        assert_eq!(requests[2].method, "GET");
        assert!(requests[2].body.is_empty());
        assert!(
            !requests[2]
                .headers
                .iter()
                .any(|(name, _)| name == "content-type")
        );
        assert!(
            requests
                .iter()
                .all(|request| !request.headers.iter().any(|(name, _)| name == "referer"))
        );
    }
    #[test]
    fn https_cross_origin_ping_hides_source_and_csp_blocks_redirect() {
        let plan = plan(
            "https://source.test/private",
            "https://source.test/ping",
            "connect-src 'self'",
        );
        assert!(
            audit_headers(&plan, &Url::parse("https://elsewhere.test/").unwrap())
                .iter()
                .all(|(name, _)| name != "ping-from")
        );
        let (network, requests) = broker(vec![(
            307,
            vec![("location".into(), "https://elsewhere.test/ping".into())],
        )]);
        let scope = Scope::default();
        let ticket = Ticket::reserve(&scope, 901, network.clone()).unwrap();
        let mut violations = Vec::new();
        assert!(
            deliver(
                &network,
                &scope,
                &plan,
                plan.urls[0].clone(),
                40,
                &ticket,
                |violation| violations.push(violation)
            )
            .is_err()
        );
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert!(!violations.is_empty());
        scope.cancel(None);
        assert!(
            deliver(
                &network,
                &scope,
                &plan,
                plan.urls[0].clone(),
                50,
                &ticket,
                |_| {}
            )
            .is_err()
        );
        assert_eq!(requests.lock().unwrap().len(), 1);
    }
    #[test]
    fn exact_prepared_owner_survives_document_retirement_but_not_page_close() {
        let context = Browser::in_process_with_config(Default::default()).new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .commit_page_url(id, "https://source.test/old".into())
            .unwrap();
        context
            .install_document_security(
                id,
                "https://source.test/old".into(),
                &[(
                    "content-security-policy".into(),
                    "connect-src 'self'".into(),
                )],
            )
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let token = HyperlinkAuditToken {
            page_id: id.0,
            capability: vec![7; 32],
        };
        context
            .prepare_hyperlink_audit(
                id,
                &creator,
                token.clone(),
                "https://destination.test/next",
                vec!["https://source.test/ping".into()],
            )
            .unwrap();
        let receipt = NavigationExecutionToken(vec![8; 32]);
        assert!(
            context
                .bind_hyperlink_audit(token.clone(), id, &receipt, "https://other.test/")
                .is_err()
        );
        context
            .bind_hyperlink_audit(token.clone(), id, &receipt, "https://destination.test/next")
            .unwrap();
        assert!(
            context
                .bind_hyperlink_audit(
                    token.clone(),
                    id,
                    &NavigationExecutionToken(vec![9; 32]),
                    "https://destination.test/next"
                )
                .is_err()
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .prepare_hyperlink_audit(
                    id,
                    &creator,
                    HyperlinkAuditToken {
                        page_id: id.0,
                        capability: vec![10; 32]
                    },
                    "https://destination.test/next",
                    vec!["https://source.test/ping".into()]
                )
                .is_err()
        );
        let mut registry = context.inner.hyperlink_audits.lock().unwrap();
        let retained = registry.pending.get(&token.capability).unwrap();
        assert_eq!(retained.source.as_str(), "https://source.test/old");
        assert_eq!(registry.bound.len(), 1);
        registry.retire(id);
        assert!(registry.pending.is_empty());
        assert!(registry.bound.is_empty());
    }
    #[test]
    fn expiry_after_cleanup_reservation_releases_capacity_without_delivery() {
        let token = HyperlinkAuditToken {
            page_id: 1,
            capability: vec![12; 32],
        };
        let mut registry = Registry::default();
        registry.pending.insert(
            token.capability.clone(),
            plan("https://source.test/", "https://source.test/ping", ""),
        );
        assert!(registry.pending.contains_key(&token.capability));
        let (network, requests) = broker(vec![]);
        let scope = Scope::default();
        let tickets = vec![Ticket::reserve(&scope, 0, network).unwrap()];
        registry
            .pending
            .get_mut(&token.capability)
            .unwrap()
            .deadline = Instant::now() - Duration::from_secs(1);
        assert!(
            registry
                .consume_reserved(&token, tickets)
                .unwrap()
                .is_none()
        );
        scope.drain(Duration::from_secs(1)).unwrap();
        assert!(scope.is_drained().unwrap());
        assert!(requests.lock().unwrap().is_empty());
        assert!(registry.pending.is_empty());
        assert!(registry.scopes.is_empty());
        assert!(registry.consume_reserved(&token, vec![]).unwrap().is_none());
    }
    #[test]
    fn canceled_and_expired_capabilities_cannot_replay() {
        let token = HyperlinkAuditToken {
            page_id: 1,
            capability: vec![1; 32],
        };
        let mut registry = Registry::default();
        registry.pending.insert(
            token.capability.clone(),
            plan("https://source.test/", "https://source.test/ping", ""),
        );
        assert!(
            registry
                .consume(&HyperlinkAuditToken {
                    page_id: 2,
                    capability: token.capability.clone()
                })
                .is_err()
        );
        assert_eq!(registry.pending.len(), 1);
        assert!(registry.consume(&token).unwrap().is_some());
        assert!(registry.consume(&token).unwrap().is_none());
        let mut expired = plan("https://source.test/", "https://source.test/ping", "");
        expired.deadline = Instant::now() - Duration::from_secs(1);
        registry.pending.insert(vec![2; 32], expired);
        registry.prune();
        assert!(registry.pending.is_empty());
    }
}
