// Main-thread owner recipe and cross-thread immutable completion transport.
// This has no V8/Rc/Stylo values in the resource executor's reply.
enum RetainedStylesheetFailure { Web, Operational(String) }
impl From<HostError> for RetainedStylesheetFailure {
    fn from(error: HostError) -> Self {
        match error {
            HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Fetch(_)
                | crate::script_load_failure::ScriptLoadFailure::Integrity(_)) => Self::Web,
            error => Self::Operational(error.to_string()),
        }
    }
}
impl From<String> for RetainedStylesheetFailure {
    fn from(error: String) -> Self { Self::Operational(error) }
}
fn retained_operational_diagnostic(phase: &RetainedParserCompletionPhase) -> Option<&str> {
    match phase {
        RetainedParserCompletionPhase::OperationalFailure { diagnostic, .. } => Some(diagnostic),
        _ => None,
    }
}
struct RetainedStylesheetReady {
    identity: ParserDocumentIdentity,
    node: NodeId,
    revision: u64,
    cancellation: Arc<crate::context_service::ResourceCancellation>,
    result: Result<RetainedStylesheetSourceEnvelope, RetainedStylesheetFailure>,
}
struct RetainedReadyCustody {
    completion: RetainedParserCompletion,
    _request: LocalStylesheetCustody,
    _allocation: Arc<dyn Send + Sync>,
}
impl RetainedReadyCustody {
    fn complete_request(self) {
        if matches!(self.completion.phase, RetainedParserCompletionPhase::Published { .. }) {
            self._request.0.complete();
        }
        // Otherwise ordinary Drop cancels exact retired request.
    }
}
impl RetainedStylesheetReady {
    fn into_completion(self, arena: &SharedDomRuntime) -> RetainedParserCompletion {
        let phase = match self.result {
            Ok(envelope) => RetainedParserCompletionPhase::Source(envelope),
            Err(RetainedStylesheetFailure::Operational(diagnostic)) =>
                RetainedParserCompletionPhase::OperationalFailure {
                    _reservations: Vec::new(), diagnostic,
                },
            Err(RetainedStylesheetFailure::Web) => RetainedParserCompletionPhase::Failed {
                _reservations: Vec::new(),
                diagnostic: None,
            },
        };
        RetainedParserCompletion {
            authority: StylesheetPublicationAuthority {
                document: self.identity,
                arena: Rc::downgrade(arena),
                owner: self.node,
                source_revision: self.revision,
                cancellation: self.cancellation,
            },
            phase,
        }
    }
}

/// Every boundary stores its owned phase through event/publication retries.
/// Returning false does not acknowledge the scheduler's completion ticket.
fn apply_retained_stylesheet_ready(
    completion: &mut RetainedParserCompletion,
    host: &mut RawV8Host,
    arena: &SharedDomRuntime,
    registry: &RefCell<RetainedStylesheets>,
    lock: oryn_layout::cssom::NativeStyleLock,
    quirks: bool,
    admission: Rc<dyn oryn_layout::cssom::native_import_graph::ImportAdmission>,
) -> Result<bool, HostError> {
    if let RetainedParserCompletionPhase::Published { success } = completion.phase {
        #[cfg(test)]
        frame_script_preparation::xml_publication_failure(frame_script_preparation::XmlPublicationFailure::BeforeStyleEvent)?;
        host.queue_stylesheet_event(
            completion.authority.owner,
            completion.authority.source_revision,
            success,
        )?;
        return Ok(true);
    }
    if !completion.authority.current(host, arena)? {
        completion.cancel_pending();
        return Ok(true); // Native retirement is terminal, not an author error event.
    }
    if let Some(diagnostic) = retained_operational_diagnostic(&completion.phase) {
        return Err(HostError::Execution(diagnostic.to_owned()));
    }
    if let RetainedParserCompletionPhase::Failed { diagnostic, .. } = &mut completion.phase {
        if let Some(diagnostic) = diagnostic.take() {
            return Err(HostError::Execution(diagnostic));
        }
        // Preserve failure custody until native event admission succeeds. No
        // refetch, reparsing or synthetic author dispatch occurs on retry.
        #[cfg(test)]
        frame_script_preparation::xml_publication_failure(frame_script_preparation::XmlPublicationFailure::BeforeStyleEvent)?;
        host.queue_stylesheet_event(
            completion.authority.owner,
            completion.authority.source_revision,
            false,
        )?;
        completion.phase = RetainedParserCompletionPhase::Published { success: false };
        return Ok(true);
    }
    if let RetainedParserCompletionPhase::InlineRoot { delivery, .. } = &mut completion.phase {
        if !delivery.poll(host, arena)? { return Ok(false); }
        host.queue_owned_css_dependencies_with_receipt(completion.authority.owner,
            completion.authority.source_revision, &mut delivery.pending_dependencies,
            &mut delivery.dependency_allocation)?;
        // Same native root/child Arcs are already installed. Event admission
        // retry never recreates that graph or releases fetched source early.
        #[cfg(test)]
        frame_script_preparation::xml_publication_failure(frame_script_preparation::XmlPublicationFailure::BeforeStyleEvent)?;
        host.queue_stylesheet_event(completion.authority.owner,
            completion.authority.source_revision, true)?;
        completion.phase = RetainedParserCompletionPhase::Published { success: true };
        return Ok(true);
    }
    if !completion.realize_source(host, arena, lock, quirks, admission.as_ref())? {
        return Ok(false);
    }
    if !completion.publish_prepared(host, arena, registry, admission)? {
        return Ok(false);
    }
    let RetainedParserCompletionPhase::Prepared { prepared, .. } = &mut completion.phase else {
        return Ok(false);
    };
    host.queue_owned_css_dependencies(
        completion.authority.owner,
        completion.authority.source_revision,
        &mut prepared.pending_dependencies,
    )?;
    completion.phase = RetainedParserCompletionPhase::Published { success: true };
    // If event queue admission fails, Published remains in the original ready
    // completion; retry cannot refetch/reparse or swap the native sheet twice.
    #[cfg(test)]
    frame_script_preparation::xml_publication_failure(frame_script_preparation::XmlPublicationFailure::BeforeStyleEvent)?;
    host.queue_stylesheet_event(
        completion.authority.owner,
        completion.authority.source_revision,
        true,
    )?;
    Ok(true)
}
