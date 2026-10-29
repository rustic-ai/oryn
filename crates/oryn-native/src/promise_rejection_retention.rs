// Diagnostic retention now uses weak Promise identity and an exact-capacity
// admitted buffer. Automatic notifications will move these records into the
// rejection lifecycle; the diagnostic drain remains separate until that wiring.
#[derive(Default)]
struct StandaloneRejectionLedger(Arc<oryn_js::idb_owned_admission::Ledger>);

enum RejectionAllocation {
    Bindings(oryn_js::v8_bindings::BindingRetainedAllocation),
    Standalone { ledger: Arc<oryn_js::idb_owned_admission::Ledger>, limit: usize, used: usize },
}
impl RejectionAllocation {
    fn reserve(&self, bytes: usize) -> Result<Arc<dyn Send + Sync>, String> {
        match self {
            Self::Bindings(allocation) => allocation.reserve(bytes),
            Self::Standalone { ledger, limit, used } => {
                let lease = ledger.reserve(bytes, *limit, *used)?;
                Ok(Arc::new(lease))
            }
        }
    }
    fn buffer(&self, capacity: usize)
        -> Result<oryn_js::idb_owned_admission::Buffer<PromiseRejectionRecord>, String>
    {
        match self {
            Self::Bindings(allocation) => allocation.allocate_buffer(capacity),
            Self::Standalone { ledger, limit, used } =>
                oryn_js::idb_owned_admission::Buffer::allocate(ledger, capacity, *limit, *used)
                    .map_err(str::to_owned),
        }
    }
}

fn retain_weak_promise_rejection(
    scope: &mut v8::PinScope,
    promise: v8::Local<v8::Promise>,
    payload_visible: bool,
) -> Result<(), String> {
    let context = promise.get_creation_context(scope).ok_or("Promise creation context unavailable")?;
    let environment = *context.get_slot::<EnvironmentDescriptor>().ok_or("Promise creation environment unavailable")?;
    if !scope.get_slot::<AgentEnvironments>().is_some_and(|registry| registry.is_live(&environment)) {
        return Err("Promise creation environment retired".into());
    }
    let state = context.get_slot::<V8BindingState>();
    let authority_epoch = state.as_ref().map_or(0, |state| state.document_authority_epoch());
    if let Some((id, epoch, _)) = promise_creation_policy(scope, promise) {
        if id != environment.id || epoch != authority_epoch {
            return Err("Promise creation authority expired".into());
        }
    }
    {
        let records = scope.get_slot_mut::<PromiseRejections>().ok_or("Promise rejection registry unavailable")?;
        if let Some(buffer) = &mut records.records {
            buffer.retain(|record| !record.promise.is_empty());
            if buffer.as_slice().iter().any(|record| record.promise == promise) { return Ok(()) }
        }
        if records.len() >= 4096 { return Err("Promise rejection retention capacity".into()); }
    }
    let scope = &mut v8::ContextScope::new(scope, context);
    let notification_id = scope.get_slot::<PromiseRejections>()
        .ok_or("Promise rejection registry unavailable")?.next_notification_id
        .checked_add(1).ok_or("Promise rejection notification identity exhausted")?;
    let allocation = if let Some(state) = state {
        RejectionAllocation::Bindings(state.capture_retained_allocation(
            scope, Arc::new(std::sync::atomic::AtomicBool::new(false)),
        ).map_err(|error| error.to_string())?)
    } else {
        // Standalone JavaScriptHost intentionally has no DOM binding state.
        // Use the actual owning isolate's heap limit, not an unadmitted fallback.
        if scope.get_slot::<StandaloneRejectionLedger>().is_none() {
            scope.set_slot(StandaloneRejectionLedger::default());
        }
        let ledger = scope.get_slot::<StandaloneRejectionLedger>().unwrap().0.clone();
        if let Some(adjustment) = ledger.begin_adjustment()? {
            scope.adjust_amount_of_external_allocated_memory(adjustment.amount());
            ledger.finish_adjustment(adjustment)?;
        }
        let heap = scope.get_heap_statistics();
        let used = heap.used_heap_size().checked_add(heap.external_memory())
            .ok_or("standalone rejection heap accounting overflow")?;
        RejectionAllocation::Standalone { ledger, limit: heap.heap_size_limit(), used }
    };
    // Pinned rusty_v8 WeakData contains an optional pointer, optional usize
    // finalizer ID and a boolean (at most four native words), plus allocator
    // metadata. The V8 persistent handle itself remains in V8 heap accounting.
    let metadata = allocation.reserve(6 * std::mem::size_of::<usize>())?;
    let (len, capacity) = scope.get_slot::<PromiseRejections>()
        .map(|records| (records.len(), records.records.as_ref().map_or(0, |buffer| buffer.capacity())))
        .ok_or("Promise rejection registry unavailable")?;
    let replacement = if len == capacity {
        let next = capacity.checked_mul(2).unwrap_or(4096).max(1).min(4096);
        Some(allocation.buffer(next)?)
    } else { None };
    let weak = v8::Weak::new(scope, promise);
    let records = scope.get_slot_mut::<PromiseRejections>().ok_or("Promise rejection registry unavailable")?;
    if let Some(mut replacement) = replacement {
        if let Some(mut old) = records.records.take() {
            while let Some(record) = old.pop() { replacement.push(record).map_err(str::to_owned)?; }
            replacement.as_mut_slice().reverse();
        }
        records.records = Some(replacement);
    }
    records.records.as_mut().ok_or("Promise rejection buffer unavailable")?
        .push(PromiseRejectionRecord {
            notification_id,
            phase: crate::promise_rejection_lifecycle::RejectionPhase::Fresh,
            diagnostic_ready: false,
            handled_during_dispatch: false,
            promise: weak,
            environment: Some(environment.id),
            authority_epoch,
            payload_visible,
            _weak_metadata: metadata,
        }).map_err(str::to_owned)?;
    records.next_notification_id = notification_id;
    Ok(())
}
