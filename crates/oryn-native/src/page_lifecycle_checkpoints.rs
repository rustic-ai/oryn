//! Retained history lifecycle publication checkpoints.

use super::*;

/// Only absence of a retained response proves this step performed no work.
/// Deferred and obsolete responses conservatively preserve the follow-up sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NavigationResponseProgress {
    NoWork,
    Processed,
}

impl PageState {
    #[cfg(feature = "v8-host")]
    pub(super) fn finish_pending_navigation_response(
        &mut self,
    ) -> Result<NavigationResponseProgress, RuntimeError> {
        let _close_phase =
            crate::ipc_progress::close_admission::phase("navigation_response_completion");
        let Some(pending) = self.pending_navigation_response.take() else {
            return Ok(NavigationResponseProgress::NoWork);
        };
        let current_identity = self
            .executable
            .as_ref()
            .and_then(|page| page.form_history_root_identity())
            .map(|(realm, generation, epoch)| (u64::from(realm.0), generation, epoch));
        if pending.generation != self.document_generation
            || pending.root_identity.is_none()
            || pending.root_identity != current_identity
        {
            return Ok(NavigationResponseProgress::Processed);
        }
        if self
            .executable
            .as_ref()
            .is_some_and(|page| page.parser_pending())
        {
            self.pending_navigation_response = Some(pending);
            return Ok(NavigationResponseProgress::Processed);
        }
        let PendingNavigationResponse {
            current,
            status,
            response_headers,
            essence,
            capture,
            ..
        } = pending;
        let capture = {
            let mut capture = capture.lock().expect("navigation capture");
            if !capture.completed {
                return Err(RuntimeError::Network(
                    "navigation parser stopped before response completion".into(),
                ));
            }
            std::mem::take(&mut *capture)
        };
        #[cfg(unix)]
        self.record_committed_navigation_progress(NavigationReadinessStage::ParserSettled);
        self.trace.push(
            Revision(self.revision),
            None,
            TraceEventKind::ResponseReceived {
                url: current.to_string(),
                status,
                bytes: capture.bytes,
            },
        );
        let resource_source = if essence == "text/html" {
            capture.source.clone()
        } else {
            self.document
                .as_ref()
                .map(|document| resource_markup_for_document(&document.dom, document.document))
                .unwrap_or_default()
        };
        if let Some(policy) = meta_referrer_policy(&resource_source) {
            self.referrer_policy = crate::referrer::ReferrerPolicy::parse_list(&policy);
            // This is the completed-response projection only. The live
            // parser publishes metadata with its exact document identity.
        }
        self.set_challenge_assessment(assess_challenge(status, &response_headers, &capture.source));
        // Re-evaluate the completed source for parent trace publication and
        // non-script resource discovery. Script execution itself was
        // authorized at its parser boundary and is never replayed here.
        // Its report/event was also published there; this projection only
        // records trace entries and must not submit a second parent report.
        let _ = self.enforce_csp_html(&resource_source);
        self.load_auxiliary_resources(&current, &resource_source);
        // Font bytes arrive through the executable's retained stylesheet
        // jobs and are projected from the current native document at layout.
        self.web_fonts.clear();
        self.layout = None;
        self.style_revision = self.style_revision.saturating_add(1);
        Ok(NavigationResponseProgress::Processed)
    }

    #[cfg(feature = "v8-host")]
    pub(super) fn queue_rollback_pageshow(&mut self, persisted: bool) -> Result<(), RuntimeError> {
        if self.history_publications.len() >= 129 {
            return Err(RuntimeError::ProtocolViolation(
                "rollback publication capacity".into(),
            ));
        }
        self.history_publications.try_reserve(1).map_err(|_| {
            RuntimeError::ProtocolViolation("rollback publication allocation".into())
        })?;
        let index = self.history_index.ok_or(RuntimeError::HistoryBoundary)?;
        let history = Self::visible_navigation_history(&self.history, index, "traverse")?;
        let session = Self::full_session_history(&self.history, index, "traverse")?;
        let executable = self
            .executable
            .as_mut()
            .ok_or(RuntimeError::EvaluationUnavailable)?;
        self.history_publications
            .push_front(PendingHistoryPublication {
                identity: executable.form_history_root_identity(),
                generation: self.document_generation,
                history,
                session,
                activation: None,
                transition: None,
                pageshow: None,
                pageshow_required: true,
                pageshow_persisted: persisted,
                receipt: None,
                stage: 6,
                retry: None,
            });
        Ok(())
    }
    #[cfg(feature = "v8-host")]
    pub(super) fn poll_history_publications(&mut self) -> Result<(), RuntimeError> {
        let _timing = crate::native_timing::stage("page.history_publication");
        loop {
            if self.terminal_close_requested() {
                return Ok(());
            }
            if self
                .history_publications
                .front()
                .is_some_and(|pending| pending.retry.is_some_and(|retry| retry > Instant::now()))
            {
                return Ok(());
            }
            let Some(mut pending) = self.history_publications.pop_front() else {
                return Ok(());
            };
            if pending.generation != self.document_generation {
                continue;
            }
            let result = (|| -> Result<bool, RuntimeError> {
                let executable = self
                    .executable
                    .as_mut()
                    .ok_or(RuntimeError::EvaluationUnavailable)?;
                loop {
                    // Each preceding stage recorded its actual completed receipt
                    // before this boundary. Retain the next stage and later jobs
                    // without interrupting an admitted callback/microtask turn.
                    if executable.terminal_retired() {
                        return Ok(false);
                    }
                    let identity = executable.form_history_root_identity();
                    if let Some(expected) = pending.identity {
                        if identity != Some(expected) {
                            return Ok(true);
                        }
                    } else {
                        pending.identity = identity;
                        if identity.is_none() {
                            return Ok(true);
                        }
                    }
                    #[cfg(test)]
                    let _stage_phase =
                        crate::ipc_progress::close_admission::phase("history_publication_stage");
                    match pending.stage {
                        0 => {
                            pending.receipt = Some(
                                executable
                                    .start_native_history_transaction(
                                        pending.history.clone(),
                                        pending.session.clone(),
                                        pending.activation.clone(),
                                    )
                                    .map_err(map_host_error)?,
                            );
                            pending.stage = 1;
                        }
                        1 => {
                            if !executable
                                .resume_native_history_publication(pending.receipt.unwrap())
                                .map_err(map_host_error)?
                            {
                                return Ok(false);
                            }
                            pending.receipt = None;
                            pending.stage = 6;
                        }
                        4 => {
                            if let Some(event) = &pending.transition {
                                pending.receipt = Some(
                                    executable
                                        .start_native_event_publication(event.clone())
                                        .map_err(map_host_error)?,
                                );
                            }
                            pending.stage = 5;
                        }
                        5 => {
                            if let Some(receipt) = pending.receipt {
                                if !executable
                                    .resume_native_history_publication(receipt)
                                    .map_err(map_host_error)?
                                {
                                    return Ok(false);
                                }
                                pending.receipt = None;
                            }
                            pending.stage = 7;
                        }
                        6 => {
                            #[cfg(test)]
                            if ROOT_RESTORE_PUBLICATION_FAULT.with(|fault| fault.replace(false)) {
                                return Err(RuntimeError::EvaluationUnavailable);
                            }
                            if pending.pageshow_required && pending.pageshow.is_none() {
                                pending.pageshow = Some(
                                    executable
                                        .prepare_native_page_transition(
                                            "pageshow",
                                            pending.pageshow_persisted,
                                        )
                                        .map_err(map_host_error)?,
                                );
                            }
                            if let Some(transition) = pending.pageshow.as_mut()
                                && !executable
                                    .resume_native_page_transition(transition)
                                    .map_err(map_host_error)?
                            {
                                return Ok(false);
                            }
                            pending.stage = 4;
                        }
                        _ => return Ok(true),
                    }
                }
            })();
            match result {
                Ok(true) => {}
                other => {
                    pending.retry = Some(Instant::now() + Duration::from_millis(25));
                    self.history_publications.push_front(pending);
                    return other.map(|_| ());
                }
            }
        }
    }
}
