//! Logical closing belongs to the traversable, not a particular renderer
//! Document or a bounded close-job receipt. Canceling unloading retains it.
use super::*;
use oryn_js::worker_wire::Creator;
impl BrowserContext {
    pub(crate) fn script_close_state(
        &self,
        source: PageId,
        creator: &Creator,
        target: PageId,
        expected: Option<crate::browsing_groups::GroupToken>,
    ) -> Result<bool, RuntimeError> {
        let Creator::Window {
            target: owner,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "Window.closed requires native Window authority".into(),
            ));
        };
        self.with_window_worker_creator_pages(
            source,
            owner,
            *document_generation,
            *stream_epoch,
            false,
            false,
            |_, _, _, pages| {
                if pages
                    .get(&source)
                    .is_some_and(|record| record.hidden_prerender)
                    || pages
                        .get(&target)
                        .is_some_and(|record| record.hidden_prerender)
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "private prerender has no public closing-state endpoint".into(),
                    ));
                }
                let groups = self.inner.group.browsing_groups.lock().map_err(|_| {
                    RuntimeError::ContextService("closing-state group lock poisoned".into())
                })?;
                if !groups.related(source, target, expected) {
                    return Err(RuntimeError::ProtocolViolation(
                        "closing-state target relationship retired".into(),
                    ));
                }
                let record = pages
                    .get(&target)
                    .ok_or(RuntimeError::PageNotFound(target))?;
                Ok(record.is_closing || !record.is_live())
            },
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    #[test]
    fn closing_survives_document_replacement_and_does_not_retire_page() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://closing.test/".into(), &[])
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
        let creator = |generation| Creator::Window {
            target: WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 0,
        };
        assert!(
            !context
                .script_close_state(id, &creator(1), id, None)
                .unwrap()
        );
        context
            .inner
            .group
            .pages
            .lock()
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .is_closing = true;
        assert!(
            context
                .script_close_state(id, &creator(1), id, None)
                .unwrap()
        );
        // Terminal receipt pruning/cancellation must not own the traversable flag.
        let (admission, _) = context
            .inner
            .script_close_admissions
            .lock()
            .unwrap()
            .admit(id, creator(1), id)
            .unwrap();
        context
            .inner
            .script_close_admissions
            .lock()
            .unwrap()
            .finish(
                id,
                admission.token,
                super::super::script_close_admission::Phase::Canceled,
            )
            .unwrap();
        assert!(
            context
                .script_close_state(id, &creator(1), id, None)
                .unwrap()
        );
        assert!(context.inner.group.pages.lock().unwrap()[&id].is_live());
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
            context
                .script_close_state(id, &creator(1), id, None)
                .is_err()
        );
        assert!(
            context
                .script_close_state(id, &creator(2), id, None)
                .unwrap()
        );
        assert!(context.inner.group.pages.lock().unwrap()[&id].is_live());
    }
    #[test]
    fn hidden_page_is_not_a_public_close_endpoint_even_with_current_root_authority() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://private.test/".into(), &[])
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
        context
            .inner
            .group
            .pages
            .lock()
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .hidden_prerender = true;
        assert!(context.script_close_state(id, &creator, id, None).is_err());
        assert!(
            context
                .request_script_close(id, &creator, id, None)
                .is_err()
        );
        let pages = context.inner.group.pages.lock().unwrap();
        assert!(pages[&id].is_live());
        assert!(!pages[&id].is_closing);
        drop(pages);
        assert!(
            context
                .inner
                .script_close_admissions
                .lock()
                .unwrap()
                .active_snapshot()
                .is_empty()
        );
        // Publication of the same native page makes its state readable; no
        // replacement Creator or synthetic history proof is introduced.
        context
            .inner
            .group
            .pages
            .lock()
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .hidden_prerender = false;
        assert!(!context.script_close_state(id, &creator, id, None).unwrap());
    }
}
