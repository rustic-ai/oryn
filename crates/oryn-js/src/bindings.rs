//! Engine-neutral state shared by generated V8 DOM bindings.
//!
//! V8 objects carry only [`WrapperId`] values. Resolution revalidates the
//! realm, document generation, node generation, and interface brand before a
//! behavioral implementation can touch the Rust DOM.

use std::collections::{BTreeMap, VecDeque};

use oryn_dom::{ArenaId, DomError, DomRuntime, NodeId, NodeKind, NodeRef};
use serde::{Deserialize, Serialize};

use crate::realm::RealmId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WrapperId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceBrand {
    EventTarget,
    Node,
    Document,
    Element,
    HtmlElement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrapperMetadata {
    pub id: WrapperId,
    pub arena: ArenaId,
    pub node: NodeId,
    pub realm: RealmId,
    pub document_generation: u64,
    pub brand: InterfaceBrand,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BindingError {
    #[error("wrapper identity space exhausted")]
    IdentityExhausted,
    #[error("unknown or collected JavaScript wrapper {0:?}")]
    UnknownWrapper(WrapperId),
    #[error("wrapper belongs to realm {actual:?}, not {expected:?}")]
    WrongRealm { expected: RealmId, actual: RealmId },
    #[error("wrapper belongs to document generation {actual}, not {expected}")]
    StaleDocument { expected: u64, actual: u64 },
    #[error("wrapper has interface brand {actual:?}, not {expected:?}")]
    WrongBrand {
        expected: InterfaceBrand,
        actual: InterfaceBrand,
    },
    #[error(transparent)]
    Dom(#[from] DomError),
}

#[derive(Debug, Default)]
pub struct WrapperRegistry {
    next_id: u64,
    by_identity: BTreeMap<(ArenaId, RealmId, NodeId), WrapperId>,
    wrappers: BTreeMap<WrapperId, WrapperMetadata>,
    pending_release: VecDeque<WrapperId>,
}

impl WrapperRegistry {
    pub fn get_or_create(
        &mut self,
        dom: &mut DomRuntime,
        realm: RealmId,
        document_generation: u64,
        node: NodeId,
        brand: InterfaceBrand,
    ) -> Result<WrapperId, BindingError> {
        validate_node_brand(dom, node, brand)?;
        let identity = (dom.arena().id(), realm, node);
        if let Some(wrapper) = self.by_identity.get(&identity) {
            let wrapper = *wrapper;
            let metadata = self
                .wrappers
                .get(&wrapper)
                .ok_or(BindingError::UnknownWrapper(wrapper))?;
            if metadata.document_generation != document_generation {
                return Err(BindingError::StaleDocument {
                    expected: document_generation,
                    actual: metadata.document_generation,
                });
            }
            // A weak callback for an earlier V8 object may already be queued
            // when the same Rust node is wrapped again. The replacement is a
            // live lease and must cancel that stale release notification.
            self.cancel_weak_release(wrapper);
            return Ok(wrapper);
        }
        let next_id = self
            .next_id
            .checked_add(1)
            .ok_or(BindingError::IdentityExhausted)?;
        dom.retain_wrapper(node)?;
        let id = WrapperId(self.next_id);
        self.next_id = next_id;
        self.by_identity.insert(identity, id);
        self.wrappers.insert(
            id,
            WrapperMetadata {
                id,
                arena: dom.arena().id(),
                node,
                realm,
                document_generation,
                brand,
            },
        );
        Ok(id)
    }

    pub fn resolve(
        &self,
        dom: &DomRuntime,
        wrapper: WrapperId,
        expected_realm: RealmId,
        expected_document_generation: u64,
        expected_brand: InterfaceBrand,
    ) -> Result<NodeId, BindingError> {
        let metadata = self
            .wrappers
            .get(&wrapper)
            .ok_or(BindingError::UnknownWrapper(wrapper))?;
        if metadata.realm != expected_realm {
            return Err(BindingError::WrongRealm {
                expected: expected_realm,
                actual: metadata.realm,
            });
        }
        if metadata.document_generation != expected_document_generation {
            return Err(BindingError::StaleDocument {
                expected: expected_document_generation,
                actual: metadata.document_generation,
            });
        }
        if !brand_inherits(metadata.brand, expected_brand) {
            return Err(BindingError::WrongBrand {
                expected: expected_brand,
                actual: metadata.brand,
            });
        }
        dom.arena().resolve(NodeRef {
            arena: metadata.arena,
            node: metadata.node,
        })?;
        Ok(metadata.node)
    }

    /// Called from a V8 weak callback. The callback itself performs no DOM
    /// mutation; leases are released at the next safe scheduler checkpoint.
    pub fn enqueue_weak_release(&mut self, wrapper: WrapperId) {
        self.pending_release.push_back(wrapper);
    }

    pub fn cancel_weak_release(&mut self, wrapper: WrapperId) {
        self.pending_release.retain(|queued| *queued != wrapper);
    }

    #[cfg(feature = "v8-host")]
    pub(crate) fn cancel_live_weak_releases(
        &mut self,
        live: &std::collections::BTreeSet<WrapperId>,
    ) {
        self.pending_release.retain(|queued| !live.contains(queued));
    }

    pub fn process_weak_releases(&mut self, dom: &mut DomRuntime) -> Result<usize, BindingError> {
        let mut released = 0;
        let mut pending = std::mem::take(&mut self.pending_release);
        while let Some(id) = pending.pop_front() {
            let Some(metadata) = self.wrappers.get(&id).copied() else {
                continue;
            };
            if metadata.arena != dom.arena().id() {
                self.pending_release.push_back(id);
                continue;
            }
            if let Err(error) = dom.release_wrapper(metadata.node) {
                self.pending_release.push_back(id);
                self.pending_release.extend(pending);
                return Err(error.into());
            }
            self.wrappers.remove(&id);
            self.by_identity
                .remove(&(metadata.arena, metadata.realm, metadata.node));
            released += 1;
        }
        Ok(released)
    }

    #[cfg(feature = "v8-host")]
    pub(crate) fn lookup(&self, arena: ArenaId, realm: RealmId, node: NodeId) -> Option<WrapperId> {
        self.by_identity.get(&(arena, realm, node)).copied()
    }

    pub fn metadata(&self, wrapper: WrapperId) -> Option<WrapperMetadata> {
        self.wrappers.get(&wrapper).copied()
    }

    pub fn invalidate_document(&mut self, realm: RealmId, document_generation: u64) {
        self.pending_release.extend(
            self.wrappers
                .values()
                .filter(|metadata| {
                    metadata.realm == realm && metadata.document_generation == document_generation
                })
                .map(|metadata| metadata.id),
        );
    }

    pub fn release_all(&mut self, dom: &mut DomRuntime) -> Result<usize, BindingError> {
        self.pending_release.extend(self.wrappers.keys().copied());
        self.process_weak_releases(dom)
    }
}

fn validate_node_brand(
    dom: &DomRuntime,
    node: NodeId,
    brand: InterfaceBrand,
) -> Result<(), BindingError> {
    let kind = &dom.arena().get(node)?.kind;
    let valid = match brand {
        InterfaceBrand::EventTarget | InterfaceBrand::Node => true,
        InterfaceBrand::Document => matches!(kind, NodeKind::Document),
        InterfaceBrand::Element | InterfaceBrand::HtmlElement => {
            matches!(kind, NodeKind::Element { .. })
        }
    };
    if valid {
        Ok(())
    } else {
        Err(BindingError::WrongBrand {
            expected: brand,
            actual: InterfaceBrand::Node,
        })
    }
}

const fn brand_inherits(actual: InterfaceBrand, expected: InterfaceBrand) -> bool {
    actual as u8 >= expected as u8
        && match expected {
            InterfaceBrand::EventTarget | InterfaceBrand::Node => true,
            InterfaceBrand::Document => matches!(actual, InterfaceBrand::Document),
            InterfaceBrand::Element => {
                matches!(
                    actual,
                    InterfaceBrand::Element | InterfaceBrand::HtmlElement
                )
            }
            InterfaceBrand::HtmlElement => matches!(actual, InterfaceBrand::HtmlElement),
        }
}

#[cfg(test)]
mod tests {
    use markup5ever::{QualName, local_name, ns};

    use super::*;

    #[test]
    fn matching_local_node_ids_in_different_arenas_do_not_resolve() {
        let mut first = DomRuntime::new();
        let mut second = DomRuntime::new();
        let first_node = first.create_text("first");
        let second_node = second.create_text("second");
        assert_eq!(first_node, second_node, "local slots intentionally collide");
        let mut wrappers = WrapperRegistry::default();
        let wrapper = wrappers
            .get_or_create(&mut first, RealmId(0), 1, first_node, InterfaceBrand::Node)
            .unwrap();
        assert!(
            wrappers
                .resolve(&second, wrapper, RealmId(0), 1, InterfaceBrand::Node)
                .is_err(),
            "a wrapper must never resolve against an unrelated arena"
        );
        let other_wrapper = wrappers
            .get_or_create(
                &mut second,
                RealmId(0),
                1,
                second_node,
                InterfaceBrand::Node,
            )
            .unwrap();
        assert_ne!(wrapper, other_wrapper);
        wrappers.enqueue_weak_release(wrapper);
        assert_eq!(wrappers.process_weak_releases(&mut second).unwrap(), 0);
        assert!(wrappers.metadata(wrapper).is_some());
        assert_eq!(wrappers.process_weak_releases(&mut first).unwrap(), 1);
        assert!(
            wrappers
                .resolve(&second, other_wrapper, RealmId(0), 1, InterfaceBrand::Node)
                .is_ok()
        );
    }

    #[test]
    fn exhausted_wrapper_ids_do_not_overwrite_or_acquire_a_root() {
        let mut dom = DomRuntime::new();
        let node = dom.create_text("unrooted");
        let mut wrappers = WrapperRegistry {
            next_id: u64::MAX,
            ..Default::default()
        };
        assert_eq!(
            wrappers.get_or_create(&mut dom, RealmId(0), 1, node, InterfaceBrand::Node),
            Err(BindingError::IdentityExhausted)
        );
        assert!(wrappers.wrappers.is_empty());
        dom.remove(node).unwrap();
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        assert!(
            dom.arena().get(node).is_err(),
            "failed allocation must not retain a wrapper root"
        );
    }

    #[test]
    fn wrappers_preserve_identity_and_validate_realm_generation_and_brand() {
        let mut dom = DomRuntime::new();
        let node = dom.create(NodeKind::Element {
            name: QualName::new(None, ns!(html), local_name!("button")),
        });
        dom.append_child(dom.document(), node).unwrap();
        let mut wrappers = WrapperRegistry::default();
        let wrapper = wrappers
            .get_or_create(&mut dom, RealmId(2), 7, node, InterfaceBrand::HtmlElement)
            .unwrap();
        assert_eq!(
            wrapper,
            wrappers
                .get_or_create(&mut dom, RealmId(2), 7, node, InterfaceBrand::HtmlElement,)
                .unwrap()
        );
        assert_eq!(
            wrapper,
            wrappers
                .get_or_create(&mut dom, RealmId(2), 7, node, InterfaceBrand::Node)
                .unwrap(),
            "one node has one JavaScript identity per realm"
        );
        assert_eq!(
            wrappers.resolve(&dom, wrapper, RealmId(2), 7, InterfaceBrand::Node),
            Ok(node)
        );
        assert!(matches!(
            wrappers.resolve(&dom, wrapper, RealmId(3), 7, InterfaceBrand::Node),
            Err(BindingError::WrongRealm { .. })
        ));
        assert!(matches!(
            wrappers.resolve(&dom, wrapper, RealmId(2), 8, InterfaceBrand::Node),
            Err(BindingError::StaleDocument { .. })
        ));
        assert!(matches!(
            wrappers.resolve(&dom, wrapper, RealmId(2), 7, InterfaceBrand::Document),
            Err(BindingError::WrongBrand { .. })
        ));
    }

    #[test]
    fn weak_release_is_deferred_until_a_safe_checkpoint() {
        let mut dom = DomRuntime::new();
        let node = dom.create(NodeKind::Text { data: "x".into() });
        let mut wrappers = WrapperRegistry::default();
        let wrapper = wrappers
            .get_or_create(&mut dom, RealmId(0), 1, node, InterfaceBrand::Node)
            .unwrap();
        dom.remove(node).unwrap();
        assert!(dom.arena().get(node).is_ok());
        wrappers.enqueue_weak_release(wrapper);
        assert!(dom.arena().get(node).is_ok());
        assert_eq!(wrappers.process_weak_releases(&mut dom).unwrap(), 1);
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        assert_eq!(dom.arena().get(node), Err(DomError::StaleNode(node)));
    }

    #[cfg(feature = "v8-host")]
    #[test]
    fn bulk_cancellation_keeps_live_wrappers_and_releases_only_dead_ones() {
        let mut dom = DomRuntime::new();
        let mut wrappers = WrapperRegistry::default();
        let mut nodes = Vec::new();
        let mut live = std::collections::BTreeSet::new();
        for index in 0..32 {
            let node = dom.create_text("node");
            let wrapper = wrappers
                .get_or_create(&mut dom, RealmId(0), 1, node, InterfaceBrand::Node)
                .unwrap();
            wrappers.enqueue_weak_release(wrapper);
            if index % 2 == 0 {
                live.insert(wrapper);
            }
            nodes.push((node, wrapper));
        }
        wrappers.cancel_live_weak_releases(&live);
        assert_eq!(wrappers.process_weak_releases(&mut dom).unwrap(), 16);
        for (node, wrapper) in nodes {
            assert_eq!(dom.arena().get(node).is_ok(), live.contains(&wrapper));
        }
    }

    #[test]
    fn recreating_a_wrapper_cancels_an_older_weak_release() {
        let mut dom = DomRuntime::new();
        let node = dom.create(NodeKind::Text { data: "x".into() });
        let mut wrappers = WrapperRegistry::default();
        let wrapper = wrappers
            .get_or_create(&mut dom, RealmId(0), 1, node, InterfaceBrand::Node)
            .unwrap();
        wrappers.enqueue_weak_release(wrapper);
        assert_eq!(
            wrappers
                .get_or_create(&mut dom, RealmId(0), 1, node, InterfaceBrand::Node)
                .unwrap(),
            wrapper
        );
        assert_eq!(wrappers.process_weak_releases(&mut dom).unwrap(), 0);
        assert_eq!(
            wrappers.resolve(&dom, wrapper, RealmId(0), 1, InterfaceBrand::Node),
            Ok(node)
        );
    }
}
