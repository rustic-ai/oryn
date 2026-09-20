//! Native receiver identity is retained through clipboard data publication.
use super::*;
use oryn_js::{clipboard::ClipboardData, worker_wire::Creator};
impl BrowserContext {
    fn with_clipboard_creator<T>(
        &self,
        page: PageId,
        creator: &Creator,
        write: bool,
        consume: impl FnOnce() -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "clipboard requires native Window creator".into(),
            ));
        };
        let (owner, _, origin) =
            self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        if !self
            .native_platform_allowed(
                &owner,
                if write {
                    "clipboard-write"
                } else {
                    "clipboard-read"
                },
            )
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?
        {
            return Err(RuntimeError::ContextService(
                "clipboard native secure ancestry or permission policy denied".into(),
            ));
        }
        self.require_policy(
            PolicyRequest {
                action: ConsequentialAction::Clipboard,
                source_origin: origin.serialized.clone(),
                target_origin: None,
                detail: Some(
                    if write {
                        "write context virtual clipboard"
                    } else {
                        "read context virtual clipboard"
                    }
                    .into(),
                ),
            },
            None,
        )?;
        self.with_window_worker_creator_pages(
            crate::runtime::WorkerCreatorPagesInputs {
                page_id: page,
                target,
                generation: *document_generation,
                epoch: *stream_epoch,
                include_urls: false,
                allow_prerender: false,
            },
            |(_, _, current), _, _, _| {
                if current != origin {
                    return Err(RuntimeError::Closed);
                }
                let active = self.inner.group.active.lock().map_err(|_| {
                    RuntimeError::ContextService("active page lock poisoned".into())
                })?;
                if *active != Some(page) {
                    return Err(RuntimeError::ContextService(
                        "clipboard document is not active".into(),
                    ));
                }
                consume()
            },
        )
    }
    pub(crate) fn authorize_clipboard_for_creator(
        &self,
        page: PageId,
        creator: &Creator,
        write: bool,
    ) -> Result<(), RuntimeError> {
        self.with_clipboard_creator(page, creator, write, || Ok(()))
    }
    pub(crate) fn read_clipboard_for_creator(
        &self,
        page: PageId,
        creator: &Creator,
    ) -> Result<ClipboardData, RuntimeError> {
        self.with_clipboard_creator(page, creator, false, || {
            self.inner
                .virtual_clipboard
                .lock()
                .map(|value| value.clone())
                .map_err(|_| RuntimeError::ContextService("clipboard lock poisoned".into()))
        })
    }
    pub(crate) fn write_clipboard_for_creator(
        &self,
        page: PageId,
        creator: &Creator,
        value: ClipboardData,
    ) -> Result<(), RuntimeError> {
        value.validate().map_err(|error| {
            RuntimeError::ContextService(format!("clipboard payload: {error:?}"))
        })?;
        self.with_clipboard_creator(page, creator, true, || {
            *self
                .inner
                .virtual_clipboard
                .lock()
                .map_err(|_| RuntimeError::ContextService("clipboard lock poisoned".into()))? =
                value;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest as Q, WindowTarget as T};
    fn fixture() -> (BrowserContext, PageHandle, Creator) {
        let mut options = ContextOptions::default();
        options.action_policy.rules.insert(
            ConsequentialAction::Clipboard,
            crate::security::PolicyRule::Allow,
        );
        let context = Browser::in_process_with_config(BrowserConfig::default())
            .new_context_with_options(options);
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://clipboard.test/".into(), &[])
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
        let owner = context.window_worker_creator(id, &T::Root, 1, 0).unwrap().0;
        context.publish_worker_secure_context(owner, true).unwrap();
        (
            context,
            page,
            Creator::Window {
                target: T::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
        )
    }
    #[test]
    fn retired_root_and_foreign_frame_cannot_publish_against_replacement_clipboard() {
        let (context, page, root) = fixture();
        let id = page.page_id();
        context
            .write_clipboard_for_creator(id, &root, ClipboardData::text(vec![65]))
            .unwrap();
        assert!(
            context
                .authorize_clipboard_for_creator(id, &root, true)
                .is_ok()
        );
        let foreign = Creator::Window {
            target: T::Frame("f".repeat(64)),
            document_generation: 1,
            stream_epoch: 0,
        };
        assert!(
            context
                .write_clipboard_for_creator(id, &foreign, ClipboardData::text(vec![66]))
                .is_err()
        );
        assert_eq!(
            context
                .read_clipboard_for_creator(id, &root)
                .unwrap()
                .plain_text(),
            &[65]
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
                .write_clipboard_for_creator(id, &root, ClipboardData::text(vec![67]))
                .is_err()
        );
        assert!(context.read_clipboard_for_creator(id, &root).is_err());
        assert_eq!(
            context.inner.virtual_clipboard.lock().unwrap().plain_text(),
            &[65]
        );
    }
    #[test]
    fn clipboard_script_security_comes_from_native_ancestry_not_supplied_origin_strings() {
        let (context, page, root) = fixture();
        let id = page.page_id();
        context
            .install_document_security(id, "http://insecure.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                Q::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 1,
                },
            )
            .unwrap();
        let current = Creator::Window {
            target: T::Root,
            document_generation: 2,
            stream_epoch: 1,
        };
        assert!(
            context
                .authorize_clipboard_for_creator(id, &current, true)
                .is_err()
        );
        assert!(
            context
                .authorize_clipboard_for_creator(id, &root, true)
                .is_err()
        );
        for value in [
            serde_json::json!({"operation":"clipboard_read","page_id":id}),
            serde_json::json!({"operation":"clipboard_authorize_script","page_id":id,"origins":["https://forged.test"],"write":true}),
        ] {
            assert!(
                serde_json::from_value::<crate::context_service::ContextServiceRequest>(value)
                    .is_err()
            );
        }
    }
    #[test]
    fn private_prerender_cannot_focus_or_read_or_publish_clipboard() {
        let (context, page, root) = fixture();
        let id = page.page_id();
        context
            .write_clipboard_for_creator(id, &root, ClipboardData::text(vec![65]))
            .unwrap();
        let scope = context
            .reserve_native_prerender(id, &root, "https://clipboard.test/candidate")
            .unwrap();
        context
            .commit_prerender_response_policy(id, &scope, &scope.url, &[])
            .unwrap();
        let private = Creator::Window {
            target: T::Root,
            document_generation: scope.lifetime.0,
            stream_epoch: scope.lifetime.1,
        };
        let owner = context
            .window_worker_creator(id, &T::Root, scope.lifetime.0, scope.lifetime.1)
            .unwrap()
            .0;
        context.publish_worker_secure_context(owner, true).unwrap();
        assert!(
            context
                .authorize_clipboard_for_creator(id, &private, true)
                .is_err()
        );
        assert!(context.read_clipboard_for_creator(id, &private).is_err());
        assert!(
            context
                .write_clipboard_for_creator(id, &private, ClipboardData::text(vec![66]))
                .is_err()
        );
        assert!(
            context
                .activate_page_from_creator(id, &private, id, None)
                .is_err()
        );
        assert_eq!(
            context
                .read_clipboard_for_creator(id, &root)
                .unwrap()
                .plain_text(),
            &[65]
        );
        context
            .activate_page_from_creator(id, &root, id, None)
            .unwrap();
    }
}
