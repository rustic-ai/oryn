// Main-thread owner recipe and cross-thread immutable completion transport.
// This has no V8/Rc/Stylo values in the resource executor's reply.
struct RetainedStylesheetReady {
    identity: ParserDocumentIdentity,
    node: NodeId,
    revision: u64,
    cancellation: Arc<crate::context_service::ResourceCancellation>,
    result: Result<RetainedStylesheetSourceEnvelope, String>,
}
struct RetainedReadyCustody {
    completion: RetainedParserCompletion,
    _allocation: Arc<dyn Send + Sync>,
}
impl RetainedStylesheetReady {
    fn into_completion(self, arena: &SharedDomRuntime) -> RetainedParserCompletion {
        let phase = match self.result {
            Ok(envelope) => RetainedParserCompletionPhase::Source(envelope),
            Err(diagnostic) => RetainedParserCompletionPhase::Failed {
                _reservations: Vec::new(),
                diagnostic: Some(diagnostic),
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
    if let RetainedParserCompletionPhase::Failed { diagnostic, .. } = &mut completion.phase {
        if let Some(diagnostic) = diagnostic.take() {
            return Err(HostError::Execution(diagnostic));
        }
        // Preserve failure custody until native event admission succeeds. No
        // refetch, reparsing or synthetic author dispatch occurs on retry.
        host.queue_stylesheet_event(
            completion.authority.owner,
            completion.authority.source_revision,
            false,
        )?;
        completion.phase = RetainedParserCompletionPhase::Published { success: false };
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
    host.queue_stylesheet_event(
        completion.authority.owner,
        completion.authority.source_revision,
        true,
    )?;
    Ok(true)
}
