//! Admission for concrete font-byte copies, not opaque shaping scratch space.
use crate::cssom::native_import_graph::ImportAdmission;
use std::sync::Arc;

pub(crate) struct Owner {
    admission: Arc<dyn ImportAdmission + Send + Sync>,
    // Byte payload owners in TextEngine/CanvasTextShape must drop first.
    leases: Vec<Arc<dyn Send + Sync>>,
    metadata: Option<Arc<dyn Send + Sync>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Lease { live: Arc<AtomicUsize>, bytes: usize }
    impl Drop for Lease { fn drop(&mut self) { self.live.fetch_sub(self.bytes, Ordering::SeqCst); } }
    struct Admission { live: Arc<AtomicUsize>, checks: AtomicUsize, deny_check: usize, deny_reserve: bool }
    impl ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            if self.checks.fetch_add(1, Ordering::SeqCst) == self.deny_check { Err("normalization cancelled".into()) } else { Ok(()) }
        }
        fn reserve(&self, bytes: usize) -> Result<Arc<dyn Send + Sync>, String> {
            if self.deny_reserve { return Err("normalization denied".into()); }
            self.live.fetch_add(bytes, Ordering::SeqCst);
            Ok(Arc::new(Lease { live: self.live.clone(), bytes }))
        }
    }
    fn admission(deny_check: usize, deny_reserve: bool) -> Arc<Admission> {
        Arc::new(Admission { live: Arc::new(AtomicUsize::new(0)), checks: AtomicUsize::new(0), deny_check, deny_reserve })
    }
    #[test]
    fn normalized_canvas_utf8_is_exact_admitted_and_preserves_non_ascii_and_nul() {
        let admission = admission(usize::MAX, false);
        let owner = Owner::new(admission.clone());
        let source = " a\r\n\t\u{c}\0\u{b}\u{fffd}🔥\u{a0} ";
        let text = owner.normalize_text(source, &None).unwrap();
        assert_eq!(text.value, " a    \0\u{b}\u{fffd}🔥\u{a0} ");
        assert_eq!(text.value.capacity(), source.len());
        assert_eq!(admission.live.load(Ordering::SeqCst), source.len());
        drop(owner);
        assert_eq!(admission.live.load(Ordering::SeqCst), source.len(), "text owns the reservation independently");
        drop(text);
        assert_eq!(admission.live.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn normalized_canvas_denial_and_mid_copy_cancellation_release_storage_charges() {
        let source = "a\r\n🔥".repeat(1024);
        let control = admission(usize::MAX, false);
        drop(Owner::new(control.clone()).normalize_text(&source, &None).unwrap());
        let checks = control.checks.load(Ordering::SeqCst);
        for denial in 0..checks {
            let admission = admission(denial, false);
            assert!(Owner::new(admission.clone()).normalize_text(&source, &None).is_err());
            assert_eq!(admission.live.load(Ordering::SeqCst), 0, "checkpoint {denial}");
        }
        let denied = admission(usize::MAX, true);
        assert!(Owner::new(denied.clone()).normalize_text(&source, &None).is_err());
        assert_eq!(denied.live.load(Ordering::SeqCst), 0);
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let cancelled: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || count.fetch_add(1, Ordering::SeqCst) >= 2);
        let control = admission(usize::MAX, false);
        assert!(matches!(Owner::new(control.clone()).normalize_text(&source, &Some(cancelled)), Err(crate::LayoutError::Cancelled)));
        assert_eq!(control.live.load(Ordering::SeqCst), 0);
    }
}
pub(crate) struct NormalizedText {
    pub(crate) value: String,
    // Dispose the normalized string before releasing its byte reservation.
    _lease: Arc<dyn Send + Sync>,
}
impl std::fmt::Debug for Owner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CanvasFontByteOwner")
    }
}
impl Owner {
    pub(crate) fn new(admission: Arc<dyn ImportAdmission + Send + Sync>) -> Self {
        Self { admission, leases: Vec::new(), metadata: None }
    }
    pub(crate) fn copy(&mut self, source: &[u8]) -> Result<Vec<u8>, String> {
        self.admission.checkpoint()?;
        if self.leases.len() == self.leases.capacity() {
            let capacity = self.leases.len().checked_add(1).ok_or("font byte lease overflow")?;
            let charge = self.admission.reserve(capacity.checked_mul(std::mem::size_of::<Arc<dyn Send + Sync>>()).ok_or("font byte lease metadata overflow")?)?;
            let mut replacement = Vec::new();
            replacement.try_reserve_exact(capacity).map_err(|e| e.to_string())?;
            if replacement.capacity() > capacity { return Err("font byte lease metadata exceeded admission".into()); }
            replacement.append(&mut self.leases);
            self.leases = replacement;
            self.metadata = Some(charge);
        }
        let charge = self.admission.reserve(source.len())?;
        // Declare bytes after the charge so error unwind disposes bytes first.
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(source.len()).map_err(|e| e.to_string())?;
        if bytes.capacity() > source.len() { return Err("font bytes exceeded admission".into()); }
        bytes.extend_from_slice(source);
        self.admission.checkpoint()?;
        self.leases.push(charge);
        Ok(bytes)
    }
    pub(crate) fn normalize_text(&self, source: &str, cancelled: &Option<Arc<dyn Fn() -> bool + Send + Sync>>) -> Result<NormalizedText, crate::LayoutError> {
        use crate::LayoutError;
        crate::layout_checkpoint(cancelled)?;
        self.admission.checkpoint().map_err(LayoutError::StyleTraversal)?;
        // Every Canvas ASCII whitespace replacement is one UTF-8 byte. Other
        // scalar values keep their bytes, so this is an exact capacity bound.
        let lease = self.admission.reserve(source.len()).map_err(LayoutError::StyleTraversal)?;
        let mut value = String::new();
        value.try_reserve_exact(source.len()).map_err(|e| LayoutError::StyleTraversal(e.to_string()))?;
        if value.capacity() > source.len() { return Err(LayoutError::StyleTraversal("Canvas normalized text exceeded admission".into())); }
        for (index, ch) in source.chars().enumerate() {
            if index % 1024 == 0 {
                crate::layout_checkpoint(cancelled)?;
                self.admission.checkpoint().map_err(LayoutError::StyleTraversal)?;
            }
            value.push(if matches!(ch, '\t' | '\n' | '\u{c}' | '\r' | ' ') { ' ' } else { ch });
        }
        crate::layout_checkpoint(cancelled)?;
        self.admission.checkpoint().map_err(LayoutError::StyleTraversal)?;
        Ok(NormalizedText { value, _lease: lease })
    }
}
