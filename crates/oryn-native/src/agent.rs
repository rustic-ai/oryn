//! Thread-affine owner of a window agent's V8 host and document arena.
//!
//! Loading/parser state remains in the document driver. The host owns the
//! agent task queues, environment lifetimes and realm contexts. Placement is
//! unchanged: this type does not authorize sharing independently rooted pages.
//! The legacy host still co-hosts opaque foreign-frame contexts with separate
//! arenas; splitting those execution lanes is a subsequent placement cutover.

use oryn_dom::SharedDomRuntime;
use std::ops::{Deref, DerefMut};

use crate::host::{HostError, RawV8Host};

pub(crate) struct AgentRuntime {
    // Drop the host (including its realm wrappers and execution services)
    // before releasing the owning arena reference.
    pub(crate) host: DocumentHost,
    pub(crate) identity: std::rc::Rc<()>,
    pub(crate) dom: SharedDomRuntime,
}

impl AgentRuntime {
    pub(crate) fn detached(dom: SharedDomRuntime, identity: std::rc::Rc<()>) -> Self {
        Self {
            host: DocumentHost(None),
            identity,
            dom,
        }
    }
}

/// Exclusive custody during a document turn. This is deliberately neither
/// Clone nor interior-mutably shared: callbacks enter native realm contexts,
/// never borrow a second document's outer host. The actor moves the unique host
/// between driver turns; a detached driver cannot silently execute elsewhere.
pub(crate) struct DocumentHost(Option<RawV8Host>);

impl DocumentHost {
    pub(crate) fn is_attached(&self) -> bool {
        self.0.is_some()
    }
    pub(crate) fn take(&mut self) -> Result<RawV8Host, HostError> {
        self.0
            .take()
            .ok_or_else(|| HostError::Execution("document has no agent turn custody".into()))
    }
    pub(crate) fn install(&mut self, host: RawV8Host) -> Result<(), Box<RawV8Host>> {
        if self.0.is_some() {
            return Err(Box::new(host));
        }
        self.0 = Some(host);
        Ok(())
    }
}
impl Deref for DocumentHost {
    type Target = RawV8Host;
    fn deref(&self) -> &RawV8Host {
        self.0
            .as_ref()
            .expect("document execution requires exclusive agent turn custody")
    }
}
impl DerefMut for DocumentHost {
    fn deref_mut(&mut self) -> &mut RawV8Host {
        self.0
            .as_mut()
            .expect("document execution requires exclusive agent turn custody")
    }
}

#[cfg(test)]
mod custody_tests {
    use super::*;
    #[test]
    fn a_host_can_have_only_one_document_custodian() {
        let mut first = DocumentHost(Some(RawV8Host::new()));
        let mut second = DocumentHost(None);
        let host = first.take().unwrap();
        assert!(first.take().is_err());
        assert!(second.install(host).is_ok());
        let extra = RawV8Host::new();
        let rejected_identity = extra.agent_host_identity();
        let rejected = second
            .install(extra)
            .expect_err("occupied driver rejects transfer");
        assert!(std::rc::Rc::ptr_eq(
            &rejected_identity,
            &rejected.agent_host_identity()
        ));
        drop(rejected);
        assert!(first.install(second.take().unwrap()).is_ok());
        assert!(second.take().is_err());
    }
}
