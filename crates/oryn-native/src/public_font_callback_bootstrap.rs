impl RawV8Host {
    fn bootstrap_callback_public_fonts(&mut self) -> Result<(), HostError> {
        // The executable checkpoint adopts contexts before reaching font work.
        // No V8 scope, binding-state borrow, or provider-slot borrow spans this
        // scalar walk or the allocation factory's owning-context refresh.
        let mut after = None;
        loop {
            let next = self.binding_states.iter()
                .filter(|(realm, _)| after.is_none_or(|last| **realm > last)
                    && self.realm_execution_live(**realm))
                .map(|(realm, _)| *realm).min();
            let Some(realm) = next else { return Ok(()); };
            after = Some(realm);
            let Some(layout) = self.native_layout_provider_in_realm(realm) else {
                continue;
            };
            if !layout.native_backend().is_some_and(|backend|
                backend.is::<LiveLayoutProvider>()) {
                continue;
            }
            let identity = Rc::as_ptr(&layout) as *const () as usize;
            let state = self.retained_binding_state(realm)?;
            if state.native_font_provider_matches(identity) { continue; }
            // Strong layout ownership above prevents pointer ABA during this
            // test. activate_public_font_loading rechecks exact provider and
            // document authority after all admission callbacks before publish.
            self.activate_public_font_loading(realm)?;
        }
    }
}
