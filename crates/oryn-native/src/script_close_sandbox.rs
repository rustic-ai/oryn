//! Window.close uses the TARGET active Document's source snapshot, while the
//! incumbent navigable remains the source for the sandbox ancestry algorithm.
use super::*;
use crate::agent_registry::AgentOwner;
use oryn_js::{message_wire::WindowTarget, worker_wire::Creator};

impl BrowserContext {
    pub(crate) fn script_close_sandbox_allowed(
        &self,
        source_page: PageId,
        source: &Creator,
        target_page: PageId,
        target: &Creator,
    ) -> Result<bool, RuntimeError> {
        let Creator::Window {
            target: source_target,
            document_generation: sg,
            stream_epoch: se,
        } = source
        else {
            return Err(RuntimeError::ProtocolViolation(
                "script-close source must be a Window".into(),
            ));
        };
        let Creator::Window {
            target: WindowTarget::Root,
            document_generation: tg,
            stream_epoch: te,
        } = target
        else {
            return Err(RuntimeError::ProtocolViolation(
                "script-close target must be a root Window".into(),
            ));
        };
        let (source_owner, permitted) = self.with_window_worker_creator_pages_checked(
            crate::runtime::CheckedWorkerCreatorPagesInputs {
                page_id: source_page,
                target: source_target,
                generation: *sg,
                epoch: *se,
                include_urls: false,
                allow_prerender: false,
                checked_root: Some((target_page, target)),
            },
            |authority, _, _, pages| {
                let permitted = pages.get(&target_page).is_some_and(|record| {
                    record.sandbox_navigator.as_ref() == Some(&(source_page, source_target.clone()))
                });
                Ok((authority.0, permitted))
            },
        )?;
        let target_owner = AgentOwner::Window {
            page: target_page,
            target: WindowTarget::Root,
            generation: *tg,
            epoch: *te,
        };
        // Snapshot the destination, not the incumbent. No sticky activation or
        // same-origin-domain frame-busting rule belongs in this algorithm.
        let flags = self
            .inner
            .document_security
            .lock()
            .map_err(|_| {
                RuntimeError::ContextService("script-close target policy poisoned".into())
            })?
            .get(&target_page)
            .ok_or(RuntimeError::PageNotFound(target_page))?
            .sandbox();
        let active = self
            .transient_native_activation(&target_owner, false)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let parents = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("script-close ancestry poisoned".into()))?
            .activation_parents
            .clone();
        let mut current = source_owner.clone();
        let mut seen = BTreeSet::new();
        let mut target_ancestor = false;
        while let Some(parent) = parents.get(&current) {
            if !seen.insert(current.clone()) || seen.len() > 4096 {
                return Err(RuntimeError::ProtocolViolation(
                    "script-close ancestry cycle or capacity".into(),
                ));
            }
            let AgentOwner::Window {
                page,
                target,
                generation,
                epoch,
            } = parent
            else {
                return Err(RuntimeError::ProtocolViolation(
                    "script-close non-Window ancestor".into(),
                ));
            };
            self.with_window_worker_creator_pages(
                crate::runtime::WorkerCreatorPagesInputs {
                    page_id: *page,
                    target,
                    generation: *generation,
                    epoch: *epoch,
                    include_urls: false,
                    allow_prerender: false,
                },
                |_, _, _, _| Ok(()),
            )?;
            if parent == &target_owner {
                target_ancestor = true;
                break;
            }
            current = parent.clone();
        }
        let allowed = if source_owner == target_owner {
            true
        } else if target_ancestor {
            flags.as_ref().is_none_or(|flags| {
                flags.allow_top_navigation
                    || (active && flags.allow_top_navigation_by_user_activation)
            })
        } else {
            permitted || flags.is_none()
        };
        self.with_window_worker_creator_pages_checked(
            crate::runtime::CheckedWorkerCreatorPagesInputs {
                page_id: source_page,
                target: source_target,
                generation: *sg,
                epoch: *se,
                include_urls: false,
                allow_prerender: false,
                checked_root: Some((target_page, target)),
            },
            |_, _, _, _| Ok(allowed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::message_wire::MessageRequest;
    fn root(context: &BrowserContext, page: PageId, policy: &str, generation: u64) -> Creator {
        context
            .install_document_security(
                page,
                "https://root.test/".into(),
                &[("content-security-policy".into(), policy.into())],
            )
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: generation,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        Creator::Window {
            target: WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 0,
        }
    }
    fn child(context: &BrowserContext, page: PageId, parent: &Creator, policy: &str) -> Creator {
        let frame = "b".repeat(64);
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 1,
                    stream_epoch: 0,
                    origin: "https://child.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let token = context
            .prepare_frame_worker_policy(
                page,
                "https://child.test/",
                &[("content-security-policy".into(), policy.into())],
                "",
            )
            .unwrap();
        context
            .bind_frame_worker_policy(page, frame.clone(), 1, 0, "https://child.test/", &token)
            .unwrap();
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = parent
        else {
            unreachable!()
        };
        context
            .register_activation_parent(crate::runtime::worker_service::ActivationParentInputs {
                page,
                frame: frame.clone(),
                generation: 1,
                epoch: 0,
                parent: target.clone(),
                parent_generation: *document_generation,
                parent_epoch: *stream_epoch,
            })
            .unwrap();
        Creator::Window {
            target: WindowTarget::Frame(frame),
            document_generation: 1,
            stream_epoch: 0,
        }
    }
    #[test]
    fn target_policy_controls_close_and_unsandboxed_target_does_not_require_framebusting_activation()
     {
        for (target_policy, source_policy, allowed) in [
            ("", "sandbox allow-same-origin", true),
            ("sandbox allow-same-origin", "", false),
            ("sandbox allow-same-origin allow-top-navigation", "", true),
        ] {
            let context = crate::Browser::new().new_context();
            let page = context.new_page().page_id();
            let target = root(&context, page, target_policy, 1);
            let source = child(&context, page, &target, source_policy);
            assert_eq!(
                context
                    .script_close_sandbox_allowed(page, &source, page, &target)
                    .unwrap(),
                allowed
            );
            assert!(
                context
                    .script_close_sandbox_allowed(page, &target, page, &target)
                    .unwrap(),
                "same navigable short circuit survives target sandbox"
            );
        }
    }
    #[test]
    fn transient_activation_is_snapshotted_from_target_and_not_consumed() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page().page_id();
        let target = root(
            &context,
            page,
            "sandbox allow-same-origin allow-top-navigation-by-user-activation",
            1,
        );
        let source = child(&context, page, &target, "");
        assert!(
            !context
                .script_close_sandbox_allowed(page, &source, page, &target)
                .unwrap()
        );
        let owner = AgentOwner::Window {
            page,
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        context.record_native_activation(owner.clone()).unwrap();
        assert!(
            context
                .script_close_sandbox_allowed(page, &source, page, &target)
                .unwrap()
        );
        assert!(context.transient_native_activation(&owner, false).unwrap());
    }
    #[test]
    fn unrelated_top_level_requires_target_sandbox_navigator_exception_and_rejects_stale_or_nonroot()
     {
        let context = crate::Browser::new().new_context();
        let a = context.new_page().page_id();
        let b = context.new_page().page_id();
        let source = root(&context, a, "", 1);
        let target = root(&context, b, "sandbox allow-same-origin", 1);
        assert!(
            !context
                .script_close_sandbox_allowed(a, &source, b, &target)
                .unwrap()
        );
        context
            .inner
            .group
            .pages
            .lock()
            .unwrap()
            .get_mut(&b)
            .unwrap()
            .sandbox_navigator = Some((a, WindowTarget::Root));
        assert!(
            context
                .script_close_sandbox_allowed(a, &source, b, &target)
                .unwrap()
        );
        let frame = child(&context, a, &source, "");
        assert!(
            context
                .script_close_sandbox_allowed(a, &source, a, &frame)
                .is_err()
        );
        let _ = root(&context, b, "", 2);
        assert!(
            context
                .script_close_sandbox_allowed(a, &source, b, &target)
                .is_err()
        );
        let _ = root(&context, a, "", 2);
        assert!(
            context
                .script_close_sandbox_allowed(
                    a,
                    &source,
                    b,
                    &Creator::Window {
                        target: WindowTarget::Root,
                        document_generation: 2,
                        stream_epoch: 0
                    }
                )
                .is_err()
        );
    }
    #[test]
    fn private_prerender_documents_cannot_supply_either_close_authority() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page().page_id();
        let active = root(&context, page, "", 1);
        let scope = context
            .reserve_native_prerender(page, &active, "https://root.test/private")
            .unwrap();
        context
            .commit_prerender_response_policy(page, &scope, &scope.url, &[])
            .unwrap();
        let private = Creator::Window {
            target: WindowTarget::Root,
            document_generation: scope.lifetime.0,
            stream_epoch: scope.lifetime.1,
        };
        assert!(
            context
                .window_worker_creator(
                    page,
                    &WindowTarget::Root,
                    scope.lifetime.0,
                    scope.lifetime.1
                )
                .is_ok(),
            "private authority exists for preparation"
        );
        assert!(
            context
                .script_close_sandbox_allowed(page, &private, page, &active)
                .is_err()
        );
        assert!(
            context
                .script_close_sandbox_allowed(page, &active, page, &private)
                .is_err()
        );
    }
}
