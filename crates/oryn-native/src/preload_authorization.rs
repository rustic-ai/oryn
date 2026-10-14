//! Parent-only reuse evidence. Opaque handles bind the original request and
//! complete redirect sequence to one authenticated Document owner.
use crate::{RuntimeError, agent_registry::AgentOwner};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
static REGISTRIES: AtomicU64 = AtomicU64::new(1);
pub(super) struct Registry {
    identity: u64,
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    next: u64,
    retired: bool,
    bytes: usize,
    entries: BTreeMap<u64, Arc<Proof>>,
}
pub(super) struct Proof {
    owner: AgentOwner,
    key: Vec<u8>,
    pub urls: Vec<String>,
    pub hash_evidence: Option<crate::csp::ScriptHashEvidence>,
    hash_consumption: Mutex<Option<[u8; 32]>>,
    expires: Instant,
    bytes: usize,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            identity: REGISTRIES
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .expect("preload evidence registry identity exhausted"),
            state: Default::default(),
        }
    }
}
fn error(message: &str) -> RuntimeError {
    RuntimeError::ProtocolViolation(message.into())
}
impl Registry {
    #[cfg(test)]
    pub(super) fn capture(
        &self,
        owner: AgentOwner,
        key: Vec<u8>,
        urls: Vec<String>,
    ) -> Result<String, RuntimeError> {
        self.capture_with_hash(owner, key, urls, None)
    }
    pub(super) fn capture_with_hash(
        &self,
        owner: AgentOwner,
        key: Vec<u8>,
        urls: Vec<String>,
        hash_evidence: Option<crate::csp::ScriptHashEvidence>,
    ) -> Result<String, RuntimeError> {
        if urls.is_empty()
            || urls.len() > 11
            || urls
                .iter()
                .any(|url| url.len() > 256 * 1024 || url::Url::parse(url).is_err())
        {
            return Err(error("invalid native preload redirect evidence"));
        }
        let bytes = key
            .len()
            .saturating_add(urls.iter().map(String::len).sum::<usize>())
            .saturating_add(
                hash_evidence
                    .as_ref()
                    .map_or(0, |e| e.sha256.len() + e.sha384.len() + e.sha512.len() + 64),
            );
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.retired {
            return Err(RuntimeError::Closed);
        }
        state
            .entries
            .retain(|_, proof| proof.expires > Instant::now());
        state.bytes = state.entries.values().map(|proof| proof.bytes).sum();
        if state.entries.len() >= 1024 || bytes > 8 * 1024 * 1024 - state.bytes {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "preload authorization evidence".into(),
            });
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or_else(|| error("preload evidence identity exhausted"))?;
        let id = state.next;
        state.entries.insert(
            id,
            Arc::new(Proof {
                owner,
                key,
                urls,
                hash_evidence,
                hash_consumption: Mutex::new(None),
                expires: Instant::now() + Duration::from_secs(60),
                bytes,
            }),
        );
        state.bytes += bytes;
        Ok(format!("{}:{id}", self.identity))
    }
    pub(super) fn get(
        &self,
        token: &str,
        owner: &AgentOwner,
        key: &[u8],
    ) -> Result<Arc<Proof>, RuntimeError> {
        let (registry, id) = token
            .split_once(':')
            .ok_or_else(|| error("invalid preload evidence token"))?;
        if registry.parse::<u64>().ok() != Some(self.identity) {
            return Err(error("foreign preload evidence registry"));
        }
        let id = id
            .parse::<u64>()
            .map_err(|_| error("invalid preload evidence identity"))?;
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.retired {
            return Err(RuntimeError::Closed);
        }
        let proof = state
            .entries
            .get(&id)
            .filter(|proof| {
                proof.expires > Instant::now() && &proof.owner == owner && proof.key == key
            })
            .ok_or_else(|| error("preload evidence does not match live request owner"))?;
        Ok(proof.clone())
    }
    pub(super) fn retire(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.retired = true;
        state.entries.clear();
        state.bytes = 0;
    }
}

impl Proof {
    /// Native enqueue only: the lock spans enqueue and receipt commit, never author
    /// callbacks or transport. Failed enqueue keeps the same consumption retryable.
    #[cfg(test)]
    pub(super) fn publish_hash_once(
        &self,
        consumer: &[u8],
        publish: impl FnOnce(&crate::csp::ScriptHashEvidence) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        self.publish_consumption_once(consumer, |evidence| match evidence {
            Some(evidence) => publish(evidence),
            None => Ok(()),
        })
    }
    pub(super) fn publish_consumption_once(
        &self,
        consumer: &[u8],
        publish: impl FnOnce(Option<&crate::csp::ScriptHashEvidence>) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        use sha2::Digest;
        let identity: [u8; 32] = sha2::Sha256::digest(consumer).into();
        let mut receipt = self
            .hash_consumption
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = *receipt {
            return if previous == identity {
                Ok(())
            } else {
                Err(error(
                    "preload response already consumed by another request",
                ))
            };
        }
        publish(self.hash_evidence.as_ref())?;
        *receipt = Some(identity);
        Ok(())
    }
}

use super::{
    ContextServiceRequest as Request, ContextServiceResponse as Response, PageResourceOptions,
    ParentContextService, TracedPageScriptInputs,
};
impl ParentContextService {
    fn preload_request_owner(
        &self,
        request: &Request,
    ) -> Result<
        (
            oryn_common::v3::PageId,
            oryn_js::worker_wire::Creator,
            AgentOwner,
        ),
        RuntimeError,
    > {
        let (page, creator) = match request {
            Request::LoadScript {
                page_id,
                creator,
                module: false,
                ..
            } => (*page_id, creator.clone()),
            Request::LoadPageResource {
                page_id,
                initiator,
                destination,
                ..
            } if destination == "stylesheet" && initiator.prepared_policy.is_none() => {
                (*page_id, initiator.creator.clone())
            }
            _ => return Err(error("unsupported preload proof request")),
        };
        self.validate_socket_owner(page)?;
        let owner = self.script_document_owner(page, &creator)?;
        Ok((page, creator, owner))
    }
    pub(super) fn load_preload_resource(
        &self,
        page: oryn_common::v3::PageId,
        token: u64,
        request: Request,
    ) -> Result<Response, RuntimeError> {
        let key = super::prepared_resource_preload::reuse_key(&request)?;
        let (inner, creator, owner) = self.preload_request_owner(&request)?;
        if page != inner {
            return Err(error("preload lease page mismatch"));
        }
        let lease = self.script_loads.claim_for(token, &owner)?;
        let mut trace = Vec::new();
        let mut hash_evidence = None;
        let response = match request {
            Request::LoadScript {
                fallback_encoding,
                page_id,
                url,
                module,
                nonce,
                integrity,
                cross_origin,
                initiator_url,
                referrer_policy,
                referrer_source,
                script_policy,
                parser_inserted,
                ..
            } => match self.load_page_script_classified_traced(TracedPageScriptInputs {
                page_id,
                requested_url: &url,
                module,
                nonce: nonce.as_deref(),
                initiator_url: initiator_url.as_deref(),
                referrer_policy: referrer_policy.as_deref(),
                integrity: integrity.as_deref(),
                cross_origin: cross_origin.as_deref(),
                referrer_source: referrer_source.as_deref(),
                kind: None,
                cancellation: Some(&lease.cancel),
                creator: Some(&creator),
                script_policy: script_policy.as_deref(),
                parser_inserted,
                fallback_encoding: fallback_encoding.as_deref(),
                trace: Some(&mut trace),
                hash_evidence: Some(&mut hash_evidence),
            }) {
                Ok(response) => response,
                Err(failure) => Response::ScriptLoadFailed { failure },
            },
            Request::LoadPageResource {
                page_id,
                url,
                destination,
                download,
                script_authority,
                initiator,
                form_authority,
                initiator_url,
                referrer_policy,
                parent_encoding,
                post,
            } => self.load_page_resource_traced(
                page_id,
                &url,
                &destination,
                PageResourceOptions {
                    download: download.as_deref(),
                    response_lease: None,
                    script_authority,
                    initiator: &initiator,
                    form_authority,
                    initiator_url: initiator_url.as_deref(),
                    referrer_policy: referrer_policy.as_deref(),
                    parent_encoding: parent_encoding.as_deref(),
                    post,
                },
                &lease.cancel,
                Some(&mut trace),
            )?,
            _ => unreachable!(),
        };
        self.script_document_owner(page, &creator)?;
        let proof = if matches!(
            &response,
            Response::ScriptSource { .. }
                | Response::PageResource {
                    stream_id: None,
                    ..
                }
        ) {
            // Reuse evidence is optional. Its bounded registry must not turn an
            // already successful resource load into a script/style load failure.
            match self
                .preload_authorizations
                .capture_with_hash(owner, key, trace, hash_evidence)
            {
                Ok(proof) => Some(proof),
                Err(RuntimeError::WorkerLimitExceeded { resource })
                    if resource == "preload authorization evidence" =>
                {
                    None
                }
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        Ok(Response::PreloadedResource {
            proof,
            response: Box::new(response),
        })
    }
    pub(super) fn authorize_preload_consumption(
        &self,
        token: &str,
        request: &Request,
    ) -> Result<Response, RuntimeError> {
        let key = super::prepared_resource_preload::reuse_key(request)?;
        let (page, creator, owner) = self.preload_request_owner(request)?;
        let proof = self.preload_authorizations.get(token, &owner, &key)?;
        let (policy, nonce, parser_inserted, destination) = match request {
            Request::LoadScript {
                script_policy: Some(token),
                nonce,
                parser_inserted,
                ..
            } => (
                self.script_policies.get(token, &owner)?,
                nonce.clone(),
                *parser_inserted,
                crate::csp::CspDestination::Script,
            ),
            Request::LoadPageResource { initiator, .. } => {
                let sheet = initiator
                    .stylesheet
                    .as_ref()
                    .ok_or_else(|| error("stylesheet preload options absent"))?;
                let token = sheet
                    .policy
                    .as_deref()
                    .ok_or_else(|| error("stylesheet preload policy absent"))?;
                (
                    self.script_policies.get(token, &owner)?,
                    sheet.nonce.clone(),
                    true,
                    crate::csp::CspDestination::Style,
                )
            }
            _ => return Err(error("preload consumer policy absent")),
        };
        let csp = crate::csp::CspPolicySet::from_agent_policy(&policy);
        let requested = match request {
            Request::LoadScript { url, .. } | Request::LoadPageResource { url, .. } => url,
            _ => unreachable!(),
        };
        let mut expected = url::Url::parse(requested)
            .map_err(|_| error("preload consumer URL must be absolute"))?;
        if let Some(upgraded) = csp.upgraded_url(&expected) {
            expected = upgraded;
        }
        expected.set_fragment(None);
        let mut first =
            url::Url::parse(&proof.urls[0]).map_err(|_| error("preload initial evidence URL"))?;
        first.set_fragment(None);
        if expected != first {
            return Ok(Response::PreloadConsumptionAuthorized { allowed: false });
        }
        if destination == crate::csp::CspDestination::Script
            && csp.sandbox().is_some_and(|sandbox| !sandbox.allow_scripts)
        {
            return Ok(Response::PreloadConsumptionAuthorized { allowed: false });
        }
        let mut violations = Vec::new();
        for (index, hop) in proof.urls.iter().enumerate() {
            let url =
                url::Url::parse(hop).map_err(|_| error("invalid retained redirect evidence"))?;
            if csp
                .upgraded_url(&url)
                .is_some_and(|upgraded| upgraded != url)
            {
                return Ok(Response::PreloadConsumptionAuthorized { allowed: false });
            }
            let decision = csp.authorize_redirect_from(
                &crate::csp::CspRequest {
                    destination,
                    url: hop.clone(),
                    nonce: nonce.clone(),
                    inline_source: None,
                    parser_inserted,
                    initiated_by_trusted_script: destination == crate::csp::CspDestination::Script
                        && !parser_inserted,
                },
                index != 0,
                requested,
            );
            if !decision.allowed {
                return Ok(Response::PreloadConsumptionAuthorized { allowed: false });
            }
            violations.extend(decision.violations);
        }
        self.script_document_owner(page, &creator)?;
        let consumer =
            serde_json::to_vec(request).map_err(|failure| error(&failure.to_string()))?;
        let context = self.context_owner()?;
        let top = context.page_url(page)?;
        // The response-policy publication is one consumer effect, including
        // report-only violations. A lost acknowledgement must not duplicate its
        // events merely because no hash policy/evidence was present.
        proof.publish_consumption_once(&consumer, |evidence| {
            if !violations.is_empty() {
                context.handle_csp_violations_for_creator(
                    page,
                    violations,
                    Some(&csp),
                    Some(creator.clone()),
                )?;
            }
            if destination == crate::csp::CspDestination::Script
                && matches!(&owner, AgentOwner::Window { .. })
                && let Some(evidence) = evidence
            {
                context.deliver_script_hash_reports(
                    Some(page),
                    &top,
                    csp.hash_reports_from_evidence(destination, "script", requested, evidence),
                );
            }
            Ok(())
        })?;
        Ok(Response::PreloadConsumptionAuthorized { allowed: true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{
        NetworkBroker, NetworkError, NetworkRequest, NetworkResponse, SharedNetworkBroker,
    };
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    use oryn_js::worker_wire::Creator;
    struct Redirects;
    impl NetworkBroker for Redirects {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let next = if request.url.contains("root.test") {
                Some("https://middle.test/hop.js")
            } else if request.url.contains("middle.test") {
                Some("https://final.test/end.js")
            } else {
                None
            };
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: if next.is_some() { 302 } else { 200 },
                headers: if let Some(next) = next {
                    vec![("location".into(), next.into())]
                } else {
                    vec![("content-type".into(), "text/javascript".into())]
                },
                body: b"void 0;".to_vec(),
            })
        }
    }
    #[test]
    fn consumption_without_hash_evidence_retains_policy_effect_receipt() {
        let registry = Registry::default();
        let owner = AgentOwner::Window {
            page: oryn_common::v3::PageId(7),
            target: WindowTarget::Root,
            generation: 2,
            epoch: 3,
        };
        let token = registry
            .capture(
                owner.clone(),
                b"style".to_vec(),
                vec!["https://example.test/style.css".into()],
            )
            .unwrap();
        let proof = registry.get(&token, &owner, b"style").unwrap();
        let mut effects = 0;
        proof
            .publish_consumption_once(b"consumer", |evidence| {
                assert!(evidence.is_none());
                effects += 1;
                Ok(())
            })
            .unwrap();
        proof
            .publish_consumption_once(b"consumer", |_| panic!("lost ACK duplicated policy event"))
            .unwrap();
        assert_eq!(effects, 1);
        assert!(
            proof
                .publish_consumption_once(b"other", |_| panic!("foreign consumer published"))
                .is_err()
        );
    }
    #[test]
    fn original_hash_evidence_has_exact_retry_and_foreign_owner_guards() {
        let registry = Registry::default();
        let owner = AgentOwner::Window {
            page: oryn_common::v3::PageId(7),
            target: WindowTarget::Root,
            generation: 2,
            epoch: 3,
        };
        let evidence = crate::csp::ScriptHashEvidence::from_bytes(&[0xff, 0x80], true);
        let token = registry
            .capture_with_hash(
                owner.clone(),
                b"resource".to_vec(),
                vec!["https://example.test/script".into()],
                Some(evidence.clone()),
            )
            .unwrap();
        let proof = registry.get(&token, &owner, b"resource").unwrap();
        let mut attempts = 0;
        assert!(
            proof
                .publish_hash_once(b"consumer-policy", |actual| {
                    assert_eq!(actual, &evidence);
                    attempts += 1;
                    Err(error("queue capacity"))
                })
                .is_err()
        );
        proof
            .publish_hash_once(b"consumer-policy", |actual| {
                assert_eq!(actual, &evidence);
                attempts += 1;
                Ok(())
            })
            .unwrap();
        proof
            .publish_hash_once(b"consumer-policy", |_| panic!("lost ACK must not re-emit"))
            .unwrap();
        assert_eq!(attempts, 2);
        assert!(
            proof
                .publish_hash_once(b"other-consumer", |_| panic!("foreign consumer"))
                .is_err()
        );
        assert!(registry.get(&token, &owner, b"foreign-resource").is_err());
        registry.retire();
        assert!(registry.get(&token, &owner, b"resource").is_err());
    }
    #[test]
    fn parent_proof_authorizes_all_original_hops_and_rejects_forgery_or_retirement() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://root.test/".into(), &[])
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
        let service = ParentContextService::new_with_network(
            context.clone(),
            id,
            Some(SharedNetworkBroker::new(Redirects)),
        );
        let capture = || {
            let Response::CapturedScriptPolicy { authority } = service
                .handle(Request::CaptureScriptPolicy {
                    page_id: id,
                    creator: creator.clone(),
                })
                .unwrap()
            else {
                panic!("policy")
            };
            authority
        };
        let request = |policy: String, parser_inserted| Request::LoadScript {
            fallback_encoding: Some("UTF-8".into()),
            parser_inserted,
            script_policy: Some(policy),
            page_id: id,
            creator: creator.clone(),
            url: "https://root.test/start.js".into(),
            module: false,
            nonce: None,
            integrity: None,
            cross_origin: None,
            initiator_url: Some("https://root.test/".into()),
            referrer_policy: Some("origin".into()),
            referrer_source: None,
        };
        let initial = capture();
        let Response::ScriptLoadReserved { token } = service
            .handle(Request::ReserveScriptLoad {
                page_id: id,
                creator: creator.clone(),
            })
            .unwrap()
        else {
            panic!("lease")
        };
        let original = request(initial.clone(), true);
        let Response::PreloadedResource {
            proof: Some(proof),
            response,
        } = service
            .handle(Request::PreloadResource {
                page_id: id,
                token,
                request: Box::new(original.clone()),
            })
            .unwrap()
        else {
            panic!("proof")
        };
        assert!(matches!(*response, Response::ScriptSource { .. }));
        let owner = service.script_document_owner(id, &creator).unwrap();
        assert_eq!(
            service
                .preload_authorizations
                .get(
                    &proof,
                    &owner,
                    &super::super::prepared_resource_preload::reuse_key(&original).unwrap()
                )
                .unwrap()
                .urls
                .len(),
            3
        );
        let retained = service
            .preload_authorizations
            .get(
                &proof,
                &owner,
                &super::super::prepared_resource_preload::reuse_key(&original).unwrap(),
            )
            .unwrap();
        assert_eq!(
            retained.hash_evidence,
            Some(crate::csp::ScriptHashEvidence::from_bytes(
                b"void 0;", false
            )),
            "opaque redirects retain non-exposure, not a decoded-source hash"
        );
        let consumer = request(initial, false);
        for _ in 0..2 {
            assert!(matches!(
                service
                    .handle(Request::AuthorizePreloadConsumption {
                        proof: proof.clone(),
                        request: Box::new(consumer.clone())
                    })
                    .unwrap(),
                Response::PreloadConsumptionAuthorized { allowed: true }
            ));
        }
        retained
            .publish_hash_once(&serde_json::to_vec(&consumer).unwrap(), |_| {
                panic!("parent consumer receipt was not retained")
            })
            .unwrap();
        // Saturate only optional proof storage. The next successful preload still
        // publishes its response, and existing proof handles stay usable.
        for _ in 1..1024 {
            service
                .preload_authorizations
                .capture(
                    owner.clone(),
                    b"capacity".to_vec(),
                    vec!["https://root.test/capacity".into()],
                )
                .unwrap();
        }
        let Response::ScriptLoadReserved { token } = service
            .handle(Request::ReserveScriptLoad {
                page_id: id,
                creator: creator.clone(),
            })
            .unwrap()
        else {
            panic!("lease")
        };
        let Response::PreloadedResource {
            proof: overflow_proof,
            response,
        } = service
            .handle(Request::PreloadResource {
                page_id: id,
                token,
                request: Box::new(original.clone()),
            })
            .unwrap()
        else {
            panic!("capacity response")
        };
        assert!(overflow_proof.is_none());
        assert!(matches!(*response, Response::ScriptSource { .. }));
        assert!(
            service
                .preload_authorizations
                .get(
                    &proof,
                    &owner,
                    &super::super::prepared_resource_preload::reuse_key(&original).unwrap()
                )
                .is_ok()
        );
        service
            .handle(Request::InstallRealmMetadata {
                page_id: id,
                creator: creator.clone(),
                meta_csp: Some("script-src https://root.test https://final.test".into()),
                meta_referrer: None,
            })
            .unwrap();
        let restricted = request(capture(), false);
        assert!(matches!(
            service
                .handle(Request::AuthorizePreloadConsumption {
                    proof: proof.clone(),
                    request: Box::new(restricted.clone())
                })
                .unwrap(),
            Response::PreloadConsumptionAuthorized { allowed: false }
        ));
        let mut forged = restricted.clone();
        if let Request::LoadScript { url, .. } = &mut forged {
            *url = "https://root.test/other.js".into();
        }
        assert!(
            service
                .handle(Request::AuthorizePreloadConsumption {
                    proof: proof.clone(),
                    request: Box::new(forged)
                })
                .is_err()
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            service
                .handle(Request::AuthorizePreloadConsumption {
                    proof,
                    request: Box::new(restricted)
                })
                .is_err()
        );
    }
    #[test]
    fn opaque_handles_are_registry_and_document_bound() {
        let first = Registry::default();
        let second = Registry::default();
        let owner = AgentOwner::Window {
            page: oryn_common::v3::PageId(1),
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        let token = first
            .capture(
                owner.clone(),
                b"request".to_vec(),
                vec!["https://root.test/".into()],
            )
            .unwrap();
        assert!(second.get(&token, &owner, b"request").is_err());
        assert!(first.get(&token, &owner, b"other").is_err());
        first.retire();
        assert!(first.get(&token, &owner, b"request").is_err());
    }
}
