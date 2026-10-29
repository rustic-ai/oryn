// Native embedding admission for known retained allocations. This uses the
// owning isolate's existing ledger and configured heap limit, never a CSS cap.
#[derive(Default)]
struct ExecutionRetirement {
    retired: std::cell::Cell<bool>,
    native: std::sync::Arc<std::sync::Mutex<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
}
impl ExecutionRetirement {
    fn get(&self) -> bool { self.retired.get() }
    fn cancel_native(&self) {
        self.native.lock().unwrap_or_else(std::sync::PoisonError::into_inner).store(true, std::sync::atomic::Ordering::Release);
    }
    fn set(&self, retired: bool) {
        self.cancel_native();
        if !retired {
            // A replacement execution gets a new token. Old captured native
            // work cannot regain authority when the binding becomes live.
            *self.native.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        }
        self.retired.set(retired);
    }
    #[cfg(test)]
    fn capture(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> { self.native.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone() }
}
pub struct BindingRetainedAllocation {
    ledger: std::sync::Arc<crate::idb_owned_admission::Ledger>,
    retired: std::sync::Arc<std::sync::atomic::AtomicBool>,
    // Allocation accounting follows a reused Window's current execution;
    // individual native work captures a permanently revocable token below.
    execution_retired: std::sync::Arc<std::sync::Mutex<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
    snapshot: std::sync::Mutex<RetainedSnapshot>,
    owner_thread: std::thread::ThreadId,
    // Payload fields above dispose before the independent metadata lease.
    native_metadata: Option<std::sync::Arc<dyn Send + Sync>>,
}
#[derive(Default)]
struct RetainedSnapshot {
    receipt: Option<crate::idb_owned_admission::HeapReceipt>,
    epoch: u64,
    active: usize,
}
impl RetainedSnapshot {
    fn revoke(&mut self) {
        self.receipt = None;
        self.active = 0;
        // Exhaustion permanently fails closed; no stale guard can regain
        // authority by wrapping its generation back to an earlier epoch.
        self.epoch = self.epoch.checked_add(1).unwrap_or(u64::MAX);
    }
}
impl BindingRetainedAllocation {
    /// Attach an already admitted metadata owner before publishing this receipt.
    /// No container growth, callbacks, or cycle through the receipt is permitted.
    pub fn with_metadata_owner(mut self, owner: std::sync::Arc<dyn Send + Sync>) -> Self {
        self.native_metadata = Some(owner);
        self
    }

    /// Pure parent-owned cancellation observation; never runs allocation,
    /// author callbacks or native admission while a style engine is borrowed.
    pub fn owner_cancelled(&self) -> bool {
        self.retired.load(std::sync::atomic::Ordering::Acquire)
            || self.execution_retired.lock().map(|token| token.load(std::sync::atomic::Ordering::Acquire)).unwrap_or(true)
    }
    pub fn capture_cancellation(&self) -> Result<std::sync::Arc<dyn Fn() -> bool + Send + Sync>, String> {
        let token = self.execution_retired.lock().map_err(|_| "execution retirement token poisoned")?.clone();
        let retired = self.retired.clone();
        Ok(std::sync::Arc::new(move || retired.load(std::sync::atomic::Ordering::Acquire)
            || token.load(std::sync::atomic::Ordering::Acquire)))
    }
    pub fn invalidate(&self) -> Result<(), String> {
        self.snapshot.lock()
            .map_err(|_| "retained allocation snapshot poisoned")?.revoke();
        Ok(())
    }
    pub fn scope_guard(self: &std::sync::Arc<Self>) -> BindingRetainedAllocationScope {
        let mut snapshot = self.snapshot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let epoch = if std::thread::current().id() == self.owner_thread
            && snapshot.receipt.is_some() && snapshot.epoch != u64::MAX {
            if let Some(active) = snapshot.active.checked_add(1) {
                snapshot.active = active;
                Some(snapshot.epoch)
            } else {
                snapshot.revoke();
                None
            }
        } else { None };
        BindingRetainedAllocationScope { allocation: self.clone(), epoch }
    }
    pub fn reserve(&self, bytes: usize) -> Result<std::sync::Arc<dyn Send + Sync>, String> {
        if self.owner_cancelled() {
            return Err("native allocation owner retired".into());
        }
        if std::thread::current().id() != self.owner_thread {
            return Err("retained allocation requires owning native scope".into());
        }
        let snapshot = self
            .snapshot
            .lock()
            .map_err(|_| "retained allocation snapshot poisoned")?.receipt.clone();
        let receipt = snapshot.ok_or("retained allocation snapshot expired")?;
        let lease = self.ledger.reserve_from_receipt(
            bytes
                .checked_add(2 * std::mem::size_of::<usize>())
                .and_then(|bytes| {
                    bytes.checked_add(std::mem::size_of::<crate::idb_owned_admission::Lease>())
                })
                .ok_or("retained allocation metadata overflow")?,
            &receipt,
        )?;
        if self.owner_cancelled() {
            return Err("native allocation owner retired".into());
        }
        Ok(std::sync::Arc::new(lease))
    }
    pub fn copy_text(&self, source: &str) -> Result<BindingRetainedText, String> {
        if self.owner_cancelled() {
            return Err("native allocation owner retired".into());
        }
        if std::thread::current().id() != self.owner_thread {
            return Err("retained text requires owning native scope".into());
        }
        let snapshot = self
            .snapshot
            .lock()
            .map_err(|_| "retained allocation snapshot poisoned")?.receipt.clone();
        let receipt = snapshot.ok_or("retained allocation snapshot expired")?;
        let text = crate::idb_owned_admission::Text::copy_string_from_receipt(&self.ledger, source, &receipt, || self.owner_cancelled())
            .map_err(str::to_owned)?;
        if self.owner_cancelled() {
            return Err("native allocation owner retired".into());
        }
        Ok(BindingRetainedText { text })
    }
    pub fn refresh(&self, scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
        if std::thread::current().id() != self.owner_thread {
            return Err(V8BindingError::Allocation(
                "foreign retained allocation scope",
            ));
        }
        // Refreshing an inner native entry invalidates the old statistics
        // during V8 adjustment, but preserves the live scope epoch on success.
        let epoch = {
            let mut snapshot = self.snapshot.lock()
                .map_err(|_| V8BindingError::Allocation("retained snapshot lock"))?;
            snapshot.receipt = None;
            snapshot.epoch
        };
        if epoch == u64::MAX { return Err(V8BindingError::Allocation("retained scope epoch exhausted")); }
        let result = (|| {
        if scope.is_execution_terminating()
            || self.owner_cancelled()
        {
            return Err(V8BindingError::JavaScriptException);
        }
        if let Some(receipt) = self
            .ledger
            .begin_adjustment()
            .map_err(V8BindingError::EventCallback)?
        {
            scope.adjust_amount_of_external_allocated_memory(receipt.amount());
            self.ledger
                .finish_adjustment(receipt)
                .map_err(V8BindingError::EventCallback)?;
        }
        if scope.is_execution_terminating()
            || self.owner_cancelled()
        {
            return Err(V8BindingError::JavaScriptException);
        }
        let heap = scope.get_heap_statistics();
        let _used = heap
            .used_heap_size()
            .checked_add(heap.external_memory())
            .ok_or(V8BindingError::Allocation(
                "native retained heap accounting",
            ))?;
        let receipt = self.ledger.heap_receipt_with_statistics(heap.heap_size_limit(),
            heap.used_heap_size(),heap.external_memory()).map_err(V8BindingError::EventCallback)?;
        let mut snapshot = self.snapshot.lock()
            .map_err(|_| V8BindingError::Allocation("retained snapshot lock"))?;
        if snapshot.epoch != epoch {
            return Err(V8BindingError::Allocation("retained scope revoked during refresh"));
        }
        snapshot.receipt = Some(receipt);
        Ok(())
        })();
        if result.is_err() { self.invalidate().map_err(V8BindingError::EventCallback)?; }
        result
    }
}
pub struct BindingRetainedAllocationScope {
    allocation: std::sync::Arc<BindingRetainedAllocation>,
    epoch: Option<u64>,
}
impl Drop for BindingRetainedAllocationScope {
    fn drop(&mut self) {
        // No V8/DOM callback occurs during cleanup, including error unwind.
        // Poisoned snapshot is never authority: erase it as well.
        let mut snapshot = self
            .allocation
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.epoch == Some(snapshot.epoch) {
            debug_assert!(snapshot.active > 0, "retained scope count underflow");
            snapshot.active = snapshot.active.checked_sub(1).unwrap_or(0);
            if snapshot.active == 0 { snapshot.receipt = None; }
        }
    }
}
pub struct BindingRetainedText {
    text: crate::idb_owned_admission::Text,
}
impl AsRef<str> for BindingRetainedText {
    fn as_ref(&self) -> &str {
        self.text.as_str()
    }
}
impl oryn_common::owned_stylesheet_text::OwnedStylesheetText for BindingRetainedText {}
impl V8BindingState {
    /// Generated DOM dispatch remains usable after its document loses execution
    /// services. Do not refresh or publish native admission for a cancelled
    /// owner: native consumers must still enter_retained_allocation themselves.
    fn enter_retained_dom_allocation(
        &self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<BindingRetainedAllocationScope>, V8BindingError> {
        let allocation = self.retained_allocation.try_borrow()
            .map_err(|_| V8BindingError::Allocation("retained DOM receipt entry borrow"))?
            .as_ref().cloned();
        if let Some(allocation) = allocation {
            if allocation.owner_cancelled() {
                allocation.invalidate().map_err(V8BindingError::EventCallback)?;
                return Ok(None);
            }
            allocation.refresh(scope)?;
            Ok(Some(allocation.scope_guard()))
        } else {
            Ok(None)
        }
    }
    pub fn install_retained_allocation_with(
        &self,
        allocation: std::sync::Arc<BindingRetainedAllocation>,
        publish: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), V8BindingError> {
        // The prepared native finalizer cannot allocate or invoke callbacks.
        // Both receiver and provider identities are validated before mutation.
        let mut installed = self
            .retained_allocation
            .try_borrow_mut()
            .map_err(|_| V8BindingError::Allocation("retained receipt installation borrow"))?;
        if let Some(existing) = installed.as_ref()
            && !std::sync::Arc::ptr_eq(existing, &allocation)
        {
            return Err(V8BindingError::Allocation("retained receipt replacement"));
        }
        publish().map_err(|_| V8BindingError::Allocation("native provider receipt publication"))?;
        if installed.is_none() {
            *installed = Some(allocation);
        }
        Ok(())
    }
    pub fn install_retained_allocation(
        &self,
        allocation: std::sync::Arc<BindingRetainedAllocation>,
    ) -> Result<(), V8BindingError> {
        let mut installed = self
            .retained_allocation
            .try_borrow_mut()
            .map_err(|_| V8BindingError::Allocation("retained receipt installation borrow"))?;
        if let Some(existing) = installed.as_ref() {
            if !std::sync::Arc::ptr_eq(existing, &allocation) {
                return Err(V8BindingError::Allocation("retained receipt replacement"));
            }
        } else {
            *installed = Some(allocation);
        }
        Ok(())
    }
    pub fn enter_retained_allocation(
        &self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<BindingRetainedAllocationScope>, V8BindingError> {
        let allocation = self
            .retained_allocation
            .try_borrow()
            .map_err(|_| V8BindingError::Allocation("retained receipt entry borrow"))?
            .as_ref()
            .cloned();
        if let Some(allocation) = allocation {
            allocation.refresh(scope)?;
            Ok(Some(allocation.scope_guard()))
        } else {
            Ok(None)
        }
    }
    pub fn capture_retained_allocation(
        &self,
        scope: &mut v8::PinScope,
        retired: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<BindingRetainedAllocation, V8BindingError> {
        let admission = BindingRetainedAllocation {
            ledger: self.idb_payload_ledger.clone(),
            retired,
            execution_retired: self.control_tasks_retired.native.clone(),
            snapshot: std::sync::Mutex::new(RetainedSnapshot::default()),
            owner_thread: std::thread::current().id(),
            native_metadata: None,
        };
        admission.refresh(scope)?;
        Ok(admission)
    }
}

#[cfg(test)]
mod nested_retained_scope_controls {
    use super::*;
    #[test]
    fn execution_replacement_never_revives_captured_native_work() {
        use std::sync::atomic::Ordering;
        let retirement = ExecutionRetirement::default();
        let original = retirement.capture();
        assert!(!original.load(Ordering::Acquire));
        retirement.cancel_native();
        assert!(original.load(Ordering::Acquire));
        assert!(!retirement.get());
        retirement.set(true);
        assert!(retirement.get());
        retirement.set(false);
        assert!(!retirement.get());
        assert!(!retirement.capture().load(Ordering::Acquire));
        assert!(original.load(Ordering::Acquire));
    }
    #[test]
    fn nested_native_scopes_preserve_outer_admission_and_explicit_revocation() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = std::rc::Rc::new(std::cell::RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(994), 1).unwrap();
        let retired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let allocation = std::sync::Arc::new(state.capture_retained_allocation(scope, retired.clone()).unwrap());
        state.install_retained_allocation(allocation.clone()).unwrap();
        let outer = state.enter_retained_allocation(scope).unwrap();
        {
            let _inner = state.enter_retained_allocation(scope).unwrap();
            assert!(allocation.reserve(8).is_ok());
            assert!(allocation.copy_text("inner").is_ok());
        }
        assert!(allocation.reserve(8).is_ok(), "inner exit must preserve original active native owner");
        assert!(allocation.copy_text("outer").is_ok());
        let first = state.enter_retained_allocation(scope).unwrap();
        let second = state.enter_retained_allocation(scope).unwrap();
        drop(first);
        assert!(allocation.reserve(8).is_ok(), "non-LIFO guard drop cannot expire another active entry");
        drop(second);
        allocation.invalidate().unwrap();
        assert!(allocation.reserve(8).is_err());
        { let _new = state.enter_retained_allocation(scope).unwrap(); assert!(allocation.reserve(8).is_ok()); }
        assert!(allocation.reserve(8).is_err(), "revoked outer epoch must not resurrect admission");
        drop(outer);
        let fresh = state.enter_retained_allocation(scope).unwrap();
        let old_work = allocation.capture_cancellation().unwrap();
        state.control_tasks_retired.set(true);
        assert!(old_work());
        assert!(allocation.reserve(8).is_err());
        assert!(state.enter_retained_dom_allocation(scope).unwrap().is_none(),
            "retained DOM dispatch must not require execution authority");
        assert!(state.enter_retained_allocation(scope).is_err());
        assert!(allocation.reserve(8).is_err(), "DOM entry must not revive native admission");
        state.control_tasks_retired.set(false);
        assert!(old_work(), "replacement cannot revive captured work");
        let _replacement = state.enter_retained_allocation(scope).unwrap();
        assert!(allocation.reserve(8).is_ok(), "replacement preserves the reused accounting ledger");
        assert!(!allocation.capture_cancellation().unwrap()());
        retired.store(true, std::sync::atomic::Ordering::Release);
        assert!(state.enter_retained_allocation(scope).is_err());
        assert!(allocation.reserve(8).is_err());
        retired.store(false, std::sync::atomic::Ordering::Release);
        assert!(allocation.reserve(8).is_err(), "failed nested refresh stays expired");
        drop(fresh);
    }
}
