//! Native frame paint resources use the exact selected realm. Existing decoded
//! surfaces are reused; missing bytes require the parent-owned frame principal.
use super::*;
#[derive(Clone)]
pub(crate) struct NativePaintCancellation {
    pub cancellation: Option<std::sync::Arc<oryn_js::prompts::PromptController>>,
    pub deadline: std::time::Instant,
}
impl NativePaintCancellation {
    pub(crate) fn check(&self) -> Result<(), HostError> {
        if std::time::Instant::now() >= self.deadline {
            return Err(HostError::Execution(
                "native paint request deadline exceeded".into(),
            ));
        }
        if self
            .cancellation
            .as_ref()
            .is_some_and(|cancel| cancel.inspect().is_err())
        {
            return Err(HostError::Execution(
                "native paint request cancelled".into(),
            ));
        }
        Ok(())
    }
}

/// Exact binding owners retained by one capture, independently of the host's
/// currently selected root or another capture's temporary resource inventory.
pub(crate) struct NativeCssCapture {
    id: u64,
    owners: RefCell<Vec<(Rc<oryn_js::v8_bindings::V8BindingState>, u64, u64, u64)>>,
}
impl NativeCssCapture {
    pub(crate) fn new() -> Result<Rc<Self>, HostError> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |value| value.checked_add(1),
            )
            .map_err(|_| HostError::Execution("CSS capture identity exhausted".into()))?;
        Ok(Rc::new(Self {
            id,
            owners: RefCell::new(Vec::new()),
        }))
    }
    pub(crate) fn retain(
        &self,
        state: &Rc<oryn_js::v8_bindings::V8BindingState>,
        urls: &BTreeSet<String>,
    ) -> Result<(), HostError> {
        let identity = (
            state.document_generation(),
            state.document_stream_epoch(),
            state.document_authority_epoch(),
        );
        let mut owners = self.owners.borrow_mut();
        if !owners.iter().any(|(owner, generation, stream, authority)| {
            Rc::ptr_eq(owner, state) && (*generation, *stream, *authority) == identity
        }) {
            if owners.len() >= 4096 {
                return Err(HostError::Execution("CSS capture document limit".into()));
            }
            owners.push((state.clone(), identity.0, identity.1, identity.2));
        }
        drop(owners);
        state
            .retain_capture_css_images(self.id, urls)
            .map_err(|error| HostError::Execution(error.to_string()))
    }
    pub(crate) fn pending(&self) -> bool {
        self.owners
            .borrow()
            .iter()
            .any(|(state, generation, stream, authority)| {
                !state.execution_retired()
                    && (
                        state.document_generation(),
                        state.document_stream_epoch(),
                        state.document_authority_epoch(),
                    ) == (*generation, *stream, *authority)
                    && state.capture_css_images_pending(self.id)
            })
    }
    pub(crate) fn release(&self) -> Result<(), HostError> {
        let mut failure = None;
        for (state, generation, stream, authority) in self.owners.borrow().iter() {
            if state.execution_retired()
                || (
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.document_authority_epoch(),
                ) != (*generation, *stream, *authority)
            {
                continue;
            }
            if let Err(error) = state.release_capture_css_images(self.id) {
                if failure.is_none() {
                    failure = Some(HostError::Execution(error.to_string()));
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
impl Drop for NativeCssCapture {
    fn drop(&mut self) {
        let _ = self.release();
    }
}
/// Switching capture owners does not borrow the executable across callbacks.
/// Drop restores only the slot this guard installed, so a replacement owner
/// cannot be overwritten by an obsolete nested operation.
pub(crate) struct NativeCssCaptureGuard {
    slot: Rc<RefCell<Option<Rc<NativeCssCapture>>>>,
    installed: Rc<NativeCssCapture>,
    previous: Option<Rc<NativeCssCapture>>,
}
impl Drop for NativeCssCaptureGuard {
    fn drop(&mut self) {
        let current = self
            .slot
            .borrow()
            .as_ref()
            .is_some_and(|owner| Rc::ptr_eq(owner, &self.installed));
        if current {
            *self.slot.borrow_mut() = self.previous.take();
        }
    }
}

impl RustExecutableDocument {
    pub(crate) fn paint_resource_creator(
        &self,
    ) -> Result<oryn_js::worker_wire::Creator, HostError> {
        let realm = if let Some(scope) = self.automation_scope.as_ref() {
            self.validate_automation_scope(scope)?;
            scope.realm
        } else {
            self.top_realm
        };
        self.agent.host.native_creator_for_realm(realm)
    }
    pub(crate) fn activate_css_capture(
        &self,
        owner: Rc<NativeCssCapture>,
    ) -> NativeCssCaptureGuard {
        let previous = self.css_capture_owner.borrow_mut().replace(owner.clone());
        NativeCssCaptureGuard {
            slot: self.css_capture_owner.clone(),
            installed: owner,
            previous,
        }
    }
    pub(crate) fn css_capture_pending(&self) -> bool {
        self.css_capture_owner
            .borrow()
            .as_ref()
            .is_some_and(|owner| owner.pending())
    }
    pub(crate) fn css_paint_images(
        &self,
        urls: &BTreeSet<String>,
    ) -> Result<Vec<oryn_common::v4::PaintImageResource>, HostError> {
        let identity = if let Some(scope) = self.automation_scope.as_ref() {
            self.validate_automation_scope(scope)?;
            Some((scope.realm, scope.document_generation, scope.stream_epoch))
        } else {
            self.automation_realms()
                .into_iter()
                .find(|entry| entry.parent_realm.is_none() && entry.live)
                .map(|root| (root.realm, root.document_generation, root.stream_epoch))
        };
        let Some((realm, generation, epoch)) = identity else {
            return Ok(Vec::new());
        };
        if self.css_capture_owner.borrow().is_none() {
            *self.css_capture_owner.borrow_mut() = Some(NativeCssCapture::new()?);
        }
        let owner = self.css_capture_owner.borrow().as_ref().unwrap().clone();
        self.agent
            .host
            .capture_css_images(realm, generation, epoch, urls, &owner)
            .map_err(|error| HostError::Execution(error.to_string()))
    }
    pub(crate) fn replace_native_paint_cancellation(
        &mut self,
        value: Option<NativePaintCancellation>,
    ) -> Option<NativePaintCancellation> {
        std::mem::replace(&mut self.native_paint_cancellation, value)
    }
    fn check_native_paint_cancellation(&self) -> Result<(), HostError> {
        if let Some(cancel) = &self.native_paint_cancellation {
            cancel.check()?;
        }
        Ok(())
    }

    pub(crate) fn paint_visual_viewport(
        &self,
    ) -> Result<Option<oryn_common::visual_viewport::Geometry>, HostError> {
        let identity = if let Some(scope) = self.automation_scope.as_ref() {
            self.validate_automation_scope(scope)?;
            Some((scope.realm, scope.document_generation, scope.stream_epoch))
        } else {
            self.automation_realms()
                .into_iter()
                .find(|entry| entry.parent_realm.is_none() && entry.live)
                .map(|root| (root.realm, root.document_generation, root.stream_epoch))
        };
        let Some((realm, generation, epoch)) = identity else {
            return Ok(None);
        };
        self.agent
            .host
            .capture_visual_viewport(realm, generation, epoch)
            .map_err(|error| HostError::Execution(error.to_string()))
    }
    pub(crate) fn document_paint_images(
        &self,
    ) -> Result<Vec<oryn_js::drag_image::Resource>, HostError> {
        if self.automation_scope.is_some() {
            return self.selected_paint_images();
        }
        let Some(root) = self
            .automation_realms()
            .into_iter()
            .find(|root| root.parent_realm.is_none() && root.live)
        else {
            return Ok(Vec::new());
        };
        self.agent
            .host
            .capture_paint_images(root.realm, root.document_generation, root.stream_epoch)
            .map_err(|error| HostError::Execution(error.to_string()))
    }
    pub(crate) fn selected_paint_images(
        &self,
    ) -> Result<Vec<oryn_js::drag_image::Resource>, HostError> {
        let scope = self.automation_scope.as_ref().ok_or_else(|| {
            HostError::Execution("frame image capture requires selected realm".into())
        })?;
        self.validate_automation_scope(scope)?;
        self.agent
            .host
            .capture_paint_images(scope.realm, scope.document_generation, scope.stream_epoch)
            .map_err(|error| HostError::Execution(error.to_string()))
    }
    pub(crate) fn load_selected_paint_image(
        &mut self,
        url: &str,
        limit: usize,
    ) -> Result<oryn_common::v4::PaintImageResource, HostError> {
        self.check_native_paint_cancellation()?;
        let scope = self.automation_scope.clone().ok_or_else(|| {
            HostError::Execution("frame resource load requires selected realm".into())
        })?;
        self.validate_automation_scope(&scope)?;
        #[cfg(not(unix))]
        {
            let _ = (url, limit);
            return Err(HostError::Execution(
                "frame resource authority bridge unavailable".into(),
            ));
        }
        #[cfg(unix)]
        {
            use crate::context_service::{
                ContextServiceRequest as Q, ContextServiceResponse as R, HttpStreamRequest,
            };
            let (client, page_id, _) = self.context.as_ref().ok_or_else(|| {
                HostError::Execution("frame resource authority bridge unavailable".into())
            })?;
            let client = client.clone();
            let page_id = *page_id;
            let creator = self.agent.host.native_creator_for_realm(scope.realm)?;
            let counter = self.next_dynamic_request_id;
            self.next_dynamic_request_id = counter.checked_add(1).ok_or_else(|| {
                HostError::Execution("native resource request identity exhausted".into())
            })?;
            let request_id = u64::MAX.checked_sub(counter).ok_or_else(|| {
                HostError::Execution("native resource request identity exhausted".into())
            })?;
            if url.len() > 64 * 1024 || limit == 0 || limit > 64 * 1024 * 1024 {
                return Err(HostError::Execution(
                    "frame image request exceeds bounds".into(),
                ));
            }
            let response = client
                .request(Q::ImageStreamStart {
                    page_id,
                    cors_enabled: false,
                    request: HttpStreamRequest {
                        upgrade_mixed_image: false,
                        fetch: None,
                        client: Some(creator.clone()),
                        use_cors_preflight: false,
                        request_id,
                        method: "GET".into(),
                        url: url.into(),
                        headers: Vec::new(),
                        body_base64: String::new(),
                        credentials: true,
                        credentials_mode: Some("include".into()),
                        initiator_url: None,
                        referrer_policy: None,
                    },
                })
                .map_err(|error| HostError::Execution(error.to_string()))?;
            let R::HttpStreamStarted {
                stream_id,
                url: final_url,
                status,
                headers,
                total_bytes,
                ..
            } = response
            else {
                return Err(HostError::Execution(
                    "invalid frame image stream response".into(),
                ));
            };
            let result = (|| {
                self.check_native_paint_cancellation()?;
                if !(200..300).contains(&status)
                    || total_bytes.is_some_and(|bytes| bytes > limit)
                    || final_url.len() > 64 * 1024
                {
                    return Err(HostError::Execution(
                        "frame image response rejected by size/status bounds".into(),
                    ));
                }
                let media_type = oryn_common::mime_type::MimeType::from_headers(&headers)
                    .map(|mime| mime.essence().to_owned())
                    .unwrap_or_else(|| "application/octet-stream".into());
                let mut bytes = Vec::new();
                for _ in 0..1024 {
                    self.check_native_paint_cancellation()?;
                    self.validate_automation_scope(&scope)?;
                    let chunk = client
                        .request(Q::HttpStreamRead {
                            page_id,
                            creator: Some(creator.clone()),
                            stream_id,
                            max_bytes: 64 * 1024,
                        })
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    let R::HttpStreamChunk {
                        stream_id: actual,
                        body_base64,
                        done,
                        encoded_body_size: _,
                    } = chunk
                    else {
                        return Err(HostError::Execution("invalid frame image chunk".into()));
                    };
                    if actual != stream_id || body_base64.len() > 96 * 1024 {
                        return Err(HostError::Execution(
                            "frame image chunk identity/size mismatch".into(),
                        ));
                    }
                    let chunk = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        body_base64,
                    )
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                    if bytes.len().saturating_add(chunk.len()) > limit {
                        return Err(HostError::Execution(
                            "frame image body exceeds bounds".into(),
                        ));
                    }
                    bytes.extend(chunk);
                    if done {
                        self.check_native_paint_cancellation()?;
                        self.validate_automation_scope(&scope)?;
                        return Ok(oryn_common::v4::PaintImageResource {
                            svg_document_snapshot: false,
                            content_type: oryn_common::xml_encoding::response_content_type(
                                &headers,
                            ),
                            url_aliases: if final_url != url {
                                vec![url.into()]
                            } else {
                                Vec::new()
                            },
                            url: final_url,
                            media_type,
                            bytes,
                        });
                    }
                }
                Err(HostError::Execution(
                    "frame image chunk count exceeds bounds".into(),
                ))
            })();
            let _ = client.request(Q::HttpStreamAbort {
                page_id,
                creator: Some(creator),
                stream_id,
            });
            result
        }
    }
}

impl RustExecutableDocument {
    pub(crate) fn install_selected_paint_capture_inventory(
        &self,
        snapshot: &oryn_common::v4::PaintSnapshot,
    ) -> Result<(), HostError> {
        let scope = self.automation_scope.as_ref().ok_or_else(|| {
            HostError::Execution("paint inventory requires selected realm".into())
        })?;
        self.validate_automation_scope(scope)?;
        self.agent
            .host
            .install_paint_capture_inventory(
                scope.realm,
                scope.document_generation,
                scope.stream_epoch,
                snapshot,
            )
            .map_err(|error| HostError::Execution(error.to_string()))
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    #[test]
    fn nested_capture_guards_restore_only_their_own_slot() {
        let page = RustExecutableDocument::load("<body>capture</body>").unwrap();
        let outer = NativeCssCapture::new().unwrap();
        let inner = NativeCssCapture::new().unwrap();
        let replacement = NativeCssCapture::new().unwrap();
        let outer_guard = page.activate_css_capture(outer.clone());
        {
            let inner_guard = page.activate_css_capture(inner.clone());
            assert!(
                page.css_capture_owner
                    .borrow()
                    .as_ref()
                    .is_some_and(|owner| Rc::ptr_eq(owner, &inner))
            );
            drop(inner_guard);
        }
        assert!(
            page.css_capture_owner
                .borrow()
                .as_ref()
                .is_some_and(|owner| Rc::ptr_eq(owner, &outer))
        );
        let inner_guard = page.activate_css_capture(inner);
        *page.css_capture_owner.borrow_mut() = Some(replacement.clone());
        drop(inner_guard);
        drop(outer_guard);
        assert!(
            page.css_capture_owner
                .borrow()
                .as_ref()
                .is_some_and(|owner| Rc::ptr_eq(owner, &replacement))
        );
    }
    #[test]
    fn expired_native_frame_paint_deadline_prevents_resource_start() {
        let token = NativePaintCancellation {
            cancellation: None,
            deadline: std::time::Instant::now() - std::time::Duration::from_millis(1),
        };
        assert!(token.check().is_err());
    }
    #[test]
    fn future_native_frame_paint_deadline_permits_bounded_work() {
        let token = NativePaintCancellation {
            cancellation: None,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(60),
        };
        assert!(token.check().is_ok());
    }
}
