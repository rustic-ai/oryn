//! Owned Rust continuations for native edits. No V8 scope or native borrow is
//! retained while a mutation waits for its children-changed author work.
type EditingFuture = Pin<Box<dyn Future<Output = Result<(), V8BindingError>>>>;

type RetainedRanges = Rc<RefCell<Option<BTreeMap<u64, RangeRecord>>>>;

use super::*;
use std::{
    future::Future,
    pin::Pin,
    task::{Context as PollContext, Poll, Waker},
};
struct RangeMove {
    id: u64,
    private_marker: bool,
    expected: RangeRecord,
    start: Option<BoundaryPoint>,
    end: Option<BoundaryPoint>,
}
fn same_range(a: &RangeRecord, b: &RangeRecord) -> bool {
    a.start_container == b.start_container
        && a.start_offset == b.start_offset
        && a.end_container == b.end_container
        && a.end_offset == b.end_offset
}
enum Operation {
    RetainMetadata { bytes: usize },
    Layout,
    ClearSelection,
    Selection { anchor: oryn_dom::TrackedNodeLease, anchor_offset:u32, focus:oryn_dom::TrackedNodeLease, focus_offset:u32 },
    LegacyFont {
        keyword: BindingLegacyFontKeyword,
    },
    RemoveAttribute {
        node: oryn_dom::TrackedNodeLease,
        name: String,
    },
    Overrides {
        value: EditingOverrides,
    },
    Attribute {
        node: oryn_dom::TrackedNodeLease,
        namespace: Option<String>,
        name: String,
        value: String,
    },
    Style {
        node: oryn_dom::TrackedNodeLease,
        name: String,
        value: String,
    },
    Control {
        node: oryn_dom::TrackedNodeLease,
        start: u32,
        end: u32,
        value: String,
    },
    Override {
        key: String,
        value: String,
    },
    Split {
        node: oryn_dom::TrackedNodeLease,
        offset: u32,
        output: Rc<RefCell<Option<oryn_dom::TrackedNodeLease>>>,
    },
    Text {
        node: oryn_dom::TrackedNodeLease,
        offset: u32,
        count: u32,
        data: String,
    },
    SpaceRun { host: NodeId, selection: u64, revision: u64, point: BoundaryPoint },
    Remove {
        node: oryn_dom::TrackedNodeLease,
        ranges: Option<RetainedRanges>,
        private_ranges: Option<(Vec<u64>,RetainedRanges)>,
    },
    RemoveWithMarkers {
        node: oryn_dom::TrackedNodeLease,
        markers: Vec<oryn_dom::LiveRangeId>,
    },
    Append {
        parent: oryn_dom::TrackedNodeLease,
        node: oryn_dom::TrackedNodeLease,
        reference: Option<oryn_dom::TrackedNodeLease>,
        ranges: Vec<RangeMove>,
    },
}
enum Preparation {
    Parent(script_children_changed::Prepared),
    Insertion(post_connection_scripts::PreparedInsertion),
}
impl Preparation {
    fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        match self {
            Self::Parent(value) => value.poll_retained(scope),
            Self::Insertion(value) => value.poll_retained(scope),
        }
    }
}
struct PrepaidCapture {
    roots: Vec<oryn_dom::TrackedNodeLease>,
    bound: usize,
}
struct Request {
    operation: Operation,
    capture: RefCell<Option<PrepaidCapture>>,
    space_run: RefCell<Option<PreparedSpaceRun>>,
    prepared: RefCell<Option<Preparation>>,
    mutated: Cell<bool>,
    recorded: Cell<bool>,
    error: RefCell<Option<V8BindingError>>,
    reply: RefCell<Option<Result<(), V8BindingError>>>,
}
struct PreparedSpaceRun {
    host: oryn_dom::TrackedNodeLease,
    spaces: Vec<(oryn_dom::TrackedNodeLease,u32,u16)>,
    ends_line: bool,
    _allocation: Option<std::sync::Arc<dyn Send+Sync>>,
    _scan: EditingSpaceAllocation,
    mutation: oryn_dom::MutationCursor,
    boundary: u64,
}
pub(super) struct Marker {
    owner: Rc<V8BindingState>,
    id: oryn_dom::LiveRangeId,
    registry: Rc<RefCell<BTreeSet<u64>>>,
}
impl Marker {
    pub(super) fn range(&self) -> Result<RangeRecord, V8BindingError> {
        Ok(self.owner.dom.borrow().live_range(self.id)?.into())
    }
}
impl Drop for Marker {
    fn drop(&mut self) {
        self.registry.borrow_mut().remove(&self.id.0);
        let _ = self.owner.dom.borrow_mut().drop_live_range(self.id);
    }
}
pub(super) struct Context {
    pub state: Rc<V8BindingState>,
    clones: RefCell<Vec<Rc<RangeCloneCompletion>>>,
    pending: RefCell<Option<Rc<Request>>>,
    tracked: RefCell<BTreeMap<NodeId, oryn_dom::TrackedNodeLease>>,
    admitted: RefCell<BTreeSet<NodeId>>,
    extra_roots: RefCell<Vec<oryn_dom::TrackedNodeLease>>,
    legacy_font_result: Cell<Option<BindingLegacyFontResult>>,
    metadata_result: RefCell<Option<Option<std::sync::Arc<dyn Send + Sync>>>>,
    markers: Rc<RefCell<BTreeSet<u64>>>,
    paragraph_split: RefCell<Option<editing_inverse::paragraph_split::ParagraphSplit>>,
}
impl Drop for Context {
    fn drop(&mut self) {
        for job in self.clones.get_mut().drain(..) {
            job.background.set(true);
        }
    }
}
impl Context {
    fn snapshot_roots(&self,host:NodeId)->Result<BTreeSet<NodeId>,V8BindingError>{
        let mut roots=BTreeSet::from([host]);
        for lease in self.extra_roots.borrow().iter(){
            let (owner,node)=lease.current();
            if Rc::ptr_eq(&owner,&self.state.dom)&&owner.borrow().arena().node_document(node)?==Some(self.state.document_root()) {roots.insert(node);}
        }
        for (original,lease) in self.tracked.borrow().iter(){
            if !self.admitted.borrow().contains(original){continue}
            let (owner,node)=lease.current();
            if !Rc::ptr_eq(&owner,&self.state.dom){continue}
            let dom=owner.borrow();
            if dom.arena().node_document(node)?!=Some(self.state.document_root())||dom.is_connected(node)?{continue}
            roots.insert(dom.arena().ancestors(node)?.last().copied().unwrap_or(node));
        }
        Ok(roots)
    }

    pub(super) async fn clear_selection(&self)->Result<(),V8BindingError>{self.request(Operation::ClearSelection).await}
    pub(super) async fn restore_selection(&self,a:BoundaryPoint,f:BoundaryPoint)->Result<(),V8BindingError>{
        self.request(Operation::Selection {anchor:self.lease(a.container)?,anchor_offset:a.offset,focus:self.lease(f.container)?,focus_offset:f.offset}).await
    }
    pub(super) fn retain_paragraph_split(&self, left: NodeId, right: NodeId, placeholders: [Option<NodeId>;2]) -> Result<(), V8BindingError> {
        *self.paragraph_split.borrow_mut() = Some(editing_inverse::paragraph_split::ParagraphSplit::retain(&self.state,left,right,placeholders)?);
        Ok(())
    }
    pub(super) fn retain_clone(
        &self,
        node: NodeId,
        clone: NodeId,
        deep: bool,
    ) -> Result<(), V8BindingError> {
        let mut dom = self.state.dom.borrow_mut();
        let source = oryn_dom::TrackedNodeLease::new_in(&self.state.dom, &mut dom, node)?;
        let target = oryn_dom::TrackedNodeLease::new_in(&self.state.dom, &mut dom, clone)?;
        let job = Rc::new(RangeCloneCompletion {
            source,
            target,
            deep,
            background: Cell::new(false),
            stage: Cell::new(0),
        });
        self.clones.borrow_mut().push(job.clone());
        self.state.range_clone_completions.borrow_mut().push_back(job);
        Ok(())
    }

    pub(super) async fn append_with_endpoint_markers(
        &self,
        parent: NodeId,
        node: NodeId,
        markers: &[&Marker],
    ) -> Result<(), V8BindingError> {
        self.lease(parent)?;
        let lease = self.lease(node)?;
        let updates = {
            let dom = self.state.dom.borrow();
            let mut updates = Vec::new();
            updates
                .try_reserve_exact(markers.len())
                .map_err(|_| V8BindingError::Allocation("editing endpoint markers"))?;
            for marker in markers {
                let range = dom.live_range(marker.id)?;
                if range.start != range.end {
                    return Err(V8BindingError::Allocation("noncollapsed command endpoint"));
                }
                updates.push(marker.id);
            }
            updates
        };
        if self.state.dom.borrow().parent(node)?.is_some() {
            self.request(Operation::RemoveWithMarkers {
                node: lease,
                markers: updates,
            })
            .await?;
        }
        // The independent collapsed markers remain rooted in the removed
        // subtree before author callbacks. Text mutation and adoption then use
        // ordinary native journals; no JS/external Range is restored.
        self.append(parent, node, None).await
    }

    pub(super) fn marker(&self, range: RangeRecord) -> Result<Marker, V8BindingError> {
        let native = LiveRange::from(&range);
        let id = self
            .state
            .dom
            .borrow_mut()
            .create_live_range(native.start, native.end)?;
        self.markers.borrow_mut().insert(id.0);
        Ok(Marker {
            owner: self.state.clone(),
            id,
            registry: self.markers.clone(),
        })
    }
    pub(super) fn lease(&self, node: NodeId) -> Result<oryn_dom::TrackedNodeLease, V8BindingError> {
        if !self.tracked.borrow().contains_key(&node) {
            if self.tracked.borrow().len() >= 16384 {
                return Err(V8BindingError::Unsupported {
                    interface: "Document".into(),
                    member: "QuotaExceededError".into(),
                });
            }
            let lease = oryn_dom::TrackedNodeLease::new(&self.state.dom, node)?;
            self.tracked.borrow_mut().insert(node, lease);
        }
        // Inert Documents can share the arena: arena identity alone cannot
        // authorize an edit after author code adopts a retained node.
        let (owner, current) = self
            .tracked
            .borrow()
            .get(&node)
            .expect("retained editing node")
            .current();
        if !Rc::ptr_eq(&owner, &self.state.dom)
            || current != node
            || owner.borrow().arena().node_document(current)? != Some(self.state.document_root())
        {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "AbortError".into(),
            });
        }
        Ok(oryn_dom::TrackedNodeLease::new(&owner, current)?)
    }
    fn snapshot(&self, host: NodeId) -> Result<EditingSnapshot, V8BindingError> {
        self.snapshot_with_roots(host,&[])
    }
    fn snapshot_with_roots(&self,host:NodeId,extra:&[oryn_dom::TrackedNodeLease])->Result<EditingSnapshot,V8BindingError>{
        let mut roots=self.snapshot_roots(host)?;
        for lease in extra {let (owner,node)=lease.current();if !Rc::ptr_eq(&owner,&self.state.dom){return Err(V8BindingError::Allocation("paragraph prepaid root owner"))}roots.insert(node);}

        let mut snapshot = capture_editing_snapshot(&self.state, host)?;
        let mut seen = snapshot
            .nodes
            .iter()
            .map(|record| record.node)
            .collect::<BTreeSet<_>>();
        for root in roots {
            if root == host || seen.contains(&root) {
                continue;
            }
            let part = capture_editing_snapshot(&self.state, root)?;
            snapshot.serialized_bytes = snapshot
                .serialized_bytes
                .saturating_add(part.serialized_bytes);
            for record in part.nodes {
                if seen.insert(record.node) {
                    snapshot.nodes.push(record);
                }
            }
            for control in part.controls {
                if !snapshot
                    .controls
                    .iter()
                    .any(|existing| existing.node == control.node)
                {
                    snapshot.controls.push(control);
                }
            }
        }
        if snapshot.serialized_bytes > UndoHistory::MAX_BYTES / 2 {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "QuotaExceededError".into(),
            });
        }
        seen.extend(snapshot.selection.anchor.map(|point| point.container));
        seen.extend(snapshot.selection.focus.map(|point| point.container));
        let mut entries = Vec::new();
        for node in seen {
            let lease = oryn_dom::TrackedNodeLease::new(&self.state.dom, node)?;
            entries.push((lease.identity(), lease));
        }
        snapshot.leases = Rc::new(EditingSnapshotLeases {
            entries,
            released: Cell::new(false),
        });
        Ok(snapshot)
    }
    pub(super) async fn remove_attribute(
        &self,
        node: NodeId,
        name: impl Into<String>,
    ) -> Result<(), V8BindingError> {
        self.request(Operation::RemoveAttribute {
            node: self.lease(node)?,
            name: name.into(),
        })
        .await
    }
    pub(super) async fn overrides(&self, value: EditingOverrides) -> Result<(), V8BindingError> {
        self.request(Operation::Overrides { value }).await
    }
    pub(super) async fn attribute(
        &self,
        node: NodeId,
        namespace: Option<String>,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), V8BindingError> {
        self.request(Operation::Attribute {
            node: self.lease(node)?,
            namespace,
            name: name.into(),
            value: value.into(),
        })
        .await
    }
    pub(super) async fn style(
        &self,
        node: NodeId,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), V8BindingError> {
        self.request(Operation::Style {
            node: self.lease(node)?,
            name: name.into(),
            value: value.into(),
        })
        .await
    }
    pub(super) async fn control(
        &self,
        node: NodeId,
        start: u32,
        end: u32,
        value: impl Into<String>,
    ) -> Result<(), V8BindingError> {
        self.request(Operation::Control {
            node: self.lease(node)?,
            start,
            end,
            value: value.into(),
        })
        .await
    }
    pub(super) async fn typing_override(
        &self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), V8BindingError> {
        self.request(Operation::Override {
            key: key.into(),
            value: value.into(),
        })
        .await
    }
    pub(super) async fn line_start_spaces(&self,host:NodeId,point:BoundaryPoint)->Result<(),V8BindingError>{
        let selection=self.state.selection_range.get().ok_or(V8BindingError::Allocation("line-space Selection"))?;
        self.request(Operation::SpaceRun{host,selection,revision:self.state.selection_revision.get(),point}).await
    }
    pub(super) async fn text(
        &self,
        node: NodeId,
        offset: u32,
        count: u32,
        data: impl Into<String>,
    ) -> Result<(), V8BindingError> {
        self.request(Operation::Text {
            node: self.lease(node)?,
            offset,
            count,
            data: data.into(),
        })
        .await
    }
    pub(super) async fn remove(&self, node: NodeId) -> Result<(), V8BindingError> {
        self.request(Operation::Remove {
            node: self.lease(node)?,
            ranges: None,
            private_ranges: None,
        })
        .await
    }
    pub(super) async fn append(
        &self,
        parent: NodeId,
        node: NodeId,
        reference: Option<NodeId>,
    ) -> Result<(), V8BindingError> {
        // Removal and insertion are distinct observable steps. Both leases survive
        // callback mutation; insertion revalidates its original reference afterward.
        let reference = {
            let dom = self.state.dom.borrow();
            validate_pre_insertion(&dom, parent, node, reference, None)?;
            if reference == Some(node) {
                let children = light_children(&dom, parent)?;
                children
                    .iter()
                    .position(|child| *child == node)
                    .and_then(|index| children.get(index + 1).copied())
            } else {
                reference
            }
        };
        let operation = Operation::Append {
            parent: self.lease(parent)?,
            node: self.lease(node)?,
            reference: reference.map(|node| self.lease(node)).transpose()?,
            ranges: Vec::new(),
        };
        for _ in 0..4096 {
            self.lease(parent)?;
            self.lease(node)?;
            if let Some(reference) = reference {
                self.lease(reference)?;
            }
            let attached = {
                let dom = self.state.dom.borrow();
                validate_pre_insertion(&dom, parent, node, reference, None)?;
                dom.parent(node)?.is_some()
            };
            if !attached {
                return self.request(operation).await;
            }
            self.remove(node).await?;
        }
        Err(V8BindingError::Unsupported {
            interface: "Document".into(),
            member: "QuotaExceededError".into(),
        })
    }
    pub(super) async fn insert_alignment_wrapper(
        &self,
        parent: NodeId,
        node: NodeId,
        reference: NodeId,
        position: u32,
    ) -> Result<(), V8BindingError> {
        synchronize_live_ranges(&self.state)?;
        let ranges = self
            .state
            .ranges
            .borrow()
            .iter()
            .filter(|(id, _)| !self.state.static_ranges.borrow().contains(id))
            .filter_map(|(id, expected)| {
                let point = |container, offset| {
                    if container == parent && offset == position {
                        Some(BoundaryPoint {
                            container,
                            offset: offset + 1,
                        })
                    } else {
                        None
                    }
                };
                let start = point(expected.start_container, expected.start_offset);
                let end = point(expected.end_container, expected.end_offset);
                (start.is_some() || end.is_some()).then(|| RangeMove {
                    id: *id,
                    private_marker: false,
                    expected: expected.clone(),
                    start,
                    end,
                })
            })
            .collect();
        self.request(Operation::Append {
            parent: self.lease(parent)?,
            node: self.lease(node)?,
            reference: Some(self.lease(reference)?),
            ranges,
        })
        .await
    }
    pub(super) async fn move_preserving_ranges(
        &self,
        node: NodeId,
        parent: NodeId,
        index: u32,
    ) -> Result<(), V8BindingError> {
        synchronize_live_ranges(&self.state)?;
        let (old_parent, old_index, reference, subtree) = {
            let dom = self.state.dom.borrow();
            let old_parent = dom.parent(node)?;
            let old_index = old_parent
                .map(|_| relative_boundary(&dom, node, false).map(|point| point.1))
                .transpose()?;
            let children = light_children(&dom, parent)?;
            if index as usize > children.len() {
                return Err(hierarchy_request_error());
            }
            (
                old_parent,
                old_index,
                children.get(index as usize).copied(),
                dom.arena()
                    .descendants_inclusive(node)?
                    .into_iter()
                    .collect::<BTreeSet<_>>(),
            )
        };
        let reference = {
            let dom = self.state.dom.borrow();
            validate_pre_insertion(&dom, parent, node, reference, None)?;
            if reference == Some(node) {
                let children = light_children(&dom, parent)?;
                children
                    .iter()
                    .position(|child| *child == node)
                    .and_then(|index| children.get(index + 1).copied())
            } else {
                reference
            }
        };
        let node_lease = self.lease(node)?;
        let parent_lease = self.lease(parent)?;
        let reference_lease = reference.map(|node| self.lease(node)).transpose()?;
        let original = self.state.ranges.borrow().clone();
        let private_original = {
            let dom=self.state.dom.borrow();
            self.markers.borrow().iter().map(|id|Ok((*id,RangeRecord::from(dom.live_range(LiveRangeId(*id))?)))).collect::<Result<Vec<_>,V8BindingError>>()?
        };
        let removed = Rc::new(RefCell::new(None));
        let private_removed = Rc::new(RefCell::new(None));
        if old_parent.is_some() {
            self.request(Operation::Remove {
                node: self.lease(node)?,
                ranges: Some(removed.clone()),
                private_ranges: Some((private_original.iter().map(|(id,_)|*id).collect(),private_removed.clone())),
            })
            .await?;
        }
        self.lease(node)?;
        self.lease(parent)?;
        let reattached = self.state.dom.borrow().parent(node)?.is_some();
        if reattached {
            return self.append(parent, node, reference).await;
        }
        let expected = removed
            .borrow_mut()
            .take()
            .unwrap_or_else(|| original.clone());
        let (parent_dom, live_parent) = parent_lease.current();
        if !Rc::ptr_eq(&parent_dom, &self.state.dom) {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "AbortError".into(),
            });
        }
        let position = {
            let dom = self.state.dom.borrow();
            let children = light_children(&dom, live_parent)?;
            match reference {
                Some(reference) => children
                    .iter()
                    .position(|child| *child == reference)
                    .ok_or_else(hierarchy_request_error)? as u32,
                None => children.len() as u32,
            }
        };
        let mut ranges = Vec::new();
        // Private command markers have no JavaScript wrapper and must never
        // enter wrapper GC/accounting. Preserve their subtree positions using
        // the same move mapping while retaining ordinary mutation admission.
        for (id,range) in private_original {
            if !self.markers.borrow().contains(&id){continue}
            let map=|container,offset| {
                if subtree.contains(&container){Some(BoundaryPoint{container,offset})}
                else if Some(container)==old_parent&&old_index.is_some_and(|index|offset==index||offset==index+1){Some(BoundaryPoint{container:live_parent,offset:position+(offset-old_index.unwrap())})}
                else{None}
            };
            let start=map(range.start_container,range.start_offset);
            let end=map(range.end_container,range.end_offset);
            if start.is_some()||end.is_some(){
                let Some(expected)=private_removed.borrow().as_ref().and_then(|ranges|ranges.get(&id)).cloned() else{continue};
                ranges.push(RangeMove{id,private_marker:true,expected,start,end});
            }
        }
        for (id, range) in original {
            if self.state.static_ranges.borrow().contains(&id) {
                continue;
            }
            let Some(expected) = expected.get(&id) else {
                continue;
            };
            let map = |container, offset| {
                if subtree.contains(&container) {
                    Some(BoundaryPoint { container, offset })
                } else if Some(container) == old_parent
                    && old_index.is_some_and(|index| offset == index || offset == index + 1)
                {
                    Some(BoundaryPoint {
                        container: live_parent,
                        offset: position + (offset - old_index.unwrap()),
                    })
                } else {
                    None
                }
            };
            let start = map(range.start_container, range.start_offset);
            let end = map(range.end_container, range.end_offset);
            if start.is_some() || end.is_some() {
                ranges.push(RangeMove {
                    id,
                    private_marker: false,
                    expected: expected.clone(),
                    start,
                    end,
                });
            }
        }
        self.request(Operation::Append {
            parent: parent_lease,
            node: node_lease,
            reference: reference_lease,
            ranges,
        })
        .await
    }
    pub(super) async fn split(&self, node: NodeId, offset: u32) -> Result<NodeId, V8BindingError> {
        let output = Rc::new(RefCell::new(None));
        self.request(Operation::Split {
            node: self.lease(node)?,
            offset,
            output: output.clone(),
        })
        .await?;
        let lease = output
            .borrow_mut()
            .take()
            .ok_or(V8BindingError::Allocation("editing split output"))?;
        let (dom, node) = lease.current();
        if !Rc::ptr_eq(&dom, &self.state.dom) {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "AbortError".into(),
            });
        }
        Ok(node)
    }
    pub(super) async fn after(
        &self,
        parent: NodeId,
        reference: NodeId,
        node: NodeId,
    ) -> Result<(), V8BindingError> {
        let siblings = light_children(&self.state.dom.borrow(), parent)?;
        let index = siblings
            .iter()
            .position(|child| *child == reference)
            .ok_or(DomError::NotAChild(reference, parent))?;
        self.append(parent, node, siblings.get(index + 1).copied())
            .await
    }
    pub(super) async fn insert_at_boundary(
        &self,
        container: NodeId,
        offset: u32,
        node: NodeId,
    ) -> Result<(), V8BindingError> {
        let kind = self.state.dom.borrow().arena().get(container)?.kind.clone();
        match kind {
            NodeKind::Text { .. } | NodeKind::CDataSection { .. } => {
                let parent = self
                    .state
                    .dom
                    .borrow()
                    .parent(container)?
                    .ok_or_else(hierarchy_request_error)?;
                let _parent = self.lease(parent)?;
                let output = Rc::new(RefCell::new(None));
                self.request(Operation::Split {
                    node: self.lease(container)?,
                    offset,
                    output: output.clone(),
                })
                .await?;
                let lease = output
                    .borrow_mut()
                    .take()
                    .ok_or(V8BindingError::Allocation("editing split output"))?;
                let (dom, tail) = lease.current();
                if !Rc::ptr_eq(&dom, &self.state.dom) {
                    return Err(V8BindingError::Unsupported {
                        interface: "Document".into(),
                        member: "AbortError".into(),
                    });
                }
                self.append(parent, node, Some(tail)).await
            }
            NodeKind::Comment { .. } | NodeKind::ProcessingInstruction { .. } => {
                Err(hierarchy_request_error())
            }
            _ => {
                let reference = light_children(&self.state.dom.borrow(), container)?
                    .get(offset as usize)
                    .copied();
                self.append(container, node, reference).await
            }
        }
    }
    pub(super) async fn layout(&self) -> Result<(), V8BindingError> {
        self.request(Operation::Layout).await
    }
    pub(super) async fn retain_metadata(&self, bytes: usize)
        -> Result<Option<std::sync::Arc<dyn Send + Sync>>, V8BindingError> {
        self.metadata_result.borrow_mut().take();
        self.request(Operation::RetainMetadata { bytes }).await?;
        self.metadata_result.borrow_mut().take()
            .ok_or(V8BindingError::Allocation("missing editing metadata receipt"))
    }

    pub(super) async fn legacy_font(&self, keyword: BindingLegacyFontKeyword)
        -> Result<BindingLegacyFontResult, V8BindingError>
    {
        self.legacy_font_result.set(None);
        self.request(Operation::LegacyFont { keyword }).await?;
        let result = self.legacy_font_result.take();
        result.ok_or(V8BindingError::Allocation("missing editing font demand result"))
    }
    async fn request(&self, operation: Operation) -> Result<(), V8BindingError> {
        let request = Rc::new(Request {
            operation,
            capture: RefCell::new(None),
            space_run: RefCell::new(None),
            prepared: RefCell::new(None),
            mutated: Cell::new(false),
            recorded: Cell::new(false),
            error: RefCell::new(None),
            reply: RefCell::new(None),
        });
        struct Wait<'a> {
            context: &'a Context,
            request: Rc<Request>,
            registered: bool,
        }
        impl Future for Wait<'_> {
            type Output = Result<(), V8BindingError>;
            fn poll(mut self: Pin<&mut Self>, _: &mut PollContext<'_>) -> Poll<Self::Output> {
                if let Some(result) = self.request.reply.borrow_mut().take() {
                    return Poll::Ready(result);
                }
                if !self.registered {
                    if self.context.pending.borrow().is_some() {
                        return Poll::Ready(Err(V8BindingError::Allocation(
                            "nested native editing operation",
                        )));
                    }
                    *self.context.pending.borrow_mut() = Some(self.request.clone());
                    self.registered = true;
                }
                Poll::Pending
            }
        }
        Wait {
            context: self,
            request,
            registered: false,
        }
        .await
    }
}
pub(super) struct Continuation {
    context: Rc<Context>,
    future: Option<EditingFuture>,
    publish_history: bool,
    host: oryn_dom::TrackedNodeLease,
    identity: (NodeId, u64, u64, u64),
    command: String,
    before: Option<EditingSnapshot>,
    segments: Vec<editing_inverse::Segment>,
    completed: Option<Result<(), V8BindingError>>,
    terminal_recorded: bool,
    result: Option<bool>,
    invocation: Option<Rc<editing_commands::Invocation>>,
    history_undo: Option<UndoUnit>,
    history_recovering: bool,
}
impl Continuation {
    pub(super) fn new(
        state: Rc<V8BindingState>,
        command: String,
        build: impl FnOnce(Rc<Context>) -> EditingFuture,
    ) -> Result<Option<Self>, V8BindingError> {
        Self::new_at(state, None, command, build)
    }
    pub(super) fn new_at(
        state: Rc<V8BindingState>,
        host: Option<NodeId>,
        command: String,
        build: impl FnOnce(Rc<Context>) -> EditingFuture,
    ) -> Result<Option<Self>, V8BindingError> {
        resume_editing_recovery(&state)?;
        let Some(host) = (match host {
            Some(host) => Some(host),
            None => editing_host(&state)?,
        }) else {
            return Ok(None);
        };
        editing_snapshot_budget(&state, &[host])?;
        let before = capture_editing_snapshot(&state, host)?;
        let host = oryn_dom::TrackedNodeLease::new(&state.dom, host)?;
        let identity = (
            state.document_root(),
            state.document_generation(),
            state.document_stream_epoch(),
            state.document_authority_epoch(),
        );
        let context = Rc::new(Context {
            state,
            clones: RefCell::new(Vec::new()),
            pending: RefCell::new(None),
            tracked: RefCell::new(BTreeMap::new()),
            admitted: RefCell::new(BTreeSet::new()),
            extra_roots: RefCell::new(Vec::new()),
            legacy_font_result: Cell::new(None),
            metadata_result: RefCell::new(None),
            markers: Rc::new(RefCell::new(BTreeSet::new())),
            paragraph_split: RefCell::new(None),
        });
        for record in &before.nodes {
            context.lease(record.node)?;
            context.admitted.borrow_mut().insert(record.node);
        }
        let future = build(context.clone());
        Ok(Some(Self {
            publish_history: true,
            context,
            future: Some(future),
            host,
            identity,
            command,
            before: Some(before),
            segments: Vec::new(),
            completed: None,
            terminal_recorded: false,
            result: None,
            invocation: None,
            history_undo: None,
            history_recovering: false,
        }))
    }
    pub(super) fn refresh_baseline(&mut self) -> Result<(), V8BindingError> {
        self.baseline()
    }
    pub(super) fn retain_invocation(
        &mut self,
        invocation: Option<Rc<editing_commands::Invocation>>,
    ) {
        self.invocation = invocation;
    }
    pub(super) fn stop_unperformed(&mut self, error: V8BindingError) {
        self.future.take();
        self.context.pending.borrow_mut().take();
        self.completed = Some(Err(error));
        self.terminal_recorded = true;
    }
    pub(super) fn terminal_finished(&self) -> bool {
        self.result.is_some()
    }
    pub(super) fn has_mutations(&self) -> bool {
        !self.segments.is_empty() || self.result == Some(true)
    }
    pub(super) fn add_hosts(&mut self, hosts: &[NodeId]) -> Result<(), V8BindingError> {
        for host in hosts {
            self.context
                .extra_roots
                .borrow_mut()
                .push(self.context.lease(*host)?);
            let nodes = self
                .context
                .state
                .dom
                .borrow()
                .arena()
                .descendants_inclusive(*host)?;
            for node in nodes {
                self.context.lease(node)?;
                self.context.admitted.borrow_mut().insert(node);
            }
        }
        self.baseline()
    }
    pub(super) fn retain_history_undo(&mut self, unit: UndoUnit, recovering: bool) -> Result<(), V8BindingError> {
        let mut endpoints = Vec::new();
        for point in [unit.before.selection.anchor, unit.before.selection.focus].into_iter().flatten() {
            endpoints.push(self.context.lease(point.container)?);
        }
        self.context.extra_roots.borrow_mut().extend(endpoints);
        self.publish_history = false;
        self.history_undo = Some(unit);
        self.history_recovering = recovering;
        self.baseline()
    }
    pub(super) fn defer_history(&mut self) {
        self.publish_history = false;
    }
    pub(super) fn take_segments(&mut self) -> Vec<editing_inverse::Segment> {
        std::mem::take(&mut self.segments)
    }
    pub(super) fn cancel(&mut self) {
        if let Some(unit)=self.history_undo.as_ref() {
            editing_commands::paragraph_history::receipt::retain_failed(&self.context.state,unit,std::mem::take(&mut self.segments),&V8BindingError::Allocation("paragraph undo cancelled"));
            editing_commands::paragraph_history::receipt::retire_failed(&self.context.state,unit);
        }

        for job in self.context.clones.borrow_mut().drain(..) {
            job.background.set(true);
        }
        self.future.take();
        self.context.pending.borrow_mut().take();
        self.context.tracked.borrow_mut().clear();
        self.context.admitted.borrow_mut().clear();
        self.context.extra_roots.borrow_mut().clear();
        self.before.take();
        self.segments.clear();
        self.completed.take();
        self.result = Some(false);
    }
    fn current(&self) -> bool {
        if let Some(unit)=self.history_undo.as_ref()
            && !editing_commands::paragraph_history::receipt::undo_owner_current(&self.context.state,unit) { return false; }

        let state = &self.context.state;
        let (dom, node) = self.host.current();
        !state.execution_retired()
            && !state.execution_tasks_stopped()
            && Rc::ptr_eq(&dom, &state.dom)
            && node == self.before.as_ref().map_or(node, |before| before.host)
            && self.identity
                == (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.document_authority_epoch(),
                )
    }
    fn reserve_actual_snapshot(&self)->Result<(),V8BindingError>{
        let Some(unit)=self.history_undo.as_ref() else{return Ok(())};
        let state=&self.context.state;
        let (_,host)=self.host.current();
        let roots=self.context.snapshot_roots(host)?;
        let roots=roots.into_iter().collect::<Vec<_>>();
        let bound=editing_snapshot_budget(state,&roots)?;
        let completed=self.segments.iter().try_fold(0usize,|n,segment|n.checked_add(segment.bytes())).ok_or(V8BindingError::Allocation("paragraph snapshot accounting"))?;
        let prospective=bound.checked_mul(2).and_then(|n|n.checked_add(completed)).and_then(|n|n.checked_mul(2)).ok_or(V8BindingError::Allocation("paragraph snapshot accounting"))?;
        unit.paragraph_reservation.admit(state,unit,prospective)
    }
    fn segment(&mut self) -> Result<(), V8BindingError> {
        let state = &self.context.state;
        let (_, host) = self.host.current();
        editing_snapshot_budget(state, &[host])?;
        let after = {
            let pending=self.context.pending.borrow();
            let capture=pending.as_ref().map(|request|request.capture.borrow());
            if let Some(capture)=capture.as_ref().and_then(|capture|capture.as_ref()) {
                self.context.snapshot_with_roots(host,&capture.roots)?
            } else { self.context.snapshot(host)? }
        };
        if let Some(before) = self.before.as_ref() {
            if let Some(request)=self.context.pending.borrow().as_ref()
                && let Some(capture)=request.capture.borrow().as_ref() {
                for lease in &capture.roots { self.resolve(lease)?; }
                if after.serialized_bytes>capture.bound { return Err(V8BindingError::Allocation("paragraph prepaid snapshot bound")); }
            }
            let segment =
                editing_inverse::Segment::new(before.clone(), after.clone()).compact(state)?;
            if segment.changed() {
                let bytes = self
                    .segments
                    .iter()
                    .fold(segment.bytes(), |bytes, segment| {
                        bytes.saturating_add(segment.bytes())
                    });
                self.segments.push(segment);
                if bytes > UndoHistory::MAX_BYTES {
                    self.before = Some(after);
                    return Err(V8BindingError::Unsupported {
                        interface: "Document".into(),
                        member: "QuotaExceededError".into(),
                    });
                }
            }
        }
        self.before = Some(after);
        Ok(())
    }
    fn baseline(&mut self) -> Result<(), V8BindingError> {
        self.reserve_actual_snapshot()?;
        let state = &self.context.state;
        let (_, host) = self.host.current();
        editing_snapshot_budget(state, &[host])?;
        self.before = Some(self.context.snapshot(host)?);
        Ok(())
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<bool>, V8BindingError> {
        match self.poll_inner(scope) {
            Err(error)
                if self.result.is_none()
                    && (matches!(&error,V8BindingError::Unsupported{member,..}if member=="QuotaExceededError")
                        || matches!(
                            &error,
                            V8BindingError::Allocation("compound editing snapshot budget")
                        )) =>
            {
                if self.history_undo.is_some() && self.context.pending.borrow().as_ref().is_some_and(|request|request.mutated.get()&&!request.recorded.get()) {
                    return Err(error);
                }
                // Capacity is terminal, not a perpetual retry. Preserve committed inverse
                // custody, cancel the exact unperformed suffix and publish the error once.
                self.future.take();
                self.context.pending.borrow_mut().take();
                self.completed = Some(Err(error));
                self.terminal_recorded = true;
                self.poll_inner(scope)
            }
            result => result,
        }
    }
    fn poll_inner(&mut self, scope: &mut v8::PinScope) -> Result<Option<bool>, V8BindingError> {
        if let Some(result) = self.result {
            return Ok(Some(result));
        }
        if !self.current() {
            if self.history_undo.is_some() && !self.context.state.execution_retired() {
                let pending=self.context.pending.borrow().clone();
                if let Some(request)=pending && request.mutated.get() && !request.recorded.get() {
                    self.segment()?;
                    request.recorded.set(true);
                    request.capture.borrow_mut().take();
                }
            }
            self.cancel();
            return Ok(Some(false));
        }
        if self.context.state.execution_suspended() || scope.is_execution_terminating() {
            return Ok(None);
        }
        let waker = Waker::noop();
        let mut poll = PollContext::from_waker(waker);
        for _ in 0..1024 {
            let pending = self.context.pending.borrow().clone();
            if let Some(request) = pending {
                if let Operation::RetainMetadata { bytes } = &request.operation {
                    if !request.recorded.get() { self.segment()?; request.recorded.set(true); }
                    let state = &self.context.state;
                    let result = (|| {
                        let _scope = state.enter_retained_allocation(scope)?;
                        if !self.current() { return Err(V8BindingError::Allocation("retired editing metadata demand")); }
                        let allocation = state.retained_allocation.borrow().clone();
                        allocation.map(|owner| owner.reserve(*bytes)).transpose()
                            .map_err(V8BindingError::EventCallback)
                    })();
                    if !self.current() { self.cancel(); return Ok(Some(false)); }
                    *request.reply.borrow_mut() = Some(result.map(|receipt| {
                        *self.context.metadata_result.borrow_mut() = Some(receipt);
                    }));
                    self.context.pending.borrow_mut().take();
                    continue;
                }
                if let Operation::LegacyFont { keyword } = &request.operation {
                    if !request.recorded.get() {
                        self.segment()?;
                        request.recorded.set(true);
                    }
                    let state = &self.context.state;
                    let result = (|| -> Result<BindingLegacyFontResult, V8BindingError> {
                        refresh_layout_in_scope(scope, state)?;
                        let _retained_scope = state.enter_retained_allocation(scope)?;
                        if !self.current() {
                            return Err(V8BindingError::Allocation("retired editing font demand"));
                        }
                        let provider = state.layout_provider.borrow().clone()
                            .ok_or(V8BindingError::Allocation("missing owning font provider"))?;
                        let body = state.dom.borrow().document_body(state.document_root())?
                            .ok_or(V8BindingError::Allocation("missing owning font body"))?;
                        let demand = BindingLegacyFontRequest {
                            document: state.document_root(), body,
                            document_generation: state.document_generation(),
                            stream_epoch: state.document_stream_epoch(),
                            authority_epoch: state.document_authority_epoch(),
                            style_generation: provider.layout_revision()
                                .ok_or(V8BindingError::Allocation("unversioned font provider"))?,
                            keyword: *keyword,
                        };
                        let bytes = {
                            let custom = state.custom_states.borrow();
                            let mut bytes = state.scroll_offsets.borrow().len()
                                .checked_mul(std::mem::size_of::<(NodeId, (f64, f64))>() + 6 * std::mem::size_of::<usize>())
                                .ok_or(V8BindingError::Allocation("font scroll metadata overflow"))?;
                            for values in custom.values() {
                                bytes = bytes.checked_add(std::mem::size_of::<NodeId>() + 8 * std::mem::size_of::<usize>())
                                    .ok_or(V8BindingError::Allocation("font state metadata overflow"))?;
                                for value in values {
                                    bytes = bytes.checked_add(value.len()).and_then(|n| n.checked_add(8 * std::mem::size_of::<usize>()))
                                        .ok_or(V8BindingError::Allocation("font state text overflow"))?;
                                }
                            }
                            bytes
                        };
                        let allocation = state.retained_allocation.borrow().clone();
                        let _metadata = allocation.map(|allocation| allocation.reserve(bytes))
                            .transpose().map_err(V8BindingError::EventCallback)?;
                        let custom = state.custom_states.borrow().clone();
                        let element_scrolls = state.element_scroll_offsets();
                        let resolved = provider.resolve_legacy_font_keyword(&state.dom, demand,
                            &custom, state.document_scroll_offset(), &element_scrolls)
                            .ok_or(V8BindingError::Allocation("font query unavailable"))?
                            .map_err(V8BindingError::EventCallback)?;
                        if !self.current() || resolved.request != demand
                            || provider.layout_revision() != Some(demand.style_generation)
                            || state.dom.borrow().document_body(demand.document)? != Some(body)
                        {
                            return Err(V8BindingError::Allocation("stale editing font result"));
                        }
                        Ok(resolved)
                    })();
                    if !self.current() {
                        self.cancel();
                        return Ok(Some(false));
                    }
                    *request.reply.borrow_mut() = Some(result.map(|value| {
                        self.context.legacy_font_result.set(Some(value));
                    }));
                    self.context.pending.borrow_mut().take();
                    continue;
                }
                if matches!(&request.operation, Operation::Layout) {
                    if !request.recorded.get() {
                        self.segment()?;
                        request.recorded.set(true);
                    }
                    // A suspended edit cannot retain a V8 scope. Serve its
                    // explicit layout demand here, under the live owner, and
                    // publish a reply before resuming native algorithm work.
                    match refresh_layout_in_scope(scope, &self.context.state) {
                        Ok(()) => {
                            if !self.current() {
                                self.cancel();
                                return Ok(Some(false));
                            }
                            *request.reply.borrow_mut() = Some(Ok(()));
                            self.context.pending.borrow_mut().take();
                            continue;
                        }
                        Err(error @ V8BindingError::Allocation(_)) => return Err(error),
                        Err(error) => {
                            *request.reply.borrow_mut() = Some(Err(error));
                            self.context.pending.borrow_mut().take();
                            continue;
                        }
                    }
                }
                if !request.mutated.get() {
                    // All native-only work preceding this await becomes an independent
                    // inverse before any script may observe it.
                    if !request.recorded.get() {
                        self.segment()?;
                        request.recorded.set(true);
                    }
                    if self.history_undo.is_some() && request.capture.borrow().is_none() {
                        self.prepay_capture(&request,1024)?;
                        let (_,host)=self.host.current();
                        let baseline={
                            let capture=request.capture.borrow();
                            #[cfg(test)]
                            let captured=if FAIL_PARAGRAPH_PREIMAGE.with(|flag|flag.replace(false)) {
                                Err(V8BindingError::Allocation("injected paragraph preimage failure"))
                            } else {self.context.snapshot_with_roots(host,&capture.as_ref().unwrap().roots)};
                            #[cfg(not(test))]
                            let captured=self.context.snapshot_with_roots(host,&capture.as_ref().unwrap().roots);
                            match captured {
                                Ok(baseline)=>baseline,
                                Err(error)=>{drop(capture);request.capture.borrow_mut().take();return Err(error);}
                            }
                        };
                        self.before=Some(baseline);
                    }
                    let result = match self.prepare(scope, &request) {
                        Ok(()) => self.mutate(&request),
                        Err(error @ V8BindingError::Allocation(_)) => return Err(error),
                        Err(error) => Err(error),
                    };
                    request.mutated.set(true);
                    *request.error.borrow_mut() = result.err();
                    #[cfg(test)]
                    if self.history_undo.is_some()
                        && request.error.borrow().is_none()
                        && FAIL_PARAGRAPH_AFTER_MUTATION.with(|flag|flag.replace(false))
                    {
                        *request.error.borrow_mut() = Some(V8BindingError::Allocation("injected paragraph terminal mutation failure"));
                    }
                    // Snapshot failure retains the mutated flag and retries capture only.
                    request.recorded.set(false);
                }
                if !request.recorded.get() {
                    self.segment()?;
                    request.recorded.set(true);
                    request.capture.borrow_mut().take();
                }
                #[cfg(test)]
                if FAIL_BEFORE_PREPARATION.with(|flag| flag.replace(false)) {
                    return Err(V8BindingError::Allocation(
                        "injected editing author-boundary failure",
                    ));
                }
                complete_editing_range_clones(scope, &self.context.state)?;
                if let Some(prepared) = request.prepared.borrow_mut().as_mut()
                    && !prepared.poll(scope)?
                {
                    return Ok(None);
                }
                if !self.current() {
                    self.cancel();
                    return Ok(Some(false));
                }
                #[cfg(test)]
                if FAIL_AFTER_PREPARATION.with(|flag| flag.replace(false)) {
                    return Err(V8BindingError::Allocation(
                        "injected editing post-author failure",
                    ));
                }
                // Author changes form the next baseline, never part of an inverse.
                self.baseline()?;
                let result = request.error.borrow_mut().take().map_or(Ok(()), Err);
                *request.reply.borrow_mut() = Some(result);
                self.context.pending.borrow_mut().take();
            }
            if self.completed.is_none() {
                match self.future.as_mut().unwrap().as_mut().poll(&mut poll) {
                    Poll::Pending => {
                        if self.context.pending.borrow().is_none() {
                            return Err(V8BindingError::Allocation(
                                "native editing continuation without operation",
                            ));
                        }
                        continue;
                    }
                    Poll::Ready(result) => {
                        self.completed = Some(result);
                        self.future.take();
                    }
                }
            }
            if !self.terminal_recorded {
                self.segment()?;
                self.terminal_recorded = true;
            }
            // A native failure never replays or rolls back committed segments. The
            // completed suffix remains owned until undo publication succeeds.
            let before = self
                .segments
                .first()
                .map(|segment| segment.before.clone())
                .or_else(|| self.before.clone())
                .ok_or(V8BindingError::Allocation("native editing undo baseline"))?;
            let after = self.before.as_ref().unwrap().clone();
            let changed = !self.segments.is_empty();
            if changed && self.publish_history {
                let mut controls: BTreeMap<NodeId, ControlEdit> = BTreeMap::new();
                for segment in &self.segments {
                    for before in &segment.before.controls {
                        if let Some(after) = segment
                            .after
                            .controls
                            .iter()
                            .find(|after| after.node == before.node)
                            && before.value != after.value
                        {
                            let entry = controls.entry(before.node).or_insert((
                                before.node,
                                before.value.clone(),
                                before.selection.clone(),
                                after.value.clone(),
                                after.selection.clone(),
                            ));
                            entry.3 = after.value.clone();
                            entry.4 = after.selection.clone();
                        }
                    }
                }
                let changes = native_keyboard::segmented_document_changes(
                    controls.into_values().collect::<Vec<_>>(),
                );
                let identity = Rc::new(());
                let control_owners = changes.iter().map(|change| change.0).collect();
                native_keyboard::record_document_edit(&self.context.state, &identity, changes);
                push_undo_unit(
                    &self.context.state,
                    UndoUnit {
                        native_progress: Default::default(),
                        native_segments: self.segments.clone(),
                        before,
                        after,
                        companions: Vec::new(),
                        identity,
                        control_owners,
                        command: self.command.clone(),
                        split_nodes: None,
                        paragraph_split: self.context.paragraph_split.borrow().clone(),
                        paragraph_redo: Default::default(),
                        paragraph_failed: Default::default(),
                        paragraph_reservation: Default::default(),
                        cross_document: None,
                    },
                )?;
            }
            if self.publish_history {
                self.segments.clear();
            }
            if let Some(unit)=self.history_undo.as_ref() {
                if let Some(Err(error))=self.completed.as_ref() {
                    editing_commands::paragraph_history::receipt::retain_failed(&self.context.state,unit,std::mem::take(&mut self.segments),error);
                } else if self.history_recovering {
                    editing_commands::paragraph_history::recovered(&self.context.state,unit)?;
                } else {
                    editing_commands::paragraph_history::commit(&self.context.state,unit,self.segments.clone())?;
                }
                self.history_undo.take();
            }
            self.before.take();
            self.result = Some(changed);
            let result = self.completed.take().unwrap();
            return result.map(|_| Some(changed));
        }
        Ok(None)
    }
    fn resolve(&self, lease: &oryn_dom::TrackedNodeLease) -> Result<NodeId, V8BindingError> {
        let (dom, node) = lease.current();
        if !Rc::ptr_eq(&dom, &self.context.state.dom) {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "AbortError".into(),
            });
        }
        if dom.borrow().arena().node_document(node)? != Some(self.context.state.document_root()) {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "AbortError".into(),
            });
        }
        Ok(node)
    }
    fn prepay_capture(&self,request:&Request,growth:usize)->Result<(),V8BindingError>{
    let Some(unit)=self.history_undo.as_ref() else{return Ok(())};
    let state=&self.context.state;
    let (_,host)=self.host.current();
    let mut roots=self.context.snapshot_roots(host)?;
    if let Operation::Append{parent,node,..}=&request.operation {
        roots.insert(self.resolve(parent)?);roots.insert(self.resolve(node)?);
        if let Some(parent)=state.dom.borrow().parent(self.resolve(node)?)? {roots.insert(parent);}
    }
    if let Operation::Remove{node,..}=&request.operation {roots.insert(self.resolve(node)?);}
    if let Operation::Selection{anchor,focus,..}=&request.operation {roots.insert(self.resolve(anchor)?);roots.insert(self.resolve(focus)?);}
    let roots=roots.into_iter().collect::<Vec<_>>();
    let before=editing_snapshot_budget(state,&roots)?;
    // Snapshot() merges overlapping roots, so summing each root is conservative.
    // Native Remove/Append creates no text/attributes; growth remains existing
    // metadata bound. No author callback occurs before its after-image capture.
    let after=before.checked_add(growth).ok_or(V8BindingError::Allocation("paragraph prepaid capture accounting"))?;
    let completed=self.segments.iter().try_fold(0usize,|n,segment|n.checked_add(segment.bytes())).ok_or(V8BindingError::Allocation("paragraph prepaid capture accounting"))?;
    let reserve=completed.checked_add(before).and_then(|n|n.checked_add(after)).and_then(|n|n.checked_mul(2)).ok_or(V8BindingError::Allocation("paragraph prepaid capture accounting"))?;
    unit.paragraph_reservation.admit(state,unit,reserve)?;
    let mut leases=Vec::new();leases.try_reserve_exact(roots.len()).map_err(|_|V8BindingError::Allocation("paragraph prepaid capture roots"))?;
    for root in roots {leases.push(oryn_dom::TrackedNodeLease::new(&state.dom,root)?);}
    *request.capture.borrow_mut()=Some(PrepaidCapture{roots:leases,bound:after});
    Ok(())
}


    fn prepare(&self, scope: &mut v8::PinScope, request: &Request) -> Result<(), V8BindingError> {
        if request.prepared.borrow().is_some() {
            return Ok(());
        }
        let state = &self.context.state;
        let growth = match &request.operation {
            Operation::RemoveAttribute { .. } => 0,
            Operation::Overrides { value } => {
                value.0.iter().fold(1024usize, |bytes, (key, value)| {
                    bytes.saturating_add(key.len()).saturating_add(value.len())
                })
            }
            Operation::Attribute { name, value, .. } | Operation::Style { name, value, .. } => name
                .len()
                .saturating_add(value.len())
                .saturating_mul(2)
                .saturating_add(1024),
            Operation::Control { value, .. } => value.len().saturating_mul(4).saturating_add(1024),
            Operation::Override { key, value } => key
                .len()
                .saturating_add(value.len())
                .saturating_mul(2)
                .saturating_add(1024),
            Operation::Text { data, .. } => data.len().saturating_mul(2).saturating_add(1024),
            Operation::SpaceRun { .. } => 1024,
            Operation::Split { .. } => 2048,
            _ => 1024,
        };
        let newly_admitted = match &request.operation {
            Operation::Attribute { node, .. }
            | Operation::RemoveAttribute { node, .. }
            | Operation::Style { node, .. }
            | Operation::Control { node, .. }
            | Operation::Text { node, .. }
            | Operation::Split { node, .. }
            | Operation::Remove { node, .. }
            | Operation::RemoveWithMarkers { node, .. }
            | Operation::Append { node, .. } => {
                let node = self.resolve(node)?;
                if self.context.admitted.borrow().contains(&node) {
                    0
                } else {
                    if self.history_undo.is_some(){editing_snapshot_budget(state,&[node])?}else{capture_editing_snapshot(state,node)?.serialized_bytes}
                }
            }
            _ => 0,
        };
        let parent_growth = if let Operation::Append { parent, .. } = &request.operation {
            let parent = self.resolve(parent)?;
            if self.context.admitted.borrow().contains(&parent) {
                0
            } else {
                if self.history_undo.is_some(){editing_snapshot_budget(state,&[parent])?}else{capture_editing_snapshot(state,parent)?.serialized_bytes}
            }
        } else {
            0
        };
        if self
            .before
            .as_ref()
            .map_or(0, |before| before.serialized_bytes)
            .saturating_add(growth)
            .saturating_add(newly_admitted)
            .saturating_add(parent_growth)
            > UndoHistory::MAX_BYTES / 2
        {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "QuotaExceededError".into(),
            });
        }
        if request.capture.borrow().is_none(){self.prepay_capture(request,growth)?;}
        if matches!(request.operation,Operation::Selection{..}|Operation::ClearSelection) {return Ok(())}
        let preparation = match &request.operation {
            Operation::Selection {..}|Operation::ClearSelection => unreachable!("selection handled before author preparation"),
            Operation::Layout | Operation::LegacyFont { .. } | Operation::RetainMetadata { .. } => return Err(V8BindingError::Allocation("unserved editing layout demand")),
            Operation::SpaceRun { host, selection, revision, point } => {
                let _retained_scope=state.enter_retained_allocation(scope)?;
                request.space_run.borrow_mut().take();
                if state.selection_range.get()!=Some(*selection)||state.selection_revision.get()!=*revision{return Ok(())}
                let live=state.dom.borrow().live_range(LiveRangeId(*selection))?;
                if live.start!=*point||live.end!=*point{return Ok(())}
                let host=self.context.lease(*host)?;
                let host_id=self.resolve(&host)?;
                let run=editing_space_run_with_allocation(state,*point,host_id,EditingSpaceAllocation{owner:state.retained_allocation.borrow().clone(),leases:Vec::new()})?;
                if !run.starts_line||!run.before.is_empty()||run.after.is_empty(){return Ok(())}
                let bytes=run.after.len().checked_mul(std::mem::size_of::<(oryn_dom::TrackedNodeLease,u32,u16)>()+std::mem::size_of::<NodeId>()+8).ok_or(V8BindingError::Allocation("line-space metadata overflow"))?;
                let allocation=state.retained_allocation.borrow().clone();
                let allocation=allocation.map(|allocation|allocation.reserve(bytes)).transpose().map_err(V8BindingError::EventCallback)?;
                let mut spaces=Vec::new();spaces.try_reserve_exact(run.after.len()).map_err(|_|V8BindingError::Allocation("line-space positions"))?;
                for space in run.after{spaces.push((self.context.lease(space.node)?,space.offset,space.value));}
                let dom=state.dom.borrow();
                *request.space_run.borrow_mut()=Some(PreparedSpaceRun{host,spaces,ends_line:run.ends_line,_allocation:allocation,_scan:run._allocation,mutation:dom.mutation_cursor(),boundary:dom.boundary_revision()});
                drop(dom);
                Preparation::Parent(script_children_changed::parent_change(scope,state,None)?)
            }
            Operation::RemoveAttribute { node, .. }
            | Operation::Attribute { node, .. }
            | Operation::Style { node, .. }
            | Operation::Control { node, .. } => {
                self.resolve(node)?;
                Preparation::Parent(script_children_changed::parent_change(scope, state, None)?)
            }
            Operation::Overrides { .. } | Operation::Override { .. } => {
                Preparation::Parent(script_children_changed::parent_change(scope, state, None)?)
            }
            Operation::Split { node, .. } | Operation::Text { node, .. } => Preparation::Parent(
                script_children_changed::character_data(scope, state, self.resolve(node)?)?,
            ),
            Operation::Remove { node, .. } | Operation::RemoveWithMarkers { node, .. } => {
                let node = self.resolve(node)?;
                let parent = state.dom.borrow().parent(node)?;
                Preparation::Parent(script_children_changed::parent_change(
                    scope, state, parent,
                )?)
            }
            Operation::Append {
                parent,
                node,
                reference,
                ..
            } => {
                let parent = self.resolve(parent)?;
                let node = self.resolve(node)?;
                let reference = reference
                    .as_ref()
                    .map(|node| self.resolve(node))
                    .transpose()?;
                validate_pre_insertion(&state.dom.borrow(), parent, node, reference, None)?;
                Preparation::Insertion(post_connection_scripts::capture_insertion(
                    scope, state, parent, node,
                )?)
            }
        };
        *request.prepared.borrow_mut() = Some(preparation);
        Ok(())
    }
    fn mutate(&self, request: &Request) -> Result<(), V8BindingError> {
        let state = &self.context.state;
        if let Operation::Append { parent, .. } = &request.operation {
            self.context
                .admitted
                .borrow_mut()
                .insert(self.resolve(parent)?);
        }
        match &request.operation {
            Operation::Attribute { node, .. }
            | Operation::RemoveAttribute { node, .. }
            | Operation::Style { node, .. }
            | Operation::Control { node, .. }
            | Operation::Text { node, .. }
            | Operation::Split { node, .. }
            | Operation::Remove { node, .. }
            | Operation::RemoveWithMarkers { node, .. }
            | Operation::Append { node, .. } => {
                self.context
                    .admitted
                    .borrow_mut()
                    .insert(self.resolve(node)?);
            }
            _ => {}
        }
        match &request.operation {
            Operation::ClearSelection => clear_selection(state),
            Operation::Selection {anchor,anchor_offset,focus,focus_offset} => {
                let a=BoundaryPoint{container:self.resolve(anchor)?,offset:*anchor_offset};
                let f=BoundaryPoint{container:self.resolve(focus)?,offset:*focus_offset};
                set_selection_boundaries(state,a,f)?;
            }
            Operation::Layout | Operation::LegacyFont { .. } | Operation::RetainMetadata { .. } => return Err(V8BindingError::Allocation("unserved editing layout demand")),
            Operation::SpaceRun { selection, revision, point, .. } => {
                let prepared=request.space_run.borrow();
                let Some(PreparedSpaceRun{host,spaces,ends_line,mutation,boundary,..})=prepared.as_ref() else{return Ok(())};
                let host=self.resolve(host)?;
                if state.selection_range.get()!=Some(*selection)||state.selection_revision.get()!=*revision{return Ok(())}
                let live=state.dom.borrow().live_range(LiveRangeId(*selection))?;
                if live.start!=*point||live.end!=*point{return Ok(())}
                {let dom=state.dom.borrow();if dom.mutation_cursor()!=*mutation||dom.boundary_revision()!=*boundary{return Ok(())}}
                let mut ids=Vec::new();ids.try_reserve_exact(spaces.len()).map_err(|_|V8BindingError::Allocation("line-space nodes"))?;
                let mut previous=None;
                for (lease,_,_) in spaces{
                    let node=self.resolve(lease)?;
                    // Prepared scan checked units; exact DOM mutation and
                    // boundary revisions prove no intervening change.
                    if previous!=Some(node){
                        let dom=state.dom.borrow();if dom.parent(node)?.is_some_and(|parent|is_script_element(&dom,parent)){return Ok(())}
                        if editing_host_for_node(&dom,Some(node),state.design_mode_enabled(),state.document_root())?!=Some(host){return Ok(())}
                        previous=Some(node);
                    }
                    ids.push(node);
                }
                // One admitted native producer validates the whole rendered
                // suffix once, then publishes author callbacks after all edits.
                let mut dom=state.dom.borrow_mut();
                for (index,(node,(_,offset,expected))) in ids.iter().zip(spaces).enumerate(){
                    let value=if index%2==0||(*ends_line&&index+1==spaces.len()){160}else{32};
                    if value==*expected{continue}
                    let replacement=oryn_dom::dom_string::from_units(&[value]);
                    dom.replace_character_data(*node,*offset,0,&replacement)?;
                    dom.replace_character_data(*node,*offset+1,1,"")?;
                }
            }
            Operation::RemoveAttribute { node, name } => {
                let node = self.resolve(node)?;
                state
                    .dom
                    .borrow_mut()
                    .remove_attribute_ns(node, None, name)?;
            }
            Operation::Overrides { value } => *state.editing_overrides.borrow_mut() = value.clone(),
            Operation::Attribute {
                node,
                namespace,
                name,
                value,
            } => {
                let node = self.resolve(node)?;
                state.dom.borrow_mut().set_attribute_ns(
                    node,
                    namespace.clone(),
                    name.clone(),
                    value.clone(),
                )?;
            }
            Operation::Style { node, name, value } => {
                let node = self.resolve(node)?;
                set_inline_style_property(&mut state.dom.borrow_mut(), node, name, value)?;
            }
            Operation::Control {
                node,
                start,
                end,
                value,
            } => {
                let node = self.resolve(node)?;
                state
                    .dom
                    .borrow_mut()
                    .edit_control_value(node, *start, *end, value)?;
            }
            Operation::Override { key, value } => state
                .editing_overrides
                .borrow_mut()
                .insert(key.clone(), value.clone()),
            Operation::Split {
                node,
                offset,
                output,
            } => {
                let node = self.resolve(node)?;
                let tail = state.dom.borrow_mut().split_text(node, *offset)?;
                *output.borrow_mut() = Some(oryn_dom::TrackedNodeLease::new(&state.dom, tail)?);
            }
            Operation::Text {
                node,
                offset,
                count,
                data,
            } => {
                let node = self.resolve(node)?;
                state.dom.borrow_mut().replace_character_data(
                    node,
                    *offset,
                    *count,
                    data.clone(),
                )?;
            }
            Operation::Remove { node, ranges, private_ranges } => {
                let node = self.resolve(node)?;
                state.dom.borrow_mut().remove(node)?;
                synchronize_live_ranges(state)?;
                if let Some((ids,output))=private_ranges {
                    let dom=state.dom.borrow();
                    *output.borrow_mut()=Some(ids.iter().map(|id|Ok((*id,RangeRecord::from(dom.live_range(LiveRangeId(*id))?)))).collect::<Result<BTreeMap<_,_>,V8BindingError>>()?);
                }
                if let Some(output) = ranges {
                    *output.borrow_mut() = Some(state.ranges.borrow().clone());
                }
            }
            Operation::RemoveWithMarkers { node, markers } => {
                let node = self.resolve(node)?;
                let retained = {
                    let dom = state.dom.borrow();
                    let subtree = dom.arena().descendants_inclusive(node)?;
                    let mut retained = Vec::new();
                    retained
                        .try_reserve_exact(markers.len())
                        .map_err(|_| V8BindingError::Allocation("editing endpoint markers"))?;
                    for id in markers {
                        let range = dom.live_range(*id)?;
                        if range.start != range.end {
                            return Err(V8BindingError::Allocation(
                                "noncollapsed command endpoint",
                            ));
                        }
                        let point = range.start;
                        if subtree.contains(&point.container) {
                            retained.push((*id, point));
                        }
                    }
                    retained
                };
                state.dom.borrow_mut().remove(node)?;
                // Restore only independent command endpoint markers before
                // author work resumes, never the ordinary Selection/Range.
                for (id, point) in retained {
                    state.dom.borrow_mut().set_live_range(id, point, point)?;
                }
                synchronize_live_ranges(state)?;
            }
            Operation::Append {
                parent,
                node,
                reference,
                ranges,
            } => {
                let parent = self.resolve(parent)?;
                let node = self.resolve(node)?;
                let reference = reference
                    .as_ref()
                    .map(|node| self.resolve(node))
                    .transpose()?;
                let mut eligible=Vec::new();
                for update in ranges {
                    let current=if update.private_marker {
                        // A scoped marker may have been dropped while author
                        // work ran. A still-registered marker must exist in the
                        // native DOM; propagate corruption/retirement errors.
                        if !self.context.markers.borrow().contains(&update.id){continue}
                        let range=state.dom.borrow().live_range(LiveRangeId(update.id))?;
                        same_range(&range.into(),&update.expected)
                    }else{
                        state.ranges.borrow().get(&update.id).is_some_and(|range|same_range(range,&update.expected))
                    };
                    if current{eligible.push(update)}
                }
                state
                    .dom
                    .borrow_mut()
                    .insert_dom_node(parent, node, reference)?;
                synchronize_live_ranges(state)?;
                for update in eligible {
                    if update.private_marker {
                        let mut dom=state.dom.borrow_mut();
                        let range=dom.live_range(LiveRangeId(update.id))?;
                        dom.set_live_range(LiveRangeId(update.id),update.start.unwrap_or(range.start),update.end.unwrap_or(range.end))?;
                        continue;
                    }
                    let present = {
                        let mut records = state.ranges.borrow_mut();
                        if let Some(range) = records.get_mut(&update.id) {
                            if let Some(point) = update.start {
                                range.start_container = point.container;
                                range.start_offset = point.offset;
                            }
                            if let Some(point) = update.end {
                                range.end_container = point.container;
                                range.end_offset = point.offset;
                            }
                            true
                        } else {
                            false
                        }
                    };
                    if present {
                        persist_live_range(state, update.id)?;
                    }
                }
            }
        };
        synchronize_live_ranges(state)
    }
}

#[cfg(test)]
thread_local! {static FAIL_BEFORE_PREPARATION:Cell<bool>=const{Cell::new(false)};static FAIL_AFTER_PREPARATION:Cell<bool>=const{Cell::new(false)};}
#[cfg(test)]
thread_local! {static FAIL_PARAGRAPH_AFTER_MUTATION:Cell<bool>=const{Cell::new(false)};}
#[cfg(test)]
thread_local! {static FAIL_PARAGRAPH_PREIMAGE:Cell<bool>=const{Cell::new(false)};}
#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, code: &str) -> bool {
        super::super::test_eval::eval(scope, code).is_true()
    }
    #[test]
    fn private_move_markers_survive_wrapper_gc_and_are_released_on_drop() {
        super::super::tests::initialize_v8();
        let mut isolate=v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let realm=v8::Context::new(scope,Default::default());
        let scope=&mut v8::ContextScope::new(scope,realm);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><main id=host contenteditable><span id=source>x</span><span id=target></span></main></body>").into_runtime().unwrap()));
        let state=install_generated_bindings(scope,realm,dom,RealmId(0),1).unwrap();
        let (node,target)={let dom=state.dom.borrow();(dom.arena().get(dom.find_by_id("source").unwrap()).unwrap().children[0],dom.find_by_id("target").unwrap())};
        assert!(eval(scope,"const selected=document.createRange();selected.selectNodeContents(document.getElementById('source').firstChild);getSelection().removeAllRanges();getSelection().addRange(selected);true"));
        let owner=state.clone();
        let mut job=Continuation::new(state.clone(),"privateMarkerMove".into(),move|context|Box::pin(async move{
            let marker=context.marker(RangeRecord{start_container:node,start_offset:1,end_container:node,end_offset:1})?;
            let id=marker.id;
            assert!(!owner.ranges.borrow().contains_key(&id.0));
            collect_dead_ranges(&owner,true)?;
            assert_eq!(marker.range()?.start_container,node);
            context.move_preserving_ranges(node,target,0).await?;
            let range=marker.range()?;
            assert_eq!((range.start_container,range.start_offset),(node,1));
            drop(marker);
            assert!(!context.markers.borrow().contains(&id.0));
            assert!(owner.dom.borrow().live_range(id).is_err());
            Ok(())
        })).unwrap().unwrap();
        assert_eq!(job.poll(scope).unwrap(),Some(true));
    }

    #[test]
    fn list_fragment_markers_are_captured_before_connection_callbacks() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let realm = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, realm);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><main id=host contenteditable></main></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, realm, dom, RealmId(0), 1).unwrap();
        assert!(eval(
            scope,
            r#"
            globalThis.calls=0;globalThis.host=document.getElementById('host');
            globalThis.authored=document.createTextNode('author');
            class ListCallback extends HTMLElement {
              connectedCallback(){if(this.parentNode.localName==='li'){calls++;this.parentNode.append(authored)}}
            }
            customElements.define('list-callback',ListCallback);
            host.innerHTML='<list-callback>x</list-callback>';calls=0;
            const r=document.createRange();r.selectNodeContents(host);getSelection().removeAllRanges();getSelection().addRange(r);true
        "#
        ));
        assert!(eval(scope, "document.execCommand('insertOrderedList')"));
        let diagnostic = v8::String::new(scope, "JSON.stringify({calls,connected:authored.isConnected,html:host.innerHTML,selection:getSelection().toString(),anchor:getSelection().anchorOffset,focus:getSelection().focusOffset})").unwrap();
        let diagnostic = v8::Script::compile(scope, diagnostic, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_rust_string_lossy(scope);
        assert!(
            eval(
                scope,
                "calls>=1&&authored.isConnected&&host.textContent.includes('author')&&getSelection().toString()==='x'"
            ),
            "{diagnostic}"
        );
    }

    #[test]
    fn list_endpoint_markers_exclude_callback_added_content_and_leave_external_ranges_ordinary() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let realm = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, realm);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><main id=host contenteditable><div id=target></div></main></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, realm, dom, RealmId(0), 1).unwrap();
        assert!(eval(
            scope,
            r#"
          globalThis.host=document.getElementById('host');globalThis.target=document.getElementById('target');
          globalThis.script=document.createElement('script');script.type='text/plain';
          globalThis.moving=document.createTextNode('x');
          script.append(moving,document.createTextNode("target.append(document.createTextNode('author'))"));
          host.append(script);script.type='text/javascript';
          globalThis.external=document.createRange();external.selectNodeContents(moving);
          const selected=document.createRange();selected.selectNodeContents(moving);
          getSelection().removeAllRanges();getSelection().addRange(selected);true
        "#
        ));
        let (node, target) = {
            let dom = state.dom.borrow();
            let script = dom
                .arena()
                .get(dom.find_by_id("host").unwrap())
                .unwrap()
                .children[1];
            (
                dom.arena().get(script).unwrap().children[0],
                dom.find_by_id("target").unwrap(),
            )
        };
        let owner = state.clone();
        let mut job = Continuation::new(state.clone(), "endpointMove".into(), move |context| {
            Box::pin(async move {
                let start = context.marker(RangeRecord {
                    start_container: node,
                    start_offset: 0,
                    end_container: node,
                    end_offset: 0,
                })?;
                let end = context.marker(RangeRecord {
                    start_container: node,
                    start_offset: 1,
                    end_container: node,
                    end_offset: 1,
                })?;
                context
                    .append_with_endpoint_markers(target, node, &[&start, &end])
                    .await?;
                let a = start.range()?;
                let b = end.range()?;
                set_selection_boundaries(
                    &owner,
                    BoundaryPoint {
                        container: a.start_container,
                        offset: a.start_offset,
                    },
                    BoundaryPoint {
                        container: b.start_container,
                        offset: b.start_offset,
                    },
                )?;
                Ok(())
            })
        })
        .unwrap()
        .unwrap();
        assert_eq!(job.poll(scope).unwrap(), Some(true));
        assert!(eval(
            scope,
            "target.textContent==='authorx'&&getSelection().toString()==='x'&&external.toString()===''&&moving.parentNode===target"
        ));
    }

    #[test]
    fn collapsed_link_typing_stops_after_author_range_mutation_during_metadata_demand() {
        super::super::tests::initialize_v8();
        let mut isolate=v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let realm=v8::Context::new(scope,Default::default());
        let scope=&mut v8::ContextScope::new(scope,realm);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><main id=host contenteditable><a id=link href=x>abc</a><span id=target>yz</span></main></body>").into_runtime().unwrap()));
        let state=install_generated_bindings(scope,realm,dom,RealmId(0),1).unwrap();
        assert!(eval(scope,"globalThis.selected=document.createRange();selected.setStart(document.getElementById('link').firstChild,0);selected.collapse(true);getSelection().removeAllRanges();getSelection().addRange(selected);true"));
        let mut job=Continuation::new(state.clone(),"insertText".into(),move|context|
            Box::pin(async move { editing_algorithms::insert_editing_text(&context,"X").await })).unwrap().unwrap();
        // Poll the actual native future to its own metadata request, then let
        // author Range API mutation run before the owner serves that request.
        let waker=Waker::noop();let mut poll=PollContext::from_waker(waker);
        assert!(matches!(job.future.as_mut().unwrap().as_mut().poll(&mut poll),Poll::Pending));
        assert!(matches!(job.context.pending.borrow().as_ref().map(|request|&request.operation),Some(Operation::RetainMetadata{..})));
        let revision=state.selection_revision.get();
        assert!(eval(scope,"selected.setStart(document.getElementById('target').firstChild,1);selected.collapse(true);true"));
        assert_eq!(state.selection_revision.get(),revision,"Range mutation need not change Selection revision");
        assert!(matches!(job.poll(scope),Ok(Some(_))));
        assert!(eval(scope,"document.getElementById('host').innerHTML==='<a id=\"link\" href=\"x\">abc</a><span id=\"target\">yz</span>'&&getSelection().anchorNode===document.getElementById('target').firstChild&&getSelection().anchorOffset===1"));
    }

    #[test]
    fn private_marker_restore_does_not_overwrite_mutated_endpoints() {
        super::super::tests::initialize_v8();
        let mut isolate=v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let realm=v8::Context::new(scope,Default::default());
        let scope=&mut v8::ContextScope::new(scope,realm);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><main id=host contenteditable><span id=source>x</span><span id=target>yz</span></main></body>").into_runtime().unwrap()));
        let state=install_generated_bindings(scope,realm,dom,RealmId(0),1).unwrap();
        let (node,target)={let dom=state.dom.borrow();(dom.arena().get(dom.find_by_id("source").unwrap()).unwrap().children[0],dom.find_by_id("target").unwrap())};
        assert!(eval(scope,"const selected=document.createRange();selected.selectNodeContents(document.getElementById('source').firstChild);getSelection().removeAllRanges();getSelection().addRange(selected);true"));
        let owner=state.clone();
        let mut job=Continuation::new(state.clone(),"privateMarkerChanged".into(),move|context|Box::pin(async move{
            let marker=context.marker(RangeRecord{start_container:node,start_offset:1,end_container:node,end_offset:1})?;
            context.remove(node).await?;
            let expected=marker.range()?;
            // Model an endpoint mutation during the author-work interval,
            // after the producer snapshot and before native Append admission.
            let changed=BoundaryPoint{container:target,offset:0};
            owner.dom.borrow_mut().set_live_range(marker.id,changed,changed)?;
            context.request(Operation::Append{parent:context.lease(target)?,node:context.lease(node)?,reference:None,ranges:vec![RangeMove{id:marker.id.0,private_marker:true,expected,start:Some(BoundaryPoint{container:node,offset:1}),end:Some(BoundaryPoint{container:node,offset:1})}]}).await?;
            let current=marker.range()?;
            assert_eq!((current.start_container,current.start_offset),(target,0));
            Ok(())
        })).unwrap().unwrap();
        assert_eq!(job.poll(scope).unwrap(),Some(true));
    }

    #[test]
    fn list_endpoint_markers_respect_author_retarget_and_foreign_adoption() {
        super::super::tests::initialize_v8();
        for adopt in [false, true] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let realm = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, realm);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse(
                    "<body><main id=host contenteditable><div id=target>B</div></main></body>",
                )
                .into_runtime()
                .unwrap(),
            ));
            let state = install_generated_bindings(scope, realm, dom, RealmId(0), 1).unwrap();
            assert!(eval(
                scope,
                &format!(
                    r#"
                globalThis.host=document.getElementById('host');globalThis.target=document.getElementById('target');
                globalThis.foreign=document.implementation.createHTMLDocument();
                globalThis.script=document.createElement('script');script.type='text/plain';
                globalThis.moving=document.createTextNode('x');
                script.append(moving,document.createTextNode({callback:?}));host.append(script);script.type='text/javascript';
                const selected=document.createRange();selected.selectNodeContents(moving);
                getSelection().removeAllRanges();getSelection().addRange(selected);true
            "#,
                    callback = if adopt {
                        "foreign.body.append(foreign.adoptNode(moving))"
                    } else {
                        "getSelection().collapse(target.firstChild,1)"
                    }
                )
            ));
            let (node, target) = {
                let dom = state.dom.borrow();
                let script = dom
                    .arena()
                    .get(dom.find_by_id("host").unwrap())
                    .unwrap()
                    .children[1];
                (
                    dom.arena().get(script).unwrap().children[0],
                    dom.find_by_id("target").unwrap(),
                )
            };
            let owner = state.clone();
            let mut job =
                Continuation::new(state.clone(), "endpointAuthority".into(), move |context| {
                    Box::pin(async move {
                        let revision = owner.selection_revision.get();
                        let start = context.marker(RangeRecord {
                            start_container: node,
                            start_offset: 0,
                            end_container: node,
                            end_offset: 0,
                        })?;
                        let end = context.marker(RangeRecord {
                            start_container: node,
                            start_offset: 1,
                            end_container: node,
                            end_offset: 1,
                        })?;
                        context
                            .append_with_endpoint_markers(target, node, &[&start, &end])
                            .await?;
                        if owner.selection_revision.get() == revision {
                            let a = start.range()?;
                            let b = end.range()?;
                            set_selection_boundaries(
                                &owner,
                                BoundaryPoint {
                                    container: a.start_container,
                                    offset: a.start_offset,
                                },
                                BoundaryPoint {
                                    container: b.start_container,
                                    offset: b.start_offset,
                                },
                            )?;
                        }
                        Ok(())
                    })
                })
                .unwrap()
                .unwrap();
            if adopt {
                assert!(
                    matches!(job.poll(scope),Err(V8BindingError::Unsupported{member,..})if member=="AbortError")
                );
                assert!(eval(
                    scope,
                    "moving.ownerDocument===foreign&&moving.parentNode===foreign.body&&moving.data==='x'&&target.textContent==='B'"
                ));
            } else {
                assert_eq!(job.poll(scope).unwrap(), Some(true));
                assert!(eval(
                    scope,
                    "moving.parentNode===target&&getSelection().anchorNode===target.firstChild&&getSelection().anchorOffset===1&&getSelection().isCollapsed"
                ));
            }
        }
    }

    #[test]
    fn mutation_retry_and_undo_preserve_author_boundary_effects() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><div id=host contenteditable=true><span id=tail>B</span></div></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(
            scope,
            "globalThis.host=document.getElementById('host');globalThis.tail=document.getElementById('tail');globalThis.calls=0;globalThis.authorBoundary=()=>{calls++;const span=document.createElement('span');span.id='authored';span.textContent='author';host.append(span);tail.setAttribute('data-author','kept')};globalThis.script=document.createElement('script');script.id='script';script.type='text/plain';script.textContent='xglobalThis.authorBoundary()';host.append(script);script.type='text/javascript';const range=document.createRange();range.setStart(script.firstChild,0);range.setEnd(script.firstChild,1);getSelection().removeAllRanges();getSelection().addRange(range);true"
        ));
        let (script, tail) = {
            let dom = state.dom.borrow();
            (
                dom.arena()
                    .get(dom.find_by_id("script").unwrap())
                    .unwrap()
                    .children[0],
                dom.arena()
                    .get(dom.find_by_id("tail").unwrap())
                    .unwrap()
                    .children[0],
            )
        };
        let mut job =
            Continuation::new(state.clone(), "testNativeSegments".into(), move |context| {
                Box::pin(async move {
                    context.text(script, 0, 1, "").await?;
                    context.text(tail, 0, 1, "C").await?;
                    Ok(())
                })
            })
            .unwrap()
            .unwrap();
        FAIL_BEFORE_PREPARATION.with(|flag| flag.set(true));
        assert!(job.poll(scope).is_err());
        assert!(eval(
            scope,
            "calls===0&&script.textContent==='globalThis.authorBoundary()'&&tail.textContent==='B'"
        ));
        FAIL_AFTER_PREPARATION.with(|flag| flag.set(true));
        assert!(job.poll(scope).is_err());
        assert!(eval(scope, "calls===1&&tail.textContent==='B'"));
        assert_eq!(job.poll(scope).unwrap(), Some(true));
        assert!(eval(
            scope,
            "calls===1&&tail.textContent==='C'&&document.querySelectorAll('#authored').length===1"
        ));
        assert!(undo_edit(&state, false).unwrap());
        assert!(eval(
            scope,
            "calls===1&&script.textContent==='xglobalThis.authorBoundary()'&&tail.textContent==='B'&&tail.getAttribute('data-author')==='kept'&&document.querySelectorAll('#authored').length===1"
        ));
        assert!(undo_edit(&state, true).unwrap());
        assert!(eval(
            scope,
            "calls===1&&tail.textContent==='C'&&document.querySelectorAll('#authored').length===1"
        ));
    }
    #[test]
    fn exec_invocation_custody_survives_suspension_and_releases_on_terminal_owner_drop() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=host contenteditable>x</div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(
            scope,
            "getSelection().selectAllChildren(document.getElementById('host'));globalThis.other=document.implementation.createHTMLDocument('');true"
        ));
        let other = state.dom.borrow_mut().create(NodeKind::Document);
        let token = editing_commands::admit_invocation(scope, &state, state.document_root())
            .unwrap()
            .unwrap();
        assert!(
            editing_commands::admit_invocation(scope, &state, state.document_root())
                .unwrap()
                .is_none()
        );
        let independent = editing_commands::admit_invocation(scope, &state, other)
            .unwrap()
            .unwrap();
        drop(independent);
        let mut job = Continuation::new(state.clone(), "invocation".into(), move |_| {
            Box::pin(async { Ok(()) })
        })
        .unwrap()
        .unwrap();
        job.retain_invocation(Some(token.clone()));
        drop(token);
        assert!(job.current());
        assert!(job.before.is_some());
        assert!(job.result.is_none());
        // Observe the termination request at a real V8 safepoint before
        // checking native continuation suspension and retained baseline.
        let interruption = v8::String::new(scope, "0").unwrap();
        scope.terminate_execution();
        {
            v8::tc_scope!(let caught, scope);
            let result = v8::Script::compile(caught, interruption, None)
                .and_then(|script| script.run(caught));
            assert!(result.is_none());
            assert!(caught.has_terminated());
            assert_eq!(job.poll(caught).unwrap(), None);
            assert!(job.before.is_some());
            assert!(job.result.is_none());
        }
        scope.cancel_terminate_execution();
        assert!(
            editing_commands::admit_invocation(scope, &state, state.document_root())
                .unwrap()
                .is_none()
        );
        job.cancel();
        assert_eq!(job.poll(scope).unwrap(), Some(false));
        // Cancelled owner's admission remains until exact owner is dropped.
        drop(job);
        let token = editing_commands::admit_invocation(scope, &state, state.document_root())
            .unwrap()
            .unwrap();
        let generation = state.document_generation();
        state.document_generation.set(generation + 1);
        let replacement = editing_commands::admit_invocation(scope, &state, state.document_root())
            .unwrap()
            .unwrap();
        drop(token);
        assert!(
            editing_commands::admit_invocation(scope, &state, state.document_root())
                .unwrap()
                .is_none()
        );
        drop(replacement);
        state.document_generation.set(generation);
        let token = editing_commands::admit_invocation(scope, &state, state.document_root())
            .unwrap()
            .unwrap();
        let weak = Rc::downgrade(&token);
        drop(token);
        assert!(weak.upgrade().is_none());
        // Retirement makes the continuation stale; exact owner drop releases it.
        let token = editing_commands::admit_invocation(scope, &state, state.document_root())
            .unwrap()
            .unwrap();
        let mut job = Continuation::new(state.clone(), "retire".into(), move |_| {
            Box::pin(async { Ok(()) })
        })
        .unwrap()
        .unwrap();
        job.retain_invocation(Some(token.clone()));
        drop(token);
        state.retire_execution();
        job.cancel();
        assert_eq!(job.result, Some(false));
        drop(job);
    }
    #[test]
    fn exec_invocation_blocks_real_script_reentry_across_retry_and_navigation_retires_owner() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><div id=host contenteditable><span id=tail>B</span></div></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(
            scope,
            r#"
            globalThis.calls=0;globalThis.rejected=0;
            const host=document.getElementById('host');
            globalThis.script=document.createElement('script');script.type='text/plain';
            script.textContent="x calls++;if(document.execCommand('insertText',false,'bad'))throw Error('reentrant accepted');rejected++;";
            host.append(script);script.type='text/javascript';
            getSelection().collapse(document.getElementById('tail').firstChild,0);true
        "#
        ));
        let (script, tail) = {
            let dom = state.dom.borrow();
            (
                dom.arena()
                    .get(dom.find_by_id("host").unwrap())
                    .unwrap()
                    .children[1],
                dom.arena()
                    .get(dom.find_by_id("tail").unwrap())
                    .unwrap()
                    .children[0],
            )
        };
        let script_text = state.dom.borrow().arena().get(script).unwrap().children[0];
        let invocation = editing_commands::admit_invocation(scope, &state, state.document_root())
            .unwrap()
            .unwrap();
        let weak = Rc::downgrade(&invocation);
        let mut job = Continuation::new(
            state.clone(),
            "callback-invocation".into(),
            move |context| {
                Box::pin(async move {
                    context.text(script_text, 0, 1, "").await?;
                    context.text(tail, 0, 1, "C").await?;
                    Ok(())
                })
            },
        )
        .unwrap()
        .unwrap();
        job.retain_invocation(Some(invocation));
        FAIL_BEFORE_PREPARATION.with(|flag| flag.set(true));
        assert!(job.poll(scope).is_err());
        assert!(eval(scope, "calls===0&&rejected===0"));
        FAIL_AFTER_PREPARATION.with(|flag| flag.set(true));
        assert!(job.poll(scope).is_err());
        assert!(eval(
            scope,
            "calls===1&&rejected===1&&document.getElementById('tail').textContent==='B'"
        ));
        assert_eq!(job.poll(scope).unwrap(), Some(true));
        assert!(weak.upgrade().is_some());
        drop(job);
        assert!(weak.upgrade().is_none());
        assert!(eval(
            scope,
            "calls===1&&rejected===1&&document.execCommand('insertText',false,'z')"
        ));
        let invocation = editing_commands::admit_invocation(scope, &state, state.document_root())
            .unwrap()
            .unwrap();
        let cancelled = Rc::downgrade(&invocation);
        let mut cancelled_job =
            Continuation::new(state.clone(), "cancel-callback".into(), move |context| {
                Box::pin(async move {
                    context.text(tail, 0, 1, "Q").await?;
                    Ok(())
                })
            })
            .unwrap()
            .unwrap();
        cancelled_job.retain_invocation(Some(invocation));
        FAIL_BEFORE_PREPARATION.with(|flag| flag.set(true));
        assert!(cancelled_job.poll(scope).is_err());
        cancelled_job.cancel();
        drop(cancelled_job);
        assert!(cancelled.upgrade().is_none());
        assert!(eval(scope, "calls===1&&rejected===1"));
        // Retirement makes a retained continuation stale; cancellation drops its owner.
        let invocation = editing_commands::admit_invocation(scope, &state, state.document_root())
            .unwrap()
            .unwrap();
        let retired = Rc::downgrade(&invocation);
        let mut retired_job =
            Continuation::new(state.clone(), "retire-callback".into(), move |_| {
                Box::pin(async { Ok(()) })
            })
            .unwrap()
            .unwrap();
        retired_job.retain_invocation(Some(invocation));
        retired_job.cancel();
        drop(retired_job);
        assert!(retired.upgrade().is_none());
        let invocation = editing_commands::admit_invocation(scope, &state, state.document_root())
            .unwrap()
            .unwrap();
        let old = state.document_root();
        let replacement = state.dom.borrow_mut().create(NodeKind::Document);
        state
            .dom
            .borrow_mut()
            .arena_mut()
            .get_mut(replacement)
            .unwrap()
            .document_metadata = Some(Box::new(oryn_dom::DocumentMetadata::html()));
        state
            .replace_initial_document(replacement, state.document_generation() + 1)
            .unwrap();
        let new = editing_commands::admit_invocation(scope, &state, replacement)
            .unwrap()
            .unwrap();
        drop(invocation);
        assert!(
            editing_commands::admit_invocation(scope, &state, replacement)
                .unwrap()
                .is_none()
        );
        assert_ne!(old, state.document_root());
        drop(new);
    }
    #[test]
    fn cancellation_releases_suspended_operation_without_running_script() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=host contenteditable=true>x</div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(
            scope,
            "const range=document.createRange();range.selectNodeContents(document.getElementById('host'));getSelection().removeAllRanges();getSelection().addRange(range);true"
        ));
        let node = {
            let dom = state.dom.borrow();
            dom.arena()
                .get(dom.find_by_id("host").unwrap())
                .unwrap()
                .children[0]
        };
        let mut job = Continuation::new(state.clone(), "cancel".into(), move |context| {
            Box::pin(async move {
                context.text(node, 0, 1, "y").await?;
                context.text(node, 0, 1, "z").await?;
                Ok(())
            })
        })
        .unwrap()
        .unwrap();
        FAIL_BEFORE_PREPARATION.with(|flag| flag.set(true));
        assert!(job.poll(scope).is_err());
        job.cancel();
        assert_eq!(job.poll(scope).unwrap(), Some(false));
        assert!(job.context.pending.borrow().is_none());
        assert!(job.context.tracked.borrow().is_empty());
        assert!(eval(
            scope,
            "document.getElementById('host').textContent==='y'"
        ));
        let mut oversized = Continuation::new(state.clone(), "capacity".into(), move |context| {
            Box::pin(async move {
                context
                    .text(node, 0, 1, "z".repeat(UndoHistory::MAX_BYTES))
                    .await
            })
        })
        .unwrap()
        .unwrap();
        assert!(
            matches!(oversized.poll(scope),Err(V8BindingError::Unsupported{member,..})if member=="QuotaExceededError")
        );
        assert_eq!(oversized.poll(scope).unwrap(), Some(false));
        assert!(eval(
            scope,
            "document.getElementById('host').textContent==='y'"
        ));
    }
    #[test]
    fn range_preserving_move_does_not_overwrite_callback_range_changes() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><div id=host contenteditable=true><span id=tail>B</span></div></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(
            scope,
            "globalThis.host=document.getElementById('host');globalThis.tail=document.getElementById('tail');globalThis.calls=0;globalThis.script=document.createElement('script');script.id='script';script.type='text/plain';globalThis.moving=document.createTextNode('x');script.append(moving,document.createTextNode('globalThis.calls++;globalThis.observed.selectNodeContents(tail)'));host.append(script);script.type='text/javascript';globalThis.observed=document.createRange();observed.selectNodeContents(moving);getSelection().removeAllRanges();getSelection().addRange(observed);true"
        ));
        let (node, target) = {
            let dom = state.dom.borrow();
            (
                dom.arena()
                    .get(dom.find_by_id("script").unwrap())
                    .unwrap()
                    .children[0],
                dom.find_by_id("tail").unwrap(),
            )
        };
        let mut job = Continuation::new(state.clone(), "rangeMove".into(), move |context| {
            Box::pin(async move { context.move_preserving_ranges(node, target, 0).await })
        })
        .unwrap()
        .unwrap();
        assert_eq!(job.poll(scope).unwrap(), Some(true));
        assert!(eval(
            scope,
            "calls===1&&moving.parentNode===tail&&observed.startContainer===tail&&observed.endContainer===tail"
        ));
    }
    #[test]
    fn callback_adoption_aborts_native_suffix_without_restoring_author_tree() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><div id=host contenteditable=true><span id=tail>B</span></div></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(
            scope,
            "globalThis.host=document.getElementById('host');globalThis.tail=document.getElementById('tail');globalThis.foreign=document.implementation.createHTMLDocument();globalThis.adoptions=0;globalThis.script=document.createElement('script');script.id='script';script.type='text/plain';script.textContent='xglobalThis.adoptions++;foreign.body.append(foreign.adoptNode(tail))';host.append(script);script.type='text/javascript';const range=document.createRange();range.selectNodeContents(script.firstChild);getSelection().removeAllRanges();getSelection().addRange(range);true"
        ));
        let (script, tail) = {
            let dom = state.dom.borrow();
            (
                dom.arena()
                    .get(dom.find_by_id("script").unwrap())
                    .unwrap()
                    .children[0],
                dom.arena()
                    .get(dom.find_by_id("tail").unwrap())
                    .unwrap()
                    .children[0],
            )
        };
        let mut job = Continuation::new(state.clone(), "adoptedSuffix".into(), move |context| {
            Box::pin(async move {
                context.text(script, 0, 1, "").await?;
                context.text(tail, 0, 1, "wrong").await
            })
        })
        .unwrap()
        .unwrap();
        assert!(
            matches!(job.poll(scope),Err(V8BindingError::Unsupported{member,..})if member=="AbortError")
        );
        assert_eq!(job.poll(scope).unwrap(), Some(true));
        assert!(eval(
            scope,
            "adoptions===1&&tail.ownerDocument===foreign&&tail.parentNode===foreign.body&&tail.textContent==='B'"
        ));
        assert!(undo_edit(&state, false).unwrap());
        assert!(eval(
            scope,
            "adoptions===1&&tail.parentNode===foreign.body&&tail.textContent==='B'&&script.textContent.startsWith('x')"
        ));
    }
    #[test]
    fn formatting_unwrap_prepares_changed_script_parent() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=host contenteditable=true></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(
            scope,
            "globalThis.calls=0;globalThis.script=document.createElement('script');script.type='text/plain';globalThis.span=document.createElement('span');span.textContent='globalThis.calls++';script.append(span);document.getElementById('host').append(script);script.type='text/javascript';const range=document.createRange();range.selectNodeContents(span);getSelection().removeAllRanges();getSelection().addRange(range);true"
        ));
        let mut job = Continuation::new(state.clone(), "removeFormat".into(), move |context| {
            Box::pin(async move { editing_algorithms::remove_selection_format(&context).await })
        })
        .unwrap()
        .unwrap();
        assert_eq!(job.poll(scope).unwrap(), Some(true));
        assert!(eval(
            scope,
            "calls===1&&span.parentNode===null&&script.firstChild.nodeType===3"
        ));
    }
    #[test]
    fn oversized_native_growth_rejects_before_publication_and_keeps_prior_inverse() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=host contenteditable=true>x</div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(
            scope,
            "const r=document.createRange();r.selectNodeContents(document.getElementById('host'));getSelection().removeAllRanges();getSelection().addRange(r);true"
        ));
        let (host, text) = {
            let dom = state.dom.borrow();
            let host = dom.find_by_id("host").unwrap();
            (host, dom.children(host).unwrap()[0])
        };
        for detached in [false, true] {
            let mut job =
                Continuation::new(state.clone(), "growthAdmission".into(), move |context| {
                    Box::pin(async move {
                        context.text(text, 0, 1, "y").await?;
                        if detached {
                            let subtree = {
                                let mut dom = context.state.dom.borrow_mut();
                                let subtree = dom.create_element("span");
                                let data = dom.create_text("a".repeat(UndoHistory::MAX_BYTES));
                                dom.append_child(subtree, data)?;
                                subtree
                            };
                            context.append(host, subtree, None).await
                        } else {
                            context
                                .attribute(
                                    host,
                                    None,
                                    "data-large",
                                    "a".repeat(UndoHistory::MAX_BYTES),
                                )
                                .await
                        }
                    })
                })
                .unwrap()
                .unwrap();
            assert!(
                matches!(job.poll(scope),Err(V8BindingError::Unsupported{member,..})if member=="QuotaExceededError")
            );
            assert_eq!(job.poll(scope).unwrap(), Some(true));
            assert!(eval(
                scope,
                "document.getElementById('host').textContent==='y'&&!document.getElementById('host').hasAttribute('data-large')"
            ));
            assert!(undo_edit(&state, false).unwrap());
            assert!(eval(
                scope,
                "document.getElementById('host').textContent==='x'"
            ));
        }
    }

    #[test]
    fn editing_html_domstrings_preserve_units_for_rich_plaintext_and_controls() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=rich contenteditable=true></div><div id=plain contenteditable=plaintext-only></div><input id=control></body>").into_runtime().unwrap()));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(
            scope,
            r#"(()=>{
   for(const value of ['a\uD800b','\uDFFF','\u{F0000}oryn-wtf16:[65]']){
    for(const id of ['rich','plain','control']){
     const host=document.getElementById(id);host.focus();
     if(id==='control'){host.value='';host.select()}else{host.textContent='';getSelection().selectAllChildren(host)}
     if(!document.execCommand('insertHTML',false,'<b>'+value+'</b>'))throw Error('HTML edit rejected '+id);
     const actual=id==='control'?host.value:host.textContent;
     if(actual!==value)throw Error('HTML edit changed code units '+id);
    }
   }
   return true;
  })()"#
        ));
    }
#[test]
fn paragraph_split_author_moved_halves_preserve_original_identity_and_redo_cycle() {
    super::super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let realm = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, realm);
    let dom = Rc::new(RefCell::new(oryn_dom::html::parse("<body><main id=host contenteditable></main></body>").into_runtime().unwrap()));
    let state = install_generated_bindings(scope, realm, dom, RealmId(0), 1).unwrap();
    for case in 0..4 {
        assert!(eval(scope, &format!(r#"
            globalThis.host=document.getElementById('host');
            host.innerHTML={html};
            globalThis.left=host.querySelector('div');
            const range=document.createRange();range.setStart(left.firstChild,3);range.collapse(true);
            getSelection().removeAllRanges();getSelection().addRange(range);
            document.execCommand('insertParagraph');
            globalThis.right=left.nextSibling;
            {movement};true
        "#,
            html=match case {0=>"'<div>abcdef</div><p>ghi</p>'",1=>"'<p>abc</p><div>defghi</div>'",_=>"'<div>abcdef</div>'"},
            movement=match case {0=>"host.append(right)",1=>"host.insertBefore(left,host.firstChild)",2=>"left.insertBefore(right,left.firstChild)",_=>"right.append(left)"})));
        assert_eq!(editing_commands::paragraph_history::dispatch(scope,&state,false).unwrap(),true);
        let expected=match case {0=>"host.firstChild===left&&left.textContent==='abcdef'&&host.lastChild.textContent==='ghi'",1=>"host.firstChild===left&&left.textContent==='defghi'&&host.lastChild.textContent==='abc'",2=>"host.firstChild===left&&host.childNodes.length===1&&left.textContent==='abcdef'",_=>"host.innerHTML===''&&left.textContent==='abcdef'"};
        assert!(eval(scope,expected));
        assert!(editing_commands::paragraph_history::dispatch(scope,&state,true).unwrap());
        assert!(editing_commands::paragraph_history::dispatch(scope,&state,false).unwrap());
        assert!(eval(scope,expected));
        *state.undo_history.borrow_mut()=UndoHistory::default();
    }
}

#[test]
fn paragraph_split_pending_inverse_rejects_opposite_direction_without_losing_receipt() {
    super::super::tests::initialize_v8();
    let mut isolate=v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let realm=v8::Context::new(scope,Default::default());
    let scope=&mut v8::ContextScope::new(scope,realm);
    let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=host contenteditable><p>abcdef</p></div></body>").into_runtime().unwrap()));
    let state=install_generated_bindings(scope,realm,dom,RealmId(0),1).unwrap();
    assert!(eval(scope,"globalThis.host=document.getElementById('host');const r=document.createRange();r.setStart(host.firstChild.firstChild,3);r.collapse(true);getSelection().removeAllRanges();getSelection().addRange(r);document.execCommand('insertParagraph')"));
    let unit=state.undo_history.borrow().undo.back().cloned().unwrap();
    FAIL_AFTER_PREPARATION.with(|flag|flag.set(true));
    assert!(editing_commands::paragraph_history::dispatch(scope,&state,false).is_err());
    assert!(unit.native_progress.borrow().as_ref().is_some_and(|p|!p.redo));
    assert!(!editing_commands::paragraph_history::dispatch(scope,&state,true).unwrap());
    assert!(Rc::ptr_eq(&state.undo_history.borrow().undo.back().unwrap().identity,&unit.identity));
    assert!(editing_commands::paragraph_history::dispatch(scope,&state,false).is_err());
    // Retry the existing owner, never construct a new inverse cursor.
    editing_commands::paragraph_history::ready_for_test(&state);
    assert_eq!(editing_commands::poll(scope,&state).unwrap(),1);
    assert!(eval(scope,"host.textContent==='abcdef'&&host.childNodes.length===1"));
    assert!(unit.native_progress.borrow().is_none());
    assert!(editing_commands::paragraph_history::dispatch(scope,&state,true).unwrap());
    assert!(editing_commands::paragraph_history::dispatch(scope,&state,false).unwrap());
    assert!(eval(scope,"host.textContent==='abcdef'&&host.childNodes.length===1"));
}

#[test]
fn paragraph_terminal_failure_and_compensation_retain_actual_native_prefix() {
    super::super::tests::initialize_v8();
    let mut isolate=v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let realm=v8::Context::new(scope,Default::default());
    let scope=&mut v8::ContextScope::new(scope,realm);
    let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=host contenteditable><div>abcdef</div></div></body>").into_runtime().unwrap()));
    let state=install_generated_bindings(scope,realm,dom,RealmId(0),1).unwrap();
    assert!(eval(scope,"globalThis.host=document.getElementById('host');getSelection().collapse(host.firstChild.firstChild,3);document.execCommand('insertParagraph');globalThis.left=host.firstChild;globalThis.right=left.nextSibling;true"));
    let unit=state.undo_history.borrow().undo.back().cloned().unwrap();
    FAIL_PARAGRAPH_AFTER_MUTATION.with(|flag|flag.set(true));
    if let Err(error)=editing_commands::paragraph_history::dispatch(scope,&state,false) { assert_paragraph_expected_error(error); }
    for _ in 0..128 {
        editing_commands::paragraph_history::ready_for_test(&state);
        if let Err(error)=editing_commands::poll(scope,&state) { assert_paragraph_expected_error(error); }
        if unit.paragraph_failed.borrow().is_some(){break;}
    }
    let original_count=unit.paragraph_failed.borrow().as_ref().expect("actual terminal failure retained").segments.len();
    assert!(original_count>0);
    assert!(Rc::ptr_eq(&state.undo_history.borrow().undo.back().unwrap().identity,&unit.identity));
    FAIL_PARAGRAPH_AFTER_MUTATION.with(|flag|flag.set(true));
    if let Err(error)=editing_commands::paragraph_history::dispatch(scope,&state,false) { assert_paragraph_expected_error(error); }
    for _ in 0..128 {
        editing_commands::paragraph_history::ready_for_test(&state);
        if let Err(error)=editing_commands::poll(scope,&state) { assert_paragraph_expected_error(error); }
        if unit.paragraph_failed.borrow().as_ref().is_some_and(|failed|failed.segments.len()>original_count){break;}
    }
    assert!(unit.paragraph_failed.borrow().as_ref().unwrap().segments.len()>original_count);
    assert!(unit.native_progress.borrow().as_ref().is_some_and(|progress|!progress.redo));
    if let Err(error)=editing_commands::paragraph_history::dispatch(scope,&state,false) { assert_paragraph_expected_error(error); }
    for _ in 0..128 {
        editing_commands::paragraph_history::ready_for_test(&state);
        if let Err(error)=editing_commands::poll(scope,&state) { assert_paragraph_expected_error(error); }
        if unit.paragraph_failed.borrow().is_none(){break;}
    }
    assert!(unit.paragraph_failed.borrow().is_none());
    assert!(eval(scope,"host.firstChild===left&&left.nextSibling===right&&left.textContent==='abc'&&right.textContent==='def'"));
    assert!(editing_commands::paragraph_history::dispatch(scope,&state,false).unwrap());
    assert!(eval(scope,"host.firstChild===left&&host.childNodes.length===1&&left.textContent==='abcdef'"));
}


#[test]
fn paragraph_preimage_failure_before_mutation_retries_changed_halves() {
    super::super::tests::initialize_v8();
    let mut isolate=v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let realm=v8::Context::new(scope,Default::default());
    let scope=&mut v8::ContextScope::new(scope,realm);
    let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=host contenteditable><div>abcdef</div><p>tail</p></div></body>").into_runtime().unwrap()));
    let state=install_generated_bindings(scope,realm,dom,RealmId(0),1).unwrap();
    assert!(eval(scope,"globalThis.host=document.getElementById('host');getSelection().collapse(host.firstChild.firstChild,3);document.execCommand('insertParagraph');globalThis.left=host.firstChild;globalThis.right=left.nextSibling;true"));
    let unit=state.undo_history.borrow().undo.back().cloned().unwrap();
    FAIL_PARAGRAPH_PREIMAGE.with(|flag|flag.set(true));
    match editing_commands::paragraph_history::dispatch(scope,&state,false) {
        Err(V8BindingError::Allocation("injected paragraph preimage failure"))=>{},
        other=>panic!("unexpected preimage result: {other:?}"),
    }
    assert!(unit.native_progress.borrow().as_ref().is_some_and(|progress|!progress.redo));
    assert!(unit.paragraph_failed.borrow().is_none());
    assert!(eval(scope,"left.textContent==='abc'&&right.textContent==='def'&&left.nextSibling===right"));
    assert!(eval(scope,"host.append(right);globalThis.authorNode=document.createElement('span');authorNode.textContent='authored';host.append(authorNode);true"));
    editing_commands::paragraph_history::ready_for_test(&state);
    for _ in 0..128 {
        if let Err(error)=editing_commands::poll(scope,&state) {assert_paragraph_expected_error(error);}
        if unit.native_progress.borrow().is_none(){break;}
        editing_commands::paragraph_history::ready_for_test(&state);
    }
    assert!(unit.native_progress.borrow().is_none());
    assert!(eval(scope,"host.firstChild===left&&left.textContent==='abcdef'&&authorNode.isConnected&&authorNode.textContent==='authored'"));
    assert!(state.undo_history.borrow().redo.back().is_some_and(|current|Rc::ptr_eq(&current.identity,&unit.identity)));
}


#[test]
fn paragraph_undo_callback_growth_stays_outside_native_inverse() {
    super::super::tests::initialize_v8();
    let mut isolate=v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let realm=v8::Context::new(scope,Default::default());
    let scope=&mut v8::ContextScope::new(scope,realm);
    let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=host contenteditable><div>abcdef</div></div></body>").into_runtime().unwrap()));
    let state=install_generated_bindings(scope,realm,dom,RealmId(0),1).unwrap();
    assert!(eval(scope,r#"
        globalThis.host=document.getElementById('host');getSelection().collapse(host.firstChild.firstChild,3);
        document.execCommand('insertParagraph');globalThis.left=host.firstChild;globalThis.right=left.nextSibling;
        globalThis.calls=0;globalThis.script=document.createElement('script');script.type='text/plain';
        script.appendChild(document.createTextNode("calls++;globalThis.authorAdded=document.createElement('span');authorAdded.textContent='q'.repeat(4096);host.append(authorAdded)"));
        right.append(script);script.type='text/javascript';calls===0
    "#));
    let unit=state.undo_history.borrow().undo.back().cloned().unwrap();
    let pending=match editing_commands::paragraph_history::dispatch(scope,&state,false) {
        Ok(done)=>{assert!(done);false},Err(error)=>{assert_paragraph_expected_error(error);true}
    };
    if pending {
        for _ in 0..128 {
            editing_commands::paragraph_history::ready_for_test(&state);
            if let Err(error)=editing_commands::poll(scope,&state) { assert_paragraph_expected_error(error); }
            if state.undo_history.borrow().undo.back().is_none_or(|unit|unit.native_progress.borrow().is_none()){break;}
        }
    }
    assert!(eval(scope,"calls===1&&authorAdded.isConnected&&authorAdded.textContent.length===4096&&left.textContent.startsWith('abcdef')"));
    assert!(state.undo_history.borrow().redo.back().is_some_and(|current|Rc::ptr_eq(&current.identity,&unit.identity)));
    assert!(editing_commands::paragraph_history::dispatch(scope,&state,true).unwrap());
    assert!(eval(scope,"calls===1&&authorAdded.isConnected&&authorAdded.textContent.length===4096"));
}


#[test]
fn paragraph_owner_withdrawal_preserves_actual_committed_prefix() {
    super::super::tests::initialize_v8();
    let mut isolate=v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let realm=v8::Context::new(scope,Default::default());
    let scope=&mut v8::ContextScope::new(scope,realm);
    let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=host contenteditable><div>abcdef</div></div></body>").into_runtime().unwrap()));
    let state=install_generated_bindings(scope,realm,dom,RealmId(0),1).unwrap();
    assert!(eval(scope,"globalThis.host=document.getElementById('host');getSelection().collapse(host.firstChild.firstChild,3);document.execCommand('insertParagraph');globalThis.left=host.firstChild;true"));
    let unit=state.undo_history.borrow().undo.back().cloned().unwrap();
    FAIL_BEFORE_PREPARATION.with(|flag|flag.set(true));
    match editing_commands::paragraph_history::dispatch(scope,&state,false) {
        Err(V8BindingError::Allocation("injected editing author-boundary failure"))=>{},
        other=>panic!("unexpected owner withdrawal setup: {other:?}"),
    }
    assert!(eval(scope,"host.textContent==='abcdef'"));
    assert!(eval(scope,"getSelection().collapse(left.firstChild,0);document.execCommand('insertText',false,'authored');left.textContent.startsWith('authored')"));
    assert!(!Rc::ptr_eq(&state.undo_history.borrow().undo.back().unwrap().identity,&unit.identity));
    editing_commands::paragraph_history::ready_for_test(&state);
    if let Err(error)=editing_commands::poll(scope,&state) { assert_paragraph_expected_error(error); }
    assert!(unit.paragraph_failed.borrow().as_ref().is_some_and(|failed|!failed.segments.is_empty()));
    assert!(unit.native_progress.borrow().as_ref().is_some_and(|progress|!progress.redo));
    assert!(state.undo_history.borrow().undo.iter().any(|current|Rc::ptr_eq(&current.identity,&unit.identity)));
}


fn assert_paragraph_expected_error(error:V8BindingError) {
    assert!(matches!(error,V8BindingError::Allocation("native editing suspended"|
        "injected paragraph terminal mutation failure"|"injected paragraph preimage failure")),
        "unexpected paragraph control error: {error:?}");
}


#[test]
fn paragraph_failed_recovery_composes_recipes_and_retains_noncurrent_history_owner() {
    super::super::tests::initialize_v8();
    let mut isolate=v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let realm=v8::Context::new(scope,Default::default());
    let scope=&mut v8::ContextScope::new(scope,realm);
    let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=host contenteditable><p>abcdef</p></div></body>").into_runtime().unwrap()));
    let state=install_generated_bindings(scope,realm,dom,RealmId(0),1).unwrap();
    assert!(eval(scope,"const host=document.getElementById('host');const r=document.createRange();r.setStart(host.firstChild.firstChild,3);r.collapse(true);getSelection().removeAllRanges();getSelection().addRange(r);document.execCommand('insertParagraph')"));
    let unit=state.undo_history.borrow().undo.back().cloned().unwrap();
    *unit.native_progress.borrow_mut()=Some(NativeUndoProgress{redo:false,next:0});
    let original=unit.native_segments.first().cloned().unwrap();
    editing_commands::paragraph_history::receipt::retain_failed(&state,&unit,vec![original.clone()],&V8BindingError::Allocation("first failure"));
    let mut newer=unit.clone();newer.identity=Rc::new(());
    state.undo_history.borrow_mut().undo.push_back(newer);
    assert!(!editing_commands::paragraph_history::receipt::undo_owner_current(&state,&unit));
    editing_commands::paragraph_history::receipt::retain_failed(&state,&unit,vec![original],&V8BindingError::Allocation("compensation failure"));
    assert_eq!(unit.paragraph_failed.borrow().as_ref().unwrap().segments.len(),2);
    assert_eq!(unit.paragraph_failed.borrow().as_ref().unwrap().error,"V8 failed to allocate compensation failure");
    assert!(unit.native_progress.borrow().as_ref().is_some_and(|p|!p.redo));
    assert!(editing_commands::paragraph_history::recovered(&state,&unit).is_err());
    assert_eq!(unit.paragraph_failed.borrow().as_ref().unwrap().segments.len(),2);
    state.undo_history.borrow_mut().undo.pop_back();
    assert!(editing_commands::paragraph_history::receipt::undo_owner_current(&state,&unit));
}

#[test]
fn paragraph_retirement_is_idempotent_and_preserves_other_accounted_units() {
    super::super::tests::initialize_v8();
    let mut isolate=v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let realm=v8::Context::new(scope,Default::default());
    let scope=&mut v8::ContextScope::new(scope,realm);
    let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=host contenteditable><p>abcdef</p></div></body>").into_runtime().unwrap()));
    let state=install_generated_bindings(scope,realm,dom,RealmId(0),1).unwrap();
    assert!(eval(scope,"const host=document.getElementById('host');const r=document.createRange();r.setStart(host.firstChild.firstChild,3);r.collapse(true);getSelection().removeAllRanges();getSelection().addRange(r);document.execCommand('insertParagraph')"));
    let unit=state.undo_history.borrow().undo.back().cloned().unwrap();
    *unit.native_progress.borrow_mut()=Some(NativeUndoProgress{redo:false,next:0});
    unit.paragraph_reservation.admit(&state,&unit,32).unwrap();
    editing_commands::paragraph_history::receipt::retain_failed(&state,&unit,
        vec![unit.native_segments.first().cloned().unwrap()],&V8BindingError::Allocation("retire control"));
    unit.paragraph_reservation.admit(&state,&unit,16).unwrap();
    let mut other=unit.clone();
    other.identity=Rc::new(());
    other.paragraph_reservation=Default::default();
    other.paragraph_failed=Default::default();
    other.native_progress=Default::default();
    let other_bytes=undo_unit_bytes(&other);
    {
        let mut history=state.undo_history.borrow_mut();
        history.bytes=history.bytes.checked_add(other_bytes).unwrap();
        history.redo.push_back(other.clone());
    }
    state.control_tasks_retired.set(true);
    for _ in 0..2 {
        editing_commands::paragraph_history::receipt::retire_failed(&state,&unit);
        let history=state.undo_history.borrow();
        assert_eq!(history.bytes,other_bytes);
        assert!(history.undo.is_empty());
        assert_eq!(history.redo.len(),1);
        assert!(Rc::ptr_eq(&history.redo[0].identity,&other.identity));
        assert_eq!(unit.paragraph_reservation.bytes.get(),0);
        assert!(unit.paragraph_failed.borrow().is_none());
        assert!(unit.native_progress.borrow().is_none());
    }
}

}
