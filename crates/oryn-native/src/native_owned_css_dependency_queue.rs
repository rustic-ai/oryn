/// Preserve the native dependency payload on every admission error. The
/// caller owns its source lease until this exact queue accepts the move.
impl RawV8Host {
    pub(crate) fn queue_owned_css_dependencies(
        &mut self,
        node: oryn_dom::NodeId,
        revision: u64,
        dependencies: &mut Option<crate::executable::CssDependencies>,
    ) -> Result<bool, HostError> {
        self.queue_owned_css_dependencies_with_receipt(node,revision,dependencies,&mut None)
    }
    pub(crate) fn queue_owned_css_dependencies_with_receipt(
        &mut self,
        node: oryn_dom::NodeId,
        revision: u64,
        dependencies: &mut Option<crate::executable::CssDependencies>,
        allocation: &mut Option<Arc<dyn Send + Sync>>,
    ) -> Result<bool, HostError> {
        let Some(payload) = dependencies.as_ref() else {
            return Ok(true);
        };
        if payload.is_empty() {
            dependencies.take();
            allocation.take();
            return Ok(true);
        }
        let epoch = self
            .document_stream_epoch(self.active_realm)
            .ok_or_else(|| HostError::Execution("CSS dependency owner retired".into()))?;
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("CSS dependency context absent".into()))?;
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &context);
        let limits = context
            .get_slot::<DynamicImportLimits>()
            .ok_or_else(|| HostError::Execution("CSS dependency budget absent".into()))?;
        if context.get_slot::<NativeCssDependencies>().is_none() {
            context.set_slot(Rc::new(NativeCssDependencies::default()));
        }
        let queue = context
            .get_slot::<NativeCssDependencies>()
            .expect("native CSS queue");
        let mut queue = queue
            .0
            .try_borrow_mut()
            .map_err(|_| HostError::Execution("CSS dependency queue borrow conflict".into()))?;
        if queue.len() >= 1024 {
            return Err(HostError::LimitExceeded {
                resource: "native CSS dependency batches".into(),
            });
        }
        let bytes = payload
            .iter()
            .try_fold(0usize, |bytes, (base, initiator, policy, css)| {
                bytes
                    .checked_add(base.len())
                    .and_then(|bytes| bytes.checked_add(initiator.len()))
                    .and_then(|bytes| bytes.checked_add(policy.len()))
                    .and_then(|bytes| bytes.checked_add(css.len()))
            })
            .ok_or_else(|| HostError::LimitExceeded {
                resource: "CSS dependency size overflow".into(),
            })?;
        let reservation = crate::executable::ScriptSourceReservation::reserve(&limits.bytes, bytes)
            .ok_or_else(|| HostError::LimitExceeded {
                resource: "queued CSS dependency bytes".into(),
            })?;
        queue
            .try_reserve(1)
            .map_err(|_| HostError::Execution("CSS dependency queue allocation".into()))?;
        // All fallible work precedes the exclusive move. Failed admission leaves
        // the caller's original Vec, URLs and request policy untouched.
        let dependencies = dependencies
            .take()
            .expect("exclusive prepared dependency payload");
        queue.push(crate::executable::CssDependencyBatch {
            node,
            revision,
            epoch,
            dependencies,
            _allocation: allocation.take(),
            _bytes: reservation,
        });
        Ok(true)
    }
}
