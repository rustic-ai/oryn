// Included within native_import_graph; original parsed FontFace Arc only.
pub struct NativeConnectedFontReadContext {
    pub rule: NativeRule,
    original: Arc<Locked<style::font_face::FontFaceRule>>,
    epoch: LeaseArc<std::sync::atomic::AtomicU64>,
    expected: u64,
    leases: LeaseArc<Mutex<FontLeaseStore>>,
}
impl ImportGraph {
    /// Capture while readonly graph borrow is held; no allocation callbacks.
    /// Invoke owned context reads only after releasing that outer borrow.
    pub fn connected_font_context(
        &self,
        rule: &NativeRule,
    ) -> Result<NativeConnectedFontReadContext, String> {
        Ok(NativeConnectedFontReadContext {
            rule: rule.clone(),
            original: self.font_face_rule(rule)?,
            epoch: self.font_revision.clone(),
            expected: self.font_revision(),
            leases: self.font_descriptor_leases.clone(),
        })
    }
}
impl NativeConnectedFontReadContext {
    pub fn identity(&self) -> usize {
        (&*self.original as *const _) as usize
    }
    fn current(&self) -> Result<(), String> {
        if self.epoch.load(std::sync::atomic::Ordering::Acquire) != self.expected {
            return Err("connected font descriptor read stale".into());
        }
        Ok(())
    }
    pub fn descriptor(
        &self,
        name: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeConstructedDescriptor, String> {
        let id = style::font_face::DescriptorId::from_ident(name)
            .map_err(|_| "unknown FontFace descriptor")?;
        self.current()?;
        let bytes = {
            let shared_lock = self.rule.owner.shared_lock();
            let guard = shared_lock.read();
            let d = &self.original.read_with(&guard).descriptors;
            let mut count = FontDescriptorCount::default();
            d.get_with_writer(id, &mut count)
                .map_err(|e| e.to_string())?;
            count.bytes
        };
        let allocation = admission.reserve(bytes)?;
        admission.checkpoint()?;
        let mut value = String::new();
        value.try_reserve_exact(bytes).map_err(|e| e.to_string())?;
        let shared_lock = self.rule.owner.shared_lock();
        let guard = shared_lock.read();
        self.current()?;
        self.original
            .read_with(&guard)
            .descriptors
            .get_with_writer(
                id,
                &mut FontBoundedWriter {
                    text: &mut value,
                    limit: bytes,
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(NativeConstructedDescriptor {
            value,
            _allocation: allocation,
        })
    }
    pub fn family(
        &self,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeConstructedDescriptor, String> {
        self.current()?;
        let bytes = {
            let shared_lock = self.rule.owner.shared_lock();
            let guard = shared_lock.read();
            self.original
                .read_with(&guard)
                .descriptors
                .font_family
                .as_ref()
                .map_or(0, |f| f.name.len())
        };
        let allocation = admission.reserve(bytes)?;
        admission.checkpoint()?;
        let mut value = String::new();
        value.try_reserve_exact(bytes).map_err(|e| e.to_string())?;
        let shared_lock = self.rule.owner.shared_lock();
        let guard = shared_lock.read();
        self.current()?;
        if let Some(f) = &self.original.read_with(&guard).descriptors.font_family {
            if f.name.len() > bytes {
                return Err("connected font family size stale".into());
            }
            value.push_str(f.name.as_ref());
        }
        Ok(NativeConstructedDescriptor {
            value,
            _allocation: allocation,
        })
    }
    pub fn axes(&self) -> Result<NativeFontAxes, String> {
        let shared_lock = self.rule.owner.shared_lock();
        let guard = shared_lock.read();
        self.current()?;
        font_descriptor_axes(&self.original.read_with(&guard).descriptors)
    }
    pub fn intersects_text(&self, text: &str) -> Result<bool, String> {
        let shared_lock = self.rule.owner.shared_lock();
        let guard = shared_lock.read();
        self.current()?;
        let d = &self.original.read_with(&guard).descriptors;
        Ok(text.chars().any(|ch| {
            d.unicode_range.as_ref().is_none_or(|ranges| {
                ranges
                    .iter()
                    .any(|r| r.start <= ch as u32 && ch as u32 <= r.end)
            })
        }))
    }
}
impl ImportCompletionContext {
    /// Owned graph context carries original rule/sheet/condition relationships.
    /// Native caller must revalidate publication mutation stamp after this read.
    pub fn connected_font_demand(
        &self,
        rule: &NativeRule,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeConnectedFontDemand, String> {
        let demands = self.font_demands(admission)?;
        let index = demands
            .faces
            .iter()
            .position(|face| face.rule.same_rule(rule))
            .ok_or("connected FontFace no longer belongs to graph")?;
        Ok(NativeConnectedFontDemand {
            snapshot: demands,
            index,
        })
    }
}
pub struct NativeConnectedFontDemand {
    snapshot: NativeFontDemandSnapshot,
    index: usize,
}
impl NativeConnectedFontDemand {
    pub fn face(&self) -> &NativeFontDemand {
        &self.snapshot.faces[self.index]
    }
    pub fn into_snapshot(self) -> NativeFontDemandSnapshot {
        self.snapshot
    }
}
#[cfg(test)]
mod connected_font_controls {
    use super::*;
    struct Admission;
    impl ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
            Ok(LeaseArc::new(()))
        }
    }
    #[test]
    fn connected_context_reads_original_descriptors_and_rejects_changed_epoch() {
        let mut graph = ImportGraph::parse(
            "@font-face{font-family:Original;src:url(face.woff2);unicode-range:U+41-5A}",
            "https://example.test/",
            false,
            8,
            SharedRwLock::new(),
            &Admission,
        )
        .unwrap();
        let rule = graph.rule_at(&graph.root(), &[], 0, &Admission).unwrap();
        let read = graph.connected_font_context(&rule).unwrap();
        assert_eq!(read.family(&Admission).unwrap().value, "Original");
        assert!(read.intersects_text("A").unwrap());
        assert!(!read.intersects_text("a").unwrap());
        assert!(
            read.descriptor("src", &Admission)
                .unwrap()
                .value
                .contains("face.woff2")
        );
        graph
            .set_rule_property(&rule, "font-family", "Revised", "", &Admission)
            .unwrap();
        assert!(read.family(&Admission).is_err());
        assert_eq!(
            graph
                .connected_font_context(&rule)
                .unwrap()
                .family(&Admission)
                .unwrap()
                .value,
            "Revised"
        );
    }
    #[test]
    fn connected_prepared_edits_keep_graph_owned_descriptor_slots() {
        let mut graph = ImportGraph::parse(
            "@font-face{font-family:Original;src:local(Original)}",
            "https://example.test/",
            false,
            8,
            SharedRwLock::new(),
            &Admission,
        )
        .unwrap();
        let rule = graph.rule_at(&graph.root(), &[], 0, &Admission).unwrap();
        let context = graph.connected_font_context(&rule).unwrap();
        let edit = context
            .prepare_descriptor("font-weight", "700", &Admission)
            .unwrap();
        let receipt = match graph.commit_connected_font_descriptor(edit) {
            Ok(receipt) => receipt,
            Err(_) => panic!("fresh edit refused"),
        };
        assert!(receipt.changed);
        drop(receipt);
        drop(context);
        assert_eq!(
            graph.font_descriptor_leases.lock().unwrap().entries.len(),
            1
        );
        let context = graph.connected_font_context(&rule).unwrap();
        assert_eq!(
            context.descriptor("font-weight", &Admission).unwrap().value,
            "700"
        );
        let stale = context
            .prepare_descriptor("font-weight", "800", &Admission)
            .unwrap();
        let edit = context
            .prepare_descriptor("font-weight", "900", &Admission)
            .unwrap();
        let receipt = match graph.commit_connected_font_descriptor(edit) {
            Ok(receipt) => receipt,
            Err(_) => panic!("fresh edit refused"),
        };
        drop(receipt);
        assert!(graph.commit_connected_font_descriptor(stale).is_err());
        assert_eq!(
            graph.font_descriptor_leases.lock().unwrap().entries.len(),
            1
        );
    }
}

struct FontLeaseSlots {
    original: Arc<Locked<style::font_face::FontFaceRule>>,
    slots: [Option<LeaseArc<dyn Send + Sync>>; style::font_face::DescriptorId::COUNT],
}
#[derive(Default)]
struct FontLeaseStore {
    entries: Vec<FontLeaseSlots>,
    allocation: Option<LeaseArc<dyn Send + Sync>>,
}
pub struct PreparedConnectedFontDescriptor {
    rule: NativeRule,
    original: Arc<Locked<style::font_face::FontFaceRule>>,
    expected: u64,
    next: u64,
    id: style::font_face::DescriptorId,
    pub descriptor_index: usize,
    staged: style::font_face::Descriptors,
    allocation: LeaseArc<dyn Send + Sync>,
    lease_count: usize,
    replacement: Vec<FontLeaseSlots>,
    replacement_allocation: LeaseArc<dyn Send + Sync>,
}
pub struct NativeConnectedFontCommit {
    pub changed: bool,
    pub descriptor_index: usize,
    pub allocation: Option<LeaseArc<dyn Send + Sync>>,
    obsolete: Option<style::font_face::Descriptors>,
    retired_allocation: Option<LeaseArc<dyn Send + Sync>>,
    retired_buffer: Vec<FontLeaseSlots>,
    spare_allocation: Option<LeaseArc<dyn Send + Sync>>,
}
impl NativeConnectedFontCommit {
    /// Native record sidecar, not ephemeral JS wrapper, owns each current slot.
    /// Transfer the returned old slot here before dropping outer borrow.
    pub fn retire_allocation(&mut self, allocation: Option<LeaseArc<dyn Send + Sync>>) {
        self.retired_allocation = allocation;
    }
}
impl Drop for NativeConnectedFontCommit {
    fn drop(&mut self) {
        drop(self.obsolete.take());
        drop(self.retired_allocation.take());
        drop(std::mem::take(&mut self.retired_buffer));
        drop(self.spare_allocation.take());
        drop(self.allocation.take());
    }
}
impl NativeConnectedFontReadContext {
    pub fn prepare_descriptor(
        &self,
        name: &str,
        value: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<PreparedConnectedFontDescriptor, String> {
        use style::font_face::{DescriptorId, Descriptors};
        self.current()?;
        let id = DescriptorId::from_ident(name).map_err(|_| "unknown FontFace descriptor")?;
        let descriptor_index = (0..DescriptorId::COUNT)
            .find(|&i| DescriptorId::at(i) == Some(id))
            .ok_or("descriptor index absent")?;
        let next = self
            .expected
            .checked_add(1)
            .ok_or("font descriptor revision exhausted")?;
        let base = {
            let shared_lock = self.rule.owner.shared_lock();
            let guard = shared_lock.read();
            self.rule.owner.sheet.contents(&guard).url_data.clone()
        };
        let allocation = admission.reserve(
            value
                .len()
                .checked_add(std::mem::size_of::<Descriptors>())
                .ok_or("connected descriptor overflow")?,
        )?;
        let context = font_constructed_context(&base);
        let mut staged = Descriptors::default();
        let mut input = cssparser::ParserInput::new(value);
        staged
            .set(id, &context, &mut cssparser::Parser::new(&mut input))
            .map_err(|_| "invalid FontFace descriptor".to_owned())?;
        admission.checkpoint()?;
        self.current()?;
        let lease_count = self
            .leases
            .lock()
            .map_err(|_| "font lease store poisoned")?
            .entries
            .len();
        let capacity = lease_count
            .checked_add(1)
            .ok_or("font lease count overflow")?;
        let replacement_allocation = admission.reserve(
            capacity
                .checked_mul(std::mem::size_of::<FontLeaseSlots>())
                .ok_or("font lease metadata overflow")?,
        )?;
        let mut replacement = Vec::new();
        replacement
            .try_reserve_exact(capacity)
            .map_err(|e| e.to_string())?;
        if replacement.capacity() > capacity {
            return Err("font lease allocator capacity exceeds admitted size".into());
        }
        admission.checkpoint()?;
        self.current()?;
        Ok(PreparedConnectedFontDescriptor {
            rule: self.rule.clone(),
            original: self.original.clone(),
            expected: self.expected,
            next,
            id,
            descriptor_index,
            staged,
            allocation,
            lease_count,
            replacement,
            replacement_allocation,
        })
    }
}
impl ImportGraph {
    pub fn commit_connected_font_descriptor(
        &mut self,
        edit: PreparedConnectedFontDescriptor,
    ) -> Result<NativeConnectedFontCommit, PreparedConnectedFontDescriptor> {
        if !LeaseArc::ptr_eq(&self.leases, &edit.rule.owner.leases)
            || self.font_revision() != edit.expected
        {
            return Err(edit);
        }
        let CssRule::FontFace(original) = &edit.rule.rule else {
            return Err(edit);
        };
        if !Arc::ptr_eq(original, &edit.original) {
            return Err(edit);
        }
        let lease_store = self.font_descriptor_leases.clone();
        let mut leases = match lease_store.lock() {
            Ok(leases) => leases,
            Err(_) => return Err(edit),
        };
        if leases.entries.len() != edit.lease_count {
            return Err(edit);
        }
        let PreparedConnectedFontDescriptor {
            rule: _,
            original,
            expected: _,
            next,
            id,
            descriptor_index,
            mut staged,
            allocation,
            lease_count: _,
            mut replacement,
            replacement_allocation,
        } = edit;
        let mut guard = self.lock.write();
        let descriptors = &mut original.write_with(&mut guard).descriptors;
        if descriptors.same_one(id, &staged) {
            drop(guard);
            return Ok(NativeConnectedFontCommit {
                changed: false,
                descriptor_index,
                allocation: Some(allocation),
                obsolete: Some(staged),
                retired_allocation: None,
                retired_buffer: replacement,
                spare_allocation: Some(replacement_allocation),
            });
        }
        let mut old_metadata = None;
        let slot = if let Some(index) = leases
            .entries
            .iter()
            .position(|entry| Arc::ptr_eq(&entry.original, &original))
        {
            index
        } else {
            replacement.extend(leases.entries.drain(..));
            replacement.push(FontLeaseSlots {
                original: original.clone(),
                slots: std::array::from_fn(|_| None),
            });
            std::mem::swap(&mut leases.entries, &mut replacement);
            old_metadata = leases.allocation.replace(replacement_allocation.clone());
            leases.entries.len() - 1
        };
        let old_lease = leases.entries[slot].slots[descriptor_index].replace(allocation);
        descriptors.swap_one(id, &mut staged);
        self.font_revision
            .store(next, std::sync::atomic::Ordering::Release);
        drop(guard);
        Ok(NativeConnectedFontCommit {
            changed: true,
            descriptor_index,
            allocation: old_metadata,
            obsolete: Some(staged),
            retired_allocation: old_lease,
            retired_buffer: replacement,
            spare_allocation: Some(replacement_allocation),
        })
    }
}

pub struct PreparedFontLeasePrune {
    count: usize,
    consumed: bool,
    retired: Vec<FontLeaseSlots>,
    kept: Vec<FontLeaseSlots>,
    old_buffer: Vec<FontLeaseSlots>,
    allocation: Option<LeaseArc<dyn Send + Sync>>,
    old_allocation: Option<LeaseArc<dyn Send + Sync>>,
}
pub struct FontLeasePruneContext {
    leases: LeaseArc<Mutex<FontLeaseStore>>,
}
impl ImportGraph {
    pub fn capture_font_lease_prune_context(&self) -> FontLeasePruneContext {
        FontLeasePruneContext {
            leases: self.font_descriptor_leases.clone(),
        }
    }
}
impl FontLeasePruneContext {
    pub fn prepare(
        &self,
        admission: &dyn ImportAdmission,
    ) -> Result<PreparedFontLeasePrune, String> {
        let count = self
            .leases
            .lock()
            .map_err(|_| "font lease store poisoned")?
            .entries
            .len();
        let bytes = count
            .checked_mul(std::mem::size_of::<FontLeaseSlots>())
            .and_then(|n| n.checked_mul(2))
            .ok_or("font prune capacity overflow")?;
        let allocation = admission.reserve(bytes)?;
        let mut retired = Vec::new();
        retired
            .try_reserve_exact(count)
            .map_err(|e| e.to_string())?;
        if retired.capacity() > count {
            return Err("font prune allocator capacity exceeds admitted size".into());
        }
        let mut kept = Vec::new();
        kept.try_reserve_exact(count).map_err(|e| e.to_string())?;
        if kept.capacity() > count {
            return Err("font prune allocator capacity exceeds admitted size".into());
        }
        admission.checkpoint()?;
        Ok(PreparedFontLeasePrune {
            count,
            consumed: false,
            retired,
            kept,
            old_buffer: Vec::new(),
            allocation: Some(allocation),
            old_allocation: None,
        })
    }
}
impl ImportGraph {
    /// Servo Arc has no Weak; uniqueness proves only this sidecar still owns
    /// the original rule. Returned custody must drop after caller graph borrow.
    pub fn prune_font_leases(
        &mut self,
        prepared: &mut PreparedFontLeasePrune,
    ) -> Result<bool, String> {
        let mut store = self
            .font_descriptor_leases
            .lock()
            .map_err(|_| "font lease store poisoned")?;
        if prepared.consumed || store.entries.len() != prepared.count {
            return Ok(false);
        }
        for entry in store.entries.drain(..) {
            if entry.original.is_unique() {
                prepared.retired.push(entry);
            } else {
                prepared.kept.push(entry);
            }
        }
        std::mem::swap(&mut store.entries, &mut prepared.kept);
        std::mem::swap(&mut prepared.old_buffer, &mut prepared.kept);
        prepared.old_allocation =
            std::mem::replace(&mut store.allocation, prepared.allocation.take());
        prepared.consumed = true;
        Ok(true)
    }
}

#[cfg(test)]
mod connected_font_prune_control {
    use super::*;
    use std::{
        cell::RefCell,
        rc::{Rc, Weak},
        sync::atomic::{AtomicUsize, Ordering},
    };
    thread_local! {
        static GRAPH:RefCell<Option<Weak<RefCell<ImportGraph>>>>=const {RefCell::new(None)};
    }
    struct Charge {
        bad: LeaseArc<AtomicUsize>,
        live: LeaseArc<AtomicUsize>,
    }
    impl Drop for Charge {
        fn drop(&mut self) {
            GRAPH.with(|graph| {
                if let Some(graph) = graph.borrow().as_ref().and_then(Weak::upgrade) {
                    if graph.try_borrow_mut().is_err() {
                        self.bad.fetch_add(1, Ordering::SeqCst);
                    }
                }
            });
            self.live.fetch_sub(1, Ordering::SeqCst);
        }
    }
    struct Admission {
        bad: LeaseArc<AtomicUsize>,
        live: LeaseArc<AtomicUsize>,
    }
    impl ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
            self.live.fetch_add(1, Ordering::SeqCst);
            Ok(LeaseArc::new(Charge {
                bad: self.bad.clone(),
                live: self.live.clone(),
            }))
        }
    }
    #[test]
    fn deleted_font_lease_prunes_after_last_native_handle_outside_borrow() {
        let bad = LeaseArc::new(AtomicUsize::new(0));
        let live = LeaseArc::new(AtomicUsize::new(0));
        let admission = Admission {
            bad: bad.clone(),
            live: live.clone(),
        };
        let graph = Rc::new(RefCell::new(
            ImportGraph::parse(
                "@font-face{font-family:Original;src:local(Original)}",
                "https://example.test/",
                false,
                8,
                SharedRwLock::new(),
                &admission,
            )
            .unwrap(),
        ));
        GRAPH.with(|slot| *slot.borrow_mut() = Some(Rc::downgrade(&graph)));
        let root = graph.borrow().root();
        // Existing rule acquisition invokes only this test admission's retain
        // operation; charge drops are checked after the mutation transaction.
        let rule = graph.borrow().rule_at(&root, &[], 0, &admission).unwrap();
        let context = graph.borrow().connected_font_context(&rule).unwrap();
        let edit = context
            .prepare_descriptor("font-weight", "700", &admission)
            .unwrap();
        let receipt = {
            let mut borrow = graph.borrow_mut();
            match borrow.commit_connected_font_descriptor(edit) {
                Ok(receipt) => receipt,
                Err(_) => panic!("fresh edit rejected"),
            }
        };
        drop(receipt);
        graph.borrow_mut().delete_rule(&root, 0).unwrap();
        let prune = graph.borrow().capture_font_lease_prune_context();
        let mut prepared = prune.prepare(&admission).unwrap();
        assert!(graph.borrow_mut().prune_font_leases(&mut prepared).unwrap());
        drop(prepared);
        assert_eq!(
            graph
                .borrow()
                .font_descriptor_leases
                .lock()
                .unwrap()
                .entries
                .len(),
            1,
            "retained rule/context must preserve payload"
        );
        drop(rule);
        drop(context);
        drop(prune);
        let prune = graph.borrow().capture_font_lease_prune_context();
        let mut prepared = prune.prepare(&admission).unwrap();
        assert!(graph.borrow_mut().prune_font_leases(&mut prepared).unwrap());
        assert_eq!(
            graph
                .borrow()
                .font_descriptor_leases
                .lock()
                .unwrap()
                .entries
                .len(),
            0
        );
        assert!(
            !graph.borrow_mut().prune_font_leases(&mut prepared).unwrap(),
            "consumed preparation cannot publish twice"
        );
        drop(prepared);
        assert_eq!(
            bad.load(Ordering::SeqCst),
            0,
            "lease destructor ran while outer graph RefCell borrowed"
        );
        GRAPH.with(|slot| *slot.borrow_mut() = None);
        drop(prune);
        drop(root);
        drop(graph);
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }
}
