//! Parent-only response grants owned by one worker route. Callers serialize
//! every operation, including commit and revocation, with RouteAuthority's lock.
//! Receiving headers does not publish policy or advance a document lifetime.

use std::collections::{BTreeMap, VecDeque};

use sha2::{Digest, Sha256};

use crate::{agent_registry::AgentOwner, runtime::RuntimeError};
use oryn_common::v3::PageId;

const CAPACITY: usize = 64;
const REMOVAL_HISTORY_CAPACITY: usize = 64;

#[derive(Clone, Copy)]
enum RemovalReason {
    Consumed,
    Discarded,
    NetworkTokenDiscarded,
    ReservationAbandoned,
    Cancelled,
    NonDocument,
    RouteRevoked,
}

impl RemovalReason {
    fn label(self) -> &'static str {
        match self {
            Self::Consumed => "consumed",
            Self::Discarded => "context_token_discarded",
            Self::NetworkTokenDiscarded => "network_token_discarded",
            Self::ReservationAbandoned => "reservation_abandoned",
            Self::Cancelled => "request_cancelled",
            Self::NonDocument => "non_document",
            Self::RouteRevoked => "route_revoked",
        }
    }
}

// Diagnostic history never retains authority, response data, or frame tokens.
// It does not participate in admission, validation, cancellation, or replay.
struct Removal {
    fingerprint: [u8; 32],
    request_id: u64,
    page: PageId,
    frame: bool,
    generation: u64,
    epoch: u64,
    headers_admitted: bool,
    reason: RemovalReason,
}

fn fingerprint(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

struct Response {
    url: String,
    headers: Vec<(String, String)>,
}

struct Grant {
    request_id: u64,
    source: AgentOwner,
    response: Option<Response>,
}

#[derive(Default)]
pub(super) struct Registry {
    retired: bool,
    grants: BTreeMap<String, Grant>,
    removals: VecDeque<Removal>,
}

fn invalid(message: &str) -> RuntimeError {
    RuntimeError::ProtocolViolation(message.into())
}

impl Registry {
    fn remove(&mut self, token: &str, reason: RemovalReason) -> Option<Grant> {
        let grant = self.grants.remove(token)?;
        if let AgentOwner::Window {
            page,
            target,
            generation,
            epoch,
        } = &grant.source
        {
            if self.removals.len() == REMOVAL_HISTORY_CAPACITY {
                self.removals.pop_front();
            }
            self.removals.push_back(Removal {
                fingerprint: fingerprint(token),
                request_id: grant.request_id,
                page: *page,
                frame: matches!(target, oryn_js::message_wire::WindowTarget::Frame(_)),
                generation: *generation,
                epoch: *epoch,
                headers_admitted: grant.response.is_some(),
                reason,
            });
        }
        Some(grant)
    }

    fn unknown(&self, token: &str, phase: &str) -> RuntimeError {
        let digest = fingerprint(token);
        let hex = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let history = match self
            .removals
            .iter()
            .rev()
            .find(|item| item.fingerprint == digest)
        {
            Some(item) => format!(
                "reason={} request_id={} source_page={} source_target={} source_generation={} source_epoch={} headers_admitted={}",
                item.reason.label(),
                item.request_id,
                item.page.0,
                if item.frame { "frame" } else { "root" },
                item.generation,
                item.epoch,
                item.headers_admitted,
            ),
            // A different live route and an evicted record are both possible.
            // Absence of local history does not prove the token was never issued.
            None => "reason=unknown_or_unretained".into(),
        };
        invalid(&format!(
            "unknown navigation response grant: phase={phase} token_sha256={hex} {history}"
        ))
    }

    /// Reserve before dispatch with a source captured by the native parent.
    /// Request IDs identify cancellation only; they are never commit authority.
    pub(super) fn begin(
        &mut self,
        request_id: u64,
        source: AgentOwner,
    ) -> Result<String, RuntimeError> {
        if self.retired {
            return Err(invalid("response policy route retired"));
        }
        if !matches!(source, AgentOwner::Window { .. }) {
            return Err(invalid("document response requires a Window source"));
        }
        if self.grants.len() >= CAPACITY {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "navigation response grants".into(),
            });
        }
        // Retain the ID until commit or explicit cancellation, even if the body
        // completes first. An ambiguous late cancellation cannot target a new
        // response under an outstanding predecessor's numeric request ID.
        if self
            .grants
            .values()
            .any(|grant| grant.request_id == request_id)
        {
            return Err(invalid("navigation response request ID still owned"));
        }
        let mut random = [0u8; 32];
        getrandom::fill(&mut random)
            .map_err(|_| invalid("navigation response entropy unavailable"))?;
        let token = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if self.grants.contains_key(&token) {
            return Err(invalid("navigation response token collision"));
        }
        self.grants.insert(
            token.clone(),
            Grant {
                request_id,
                source,
                response: None,
            },
        );
        Ok(token)
    }

    /// Only the parent network broker supplies these response bytes. Redirects
    /// never produce a document grant. Header order and duplicates are retained.
    pub(super) fn headers(
        &mut self,
        token: &str,
        status: u16,
        url: String,
        headers: Vec<(String, String)>,
    ) -> Result<(), RuntimeError> {
        if self.retired {
            return Err(invalid("response policy route retired"));
        }
        if !crate::frame_embedding::creates_document(status, &headers) {
            self.remove(token, RemovalReason::NonDocument);
            return Err(invalid("response does not create a document"));
        }
        url::Url::parse(&url).map_err(|_| invalid("invalid response policy URL"))?;
        if !self.grants.contains_key(token) {
            return Err(self.unknown(token, "headers"));
        }
        let grant = self.grants.get_mut(token).unwrap();
        if grant.response.is_some() {
            return Err(invalid("navigation response headers already admitted"));
        }
        grant.response = Some(Response { url, headers });
        Ok(())
    }

    /// Consume once, after the worker has preserved the departing document.
    /// The installation callback must validate the captured source against the
    /// current native lifetime (or its parked snapshot) before publishing policy.
    /// A failed native commit consumes its grant, so it cannot later be replayed
    /// against another document. A mismatched caller cannot consume a grant.
    pub(super) fn commit(
        &mut self,
        page: PageId,
        token: &str,
        url: &str,
        headers: &[(String, String)],
        install: impl FnOnce(&AgentOwner, String, &[(String, String)]) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        if self.retired {
            return Err(invalid("response policy route retired"));
        }
        let grant = self
            .grants
            .get(token)
            .ok_or_else(|| self.unknown(token, "commit"))?;
        if !matches!(&grant.source, AgentOwner::Window { page: owner, .. } if *owner == page) {
            return Err(invalid("foreign navigation response page"));
        }
        let response = grant
            .response
            .as_ref()
            .ok_or_else(|| invalid("navigation response headers not admitted"))?;
        if response.url != url
            || crate::network::sanitize_parent_response_headers(response.headers.clone()) != headers
        {
            return Err(invalid("navigation response payload mismatch"));
        }
        let grant = self.remove(token, RemovalReason::Consumed).unwrap();
        let response = grant.response.unwrap();
        install(&grant.source, response.url, &response.headers)
    }

    /// Token-based cleanup remains safe after the numeric ID has been reused.
    pub(super) fn discard(&mut self, token: &str) {
        self.remove(token, RemovalReason::Discarded);
    }

    pub(super) fn discard_network_token(&mut self, token: &str) {
        self.remove(token, RemovalReason::NetworkTokenDiscarded);
    }

    pub(super) fn abandon_reservation(&mut self, token: &str) {
        self.remove(token, RemovalReason::ReservationAbandoned);
    }

    pub(super) fn cancel_request(&mut self, request_id: u64) {
        let tokens = self
            .grants
            .iter()
            .filter(|(_, grant)| grant.request_id == request_id)
            .map(|(token, _)| token.clone())
            .collect::<Vec<_>>();
        for token in tokens {
            self.remove(&token, RemovalReason::Cancelled);
        }
    }

    pub(super) fn revoke(&mut self) {
        self.retired = true;
        while let Some((token, _)) = self.grants.first_key_value() {
            let token = token.clone();
            self.remove(&token, RemovalReason::RouteRevoked);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::message_wire::WindowTarget;

    const URL: &str = "https://example.test/document";

    fn source(page: u64, generation: u64) -> AgentOwner {
        AgentOwner::Window {
            page: PageId(page),
            target: WindowTarget::Root,
            generation,
            epoch: 0,
        }
    }

    fn headers() -> Vec<(String, String)> {
        vec![
            ("Content-Security-Policy".into(), "script-src 'none'".into()),
            (
                "Content-Security-Policy".into(),
                "default-src 'self'".into(),
            ),
        ]
    }

    fn ready(registry: &mut Registry, id: u64) -> String {
        let token = registry.begin(id, source(1, 7)).unwrap();
        registry
            .headers(&token, 200, URL.into(), headers())
            .unwrap();
        token
    }

    fn rejected_commit(registry: &mut Registry, token: &str) -> String {
        registry
            .commit(PageId(1), token, URL, &headers(), |_, _, _| {
                panic!("removed grant reached policy installation")
            })
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn missing_grant_diagnostics_preserve_first_removal_and_source_without_authority() {
        let mut registry = Registry::default();
        for (id, reason) in [
            (1, "context_token_discarded"),
            (2, "reservation_abandoned"),
            (3, "request_cancelled"),
            (4, "non_document"),
        ] {
            let token = registry
                .begin(
                    id,
                    AgentOwner::Window {
                        page: PageId(1),
                        target: WindowTarget::Frame("private-frame-authority".into()),
                        generation: 17,
                        epoch: 23,
                    },
                )
                .unwrap();
            registry
                .headers(&token, 200, URL.into(), headers())
                .unwrap();
            match id {
                1 => registry.discard(&token),
                2 => registry.abandon_reservation(&token),
                3 => registry.cancel_request(id),
                4 => assert!(
                    registry
                        .headers(&token, 204, URL.into(), headers())
                        .is_err()
                ),
                _ => unreachable!(),
            }
            // Late cleanup must not overwrite the actual removal cause.
            registry.discard(&token);
            registry.abandon_reservation(&token);
            registry.cancel_request(id);
            let error = rejected_commit(&mut registry, &token);
            assert!(
                error.contains("unknown navigation response grant: phase=commit token_sha256=")
            );
            assert!(error.contains(&format!("reason={reason} request_id={id} source_page=1 source_target=frame source_generation=17 source_epoch=23 headers_admitted=true")), "{error}");
            assert!(!error.contains(&token));
            assert!(!error.contains("private-frame-authority"));
            assert!(!error.contains(URL));
            assert!(!error.contains("script-src"));
            let header_error = registry
                .headers(&token, 200, URL.into(), headers())
                .unwrap_err()
                .to_string();
            assert!(header_error.contains("phase=headers"));
            assert!(header_error.contains(&format!("reason={reason}")));
        }
    }

    #[test]
    fn diagnostic_history_is_bounded_and_does_not_authorize_evicted_or_foreign_grants() {
        let mut registry = Registry::default();
        let first = ready(&mut registry, 1);
        registry.discard(&first);
        for id in 2..=(REMOVAL_HISTORY_CAPACITY as u64 + 1) {
            let token = ready(&mut registry, id);
            registry.cancel_request(id);
        }
        assert_eq!(registry.removals.len(), REMOVAL_HISTORY_CAPACITY);
        assert!(rejected_commit(&mut registry, &first).contains("reason=unknown_or_unretained"));
        let live = ready(&mut registry, 1);
        let mut other_route = Registry::default();
        assert!(rejected_commit(&mut other_route, &live).contains("reason=unknown_or_unretained"));
        assert!(registry.grants.contains_key(&live));
        registry.revoke();
        assert_eq!(registry.removals.len(), REMOVAL_HISTORY_CAPACITY);
        assert_eq!(
            registry.removals.back().unwrap().reason.label(),
            "route_revoked"
        );
        assert!(registry.grants.is_empty());
        assert!(rejected_commit(&mut registry, &live).contains("response policy route retired"));
        assert!(registry.begin(1, source(1, 7)).is_err());
    }

    #[test]
    fn consumed_grant_diagnostics_preserve_single_use_after_success_and_failed_install() {
        let mut registry = Registry::default();
        for fail in [false, true] {
            let token = ready(&mut registry, 41);
            let result = registry.commit(PageId(1), &token, URL, &headers(), |_, _, _| {
                if fail {
                    Err(invalid("native lifetime rejected"))
                } else {
                    Ok(())
                }
            });
            assert_eq!(result.is_err(), fail);
            registry.discard(&token);
            registry.cancel_request(41);
            let successor = ready(&mut registry, 41);
            let error = rejected_commit(&mut registry, &token);
            assert!(error.contains("reason=consumed request_id=41"), "{error}");
            assert!(!error.contains(&token));
            registry
                .commit(PageId(1), &successor, URL, &headers(), |owner, _, _| {
                    assert_eq!(owner, &source(1, 7));
                    Ok(())
                })
                .unwrap();
        }
    }

    #[test]
    fn headers_stage_without_installing_and_commit_once_with_captured_source() {
        let mut registry = Registry::default();
        let token = ready(&mut registry, 1);
        let mut installed = None;
        registry
            .commit(PageId(1), &token, URL, &headers(), |owner, url, values| {
                installed = Some((owner.clone(), url, values.to_vec()));
                Ok(())
            })
            .unwrap();
        assert_eq!(installed, Some((source(1, 7), URL.into(), headers())));
        assert!(
            registry
                .commit(PageId(1), &token, URL, &headers(), |_, _, _| {
                    panic!("replayed grant reached installation")
                })
                .is_err()
        );
        assert!(registry.grants.is_empty());
    }

    #[test]
    fn foreign_page_route_and_substituted_payload_cannot_consume_grant() {
        let mut registry = Registry::default();
        let token = ready(&mut registry, 1);
        let mut other_route = Registry::default();
        assert!(
            other_route
                .commit(PageId(1), &token, URL, &headers(), |_, _, _| {
                    panic!("foreign route reached installation")
                })
                .is_err()
        );
        let mut reordered = headers();
        reordered.reverse();
        for (page, url, values) in [
            (PageId(2), URL, headers()),
            (PageId(1), "https://example.test/substituted", headers()),
            (PageId(1), URL, reordered),
            (PageId(1), URL, Vec::new()),
        ] {
            assert!(
                registry
                    .commit(page, &token, url, &values, |_, _, _| {
                        panic!("mismatched payload reached installation")
                    })
                    .is_err()
            );
            assert!(registry.grants.contains_key(&token));
        }
        registry
            .commit(PageId(1), &token, URL, &headers(), |_, _, _| Ok(()))
            .unwrap();
    }

    #[test]
    fn native_source_rejection_consumes_grant_without_retry() {
        let mut registry = Registry::default();
        let token = ready(&mut registry, 1);
        assert!(
            registry
                .commit(PageId(1), &token, URL, &headers(), |owner, _, _| {
                    assert_eq!(owner, &source(1, 7));
                    Err(invalid("source retired"))
                })
                .is_err()
        );
        assert!(!registry.grants.contains_key(&token));
    }

    #[test]
    fn private_headers_stay_native_while_visible_header_order_authenticates_commit() {
        let mut registry = Registry::default();
        let token = registry.begin(1, source(1, 7)).unwrap();
        let mut native = headers();
        native.insert(1, ("Set-Cookie".into(), "session=private; HttpOnly".into()));
        registry
            .headers(&token, 200, URL.into(), native.clone())
            .unwrap();
        registry
            .commit(PageId(1), &token, URL, &headers(), |_, _, values| {
                assert_eq!(values, native);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn incomplete_redirect_and_no_content_responses_cannot_commit() {
        let mut registry = Registry::default();
        let token = registry.begin(1, source(1, 7)).unwrap();
        assert!(
            registry
                .commit(PageId(1), &token, URL, &headers(), |_, _, _| {
                    panic!("response without headers reached installation")
                })
                .is_err()
        );
        registry.discard(&token);
        for status in [301, 302, 303, 307, 308, 204, 205] {
            let token = registry.begin(1, source(1, 7)).unwrap();
            assert!(
                registry
                    .headers(&token, status, URL.into(), headers())
                    .is_err()
            );
            assert!(registry.grants.is_empty());
        }
    }

    #[test]
    fn duplicate_headers_cannot_replace_the_admitted_policy() {
        let mut registry = Registry::default();
        let token = ready(&mut registry, 1);
        assert!(
            registry
                .headers(&token, 200, URL.into(), Vec::new())
                .is_err()
        );
        registry
            .commit(PageId(1), &token, URL, &headers(), |_, _, values| {
                assert_eq!(values, headers());
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn cancellation_and_late_token_cleanup_do_not_revoke_successor() {
        let mut registry = Registry::default();
        let first = ready(&mut registry, 1);
        assert!(registry.begin(1, source(1, 8)).is_err());
        registry.cancel_request(1);
        let next = ready(&mut registry, 1);
        assert_ne!(first, next);
        registry.discard(&first);
        assert!(
            registry
                .commit(PageId(1), &first, URL, &headers(), |_, _, _| {
                    panic!("cancelled grant reached installation")
                })
                .is_err()
        );
        registry
            .commit(PageId(1), &next, URL, &headers(), |_, _, _| Ok(()))
            .unwrap();
    }

    #[test]
    fn capacity_and_retirement_bound_retained_response_authority() {
        let mut registry = Registry::default();
        for id in 0..CAPACITY {
            ready(&mut registry, id as u64);
        }
        assert!(registry.begin(CAPACITY as u64, source(1, 7)).is_err());
        registry.revoke();
        assert!(registry.grants.is_empty());
        assert!(registry.begin(0, source(1, 7)).is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_initial_document_can_be_parked_after_headers_before_policy_commit() {
        let browser = crate::Browser::in_process_probe();
        let context = browser.new_context();
        let page = context.new_page();
        let id = page.page_id();
        let source = context.capture_navigation_response_source(id).unwrap();
        let mut registry = Registry::default();
        let token = registry.begin(1, source).unwrap();
        registry
            .headers(&token, 200, URL.into(), headers())
            .unwrap();
        assert!(context.native_initial_document_creator(id).is_ok());
        let parked = context.park_initial_document(id).unwrap();
        registry
            .commit(id, &token, URL, &headers(), |owner, url, values| {
                context.commit_navigation_response_policy(id, owner, url, values)
            })
            .unwrap();
        context
            .verify_document_security(id, URL, &headers())
            .unwrap();
        assert!(context.native_initial_document_creator(id).is_err());
        context.finish_initial_document(id, parked, false).unwrap();
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_replacement_rejects_older_response_without_overwriting_successor() {
        let browser = crate::Browser::in_process_probe();
        let context = browser.new_context();
        let page = context.new_page();
        let id = page.page_id();
        let source = context.capture_navigation_response_source(id).unwrap();
        let mut registry = Registry::default();
        let token = registry.begin(1, source).unwrap();
        registry
            .headers(&token, 200, URL.into(), headers())
            .unwrap();
        let parked = context.park_initial_document(id).unwrap();
        let next = "https://successor.test/";
        context
            .install_document_security(id, next.into(), &[])
            .unwrap();
        assert!(
            registry
                .commit(id, &token, URL, &headers(), |owner, url, values| {
                    context.commit_navigation_response_policy(id, owner, url, values)
                })
                .is_err()
        );
        assert!(registry.grants.is_empty());
        context.verify_document_security(id, next, &[]).unwrap();
        context.finish_initial_document(id, parked, false).unwrap();
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_closed_page_cannot_commit_a_retained_response() {
        let browser = crate::Browser::in_process_probe();
        let context = browser.new_context();
        let page = context.new_page();
        let id = page.page_id();
        let source = context.capture_navigation_response_source(id).unwrap();
        let mut registry = Registry::default();
        let token = registry.begin(1, source).unwrap();
        registry
            .headers(&token, 200, URL.into(), headers())
            .unwrap();
        page.close().await.unwrap();
        assert!(
            registry
                .commit(id, &token, URL, &headers(), |owner, url, values| {
                    context.commit_navigation_response_policy(id, owner, url, values)
                })
                .is_err()
        );
        assert!(registry.grants.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cached_predecessor_policy_survives_headers_and_restores_after_failed_install() {
        use oryn_js::message_wire::{MessageRequest, MessageResponse};
        let browser = crate::Browser::in_process_probe();
        let context = browser.new_context();
        let page = context.new_page();
        let id = page.page_id();
        let source_url = "https://source.test/";
        let source_headers = vec![
            ("Content-Security-Policy".into(), "script-src 'self'".into()),
            ("Referrer-Policy".into(), "no-referrer".into()),
        ];
        context
            .install_document_security(id, source_url.into(), &source_headers)
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
        context.commit_page_url(id, source_url.into()).unwrap();
        let source = context.capture_navigation_response_source(id).unwrap();
        let mut registry = Registry::default();
        let token = registry.begin(1, source).unwrap();
        registry
            .headers(&token, 200, URL.into(), headers())
            .unwrap();
        context
            .verify_document_security(id, source_url, &source_headers)
            .unwrap();
        let parked = match context
            .handle_message_wire(
                id,
                MessageRequest::ParkDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap()
        {
            MessageResponse::DocumentParked(token) => token,
            response => panic!("expected native source preservation: {response:?}"),
        };
        registry
            .commit(id, &token, URL, &headers(), |owner, url, values| {
                context.commit_navigation_response_policy(id, owner, url, values)
            })
            .unwrap();
        context
            .verify_document_security(id, URL, &headers())
            .unwrap();
        // Model failure before destination realm installation. The original
        // native snapshot, not worker-supplied headers, restores its policy.
        context
            .handle_message_wire(id, MessageRequest::RestoreDocument { token: parked })
            .unwrap();
        context
            .verify_document_security(id, source_url, &source_headers)
            .unwrap();
        assert!(
            registry
                .commit(id, &token, URL, &headers(), |_, _, _| {
                    panic!("consumed response grant replaced restored source")
                })
                .is_err()
        );
        page.close().await.unwrap();
    }
}
