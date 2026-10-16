// Original-response acquisition for parser/style/link owners. This replaces
// flattening only when the owning completion has a retained graph destination.
#[cfg(unix)]
fn load_retained_parser_stylesheet(
    params: ParserStylesheetInput<'_>,
) -> Result<RetainedStylesheetSourceEnvelope, HostError> {
    let ParserStylesheetInput {
        client,
        page_id,
        initiator,
        requested_url,
        initiator_url,
        referrer_policy,
        environment_encoding,
        depth,
        budget,
    } = params;
    if depth != 0 {
        return Err(HostError::Execution(
            "retained stylesheet root depth mismatch".into(),
        ));
    }
    let mut walk = ImportWalk {
        client,
        page_id,
        initiator,
        budget,
    };
    let tree = walk.collect_root_source_tree(
        requested_url,
        initiator_url.as_deref().unwrap_or(requested_url),
        referrer_policy.as_deref().unwrap_or(""),
        environment_encoding,
    )?;
    walk.budget.finish_retained_tree(tree)
}

#[cfg(unix)]
fn load_retained_inline_stylesheet(
    params: InlineCssResourceInput<'_>,
    authority: NativeInlineStylesheetAuthority,
) -> Result<RetainedStylesheetSourceEnvelope, HostError> {
    let InlineCssResourceInput {
        client,
        page_id,
        initiator,
        document_url: _,
        base_url,
        referrer_policy,
        css,
        environment_encoding,
        budget,
    } = params;
    budget.check()?;
    let bytes = css
        .len()
        .checked_add(base_url.len())
        .and_then(|bytes| bytes.checked_add(referrer_policy.len()))
        .ok_or_else(|| HostError::Execution("inline stylesheet source size overflow".into()))?;
    // Original source remains in the same source budget while the immutable
    // tree is queued. It is never fetched from its document's base URL.
    budget.retain(bytes)?;
    let response = RetainedStylesheetResponse {
        sheet: None,
        final_url: base_url.to_owned(),
        source: Arc::from(css),
        referrer_policy: referrer_policy.to_owned(),
        encoding: environment_encoding.to_owned(),
        proof: NativeStylesheetResponseAuthority::Inline(authority),
        guard: None,
    };
    let mut walk = ImportWalk {
        client,
        page_id,
        initiator,
        budget,
    };
    let tree = walk.collect_source_tree(response, 0, &mut Vec::new())?;
    walk.budget.finish_retained_tree(tree)
}

/// Ready source is moved exactly once into preparation. Its existing source
/// reservations survive parsing, parent proof validation and publication retry.
struct RetainedParserCompletion {
    authority: StylesheetPublicationAuthority,
    phase: RetainedParserCompletionPhase,
}
// Phase payload is already owned and charged by the source/completion queue.
// Boxing would add an unadmitted allocation to the delivery transition.
#[expect(clippy::large_enum_variant, reason = "fixed inline phase ownership avoids an unadmitted allocation during completion transfer")]
enum RetainedParserCompletionPhase {
    Source(RetainedStylesheetSourceEnvelope),
    InlineRoot {
        delivery: Box<InlinePendingRootDelivery>,
        _allocation: Arc<dyn Send + Sync>,
    },
    OperationalFailure {
        _reservations: Vec<ScriptSourceReservation>,
        diagnostic: String,
    },
    Failed {
        _reservations: Vec<ScriptSourceReservation>,
        diagnostic: Option<String>,
    },
    Parsed {
        source: RetainedStylesheetSource,
        reservations: Vec<ScriptSourceReservation>,
    },
    Prepared {
        prepared: Box<PreparedRetainedCssPublication>,
        _allocation: std::sync::Arc<dyn Send + Sync>,
    },
    Retired,
    Published {
        success: bool,
    },
}
impl RetainedParserCompletion {
    fn cancel_pending(&mut self) {
        match &mut self.phase {
            RetainedParserCompletionPhase::Source(envelope) => {
                if let Some(tree) = &envelope.tree {
                    tree.cancel_proofs();
                }
            }
            RetainedParserCompletionPhase::Parsed { source, .. } => {
                for response in &source.responses {
                    if let Some(guard) = &response.guard {
                        guard.cancel();
                    }
                }
            }
            RetainedParserCompletionPhase::Prepared { prepared, .. } => prepared.proofs.cancel(),
            RetainedParserCompletionPhase::InlineRoot { delivery, .. } => delivery.cancel(),
            _ => {}
        }
        self.phase = RetainedParserCompletionPhase::Retired;
    }

    fn publish_prepared(
        &mut self,
        host: &RawV8Host,
        arena: &SharedDomRuntime,
        registry: &RefCell<RetainedStylesheets>,
        admission: Rc<dyn oryn_layout::cssom::native_import_graph::ImportAdmission>,
    ) -> Result<bool, HostError> {
        if !self.authority.current(host, arena)? {
            self.cancel_pending();
            return Ok(false);
        }
        if let RetainedParserCompletionPhase::Prepared { prepared, .. } = &mut self.phase {
            if !prepared.refresh_expectations(arena, registry, admission.as_ref())? {
                return Ok(false);
            }
            let committed = prepared.commit(host, arena, registry)?;
            return Ok(committed);
        }
        // Snapshot capture can report a race. Do not move source/guards before
        // it succeeds, so the scheduler's retry owns the exact original source.
        let Some(snapshot) = RetainedPublicationSnapshot::capture(registry, admission.as_ref())?
        else {
            return Ok(false);
        };
        if !matches!(self.phase, RetainedParserCompletionPhase::Parsed { .. }) {
            return Ok(false);
        }
        // Metadata admission is fallible: retain Parsed source and cleanup
        // custody until it succeeds rather than retiring a retryable delivery.
        let prepared_charge = admission
            .reserve(std::mem::size_of::<PreparedRetainedCssPublication>())
            .map_err(HostError::Execution)?;
        let previous = std::mem::replace(&mut self.phase, RetainedParserCompletionPhase::Retired);
        let RetainedParserCompletionPhase::Parsed {
            source,
            reservations,
        } = previous
        else {
            unreachable!()
        };
        let mut source_slot = Some(source);
        let mut reservations_slot = Some(reservations);
        let outcome = PreparedRetainedCssPublication::prepare(
            host,
            arena,
            snapshot,
            self.authority.clone(),
            &mut source_slot,
            &mut reservations_slot,
            admission.clone(),
        );
        let prepared = match outcome {
            Ok(Some(prepared)) => prepared,
            Ok(None) => {
                self.phase = RetainedParserCompletionPhase::Parsed {
                    source: source_slot.expect("precommit race preserves source custody"),
                    reservations: reservations_slot
                        .expect("precommit race preserves reservation custody"),
                };
                return Ok(false);
            }
            Err(error) => {
                self.phase = RetainedParserCompletionPhase::Parsed {
                    source: source_slot.expect("precommit error preserves source custody"),
                    reservations: reservations_slot
                        .expect("precommit error preserves reservation custody"),
                };
                return Err(error);
            }
        };
        self.phase = RetainedParserCompletionPhase::Prepared { prepared: Box::new(prepared), _allocation: prepared_charge };
        let RetainedParserCompletionPhase::Prepared { prepared, .. } = &mut self.phase else {
            unreachable!()
        };
        let committed = prepared.commit(host, arena, registry)?;
        Ok(committed)
    }

    fn realize_source(
        &mut self,
        host: &RawV8Host,
        arena: &SharedDomRuntime,
        lock: oryn_layout::cssom::NativeStyleLock,
        quirks: bool,
        admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
    ) -> Result<bool, HostError> {
        if !self.authority.current(host, arena)? {
            return Ok(false);
        }
        let RetainedParserCompletionPhase::Source(envelope) = &mut self.phase else {
            return Ok(true);
        };
        // The envelope keeps body reservations even if native parsing fails.
        // A consumed source is a terminal parse failure, not a refetch/retry.
        let tree = envelope.take_tree()?;
        let mut source = match tree.realize(lock, quirks, admission) {
            Ok(source) => source,
            Err(error) => {
                let reservations = std::mem::take(&mut envelope.reservations);
                self.phase = RetainedParserCompletionPhase::OperationalFailure {
                    _reservations: reservations,
                    // Retry retains the primary diagnostic and source quota;
                    // operational parsing failure is never an author event.
                    diagnostic: error.to_string(),
                };
                return Err(error);
            }
        };
        let reservations = std::mem::take(&mut envelope.reservations);
        source.retained_allocation = envelope._allocation.take();
        self.phase = RetainedParserCompletionPhase::Parsed {
            source,
            reservations,
        };
        if !self.authority.current(host, arena)? {
            return Ok(false);
        }
        if let RetainedParserCompletionPhase::Parsed { source, .. } = &self.phase {
            for response in &source.responses {
                if !response.proof.validate_native_owner(host, arena)? {
                    return Ok(false);
                }
            }
            if !source.validate_responses()? {
                for response in &source.responses {
                    if let Some(guard) = &response.guard {
                        guard.cancel();
                    }
                }
                let previous =
                    std::mem::replace(&mut self.phase, RetainedParserCompletionPhase::Retired);
                let RetainedParserCompletionPhase::Parsed { reservations, .. } = previous else {
                    unreachable!();
                };
                self.phase = RetainedParserCompletionPhase::Failed {
                    _reservations: reservations,
                    diagnostic: None,
                };
                return Ok(false);
            }
        }
        Ok(true)
    }
}
