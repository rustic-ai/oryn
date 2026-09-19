//! Shared ownership for script tokens admitted off the page thread.
#[cfg(unix)]
use crate::context_service::ContextScriptLoad;
use std::sync::{Arc, Mutex, Weak};
#[cfg(not(unix))]
type ContextScriptLoad = ();
#[cfg(unix)]
pub(super) type ResourceJob = crate::context_service::ResourceJob;
#[cfg(not(unix))]
pub(super) type ResourceJob = ();
pub(super) type WeakScriptAdmission = Weak<Mutex<Option<ContextScriptLoad>>>;

pub(super) type SharedScriptAdmission = Arc<Mutex<Option<ContextScriptLoad>>>;

#[cfg(unix)]
pub(super) fn reserve_script_admission(
    client: &crate::context_service::SharedContextClient,
    page_id: oryn_common::v3::PageId,
    creator: oryn_js::worker_wire::Creator,
    owner: &Weak<Mutex<Option<ContextScriptLoad>>>,
) -> Result<u64, crate::RuntimeError> {
    if owner.strong_count() == 0 {
        return Err(crate::RuntimeError::ContextService(
            "script retired before admission".into(),
        ));
    }
    let lease = client.reserve_script_load(page_id, creator)?;
    publish_script_admission(owner, lease).ok_or_else(|| {
        crate::RuntimeError::ContextService("script retired during admission".into())
    })
}

#[cfg(unix)]
pub(super) fn publish_script_admission(
    owner: &Weak<Mutex<Option<ContextScriptLoad>>>,
    lease: ContextScriptLoad,
) -> Option<u64> {
    // The IO job retains no strong owner while admission is pending. A late
    // token for an abandoned preparation is cancelled here on its IO lane.
    let owner = owner.upgrade()?;
    let mut slot = owner.lock().ok()?;
    if slot.is_some() {
        return None;
    }
    let token = lease.token;
    *slot = Some(lease);
    Some(token)
}
