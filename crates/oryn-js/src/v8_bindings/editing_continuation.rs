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
    Remove {
        node: oryn_dom::TrackedNodeLease,
        ranges: Option<RetainedRanges>,
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
struct Request {
    operation: Operation,
    prepared: RefCell<Option<Preparation>>,
    mutated: Cell<bool>,
    recorded: Cell<bool>,
    error: RefCell<Option<V8BindingError>>,
    reply: RefCell<Option<Result<(), V8BindingError>>>,
}
pub(super) struct Marker {
    owner: Rc<V8BindingState>,
    id: oryn_dom::LiveRangeId,
}
impl Marker {
    pub(super) fn range(&self) -> Result<RangeRecord, V8BindingError> {
        Ok(self.owner.dom.borrow().live_range(self.id)?.into())
    }
}
impl Drop for Marker {
    fn drop(&mut self) {
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
}
impl Drop for Context {
    fn drop(&mut self) {
        for job in self.clones.get_mut().drain(..) {
            job.background.set(true);
        }
    }
}
impl Context {
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
        self.state.range_clone_completions.borrow_mut().push(job);
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
        Ok(Marker {
            owner: self.state.clone(),
            id,
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
        let mut roots = BTreeSet::from([host]);
        for lease in self.extra_roots.borrow().iter() {
            let (dom, node) = lease.current();
            if Rc::ptr_eq(&dom, &self.state.dom)
                && dom.borrow().arena().node_document(node)? == Some(self.state.document_root())
            {
                roots.insert(node);
            }
        }
        for (original, lease) in self.tracked.borrow().iter() {
            if !self.admitted.borrow().contains(original) {
                continue;
            }
            let (dom, node) = lease.current();
            if !Rc::ptr_eq(&dom, &self.state.dom) {
                continue;
            }
            let dom = dom.borrow();
            if dom.arena().node_document(node)? != Some(self.state.document_root())
                || dom.is_connected(node)?
            {
                continue;
            }
            if let Some(root) = dom.arena().ancestors(node)?.last() {
                roots.insert(*root);
            } else {
                roots.insert(node);
            }
        }
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
        let removed = Rc::new(RefCell::new(None));
        if old_parent.is_some() {
            self.request(Operation::Remove {
                node: self.lease(node)?,
                ranges: Some(removed.clone()),
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
    async fn request(&self, operation: Operation) -> Result<(), V8BindingError> {
        let request = Rc::new(Request {
            operation,
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
    pub(super) fn defer_history(&mut self) {
        self.publish_history = false;
    }
    pub(super) fn take_segments(&mut self) -> Vec<editing_inverse::Segment> {
        std::mem::take(&mut self.segments)
    }
    pub(super) fn cancel(&mut self) {
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
    fn segment(&mut self) -> Result<(), V8BindingError> {
        let state = &self.context.state;
        let (_, host) = self.host.current();
        editing_snapshot_budget(state, &[host])?;
        let after = self.context.snapshot(host)?;
        if let Some(before) = self.before.as_ref() {
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
                if !request.mutated.get() {
                    // All native-only work preceding this await becomes an independent
                    // inverse before any script may observe it.
                    if !request.recorded.get() {
                        self.segment()?;
                        request.recorded.set(true);
                    }
                    let result = match self.prepare(scope, &request) {
                        Ok(()) => self.mutate(&request),
                        Err(error @ V8BindingError::Allocation(_)) => return Err(error),
                        Err(error) => Err(error),
                    };
                    request.mutated.set(true);
                    *request.error.borrow_mut() = result.err();
                    // Snapshot failure retains the mutated flag and retries capture only.
                    request.recorded.set(false);
                }
                if !request.recorded.get() {
                    self.segment()?;
                    request.recorded.set(true);
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
                        cross_document: None,
                    },
                )?;
            }
            if self.publish_history {
                self.segments.clear();
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
                    capture_editing_snapshot(state, node)?.serialized_bytes
                }
            }
            _ => 0,
        };
        let parent_growth = if let Operation::Append { parent, .. } = &request.operation {
            let parent = self.resolve(parent)?;
            if self.context.admitted.borrow().contains(&parent) {
                0
            } else {
                capture_editing_snapshot(state, parent)?.serialized_bytes
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
        let preparation = match &request.operation {
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
            Operation::Remove { node, ranges } => {
                let node = self.resolve(node)?;
                state.dom.borrow_mut().remove(node)?;
                synchronize_live_ranges(state)?;
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
                let eligible = ranges
                    .iter()
                    .filter(|update| {
                        state
                            .ranges
                            .borrow()
                            .get(&update.id)
                            .is_some_and(|range| same_range(range, &update.expected))
                    })
                    .collect::<Vec<_>>();
                state
                    .dom
                    .borrow_mut()
                    .insert_dom_node(parent, node, reference)?;
                synchronize_live_ranges(state)?;
                for update in eligible {
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
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, code: &str) -> bool {
        super::super::test_eval::eval(scope, code).is_true()
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
}
