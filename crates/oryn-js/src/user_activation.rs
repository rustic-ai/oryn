//! Trusted-input publication; no JavaScript binding constructs this backend.
pub trait ActivationBackend: Send + Sync {
    fn notify(&self, creator: crate::worker_wire::Creator) -> Result<(), String>;
    fn sticky(&self, _creator: crate::worker_wire::Creator) -> Result<bool, String> {
        Ok(false)
    }
    /// Query/consume exact native document activation. Absence never grants it.
    fn transient(
        &self,
        _creator: crate::worker_wire::Creator,
        _consume: bool,
    ) -> Result<bool, String> {
        Ok(false)
    }
}

/// Native, thread-affine standalone document activation authority.
/// Hosts install this explicitly; author code cannot select this route.
pub trait LocalActivationBackend {
    fn notify(&self, creator: crate::worker_wire::Creator) -> Result<(), String>;
    fn sticky(&self, creator: crate::worker_wire::Creator) -> Result<bool, String>;
    fn transient(
        &self,
        creator: crate::worker_wire::Creator,
        consume: bool,
    ) -> Result<bool, String>;
}
