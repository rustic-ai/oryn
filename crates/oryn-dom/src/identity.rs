//! Process-local identity. These values are not cross-process authority tokens.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::NodeId;

static NEXT_ARENA: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArenaId(u64);

impl ArenaId {
    pub(crate) fn fresh() -> Self {
        Self(
            NEXT_ARENA
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                    next.checked_add(1)
                })
                .expect("DOM arena identity space exhausted"),
        )
    }
}

/// Stable allocation identity, independent of the node's owning document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeRef {
    pub arena: ArenaId,
    pub node: NodeId,
}

/// Identity of a Document root, not of every node currently owned by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentId(pub(crate) NodeRef);

impl DocumentId {
    pub const fn root(self) -> NodeRef {
        self.0
    }
}
