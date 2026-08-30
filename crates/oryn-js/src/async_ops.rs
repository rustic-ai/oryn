//! Page-thread registry for asynchronous host operations.
//!
//! The registry owns no sockets and stores no V8 pointers. Generated bindings
//! associate their promise resolver or stream controller with the returned
//! operation ID inside the isolate, while this Rust state enforces realm and
//! document cancellation and rejects late completions.

use std::collections::BTreeMap;

use crate::realm::RealmId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Fetch,
    XmlHttpRequest,
    Module,
    Resource,
    WebSocket,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationEvent {
    Headers {
        status: u16,
        headers: Vec<(String, String)>,
    },
    Chunk(Vec<u8>),
    Complete,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationOwner {
    pub realm: RealmId,
    pub document_generation: u64,
    pub kind: OperationKind,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OperationError {
    #[error("pending operation limit exceeded")]
    LimitExceeded,
    #[error("duplicate operation {0:?}")]
    Duplicate(OperationId),
    #[error("late or unmatched completion for operation {0:?}")]
    Unmatched(OperationId),
    #[error("operation {0:?} produced body bytes before accepted headers")]
    BodyBeforeHeaders(OperationId),
    #[error("operation {0:?} produced duplicate response headers")]
    DuplicateHeaders(OperationId),
}

#[derive(Debug)]
struct PendingOperation {
    owner: OperationOwner,
    headers_accepted: bool,
}

#[derive(Debug)]
pub struct AsyncOperationRegistry {
    max_pending: usize,
    pending: BTreeMap<OperationId, PendingOperation>,
}

impl AsyncOperationRegistry {
    pub fn new(max_pending: usize) -> Self {
        Self {
            max_pending,
            pending: BTreeMap::new(),
        }
    }

    pub fn begin(&mut self, id: OperationId, owner: OperationOwner) -> Result<(), OperationError> {
        if self.pending.len() >= self.max_pending {
            return Err(OperationError::LimitExceeded);
        }
        if self.pending.contains_key(&id) {
            return Err(OperationError::Duplicate(id));
        }
        self.pending.insert(
            id,
            PendingOperation {
                owner,
                headers_accepted: false,
            },
        );
        Ok(())
    }

    pub fn accept(
        &mut self,
        id: OperationId,
        event: &OperationEvent,
    ) -> Result<OperationOwner, OperationError> {
        let operation = self
            .pending
            .get_mut(&id)
            .ok_or(OperationError::Unmatched(id))?;
        match event {
            OperationEvent::Headers { .. } if operation.headers_accepted => {
                return Err(OperationError::DuplicateHeaders(id));
            }
            OperationEvent::Headers { .. } => operation.headers_accepted = true,
            OperationEvent::Chunk(_) if !operation.headers_accepted => {
                return Err(OperationError::BodyBeforeHeaders(id));
            }
            OperationEvent::Complete | OperationEvent::Failed(_) | OperationEvent::Cancelled => {
                return Ok(self.pending.remove(&id).expect("looked up operation").owner);
            }
            OperationEvent::Chunk(_) => {}
        }
        Ok(operation.owner)
    }

    pub fn cancel_document(
        &mut self,
        realm: RealmId,
        document_generation: u64,
    ) -> Vec<OperationId> {
        let ids = self
            .pending
            .iter()
            .filter(|(_, operation)| {
                operation.owner.realm == realm
                    && operation.owner.document_generation == document_generation
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in &ids {
            self.pending.remove(id);
        }
        ids
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(generation: u64) -> OperationOwner {
        OperationOwner {
            realm: RealmId(1),
            document_generation: generation,
            kind: OperationKind::Fetch,
        }
    }

    #[test]
    fn permits_32_concurrent_operations_and_rejects_the_33rd() {
        let mut registry = AsyncOperationRegistry::new(32);
        for id in 0..32 {
            registry.begin(OperationId(id), owner(1)).unwrap();
        }
        assert_eq!(
            registry.begin(OperationId(32), owner(1)),
            Err(OperationError::LimitExceeded)
        );
    }

    #[test]
    fn body_cannot_cross_the_policy_boundary_before_headers() {
        let mut registry = AsyncOperationRegistry::new(32);
        registry.begin(OperationId(1), owner(1)).unwrap();
        assert_eq!(
            registry.accept(OperationId(1), &OperationEvent::Chunk(vec![1])),
            Err(OperationError::BodyBeforeHeaders(OperationId(1)))
        );
        registry
            .accept(
                OperationId(1),
                &OperationEvent::Headers {
                    status: 200,
                    headers: vec![],
                },
            )
            .unwrap();
        registry
            .accept(OperationId(1), &OperationEvent::Chunk(vec![1]))
            .unwrap();
    }

    #[test]
    fn navigation_discards_late_completions_for_only_the_old_document() {
        let mut registry = AsyncOperationRegistry::new(32);
        registry.begin(OperationId(1), owner(3)).unwrap();
        registry.begin(OperationId(2), owner(4)).unwrap();
        assert_eq!(registry.cancel_document(RealmId(1), 3), [OperationId(1)]);
        assert_eq!(
            registry.accept(OperationId(1), &OperationEvent::Complete),
            Err(OperationError::Unmatched(OperationId(1)))
        );
        assert_eq!(registry.len(), 1);
    }
}
