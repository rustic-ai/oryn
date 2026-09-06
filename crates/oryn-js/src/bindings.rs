//! Engine-neutral state shared by generated V8 DOM bindings.
//!
//! V8 objects carry only [`WrapperId`] values. Resolution revalidates the
//! realm, document generation, node generation, and interface brand before a
//! behavioral implementation can touch the Rust DOM.

use std::collections::{BTreeMap, VecDeque};

use oryn_dom::{DomError, DomRuntime, NodeId, NodeKind};
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
    pub node: NodeId,
    pub realm: RealmId,
    pub document_generation: u64,
    pub brand: InterfaceBrand,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BindingError {
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
    by_identity: BTreeMap<(RealmId, NodeId), WrapperId>,
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
        let identity = (realm, node);
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
        dom.retain_wrapper(node)?;
        let id = WrapperId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.by_identity.insert(identity, id);
        self.wrappers.insert(
            id,
            WrapperMetadata {
                id,
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
        dom.arena().get(metadata.node)?;
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

    pub fn process_weak_releases(&mut self, dom: &mut DomRuntime) -> Result<usize, BindingError> {
        let mut released = 0;
        while let Some(id) = self.pending_release.pop_front() {
            let Some(metadata) = self.wrappers.remove(&id) else {
                continue;
            };
            self.by_identity.remove(&(metadata.realm, metadata.node));
            dom.release_wrapper(metadata.node)?;
            released += 1;
        }
        Ok(released)
    }

    #[cfg(feature = "v8-host")]
    pub(crate) fn lookup(&self, realm: RealmId, node: NodeId) -> Option<WrapperId> {
        self.by_identity.get(&(realm, node)).copied()
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
