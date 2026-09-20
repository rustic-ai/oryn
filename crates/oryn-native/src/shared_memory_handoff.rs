//! Native queue and IPC handoff leases. Sender release cannot reclaim bytes
//! between writing an event frame and the parent actually adopting that frame.
use super::*;
use oryn_js::{
    message_wire::Principal,
    shared_memory::{SharedMemoryHandoff, SharedMemoryRecipient, SharedMemoryReference},
};
#[derive(Clone, PartialEq, Eq)]
enum Location {
    Source { owner: AgentOwner, committed: bool },
    Queued,
    Delivered(SharedMemoryRecipient),
}
pub(super) struct Handoff {
    pub(super) descriptor: SharedMemoryDescriptor,
    location: Location,
}
fn recipient(owner: &AgentOwner) -> SharedMemoryRecipient {
    match owner {
        AgentOwner::Window {
            page,
            target,
            generation,
            epoch,
        } => SharedMemoryRecipient::Window {
            page: page.0,
            target: target.clone(),
            document_generation: *generation,
            stream_epoch: *epoch,
        },
        AgentOwner::Agent(id) => SharedMemoryRecipient::Agent(id.0),
    }
}
fn principal(owner: &AgentOwner) -> Principal {
    match owner {
        AgentOwner::Window { page, .. } => Principal::Window(page.0),
        AgentOwner::Agent(id) => Principal::Agent(id.0),
    }
}
impl SharedMemoryRegistry {
    #[cfg(unix)]
    pub(super) fn reap_backings(&mut self) {
        let unused = self
            .backings
            .iter()
            .filter(|(token, backing)| {
                backing.holders.is_empty()
                    && !self.waits.retains(**token)
                    && !self
                        .handoffs
                        .values()
                        .any(|handoff| handoff.descriptor.token == **token)
            })
            .map(|(token, _)| *token)
            .collect::<Vec<_>>();
        for token in unused {
            if let Some(backing) = self.backings.remove(&token) {
                self.bytes -= backing.descriptor.mapped_length;
            }
        }
        self.clusters.retain(|id, cluster| {
            !cluster.members.is_empty()
                || self.backings.values().any(|backing| backing.cluster == *id)
        });
    }
    fn handoff_capacity(&self, count: usize) -> Result<(), String> {
        if count > 4096usize.saturating_sub(self.handoffs.len()) {
            Err("shared message handoff capacity".into())
        } else {
            Ok(())
        }
    }
    fn handoff_ids(&self, count: usize) -> Result<Vec<SharedMemoryHandoff>, String> {
        self.handoff_capacity(count)?;
        let mut ids = BTreeSet::new();
        for _ in 0..count {
            let id = SharedMemoryHandoff(random()?);
            if self.handoffs.contains_key(&id) || !ids.insert(id) {
                return Err("shared message handoff collision".into());
            }
        }
        Ok(ids.into_iter().collect())
    }
    #[cfg(unix)]
    pub(crate) fn offer_shared(
        &mut self,
        owner: &AgentOwner,
        descriptor: SharedMemoryDescriptor,
    ) -> Result<SharedMemoryReference, String> {
        if self.backing(owner, descriptor.token)?.descriptor != descriptor {
            return Err("shared message descriptor mismatch".into());
        }
        let handoff = self.handoff_ids(1)?[0];
        self.handoffs.insert(
            handoff,
            Handoff {
                descriptor,
                location: Location::Source {
                    owner: owner.clone(),
                    committed: false,
                },
            },
        );
        Ok(SharedMemoryReference {
            descriptor,
            handoff,
        })
    }
    pub(crate) fn commit_shared(
        &mut self,
        owner: &AgentOwner,
        references: &[SharedMemoryReference],
    ) -> Result<(), String> {
        if references.len() > 1024
            || references
                .iter()
                .map(|reference| reference.handoff)
                .collect::<BTreeSet<_>>()
                .len()
                != references.len()
        {
            return Err("invalid shared offer batch".into());
        }
        for reference in references {
            let item = self
                .handoffs
                .get(&reference.handoff)
                .ok_or("shared offer retired")?;
            if item.descriptor != reference.descriptor
                || !matches!(&item.location,Location::Source{owner:source,..}if source==owner)
            {
                return Err("foreign shared offer".into());
            }
        }
        for reference in references {
            if let Some(Handoff {
                location: Location::Source { committed, .. },
                ..
            }) = self.handoffs.get_mut(&reference.handoff)
            {
                *committed = true;
            }
        }
        Ok(())
    }
    /// Parent-only adoption consumes offered capabilities. A single native
    /// queue retention object owns the returned references across fanout clones.
    pub(crate) fn adopt_shared(
        &mut self,
        source: Principal,
        references: &[SharedMemoryReference],
    ) -> Result<Vec<SharedMemoryReference>, String> {
        let mut unique = BTreeSet::new();
        for reference in references {
            let item = self
                .handoffs
                .get(&reference.handoff)
                .ok_or("shared offer retired")?;
            if !unique.insert(reference.handoff)
                || item.descriptor != reference.descriptor
                || !matches!(&item.location,Location::Source{owner,committed:true}if principal(owner)==source)
            {
                return Err("foreign or uncommitted shared offer".into());
            }
        }
        let ids = self.handoff_ids(references.len())?;
        let mut queued = Vec::with_capacity(ids.len());
        for (reference, id) in references.iter().zip(ids) {
            self.handoffs.remove(&reference.handoff);
            self.handoffs.insert(
                id,
                Handoff {
                    descriptor: reference.descriptor,
                    location: Location::Queued,
                },
            );
            queued.push(SharedMemoryReference {
                descriptor: reference.descriptor,
                handoff: id,
            });
        }
        Ok(queued)
    }
    /// Each actual delivery gets its own claim, including BroadcastChannel
    /// fanout. Claims retain the backing through serialization and socket reads.
    pub(crate) fn deliver_shared(
        &mut self,
        target: SharedMemoryRecipient,
        queued: &[SharedMemoryReference],
    ) -> Result<Vec<SharedMemoryReference>, String> {
        for reference in queued {
            let item = self
                .handoffs
                .get(&reference.handoff)
                .ok_or("shared queue lease retired")?;
            if item.descriptor != reference.descriptor || item.location != Location::Queued {
                return Err("invalid shared queue lease".into());
            }
        }
        let ids = self.handoff_ids(queued.len())?;
        let mut deliveries = Vec::with_capacity(ids.len());
        for (reference, id) in queued.iter().zip(ids) {
            self.handoffs.insert(
                id,
                Handoff {
                    descriptor: reference.descriptor,
                    location: Location::Delivered(target.clone()),
                },
            );
            deliveries.push(SharedMemoryReference {
                descriptor: reference.descriptor,
                handoff: id,
            });
        }
        Ok(deliveries)
    }
    #[cfg(unix)]
    pub(crate) fn open_shared_delivery(
        &mut self,
        owner: &AgentOwner,
        reference: &SharedMemoryReference,
    ) -> Result<std::os::fd::OwnedFd, String> {
        let item = self
            .handoffs
            .get(&reference.handoff)
            .ok_or("shared delivery retired")?;
        if item.descriptor != reference.descriptor
            || item.location != Location::Delivered(recipient(owner))
        {
            return Err("foreign shared delivery".into());
        }
        // open enforces the recipient's exact document/agent cluster, not only
        // its page-level transport principal. Consume only after successful map.
        let fd = self.open(owner, reference.descriptor)?;
        self.handoffs.remove(&reference.handoff);
        Ok(fd)
    }
    #[cfg(unix)]
    pub(crate) fn release_shared_delivery(
        &mut self,
        owner: &AgentOwner,
        reference: &SharedMemoryReference,
    ) -> Result<(), String> {
        if let Some(item) = self.handoffs.get(&reference.handoff)
            && (item.descriptor != reference.descriptor
                || item.location != Location::Delivered(recipient(owner)))
        {
            return Err("foreign shared delivery release".into());
        }
        self.handoffs.remove(&reference.handoff);
        self.reap_backings();
        Ok(())
    }
    #[cfg(unix)]
    pub(crate) fn discard_shared_delivered(
        &mut self,
        target: Principal,
        references: &[SharedMemoryReference],
    ) -> Result<(), String> {
        for reference in references {
            if let Some(item) = self.handoffs.get(&reference.handoff)
                && (item.descriptor != reference.descriptor
                    || !matches!(&item.location,Location::Delivered(recipient)if recipient.principal()==target))
            {
                return Err("foreign shared delivery discard".into());
            }
        }
        for reference in references {
            self.handoffs.remove(&reference.handoff);
        }
        self.reap_backings();
        Ok(())
    }
    #[cfg(unix)]
    pub(crate) fn release_shared_queue(
        &mut self,
        queued: &[SharedMemoryReference],
    ) -> Result<(), String> {
        for reference in queued {
            if let Some(item) = self.handoffs.get(&reference.handoff)
                && (item.descriptor != reference.descriptor || item.location != Location::Queued)
            {
                return Err("foreign shared queue release".into());
            }
        }
        for reference in queued {
            self.handoffs.remove(&reference.handoff);
        }
        self.reap_backings();
        Ok(())
    }
    /// Called on native serialization/write failure, never as a GC inference
    /// that an already committed IPC event was not delivered.
    #[cfg(unix)]
    pub(crate) fn abort_shared_offer(
        &mut self,
        owner: &AgentOwner,
        reference: &SharedMemoryReference,
    ) -> Result<(), String> {
        if let Some(item) = self.handoffs.get(&reference.handoff)
            && (item.descriptor != reference.descriptor
                || !matches!(&item.location,Location::Source{owner:source,..}if source==owner))
        {
            return Err("foreign shared offer abort".into());
        }
        self.handoffs.remove(&reference.handoff);
        self.reap_backings();
        Ok(())
    }
    #[cfg(unix)]
    pub(crate) fn discard_shared_offered(
        &mut self,
        source: Principal,
        references: &[SharedMemoryReference],
    ) -> Result<(), String> {
        for reference in references {
            if let Some(item) = self.handoffs.get(&reference.handoff)
                && (item.descriptor != reference.descriptor
                    || !matches!(&item.location,Location::Source{owner,..}if principal(owner)==source))
            {
                return Err("foreign shared offer discard".into());
            }
        }
        for reference in references {
            self.handoffs.remove(&reference.handoff);
        }
        self.reap_backings();
        Ok(())
    }
    pub(super) fn retire_uncommitted_handoffs(&mut self, owner: &AgentOwner) {
        self.handoffs.retain(|_, item| {
            !matches!(&item.location,Location::Source{owner:source,committed:false}if source==owner)
                && item.location != Location::Delivered(recipient(owner))
        });
    }
    /// Call after the native principal's ordered incoming messages have drained.
    /// Queued messages already adopted for a surviving peer remain retained.
    #[cfg(unix)]
    pub(crate) fn retire_shared_principal(&mut self, source: Principal) {
        let owners = self
            .members
            .keys()
            .chain(
                self.backings
                    .values()
                    .flat_map(|backing| backing.holders.keys()),
            )
            .filter(|owner| principal(owner) == source)
            .cloned()
            .collect::<BTreeSet<_>>();
        for owner in owners {
            self.retire(&owner);
        }
        self.handoffs.retain(|_, item| match &item.location {
            Location::Source { owner, .. } => principal(owner) != source,
            Location::Delivered(target) => target.principal() != source,
            Location::Queued => true,
        });
        for backing in self.backings.values_mut() {
            backing
                .holders
                .retain(|owner, _| principal(owner) != source);
        }
        self.reap_backings();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use oryn_js::worker_wire::AgentId;
    #[test]
    fn committed_event_survives_sender_release_and_retirement_until_native_adoption() {
        let owner = AgentOwner::Agent(AgentId(1));
        let peer = AgentOwner::Agent(AgentId(2));
        let mut registry = SharedMemoryRegistry::default();
        registry.admit_root(owner.clone(), "origin".into()).unwrap();
        registry
            .admit_child(&owner, peer.clone(), "origin")
            .unwrap();
        let (descriptor, _) = registry
            .allocate(crate::shared_memory::SharedAllocationInputs {
                owner: &owner,
                byte_length: 4,
                maximum_length: None,
                granularity: 4096,
                guard_length: 0,
                wasm: false,
                memory64: false,
            })
            .unwrap();
        let offer = registry.offer_shared(&owner, descriptor).unwrap();
        registry
            .commit_shared(&owner, std::slice::from_ref(&offer))
            .unwrap();
        registry.release(&owner, descriptor.token).unwrap();
        registry.retire(&owner);
        assert!(registry.backings.contains_key(&descriptor.token));
        let queued = registry.adopt_shared(principal(&owner), &[offer]).unwrap();
        registry.retire_shared_principal(principal(&owner));
        let delivery = registry.deliver_shared(recipient(&peer), &queued).unwrap();
        registry.release_shared_queue(&queued).unwrap();
        assert!(registry.backings.contains_key(&descriptor.token));
        let _fd = registry.open_shared_delivery(&peer, &delivery[0]).unwrap();
        assert!(registry.open_shared_delivery(&peer, &delivery[0]).is_err());
        registry.release(&peer, descriptor.token).unwrap();
        assert!(registry.backings.is_empty());
    }
    #[test]
    fn broadcast_delivery_claims_are_independent_and_foreign_cluster_cannot_map() {
        let owner = AgentOwner::Agent(AgentId(1));
        let peer = AgentOwner::Agent(AgentId(2));
        let foreign = AgentOwner::Agent(AgentId(3));
        let mut registry = SharedMemoryRegistry::default();
        registry.admit_root(owner.clone(), "origin".into()).unwrap();
        registry
            .admit_child(&owner, peer.clone(), "origin")
            .unwrap();
        registry
            .admit_root(foreign.clone(), "origin".into())
            .unwrap();
        let (descriptor, _) = registry
            .allocate(crate::shared_memory::SharedAllocationInputs {
                owner: &owner,
                byte_length: 4,
                maximum_length: None,
                granularity: 4096,
                guard_length: 0,
                wasm: false,
                memory64: false,
            })
            .unwrap();
        let offer = registry.offer_shared(&owner, descriptor).unwrap();
        assert!(
            registry
                .adopt_shared(principal(&owner), std::slice::from_ref(&offer))
                .is_err()
        );
        registry
            .commit_shared(&owner, std::slice::from_ref(&offer))
            .unwrap();
        assert!(
            registry
                .adopt_shared(principal(&foreign), std::slice::from_ref(&offer))
                .is_err()
        );
        let queued = registry.adopt_shared(principal(&owner), &[offer]).unwrap();
        let first = registry.deliver_shared(recipient(&peer), &queued).unwrap();
        let second = registry.deliver_shared(recipient(&peer), &queued).unwrap();
        assert_ne!(first[0].handoff, second[0].handoff);
        let invalid = registry
            .deliver_shared(recipient(&foreign), &queued)
            .unwrap();
        assert!(
            registry
                .open_shared_delivery(&foreign, &invalid[0])
                .is_err()
        );
        registry
            .release_shared_delivery(&foreign, &invalid[0])
            .unwrap();
        registry.release_shared_delivery(&peer, &first[0]).unwrap();
        assert!(registry.open_shared_delivery(&peer, &second[0]).is_ok());
        registry.release_shared_queue(&queued).unwrap();
        registry.retire(&owner);
        registry.retire(&peer);
        registry.retire_shared_principal(principal(&owner));
        registry.retire_shared_principal(principal(&peer));
        assert!(registry.backings.is_empty());
    }
    #[test]
    fn failed_serialization_and_undelivered_principal_cleanup_release_all_handoffs() {
        let owner = AgentOwner::Agent(AgentId(1));
        let mut registry = SharedMemoryRegistry::default();
        registry.admit_root(owner.clone(), "origin".into()).unwrap();
        let (descriptor, _) = registry
            .allocate(crate::shared_memory::SharedAllocationInputs {
                owner: &owner,
                byte_length: 4,
                maximum_length: None,
                granularity: 4096,
                guard_length: 0,
                wasm: false,
                memory64: false,
            })
            .unwrap();
        let offer = registry.offer_shared(&owner, descriptor).unwrap();
        registry.abort_shared_offer(&owner, &offer).unwrap();
        assert!(registry.handoffs.is_empty());
        let offer = registry.offer_shared(&owner, descriptor).unwrap();
        registry.commit_shared(&owner, &[offer]).unwrap();
        registry.retire(&owner);
        assert!(!registry.backings.is_empty());
        registry.retire_shared_principal(principal(&owner));
        assert!(registry.handoffs.is_empty());
        assert!(registry.backings.is_empty());
    }
}
