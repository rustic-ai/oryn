// Final original-source/Stylo graph association is one native transaction.
// Preparation and custody objects live outside both registry and DOM borrows.
struct PreparedRetainedCssPublication {
    authority: StylesheetPublicationAuthority,
    native: PreparedRetainedPublication,
    dom: oryn_dom::PreparedSheetSourceReplacement,
    proofs: StylesheetPublicationGuards,
    source_reservations: Vec<ScriptSourceReservation>,
    committed: bool,
    pending_dependencies: Option<CssDependencies>,
}
impl PreparedRetainedCssPublication {
    fn refresh_expectations(
        &mut self,
        arena: &SharedDomRuntime,
        registry: &RefCell<RetainedStylesheets>,
        admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
    ) -> Result<bool, HostError> {
        if self.committed {
            return Ok(true);
        }
        let Some(snapshot) = RetainedPublicationSnapshot::capture(registry, admission)? else {
            return Ok(false);
        };
        let stamp = arena
            .try_borrow()
            .map_err(|_| HostError::Execution("stylesheet retry journal borrow conflict".into()))?
            .stylesheet_journal_stamp();
        let bytes = stamp
            .requested_bytes()
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let charge = admission.reserve(bytes).map_err(HostError::Execution)?;
        let buffers = stamp
            .prepare_buffers(charge, bytes)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        if !self.native.rebase(snapshot, admission)? {
            return Ok(false);
        }
        let mut buffers = Some(buffers);
        if !self.dom.rebase_buffers(&mut buffers) {
            return Ok(false);
        }
        Ok(true)
    }
    /// Preparation owns every value across broker/admission callbacks. None is
    /// an ordinary retired owner or changed snapshot, not a successful commit.
    fn prepare(
        host: &RawV8Host,
        arena: &SharedDomRuntime,
        snapshot: RetainedPublicationSnapshot,
        authority: StylesheetPublicationAuthority,
        source_slot: &mut Option<RetainedStylesheetSource>,
        reservations_slot: &mut Option<Vec<ScriptSourceReservation>>,
        admission: Rc<dyn oryn_layout::cssom::native_import_graph::ImportAdmission>,
    ) -> Result<Option<Self>, HostError> {
        let source = source_slot
            .as_mut()
            .ok_or_else(|| HostError::Execution("stylesheet preparation source absent".into()))?;
        if reservations_slot.is_none() {
            return Err(HostError::Execution(
                "stylesheet preparation reservations absent".into(),
            ));
        }
        if !authority.current(host, arena)? {
            for response in &source.responses {
                if let Some(guard) = &response.guard {
                    guard.cancel();
                }
            }
            return Ok(None);
        }
        for response in &source.responses {
            if !response.proof.validate_native_owner(host, arena)? {
                for response in &source.responses {
                    if let Some(guard) = &response.guard {
                        guard.cancel();
                    }
                }
                return Ok(None);
            }
        }
        let identity = authority.document;
        let owner_identity = oryn_js::v8_bindings::BindingStylesheetDocumentOwner {
            realm: identity.realm,
            document: identity.document,
            document_generation: identity.document_generation,
            stream_epoch: identity.stream_epoch,
        };
        let Some(lifecycle) = host.stylesheet_owner_lifecycle(owner_identity, arena)? else {
            for response in &source.responses {
                if let Some(guard) = &response.guard {
                    guard.cancel();
                }
            }
            return Ok(None);
        };
        // Temporary source-queue leases cannot be the allocation owner of a
        // loaded sheet. Keep a separate retained lease shared by record and DOM.
        // This covers retained source ownership, not an allocator-exact parser
        // scratch upper bound; the parser's separate admission contract remains.
        let source_bytes = source
            .original
            .len()
            .checked_add(2 * std::mem::size_of::<usize>())
            .and_then(|bytes| bytes.checked_add(source.final_url.len()))
            .ok_or_else(|| {
                HostError::Execution("retained stylesheet source size overflow".into())
            })?;
        let source_bytes = source
            .responses
            .iter()
            .try_fold(source_bytes, |bytes, response| {
                bytes
                    .checked_add(response.source.len())
                    .and_then(|bytes| bytes.checked_add(response.final_url.capacity()))
                    .and_then(|bytes| bytes.checked_add(response.referrer_policy.capacity()))
                    .and_then(|bytes| bytes.checked_add(response.encoding.capacity()))
                    .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<usize>()))
                    .ok_or_else(|| {
                        HostError::Execution("retained stylesheet response size overflow".into())
                    })
            })?;
        let source_bytes = source_bytes
            .checked_add(
                source
                    .responses
                    .capacity()
                    .checked_mul(std::mem::size_of::<RetainedStylesheetResponse>())
                    .ok_or_else(|| {
                        HostError::Execution("retained response vector size overflow".into())
                    })?,
            )
            .ok_or_else(|| HostError::Execution("retained response size overflow".into()))?;
        let source_bytes = source
            .dependencies
            .iter()
            .try_fold(source_bytes, |bytes, (base, initiator, policy, css)| {
                bytes
                    .checked_add(base.capacity())
                    .and_then(|bytes| bytes.checked_add(initiator.capacity()))
                    .and_then(|bytes| bytes.checked_add(policy.capacity()))
                    .and_then(|bytes| bytes.checked_add(css.capacity()))
            })
            .and_then(|bytes| {
                bytes.checked_add(
                    source
                        .dependencies
                        .capacity()
                        .checked_mul(std::mem::size_of::<(String, String, String, String)>())?,
                )
            })
            .ok_or_else(|| {
                HostError::Execution("retained dependency allocation overflow".into())
            })?;
        let retained_source = admission
            .reserve(source_bytes)
            .map_err(HostError::Execution)?;
        let owned_source = oryn_dom::OwnedNativeStylesheetSource::admitted(
            source.original.clone(),
            Arc::new((retained_source.clone(), source.retained_allocation.clone())),
        );
        let base_bytes = source.final_url.len();
        let mut base_url = String::new();
        base_url
            .try_reserve_exact(base_bytes)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        base_url.push_str(&source.final_url);
        let stamp = arena
            .try_borrow()
            .map_err(|_| {
                HostError::Execution("stylesheet journal snapshot borrow conflict".into())
            })?
            .stylesheet_journal_stamp();
        let journal_bytes = stamp
            .requested_bytes()
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let journal_charge = admission
            .reserve(journal_bytes)
            .map_err(HostError::Execution)?;
        let buffers = stamp
            .prepare_buffers(journal_charge, journal_bytes)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let owner_is_style = {
            let dom = arena.try_borrow().map_err(|_| {
                HostError::Execution("stylesheet base owner borrow conflict".into())
            })?;
            matches!(&dom.arena().get(authority.owner)
                .map_err(|error| HostError::Execution(error.to_string()))?.kind,
                oryn_dom::NodeKind::Element { name } if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "style")
        };
        let base_url = (!owner_is_style).then_some(base_url);
        let storage = PreparedRetainedPublication::preflight(snapshot, admission.clone())?;
        let proofs = StylesheetPublicationGuards::take(&mut source.responses, admission.as_ref())?;
        // No Result, admission callback, DOM borrow or guard validation remains.
        // Physical Rc/Arc allocator exhaustion remains the existing fatal-OOM caveat.
        let mut source = source_slot
            .take()
            .expect("exclusive checked source custody");
        source.retained_allocation = Some(Arc::new((
            retained_source.clone(), source.retained_allocation.take(),
        )));
        let source_reservations = reservations_slot
            .take()
            .expect("exclusive checked reservation custody");
        let owner = RetainedStylesheetOwner {
            arena: Rc::downgrade(arena),
            document: identity,
            node: authority.owner,
            revision: authority.source_revision,
            lifecycle,
        };
        let native = PreparedRetainedPublication::finish_preflight(
            storage,
            owner,
            source,
            admission,
            retained_source,
        );

        let dom =
            oryn_dom::PreparedSheetSourceReplacement::new(owned_source, base_url, true, buffers);
        Ok(Some(Self {
            authority,
            native,
            dom,
            proofs,
            source_reservations,
            committed: false,
            pending_dependencies: None,
        }))
    }

    fn commit(
        &mut self,
        host: &RawV8Host,
        arena: &SharedDomRuntime,
        registry: &RefCell<RetainedStylesheets>,
    ) -> Result<bool, HostError> {
        if self.committed {
            return Ok(true);
        }
        if !self.authority.current(host, arena)? {
            self.proofs.cancel();
            return Ok(false);
        }
        if !self.proofs.validate()? {
            return Ok(false);
        }
        // Validation RPC may retire the original owner. Check again without
        // holding either mutable borrow before entering the linearization point.
        if !self.authority.current(host, arena)? {
            self.proofs.cancel();
            return Ok(false);
        }
        {
            let mut registry = registry.try_borrow_mut().map_err(|_| {
                HostError::Execution("stylesheet publication registry borrow conflict".into())
            })?;
            if !self.native.snapshot.matches(&registry) {
                return Ok(false);
            }
            let mut dom = arena.try_borrow_mut().map_err(|_| {
                HostError::Execution("stylesheet publication DOM borrow conflict".into())
            })?;
            let Some(source) = dom
                .prepare_stylesheet_source_replacement(
                    self.authority.owner,
                    self.authority.source_revision,
                    &mut self.dom,
                )
                .map_err(|error| {
                    match error {
                        oryn_dom::StylesheetMetadataPreparationError::Stale =>
                            HostError::Execution("stylesheet source preparation became stale".into()),
                        oryn_dom::StylesheetMetadataPreparationError::Dom(error) =>
                            HostError::Execution(format!("stylesheet source preparation: {error}")),
                    }
                })?
            else {
                return Ok(false);
            };
            // Exclusive DOM and registry borrows now protect every checked
            // predicate. Both commits move only pre-admitted owned values.
            if !self.native.commit(&mut registry) {
                return Ok(false);
            }
            source.commit();
        }
        self.committed = true;
        self.pending_dependencies = self.native.take_committed_dependencies();
        // Parent proofs/source queue reservations are temporary. They release
        // only after both mutable borrows end. Native graph/DOM leases remain.
        self.source_reservations.clear();
        self.proofs.entries.clear();
        Ok(true)
    }
}
