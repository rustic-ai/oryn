// Native embedding admission for known retained allocations. This uses the
// owning isolate's existing ledger and configured heap limit, never a CSS cap.
pub struct BindingRetainedAllocation {
    ledger: std::sync::Arc<crate::idb_owned_admission::Ledger>,
    retired: std::sync::Arc<std::sync::atomic::AtomicBool>,
    snapshot: std::sync::Mutex<Option<crate::idb_owned_admission::HeapReceipt>>,
    owner_thread: std::thread::ThreadId,
}
impl BindingRetainedAllocation {
    /// Pure parent-owned cancellation observation; never runs allocation,
    /// author callbacks or native admission while a style engine is borrowed.
    pub fn owner_cancelled(&self) -> bool {
        self.retired.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn invalidate(&self) -> Result<(), String> {
        *self
            .snapshot
            .lock()
            .map_err(|_| "retained allocation snapshot poisoned")? = None;
        Ok(())
    }
    pub fn scope_guard(self: &std::sync::Arc<Self>) -> BindingRetainedAllocationScope {
        BindingRetainedAllocationScope {
            allocation: self.clone(),
        }
    }
    pub fn reserve(&self, bytes: usize) -> Result<std::sync::Arc<dyn Send + Sync>, String> {
        use std::sync::atomic::Ordering;
        if self.retired.load(Ordering::Acquire) {
            return Err("native allocation owner retired".into());
        }
        if std::thread::current().id() != self.owner_thread {
            return Err("retained allocation requires owning native scope".into());
        }
        let snapshot = self
            .snapshot
            .lock()
            .map_err(|_| "retained allocation snapshot poisoned")?.clone();
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
        if self.retired.load(Ordering::Acquire) {
            return Err("native allocation owner retired".into());
        }
        Ok(std::sync::Arc::new(lease))
    }
    pub fn copy_text(&self, source: &str) -> Result<BindingRetainedText, String> {
        use std::sync::atomic::Ordering;
        if self.retired.load(Ordering::Acquire) {
            return Err("native allocation owner retired".into());
        }
        if std::thread::current().id() != self.owner_thread {
            return Err("retained text requires owning native scope".into());
        }
        let snapshot = self
            .snapshot
            .lock()
            .map_err(|_| "retained allocation snapshot poisoned")?.clone();
        let receipt = snapshot.ok_or("retained allocation snapshot expired")?;
        let text = crate::idb_owned_admission::Text::copy_string_from_receipt(&self.ledger, source, &receipt, || self.retired.load(Ordering::Acquire))
            .map_err(str::to_owned)?;
        if self.retired.load(Ordering::Acquire) {
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
        *self
            .snapshot
            .lock()
            .map_err(|_| V8BindingError::Allocation("retained snapshot lock"))? = None;
        if scope.is_execution_terminating()
            || self.retired.load(std::sync::atomic::Ordering::Acquire)
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
            || self.retired.load(std::sync::atomic::Ordering::Acquire)
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
        *self
            .snapshot
            .lock()
            .map_err(|_| V8BindingError::Allocation("retained snapshot lock"))? =
            Some(self.ledger.heap_receipt_with_statistics(heap.heap_size_limit(),
                heap.used_heap_size(),heap.external_memory()).map_err(V8BindingError::EventCallback)?);
        Ok(())
    }
}
pub struct BindingRetainedAllocationScope {
    allocation: std::sync::Arc<BindingRetainedAllocation>,
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
        *snapshot = None;
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
            snapshot: std::sync::Mutex::new(None),
            owner_thread: std::thread::current().id(),
        };
        admission.refresh(scope)?;
        Ok(admission)
    }
}
