//! Blank-target candidates have their final native page principal from creation.
//! Promotion publishes that hidden page; it never migrates live storage, worker
//! or message ownership from the initiating page.
use super::worker_service::prerender_policy;
use super::*;
impl BrowserContext {
    pub(crate) fn reserve_blank_prerender(
        &self,
        source: PageId,
        creator: &oryn_js::worker_wire::Creator,
        url: &str,
    ) -> Result<
        (
            PageId,
            crate::context_service::RelatedRootAdmission,
            prerender_policy::Scope,
        ),
        RuntimeError,
    > {
        let oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "blank prerender requires root creator".into(),
            ));
        };
        self.window_worker_creator(
            source,
            &oryn_js::message_wire::WindowTarget::Root,
            *document_generation,
            *stream_epoch,
        )?;
        let page = self.new_named_popup_from_creator_visibility(
            source,
            creator,
            false,
            String::new(),
            true,
        )?;
        let destination = page.page_id();
        let result = (|| {
            let initial = self.native_initial_document_creator(destination)?;
            let mut scope = self.reserve_native_prerender(destination, &initial, url)?;
            scope.blank_source = Some((source, creator.clone()));
            self.inner
                .worker_policies
                .lock()
                .map_err(|_| {
                    RuntimeError::ContextService("blank prerender registry poisoned".into())
                })?
                .prerenders
                .get_mut(&(destination, scope.lifetime.0, scope.lifetime.1))
                .ok_or(RuntimeError::NavigationCancelled)?
                .blank_source = scope.blank_source.clone();
            self.window_worker_creator(
                source,
                &oryn_js::message_wire::WindowTarget::Root,
                *document_generation,
                *stream_epoch,
            )?;
            let admission = self.related_root_admission(destination).ok_or_else(|| {
                RuntimeError::ProtocolViolation(
                    "hidden prerender actor admission unavailable".into(),
                )
            })?;
            Ok((destination, admission, scope))
        })();
        if result.is_err() {
            let _ = self.queue_script_close(destination);
        }
        result
    }
    pub(crate) fn reserve_blank_prerender_target(
        &self,
        source: PageId,
        creator: &oryn_js::worker_wire::Creator,
        url: &str,
    ) -> Result<Option<PageId>, RuntimeError> {
        self.blank_prerender_selection(source, creator, None, url, false)
            .map(|value| value.map(|value| value.page_id))
    }
    pub(crate) fn claim_blank_prerender(
        &self,
        source: PageId,
        creator: &oryn_js::worker_wire::Creator,
        target: Option<PageId>,
        url: &str,
    ) -> Result<Option<crate::context_service::BlankPrerenderActivation>, RuntimeError> {
        self.blank_prerender_selection(source, creator, target, url, true)
    }
    fn blank_prerender_selection(
        &self,
        source: PageId,
        creator: &oryn_js::worker_wire::Creator,
        target: Option<PageId>,
        url: &str,
        claim: bool,
    ) -> Result<Option<crate::context_service::BlankPrerenderActivation>, RuntimeError> {
        let oryn_js::worker_wire::Creator::Window {
            target: owner,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Ok(None);
        };
        if *owner != oryn_js::message_wire::WindowTarget::Root {
            return Ok(None);
        }
        if self
            .window_worker_creator(source, owner, *document_generation, *stream_epoch)
            .is_err()
        {
            return Ok(None);
        }
        let pages = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| RuntimeError::ContextService("blank page registry poisoned".into()))?;
        let mut policies =
            self.inner.worker_policies.lock().map_err(|_| {
                RuntimeError::ContextService("blank policy registry poisoned".into())
            })?;
        let hub =
            self.inner.group.messages.lock().map_err(|_| {
                RuntimeError::ContextService("blank creator registry poisoned".into())
            })?;
        hub.root_document_authority(source.0, *document_generation, *stream_epoch)
            .map_err(RuntimeError::ProtocolViolation)?;
        let Some(((page, generation, epoch), scope)) =
            policies
                .prerenders
                .iter_mut()
                .find(|((page, _, _), scope)| {
                    target.is_none_or(|target| target == *page)
                        && pages
                            .get(page)
                            .is_some_and(|record| record.is_live() && record.hidden_prerender)
                        && scope.blank_source.as_ref() == Some(&(source, creator.clone()))
                        && scope.url == url
                        && !scope.canceling
                        && !scope.blank_selected
                })
        else {
            return Ok(None);
        };
        if claim {
            scope.blank_selected = true;
        }
        Ok(Some(crate::context_service::BlankPrerenderActivation {
            page_id: *page,
            token: scope.token.clone(),
            generation: *generation,
            epoch: *epoch,
            url: scope.url.clone(),
        }))
    }
    pub(crate) fn hidden_prerender_page(&self, page: PageId) -> bool {
        self.inner
            .group
            .pages
            .lock()
            .map(|pages| {
                pages
                    .get(&page)
                    .is_some_and(|record| record.is_live() && record.hidden_prerender)
            })
            .unwrap_or(false)
    }
    pub(crate) fn publish_blank_prerender_page(&self, page: PageId) -> Result<(), RuntimeError> {
        let descriptor = {
            let mut pages = self.inner.group.pages.lock().map_err(|_| {
                RuntimeError::ContextService("blank publication registry poisoned".into())
            })?;
            let record = pages
                .get(&page)
                .filter(|record| record.is_live())
                .ok_or(RuntimeError::PageNotFound(page))?;
            if !record.hidden_prerender {
                return Ok(());
            }
            // Preflight both locks before making the hidden page visible. A
            // publication replay never steals focus from a later activation.
            let mut active = self.inner.group.active.lock().map_err(|_| {
                RuntimeError::ContextService("blank active page registry poisoned".into())
            })?;
            let record = pages.get_mut(&page).unwrap();
            record.hidden_prerender = false;
            record.descriptor.page.opener_page_id = None;
            record.created_by_web_content = true;
            for record in pages.values_mut() {
                record.descriptor.active = record.descriptor.page.page_id == page;
                if record.descriptor.active
                    && record.descriptor.status == PageStatus::AttentionRequired
                {
                    record.descriptor.status = PageStatus::Open;
                }
            }
            *active = Some(page);
            pages[&page].descriptor.clone()
        };
        self.inner.group.push_event(PageLifecycleEvent::Opened {
            page: descriptor.clone(),
        });
        self.record_context_diagnostic(ContextEvent::PageLifecycle {
            event: PageLifecycleEvent::Opened { page: descriptor },
        });
        self.inner
            .group
            .push_event(PageLifecycleEvent::Activated { page_id: page });
        self.record_context_diagnostic(ContextEvent::PageLifecycle {
            event: PageLifecycleEvent::Activated { page_id: page },
        });
        Ok(())
    }
    pub(crate) fn cancel_blank_prerender(
        &self,
        source: PageId,
        page: PageId,
        token: &oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    ) -> Result<(), RuntimeError> {
        {
            let pages =
                self.inner.group.pages.lock().map_err(|_| {
                    RuntimeError::ContextService("blank cleanup pages poisoned".into())
                })?;
            let record = pages.get(&page).ok_or(RuntimeError::PageNotFound(page))?;
            if record.prerender_source != Some(source) {
                return Err(RuntimeError::ProtocolViolation(
                    "foreign blank prerender cleanup".into(),
                ));
            }
            if !record.hidden_prerender {
                return Ok(());
            }
        }
        self.cancel_native_prerender(page, token, (generation, epoch))?;
        self.queue_script_close(page)
    }
}
