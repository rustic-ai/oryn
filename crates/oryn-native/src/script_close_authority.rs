//! Script-close familiarity uses strict origins and native browsing-context
//! ancestry. Relaxing document.domain must not create close authority.
use super::*;
use crate::agent_registry::AgentOwner;
use oryn_js::{message_wire::WindowTarget, worker_wire::Creator};

impl BrowserContext {
    pub(crate) fn script_close_familiar(
        &self,
        source_page: PageId,
        source: &Creator,
        target_page: PageId,
        target: &Creator,
        expected: Option<crate::browsing_groups::GroupToken>,
    ) -> Result<bool, RuntimeError> {
        if !self
            .native_window_group(source_page, target_page, expected)?
            .2
        {
            return Ok(false);
        }
        let owner = |page, creator: &Creator| match creator {
            Creator::Window {
                target,
                document_generation,
                stream_epoch,
            } => Ok(AgentOwner::Window {
                page,
                target: target.clone(),
                generation: *document_generation,
                epoch: *stream_epoch,
            }),
            _ => Err(RuntimeError::ProtocolViolation(
                "script-close requires Window authority".into(),
            )),
        };
        let source_owner = owner(source_page, source)?;
        let target_owner = owner(target_page, target)?;
        self.native_worker_authority(&source_owner)
            .map_err(|e| RuntimeError::ContextService(e.to_string()))?;
        self.native_worker_authority(&target_owner)
            .map_err(|e| RuntimeError::ContextService(e.to_string()))?;
        let source_origin = self
            .native_document_domain_identity(&source_owner)
            .map_err(RuntimeError::ContextService)?
            .origin()
            .clone();
        // Recursive opener familiarity and the ancestor-origin clause are
        // separate. An ancestor cannot acquire the opener/top-level clauses.
        let mut pending = vec![(target_owner, true)];
        let mut seen = BTreeSet::new();
        let mut familiar = false;
        while let Some((candidate, recurse)) = pending.pop() {
            if !seen.insert((candidate.clone(), recurse)) {
                continue;
            }
            if seen.len() > 4096 {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "script-close familiarity ancestry".into(),
                });
            }
            self.native_worker_authority(&candidate)
                .map_err(|e| RuntimeError::ContextService(e.to_string()))?;
            let identity = self
                .native_document_domain_identity(&candidate)
                .map_err(RuntimeError::ContextService)?;
            if identity.origin() == &source_origin {
                familiar = true;
                break;
            }
            let AgentOwner::Window {
                page,
                target: context_target,
                ..
            } = &candidate
            else {
                continue;
            };
            if recurse && *page == source_page && matches!(context_target, WindowTarget::Root) {
                familiar = true;
                break;
            }
            let parent = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| RuntimeError::ContextService("script-close ancestry poisoned".into()))?
                .activation_parents
                .get(&candidate)
                .cloned();
            if let Some(parent) = parent {
                pending.push((parent, false));
            }
            if recurse
                && matches!(context_target, WindowTarget::Root)
                && let Some(opener) = self.native_window_group(*page, *page, None)?.1
            {
                let opener_target = self.native_opener_target(*page)?;
                let snapshot = self.read_related_window(
                    *page,
                    &match &candidate {
                        AgentOwner::Window {
                            target,
                            generation,
                            epoch,
                            ..
                        } => Creator::Window {
                            target: target.clone(),
                            document_generation: *generation,
                            stream_epoch: *epoch,
                        },
                        _ => unreachable!(),
                    },
                    opener,
                    &opener_target,
                    None,
                )?;
                pending.push((owner(opener, &snapshot.creator)?, true));
            }
        }
        // Do not let retirement/group separation during traversal authenticate
        // a successor or a now-unrelated Window proxy.
        self.native_worker_authority(&source_owner)
            .map_err(|e| RuntimeError::ContextService(e.to_string()))?;
        let target_owner = owner(target_page, target)?;
        self.native_worker_authority(&target_owner)
            .map_err(|e| RuntimeError::ContextService(e.to_string()))?;
        Ok(familiar
            && self
                .native_window_group(source_page, target_page, expected)?
                .2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::message_wire::MessageRequest;
    fn install(context: &BrowserContext, page: PageId, url: &str, generation: u64) -> Creator {
        context
            .install_document_security(page, url.into(), &[])
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
    #[test]
    fn familiar_opener_chain_does_not_authorize_cross_origin_reverse_or_stale_sources() {
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        let source = install(&context, parent.page_id(), "https://parent.test/", 1);
        let popup = context.new_popup(parent.page_id(), None);
        let target = install(&context, popup.page_id(), "https://popup.test/", 2);
        assert!(
            context
                .script_close_familiar(parent.page_id(), &source, popup.page_id(), &target, None)
                .unwrap()
        );
        assert!(
            !context
                .script_close_familiar(popup.page_id(), &target, parent.page_id(), &source, None)
                .unwrap()
        );
        let unrelated = context.new_page();
        let other = install(&context, unrelated.page_id(), "https://parent.test/", 3);
        assert!(
            !context
                .script_close_familiar(parent.page_id(), &source, unrelated.page_id(), &other, None)
                .unwrap()
        );
        install(&context, parent.page_id(), "https://parent.test/", 4);
        assert!(
            context
                .script_close_familiar(parent.page_id(), &source, popup.page_id(), &target, None)
                .is_err()
        );
    }
}
