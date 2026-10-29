//! Admission for concrete font-byte copies, not opaque shaping scratch space.
use crate::cssom::native_import_graph::ImportAdmission;
use std::sync::Arc;

pub(crate) struct Owner {
    admission: Arc<dyn ImportAdmission + Send + Sync>,
    // Byte payload owners in TextEngine/CanvasTextShape must drop first.
    leases: Vec<Arc<dyn Send + Sync>>,
    metadata: Option<Arc<dyn Send + Sync>>,
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
}
