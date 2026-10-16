// The ready job owns its native phase, never borrows a completion payload.
// A failed event queue admission keeps Published and cannot replay IO.
impl ParserResourceScheduler {
    fn poll_retained_ready_jobs(
        &mut self,
        arena: &SharedDomRuntime,
        host: &mut RawV8Host,
    ) -> Result<(), HostError> {
        let turns = self
            .retained_ready
            .len()
            .min(MAX_SCRIPT_COMPLETIONS_PER_SLICE);
        for _ in 0..turns {
            let Some((key, mut custody)) = self.retained_ready.pop_first() else {
                break;
            };
            match publish_ready_retained_stylesheet(&mut custody.completion, host, arena) {
                Ok(true) => {
                    self.complete_render_stylesheet(key.0, key.1);
                    custody.complete_request();
                }
                Ok(false) => {
                    self.retained_ready.insert(key, custody);
                    break;
                }
                Err(error) => {
                    self.retained_ready.insert(key, custody);
                    return Err(error);
                }
            }
        }
        Ok(())
    }
    fn apply_retained_ready_job(
        &mut self,
        mut ready: Option<RetainedStylesheetReady>,
        arena: &SharedDomRuntime,
        host: &mut RawV8Host,
    ) -> Result<bool, HostError> {
        let descriptor = ready.as_ref().expect("owned retained ready input");
        let key = (descriptor.node, descriptor.revision);
        let realm = descriptor.identity.realm;
        let authority = StylesheetPublicationAuthority {
            document: descriptor.identity,
            arena: Rc::downgrade(arena),
            owner: descriptor.node,
            source_revision: descriptor.revision,
            cancellation: descriptor.cancellation.clone(),
        };
        match authority.current(host, arena) {
            Ok(false) => {
                descriptor
                    .cancellation
                    .flag
                    .store(true, std::sync::atomic::Ordering::Release);
                if let Ok(envelope) = &descriptor.result
        && let Some(tree) = &envelope.tree {
                    tree.cancel_proofs();
                }
                self.complete_render_stylesheet(key.0, key.1);
                let retired_request = self.local_stylesheets.remove(&key);
                drop(retired_request);
                return Ok(true);
            }
            Err(error) => {
                self.retry_completion(ParserResourceCompletion::RetainedStylesheet(
                    ready.take().expect("retained owner check custody"),
                ));
                return Err(error);
            }
            Ok(true) => {}
        }
        if !self.retained_ready.contains_key(&key) {
            let preparation = (|| {
                // Delivery may follow arbitrary author work. Obtain a current
                // owning-scope receipt before even allocating ready-map custody.
                host.native_retained_admission_for_realm(realm)?
                    .ok_or_else(|| {
                        HostError::Execution("retained ready authority missing".into())
                    })?;
                let _receipt_scope = host.retained_allocation_scope(realm)?;
                let provider = host.native_layout_provider_in_realm(realm).ok_or_else(|| {
                    HostError::Execution("retained ready provider missing".into())
                })?;
                let backend = provider
                    .native_backend()
                    .and_then(|value| value.downcast_ref::<LiveLayoutProvider>())
                    .ok_or_else(|| {
                        HostError::Execution("retained ready provider incompatible".into())
                    })?;
                let admission = backend
                    .retained_revision_admission()
                    .map_err(HostError::Execution)?;
                let bytes = std::mem::size_of::<((NodeId, u64), RetainedReadyCustody)>()
                    .checked_add(6 * std::mem::size_of::<usize>())
                    .ok_or_else(|| {
                        HostError::Execution("retained ready metadata overflow".into())
                    })?;
                let charge = admission.reserve(bytes).map_err(HostError::Execution)?;
                Ok::<_, HostError>(charge)
            })();
            let charge = match preparation {
                Ok(charge) => charge,
                Err(error) => {
                    self.retry_completion(ParserResourceCompletion::RetainedStylesheet(
                        ready.take().expect("preflight retained source custody"),
                    ));
                    return Err(error);
                }
            };
            if !self.local_stylesheets.contains_key(&key) {
                self.retry_completion(ParserResourceCompletion::RetainedStylesheet(
                    ready.take().expect("missing request preserves reply custody"),
                ));
                return Err(HostError::Execution("retained response request custody missing".into()));
            }
            let completion = ready
                .take()
                .expect("admitted retained source")
                .into_completion(arena);
            let request = self.local_stylesheets.remove(&key).expect("checked exact request custody");
            self.retained_ready.insert(
                key,
                RetainedReadyCustody {
                    completion,
                    _request: request,
                    _allocation: charge,
                },
            );
        }
        // Move phase out of the map before native broker/admission callbacks.
        // Reentrant scheduler work cannot alias a mutable completion borrow.
        let mut completion = self
            .retained_ready
            .remove(&key)
            .ok_or_else(|| HostError::Execution("retained completion missing".into()))?;
        let result = publish_ready_retained_stylesheet(&mut completion.completion, host, arena);
        match result {
            Ok(true) => {
                self.complete_render_stylesheet(key.0, key.1);
                completion.complete_request();
                Ok(true)
            }
            Ok(false) => {
                self.retained_ready.insert(key, completion);
                Ok(false)
            }
            Err(error) => {
                self.retained_ready.insert(key, completion);
                Err(error)
            }
        }
    }
}
