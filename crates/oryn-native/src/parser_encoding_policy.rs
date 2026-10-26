//! Exact-owner policy snapshots for one tentative parser restart. Opaque tokens
//! are allocated before admission so abandoned responses can still be released.
use super::*;
pub(super) struct Checkpoint {
    pub(super) owner: AgentOwner,
    baseline: Baseline,
    restored: bool,
}
#[derive(Clone)]
enum Baseline {
    Root(
        crate::csp::CspPolicySet,
        Option<crate::referrer::ReferrerPolicy>,
    ),
    Frame(oryn_js::worker_wire::AgentPolicy),
    PrerenderRoot(oryn_js::worker_wire::AgentPolicy),
}
impl BrowserContext {
    pub(crate) fn parser_encoding_policy(
        &self,
        owner: AgentOwner,
        token: &str,
        operation: u8,
    ) -> Result<(), RuntimeError> {
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(RuntimeError::ProtocolViolation(
                "invalid parser encoding capability".into(),
            ));
        }
        let AgentOwner::Window {
            page,
            target,
            generation,
            epoch,
        } = &owner
        else {
            return Err(RuntimeError::ProtocolViolation(
                "parser encoding requires a Document".into(),
            ));
        };
        // Release is idempotent even after retirement; the native socket still binds
        // the page and the stored capability binds the exact old Document owner.
        if operation == 2 {
            let mut policies = self.inner.worker_policies.lock().map_err(|_| {
                RuntimeError::ContextService("parser policy registry poisoned".into())
            })?;
            if policies
                .parser_encoding
                .get(token)
                .is_some_and(|entry| entry.owner != owner)
            {
                return Err(RuntimeError::ProtocolViolation(
                    "foreign parser encoding capability".into(),
                ));
            }
            policies.parser_encoding.remove(token);
            return Ok(());
        }
        self.native_worker_authority(&owner)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let pages = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| RuntimeError::ContextService("parser policy pages poisoned".into()))?;
        if !pages.get(page).is_some_and(|page| page.is_live()) {
            return Err(RuntimeError::PageNotFound(*page));
        }
        // Match activation publication: pages, document policy, referrer,
        // worker policy, then hub. A checkpoint may outlive private promotion.
        let mut documents = self
            .inner
            .document_security
            .lock()
            .map_err(|_| RuntimeError::ContextService("parser security poisoned".into()))?;
        let mut referrers = self
            .inner
            .document_referrer_policy
            .lock()
            .map_err(|_| RuntimeError::ContextService("parser referrer poisoned".into()))?;
        let mut policies =
            self.inner.worker_policies.lock().map_err(|_| {
                RuntimeError::ContextService("parser policy registry poisoned".into())
            })?;
        let hub =
            self.inner.group.messages.lock().map_err(|_| {
                RuntimeError::ContextService("parser policy authority poisoned".into())
            })?;
        let private = hub.prerender_scope_for_document(page.0, target, *generation, *epoch);
        if let Some((token, lifetime)) = &private {
            let scope = policies
                .prerenders
                .get(&(*page, lifetime.0, lifetime.1))
                .filter(|scope| scope.token == *token && !scope.canceling && scope.policy.is_some())
                .ok_or(RuntimeError::NavigationCancelled)?;
            if !policies
                .root_lifetimes
                .get(page)
                .is_some_and(|root| root.current(scope.initiator.0))
            {
                return Err(RuntimeError::NavigationCancelled);
            }
            let registered = hub
                .prerender_target_authority(page.0, token, *lifetime, target, *generation, *epoch)
                .map_err(RuntimeError::ProtocolViolation)?;
            if let oryn_js::message_wire::WindowTarget::Frame(frame) = target {
                let key = (*page, frame.clone(), *generation, *epoch);
                if policies.frame_security_epochs.get(&key).copied() != Some(lifetime.1)
                    || policies.frame_origins.get(&key) != registered.as_ref()
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "retired private parser frame".into(),
                    ));
                }
            }
        } else {
            match target {
                oryn_js::message_wire::WindowTarget::Root => {
                    hub.root_document_authority(page.0, *generation, *epoch)
                        .map_err(RuntimeError::ProtocolViolation)?;
                    if !policies
                        .root_lifetimes
                        .get(page)
                        .is_some_and(|life| life.current(*generation))
                    {
                        return Err(RuntimeError::ProtocolViolation(
                            "retired parser root".into(),
                        ));
                    }
                }
                oryn_js::message_wire::WindowTarget::Frame(frame) => {
                    hub.frame_document_authority(page.0, frame, *generation, *epoch)
                        .map_err(RuntimeError::ProtocolViolation)?;
                    if policies
                        .frame_security_epochs
                        .get(&(*page, frame.clone(), *generation, *epoch))
                        .copied()
                        != Some(
                            policies
                                .root_lifetimes
                                .get(page)
                                .map_or(0, |life| life.epoch),
                        )
                    {
                        return Err(RuntimeError::ProtocolViolation(
                            "retired parser frame".into(),
                        ));
                    }
                }
            }
        }
        match operation {
            0 => {
                if policies.parser_encoding.contains_key(token)
                    || policies
                        .parser_encoding
                        .values()
                        .any(|entry| entry.owner == owner)
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "duplicate parser policy checkpoint".into(),
                    ));
                }
                if policies.parser_encoding.len() >= 4096 {
                    return Err(RuntimeError::WorkerLimitExceeded {
                        resource: "parser encoding checkpoints".into(),
                    });
                }
                let baseline = match target {
                    oryn_js::message_wire::WindowTarget::Root => {
                        if let Some((_, lifetime)) = &private {
                            Baseline::PrerenderRoot(
                                policies.prerenders[&(*page, lifetime.0, lifetime.1)]
                                    .policy
                                    .clone()
                                    .expect("private parser policy validated under lock"),
                            )
                        } else {
                            let security = documents.get(page).cloned().ok_or_else(|| {
                                RuntimeError::ProtocolViolation("parser root policy missing".into())
                            })?;
                            Baseline::Root(security, referrers.get(page).copied())
                        }
                    }
                    oryn_js::message_wire::WindowTarget::Frame(frame) => Baseline::Frame(
                        policies
                            .frames
                            .get(&(*page, frame.clone(), *generation, *epoch))
                            .cloned()
                            .ok_or_else(|| {
                                RuntimeError::ProtocolViolation(
                                    "parser frame policy missing".into(),
                                )
                            })?,
                    ),
                };
                policies.parser_encoding.insert(
                    token.into(),
                    Checkpoint {
                        owner,
                        baseline,
                        restored: false,
                    },
                );
            }
            1 => {
                let entry = policies
                    .parser_encoding
                    .get(token)
                    .filter(|entry| entry.owner == owner)
                    .ok_or_else(|| {
                        RuntimeError::ProtocolViolation("stale parser encoding capability".into())
                    })?;
                if entry.restored {
                    return Ok(());
                }
                let baseline = entry.baseline.clone();
                match baseline {
                    Baseline::Root(security, referrer) => {
                        documents.insert(*page, security);
                        if let Some(referrer) = referrer {
                            referrers.insert(*page, referrer);
                        } else {
                            referrers.remove(page);
                        }
                    }
                    Baseline::PrerenderRoot(policy) => {
                        if let Some((_, lifetime)) = &private {
                            policies
                                .prerenders
                                .get_mut(&(*page, lifetime.0, lifetime.1))
                                .expect("private parser scope validated under lock")
                                .policy = Some(policy);
                        } else {
                            // Activation preserves the exact generation and epoch.
                            // Apply the same policy conversion used by promotion,
                            // now to that current document, never its predecessor.
                            documents.insert(
                                *page,
                                crate::csp::CspPolicySet::from_agent_policy(&policy),
                            );
                            referrers.insert(
                                *page,
                                crate::referrer::ReferrerPolicy::default()
                                    .after_response(&policy.headers),
                            );
                        }
                    }
                    Baseline::Frame(policy) => {
                        let oryn_js::message_wire::WindowTarget::Frame(frame) = target else {
                            return Err(RuntimeError::ProtocolViolation(
                                "parser policy type mismatch".into(),
                            ));
                        };
                        policies
                            .frames
                            .insert((*page, frame.clone(), *generation, *epoch), policy);
                    }
                }
                policies
                    .parser_encoding
                    .get_mut(token)
                    .expect("checkpoint retained under lock")
                    .restored = true;
            }
            _ => {
                return Err(RuntimeError::ProtocolViolation(
                    "invalid parser policy operation".into(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    #[test]
    fn parser_policy_restore_preserves_inherited_baseline_and_is_single_use() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .prepare_document_security(
                id,
                "https://root.test/".into(),
                &[(
                    "Content-Security-Policy".into(),
                    "script-src 'self' 'unsafe-eval'".into(),
                )],
                "",
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
        let owner = AgentOwner::Window {
            page: id,
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let token = "a".repeat(64);
        context
            .parser_encoding_policy(owner.clone(), &token, 0)
            .unwrap();
        context
            .install_realm_metadata(&owner, Some("script-src 'none'"), Some("no-referrer"))
            .unwrap();
        assert!(
            !crate::csp::CspPolicySet::from_agent_policy(
                &context
                    .window_worker_policy(id, &WindowTarget::Root, 1, 0)
                    .unwrap()
            )
            .allows_eval(false)
            .allowed
        );
        context
            .parser_encoding_policy(owner.clone(), &token, 1)
            .unwrap();
        let restored = crate::csp::CspPolicySet::from_agent_policy(
            &context
                .window_worker_policy(id, &WindowTarget::Root, 1, 0)
                .unwrap(),
        );
        assert!(restored.allows_eval(false).allowed);
        assert!(!restored.contains_meta("script-src 'none'"));
        context
            .install_realm_metadata(&owner, Some("script-src 'none'"), None)
            .unwrap();
        context
            .parser_encoding_policy(owner.clone(), &token, 1)
            .unwrap();
        assert!(
            !crate::csp::CspPolicySet::from_agent_policy(
                &context
                    .window_worker_policy(id, &WindowTarget::Root, 1, 0)
                    .unwrap()
            )
            .allows_eval(false)
            .allowed,
            "retry acknowledges original restore without overwriting replayed policy"
        );
        let foreign = AgentOwner::Window {
            page: id,
            target: WindowTarget::Root,
            generation: 2,
            epoch: 0,
        };
        assert!(context.parser_encoding_policy(foreign, &token, 2).is_err());
        context
            .parser_encoding_policy(owner.clone(), &token, 2)
            .unwrap();
        context.parser_encoding_policy(owner, &token, 2).unwrap();
        assert!(
            context
                .inner
                .worker_policies
                .lock()
                .unwrap()
                .parser_encoding
                .is_empty()
        );
    }
    #[test]
    fn parser_checkpoint_rejects_replaced_document_restore_but_accepts_retired_release() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .prepare_document_security(id, "https://root.test/".into(), &[], "")
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
        let owner = AgentOwner::Window {
            page: id,
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let token = "b".repeat(64);
        context
            .parser_encoding_policy(owner.clone(), &token, 0)
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let replacement = AgentOwner::Window {
            page: id,
            target: WindowTarget::Root,
            generation: 2,
            epoch: 0,
        };
        context
            .publish_worker_secure_context(replacement.clone(), true)
            .unwrap();
        assert!(
            context
                .parser_encoding_policy(owner.clone(), &token, 1)
                .is_err()
        );
        assert!(
            context
                .parser_encoding_policy(replacement, &token, 1)
                .is_err()
        );
        context.parser_encoding_policy(owner, &token, 2).unwrap();
        assert!(
            context
                .inner
                .worker_policies
                .lock()
                .unwrap()
                .parser_encoding
                .is_empty()
        );
    }

    #[cfg(unix)]
    fn private_parser_fixture() -> (
        BrowserContext,
        PageId,
        super::super::prerender_policy::Scope,
        AgentOwner,
    ) {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://prerender.test/source".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let source = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let scope = context
            .reserve_native_prerender(page, &source, "https://prerender.test/candidate")
            .unwrap();
        context
            .commit_prerender_response_policy(
                page,
                &scope,
                &scope.url,
                &[
                    (
                        "content-security-policy".into(),
                        "script-src 'self' 'unsafe-eval'".into(),
                    ),
                    ("referrer-policy".into(), "origin".into()),
                ],
            )
            .unwrap();
        let owner = context
            .window_worker_creator(
                page,
                &WindowTarget::Root,
                scope.lifetime.0,
                scope.lifetime.1,
            )
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        (context, page, scope, owner)
    }
    #[cfg(unix)]
    fn owner_policy(
        context: &BrowserContext,
        owner: &AgentOwner,
    ) -> oryn_js::worker_wire::AgentPolicy {
        context.native_worker_authority(owner).unwrap().2
    }
    #[cfg(unix)]
    #[test]
    fn private_parser_checkpoint_restores_own_root_and_frame_without_touching_incumbent() {
        let (context, page, scope, owner) = private_parser_fixture();
        let incumbent = context
            .window_worker_policy(page, &WindowTarget::Root, 1, 0)
            .unwrap();
        let parent = Creator::Window {
            target: WindowTarget::Root,
            document_generation: scope.lifetime.0,
            stream_epoch: scope.lifetime.1,
        };
        let frame = "d".repeat(64);
        context
            .register_native_prerender_frame(crate::runtime::NativePrerenderFrameInputs {
                page,
                parent: &parent,
                frame: frame.clone(),
                generation: 1,
                epoch: 0,
                origin: "https://prerender.test".into(),
                inherit_origin: true,
            })
            .unwrap();
        context
            .inherit_frame_worker_policy_at_url(
                crate::runtime::worker_service::InheritedFramePolicyInputs {
                    page,
                    frame: frame.clone(),
                    generation: 1,
                    epoch: 0,
                    parent: &WindowTarget::Root,
                    parent_generation: scope.lifetime.0,
                    parent_epoch: scope.lifetime.1,
                    document_url: "about:srcdoc",
                },
            )
            .unwrap();
        let child = AgentOwner::Window {
            page,
            target: WindowTarget::Frame(frame),
            generation: 1,
            epoch: 0,
        };
        for (owner, token) in [(owner.clone(), "c".repeat(64)), (child, "d".repeat(64))] {
            let before = owner_policy(&context, &owner);
            context
                .parser_encoding_policy(owner.clone(), &token, 0)
                .unwrap();
            context
                .install_realm_metadata(&owner, Some("script-src 'none'"), Some("no-referrer"))
                .unwrap();
            assert!(
                !crate::csp::CspPolicySet::from_agent_policy(&owner_policy(&context, &owner))
                    .allows_eval(false)
                    .allowed
            );
            context
                .parser_encoding_policy(owner.clone(), &token, 1)
                .unwrap();
            assert_eq!(owner_policy(&context, &owner).headers, before.headers);
            context
                .install_realm_metadata(&owner, Some("img-src 'none'"), None)
                .unwrap();
            context
                .parser_encoding_policy(owner.clone(), &token, 1)
                .unwrap();
            assert!(
                crate::csp::CspPolicySet::from_agent_policy(&owner_policy(&context, &owner))
                    .contains_meta("img-src 'none'")
            );
            let snapshot = owner_policy(&context, &owner);
            let mut same_document = url::Url::parse(&snapshot.url).unwrap();
            same_document.set_fragment(Some("meta-origin-retained"));
            context
                .update_realm_document_url(&owner, same_document.as_str())
                .unwrap();
            let updated = owner_policy(&context, &owner);
            assert_eq!(updated.document_self_url, snapshot.document_self_url);
            assert_eq!(updated.headers, snapshot.headers);
            assert_eq!(updated.policy_self_urls, snapshot.policy_self_urls);
            assert!(
                crate::csp::CspPolicySet::from_agent_policy(&updated)
                    .contains_meta("img-src 'none'")
            );
            assert!(
                context
                    .update_realm_document_url(&owner, "https://foreign.test/changed")
                    .is_err()
            );
            assert_eq!(owner_policy(&context, &owner), updated);
            context.parser_encoding_policy(owner, &token, 2).unwrap();
        }
        assert_eq!(
            context
                .window_worker_policy(page, &WindowTarget::Root, 1, 0)
                .unwrap()
                .headers,
            incumbent.headers
        );
        assert_eq!(
            context
                .inner
                .group
                .messages
                .lock()
                .unwrap()
                .root_document_identity(page.0),
            Some((1, 0))
        );
        let token = "e".repeat(64);
        context
            .parser_encoding_policy(owner.clone(), &token, 0)
            .unwrap();
        context
            .cancel_native_prerender(page, &scope.token, scope.lifetime)
            .unwrap();
        assert!(
            context
                .parser_encoding_policy(owner.clone(), &token, 1)
                .is_err()
        );
        context
            .parser_encoding_policy(owner.clone(), &token, 2)
            .unwrap();
        context.parser_encoding_policy(owner, &token, 2).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn private_parser_checkpoint_follows_exact_document_promotion_before_body_arrives() {
        let (context, page, scope, owner) = private_parser_fixture();
        let before = owner_policy(&context, &owner);
        let token = "f".repeat(64);
        context
            .parser_encoding_policy(owner.clone(), &token, 0)
            .unwrap();
        context
            .install_realm_metadata(&owner, Some("script-src 'none'"), Some("no-referrer"))
            .unwrap();
        context
            .mark_native_prerender_ready(page, &scope.token, scope.lifetime)
            .unwrap();
        let predecessor = context
            .inner
            .group
            .messages
            .lock()
            .unwrap()
            .park_document_authority(page.0, 1, 0)
            .unwrap();
        context
            .activate_native_prerender(page, &scope.token, scope.lifetime, &predecessor)
            .unwrap();
        context
            .parser_encoding_policy(owner.clone(), &token, 1)
            .unwrap();
        let restored = owner_policy(&context, &owner);
        assert!(
            crate::csp::CspPolicySet::from_agent_policy(&restored)
                .allows_eval(false)
                .allowed
        );
        assert!(
            !crate::csp::CspPolicySet::from_agent_policy(&restored)
                .contains_meta("script-src 'none'")
        );
        assert_eq!(
            crate::referrer::ReferrerPolicy::default().after_response(&restored.headers),
            crate::referrer::ReferrerPolicy::default().after_response(&before.headers)
        );
        assert_eq!(
            context
                .inner
                .group
                .messages
                .lock()
                .unwrap()
                .root_document_identity(page.0),
            Some(scope.lifetime)
        );
        context.parser_encoding_policy(owner, &token, 2).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn private_compatibility_poll_preserves_incumbent_events_and_rejects_revoked_scope() {
        let (context, page, scope, _) = private_parser_fixture();
        let candidate = Creator::Window {
            target: WindowTarget::Root,
            document_generation: scope.lifetime.0,
            stream_epoch: scope.lifetime.1,
        };
        let source = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        context.record_context_diagnostic(crate::context_service::ContextEvent::CspReport {
            target_page_id: page,
            endpoint: "https://prerender.test/report".into(),
            format: crate::csp::CspReportFormat::LegacyReportUri,
            outcome: "incumbent diagnostic".into(),
        });
        assert!(matches!(
            context
                .native_platform_query(
                    page,
                    candidate.clone(),
                    oryn_js::platform_control::Request::WindowState
                )
                .unwrap(),
            oryn_js::platform_control::Response::WindowState { hidden: true, .. }
        ));
        assert!(
            context
                .poll_context_events(page, &candidate, 0, 32)
                .unwrap()
                .is_empty()
        );
        let events = context.poll_context_events(page, &source, 0, 32).unwrap();
        assert!(events.iter().any(|record| matches!(&record.event, crate::context_service::ContextEvent::CspReport { outcome, .. } if outcome == "incumbent diagnostic")));
        let violation = crate::csp::CspViolation {
            disposition: crate::csp::CspDisposition::Enforce,
            effective_directive: "script-src".into(),
            blocked_uri: "inline".into(),
            worker_event_blocked_uri: None,
            document_uri: scope.url.clone(),
            source: "inline".into(),
            original_policy: "script-src 'none'".into(),
            referrer: String::new(),
            source_file: String::new(),
            sample: String::new(),
            status_code: 200,
            line_number: 0,
            column_number: 0,
        };
        context
            .publish_context_event_batch(vec![crate::context_service::ContextEvent::CspViolation {
                target_page_id: page,
                target_creator: Some(candidate.clone()),
                violation,
            }])
            .unwrap();
        context
            .native_platform_query(
                page,
                source.clone(),
                oryn_js::platform_control::Request::Storage {
                    local: true,
                    operation: oryn_js::platform_control::WindowStorageOperation::Set {
                        key: "incumbent".into(),
                        value: "mutation".into(),
                    },
                },
            )
            .unwrap();
        let source_events = context
            .native_platform_query(
                page,
                source.clone(),
                oryn_js::platform_control::Request::CspEvents { after: 0 },
            )
            .unwrap();
        assert!(
            matches!(source_events, oryn_js::platform_control::Response::CspEvents { events } if events.is_empty())
        );
        let private_events = context
            .native_platform_query(
                page,
                candidate.clone(),
                oryn_js::platform_control::Request::CspEvents { after: 0 },
            )
            .unwrap();
        let oryn_js::platform_control::Response::CspEvents { events } = private_events else {
            panic!("private CSP events");
        };
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].document_uri, scope.url);
        assert!(
            matches!(context.native_platform_query(page, candidate.clone(), oryn_js::platform_control::Request::CspEvents { after: events[0].id }).unwrap(), oryn_js::platform_control::Response::CspEvents { events } if events.is_empty())
        );
        let forged = Creator::Window {
            target: WindowTarget::Root,
            document_generation: scope.lifetime.0,
            stream_epoch: scope.lifetime.1 + 1,
        };
        assert!(context.poll_context_events(page, &forged, 0, 32).is_err());
        context
            .cancel_native_prerender(page, &scope.token, scope.lifetime)
            .unwrap();
        assert!(
            context
                .poll_context_events(page, &candidate, 0, 32)
                .is_err()
        );
        assert!(matches!(
            context
                .native_platform_query(
                    page,
                    candidate,
                    oryn_js::platform_control::Request::WindowState
                )
                .unwrap(),
            oryn_js::platform_control::Response::WindowRetired
        ));
        assert!(context.poll_context_events(page, &source, 0, 32).is_ok());
    }
}
