//! Public close admission. The synchronous method sets logical closing only
//! after exact native eligibility; its retained job performs departure later.
use super::*;
use oryn_js::worker_wire::Creator;
impl BrowserContext {
    pub(crate) fn request_script_close(
        &self,
        source: PageId,
        creator: &Creator,
        target: PageId,
        expected: Option<crate::browsing_groups::GroupToken>,
    ) -> Result<bool, RuntimeError> {
        if self.script_close_state(source, creator, target, expected)? {
            self.ensure_script_close_runner()?;
            return Ok(true);
        }
        let target_creator = self.root_navigation_creator(target)?;
        if !self.script_close_familiar(source, creator, target, &target_creator, expected)? {
            return Ok(false);
        }
        if !self.script_close_sandbox_allowed(source, creator, target, &target_creator)? {
            return Ok(false);
        }
        let web_created = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| RuntimeError::ContextService("close page registry poisoned".into()))?
            .get(&target)
            .ok_or(RuntimeError::PageNotFound(target))?
            .created_by_web_content;
        if !web_created && self.wait_script_close_history(target, &target_creator)? != 1 {
            return Ok(false);
        }
        // Establish runnable custody before publishing sticky closing. A
        // resource failure must leave the target eligible for a later attempt.
        self.ensure_script_close_runner()?;
        let Creator::Window {
            target: source_target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "close requires an incumbent Window".into(),
            ));
        };
        self.with_window_worker_creator_pages_checked(
            source,
            source_target,
            *document_generation,
            *stream_epoch,
            false,
            false,
            Some((target, &target_creator)),
            |_, _, _, pages| {
                let groups = self.inner.group.browsing_groups.lock().map_err(|_| {
                    RuntimeError::ContextService("close group registry poisoned".into())
                })?;
                if !groups.related(source, target, expected) {
                    return Err(RuntimeError::NavigationCancelled);
                }
                if pages
                    .get(&source)
                    .is_some_and(|record| record.hidden_prerender)
                {
                    return Err(RuntimeError::NavigationCancelled);
                }
                let record = pages
                    .get_mut(&target)
                    .ok_or(RuntimeError::PageNotFound(target))?;
                if record.hidden_prerender {
                    return Err(RuntimeError::NavigationCancelled);
                }
                if record.is_closing {
                    return Ok(true);
                }
                if !record.created_by_web_content {
                    match record
                        .history_projection
                        .as_ref()
                        .and_then(|projection| projection.count(&target_creator))
                    {
                        Some(1) => {}
                        Some(_) => return Ok(false),
                        None => {
                            return Err(RuntimeError::ContextService(
                                "history projection changed during close admission; retry required"
                                    .into(),
                            ));
                        }
                    }
                }
                let mut registry = self.inner.script_close_admissions.lock().map_err(|_| {
                    RuntimeError::ContextService("close admission registry poisoned".into())
                })?;
                let (admission, _) = registry.admit(source, creator.clone(), target)?;
                registry.preparing(target, admission.token, target_creator.clone())?;
                record.is_closing = true;
                Ok(true)
            },
        )
    }
}
