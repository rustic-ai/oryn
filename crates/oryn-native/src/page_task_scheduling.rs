//! Page task scheduling, readiness waits, and retained checkpoint pumping.

use super::*;

impl PageState {
    pub(super) fn has_pending_page_work(&mut self) -> bool {
        let _timing = crate::native_timing::stage("page.scheduler_pending");
        #[cfg(feature = "v8-host")]
        {
            // A V8 realm can have promise continuations queued after the host
            // operation which made them runnable has left the typed pending
            // tables. V8 does not expose a reliable "microtask queue empty"
            // predicate, so an installed realm remains scheduler-driven. This
            // is the browser event-loop contract: otherwise long-lived remote
            // executors can strand the continuation which starts their next
            // poll immediately after a Fetch completion.
            {
                #[cfg(unix)]
                if self.initial_document_cleanup.is_some() || self.prerender_recovery.is_some() {
                    return true;
                }
            }
            self.failed_install_recovery.is_some()
                || self.root_source_resume.is_some()
                || self.pending_navigation_transaction.is_some()
                || self.settling_navigation.is_some()
                || self.has_retired_agent_documents()
                || self.executable.is_some()
        }
        #[cfg(not(feature = "v8-host"))]
        {
            false
        }
    }

    #[cfg(feature = "v8-host")]
    pub(super) fn history_publication_delay(&self) -> Option<Duration> {
        self.executable.as_ref()?;
        self.history_publications.front().map(|pending| {
            if pending.generation != self.document_generation {
                Duration::ZERO
            } else {
                pending
                    .retry
                    .map(|retry| retry.saturating_duration_since(Instant::now()))
                    .unwrap_or(Duration::ZERO)
            }
        })
    }

    pub(super) fn next_timer_delay(&mut self) -> Option<Duration> {
        let _timing = crate::native_timing::stage("page.scheduler_delay");
        #[cfg(feature = "v8-host")]
        {
            let publication = self.history_publication_delay();
            #[cfg(unix)]
            let prerender = self
                .prerender_candidates
                .delay()
                .into_iter()
                .chain(self.prerender_recovery.as_ref().map(|job| job.delay()))
                .chain(
                    self.pending_navigation_transaction
                        .as_ref()
                        .and_then(|transaction| match &transaction.phase {
                            NavigationTransactionPhase::Prerender(job) => Some(job.delay()),
                            NavigationTransactionPhase::PrerenderWaiting { retry } => Some(
                                retry
                                    .map(|time| time.saturating_duration_since(Instant::now()))
                                    .unwrap_or(Duration::ZERO),
                            ),
                            _ => None,
                        }),
                )
                .min();
            #[cfg(not(unix))]
            let prerender: Option<Duration> = None;
            #[cfg(unix)]
            let initial_cleanup = self.initial_document_cleanup_delay();
            #[cfg(not(unix))]
            let initial_cleanup: Option<Duration> = None;
            let accessibility = self.accessibility_operation_delay();
            self.executable
                .as_mut()
                .and_then(|executable| executable.next_timer_delay())
                .into_iter()
                .chain(publication)
                .chain(prerender)
                .chain(self.source_resume_delay())
                .chain(self.failed_install_delay())
                .chain(initial_cleanup)
                .chain(accessibility)
                .chain(
                    (!self.pending_execution_replies.is_empty())
                        .then_some(Duration::from_millis(25)),
                )
                .min()
        }
        #[cfg(not(feature = "v8-host"))]
        {
            None
        }
    }

    pub(super) fn has_runnable_page_work(&mut self) -> bool {
        let _timing = crate::native_timing::stage("page.scheduler_runnable");
        #[cfg(feature = "v8-host")]
        {
            {
                #[cfg(unix)]
                if self
                    .initial_document_cleanup_delay()
                    .is_some_and(|delay| delay.is_zero())
                {
                    return true;
                }
            }
            {
                #[cfg(unix)]
                if self
                    .prerender_candidates
                    .delay()
                    .is_some_and(|delay| delay.is_zero())
                    || self
                        .prerender_recovery
                        .as_ref()
                        .is_some_and(|job| job.delay().is_zero())
                {
                    return true;
                }
            }
            self.failed_install_delay()
                .is_some_and(|delay| delay.is_zero())
                || self
                    .source_resume_delay()
                    .is_some_and(|delay| delay.is_zero())
                || self
                    .accessibility_operation_delay()
                    .is_some_and(|delay| delay.is_zero())
                || self
                    .history_publication_delay()
                    .is_some_and(|delay| delay.is_zero())
                || self
                    .executable
                    .as_mut()
                    .is_some_and(|executable| executable.has_runnable_page_work())
        }
        #[cfg(not(feature = "v8-host"))]
        {
            false
        }
    }

    pub(super) fn wait_for(&mut self, predicate: &WaitPredicate) -> Result<(), RuntimeError> {
        self.wait_for_timeout(predicate, Some(self.config.command_timeout))
    }

    pub(super) fn wait_for_timeout(
        &mut self,
        predicate: &WaitPredicate,
        timeout: Option<Duration>,
    ) -> Result<(), RuntimeError> {
        let mut progress = wait_fairness::Progress::new(predicate.clone(), timeout);
        loop {
            if progress.poll(self)? {
                return Ok(());
            }
            #[cfg(feature = "v8-host")]
            progress.pump(self)?;
            let delay = progress.next_delay();
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
        }
    }

    #[cfg(feature = "v8-host")]
    pub(super) fn reconcile_async_fetch_activity(
        &mut self,
        before: Option<(u64, bool)>,
        after: Option<(u64, bool)>,
    ) {
        if after.is_some_and(|(_, pending)| pending) {
            self.network_quiet_since = None;
        } else if before != after || before.is_some_and(|(_, pending)| pending) {
            self.network_quiet_since = Some(Instant::now());
        }
    }

    #[cfg(feature = "v8-host")]
    pub(super) fn poll_page_tasks(&mut self) -> Result<(), RuntimeError> {
        let _timing = crate::native_timing::stage("page.task_poll");
        #[cfg(unix)]
        if self.script_close_departure_completed() {
            return Ok(());
        }
        let fetch_before = self
            .executable
            .as_ref()
            .map(|executable| executable.async_fetch_activity());
        let result = self.poll_page_tasks_inner();
        let fetch_after = self
            .executable
            .as_ref()
            .map(|executable| executable.async_fetch_activity());
        self.reconcile_async_fetch_activity(fetch_before, fetch_after);
        if let Err(error) = &result
            && let Some(navigation) = &self.active_navigation
        {
            let reason = navigation
                .checkpoint()
                .err()
                .unwrap_or_else(|| error.navigation_stop());
            let _ = navigation.publish(crate::navigation_control::NavigationEvent::Stopped(reason));
            return Err(self.stop_failed_navigation_load(error.clone()));
        }
        result
    }

    #[cfg(all(unix, feature = "v8-host"))]
    pub(super) fn poll_prerender_candidates(&mut self) -> Result<(), RuntimeError> {
        let _timing = crate::native_timing::stage("page.prerender_poll");
        #[cfg(test)]
        let _candidate_phase = crate::ipc_progress::close_admission::phase("page_poll_prerender");
        if self.frame_scope.is_some() || self.pending_navigation_transaction.is_some() {
            return Ok(());
        }
        let (Some(client), Some(broker), Some(executable)) = (
            self.context_client.clone(),
            self.broker.clone(),
            self.executable.as_mut(),
        ) else {
            return Ok(());
        };
        let source = executable
            .prerender_source_snapshot()
            .map_err(map_host_error)?;
        let mut host = Some(executable.take_agent_host().map_err(map_host_error)?);
        let (high, diagnostics) = self.prerender_candidates.poll_for_page(
            prerender_roster::PrerenderPollInput {
                snapshot: source,
                host: &mut host,
                client,
                page: self.page_id,
                broker,
                source_url: &self.url,
                policy: self.referrer_policy,
                config: &self.config,
            },
            self.page_task_terminal_control.clone(),
        );
        self.last_issued_document_generation = self.last_issued_document_generation.max(high);
        if let Some(host) = host {
            match executable.return_agent_host_after_turn(host) {
                Ok(None) => {}
                Ok(Some(error)) => return Err(map_host_error(error)),
                Err((error, host)) => {
                    self.pending_agent_host = Some(*host);
                    return Err(map_host_error(error));
                }
            }
        } else {
            return Err(RuntimeError::ProtocolViolation(
                "prerender lost shared actor host".into(),
            ));
        }
        for detail in diagnostics {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::ScriptDiagnostic {
                    capability: "navigation.prerender".into(),
                    detail,
                },
            );
        }
        Ok(())
    }

    #[cfg(feature = "v8-host")]
    pub(super) fn terminal_close_requested(&self) -> bool {
        crate::page_terminal_control::retired(Some(&self.page_task_terminal_control))
    }

    #[cfg(feature = "v8-host")]
    pub(super) fn poll_page_tasks_inner(&mut self) -> Result<(), RuntimeError> {
        crate::native_timing::count("page.task_passes", 1);
        let _close_phase = crate::ipc_progress::close_admission::phase("page_poll");
        if self.terminal_close_requested() {
            return Ok(());
        }
        if let Some(executable) = self.executable.as_mut() {
            if executable.standalone_close_pending() {
                let _timing = crate::native_timing::stage("page.task_drain");
                executable.drain_tasks().map_err(map_host_error)?;
                return Ok(());
            }
            if executable.standalone_departed() {
                return Ok(());
            }
        }
        #[cfg(unix)]
        {
            self.poll_prerender_recovery();
            if self.prerender_recovery.is_some() {
                return Ok(());
            }
        }
        #[cfg(unix)]
        self.poll_initial_document_cleanup();
        self.poll_failed_install_recovery();
        if self.failed_install_recovery.is_some() {
            return Ok(());
        }
        self.poll_source_resume();
        if self.root_source_resume.is_some() || self.terminal_close_requested() {
            return Ok(());
        }
        self.poll_accessibility_operations();
        if self.terminal_close_requested() {
            return Ok(());
        }
        for pending in std::mem::take(&mut self.pending_execution_replies) {
            if self.terminal_close_requested() {
                self.pending_execution_replies.push(pending);
                continue;
            }
            if let Some(pending) = self.poll_navigation_reply(pending) {
                self.pending_execution_replies.push(pending);
            }
        }
        if self.terminal_close_requested() {
            return Ok(());
        }
        if let Err(error) = self.poll_history_publications()
            && !self
                .pending_navigation_transaction
                .as_ref()
                .is_some_and(|transaction| {
                    matches!(
                        &transaction.phase,
                        NavigationTransactionPhase::Publication { .. }
                    )
                })
        {
            return Err(error);
        }
        if self.terminal_close_requested() {
            return Ok(());
        }
        self.poll_navigation_control_checkpoint()?;
        if self.terminal_close_requested() {
            return Ok(());
        }
        if let Some(pending) = self.poll_navigation_transaction()
            && let Some(previous) = self.pending_committed_navigation_reply.replace(pending)
        {
            previous
                .reply
                .complete(self, Err(RuntimeError::NavigationCancelled));
        }
        if self.terminal_close_requested() {
            return Ok(());
        }
        if self.frame_scope.is_none()
            && let Some(reason) = self
                .active_navigation
                .as_ref()
                .and_then(|job| job.checkpoint().err())
        {
            return Err(self.stop_failed_navigation_load(RuntimeError::NavigationStopped(reason)));
        }
        if self.frame_scope.is_some() {
            {
                let _timing = crate::native_timing::stage("page.task_drain");
                self.executable
                    .as_mut()
                    .ok_or(RuntimeError::EvaluationUnavailable)?
                    .drain_tasks()
                    .map_err(map_host_error)?;
            }
            #[cfg(test)]
            if let Some(after_drain) = self.after_page_task_drain.take() {
                after_drain();
            }
            if self.terminal_close_requested() {
                return Ok(());
            }
            self.sync_executable_semantics()?;
            if self.terminal_close_requested() {
                return Ok(());
            }
            self.refresh_page_task_layout()?;
            return Ok(());
        }
        #[cfg(unix)] if self.prerender_recovery.is_some()||self.pending_navigation_transaction.as_ref().is_some_and(|transaction|matches!(&transaction.phase,NavigationTransactionPhase::Prerender(job) if job.source_suspended())){return Ok(())}
        let before = self.semantic_graph.semantic_revision();
        if self.executable.is_none() {
            return Ok(());
        }
        // A drain can run author work long enough for another thread to enqueue
        // terminal Close. Yield before starting a new rendering pass; leave all
        // retained replies and the Close acknowledgement with their current owners.
        {
            let _timing = crate::native_timing::stage("page.task_drain");
            self.executable
                .as_mut()
                .ok_or(RuntimeError::EvaluationUnavailable)?
                .drain_tasks()
                .map_err(map_host_error)?;
        }
        #[cfg(test)]
        if let Some(after_drain) = self.after_page_task_drain.take() {
            after_drain();
        }
        if self.terminal_close_requested() {
            return Ok(());
        }
        let signals = {
            let executable = self
                .executable
                .as_mut()
                .ok_or(RuntimeError::EvaluationUnavailable)?;
            if executable.standalone_departed() {
                return Ok(());
            }
            PageTaskSignals {
                console_entries: executable.take_console_entries().unwrap_or_default(),
                context_entries: executable.take_context_trace_entries().unwrap_or_default(),
                location: take_script_navigation(executable),
                history_traversal: executable
                    .take_history_traversal()
                    .map_err(map_host_error)?,
            }
        };
        // Keep extraction order unchanged. An admitted operation completes (and
        // its real error still propagates) before terminal handoff; the exact
        // already-extracted batch remains owned until real page destruction.
        let ready = (|| -> Result<bool, RuntimeError> {
            if self.terminal_close_requested() {
                return Ok(false);
            }
            #[cfg(unix)]
            self.poll_prerender_candidates()?;
            if self.terminal_close_requested() {
                return Ok(false);
            }
            self.sync_executable_semantics()?;
            if self.terminal_close_requested() {
                return Ok(false);
            }
            let response = {
                let _timing = crate::native_timing::stage("page.response_completion");
                self.finish_pending_navigation_response()?
            };
            if self.terminal_close_requested() {
                return Ok(false);
            }
            // Only an absent response proves this intervening step did no work.
            // Deferred/obsolete/completed responses retain their existing sync.
            if response != page_lifecycle_checkpoints::NavigationResponseProgress::NoWork {
                self.sync_executable_semantics()?;
            }
            Ok(!self.terminal_close_requested())
        })();
        let ready = match ready {
            Ok(ready) => ready,
            // Retirement during projection is a terminal handoff, not loss of
            // the signals already extracted above. Unbound/nonterminal yields
            // and real errors retain their original failure behavior.
            Err(RuntimeError::PageTaskYield) if self.terminal_close_requested() => false,
            Err(error) => return Err(error),
        };
        if !ready {
            self.terminal_page_task_signals = Some(signals);
            return Ok(());
        }
        let PageTaskSignals {
            console_entries,
            context_entries,
            location,
            history_traversal,
        } = signals;
        for entry in console_entries {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::Console {
                    level: entry.level,
                    message: entry.message,
                },
            );
        }
        for entry in context_entries {
            if let Some(kind) = context_trace_kind(entry) {
                self.trace.push(Revision(self.revision), None, kind);
            }
        }
        if let Some(task) = history_traversal.filter(|_| location.is_none())
            && self.begin_queued_history_navigation(task)?.is_some()
        {
            return Ok(());
        }
        if let Some((location, update)) = location {
            self.begin_script_navigation(location, update)?;
            return Ok(());
        }
        if self.terminal_close_requested() {
            return Ok(());
        }
        let after = self.semantic_graph.semantic_revision();
        if before != after
            || self.executable.as_ref().is_some_and(|executable| {
                executable.rendering_due()
                    || executable.native_animation_revision() != self.layout_animation_revision
            })
        {
            if !self.refresh_page_task_layout()? {
                return Ok(());
            }
            self.revision += 1;
            self.record_snapshot()?;
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::Mutation {
                    summary: "asynchronous page checkpoint".into(),
                },
            );
        }
        if let Err(error) = self.publish_native_rendering_opportunity() {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::ScriptDiagnostic {
                    capability: "rendering.presentation".into(),
                    detail: error.to_string(),
                },
            );
        }
        Ok(())
    }

    #[cfg(not(feature = "v8-host"))]
    pub(super) fn poll_page_tasks(&mut self) -> Result<(), RuntimeError> {
        let _timing = crate::native_timing::stage("page.task_poll");
        Ok(())
    }

    pub(super) fn wait_satisfied(
        &mut self,
        predicate: &WaitPredicate,
    ) -> Result<bool, RuntimeError> {
        #[cfg(feature = "v8-host")]
        if matches!(
            predicate,
            WaitPredicate::Lifecycle(LifecycleState::NetworkIdle)
        ) && self
            .executable
            .as_ref()
            .is_some_and(|executable| executable.async_fetch_activity().1)
        {
            #[cfg(all(test, feature = "v8-host"))]
            loading_stage_diagnostics::wait(self, "wait-async-active", String::new);
            return Ok(false);
        }

        let selector = match predicate {
            WaitPredicate::SelectorExists(selector)
            | WaitPredicate::SelectorGone(selector)
            | WaitPredicate::SelectorVisible(selector)
            | WaitPredicate::SelectorHidden(selector)
            | WaitPredicate::SelectorCount { selector, .. } => Some(selector),
            _ => None,
        };
        let nodes = if let Some(selector) = selector {
            let matches = self.native_target_selectors(&oryn_common::agent::Target::Css {
                selector: selector.clone(),
            })?;
            matches
                .get(&(false, selector.clone()))
                .into_iter()
                .flatten()
                .map(|reference| crate::semantic::unpack_node_ref(reference.node))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        Ok(match predicate {
            WaitPredicate::JavaScript(expression) => {
                self.evaluate(&format!("!!(\n{expression}\n)"))? == "true"
            }
            WaitPredicate::Lifecycle(LifecycleState::NetworkIdle) => {
                let mut quiet_since = self.network_quiet_since;
                if let Some(broker) = self.broker.as_ref() {
                    let (active, changed) = broker.owned_operation_activity();
                    #[cfg(all(test, feature = "v8-host"))]
                    loading_stage_diagnostics::wait(self, "wait-broker", || {
                        format!(
                            "active={active} changed={changed:?} state_quiet={:?}",
                            self.network_quiet_since
                        )
                    });
                    if active != 0 {
                        return Ok(false);
                    }
                    quiet_since = quiet_since.max(changed);
                }
                #[cfg(all(unix, feature = "v8-host"))]
                if let Some(executable) = self.executable.as_ref() {
                    let (active, changed) = executable.resource_job_activity();
                    #[cfg(test)]
                    loading_stage_diagnostics::wait(self, "wait-native-jobs", || {
                        format!("active={active} changed={changed:?}")
                    });
                    if active != 0 {
                        return Ok(false);
                    }
                    quiet_since = quiet_since.max(changed);
                }
                let now = Instant::now();
                #[cfg(test)]
                let now = network_idle_test_clock::now().unwrap_or(now);
                let satisfied = quiet_since.is_some_and(|since| {
                    now.saturating_duration_since(since) >= NETWORK_IDLE_QUIET_PERIOD
                });
                #[cfg(all(test, feature = "v8-host"))]
                loading_stage_diagnostics::wait(self, "wait-decision", || {
                    format!(
                        "satisfied={satisfied} now={:?} quiet_since={quiet_since:?} period={NETWORK_IDLE_QUIET_PERIOD:?}",
                        Instant::now()
                    )
                });
                satisfied
            }
            WaitPredicate::Lifecycle(state) => self.lifecycle.contains(state),
            WaitPredicate::UrlContains(fragment) => self.url.contains(fragment),
            WaitPredicate::SelectorExists(_) => !nodes.is_empty(),
            WaitPredicate::SelectorGone(_) => nodes.is_empty(),
            WaitPredicate::SelectorCount { minimum, .. } => nodes.len() as u64 >= *minimum,
            WaitPredicate::SelectorVisible(_) => {
                self.document.as_ref().is_some_and(|document| {
                    nodes.iter().copied().any(|id| {
                        #[cfg(feature = "v8-host")]
                        if let Some(geometry) = self
                            .layout
                            .as_ref()
                            .and_then(|layout| layout.geometry.get(&pack_node_id(id)))
                        {
                            // Unsupported geometry is not the same as hidden:
                            // table content, for example, can be visibly
                            // present even though bounded layout intentionally exposes no
                            // table box geometry.
                            return geometry.availability
                                != oryn_common::v4::GeometryAvailability::Hidden;
                        }
                        project_element(
                            &document.dom,
                            id,
                            self.page_id,
                            self.document_generation,
                            id.slot + 1,
                        )
                        .is_some_and(|node| {
                            node.states.contains(&oryn_common::v3::NodeState::Visible)
                        })
                    })
                })
            }
            WaitPredicate::SelectorHidden(_) => self.document.as_ref().is_some_and(|document| {
                let mut nodes = nodes.iter().copied().peekable();
                nodes.peek().is_none()
                    || nodes.all(|id| {
                        #[cfg(feature = "v8-host")]
                        if let Some(geometry) = self
                            .layout
                            .as_ref()
                            .and_then(|layout| layout.geometry.get(&pack_node_id(id)))
                        {
                            return geometry.availability
                                == oryn_common::v4::GeometryAvailability::Hidden;
                        }
                        project_element(
                            &document.dom,
                            id,
                            self.page_id,
                            self.document_generation,
                            id.slot + 1,
                        )
                        .is_some_and(|node| {
                            node.states.contains(&oryn_common::v3::NodeState::Hidden)
                        })
                    })
            }),
        })
    }
}

#[cfg(all(test, feature = "v8-host"))]
mod tests {
    use super::*;
    use crate::host::{DocumentInput, DocumentStream, HostError};
    use std::{cell::Cell, rc::Rc, task::Poll};

    fn page() -> PageState {
        let page = PageState::new(
            PageId(829),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            None,
            #[cfg(unix)]
            None,
        );
        page.bind_direct_fixture_terminal_owner();
        page
    }

    fn settled_page() -> PageState {
        // This fixture explicitly binds its owner; unknown ownership must still
        // fail closed in the separate negative below.
        let mut page = page();
        page.load_document(
            "https://projection.test/".into(),
            "<p>Stable document</p>",
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
        for _ in 0..64 {
            let executable = page.executable.as_mut().unwrap();
            executable.drain_tasks().unwrap();
            if !executable.parser_pending() {
                break;
            }
        }
        assert!(!page.executable.as_ref().unwrap().parser_pending());
        page.sync_executable_semantics().unwrap();
        page.refresh_layout_geometry().unwrap();
        assert!(page.pending_navigation_response.is_none());
        page
    }

    fn waiting_page() -> (PageState, Rc<Cell<usize>>) {
        struct WaitingInput(Rc<Cell<usize>>);
        impl DocumentInput for WaitingInput {
            fn poll_next(&mut self) -> Poll<Option<Result<String, HostError>>> {
                self.0.set(self.0.get() + 1);
                Poll::Pending
            }
        }
        let mut page = page();
        let url = "https://projection.test/waiting";
        // A live task pump publishes native history even while its parser is
        // waiting. Seed the same owned history and executable resources as a
        // real navigation, rather than attaching a document to an empty page.
        let generation = advance_document_generation(
            page.last_issued_document_generation,
            page.config.document_generation_base,
        )
        .unwrap();
        let (history, index) = page
            .prepare_navigation_history(url, HistoryUpdate::Push, generation)
            .unwrap();
        let visible = PageState::visible_navigation_history(&history, index, "push").unwrap();
        let session = PageState::full_session_history(&history, index, "push").unwrap();
        assert_eq!(session.entries.len(), history.len());
        assert_eq!(session.current, index);
        let resources = BTreeMap::from([
            (
                "oryn:navigation-history".into(),
                serde_json::to_string(&visible).unwrap(),
            ),
            (
                "oryn:session-history".into(),
                serde_json::to_string(&session).unwrap(),
            ),
        ]);
        page.history = history;
        page.history_index = Some(index);
        page.url = url.into();
        page.document_generation = generation;
        page.last_issued_document_generation = generation;
        let polls = Rc::new(Cell::new(0));
        let mut executable = crate::executable::RustExecutableDocument::load_stream_with_limits(
            DocumentStream {
                content_language: None,
                chunks: Box::new(WaitingInput(polls.clone())),
                content_type: None,
                encoding: Default::default(),
            },
            &resources,
            None,
            Some(Url::parse(&page.url).unwrap()),
            256 * 1024 * 1024,
            true,
        )
        .unwrap();
        executable.adopt_page_terminal_control(page.page_task_terminal_control.clone());
        page.document = Some(executable.document.clone());
        page.executable = Some(executable);
        assert!(page.executable.as_ref().unwrap().parser_pending());
        (page, polls)
    }

    fn response(page: &mut PageState, completed: bool) -> Arc<Mutex<NavigationBodyCapture>> {
        let capture = Arc::new(Mutex::new(NavigationBodyCapture {
            source: "<p>Captured response</p>".into(),
            bytes: 24,
            completed,
        }));
        let root_identity = page
            .executable
            .as_ref()
            .unwrap()
            .form_history_root_identity()
            .map(|(realm, generation, epoch)| (u64::from(realm.0), generation, epoch));
        assert!(root_identity.is_some());
        page.pending_navigation_response = Some(PendingNavigationResponse {
            generation: page.document_generation,
            root_identity,
            current: Url::parse(&page.url).unwrap(),
            status: 200,
            response_headers: Vec::new(),
            essence: "text/html".into(),
            capture: capture.clone(),
        });
        capture
    }

    // Later rendering has its own required projection. Measure only the actual
    // response-completion suffix before a new layout operation starts.
    fn response_syncs(labels: &[&str]) -> usize {
        let completion = labels
            .iter()
            .position(|label| *label == "navigation_response_completion")
            .expect("real response completion boundary was reached");
        assert!(labels[..completion].contains(&"semantic_projection"));
        labels[completion + 1..]
            .iter()
            .take_while(|label| **label != "layout")
            .filter(|label| **label == "semantic_projection")
            .count()
    }

    #[test]
    fn absent_response_skips_only_redundant_scheduler_projection() {
        let mut page = settled_page();
        assert_eq!(
            page.finish_pending_navigation_response().unwrap(),
            page_lifecycle_checkpoints::NavigationResponseProgress::NoWork
        );
        let (result, labels) =
            crate::ipc_progress::close_admission::capture_test_phase_labels(|| {
                page.poll_page_tasks_inner()
            });
        result.unwrap();
        assert_eq!(response_syncs(&labels), 0);
        assert!(page.pending_navigation_response.is_none());
        assert!(page.document.is_some());
    }

    #[test]
    fn unbound_semantic_publication_is_not_successful_terminal_handoff() {
        let mut page = PageState::new(
            PageId(839), None, BrowserConfig::default(), ContextOptions::default(),
            None, #[cfg(unix)] None,
        );
        page.load_document("https://projection.test/unbound".into(), "<p>Unbound</p>",
            None, None, HistoryUpdate::Push, &BTreeMap::new(), Vec::new()).unwrap();
        assert!(page.page_task_terminal_control.controller().is_none());
        assert!(!page.terminal_close_requested());
        let cursor = page.executable.as_ref().unwrap().semantic_mutations().next;
        assert!(matches!(page.poll_page_tasks_inner(), Err(RuntimeError::PageTaskYield)));
        assert!(page.terminal_page_task_signals.is_none());
        assert_eq!(page.executable.as_ref().unwrap().semantic_mutations().next, cursor);
        assert!(!page.terminal_close_requested());
    }

    #[test]
    fn pending_parser_response_retains_capture_and_followup_projection() {
        let (mut page, polls) = waiting_page();
        let capture = response(&mut page, false);
        assert_eq!(
            page.finish_pending_navigation_response().unwrap(),
            page_lifecycle_checkpoints::NavigationResponseProgress::Processed
        );
        let (result, labels) =
            crate::ipc_progress::close_admission::capture_test_phase_labels(|| {
                page.poll_page_tasks_inner()
            });
        result.unwrap();
        assert!(polls.get() > 0, "real retained input was polled");
        assert_eq!(response_syncs(&labels), 1);
        let pending = page.pending_navigation_response.as_ref().unwrap();
        assert!(Arc::ptr_eq(&pending.capture, &capture));
        assert_eq!(capture.lock().unwrap().source, "<p>Captured response</p>");
        assert!(!capture.lock().unwrap().completed);
    }

    #[test]
    fn completed_response_keeps_followup_projection() {
        let mut page = settled_page();
        let capture = response(&mut page, true);
        let (result, labels) =
            crate::ipc_progress::close_admission::capture_test_phase_labels(|| {
                page.poll_page_tasks_inner()
            });
        result.unwrap();
        assert_eq!(response_syncs(&labels), 1);
        assert!(page.pending_navigation_response.is_none());
        assert!(capture.lock().unwrap().source.is_empty());
    }

    #[test]
    fn failed_response_preserves_original_error_without_extra_projection() {
        let mut page = settled_page();
        let capture = response(&mut page, false);
        let (result, labels) =
            crate::ipc_progress::close_admission::capture_test_phase_labels(|| {
                page.poll_page_tasks_inner()
            });
        assert!(matches!(
            result,
            Err(RuntimeError::Network(message))
                if message == "navigation parser stopped before response completion"
        ));
        assert_eq!(response_syncs(&labels), 0);
        // This is the existing failure contract: the extracted response is
        // rejected, while its independently retained capture was not consumed.
        assert!(page.pending_navigation_response.is_none());
        assert_eq!(capture.lock().unwrap().source, "<p>Captured response</p>");
    }

    #[test]
    fn retirement_during_response_step_retains_pending_capture_and_signals() {
        let (mut page, _) = waiting_page();
        let capture = response(&mut page, false);
        let control = page.page_task_terminal_control.controller().cloned().unwrap();
        let (result, labels) =
            crate::ipc_progress::close_admission::capture_test_phase_labels_with_hook(
                || page.poll_page_tasks_inner(),
                move |label| {
                    if label == "navigation_response_completion" {
                        control.retire();
                    }
                },
            );
        result.unwrap();
        assert_eq!(response_syncs(&labels), 0);
        assert!(page.terminal_close_requested());
        assert!(page.terminal_page_task_signals.is_some());
        assert!(Arc::ptr_eq(
            &page.pending_navigation_response.as_ref().unwrap().capture,
            &capture
        ));
        assert_eq!(capture.lock().unwrap().source, "<p>Captured response</p>");
        assert!(!labels.contains(&"layout"));
    }
}
