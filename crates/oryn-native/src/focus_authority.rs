//! Script focus consumes a live caller and native relationship in one admission.
use super::*;
use oryn_js::worker_wire::Creator;
impl BrowserContext {
    pub(crate) fn activate_page_from_creator(
        &self,
        source: PageId,
        creator: &Creator,
        target: PageId,
        expected: Option<crate::browsing_groups::GroupToken>,
    ) -> Result<(), RuntimeError> {
        let Creator::Window {
            target: owner,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "focus requires a Window creator".into(),
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
                if !pages
                    .get(&target)
                    .is_some_and(|record| record.is_live() && !record.hidden_prerender)
                {
                    return Err(RuntimeError::PageNotFound(target));
                }
                let groups = self.inner.group.browsing_groups.lock().map_err(|_| {
                    RuntimeError::ContextService("browsing group lock poisoned".into())
                })?;
                if !groups.related(source, target, expected) {
                    return Err(RuntimeError::ProtocolViolation(
                        "focus target relationship retired".into(),
                    ));
                }
                let mut active = self.inner.group.active.lock().map_err(|_| {
                    RuntimeError::ContextService("active page lock poisoned".into())
                })?;
                for record in pages.values_mut() {
                    record.descriptor.active = record.descriptor.page.page_id == target;
                    if record.descriptor.active
                        && record.descriptor.status == PageStatus::AttentionRequired
                    {
                        record.descriptor.status = PageStatus::Open;
                    }
                }
                *active = Some(target);
                Ok(())
            },
        )?;
        self.inner
            .group
            .push_event(PageLifecycleEvent::Activated { page_id: target });
        self.record_context_diagnostic(ContextEvent::PageLifecycle {
            event: PageLifecycleEvent::Activated { page_id: target },
        });
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest as Q, WindowTarget as T};
    #[test]
    fn focus_rejects_retired_and_forged_callers_before_mutating_active_page() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://focus.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                Q::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: T::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        context
            .activate_page_from_creator(id, &creator, id, None)
            .unwrap();
        let active = *context.inner.group.active.lock().unwrap();
        let foreign = Creator::Window {
            target: T::Frame("f".repeat(64)),
            document_generation: 1,
            stream_epoch: 0,
        };
        assert!(
            context
                .activate_page_from_creator(id, &foreign, id, None)
                .is_err()
        );
        context
            .handle_message_wire(
                id,
                Q::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .activate_page_from_creator(id, &creator, id, None)
                .is_err()
        );
        assert_eq!(*context.inner.group.active.lock().unwrap(), active);
        let current = Creator::Window {
            target: T::Root,
            document_generation: 2,
            stream_epoch: 0,
        };
        let other_context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let other = other_context.new_page();
        let other_id = other.page_id();
        other_context
            .install_document_security(other_id, "https://other.test/".into(), &[])
            .unwrap();
        let unrelated = other_context
            .native_window_group(other_id, other_id, None)
            .unwrap()
            .0;
        assert!(unrelated.is_some());
        assert!(
            context
                .activate_page_from_creator(id, &current, id, unrelated)
                .is_err()
        );
        assert!(
            context
                .activate_page_from_creator(id, &current, PageId(u64::MAX), None)
                .is_err()
        );
        assert_eq!(*context.inner.group.active.lock().unwrap(), active);
    }
    #[test]
    fn focus_wire_rejects_legacy_page_only_authority() {
        assert!(serde_json::from_value::<crate::context_service::ContextServiceRequest>(serde_json::json!({"operation":"activate_page","source_page_id":1,"target_page_id":2})).is_err());
    }
}
