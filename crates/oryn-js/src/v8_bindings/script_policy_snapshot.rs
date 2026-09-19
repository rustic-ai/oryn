//! Native-only immutable policy snapshots shared by prepared script owners.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
static SOURCE_BYTES: AtomicUsize = AtomicUsize::new(0);
pub struct ScriptPolicySnapshot {
    serialized: String,
    payload: Box<dyn std::any::Any + Send + Sync>,
    charged: usize,
}
impl ScriptPolicySnapshot {
    pub fn new<T: std::any::Any + Send + Sync>(
        serialized: String,
        payload: T,
    ) -> Result<Arc<Self>, String> {
        if serialized.len() > 4 * 1024 * 1024 {
            return Err("script policy snapshot size limit".into());
        }
        // The serialized representation and native policy string storage are
        // retained together; all prepared consumers share this one allocation.
        let charged = serialized.len().saturating_mul(2);
        SOURCE_BYTES
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(charged)
                    .filter(|next| *next <= 32 * 1024 * 1024)
            })
            .map_err(|_| "script policy snapshot source capacity")?;
        Ok(Arc::new(Self {
            serialized,
            payload: Box::new(payload),
            charged,
        }))
    }
    pub fn as_str(&self) -> &str {
        &self.serialized
    }
    pub fn native<T: std::any::Any>(&self) -> Option<&T> {
        self.payload.downcast_ref()
    }
}
impl Drop for ScriptPolicySnapshot {
    fn drop(&mut self) {
        SOURCE_BYTES.fetch_sub(self.charged, Ordering::AcqRel);
    }
}
