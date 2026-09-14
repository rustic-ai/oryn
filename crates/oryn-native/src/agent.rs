//! Thread-affine owner of a window agent's V8 host and document arena.
//!
//! Loading/parser state remains in the document driver. The host owns the
//! agent task queues, environment lifetimes and realm contexts. Placement is
//! unchanged: this type does not authorize sharing independently rooted pages.
//! The legacy host still co-hosts opaque foreign-frame contexts with separate
//! arenas; splitting those execution lanes is a subsequent placement cutover.

use oryn_dom::SharedDomRuntime;

use crate::host::{HostError, RawV8Host};

pub(crate) struct AgentRuntime {
    // Drop the host (including its realm wrappers and execution services)
    // before releasing the owning arena reference.
    pub(crate) host: RawV8Host,
    pub(crate) dom: SharedDomRuntime,
}

impl AgentRuntime {
    pub(crate) fn new(host: RawV8Host, dom: SharedDomRuntime) -> Result<Self, HostError> {
        host.validate_primary_arena(&dom)?;
        Ok(Self { host, dom })
    }
}
