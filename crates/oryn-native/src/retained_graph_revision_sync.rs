impl LiveLayoutProvider {
    fn synchronize_retained_graph_revision(&self) -> Result<(), String> {
        let admission = self.retained_revision_admission()?;
        sweep_retired_stylesheet_records(&self.retained_stylesheets, admission.as_ref())
            .map_err(|error| error.to_string())?;
        let count = self
            .retained_stylesheets
            .try_borrow()
            .map_err(|_| "retained revision count borrow conflict")?
            .records
            .len();
        let bytes = count
            .checked_mul(std::mem::size_of::<RetainedGraphRevision>())
            .ok_or("retained revision size overflow")?;
        let charge = admission.reserve(bytes)?;
        let mut key = Vec::new();
        key.try_reserve_exact(count)
            .map_err(|_| "retained revision allocation")?;
        {
            let records = self
                .retained_stylesheets
                .try_borrow()
                .map_err(|_| "retained revision snapshot borrow conflict")?;
            if records.records.len() != count {
                return Err("retained revision changed; recapture".into());
            }
            for record in &records.records {
                key.push(RetainedGraphRevision {
                    publication: Rc::as_ptr(record) as usize,
                    mutation: record.mutation_revision.get(),
                });
            }
        }
        // Prevalidate every borrow and overflow before changing keys. Old
        // charge/key custody remains local until both RefCell borrows end.
        let mut old = self
            .retained_revision_key
            .try_borrow_mut()
            .map_err(|_| "retained revision key borrow conflict")?;
        if old.as_ref() == Some(&key) {
            return Ok(());
        }
        let next = self
            .stylesheet_graph_revision
            .get()
            .checked_add(1)
            .ok_or("retained revision exhausted")?;
        let mut old_charge = self
            .retained_revision_key_charge
            .try_borrow_mut()
            .map_err(|_| "retained revision charge borrow conflict")?;
        let retired_key = old.replace(key);
        let retired_charge = old_charge.replace(charge);
        self.stylesheet_graph_revision.set(next);
        drop(old_charge);
        drop(old);
        drop(retired_charge);
        drop(retired_key);
        Ok(())
    }
}
