//! Parent queue lease adapter. No V8 handles and no duplicated shared bytes.
use super::SharedMemoryRegistry;
use oryn_js::{
    message_wire::Principal,
    shared_memory::{SharedMemoryReference, SharedMessageLease},
};
use std::sync::{Arc, Mutex, Weak};
pub(crate) struct SharedMessageBackend(pub Arc<Mutex<SharedMemoryRegistry>>);
struct QueueLease {
    registry: Weak<Mutex<SharedMemoryRegistry>>,
    references: Vec<SharedMemoryReference>,
}
impl std::fmt::Debug for QueueLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedQueueLease")
            .field("backings", &self.references.len())
            .finish()
    }
}
impl SharedMessageLease for QueueLease {
    fn deliver(
        &self,
        target: oryn_js::shared_memory::SharedMemoryRecipient,
    ) -> Result<Vec<SharedMemoryReference>, String> {
        self.registry
            .upgrade()
            .ok_or("shared queue context retired")?
            .lock()
            .map_err(|_| "shared queue registry poisoned")?
            .deliver_shared(target, &self.references)
    }
}
impl Drop for QueueLease {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            let _ = registry
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .release_shared_queue(&self.references);
        }
    }
}
impl oryn_js::shared_memory::SharedMessageBackend for SharedMessageBackend {
    fn adopt(
        &self,
        source: Principal,
        references: &[SharedMemoryReference],
    ) -> Result<Arc<dyn SharedMessageLease>, String> {
        let references = self
            .0
            .lock()
            .map_err(|_| "shared queue registry poisoned")?
            .adopt_shared(source, references)?;
        Ok(Arc::new(QueueLease {
            registry: Arc::downgrade(&self.0),
            references,
        }))
    }
    fn discard_delivered(
        &self,
        target: Principal,
        references: &[SharedMemoryReference],
    ) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|_| "shared queue registry poisoned")?
            .discard_shared_delivered(target, references)
    }
    fn discard_offered(
        &self,
        source: Principal,
        references: &[SharedMemoryReference],
    ) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|_| "shared queue registry poisoned")?
            .discard_shared_offered(source, references)
    }
    fn retire_principal(&self, source: Principal) {
        self.0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .retire_shared_principal(source);
    }
}
