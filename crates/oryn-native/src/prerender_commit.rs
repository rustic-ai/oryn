//! Actor-owned activation. An unanswered publishing request is not a failed
//! activation: both roots remain owned until the exact request is resolved.
use super::*;
use std::task::Poll;

pub(super) struct Commit {
    // Keep the retained executable out of navigation phase values. Otherwise
    // every match/move of the phase reserves a full document on the call stack.
    candidate: Option<Box<prerender_load::CandidateDocument>>,
    reservation: Option<prerender_load::Reservation>,
    departure: RootDeparture,
    status: u16,
    source_index: usize,
    source_realm: oryn_js::realm::RealmId,
    destination_realm: oryn_js::realm::RealmId,
    destination_identity: Option<(oryn_js::realm::RealmId, u64, u64)>,
    destination_epoch: Option<u64>,
    original_history: Vec<NavigationHistoryEntry>,
    history_update: HistoryUpdate,
    activation_from: Option<oryn_js::navigation_api::NavigationHistorySnapshot>,
    joint: Option<oryn_js::joint_history::Timeline>,
    transport: Option<prerender_load::PrerenderActivationTransport>,
    tree: Option<crate::host::NativePrerenderActivationTree>,
    history_receipt: Option<u64>,
    interactive_published: bool,
    phase: u8,
    retry: Option<Instant>,
    installed: bool,
    recovering_successor: bool,
    retired: bool,
    retirement: Option<navigation_fetch::PendingPolicy>,
    retirement_acked: bool,
    retirement_frozen: bool,
    retirement_targets: VecDeque<oryn_js::message_wire::MessageRequest>,
    #[cfg(test)]
    diagnostic_failure: Option<String>,
}
fn same_retirement(
    left: &oryn_js::message_wire::MessageRequest,
    right: &oryn_js::message_wire::MessageRequest,
) -> bool {
    use oryn_js::message_wire::MessageRequest as Q;
    match (left, right) {
        (
            Q::RetireRootDocument {
                document_generation: a,
                stream_epoch: b,
            },
            Q::RetireRootDocument {
                document_generation: c,
                stream_epoch: d,
            },
        ) => a == c && b == d,
        (
            Q::RetireWindow {
                frame: a,
                document_generation: b,
                stream_epoch: c,
            },
            Q::RetireWindow {
                frame: d,
                document_generation: e,
                stream_epoch: f,
            },
        ) => a == d && b == e && c == f,
        _ => false,
    }
}
/// Keep detached historical owners first, then the current postorder tree.
/// Appending only newly discovered children would place a new grandchild after
/// its already captured parent, revoking its cleanup authority too early.
fn merge_retirement_targets(
    existing: &mut VecDeque<oryn_js::message_wire::MessageRequest>,
    current: VecDeque<oryn_js::message_wire::MessageRequest>,
) {
    existing.retain(|old| !current.iter().any(|target| same_retirement(old, target)));
    existing.extend(current);
}
impl Commit {
    #[cfg(test)]
    fn diagnostic_for_test(&self) -> String {
        repair44_bounded_diagnostic(format_args!(
            "phase={} installed={} successor_recovery={} retired={} retirement_ack={} frozen={} pending_retirement={} retirement_targets={} pending_transport={} first_failure={:?}",
            self.phase,
            self.installed,
            self.recovering_successor,
            self.retired,
            self.retirement_acked,
            self.retirement_frozen,
            self.retirement.is_some(),
            self.retirement_targets.len(),
            self.transport.is_some(),
            self.diagnostic_failure
        ))
    }
    pub fn new(
        page: &mut PageState,
        candidate: Box<prerender_load::CandidateDocument>,
        history: HistoryUpdate,
    ) -> Result<Self, (RuntimeError, Box<prerender_load::CandidateDocument>)> {
        let prepared = (|| {
            let source_index = page.history_index.ok_or(RuntimeError::HistoryBoundary)?;
            let source = page
                .executable
                .as_ref()
                .ok_or(RuntimeError::NavigationCancelled)?;
            let source_realm = source.agent_root_realm();
            let from = page
                .history
                .get(source_index)
                .and_then(|entry| entry.navigation.clone())
                .filter(|_| source.can_disclose_navigation_destination(&candidate.url));
            let joint = source.joint_history_snapshot().map_err(map_host_error)?;
            let mut departure = page.prepare_root_departure_at_generation(
                &candidate.url,
                history,
                false,
                false,
                candidate.reservation.generation,
            )?;
            departure.history[departure.index].request_state =
                Some(candidate.request_state.clone());
            departure.history[departure.index].origin = candidate.origin.clone();
            Ok((source_index, source_realm, from, joint, departure))
        })();
        match prepared {
            Ok((source_index, source_realm, activation_from, joint, departure)) => Ok(Self {
                status: candidate.status,
                destination_realm: candidate.executable.agent_root_realm(),
                destination_identity: None,
                destination_epoch: None,
                candidate: Some(candidate),
                reservation: None,
                departure,
                source_index,
                source_realm,
                original_history: page.history.clone(),
                history_update: history,
                activation_from,
                joint: Some(joint),
                transport: None,
                tree: None,
                history_receipt: None,
                interactive_published: false,
                phase: 0,
                retry: None,
                installed: false,
                recovering_successor: false,
                retired: false,
                retirement: None,
                retirement_acked: false,
                retirement_frozen: false,
                retirement_targets: VecDeque::new(),
                #[cfg(test)]
                diagnostic_failure: None,
            }),
            Err(error) => Err((error, candidate)),
        }
    }
    pub fn delay(&self) -> Duration {
        self.retry
            .map(|time| time.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|| {
                self.transport
                    .as_ref()
                    .map_or(Duration::ZERO, |transport| transport.delay())
            })
    }
    pub fn source_suspended(&self) -> bool {
        self.phase >= 1
    }
    pub fn installed(&self) -> bool {
        self.installed
    }
    pub fn status(&self) -> u16 {
        self.status
    }
    pub fn publication_milestone(&self) -> u8 {
        u8::from(self.interactive_published)
    }
    pub fn destination(&self) -> oryn_js::realm::RealmId {
        self.destination_realm
    }
    pub fn poll(&mut self, page: &mut PageState) -> Result<bool, RuntimeError> {
        if self.retry.is_some_and(|time| time > Instant::now()) {
            return Ok(false);
        }
        self.retry = None;
        let result = self.advance(page);
        match &result {
            Err(_) => self.retry = Some(Instant::now() + Duration::from_millis(25)),
            Ok(false) => self.retry = Some(Instant::now() + Duration::from_millis(16)),
            Ok(true) => {}
        }
        result
    }
    fn advance(&mut self, page: &mut PageState) -> Result<bool, RuntimeError> {
        if self.installed
            && page.executable.as_ref().is_none_or(|executable| {
                executable.agent_root_realm() != self.destination_realm
                    || executable.form_history_root_identity() != self.destination_identity
                    || executable.native_navigation_load_epoch() != self.destination_epoch
            })
        {
            return Err(RuntimeError::NavigationCancelled);
        }
        if self.phase == 0 {
            if !page.poll_root_departure(&mut self.departure)? {
                return Ok(false);
            }
            if self.retirement_targets.is_empty() {
                let executable = page
                    .executable
                    .as_mut()
                    .ok_or(RuntimeError::NavigationCancelled)?;
                let mut host = executable.take_agent_host().map_err(map_host_error)?;
                let targets = host.prerender_retirement_documents(self.destination_realm);
                match executable.return_agent_host_after_turn(host) {
                    Ok(None) => {}
                    Ok(Some(error)) => return Err(map_host_error(error)),
                    Err((error, host)) => {
                        page.pending_agent_host = Some(*host);
                        return Err(map_host_error(error));
                    }
                }
                self.retirement_targets = targets.map_err(map_host_error)?;
            }
            page.prepared_root_source = self.departure.source.take();
            page.suspend_document_with_transition(true, Some(self.departure.persisted))?;
            // Suspension has transferred the sole host before any publishing
            // request starts. A later retry never dispatches pagehide again.
            self.phase = 1;
            self.original_history = page.history.clone();
            for entry in &mut self.departure.history {
                if let Some(saved) = page.history.iter().find(|saved| {
                    saved
                        .navigation
                        .as_ref()
                        .zip(entry.navigation.as_ref())
                        .is_some_and(|(saved, current)| saved.id == current.id)
                }) {
                    entry.target_name = saved.target_name.clone();
                    entry.form_state = saved.form_state.clone();
                    entry.frame_form_state = saved.frame_form_state.clone();
                }
            }
        }
        if self.phase == 1 {
            if self.transport.is_none() {
                let predecessor = page
                    .pending_agent_host
                    .as_ref()
                    .ok_or(RuntimeError::EvaluationUnavailable)?
                    .parked_prerender_predecessor(self.source_realm)
                    .map_err(map_host_error)?;
                let client = page
                    .context_client
                    .clone()
                    .ok_or(RuntimeError::EvaluationUnavailable)?;
                self.transport = Some(prerender_load::PrerenderActivationTransport::new(
                    client,
                    page.page_id,
                    &self.candidate.as_ref().unwrap().reservation,
                    predecessor,
                ));
            }
            match self.transport.as_mut().unwrap().poll() {
                Poll::Pending => return Ok(false),
                Poll::Ready(false) => return Err(RuntimeError::NavigationCancelled),
                Poll::Ready(true) => self.phase = 2,
            }
        }
        #[cfg(test)]
        if self.phase == 2 && PAUSE_AFTER_PROMOTION.with(|pause| pause.replace(false)) {
            return Ok(false);
        }
        if self.phase == 2 {
            let candidate = self.candidate.as_mut().unwrap();
            if !candidate.executable.has_agent_host() {
                let host = page
                    .pending_agent_host
                    .as_mut()
                    .ok_or(RuntimeError::EvaluationUnavailable)?;
                host.publish_prerender_root(
                    self.source_realm,
                    self.destination_realm,
                    &candidate.reservation.token,
                    candidate.reservation.generation,
                    candidate.reservation.epoch,
                )
                .map_err(map_host_error)?;
                let host = page.pending_agent_host.take().unwrap();
                if let Err((error, host)) = candidate.executable.install_agent_host(host) {
                    page.pending_agent_host = Some(*host);
                    return Err(map_host_error(error));
                }
            }
            if self.destination_identity.is_none() {
                self.destination_identity = candidate.executable.form_history_root_identity();
                self.destination_epoch = candidate.executable.native_navigation_load_epoch();
            }
            if let Some(joint) = self.joint.as_ref() {
                candidate
                    .executable
                    .restore_joint_history(joint.clone())
                    .map_err(map_host_error)?;
                self.joint = None;
            }
            let kind = match self.history_update {
                HistoryUpdate::Replace => "replace",
                _ => "push",
            };
            if self.history_receipt.is_none() {
                let history = PageState::visible_navigation_history(
                    &self.departure.history,
                    self.departure.index,
                    kind,
                )?;
                let session = PageState::full_session_history(
                    &self.departure.history,
                    self.departure.index,
                    kind,
                )?;
                self.history_receipt = Some(
                    candidate
                        .executable
                        .start_native_history_transaction(
                            history,
                            session,
                            Some(self.activation_from.clone()),
                        )
                        .map_err(map_host_error)?,
                );
            }
            if !candidate
                .executable
                .resume_native_history_publication(self.history_receipt.unwrap())
                .map_err(map_host_error)?
            {
                return Ok(false);
            }
            #[cfg(test)]
            if REPLACE_AFTER_HISTORY.with(|fault| fault.replace(false)) {
                candidate.executable.execute("document.open();document.write('<title>authored successor</title><body>new stream</body>');document.close();globalThis.successorDocument=document").map_err(map_host_error)?;
                return Ok(false);
            }
            self.prepare_activation_tree(page)?;
            self.phase = 3;
        }
        if self.phase == 3 {
            let revision =
                page.revision
                    .checked_add(1)
                    .ok_or_else(|| RuntimeError::IdentityExhausted {
                        namespace: "page revision".into(),
                    })?;
            let response_url = Url::parse(&self.candidate.as_ref().unwrap().url)
                .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
            let candidate = self.candidate.take().unwrap();
            let prerender_load::CandidateDocument {
                reservation,
                executable,
                url,
                status,
                content_type,
                headers,
                broker,
                csp,
                capture,
                ..
            } = *candidate;
            let root_identity = executable
                .form_history_root_identity()
                .map(|(realm, generation, epoch)| (u64::from(realm.0), generation, epoch));
            page.pending_navigation_response = Some(PendingNavigationResponse {
                generation: reservation.generation,
                root_identity,
                current: response_url,
                status,
                response_headers: headers.clone(),
                essence: content_type
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase(),
                capture,
            });
            page.revision = revision;
            page.frame_states.clear();
            page.document_generation = reservation.generation;
            page.last_issued_document_generation = page
                .last_issued_document_generation
                .max(reservation.generation);
            self.reservation = Some(reservation);
            page.url = url;
            page.broker = Some(broker);
            page.csp = csp;
            page.script_isolation =
                crate::agent_isolation::DocumentIsolationPolicy::from_headers(&headers);
            page.referrer_policy =
                crate::referrer::ReferrerPolicy::default().after_response(&headers);
            page.response_policy_headers =
                crate::standalone_document_policy::retained_headers(&headers);
            page.history = self.departure.history.clone();
            page.history_index = Some(self.departure.index);
            page.document = Some(executable.document.clone());
            page.semantic_graph
                .reset(&executable.document.dom, page.document_generation);
            page.semantic_graph
                .set_frame_ids(executable.semantic_frame_ids());
            page.semantic_publication_pending = true;
            page.executable = Some(executable);
            page.layout = None;
            page.layout_input_cache = None;
            page.scroll_state = ScrollState {
                content_width: page.viewport.width,
                content_height: page.viewport.height,
                ..ScrollState::default()
            };
            page.lifecycle = vec![LifecycleState::Commit];
            if page
                .executable
                .as_ref()
                .is_some_and(|page| page.document_interactive())
            {
                page.lifecycle.push(LifecycleState::DomContentLoaded);
            }
            if page
                .executable
                .as_ref()
                .is_some_and(|page| page.document_load_complete())
            {
                page.lifecycle.push(LifecycleState::Load);
            }
            self.installed = true;
            self.phase = 4;
        }
        #[cfg(test)]
        if self.phase == 4 && PAUSE_AFTER_INSTALL.with(|pause| pause.replace(false)) {
            return Ok(false);
        }
        if self.phase == 4 {
            if let Some(emitter) = page
                .active_navigation
                .as_ref()
                .filter(|_| !page.navigation_committed)
            {
                emitter
                    .publish(crate::navigation_control::NavigationEvent::Committed {
                        document_generation: page.document_generation,
                        url: page.url.clone(),
                    })
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                page.navigation_committed = true;
            }
            let executable = page
                .executable
                .as_mut()
                .filter(|page| page.agent_root_realm() == self.destination_realm)
                .ok_or(RuntimeError::NavigationCancelled)?;
            let mut host = executable.take_agent_host().map_err(map_host_error)?;
            let result = host.resume_prerender_activation(self.tree.as_mut().unwrap());
            let returned =
                executable
                    .return_agent_host_after_turn(host)
                    .map_err(|(error, host)| {
                        page.pending_agent_host = Some(*host);
                        map_host_error(error)
                    })?;
            if let Some(error) = returned {
                return Err(map_host_error(error));
            }
            if !result.map_err(map_host_error)? {
                return Ok(false);
            }
            if self.tree.as_ref().unwrap().canceled() {
                return Err(RuntimeError::NavigationCancelled);
            }
            self.phase = 5;
        }
        if self.phase == 5 && self.recovering_successor {
            // The canceled caller no longer owns load completion for the new
            // authored stream. Its accepted Document remains current; normal
            // page task scheduling owns that stream from this point onward.
            page.pending_navigation_response = None;
            self.reservation.take().unwrap().activated();
            if matches!(self.history_update, HistoryUpdate::Push) {
                page.retain_cached_documents_through(self.source_index)
            }
            if matches!(self.history_update, HistoryUpdate::Replace) {
                page.evict_cached_document(self.source_index)
            }
            page.bound_cached_documents();
            self.phase = 6;
        }
        if self.phase == 5 {
            // The created Document may still own a parser/network body. Move
            // that exact driver through activation; do not refetch or report
            // load completion merely because its context became visible.
            let executable = page
                .executable
                .as_mut()
                .ok_or(RuntimeError::NavigationCancelled)?;
            executable.drain_tasks().map_err(map_host_error)?;
            executable.poll_context_events().map_err(map_host_error)?;
            if executable.form_history_root_identity() != self.destination_identity
                || executable.native_navigation_load_epoch() != self.destination_epoch
            {
                return Err(RuntimeError::NavigationCancelled);
            }
            if executable.document_interactive() && !self.interactive_published {
                if let Some(emitter) = page.active_navigation.as_ref() {
                    emitter
                        .publish(crate::navigation_control::NavigationEvent::Interactive {
                            document_generation: page.document_generation,
                        })
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                    self.interactive_published = true;
                }
                if !page.lifecycle.contains(&LifecycleState::DomContentLoaded) {
                    page.lifecycle.push(LifecycleState::DomContentLoaded);
                }
            }
            if !executable.document_load_complete() {
                return Ok(false);
            }
            if executable.form_history_root_identity() != self.destination_identity
                || executable.native_navigation_load_epoch() != self.destination_epoch
            {
                return Err(RuntimeError::NavigationCancelled);
            }
            page.sync_executable_semantics()?;
            page.finish_pending_navigation_response()?;
            // The retained native load lifecycle owns its initial pageshow.
            // Neither private completion nor activation dispatches a second.
            self.reservation.take().unwrap().activated();
            if matches!(self.history_update, HistoryUpdate::Push) {
                page.retain_cached_documents_through(self.source_index)
            }
            if matches!(self.history_update, HistoryUpdate::Replace) {
                page.evict_cached_document(self.source_index)
            }
            page.bound_cached_documents();
            self.phase = 6;
        }
        Ok(true)
    }
    fn prepare_activation_tree(&mut self, page: &mut PageState) -> Result<(), RuntimeError> {
        if self.tree.is_none() {
            let candidate = self
                .candidate
                .as_mut()
                .ok_or(RuntimeError::NavigationCancelled)?;
            let mut host = candidate
                .executable
                .take_agent_host()
                .map_err(map_host_error)?;
            let prepared = (|| {
                host.check_prerender_activation_authority(
                    self.destination_realm,
                    candidate.reservation.generation,
                    candidate.reservation.epoch,
                )?;
                let tree = host.prepare_prerender_activation(self.destination_realm)?;
                let targets = host.prerender_retirement_documents(self.destination_realm)?;
                Ok::<_, crate::host::HostError>((tree, targets))
            })();
            match candidate.executable.return_agent_host_after_turn(host) {
                Ok(None) => {}
                Ok(Some(error)) => return Err(map_host_error(error)),
                Err((error, host)) => {
                    page.pending_agent_host = Some(*host);
                    return Err(map_host_error(error));
                }
            }
            let (tree, targets) = prepared.map_err(map_host_error)?;
            // Keep old removed children for exact cleanup as well as any
            // children created by the retained history publication.
            merge_retirement_targets(&mut self.retirement_targets, targets);
            self.tree = Some(tree);
            self.destination_identity = candidate.executable.form_history_root_identity();
            self.destination_epoch = candidate.executable.native_navigation_load_epoch();
        }
        Ok(())
    }
    /// Cancellation keeps the fixed publishing request until a definitive ACK.
    /// After that ACK, retire the candidate before restoring the predecessor:
    /// an older in-flight replay can no longer publish it a second time.
    pub fn recover(&mut self, page: &mut PageState) -> Result<bool, RuntimeError> {
        if self.retry.is_some_and(|time| time > Instant::now()) {
            return Ok(false);
        }
        if let Some(transport) = self.transport.as_mut()
            && transport.poll().is_pending()
        {
            return Ok(false);
        }
        if self.recovering_successor {
            if !self.installed
                || page.executable.as_ref().is_some_and(|executable| {
                    executable.form_history_root_identity() == self.destination_identity
                        && executable.native_navigation_load_epoch() == self.destination_epoch
                })
            {
                return self.advance(page);
            }
            self.recovering_successor = false;
        }
        if !self.installed
            && self.phase >= 2
            && self.destination_identity.is_some()
            && self.candidate.as_ref().is_some_and(|candidate| {
                candidate.executable.form_history_root_identity() != self.destination_identity
                    || candidate.executable.native_navigation_load_epoch() != self.destination_epoch
            })
        {
            // Parent and root-map promotion are committed. A history callback
            // has replaced the stream in that accepted Document; cancellation
            // of the old caller cannot restore its predecessor over it.
            let candidate = self.candidate.as_mut().unwrap();
            let host = candidate
                .executable
                .take_agent_host()
                .map_err(map_host_error)?;
            let checked = host.check_prerender_activation_authority(
                self.destination_realm,
                candidate.reservation.generation,
                candidate.reservation.epoch,
            );
            if checked.is_ok()
                && let Some(receipt) = self.history_receipt.take()
            {
                host.cancel_native_history_receipt(self.destination_realm, receipt);
            }
            match candidate.executable.return_agent_host_after_turn(host) {
                Ok(_) => {}
                Err((error, host)) => {
                    page.pending_agent_host = Some(*host);
                    return Err(map_host_error(error));
                }
            }
            checked.map_err(map_host_error)?;
            if let Some(tree) = self.tree.as_mut() {
                tree.cancel();
            }
            self.tree = None;
            self.prepare_activation_tree(page)?;
            self.recovering_successor = true;
            self.phase = 3;
            return self.advance(page);
        }
        if self.installed
            && page.executable.as_ref().is_some_and(|executable| {
                executable.agent_root_realm() == self.destination_realm
                    && executable.form_history_root_identity() == self.destination_identity
            })
        {
            // Local document/history publication is the commit boundary. A
            // canceled waiter, stopped load, or later body failure cannot undo
            // that navigation and restore the predecessor over its Document.
            // Finish only the retained activation event suffix; ordinary page
            // work owns any parser continuation after the caller has ended.
            self.destination_epoch = page
                .executable
                .as_ref()
                .and_then(|executable| executable.native_navigation_load_epoch());
            self.recovering_successor = true;
            return self.advance(page);
        }
        if self.installed
            && page.executable.as_ref().is_some_and(|executable| {
                executable.agent_root_realm() == self.destination_realm
                    && (executable.form_history_root_identity() != self.destination_identity
                        || executable.native_navigation_load_epoch() != self.destination_epoch)
            })
        {
            // An authored replacement in the already-published context owns a
            // newer Document/stream. Disposal of this old activation must not
            // retire that successor or restore the predecessor over it.
            if let Some(tree) = self.tree.as_mut() {
                tree.cancel();
            }
            if let Some(reservation) = self.reservation.take() {
                reservation.activated();
            }
            if matches!(self.history_update, HistoryUpdate::Replace) {
                page.evict_cached_document(self.source_index)
            }
            return Ok(true);
        }
        // Activation callbacks may create new child Documents after the
        // pre-activation snapshot. Freeze those exact cleanup owners before
        // issuing the first retirement; no author task runs during recovery.
        if self.transport.is_some() && !self.retirement_frozen {
            let executable = if self.installed {
                page.executable.as_mut()
            } else {
                self.candidate
                    .as_mut()
                    .map(|candidate| &mut candidate.executable)
            };
            let targets = if let Some(executable) =
                executable.filter(|executable| executable.has_agent_host())
            {
                let mut host = executable.take_agent_host().map_err(map_host_error)?;
                let targets = host.prerender_retirement_documents(self.destination_realm);
                match executable.return_agent_host_after_turn(host) {
                    Ok(_) => {}
                    Err((error, host)) => {
                        page.pending_agent_host = Some(*host);
                        return Err(map_host_error(error));
                    }
                }
                targets.map_err(map_host_error)?
            } else if let Some(host) = page.pending_agent_host.as_mut() {
                host.prerender_retirement_documents(self.destination_realm)
                    .map_err(map_host_error)?
            } else {
                return Err(RuntimeError::EvaluationUnavailable);
            };
            merge_retirement_targets(&mut self.retirement_targets, targets);
            self.retirement_frozen = true;
        }
        if self.transport.is_some() && !self.retirement_acked {
            if self.retirement.is_none() {
                let request = self.retirement_targets.front().cloned().ok_or_else(|| {
                    RuntimeError::ProtocolViolation("prerender retirement owner list absent".into())
                })?;
                self.retirement =
                    Some(navigation_fetch::PendingPolicy::start_prerender_retirement(
                        page.context_client
                            .clone()
                            .ok_or(RuntimeError::EvaluationUnavailable)?,
                        crate::context_service::ContextServiceRequest::MessageWire {
                            source_page_id: page.page_id,
                            request,
                        },
                    )?);
            }
            let result = match self.retirement.as_mut().unwrap().poll() {
                Poll::Pending => return Ok(false),
                Poll::Ready(result) => result,
            };
            self.retirement = None;
            match result {
                Ok(crate::context_service::ContextServiceResponse::MessageWire {
                    result: Ok(oryn_js::message_wire::MessageResponse::Done),
                })
                | Err(RuntimeError::Closed | RuntimeError::PageNotFound(_)) => {
                    self.retirement_targets.pop_front();
                    self.retirement_acked = self.retirement_targets.is_empty();
                }
                Err(error) => return Err(error),
                _ => {
                    return Err(RuntimeError::ProtocolViolation(
                        "prerender retirement acknowledgement mismatch".into(),
                    ));
                }
            }
        }
        if self.transport.is_some() && !self.retirement_acked {
            return Ok(false);
        }
        if let Some(tree) = self.tree.as_mut() {
            tree.cancel();
        }
        if !self.retired {
            if let Some(candidate) = self.candidate.as_mut()
                && candidate.executable.has_agent_host()
            {
                page.pending_agent_host = Some(
                    candidate
                        .executable
                        .take_agent_host()
                        .map_err(map_host_error)?,
                );
            }
            if self.installed
                && page.executable.as_ref().is_some_and(|executable| {
                    executable.agent_root_realm() == self.destination_realm
                })
            {
                let executable = page.executable.as_mut().unwrap();
                if executable.has_agent_host() {
                    page.pending_agent_host =
                        Some(executable.take_agent_host().map_err(map_host_error)?);
                }
                page.executable.take();
                page.document.take();
                page.layout.take();
            }
            let expected = self
                .candidate
                .as_ref()
                .map(|candidate| candidate.executable.agent_host_identity());
            let mut host = page
                .extract_agent_host(expected.as_ref())?
                .ok_or(RuntimeError::EvaluationUnavailable)?;
            let result = host.retire_agent_root(self.destination_realm);
            page.pending_agent_host = Some(host);
            result.map_err(map_host_error)?;
            self.retired = true;
            self.candidate.take();
            self.reservation.take();
        }
        if self.source_suspended()
            && page.executable.is_none()
            && page
                .back_forward_cache
                .get(&self.source_index)
                .is_some_and(|entry| entry.document_generation == self.departure.source_generation)
        {
            page.history = self.original_history.clone();
            page.history_index = Some(self.source_index);
            page.restore_parked_incumbent(self.source_index)?;
        }
        // A new current root is never overwritten by this old recovery owner.
        if let Some(executable) = page.executable.as_mut()
            && !executable.has_agent_host()
            && let Some(host) = page.pending_agent_host.take()
            && let Err((error, host)) = executable.return_agent_host_after_turn(host)
        {
            page.pending_agent_host = Some(*host);
            return Err(map_host_error(error));
        }
        Ok(true)
    }
    pub fn defer_recovery(&mut self) {
        let now = Instant::now();
        // An actor can poll again before this retry is due. Preserve that
        // deadline; moving it on every early poll can starve recovery forever.
        if self.retry.is_none_or(|deadline| deadline <= now) {
            self.retry = Some(now + Duration::from_millis(25));
        }
    }
}

impl PageState {
    pub(super) fn retain_prerender_recovery(&mut self, transaction: &mut NavigationTransaction) {
        if !matches!(&transaction.phase, NavigationTransactionPhase::Prerender(_)) {
            return;
        }
        let NavigationTransactionPhase::Prerender(job) =
            std::mem::replace(&mut transaction.phase, NavigationTransactionPhase::Finished)
        else {
            unreachable!()
        };
        // New actor admission is blocked while this slot exists. Never replace
        // an unresolved publication owner with a new navigation owner.
        assert!(
            self.prerender_recovery.is_none(),
            "one retained prerender publication per page actor"
        );
        self.prerender_recovery = Some(job);
    }
    pub(super) fn poll_prerender_recovery(&mut self) {
        let Some(mut job) = self.prerender_recovery.take() else {
            return;
        };
        match job.recover(self) {
            Ok(true) => {}
            Ok(false) => {
                job.defer_recovery();
                self.prerender_recovery = Some(job)
            }
            Err(error) => {
                #[cfg(test)]
                if job.diagnostic_failure.is_none() {
                    job.diagnostic_failure =
                        Some(repair44_bounded_diagnostic(format_args!("{error}")));
                }
                #[cfg(not(test))]
                let _ = error;
                job.defer_recovery();
                self.prerender_recovery = Some(job)
            }
        }
    }
}

#[cfg(test)]
pub(super) fn owner_diagnostic_for_test(page: &PageState) -> String {
    let cached = page
        .back_forward_cache
        .iter()
        .take(8)
        .map(|(index, entry)| {
            (
                *index,
                entry.document_generation,
                entry.reusable,
                entry.executable.has_agent_host(),
            )
        })
        .collect::<Vec<_>>();
    // Recovery drivers may have lent their host to the actor. Completion
    // getters can inspect async queues through that host, so this snapshot
    // deliberately reads structural fields only.
    let native = page.executable.as_ref().map(|driver| {
        (
            driver.agent_root_realm(),
            driver.has_agent_host(),
            driver.parser_pending(),
        )
    });
    repair44_bounded_diagnostic(format_args!(
        "generation={} history_index={:?} history_count={} native={native:?} pending_host={} pending_response={} pending_transaction={} settling={} failed_install={} source_resume={} cache_count={} cached_first8={cached:?} recovery={:?}",
        page.document_generation,
        page.history_index,
        page.history.len(),
        page.pending_agent_host.is_some(),
        page.pending_navigation_response.is_some(),
        page.pending_navigation_transaction.is_some(),
        page.settling_navigation.is_some(),
        page.failed_install_recovery.is_some(),
        page.root_source_resume.is_some(),
        page.back_forward_cache.len(),
        page.prerender_recovery
            .as_deref()
            .map(Commit::diagnostic_for_test)
    ))
}

#[cfg(test)]
thread_local! {static PAUSE_AFTER_INSTALL:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};static PAUSE_AFTER_PROMOTION:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};static REPLACE_AFTER_HISTORY:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}

#[cfg(all(test, feature = "in-process-probe"))]
mod tests {
    use super::*;
    #[test]
    fn detached_async_driver_diagnostic_never_requires_the_lent_host() {
        let mut page = PageState::new(
            PageId(909),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            None,
            None,
        );
        let mut driver = crate::executable::RustExecutableDocument::load(
            "<body><script async type=module>globalThis.started=true;await new Promise(resolve=>globalThis.releaseModule=resolve);globalThis.finished=true</script>"
        ).unwrap();
        for _ in 0..16 {
            driver.drain_tasks().unwrap();
            if driver.evaluate("typeof releaseModule").unwrap() == "function" {
                break;
            }
        }
        assert_eq!(driver.evaluate("typeof releaseModule").unwrap(), "function");
        assert_eq!(
            driver
                .evaluate("String(globalThis.finished===true)")
                .unwrap(),
            "false"
        );
        let host = driver.take_agent_host().unwrap();
        assert!(!driver.has_agent_host());
        page.executable = Some(driver);
        let diagnostic = owner_diagnostic_for_test(&page);
        assert!(diagnostic.len() <= 1024);
        assert!(diagnostic.contains("native=Some"));
        assert!(
            !page.executable.as_ref().unwrap().has_agent_host(),
            "formatting must not consume or attach the separately owned host"
        );
        assert!(
            page.executable
                .as_mut()
                .unwrap()
                .install_agent_host(host)
                .is_ok()
        );
        page.executable
            .as_mut()
            .unwrap()
            .execute("releaseModule()")
            .unwrap();
    }
    struct Response(Arc<std::sync::atomic::AtomicUsize>);
    impl crate::network::NetworkBroker for Response {
        fn send(
            &self,
            request: crate::network::NetworkRequest,
        ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(crate::network::NetworkResponse{response_policy: None,timing:None,request_id:request.request_id,status:203,
                headers:vec![("content-type".into(),"text/html;charset=utf-8".into())],
                body:br#"<!doctype html><script>
                    globalThis.savedWindow=window;globalThis.savedDocument=document;globalThis.savedNavigation=performance.getEntriesByType('navigation')[0];globalThis.savedTimeOrigin=performance.timeOrigin;globalThis.beforeActivation=savedNavigation.activationStart;
                    globalThis.order=[];globalThis.initiallyPrerendering=document.prerendering;
                    addEventListener('pageshow',()=>order.push('pageshow'));
                    document.addEventListener('prerenderingchange',()=>order.push('prerenderingchange'));
                    addEventListener('pagereveal',()=>order.push('pagereveal'));
                    requestAnimationFrame(()=>order.push('raf'));
                </script><body>candidate<iframe srcdoc="&lt;script>globalThis.savedChild=document;globalThis.wasPrerendering=document.prerendering;globalThis.childNavigation=performance.getEntriesByType('navigation')[0];globalThis.childTimeOrigin=performance.timeOrigin;globalThis.childBeforeActivation=childNavigation.activationStart;document.addEventListener('prerenderingchange',()=>parent.childActivated=(savedChild===document&amp;&amp;!document.prerendering))&lt;/script>"></iframe></body>"#.to_vec()})
        }
    }
    fn pending_fixture() -> (
        BrowserContext,
        PageState,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let context = Browser::in_process_probe().new_context();
        let id = context.new_page().page_id();
        let client = SharedContextClient::direct(context.clone(), id);
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let broker = SharedNetworkBroker::new(Response(requests.clone()));
        let page = pending_page(id, client, broker);
        (context, page, requests)
    }
    fn pending_page(
        id: PageId,
        client: SharedContextClient,
        broker: SharedNetworkBroker,
    ) -> PageState {
        let mut page = PageState::new(
            id,
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            Some(broker),
            Some(client),
        );
        page.load_document_with_redirect_history("https://prerender.test/source".into(),r#"<!doctype html>
            <script>globalThis.originalDocument=document;globalThis.sourceTiming=performance.getEntriesByType('navigation')[0];globalThis.sourceTimingOrigin=performance.timeOrigin;globalThis.sourceEvents=[];globalThis.beforeUnloadCount=0;addEventListener('beforeunload',()=>beforeUnloadCount++);
            addEventListener('pageswap',()=>sourceEvents.push('swap'));
            addEventListener('pagehide',()=>sourceEvents.push('hide'));</script>
            <script type="speculationrules">{"prerender":[{"source":"list","urls":["/next"],"eagerness":"immediate"}]}</script>
            <body>source</body>"#,None,None,HistoryUpdate::Push,&BTreeMap::new(),Vec::new(),false,Some(Vec::new())).unwrap();
        page
    }
    fn fixture() -> (
        BrowserContext,
        PageState,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let (context, mut page, requests) = pending_fixture();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !page.prerender_candidates.loaded_for_test() {
            page.poll_page_tasks().unwrap();
            assert!(
                Instant::now() < deadline,
                "candidate never reached owned load publication"
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        (context, page, requests)
    }
    #[test]
    fn loaded_candidate_keeps_dynamic_child_parser_running_before_activation() {
        struct LateChild;
        impl crate::network::NetworkBroker for LateChild {
            fn send(
                &self,
                request: crate::network::NetworkRequest,
            ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                Ok(crate::network::NetworkResponse{response_policy: None,timing:None,request_id:request.request_id,status:200,
                    headers:vec![("content-type".into(),"text/html".into())],
                    body:br#"<!doctype html><script>
                      globalThis.beforeActivation=false;
                      addEventListener('load',()=>setTimeout(()=>{
                        const frame=document.createElement('iframe');
                        frame.srcdoc='<script>parent.beforeActivation=document.prerendering<\/script>';
                        document.body.append(frame);
                      },40));
                    </script><body>late child</body>"#.to_vec()})
            }
        }
        let context = Browser::in_process_probe().new_context();
        let id = context.new_page().page_id();
        let mut page = pending_page(
            id,
            SharedContextClient::direct(context.clone(), id),
            SharedNetworkBroker::new(LateChild),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !page.prerender_candidates.loaded_for_test() {
            page.poll_page_tasks().unwrap();
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert!(
            page.prerender_candidates.delay().is_some(),
            "load completion must retain candidate task ownership"
        );
        // Give the timer and its separately owned frame parser multiple actor
        // turns; the assertion after activation observes the child's original
        // prerendering value, so activation cannot conceal a stranded parser.
        let until = Instant::now() + Duration::from_millis(200);
        while Instant::now() < until {
            page.poll_page_tasks().unwrap();
            std::thread::park_timeout(Duration::from_millis(1));
        }
        page.begin_navigation_resource(
            "https://prerender.test/next".into(),
            HistoryUpdate::Push,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        while page.pending_navigation_transaction.is_some() || page.settling_navigation.is_some() {
            page.poll_page_tasks().unwrap();
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("String(beforeActivation===true&&!document.prerendering)")
                .unwrap(),
            "true"
        );
    }
    #[test]
    fn failed_candidate_construction_retires_local_root_before_incumbent_tasks() {
        let (context, mut page, requests) = pending_fixture();
        let source_realm = page.executable.as_ref().unwrap().agent_root_realm();
        let source_identity = context
            .inner
            .group
            .messages
            .lock()
            .unwrap()
            .root_document_identity(page.page_id.0);
        prerender_roster::FAIL_AFTER_CONSTRUCTION.with(|fail| fail.set(true));
        let deadline = Instant::now() + Duration::from_secs(5);
        while prerender_roster::FAIL_AFTER_CONSTRUCTION.with(|fail| fail.get()) {
            page.poll_page_tasks().unwrap();
            assert!(
                Instant::now() < deadline,
                "candidate construction injection was not reached"
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        let executable = page.executable.as_mut().unwrap();
        let host = executable.take_agent_host().unwrap();
        assert_eq!(host.realm_ids(), vec![source_realm]);
        assert!(matches!(
            executable.return_agent_host_after_turn(host),
            Ok(None)
        ));
        // The very next turn used to poll the canceled root before retirement.
        page.poll_page_tasks().unwrap();
        assert_eq!(
            context
                .inner
                .group
                .messages
                .lock()
                .unwrap()
                .root_document_identity(page.page_id.0),
            source_identity
        );
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(page.executable.as_mut().unwrap().evaluate("String(originalDocument===document&&!document.prerendering&&beforeUnloadCount===0)").unwrap(), "true");
    }
    #[test]
    fn actor_activates_the_loaded_document_without_refetch_or_second_pageshow() {
        let (_context, mut page, requests) = fixture();
        let source = page.document_generation;
        page.executable
            .as_mut()
            .unwrap()
            .execute("globalThis.sourceTimingJSON=JSON.stringify(sourceTiming.toJSON())")
            .unwrap_or_else(|error| {
                panic!(
                    "source timing: {}; {}; diagnostics={:?}",
                    repair44_bounded_diagnostic(format_args!("{error}")),
                    owner_diagnostic_for_test(&page),
                    page.trace
                        .events()
                        .iter()
                        .filter_map(|event| match &event.kind {
                            TraceEventKind::ScriptDiagnostic { capability, detail } =>
                                Some((capability, detail)),
                            _ => None,
                        })
                        .take(8)
                        .collect::<Vec<_>>()
                )
            });
        page.begin_navigation_resource(
            "https://prerender.test/next".into(),
            HistoryUpdate::Push,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while page.pending_navigation_transaction.is_some() || page.settling_navigation.is_some() {
            page.poll_page_tasks().unwrap();
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert_ne!(page.document_generation, source);
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
        let executable = page.executable.as_mut().unwrap();
        assert_eq!(executable.evaluate("String(initiallyPrerendering&&!document.prerendering&&savedWindow===window&&savedDocument===document)").unwrap(),"true");
        assert_eq!(
            executable.evaluate("order.slice(0,2).join(',')").unwrap(),
            "pageshow,prerenderingchange"
        );
        assert_eq!(
            executable
                .evaluate("String(order.filter(event=>event==='pageshow').length)")
                .unwrap(),
            "1"
        );
        assert_eq!(executable.evaluate("String(beforeActivation===0&&savedNavigation instanceof PerformanceNavigationTiming&&savedNavigation.activationStart===performance.getEntriesByType('navigation')[0].activationStart&&savedTimeOrigin===performance.timeOrigin&&savedNavigation.activationStart>0&&savedNavigation.toJSON().activationStart===savedNavigation.activationStart&&savedNavigation.responseStatus===203&&savedNavigation.loadEventEnd>0)").unwrap(),"true");
        assert_eq!(page.history.len(), 2);
        assert_eq!(page.executable.as_mut().unwrap().evaluate("String(childActivated===true&&document.querySelector('iframe').contentWindow.wasPrerendering===true)").unwrap(),"true");
        assert_eq!(page.executable.as_mut().unwrap().evaluate("String((()=>{const c=document.querySelector('iframe').contentWindow;return c.childBeforeActivation===0&&c.childNavigation.activationStart>0&&c.childTimeOrigin===c.performance.timeOrigin&&c.childNavigation.activationStart===c.performance.getEntriesByType('navigation')[0].activationStart})())").unwrap(),"true");
        while page
            .executable
            .as_mut()
            .unwrap()
            .evaluate("String(order.includes('raf'))")
            .unwrap()
            != "true"
        {
            page.poll_page_tasks().unwrap();
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("order.join(',')")
                .unwrap(),
            "pageshow,prerenderingchange,pagereveal,raf"
        );
        page.restore_cached_document(0, "https://prerender.test/source".into())
            .unwrap()
            .unwrap();
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate(
                    "String(originalDocument===document&&sourceEvents.join(',')==='swap,hide')"
                )
                .unwrap(),
            "true"
        );
        assert_eq!(page.executable.as_mut().unwrap().evaluate("String(sourceTimingOrigin===performance.timeOrigin&&sourceTimingJSON===JSON.stringify(sourceTiming.toJSON())&&performance.getEntriesByType('navigation')[0].activationStart===0)").unwrap(),"true");
    }
    #[test]
    fn cancellation_after_local_install_keeps_committed_document_and_activation_suffix() {
        for stop in [false, true] {
            let (context, mut page, requests) = fixture();
            let source = page.document_generation;
            PAUSE_AFTER_INSTALL.with(|pause| pause.set(true));
            page.begin_navigation_resource(
                "https://prerender.test/next".into(),
                HistoryUpdate::Push,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                page.poll_navigation_transaction();
                if matches!(page.pending_navigation_transaction.as_ref().map(|transaction|&transaction.phase),Some(NavigationTransactionPhase::Prerender(job)) if job.installed()&&job.phase==4)
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "{}",
                    owner_diagnostic_for_test(&page)
                );
                std::thread::yield_now();
            }
            let destination = page.document_generation;
            assert_ne!(destination, source);
            if stop {
                page.executable
                    .as_mut()
                    .unwrap()
                    .stop_pending_document_load()
                    .unwrap();
            }
            page.cancel_navigation_transaction(RuntimeError::NavigationCancelled);
            while page.prerender_recovery.is_some() {
                page.poll_prerender_recovery();
                assert!(
                    Instant::now() < deadline,
                    "{}",
                    owner_diagnostic_for_test(&page)
                );
                std::thread::yield_now();
            }
            assert_eq!(page.document_generation, destination);
            assert_eq!(page.history.len(), 2);
            assert_eq!(page.history_index, Some(1));
            assert_eq!(
                context.page_url(page.page_id).unwrap(),
                "https://prerender.test/next"
            );
            assert_eq!(page.executable.as_mut().unwrap().evaluate("String(savedDocument===document&&!document.prerendering&&order.filter(event=>event==='prerenderingchange').length===1)").unwrap(),"true");
            assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert!(page.pending_navigation_response.is_none());
        }
    }

    #[test]
    fn activated_response_retains_original_http_status_in_actor_publication() {
        let (_context, mut page, requests) = fixture();
        page.begin_navigation_resource(
            "https://prerender.test/next".into(),
            HistoryUpdate::Push,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(transaction) = page.pending_navigation_transaction.as_ref()
                && let NavigationTransactionPhase::Publication { result, .. } = &transaction.phase
            {
                assert_eq!(result.status, 203);
                break;
            }
            assert!(
                page.pending_navigation_transaction.is_some(),
                "activation skipped its retained publication"
            );
            page.poll_page_tasks().unwrap();
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
    #[test]
    fn canceled_accepted_activation_retires_destination_then_restores_exact_predecessor() {
        let (context, mut page, _) = fixture();
        let source = page.document_generation;
        page.executable.as_mut().unwrap().execute("globalThis.sourceTiming=performance.getEntriesByType('navigation')[0];globalThis.sourceTimingOrigin=performance.timeOrigin;globalThis.sourceTimingJSON=JSON.stringify(sourceTiming.toJSON())").unwrap();
        PAUSE_AFTER_PROMOTION.with(|pause| pause.set(true));
        page.begin_navigation_resource(
            "https://prerender.test/next".into(),
            HistoryUpdate::Replace,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_navigation_transaction();
            if matches!(page.pending_navigation_transaction.as_ref().map(|transaction|&transaction.phase),Some(NavigationTransactionPhase::Prerender(job)) if job.phase==2)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert!(page.executable.is_none());
        page.cancel_navigation_transaction(RuntimeError::NavigationCancelled);
        assert!(
            page.prerender_recovery.is_some(),
            "accepted publication cannot be dropped with its canceled caller"
        );
        let retry = Instant::now() + Duration::from_millis(25);
        page.prerender_recovery.as_mut().unwrap().retry = Some(retry);
        page.poll_prerender_recovery();
        assert_eq!(
            page.prerender_recovery.as_ref().unwrap().retry,
            Some(retry),
            "early actor polling must not postpone accepted activation recovery"
        );
        while page.prerender_recovery.is_some() {
            page.poll_prerender_recovery();
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert_eq!(page.document_generation, source);
        assert_eq!(page.history.len(), 1);
        assert_eq!(
            context.page_url(page.page_id).unwrap(),
            "https://prerender.test/source"
        );
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate(
                    "String(originalDocument===document&&sourceEvents.join(',')==='swap,hide')"
                )
                .unwrap(),
            "true"
        );
    }
    #[test]
    fn canceled_preinstall_publication_preserves_authored_successor_stream() {
        let (context, mut page, _) = fixture();
        let source = page.document_generation;
        REPLACE_AFTER_HISTORY.with(|fault| fault.set(true));
        page.begin_navigation_resource(
            "https://prerender.test/next".into(),
            HistoryUpdate::Push,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_navigation_transaction();
            if matches!(page.pending_navigation_transaction.as_ref().map(|transaction|&transaction.phase),Some(NavigationTransactionPhase::Prerender(job)) if job.phase==2&&job.candidate.as_ref().is_some_and(|candidate|candidate.executable.form_history_root_identity()!=job.destination_identity))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert!(page.executable.is_none());
        page.cancel_navigation_transaction(RuntimeError::NavigationCancelled);
        while page.prerender_recovery.is_some() {
            page.poll_prerender_recovery();
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert_ne!(page.document_generation, source);
        assert_eq!(
            context.page_url(page.page_id).unwrap(),
            "https://prerender.test/next"
        );
        assert_eq!(page.executable.as_mut().unwrap().evaluate("String(successorDocument===document&&!document.prerendering&&document.title==='authored successor')").unwrap(),"true");
        assert_eq!(page.history.len(), 2);
        assert_eq!(page.history_index, Some(1));
        assert!(
            page.pending_navigation_response.is_none(),
            "original response capture cannot publish into authored stream"
        );
    }
    #[test]
    fn replacement_request_waits_for_uncertain_activation_recovery_before_source_capture() {
        let (_context, mut page, _) = fixture();
        PAUSE_AFTER_PROMOTION.with(|pause| pause.set(true));
        page.begin_navigation_resource(
            "https://prerender.test/next".into(),
            HistoryUpdate::Replace,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_navigation_transaction();
            if matches!(page.pending_navigation_transaction.as_ref().map(|transaction|&transaction.phase),Some(NavigationTransactionPhase::Prerender(job)) if job.phase==2)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        page.begin_navigation_resource(
            "data:text/html,<p>replacement</p>".into(),
            HistoryUpdate::Push,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(page.prerender_recovery.is_some());
        assert!(
            page.pending_navigation_transaction
                .as_ref()
                .unwrap()
                .deferred_source
        );
        while page.prerender_recovery.is_some() || page.pending_navigation_transaction.is_some() {
            page.poll_page_tasks().unwrap();
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert!(page.url.starts_with("data:text/html,"));
        assert_eq!(page.history[0].url, "https://prerender.test/source");
        assert_eq!(
            page.history.len(),
            2,
            "canceled candidate never becomes a committed history entry"
        );
    }

    #[test]
    fn created_candidate_activates_with_original_body_pending_and_load_stays_pending() {
        use crate::network::{
            NetworkBroker, NetworkError, NetworkEvent, NetworkOperation, NetworkRequest,
            NetworkResponse,
        };
        type PendingBody = Option<(u64, tokio::sync::mpsc::Sender<NetworkEvent>)>;

        struct HeldBody {
            held: Arc<Mutex<PendingBody>>,
            requests: Arc<std::sync::atomic::AtomicUsize>,
        }
        impl NetworkBroker for HeldBody {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                unreachable!("native streaming owner")
            }
            fn start_operation(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkOperation, NetworkError> {
                self.requests
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let (sender, events) = tokio::sync::mpsc::channel(8);
                sender
                    .try_send(NetworkEvent::Headers {
                        response_policy: None,
                        timing: None,
                        request_id: request.request_id,
                        status: 203,
                        headers: vec![("content-type".into(), "text/html;charset=utf-8".into())],
                        content_length: None,
                        upload: None,
                    })
                    .unwrap();
                sender.try_send(NetworkEvent::Chunk{request_id:request.request_id,bytes:b"<!doctype html><script>globalThis.original=document;globalThis.shows=0;globalThis.savedNav=performance.getEntriesByType('navigation')[0];globalThis.savedOrigin=performance.timeOrigin;onpageshow=()=>shows++;</script><body>".to_vec().into()}).unwrap();
                *self.held.lock().unwrap() = Some((request.request_id, sender));
                Ok(NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id: request.request_id,
                    events,
                    demand: None,
                })
            }
        }
        let context = Browser::in_process_probe().new_context();
        let id = context.new_page().page_id();
        let held = Arc::new(Mutex::new(None));
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut page = pending_page(
            id,
            SharedContextClient::direct(context.clone(), id),
            SharedNetworkBroker::new(HeldBody {
                held: held.clone(),
                requests: requests.clone(),
            }),
        );
        page.begin_navigation_resource(
            "https://prerender.test/next".into(),
            HistoryUpdate::Push,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_page_tasks().unwrap();
            if matches!(page.pending_navigation_transaction.as_ref().map(|transaction|&transaction.phase),Some(NavigationTransactionPhase::Prerender(job)) if job.installed()&&job.phase==5)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "candidate activation incorrectly waited for EOF"
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert!(!page.executable.as_ref().unwrap().document_load_complete());
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("String(!document.prerendering)")
                .unwrap(),
            "true"
        );
        assert!(!page.lifecycle.contains(&LifecycleState::Load));
        assert_eq!(page.executable.as_mut().unwrap().evaluate("String(savedNav.responseEnd===0&&savedNav.duration===0&&savedNav.activationStart>0&&performance.timeOrigin===savedOrigin)").unwrap(),"true");
        let (id, sender) = held.lock().unwrap().take().unwrap();
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: id,
                bytes: b"<p id='tail'>retained body</p></body>".to_vec().into(),
            })
            .unwrap();
        sender
            .try_send(NetworkEvent::Complete {
                encoded_body_size: None,
                request_id: id,
            })
            .unwrap();
        while page.pending_navigation_transaction.is_some() {
            page.poll_page_tasks().unwrap();
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(page.executable.as_mut().unwrap().evaluate("String(original===document&&shows===1&&document.getElementById('tail').textContent==='retained body')").unwrap(),"true");
        assert_eq!(page.executable.as_mut().unwrap().evaluate("String(savedNav.responseEnd===performance.getEntriesByType('navigation')[0].responseEnd&&savedNav.responseEnd>0&&savedNav.loadEventEnd>=savedNav.responseEnd&&savedNav.duration===savedNav.loadEventEnd&&savedOrigin===performance.timeOrigin)").unwrap(),"true");
    }
    #[test]
    fn early_navigation_waits_for_original_candidate_without_second_fetch_or_beforeunload() {
        let (_context, mut page, requests) = pending_fixture();
        page.begin_navigation_resource(
            "https://prerender.test/next".into(),
            HistoryUpdate::Push,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut waited = false;
        while page.pending_navigation_transaction.is_some() {
            page.poll_page_tasks().unwrap();
            waited |= matches!(
                page.pending_navigation_transaction
                    .as_ref()
                    .map(|transaction| &transaction.phase),
                Some(NavigationTransactionPhase::PrerenderWaiting { .. })
            );
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert!(waited);
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(page.executable.as_mut().unwrap().evaluate("String(!document.prerendering&&order.filter(event=>event==='pageshow').length===1)").unwrap(),"true");
        page.restore_cached_document(0, "https://prerender.test/source".into())
            .unwrap()
            .unwrap();
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("String(beforeUnloadCount)")
                .unwrap(),
            "1"
        );
    }
    #[test]
    fn removing_pending_rule_falls_back_after_the_owned_beforeunload_prefix() {
        let (_context, mut page, requests) = pending_fixture();
        page.begin_navigation_resource(
            "https://prerender.test/next".into(),
            HistoryUpdate::Push,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_navigation_transaction();
            if matches!(
                page.pending_navigation_transaction
                    .as_ref()
                    .map(|transaction| &transaction.phase),
                Some(NavigationTransactionPhase::PrerenderWaiting { .. })
            ) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        page.executable
            .as_mut()
            .unwrap()
            .evaluate("document.querySelector('script[type=speculationrules]').remove()")
            .unwrap();
        while page.pending_navigation_transaction.is_some() {
            page.poll_page_tasks().unwrap();
            assert!(
                Instant::now() < deadline,
                "{}",
                owner_diagnostic_for_test(&page)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
        page.restore_cached_document(0, "https://prerender.test/source".into())
            .unwrap()
            .unwrap();
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("String(beforeUnloadCount)")
                .unwrap(),
            "1"
        );
    }
}

#[cfg(test)]
mod retirement_order_tests {
    use super::*;
    use oryn_js::message_wire::MessageRequest;
    // Root requests exercise exact generation/epoch deduplication without
    // fabricating public frame identities. Ordering is independent of variant.
    fn owner(generation: u64) -> MessageRequest {
        MessageRequest::RetireRootDocument {
            document_generation: generation,
            stream_epoch: 1,
        }
    }
    #[test]
    fn newly_created_descendant_precedes_its_previously_captured_parent() {
        let mut retained = VecDeque::from([owner(2), owner(1)]);
        merge_retirement_targets(
            &mut retained,
            VecDeque::from([owner(3), owner(2), owner(1)]),
        );
        assert!(
            retained
                .iter()
                .zip([3, 2, 1])
                .all(|(request, generation)| same_retirement(request, &owner(generation)))
        );
        assert_eq!(retained.len(), 3);
        // A retry of the same snapshot neither duplicates nor reorders owners.
        merge_retirement_targets(
            &mut retained,
            VecDeque::from([owner(3), owner(2), owner(1)]),
        );
        assert_eq!(retained.len(), 3);
        assert!(
            retained
                .iter()
                .zip([3, 2, 1])
                .all(|(request, generation)| same_retirement(request, &owner(generation)))
        );
    }
    #[test]
    fn detached_owner_stays_before_current_ancestors_and_new_stream_is_distinct() {
        let mut retained = VecDeque::from([owner(4), owner(2), owner(1)]);
        let newer = MessageRequest::RetireRootDocument {
            document_generation: 4,
            stream_epoch: 2,
        };
        merge_retirement_targets(
            &mut retained,
            VecDeque::from([newer.clone(), owner(2), owner(1)]),
        );
        assert_eq!(retained.len(), 4);
        assert!(same_retirement(&retained[0], &owner(4)));
        assert!(same_retirement(&retained[1], &newer));
        assert!(same_retirement(&retained[2], &owner(2)));
        assert!(same_retirement(&retained[3], &owner(1)));
    }
}

#[cfg(test)]
mod retained_candidate_layout_tests {
    #[test]
    fn activation_and_navigation_phases_do_not_inline_the_candidate_document() {
        assert!(
            std::mem::size_of::<super::Commit>()
                < std::mem::size_of::<super::prerender_load::CandidateDocument>()
        );
        assert!(
            std::mem::size_of::<super::NavigationTransactionPhase>()
                < std::mem::size_of::<super::prerender_load::CandidateDocument>()
        );
    }
}
