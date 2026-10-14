// Method added inside LiveLayoutProvider. The provider owns exactly one native
// document arena and shared style lock for all retained graph publications.
enum RetainedCascadeSync {
    Ready,
    /// No publication occurred. The owning checkpoint must recapture its input;
    /// a cursor race is not evidence that a retained sheet may be reclaimed.
    Retry,
}

impl LiveLayoutProvider {
    fn synchronize_retained_cascade(
        &self,
        input: &oryn_layout::LayoutInput,
        identity: ParserDocumentIdentity,
        admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
    ) -> Result<RetainedCascadeSync, String> {
        // Clone only admitted Rc handles outside registry borrowing. Each native
        // reconciliation performs its own exact owner/cursor checks and releases
        // DOM borrowing before parser/admission callbacks.
        let count = self
            .retained_stylesheets
            .try_borrow()
            .map_err(|_| "retained stylesheet registry borrow conflict")?
            .records
            .len();
        let _owner_charge = admission.reserve(
            count
                .checked_mul(std::mem::size_of::<Rc<RetainedStylesheetRecord>>())
                .ok_or("retained stylesheet owner metadata overflow")?,
        )?;
        let mut owners = Vec::new();
        owners
            .try_reserve_exact(count)
            .map_err(|_| "retained stylesheet owner metadata allocation")?;
        {
            let registry = self
                .retained_stylesheets
                .try_borrow()
                .map_err(|_| "retained stylesheet registry borrow conflict")?;
            if registry.records.len() != count {
                return Ok(RetainedCascadeSync::Retry);
            }
            owners.extend(registry.records.iter().cloned());
        }
        for owner in owners {
            if owner
                .reconcile_owner_state_admitted()
                .map_err(|error| error.to_string())?
                .is_none()
            {
                return Ok(RetainedCascadeSync::Retry);
            }
        }
        let viewport = self.viewport.get();
        let mut mutation_revisions = Vec::new();
        let record_count = self
            .retained_stylesheets
            .try_borrow()
            .map_err(|_| "retained stylesheet registry borrow conflict")?
            .records
            .len();
        let _revision_charge = admission.reserve(
            record_count
                .checked_mul(std::mem::size_of::<(NodeId, u64, u64)>())
                .ok_or("retained stylesheet revision overflow")?,
        )?;
        mutation_revisions
            .try_reserve_exact(record_count)
            .map_err(|_| "retained stylesheet revision allocation")?;
        {
            let registry = self
                .retained_stylesheets
                .try_borrow()
                .map_err(|_| "retained stylesheet registry borrow conflict")?;
            if registry.records.len() != record_count {
                return Ok(RetainedCascadeSync::Retry);
            }
            for record in &registry.records {
                mutation_revisions.push((
                    record.owner.node,
                    record.owner.revision,
                    record.mutation_revision.get(),
                ));
            }
        }
        let key = (
            self.dom
                .try_borrow()
                .map_err(|_| "retained style DOM borrow conflict")?
                .mutation_cursor()
                .0,
            self.stylesheet_graph_revision.get(),
            viewport.width.to_bits(),
            viewport.height.to_bits(),
            viewport.device_pixel_ratio.to_bits(),
            self.native_media.get(),
            mutation_revisions,
        );
        if self.retained_cascade_key.borrow().as_ref() == Some(&key) {
            return Ok(RetainedCascadeSync::Ready);
        }
        let cascade_bytes = input
            .stylesheet_owner_ids
            .len()
            .checked_mul(
                std::mem::size_of::<(usize, oryn_layout::cssom::native_import_graph::NativeSheet)>(
                ) + 6 * std::mem::size_of::<usize>(),
            )
            .and_then(|bytes| {
                bytes.checked_add(
                    std::mem::size_of::<
                        BTreeMap<usize, oryn_layout::cssom::native_import_graph::NativeSheet>,
                    >() + 2 * std::mem::size_of::<usize>(),
                )
            })
            .ok_or("retained stylesheet cascade metadata overflow")?;
        admission.checkpoint()?;
        let cascade_charge = admission.reserve(cascade_bytes)?;
        let cursor = self
            .dom
            .try_borrow()
            .map_err(|_| "retained style DOM borrow conflict")?
            .mutation_cursor()
            .0;
        if cursor != key.0
            || self.stylesheet_graph_revision.get() != key.1
            || self.viewport.get().width.to_bits() != key.2
            || self.viewport.get().height.to_bits() != key.3
            || self.viewport.get().device_pixel_ratio.to_bits() != key.4
            || self.native_media.get() != key.5
        {
            return Ok(RetainedCascadeSync::Retry);
        }
        // Admission can invoke an owning checkpoint. Revalidate native CSSOM
        // revisions as well as the DOM cursor before publishing the cascade.
        {
            let registry = self
                .retained_stylesheets
                .try_borrow()
                .map_err(|_| "retained stylesheet registry borrow conflict")?;
            if registry.records.len() != key.6.len()
                || registry
                    .records
                    .iter()
                    .zip(&key.6)
                    .any(|(record, expected)| {
                        (
                            record.owner.node,
                            record.owner.revision,
                            record.mutation_revision.get(),
                        ) != *expected
                    })
            {
                return Ok(RetainedCascadeSync::Retry);
            }
        }
        let cascade = self
            .retained_stylesheets
            .try_borrow()
            .map_err(|_| "retained stylesheet registry borrow conflict")?
            .cascade(
                &self.dom,
                identity,
                &input.stylesheet_owner_ids,
                self.style_lock.clone(),
                self.stylesheet_graph_revision.get(),
                cascade_charge,
            )
            .map_err(|error| error.to_string())?;
        // Acquire every mutable destination before publishing any component.
        let mut engine = self
            .style_engine
            .try_borrow_mut()
            .map_err(|_| "retained style engine borrow conflict")?;
        let mut key_slot = self
            .retained_cascade_key
            .try_borrow_mut()
            .map_err(|_| "retained cascade key borrow conflict")?;
        let mut charge_slot = self
            .retained_cascade_key_charge
            .try_borrow_mut()
            .map_err(|_| "retained cascade charge borrow conflict")?;
        let old_sheets = engine.swap_retained_stylesheets(Some(cascade));
        let old_key = key_slot.replace(key);
        let old_charge = charge_slot.replace(_revision_charge);
        drop(charge_slot);
        drop(key_slot);
        drop(engine);
        // Allocation owners are destroyed only after all destination borrows end.
        drop(old_sheets);
        drop(old_key);
        drop(old_charge);
        Ok(RetainedCascadeSync::Ready)
    }
}
