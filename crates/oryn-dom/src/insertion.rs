//! Retained native insertion suboperations. Cursors advance at the actual
//! publication boundary; a failed suffix never repeats an admitted prefix.
use super::*;

pub(crate) struct OwnershipPublication {
    arena: ArenaId,
    root: Option<NodeId>,
    pending: std::collections::VecDeque<(NodeId, Option<NodeId>)>,
    phase: u8,
}
impl OwnershipPublication {
    pub(crate) fn new(arena: ArenaId, previous: Vec<(NodeId, Option<NodeId>)>) -> Self {
        Self {
            arena,
            root: previous.first().map(|(node, _)| *node),
            pending: previous.into(),
            phase: 0,
        }
    }
    pub(crate) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        if dom.arena.id() != self.arena {
            return Err(DomError::MigrationChanged);
        }
        while let Some(&(node, document)) = self.pending.front() {
            match self.phase {
                0 => {
                    if dom.arena.node_document(node)? == document {
                        self.pending.pop_front();
                        continue;
                    }
                    self.phase = 1;
                }
                1 => {
                    if document.is_some() {
                        dom.rebind_adopted_custom_registry(node, self.root)?;
                    }
                    self.phase = 2;
                }
                2 => {
                    dom.note_custom_registry_document(node)?;
                    self.phase = 3;
                }
                3 => {
                    let record = dom.arena.get_mut(node)?;
                    record.permission_granted = false;
                    record.interest_source = false;
                    record.media_selector_state = None;
                    record.interest_target = false;
                    self.phase = 4;
                }
                4 => {
                    dom.record_mutation(Mutation::DocumentOwnership {
                        node,
                        previous_document: document,
                    });
                    self.pending.pop_front();
                    self.phase = 0;
                }
                _ => unreachable!(),
            }
            #[cfg(test)]
            checkpoint()?;
        }
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    static FAIL_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}
#[cfg(test)]
pub(crate) fn set_failure_after(value: Option<usize>) {
    FAIL_AFTER.with(|counter| counter.set(value));
}
#[cfg(test)]
pub(crate) fn checkpoint() -> Result<(), DomError> {
    FAIL_AFTER.with(|counter| match counter.get() {
        Some(0) => {
            counter.set(None);
            Err(DomError::MigrationCapacity)
        }
        Some(left) => {
            counter.set(Some(left - 1));
            Ok(())
        }
        None => Ok(()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_failure_preserves_completed_node_and_current_node_suffix() {
        let mut dom = DomRuntime::new();
        let first = dom.create_element("div");
        let second = dom.create_element("span");
        let document = dom.document();
        dom.arena.get_mut(first).unwrap().owner_document = Some(document);
        dom.arena.get_mut(second).unwrap().owner_document = Some(document);
        let mut publication =
            OwnershipPublication::new(dom.arena.id(), vec![(first, None), (second, None)]);
        // First node is complete; second has reset its native state but has
        // not yet published its ownership record.
        FAIL_AFTER.with(|counter| counter.set(Some(8)));
        assert!(publication.resume(&mut dom).is_err());
        dom.arena.get_mut(first).unwrap().interest_target = true;
        dom.arena.get_mut(second).unwrap().interest_target = true;
        publication.resume(&mut dom).unwrap();
        assert!(dom.arena.get(first).unwrap().interest_target);
        assert!(dom.arena.get(second).unwrap().interest_target);
        publication.resume(&mut dom).unwrap();
        assert!(publication.pending.is_empty());
    }
}

struct CheckedGroup {
    node: NodeId,
    peers: Vec<NodeId>,
    next: usize,
    checked: bool,
}
impl CheckedGroup {
    fn prepare(dom: &DomRuntime, node: NodeId) -> Result<Self, DomError> {
        dom.arena.get(node)?;
        let mut peers = Vec::new();
        if dom.is_radio(node) {
            for (other, record) in dom.arena.iter() {
                if other != node && record.checked() && dom.same_radio_group(node, other)? {
                    peers.push(other);
                }
            }
        }
        Ok(Self {
            node,
            peers,
            next: 0,
            checked: false,
        })
    }
    fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        if !self.checked {
            dom.arena.get_mut(self.node)?.checkedness = Some(true);
            dom.mutations.push(Mutation::FormState { node: self.node });
            self.checked = true;
            #[cfg(test)]
            checkpoint()?;
        }
        while let Some(&peer) = self.peers.get(self.next) {
            dom.arena.get_mut(peer)?.checkedness = Some(false);
            dom.mutations.push(Mutation::FormState { node: peer });
            self.next += 1;
            #[cfg(test)]
            checkpoint()?;
        }
        Ok(())
    }
}

pub(crate) struct RadioReconciliation {
    arena: ArenaId,
    root: NodeId,
    refresh_only: bool,
    phase: u8,
    pending: std::collections::VecDeque<NodeId>,
    group: Option<CheckedGroup>,
}
impl RadioReconciliation {
    pub(crate) fn new(arena: ArenaId, root: NodeId) -> Self {
        Self {
            arena,
            root,
            refresh_only: false,
            phase: 0,
            pending: Default::default(),
            group: None,
        }
    }
    pub(crate) fn refresh(arena: ArenaId, root: NodeId) -> Self {
        Self {
            arena,
            root,
            refresh_only: true,
            phase: 1,
            pending: Default::default(),
            group: None,
        }
    }
    pub(crate) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        if dom.arena.id() != self.arena {
            return Err(DomError::MigrationChanged);
        }
        loop {
            match self.phase {
                0 => {
                    let nodes = dom.arena.descendants_inclusive(self.root)?;
                    for node in nodes {
                        if dom.is_radio(node) {
                            dom.radio_form_owners.entry(node).or_insert(None);
                        }
                    }
                    self.phase = 1;
                }
                1 => {
                    let radios = dom
                        .radio_form_owners
                        .keys()
                        .copied()
                        .filter(|node| dom.is_radio(*node))
                        .collect::<Vec<_>>();
                    let mut owners = BTreeMap::new();
                    let mut changed = std::collections::VecDeque::new();
                    for node in radios {
                        let owner = dom.form_owner(node)?;
                        if dom.radio_form_owners.get(&node).copied().flatten() != owner
                            && dom.checked(node)?
                        {
                            changed.push_back(node);
                        }
                        owners.insert(node, owner);
                    }
                    dom.radio_form_owners = owners;
                    self.pending = changed;
                    self.phase = 2;
                }
                2 | 4 => {
                    while let Some(&node) = self.pending.front() {
                        if self.group.is_none() {
                            if self.phase == 4 && !dom.is_connected(node)? {
                                self.pending.pop_front();
                                continue;
                            }
                            self.group = Some(CheckedGroup::prepare(dom, node)?);
                        }
                        self.group.as_mut().unwrap().resume(dom)?;
                        self.group = None;
                        self.pending.pop_front();
                    }
                    self.phase += 1;
                }
                3 => {
                    if self.refresh_only {
                        self.phase = 5;
                        continue;
                    }
                    self.pending = dom
                        .arena
                        .descendants_inclusive(self.root)?
                        .into_iter()
                        .filter(|node| dom.is_radio(*node) && dom.checked(*node).unwrap_or(false))
                        .collect();
                    self.phase = 4;
                }
                5 => return Ok(()),
                _ => unreachable!(),
            }
            #[cfg(test)]
            checkpoint()?;
        }
    }
}

#[cfg(test)]
mod radio_tests {
    use super::*;
    #[test]
    fn radio_peer_cursor_does_not_repeat_the_completed_peer() {
        let mut dom = html::parse("<input id=a type=radio name=g><input id=b type=radio name=g><input id=c type=radio name=g>").into_runtime().unwrap();
        let a = dom.find_by_id("a").unwrap();
        let b = dom.find_by_id("b").unwrap();
        let c = dom.find_by_id("c").unwrap();
        dom.arena.get_mut(b).unwrap().checkedness = Some(true);
        dom.arena.get_mut(c).unwrap().checkedness = Some(true);
        let mut group = CheckedGroup::prepare(&dom, a).unwrap();
        FAIL_AFTER.with(|counter| counter.set(Some(1)));
        assert!(group.resume(&mut dom).is_err());
        assert!(dom.checked(a).unwrap());
        assert!(!dom.checked(b).unwrap());
        dom.arena.get_mut(a).unwrap().checkedness = Some(false);
        dom.arena.get_mut(b).unwrap().checkedness = Some(true);
        group.resume(&mut dom).unwrap();
        assert!(!dom.checked(a).unwrap());
        assert!(dom.checked(b).unwrap());
        assert!(!dom.checked(c).unwrap());
    }
}

pub(crate) struct DocumentAssignment {
    arena: ArenaId,
    pending: Vec<(NodeId, NodeId)>,
    seen: BTreeSet<NodeId>,
}
impl DocumentAssignment {
    pub(crate) fn new(arena: ArenaId, root: NodeId, document: NodeId) -> Self {
        Self {
            arena,
            pending: vec![(root, document)],
            seen: BTreeSet::new(),
        }
    }
    pub(crate) fn resume(&mut self, arena: &mut DomArena) -> Result<(), DomError> {
        if arena.id() != self.arena {
            return Err(DomError::MigrationChanged);
        }
        while let Some(&(node, document)) = self.pending.last() {
            arena.document_id(document)?;
            if self.seen.contains(&node) {
                return Err(DomError::Cycle);
            }
            let record = arena.get(node)?;
            if matches!(record.kind, NodeKind::Document) {
                self.pending.pop();
                self.seen.insert(node);
                continue;
            }
            let children = record.children.clone();
            let content = record.template_content;
            let attributes = arena.attribute_node_ids(node);
            // This helper is itself idempotent: it stores an allocated inert
            // owner on the Document before returning it. Resolve it before
            // publishing the current node's owner and advancing its cursor.
            let content_owner = content
                .map(|_| arena.template_contents_owner_document(document))
                .transpose()?;
            arena.get_mut(node)?.owner_document = Some(document);
            self.pending.pop();
            self.seen.insert(node);
            self.pending.extend(
                attributes
                    .into_iter()
                    .map(|attribute| (attribute, document)),
            );
            self.pending
                .extend(children.into_iter().map(|child| (child, document)));
            if let Some((content, owner)) = content.zip(content_owner) {
                self.pending.push((content, owner));
            }
            #[cfg(test)]
            checkpoint()?;
        }
        Ok(())
    }
}

pub(crate) struct StyleInvalidation {
    arena: ArenaId,
    nodes: std::collections::VecDeque<NodeId>,
    ancestors: bool,
}
impl StyleInvalidation {
    pub(crate) fn subtree(arena: &DomArena, root: NodeId) -> Result<Self, DomError> {
        Ok(Self {
            arena: arena.id(),
            nodes: arena.descendants_inclusive(root)?.into(),
            ancestors: false,
        })
    }
    pub(crate) fn ancestors(arena: &DomArena, mut node: Option<NodeId>) -> Result<Self, DomError> {
        let mut nodes = std::collections::VecDeque::new();
        let mut seen = BTreeSet::new();
        while let Some(id) = node {
            if !seen.insert(id) {
                return Err(DomError::Cycle);
            }
            node = arena.get(id)?.parent;
            nodes.push_back(id);
        }
        Ok(Self {
            arena: arena.id(),
            nodes,
            ancestors: true,
        })
    }
    pub(crate) fn resume(&mut self, arena: &mut DomArena) -> Result<(), DomError> {
        if arena.id() != self.arena {
            return Err(DomError::MigrationChanged);
        }
        while let Some(&node) = self.nodes.front() {
            let record = arena.get_mut(node)?;
            let sheet = matches!(&record.kind, NodeKind::Element { name } if matches!(name.local.as_ref(), "style" | "link"))
                || matches!(&record.kind, NodeKind::ProcessingInstruction { target, .. } if target == "xml-stylesheet");
            if self.ancestors || sheet {
                record.stylesheet_override = None;
                record.stylesheet_disabled = None;
            }
            if sheet {
                record.stylesheet_media = None;
                arena.bump_stylesheet_revision(node);
            }
            self.nodes.pop_front();
            #[cfg(test)]
            checkpoint()?;
        }
        Ok(())
    }
}

pub(crate) struct ConnectionUpdate {
    arena: ArenaId,
    nodes: std::collections::VecDeque<(NodeId, bool)>,
    connected: bool,
    preserve_focus: bool,
    phase: u8,
    was_connected: bool,
}
impl ConnectionUpdate {
    pub(crate) fn new(
        dom: &DomRuntime,
        root: NodeId,
        connected: bool,
        preserve_focus: bool,
    ) -> Result<Self, DomError> {
        let mut pending = vec![(root, false)];
        let mut seen = BTreeSet::new();
        let mut nodes = std::collections::VecDeque::new();
        while let Some((node, notification)) = pending.pop() {
            if notification {
                nodes.push_back((node, true));
                continue;
            }
            if !seen.insert(node) {
                return Err(DomError::Cycle);
            }
            let children = dom.arena.get(node)?.children.clone();
            nodes.push_back((node, false));
            if let Some(document) = dom.arena.hosted_document(node) {
                pending.push((node, true));
                pending.push((document, false));
            }
            pending.extend(children.into_iter().rev().map(|child| (child, false)));
        }
        Ok(Self {
            arena: dom.arena.id(),
            nodes,
            connected,
            preserve_focus,
            phase: 0,
            was_connected: false,
        })
    }
    pub(crate) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        if dom.arena.id() != self.arena {
            return Err(DomError::MigrationChanged);
        }
        while let Some(&(node, notification)) = self.nodes.front() {
            dom.arena.get(node)?;
            if notification {
                dom.mutations.push(Mutation::HostedDocument { frame: node });
                self.nodes.pop_front();
                #[cfg(test)]
                checkpoint()?;
                continue;
            }
            match self.phase {
                0 => {
                    if self.connected {
                        dom.connected.insert(node);
                    } else {
                        self.was_connected = dom.connected.remove(&node);
                        if !self.preserve_focus {
                            dom.forget_pointer_hover_target(node);
                        }
                    }
                    self.phase = 1;
                }
                1 => {
                    if self.connected {
                        dom.hide_nonce_on_insertion(node)?;
                    } else if !self.preserve_focus {
                        dom.remove_from_top_layer(node);
                    }
                    self.phase = 2;
                }
                2 => {
                    if !self.connected && self.was_connected && !self.preserve_focus {
                        // A discovered frame can own a request before its
                        // browsing context has a generation. Retire that
                        // request on disconnect as well as existing contexts.
                        dom.invalidate_frame_navigation_identity(node);
                        if let Some(generation) = dom.frame_context_generations.get_mut(&node) {
                            *generation = generation.saturating_add(1);
                        }
                    }
                    self.phase = 3;
                }
                3 => {
                    if !self.connected && !self.preserve_focus && dom.focused == Some(node) {
                        dom.focus(None)?;
                    }
                    self.nodes.pop_front();
                    self.phase = 0;
                }
                _ => unreachable!(),
            }
            #[cfg(test)]
            checkpoint()?;
        }
        Ok(())
    }
}

/// Root storage shared by the structural and clone cursors. A parser cursor
/// has a real shared owner and therefore migration-aware automatic cleanup.
/// Synchronous cursors release their raw roots before leaving their DOM borrow.
pub(crate) struct CursorRoots {
    owner: Option<SharedDomRuntime>,
    arena: ArenaId,
    raw: Vec<NodeId>,
    tracked: Vec<TrackedNodeLease>,
}
impl CursorRoots {
    pub(crate) fn new(owner: Option<&SharedDomRuntime>, dom: &DomRuntime) -> Self {
        Self {
            owner: owner.cloned(),
            arena: dom.arena.id(),
            raw: Vec::new(),
            tracked: Vec::new(),
        }
    }
    pub(crate) fn retain(&mut self, dom: &mut DomRuntime, node: NodeId) -> Result<(), DomError> {
        self.validate(dom)?;
        if let Some(owner) = &self.owner {
            self.tracked
                .push(TrackedNodeLease::parser_in(owner, dom, node)?);
        } else {
            dom.retain_native(node)?;
            self.raw.push(node);
        }
        Ok(())
    }
    pub(crate) fn validate(&self, dom: &DomRuntime) -> Result<(), DomError> {
        if self.owner.as_ref().is_some_and(|owner| {
            !std::ptr::eq(owner.as_ptr(), dom as *const DomRuntime as *mut DomRuntime)
        }) || dom.arena.id() != self.arena
            || self
                .tracked
                .iter()
                .any(|lease| lease.identity().arena != self.arena)
        {
            return Err(DomError::MigrationChanged);
        }
        Ok(())
    }
    pub(crate) fn owner(&self) -> Option<&SharedDomRuntime> {
        self.owner.as_ref()
    }
    pub(crate) fn release(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        let mut failure = None;
        for lease in &self.tracked {
            if let Err(error) = lease.release_in(dom) {
                failure.get_or_insert(error);
            }
        }
        self.tracked.clear();
        if dom.arena.id() == self.arena {
            while let Some(&node) = self.raw.last() {
                let before = self.raw.len();
                let result = dom.release_native_with_commit(node, || {
                    self.raw.pop();
                });
                if let Err(error) = result {
                    failure.get_or_insert(error);
                    if self.raw.len() == before {
                        break;
                    }
                }
            }
        } else {
            self.raw.clear();
        }
        if dom.arena.id() == self.arena {
            dom.pending_cursor_releases.append(&mut self.raw);
        }
        failure.map_or(Ok(()), Err)
    }
}

struct SelectNormalization {
    select: NodeId,
    changes: std::collections::VecDeque<(NodeId, bool)>,
    content: Option<crate::selected_content::SelectedContentUpdate>,
}
impl SelectNormalization {
    fn new(dom: &DomRuntime, select: NodeId) -> Result<Self, DomError> {
        let options = dom.select_options(select)?;
        let mut changes = std::collections::VecDeque::new();
        if dom
            .attribute_in_null_namespace(select, "multiple")?
            .is_none()
        {
            if let Some(last) = options
                .iter()
                .copied()
                .rfind(|id| dom.arena.get(*id).is_ok_and(Node::selected))
            {
                for option in options {
                    if option != last && dom.option_selected(option)? {
                        changes.push_back((option, false));
                    }
                }
            } else if dom.select_display_size(select)? == 1 {
                for option in options {
                    if !dom.option_disabled(option)? {
                        changes.push_back((option, true));
                        break;
                    }
                }
            }
        }
        Ok(Self {
            select,
            changes,
            content: None,
        })
    }
    fn resume(
        &mut self,
        owner: Option<&SharedDomRuntime>,
        dom: &mut DomRuntime,
    ) -> Result<(), DomError> {
        while let Some(&(node, selected)) = self.changes.front() {
            dom.set_option_selectedness(node, selected)?;
            self.changes.pop_front();
            #[cfg(test)]
            checkpoint()?;
        }
        if self.content.is_none() {
            self.content = Some(crate::selected_content::SelectedContentUpdate::select(
                owner,
                dom,
                self.select,
            )?);
        }
        self.content.as_mut().unwrap().resume(dom)
    }
    fn abort(&mut self, dom: &mut DomRuntime) {
        if let Some(content) = &mut self.content {
            let _ = content.abort(dom);
        }
    }
}

pub(crate) struct OptionReconciliation {
    arena: ArenaId,
    owner: Option<SharedDomRuntime>,
    incoming: std::collections::VecDeque<NodeId>,
    affected: std::collections::VecDeque<NodeId>,
    phase: u8,
    peers: std::collections::VecDeque<NodeId>,
    current_select: Option<NodeId>,
    normalization: Option<SelectNormalization>,
}
impl OptionReconciliation {
    pub(crate) fn new(
        owner: Option<&SharedDomRuntime>,
        dom: &DomRuntime,
        previous: &[(NodeId, Option<NodeId>)],
    ) -> Result<Self, DomError> {
        let mut affected = BTreeSet::new();
        let mut incoming = std::collections::VecDeque::new();
        for &(option, old) in previous {
            let new = dom.option_nearest_select(option)?;
            if old != new {
                affected.extend(old);
                affected.extend(new);
                if new.is_some() && dom.option_selected(option)? {
                    incoming.push_back(option);
                }
            }
        }
        Ok(Self {
            arena: dom.arena.id(),
            owner: owner.cloned(),
            incoming,
            affected: affected.into_iter().collect(),
            phase: 0,
            peers: Default::default(),
            current_select: None,
            normalization: None,
        })
    }
    pub(crate) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        if dom.arena.id() != self.arena {
            return Err(DomError::MigrationChanged);
        }
        while let Some(&option) = self.incoming.front() {
            match self.phase {
                0 => {
                    dom.set_option_selectedness(option, true)?;
                    self.phase = 1;
                }
                1 => {
                    let Some(select) = dom.option_nearest_select(option)? else {
                        self.incoming.pop_front();
                        self.phase = 0;
                        continue;
                    };
                    let options = dom.select_options(select)?;
                    if !options.contains(&option) {
                        self.incoming.pop_front();
                        self.phase = 0;
                        continue;
                    }
                    if dom.option_selected(option)?
                        && dom
                            .attribute_in_null_namespace(select, "multiple")?
                            .is_none()
                    {
                        let mut peers = std::collections::VecDeque::new();
                        for other in options {
                            if other != option && dom.option_selected(other)? {
                                peers.push_back(other);
                            }
                        }
                        self.peers = peers;
                    }
                    self.current_select = Some(select);
                    self.phase = 2;
                }
                2 => {
                    while let Some(&other) = self.peers.front() {
                        dom.set_option_selectedness(other, false)?;
                        self.peers.pop_front();
                        #[cfg(test)]
                        checkpoint()?;
                    }
                    self.phase = 3;
                }
                3 => {
                    if self.normalization.is_none() {
                        let select = self.current_select.ok_or(DomError::MigrationChanged)?;
                        self.normalization = Some(SelectNormalization::new(dom, select)?);
                    }
                    self.normalization
                        .as_mut()
                        .unwrap()
                        .resume(self.owner.as_ref(), dom)?;
                    self.normalization.as_mut().unwrap().abort(dom);
                    self.normalization = None;
                    self.incoming.pop_front();
                    self.phase = 0;
                }
                _ => unreachable!(),
            }
            #[cfg(test)]
            checkpoint()?;
        }
        while let Some(&select) = self.affected.front() {
            if self.normalization.is_none() {
                self.normalization = Some(SelectNormalization::new(dom, select)?);
            }
            self.normalization
                .as_mut()
                .unwrap()
                .resume(self.owner.as_ref(), dom)?;
            self.normalization.as_mut().unwrap().abort(dom);
            self.normalization = None;
            self.affected.pop_front();
            #[cfg(test)]
            checkpoint()?;
        }
        Ok(())
    }
    pub(crate) fn abort(&mut self, dom: &mut DomRuntime) {
        if let Some(normalization) = &mut self.normalization {
            normalization.abort(dom);
        }
    }
}

/// Custody of an existing root after the owning range/iterator has replaced
/// its endpoint. A failed reclamation after decrement never decrements twice.
pub(crate) struct RootRelease {
    arena: ArenaId,
    raw: Option<NodeId>,
    tracked: Option<TrackedNodeLease>,
}
impl RootRelease {
    pub(crate) fn take(owner: Option<&SharedDomRuntime>, dom: &DomRuntime, node: NodeId) -> Self {
        Self {
            arena: dom.arena.id(),
            raw: owner.is_none().then_some(node),
            tracked: owner.map(|owner| TrackedNodeLease::take_parser_root(owner, dom, node)),
        }
    }
    pub(crate) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        if let Some(lease) = &self.tracked {
            lease.release_in(dom)?;
            self.tracked = None;
        }
        if let Some(node) = self.raw {
            if self.arena != dom.arena.id() {
                self.raw = None;
                return Ok(());
            }
            dom.release_native_with_commit(node, || self.raw = None)?;
        }
        Ok(())
    }
    pub(crate) fn abort(&mut self, dom: &mut DomRuntime) {
        let _ = self.resume(dom);
        if self.arena == dom.arena.id()
            && let Some(node) = self.raw.take()
        {
            dom.pending_cursor_releases.push(node);
        }
    }
}

pub(crate) struct RangeRemoval {
    roots: CursorRoots,
    parent: NodeId,
    index: u32,
    descendants: Vec<NodeId>,
    iterators: crate::node_iterators::IteratorRemoval,
    journaled: bool,
    shadow: std::collections::VecDeque<LiveRangeId>,
    boundaries: std::collections::VecDeque<(LiveRangeId, bool, BoundaryPoint, BoundaryPoint)>,
    release: Option<RootRelease>,
}
impl RangeRemoval {
    pub(crate) fn new(
        owner: Option<&SharedDomRuntime>,
        dom: &mut DomRuntime,
        parent: NodeId,
        index: u32,
        removed: NodeId,
    ) -> Result<Self, DomError> {
        dom.arena.get(parent)?;
        let mut light = BTreeSet::new();
        let mut pending = vec![removed];
        while let Some(node) = pending.pop() {
            let record = dom.arena.get(node)?;
            if matches!(record.kind, NodeKind::ShadowRoot { .. }) {
                continue;
            }
            if !light.insert(node) {
                return Err(DomError::Cycle);
            }
            pending.extend(record.children.iter().copied());
        }
        let mut inclusive = BTreeSet::new();
        let mut pending = vec![removed];
        while let Some(node) = pending.pop() {
            if !inclusive.insert(node) {
                return Err(DomError::Cycle);
            }
            let record = dom.arena.get(node)?;
            if !matches!(record.kind, NodeKind::Document) {
                pending.extend(record.children.iter().copied());
            }
        }
        let mut shadow = std::collections::VecDeque::new();
        let mut boundaries = std::collections::VecDeque::new();
        for (&id, range) in &dom.ranges {
            let composed = dom.composed_selection_ranges.contains(&id);
            if !composed
                && [range.start.container, range.end.container]
                    .iter()
                    .any(|node| inclusive.contains(node) && !light.contains(node))
            {
                shadow.push_back(id);
            }
            for (start, old) in [(true, range.start), (false, range.end)] {
                let mut new = old;
                if (if composed { &inclusive } else { &light }).contains(&old.container) {
                    new = BoundaryPoint {
                        container: parent,
                        offset: index,
                    };
                } else if old.container == parent && old.offset > index {
                    new.offset -= 1;
                }
                if new != old {
                    boundaries.push_back((id, start, old, new));
                }
            }
        }
        let mut roots = CursorRoots::new(owner, dom);
        for node in inclusive.iter().copied().chain(std::iter::once(parent)) {
            if let Err(error) = roots.retain(dom, node) {
                let _ = roots.release(dom);
                return Err(error);
            }
        }
        let iterators = match crate::node_iterators::IteratorRemoval::new(
            owner, dom, parent, removed, &light,
        ) {
            Ok(cursor) => cursor,
            Err(error) => {
                let _ = roots.release(dom);
                return Err(error);
            }
        };
        Ok(Self {
            roots,
            parent,
            index,
            descendants: light.into_iter().collect(),
            iterators,
            journaled: false,
            shadow,
            boundaries,
            release: None,
        })
    }
    pub(crate) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        self.roots.validate(dom)?;
        self.iterators.resume(dom)?;
        if !self.journaled {
            dom.boundary_journal.push(boundary_journal::Change::Remove {
                parent: self.parent,
                index: self.index,
                nodes: self.descendants.clone(),
            });
            self.journaled = true;
            #[cfg(test)]
            checkpoint()?;
        }
        while let Some(id) = self.shadow.pop_front() {
            if dom.ranges.contains_key(&id) {
                let revision = dom.range_shadow_removals.entry(id).or_default();
                *revision = revision.saturating_add(1);
            }
            #[cfg(test)]
            checkpoint()?;
        }
        loop {
            if let Some(release) = &mut self.release {
                release.resume(dom)?;
                self.release = None;
            }
            let Some(&(id, start, old, new)) = self.boundaries.front() else {
                return Ok(());
            };
            let current = dom
                .ranges
                .get(&id)
                .map(|range| if start { range.start } else { range.end });
            if current != Some(old) {
                self.boundaries.pop_front();
                continue;
            }
            if old.container != new.container {
                dom.retain_native(new.container)?;
            }
            dom.note_range_endpoint_change();
            let range = dom
                .ranges
                .get_mut(&id)
                .expect("validated range under native borrow");
            if start {
                range.start = new
            } else {
                range.end = new
            }
            if old.container != new.container {
                self.release = Some(RootRelease::take(self.roots.owner(), dom, old.container));
            }
            self.boundaries.pop_front();
            #[cfg(test)]
            checkpoint()?;
        }
    }
    pub(crate) fn abort(&mut self, dom: &mut DomRuntime) {
        if let Some(release) = &mut self.release {
            release.abort(dom);
        }
        self.iterators.abort(dom);
        let _ = self.roots.release(dom);
    }
}

struct OpaqueDisconnect {
    pending: std::collections::VecDeque<(OpaqueRangeId, NodeId)>,
    release: Option<RootRelease>,
}
impl OpaqueDisconnect {
    fn new(dom: &DomRuntime, root: NodeId) -> Result<Self, DomError> {
        let mut pending = std::collections::VecDeque::new();
        if dom.opaque_ranges.has_live_ranges() {
            for node in dom.arena.descendants_inclusive(root)? {
                for id in dom.opaque_ranges.owner_ids(node) {
                    pending.push_back((id, node));
                }
            }
        }
        Ok(Self {
            pending,
            release: None,
        })
    }
    fn resume(
        &mut self,
        owner: Option<&SharedDomRuntime>,
        dom: &mut DomRuntime,
    ) -> Result<(), DomError> {
        loop {
            if let Some(release) = &mut self.release {
                release.resume(dom)?;
                self.release = None;
            }
            let Some(&(id, node)) = self.pending.front() else {
                return Ok(());
            };
            if dom
                .opaque_ranges
                .get(id)
                .is_ok_and(|range| range.owner == Some(node))
            {
                dom.opaque_ranges.disconnect(id)?;
                self.release = Some(RootRelease::take(owner, dom, node));
            }
            self.pending.pop_front();
            #[cfg(test)]
            checkpoint()?;
        }
    }
    fn abort(&mut self, dom: &mut DomRuntime) {
        if let Some(release) = &mut self.release {
            release.abort(dom);
        }
    }
}

/// One insertion request, including its committed structural prefix. Kept by
/// the parser queue rather than inferred from a node's current parent.
pub(crate) struct TreeInsertion {
    roots: CursorRoots,
    parent: NodeId,
    node: NodeId,
    reference: Option<NodeId>,
    former_parent: Option<NodeId>,
    preserve_focus: bool,
    was_connected: bool,
    phase: u8,
    detached_only: bool,
    removed_siblings: MutationSiblings,
    inserted_siblings: MutationSiblings,
    option_owners: Vec<(NodeId, Option<NodeId>)>,
    ranges: Option<RangeRemoval>,
    style: Option<StyleInvalidation>,
    assignment: Option<DocumentAssignment>,
    connection: Option<ConnectionUpdate>,
    ownership: OwnershipPublication,
    opaque: Option<OpaqueDisconnect>,
    radios: RadioReconciliation,
    options: Option<OptionReconciliation>,
    selected: Option<crate::selected_content::SelectedContentUpdate>,
}
impl TreeInsertion {
    pub(crate) fn new(
        owner: Option<&SharedDomRuntime>,
        dom: &mut DomRuntime,
        parent: NodeId,
        node: NodeId,
        reference: Option<NodeId>,
        preserve_focus: bool,
    ) -> Result<Self, DomError> {
        dom.drain_cursor_releases()?;
        dom.arena.validate_tree_edge(parent, node)?;
        if let Some(reference) = reference
            && dom.arena.get(reference)?.parent != Some(parent)
        {
            return Err(DomError::NotAChild(reference, parent));
        }
        let previous = dom.ownership_before_adoption(node, dom.arena.node_document(parent)?)?;
        let option_owners = dom.option_owners_in_subtree(node)?;
        let former_parent = dom.arena.get(node)?.parent;
        let mut removed_siblings = if let Some(parent) = former_parent {
            dom.mutation_siblings_at(parent, dom.child_index(parent, node)?)?
        } else {
            MutationSiblings::default()
        };
        removed_siblings.state_preserving_move = preserve_focus;
        let mut roots = CursorRoots::new(owner, dom);
        roots.validate(dom)?;
        // All nodes used by deferred publications remain live even when an
        // author detaches a subtree between failure and continuation.
        let nodes = dom.arena.descendants_inclusive(node)?;
        for retained in nodes.into_iter().chain(
            [Some(parent), former_parent, reference]
                .into_iter()
                .flatten(),
        ) {
            if let Err(error) = roots.retain(dom, retained) {
                let _ = roots.release(dom);
                return Err(error);
            }
        }
        Ok(Self {
            roots,
            parent,
            node,
            reference,
            former_parent,
            preserve_focus,
            detached_only: false,
            was_connected: dom.connected.contains(&node),
            phase: if reference == Some(node) { 21 } else { 0 },
            removed_siblings,
            inserted_siblings: MutationSiblings::default(),
            option_owners,
            ranges: None,
            style: None,
            assignment: None,
            connection: None,
            ownership: OwnershipPublication::new(dom.arena.id(), previous),
            opaque: None,
            radios: RadioReconciliation::new(dom.arena.id(), node),
            options: None,
            selected: None,
        })
    }
    pub(crate) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        self.roots.validate(dom)?;
        if self.phase == u8::MAX {
            return Err(DomError::MigrationChanged);
        }
        loop {
            match self.phase {
                0 => {
                    if let Some(parent) = self.former_parent
                        && self.ranges.is_none()
                    {
                        let index = dom.child_index(parent, self.node)?;
                        self.ranges = Some(RangeRemoval::new(
                            self.roots.owner(),
                            dom,
                            parent,
                            index,
                            self.node,
                        )?);
                    }
                    self.phase = 1;
                }
                1 | 2 => {
                    let parent = if self.phase == 1 {
                        Some(self.parent)
                    } else {
                        self.former_parent
                    };
                    if self.style.is_none() {
                        self.style = Some(StyleInvalidation::ancestors(&dom.arena, parent)?);
                    }
                    self.style.as_mut().unwrap().resume(&mut dom.arena)?;
                    self.style = None;
                    self.phase += 1;
                }
                3 => {
                    // Validate all IDs and the expected precommit edge first.
                    // The following two writes cannot fail under this borrow.
                    if dom.arena.get(self.node)?.parent != self.former_parent {
                        return Err(DomError::MigrationChanged);
                    }
                    if let Some(parent) = self.former_parent {
                        dom.arena.get(parent)?;
                        dom.arena
                            .get_mut(parent)?
                            .children
                            .retain(|node| *node != self.node);
                        dom.arena.get_mut(self.node)?.parent = None;
                    }
                    self.phase = 4;
                }
                4 => {
                    if let Some(ranges) = &mut self.ranges {
                        ranges.resume(dom)?;
                    }
                    if self.former_parent.is_some() {
                        if self.style.is_none() {
                            self.style = Some(StyleInvalidation::subtree(&dom.arena, self.node)?);
                        }
                        self.style.as_mut().unwrap().resume(&mut dom.arena)?;
                        self.style = None;
                    }
                    self.phase = 5;
                }
                5 => {
                    if self.detached_only {
                        self.phase = 11;
                        continue;
                    }
                    dom.arena.validate_tree_edge(self.parent, self.node)?;
                    if dom.arena.get(self.node)?.parent.is_some() {
                        return Err(DomError::MigrationChanged);
                    }
                    let index = if let Some(reference) = self.reference {
                        dom.child_index(self.parent, reference)? as usize
                    } else {
                        dom.arena.get(self.parent)?.children.len()
                    };
                    dom.arena.get_mut(self.node)?.parent = Some(self.parent);
                    dom.arena
                        .get_mut(self.parent)?
                        .children
                        .insert(index, self.node);
                    // Capture neighbors at the structural commit itself.
                    self.inserted_siblings = dom.mutation_siblings_at(self.parent, index as u32)?;
                    self.inserted_siblings.state_preserving_move = self.preserve_focus;
                    self.phase = 6;
                }
                6 => {
                    if self.assignment.is_none()
                        && let Some(document) = dom.arena.node_document(self.parent)?
                    {
                        self.assignment =
                            Some(DocumentAssignment::new(dom.arena.id(), self.node, document));
                    }
                    if let Some(assignment) = &mut self.assignment {
                        assignment.resume(&mut dom.arena)?;
                    }
                    self.phase = 7;
                }
                7 => {
                    if self.style.is_none() {
                        self.style = Some(StyleInvalidation::subtree(&dom.arena, self.node)?);
                    }
                    self.style.as_mut().unwrap().resume(&mut dom.arena)?;
                    self.style = None;
                    self.phase = 8;
                }
                8 => {
                    dom.arena.initialize_preferred_style_set(self.node)?;
                    self.phase = 9;
                }
                9 => {
                    dom.arena.process_language_pragmas(self.node)?;
                    self.phase = 10;
                }
                10 => {
                    let index = dom.child_index(self.parent, self.node)?;
                    dom.adjust_ranges_for_insertion(self.parent, index, 1)?;
                    self.phase = 11;
                }
                11 => {
                    if self.was_connected {
                        if self.connection.is_none() {
                            self.connection = Some(ConnectionUpdate::new(
                                dom,
                                self.node,
                                false,
                                self.preserve_focus && dom.connected.contains(&self.parent),
                            )?);
                        }
                        self.connection.as_mut().unwrap().resume(dom)?;
                        self.connection = None;
                    }
                    self.phase = 12;
                }
                12 => {
                    if !self.detached_only && dom.connected.contains(&self.parent) {
                        if self.connection.is_none() {
                            self.connection =
                                Some(ConnectionUpdate::new(dom, self.node, true, false)?);
                        }
                        self.connection.as_mut().unwrap().resume(dom)?;
                        self.connection = None;
                    }
                    self.phase = 13;
                }
                13 => {
                    if self.former_parent.is_some() && !self.preserve_focus {
                        if self.opaque.is_none() {
                            self.opaque = Some(OpaqueDisconnect::new(dom, self.node)?);
                        }
                        self.opaque
                            .as_mut()
                            .unwrap()
                            .resume(self.roots.owner(), dom)?;
                    }
                    self.phase = 14;
                }
                14 => {
                    if self.former_parent.is_some() {
                        dom.retain_native(self.node)?;
                        dom.record_mutation(Mutation::Removed {
                            lease: MutationRemovalLease::default(),
                            node: self.node,
                            former_parent: self.former_parent,
                            siblings: self.removed_siblings,
                            observer_suppressed: false,
                        });
                    }
                    self.phase = 15;
                }
                15 => {
                    self.ownership.resume(dom)?;
                    self.phase = 16;
                }
                16 => {
                    if !self.detached_only {
                        dom.record_mutation(Mutation::Inserted {
                            node: self.node,
                            parent: Some(self.parent),
                            siblings: self.inserted_siblings,
                            observer_suppressed: false,
                        });
                    }
                    self.phase = 17;
                }
                17 => {
                    self.radios.resume(dom)?;
                    self.phase = 18;
                }
                18 => {
                    if self.options.is_none() {
                        self.options = Some(OptionReconciliation::new(
                            self.roots.owner(),
                            dom,
                            &self.option_owners,
                        )?);
                    }
                    self.options.as_mut().unwrap().resume(dom)?;
                    self.phase = 19;
                }
                19 => {
                    if self.detached_only {
                        self.phase = 21;
                        continue;
                    }
                    if self.selected.is_none() {
                        self.selected =
                            Some(crate::selected_content::SelectedContentUpdate::inserted(
                                self.roots.owner(),
                                dom,
                                self.node,
                            )?);
                    }
                    self.selected.as_mut().unwrap().resume(dom)?;
                    self.phase = 20;
                }
                20 => {
                    dom.reconcile_inserted_details(self.node)?;
                    self.phase = 21;
                }
                21 => return Ok(()),
                _ => unreachable!(),
            }
            #[cfg(test)]
            checkpoint()?;
        }
    }
    pub(crate) fn committed_tail(&mut self) -> bool {
        if (6..21).contains(&self.phase)
            || (4..6).contains(&self.phase) && self.former_parent.is_some()
        {
            self.detached_only = self.phase < 6;
            if self.detached_only {
                self.preserve_focus = false;
            }
            true
        } else {
            false
        }
    }
    pub(crate) fn abort(&mut self, dom: &mut DomRuntime) {
        self.phase = u8::MAX;
        if let Some(ranges) = &mut self.ranges {
            ranges.abort(dom);
        }
        if let Some(opaque) = &mut self.opaque {
            opaque.abort(dom);
        }
        if let Some(options) = &mut self.options {
            options.abort(dom);
        }
        if let Some(selected) = &mut self.selected {
            let _ = selected.abort(dom);
        }
        let _ = self.roots.release(dom);
    }
}

pub(crate) struct TreeRemoval {
    roots: CursorRoots,
    node: NodeId,
    parent: Option<NodeId>,
    siblings: MutationSiblings,
    phase: u8,
    option_owners: Vec<(NodeId, Option<NodeId>)>,
    ranges: Option<RangeRemoval>,
    style: Option<StyleInvalidation>,
    connection: Option<ConnectionUpdate>,
    radios: RadioReconciliation,
    options: Option<OptionReconciliation>,
    opaque: Option<OpaqueDisconnect>,
}
impl TreeRemoval {
    pub(crate) fn new(
        owner: Option<&SharedDomRuntime>,
        dom: &mut DomRuntime,
        node: NodeId,
    ) -> Result<Self, DomError> {
        dom.drain_cursor_releases()?;
        let parent = dom.arena.get(node)?.parent;
        let siblings = if let Some(parent) = parent {
            dom.mutation_siblings_at(parent, dom.child_index(parent, node)?)?
        } else {
            MutationSiblings::default()
        };
        let option_owners = dom.option_owners_in_subtree(node)?;
        let nodes = dom.arena.descendants_inclusive(node)?;
        let mut roots = CursorRoots::new(owner, dom);
        roots.validate(dom)?;
        for retained in nodes.into_iter().chain(parent) {
            if let Err(error) = roots.retain(dom, retained) {
                let _ = roots.release(dom);
                return Err(error);
            }
        }
        Ok(Self {
            roots,
            node,
            parent,
            siblings,
            phase: 0,
            option_owners,
            ranges: None,
            style: None,
            connection: None,
            radios: RadioReconciliation::refresh(dom.arena.id(), node),
            options: None,
            opaque: None,
        })
    }
    pub(crate) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        self.roots.validate(dom)?;
        if self.phase == u8::MAX {
            return Err(DomError::MigrationChanged);
        }
        loop {
            match self.phase {
                0 => {
                    if let Some(parent) = self.parent
                        && self.ranges.is_none()
                    {
                        let index = dom.child_index(parent, self.node)?;
                        self.ranges = Some(RangeRemoval::new(
                            self.roots.owner(),
                            dom,
                            parent,
                            index,
                            self.node,
                        )?);
                    }
                    self.phase = 1;
                }
                1 => {
                    if self.style.is_none() {
                        self.style = Some(StyleInvalidation::ancestors(&dom.arena, self.parent)?);
                    }
                    self.style.as_mut().unwrap().resume(&mut dom.arena)?;
                    self.style = None;
                    self.phase = 2;
                }
                2 => {
                    if dom.arena.get(self.node)?.parent != self.parent {
                        return Err(DomError::MigrationChanged);
                    }
                    if let Some(parent) = self.parent {
                        dom.arena.get(parent)?;
                        dom.arena
                            .get_mut(parent)?
                            .children
                            .retain(|node| *node != self.node);
                        dom.arena.get_mut(self.node)?.parent = None;
                    }
                    self.phase = 3;
                }
                3 => {
                    if let Some(ranges) = &mut self.ranges {
                        ranges.resume(dom)?;
                    }
                    if self.parent.is_some() {
                        if self.style.is_none() {
                            self.style = Some(StyleInvalidation::subtree(&dom.arena, self.node)?);
                        }
                        self.style.as_mut().unwrap().resume(&mut dom.arena)?;
                        self.style = None;
                    }
                    self.phase = 4;
                }
                4 => {
                    if self.connection.is_none() {
                        self.connection =
                            Some(ConnectionUpdate::new(dom, self.node, false, false)?);
                    }
                    self.connection.as_mut().unwrap().resume(dom)?;
                    self.phase = 5;
                }
                5 => {
                    self.radios.resume(dom)?;
                    self.phase = 6;
                }
                6 => {
                    if self.options.is_none() {
                        self.options = Some(OptionReconciliation::new(
                            self.roots.owner(),
                            dom,
                            &self.option_owners,
                        )?);
                    }
                    self.options.as_mut().unwrap().resume(dom)?;
                    self.phase = 7;
                }
                7 => {
                    if self.parent.is_some() {
                        if self.opaque.is_none() {
                            self.opaque = Some(OpaqueDisconnect::new(dom, self.node)?);
                        }
                        self.opaque
                            .as_mut()
                            .unwrap()
                            .resume(self.roots.owner(), dom)?;
                    }
                    self.phase = 8;
                }
                8 => {
                    dom.retain_native(self.node)?;
                    dom.record_mutation(Mutation::Removed {
                        lease: MutationRemovalLease::default(),
                        node: self.node,
                        former_parent: self.parent,
                        siblings: self.siblings,
                        observer_suppressed: false,
                    });
                    self.phase = 9;
                }
                9 => return Ok(()),
                _ => unreachable!(),
            }
            #[cfg(test)]
            checkpoint()?;
        }
    }
    pub(crate) fn committed_tail(&self) -> bool {
        self.parent.is_some() && (3..9).contains(&self.phase)
    }
    pub(crate) fn abort(&mut self, dom: &mut DomRuntime) {
        self.phase = u8::MAX;
        if let Some(ranges) = &mut self.ranges {
            ranges.abort(dom);
        }
        if let Some(options) = &mut self.options {
            options.abort(dom);
        }
        if let Some(opaque) = &mut self.opaque {
            opaque.abort(dom);
        }
        let _ = self.roots.release(dom);
    }
}

#[cfg(test)]
mod tree_tests {
    use super::*;
    #[test]
    fn every_insert_checkpoint_resumes_one_structural_and_journal_commit() {
        for fail_after in 0..100 {
            set_failure_after(None);
            let mut dom = html::parse("<div id=a><span id=n>text</span></div><div id=b></div>")
                .into_runtime()
                .unwrap();
            let a = dom.find_by_id("a").unwrap();
            let b = dom.find_by_id("b").unwrap();
            let node = dom.find_by_id("n").unwrap();
            let range = dom
                .create_live_range(
                    BoundaryPoint {
                        container: a,
                        offset: 1,
                    },
                    BoundaryPoint {
                        container: a,
                        offset: 1,
                    },
                )
                .unwrap();
            let mutations = dom.mutation_cursor();
            let revision = dom.boundary_revision();
            let mut insertion = TreeInsertion::new(None, &mut dom, b, node, None, false).unwrap();
            set_failure_after(Some(fail_after));
            let first = insertion.resume(&mut dom);
            set_failure_after(None);
            if first.is_err() {
                insertion.resume(&mut dom).unwrap();
            }
            insertion.resume(&mut dom).unwrap();
            assert_eq!(dom.arena.get(node).unwrap().parent, Some(b));
            assert_eq!(dom.arena.get(b).unwrap().children, vec![node]);
            assert!(dom.arena.get(a).unwrap().children.is_empty());
            assert_eq!(dom.live_range(range).unwrap().start.offset, 0);
            assert_eq!(dom.boundary_revision(), revision + 2);
            let batch = dom.mutations_since(mutations);
            assert_eq!(
                batch
                    .mutations
                    .iter()
                    .filter(|row| matches!(row,Mutation::Inserted{node:id,..}if *id==node))
                    .count(),
                1
            );
            assert_eq!(
                batch
                    .mutations
                    .iter()
                    .filter(|row| matches!(row,Mutation::Removed{node:id,..}if *id==node))
                    .count(),
                1
            );
            insertion.abort(&mut dom);
            assert!(matches!(
                insertion.resume(&mut dom),
                Err(DomError::MigrationChanged)
            ));
        }
    }
    #[test]
    fn every_removal_checkpoint_preserves_exact_range_transfer() {
        for fail_after in 0..90 {
            set_failure_after(None);
            let mut dom = html::parse("<div id=p><span id=n>text</span><i></i></div>")
                .into_runtime()
                .unwrap();
            let parent = dom.find_by_id("p").unwrap();
            let node = dom.find_by_id("n").unwrap();
            let range = dom
                .create_live_range(
                    BoundaryPoint {
                        container: node,
                        offset: 0,
                    },
                    BoundaryPoint {
                        container: node,
                        offset: 1,
                    },
                )
                .unwrap();
            let cursor = dom.mutation_cursor();
            let revision = dom.boundary_revision();
            let mut removal = TreeRemoval::new(None, &mut dom, node).unwrap();
            set_failure_after(Some(fail_after));
            let result = removal.resume(&mut dom);
            set_failure_after(None);
            if result.is_err() {
                removal.resume(&mut dom).unwrap();
            }
            removal.resume(&mut dom).unwrap();
            let boundaries = dom.live_range(range).unwrap();
            assert_eq!(
                boundaries.start,
                BoundaryPoint {
                    container: parent,
                    offset: 0
                }
            );
            assert_eq!(boundaries.end, boundaries.start);
            assert_eq!(dom.boundary_revision(), revision + 1);
            assert_eq!(
                dom.mutations_since(cursor)
                    .mutations
                    .iter()
                    .filter(|row| matches!(row,Mutation::Removed{node:id,..}if *id==node))
                    .count(),
                1
            );
            removal.abort(&mut dom);
        }
    }
    #[test]
    fn completed_radio_effect_is_not_replayed_after_later_failure() {
        let mut dom = html::parse("<div id=p></div>").into_runtime().unwrap();
        let parent = dom.find_by_id("p").unwrap();
        let node = dom.create_element("input");
        dom.set_attribute(node, "type", "radio").unwrap();
        dom.set_attribute(node, "checked", "").unwrap();
        let mut insertion = TreeInsertion::new(None, &mut dom, parent, node, None, false).unwrap();
        // Stop exactly after the radio cursor has completed, without running
        // the remaining options/selectedcontent tail.
        for _ in 0..256 {
            if insertion.phase >= 18 {
                break;
            }
            set_failure_after(Some(0));
            let _ = insertion.resume(&mut dom);
        }
        assert_eq!(insertion.phase, 18);
        set_failure_after(None);
        dom.set_checkedness(node, false).unwrap();
        insertion.resume(&mut dom).unwrap();
        assert!(!dom.checked(node).unwrap());
        insertion.abort(&mut dom);
    }
    #[test]
    fn raw_cancel_retains_predecrement_cleanup_and_retries_once() {
        let mut dom = DomRuntime::new();
        let node = dom.create_text("owned");
        dom.retain_native(node).unwrap();
        let mut release = RootRelease::take(None, &dom, node);
        dom.roots.get_mut(&node).unwrap().native = 0;
        release.abort(&mut dom);
        assert_eq!(dom.pending_cursor_releases, vec![node]);
        dom.roots.get_mut(&node).unwrap().native = 1;
        dom.drain_cursor_releases().unwrap();
        assert!(dom.pending_cursor_releases.is_empty());
        assert!(dom.arena.get(node).is_err());
        dom.drain_cursor_releases().unwrap();
    }
    #[test]
    fn wrong_owner_is_rejected_before_any_insertion_effect() {
        let owner = Rc::new(RefCell::new(DomRuntime::new()));
        let mut dom = DomRuntime::new();
        let parent = dom.create_element("div");
        let child = dom.create_text("child");
        assert!(matches!(
            TreeInsertion::new(Some(&owner), &mut dom, parent, child, None, false),
            Err(DomError::MigrationChanged)
        ));
        assert!(dom.arena.get(parent).unwrap().children.is_empty());
        assert!(dom.arena.get(child).unwrap().parent.is_none());
    }
}

#[cfg(test)]
mod removal_lifetime_tests {
    use super::*;
    #[test]
    fn iterator_author_position_survives_a_completed_native_prefix() {
        let mut dom =
            html::parse("<div id=p><span id=a></span><span id=b></span><span id=c></span></div>")
                .into_runtime()
                .unwrap();
        let parent = dom.find_by_id("p").unwrap();
        let a = dom.find_by_id("a").unwrap();
        let c = dom.find_by_id("c").unwrap();
        let iterator = dom.create_node_iterator(parent).unwrap();
        dom.set_node_iterator_reference(iterator, a, false).unwrap();
        let mut cursor = RangeRemoval::new(None, &mut dom, parent, 0, a).unwrap();
        set_failure_after(Some(0));
        assert!(cursor.resume(&mut dom).is_err());
        set_failure_after(None);
        dom.set_node_iterator_reference(iterator, c, true).unwrap();
        cursor.resume(&mut dom).unwrap();
        assert_eq!(dom.node_iterator(iterator).unwrap().reference, c);
        assert!(dom.node_iterator(iterator).unwrap().before);
        cursor.abort(&mut dom);
    }
    #[test]
    fn range_author_endpoint_is_not_replaced_by_an_accepted_old_snapshot() {
        let mut dom = html::parse("<div id=p><span id=a></span><span id=b></span></div>")
            .into_runtime()
            .unwrap();
        let parent = dom.find_by_id("p").unwrap();
        let a = dom.find_by_id("a").unwrap();
        let b = dom.find_by_id("b").unwrap();
        let range = dom
            .create_live_range(
                BoundaryPoint {
                    container: a,
                    offset: 0,
                },
                BoundaryPoint {
                    container: a,
                    offset: 0,
                },
            )
            .unwrap();
        let mut cursor = RangeRemoval::new(None, &mut dom, parent, 0, a).unwrap();
        // The first checkpoint is the journal commit, before endpoint work.
        set_failure_after(Some(0));
        assert!(cursor.resume(&mut dom).is_err());
        set_failure_after(None);
        let author = BoundaryPoint {
            container: b,
            offset: 0,
        };
        dom.set_live_range(range, author, author).unwrap();
        cursor.resume(&mut dom).unwrap();
        assert_eq!(dom.live_range(range).unwrap().start, author);
        cursor.abort(&mut dom);
    }
    #[test]
    fn release_after_accounting_commit_is_not_repeated() {
        let mut dom = DomRuntime::new();
        let node = dom.create_text("cycle");
        dom.retain_native(node).unwrap();
        let mut release = RootRelease::take(None, &dom, node);
        dom.arena.get_mut(node).unwrap().parent = Some(node);
        assert!(matches!(release.resume(&mut dom), Err(DomError::Cycle)));
        assert_eq!(dom.roots[&node].native, 0);
        dom.arena.get_mut(node).unwrap().parent = None;
        release.resume(&mut dom).unwrap();
        assert_eq!(dom.roots[&node].native, 0);
        dom.reclaim_if_unrooted(node).unwrap();
    }
    #[test]
    fn replaced_runtime_rejects_original_tree_receipt_and_keeps_new_counts() {
        let owner = Rc::new(RefCell::new(DomRuntime::new()));
        let mut dom = owner.borrow_mut();
        let parent = dom.create_element("div");
        let child = dom.create_text("old");
        let mut insertion =
            TreeInsertion::new(Some(&owner), &mut dom, parent, child, None, false).unwrap();
        let mut replacement = DomRuntime::new();
        let new_parent = replacement.create_element("div");
        let new_child = replacement.create_text("new");
        assert_eq!(child, new_child);
        let (arena, document) = replacement.snapshot();
        dom.replace_parsed_document(arena, document).unwrap();
        dom.retain_native(new_parent).unwrap();
        dom.retain_native(new_child).unwrap();
        assert!(matches!(
            insertion.resume(&mut dom),
            Err(DomError::MigrationChanged)
        ));
        insertion.abort(&mut dom);
        assert_eq!(dom.roots[&new_child].native, 1);
        assert_eq!(dom.roots[&new_parent].native, 1);
    }
}

#[cfg(test)]
mod leaf_and_cleanup_tests {
    use super::*;
    #[test]
    fn pending_synchronous_cleanup_follows_native_root_migration() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let target = Rc::new(RefCell::new(DomRuntime::new()));
        let node = source.borrow_mut().create_element("div");
        source.borrow_mut().retain_native(node).unwrap();
        source.borrow_mut().pending_cursor_releases.push(node);
        let document = target.borrow().document();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &target, node, document).unwrap();
        let moved = DomRuntime::commit_subtree_migration(&source, &target, &mut prepared).unwrap();
        assert!(source.borrow().pending_cursor_releases.is_empty());
        assert_eq!(target.borrow().pending_cursor_releases, vec![moved.root]);
        let count = target.borrow().roots[&moved.root].native;
        target.borrow_mut().drain_cursor_releases().unwrap();
        assert_eq!(target.borrow().roots[&moved.root].native, count - 1);
    }
    #[test]
    fn nonce_leaf_updates_existing_attr_and_publishes_once() {
        let mut dom = html::parse("<script id=s nonce=abc></script>")
            .into_runtime()
            .unwrap();
        let node = dom.find_by_id("s").unwrap();
        let document = dom.document();
        let attribute = dom.attribute_node(node, "nonce").unwrap().unwrap();
        let cursor = dom.mutation_cursor();
        dom.set_document_nonce_hiding(document, true).unwrap();
        dom.hide_nonce_on_insertion(node).unwrap();
        assert_eq!(dom.element_nonce(node).unwrap(), Some("abc"));
        assert_eq!(dom.attribute(node, "nonce").unwrap(), Some(""));
        assert!(
            matches!(&dom.arena.get(attribute).unwrap().kind,NodeKind::Attribute{data,..}if data.value.is_empty())
        );
        assert_eq!(dom.mutations_since(cursor).mutations.iter().filter(|row|matches!(row,Mutation::Attribute{node:id,name,..}if *id==node&&name=="nonce")).count(),1);
    }
    #[test]
    fn language_scan_failure_does_not_publish_a_partial_pragma() {
        let mut dom = html::parse(
            "<div id=p><meta http-equiv=content-language content=fr><span id=s></span></div>",
        )
        .into_runtime()
        .unwrap();
        let parent = dom.find_by_id("p").unwrap();
        let stale = dom.find_by_id("s").unwrap();
        let document = dom.document();
        dom.arena
            .get_mut(document)
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .pragma_language = None;
        dom.arena.remove(stale).unwrap();
        dom.arena.get_mut(parent).unwrap().children.push(stale);
        assert!(matches!(
            dom.arena.process_language_pragmas(parent),
            Err(DomError::StaleNode(_))
        ));
        assert!(
            dom.arena
                .get(document)
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .pragma_language
                .is_none()
        );
    }
}

/// Only the already committed primitive is retained after a synchronous error.
/// This never contains a caller's unperformed fragment/variadic suffix.
pub(crate) enum CommittedTreeTail {
    Insert(Box<TreeInsertion>),
    Remove(Box<TreeRemoval>),
}
impl std::fmt::Debug for CommittedTreeTail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Insert(cursor) => f
                .debug_struct("CommittedInsertionTail")
                .field("node", &cursor.node)
                .field("phase", &cursor.phase)
                .finish(),
            Self::Remove(cursor) => f
                .debug_struct("CommittedRemovalTail")
                .field("node", &cursor.node)
                .field("phase", &cursor.phase)
                .finish(),
        }
    }
}

impl CommittedTreeTail {
    pub(crate) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        match self {
            Self::Insert(cursor) => cursor.resume(dom),
            Self::Remove(cursor) => cursor.resume(dom),
        }
    }
    pub(crate) fn abort(&mut self, dom: &mut DomRuntime) {
        match self {
            Self::Insert(cursor) => cursor.abort(dom),
            Self::Remove(cursor) => cursor.abort(dom),
        }
    }
}

#[cfg(test)]
mod synchronous_group_tests {
    use super::*;
    #[test]
    fn synchronous_variadic_failure_finishes_only_committed_primitives_once() {
        for operation in 0..3 {
            for fail_after in 0..240 {
                set_failure_after(None);
                let mut dom = html::parse("<div id=p><b id=old></b></div>")
                    .into_runtime()
                    .unwrap();
                let parent = dom.find_by_id("p").unwrap();
                let old = dom.find_by_id("old").unwrap();
                let fragment = dom.create(NodeKind::DocumentFragment);
                let first = dom.create_element("i");
                let second = dom.create_element("u");
                dom.retain_native(fragment).unwrap();
                dom.retain_native(first).unwrap();
                dom.retain_native(second).unwrap();
                dom.append_child(fragment, first).unwrap();
                dom.append_child(fragment, second).unwrap();
                let cursor = dom.mutation_cursor();
                set_failure_after(Some(fail_after));
                let result = match operation {
                    0 => dom.insert_dom_node(parent, fragment, None),
                    1 => dom.replace_dom_children(parent, fragment),
                    _ => dom.replace_dom_child(parent, fragment, old, None),
                };
                set_failure_after(None);
                let target_before = dom.children(parent).unwrap().to_vec();
                let source_before = dom.children(fragment).unwrap().to_vec();
                dom.drain_committed_tree_tails().unwrap();
                dom.drain_committed_tree_tails().unwrap();
                assert_eq!(
                    dom.children(parent).unwrap(),
                    target_before,
                    "failed caller suffix must not resume"
                );
                assert_eq!(
                    dom.children(fragment).unwrap(),
                    source_before,
                    "unperformed fragment removals must not resume"
                );
                let mutations = dom.mutations_since(cursor);
                for child in [first, second] {
                    let additions = mutations
                        .mutations
                        .iter()
                        .map(|row| match row {
                            Mutation::Inserted {
                                node,
                                parent: Some(owner),
                                observer_suppressed: false,
                                ..
                            } if *node == child && *owner == parent => 1,
                            Mutation::ChildrenReplaced {
                                node, added_nodes, ..
                            } if *node == parent => usize::from(added_nodes.contains(&child)),
                            _ => 0,
                        })
                        .sum::<usize>();
                    assert_eq!(
                        additions,
                        usize::from(dom.parent(child).unwrap() == Some(parent)),
                        "one publication per committed insertion"
                    );
                }
                if result.is_ok() {
                    assert_eq!(
                        dom.children(parent)
                            .unwrap()
                            .iter()
                            .filter(|node| **node == first || **node == second)
                            .count(),
                        2
                    );
                }
                assert!(dom.pending_tree_tails.is_empty());
            }
        }
    }
    #[test]
    fn detached_move_failure_never_finishes_unperformed_destination_insertion() {
        for fail_after in 0..120 {
            set_failure_after(None);
            let mut dom = html::parse("<div id=a><i id=n></i></div><div id=b></div>")
                .into_runtime()
                .unwrap();
            let a = dom.find_by_id("a").unwrap();
            let b = dom.find_by_id("b").unwrap();
            let node = dom.find_by_id("n").unwrap();
            dom.retain_native(node).unwrap();
            let range = dom
                .create_live_range(
                    BoundaryPoint {
                        container: a,
                        offset: 1,
                    },
                    BoundaryPoint {
                        container: a,
                        offset: 1,
                    },
                )
                .unwrap();
            let cursor = dom.mutation_cursor();
            set_failure_after(Some(fail_after));
            let _ = dom.append_child(b, node);
            set_failure_after(None);
            let committed_parent = dom.parent(node).unwrap();
            dom.drain_committed_tree_tails().unwrap();
            assert_eq!(dom.parent(node).unwrap(), committed_parent);
            assert_eq!(
                dom.live_range(range).unwrap().start.offset,
                u32::from(committed_parent == Some(a)),
                "precommit failure keeps old boundaries; committed removal finishes them"
            );
            let mutations = dom.mutations_since(cursor);
            assert_eq!(mutations.mutations.iter().filter(|row|matches!(row,Mutation::Removed{node:id,former_parent:Some(owner),..}if *id==node&&*owner==a)).count(),usize::from(committed_parent!=Some(a)));
            assert_eq!(mutations.mutations.iter().filter(|row|matches!(row,Mutation::Inserted{node:id,parent:Some(owner),..}if *id==node&&*owner==b)).count(),usize::from(committed_parent==Some(b)));
        }
    }
}

#[cfg(test)]
mod parser_frame_connection_tests {
    use super::*;
    #[test]
    fn disconnected_request_identity_expires_without_a_context_generation() {
        let mut dom =
            html::parse("<body><div id=a><iframe id=f></iframe></div><div id=b></div></body>")
                .into_runtime()
                .unwrap();
        let a = dom.find_by_id("a").unwrap();
        let b = dom.find_by_id("b").unwrap();
        let frame = dom.find_by_id("f").unwrap();
        assert_eq!(dom.frame_context_generation(frame), None);
        let request = dom.frame_navigation_identity(frame);
        dom.move_before(b, frame, None).unwrap();
        assert!(
            dom.frame_navigation_identity_is_current(frame, &request),
            "state-preserving movement retains the admitted request"
        );
        dom.remove(frame).unwrap();
        dom.append_child(a, frame).unwrap();
        assert!(
            !dom.frame_navigation_identity_is_current(frame, &request),
            "ordinary reconnect cannot revive a pre-context request"
        );
        let replacement = dom.frame_navigation_identity(frame);
        dom.track_frame_context(frame);
        dom.remove(frame).unwrap();
        dom.untrack_frame_context(frame);
        dom.append_child(b, frame).unwrap();
        dom.track_frame_context(frame);
        assert!(
            !dom.frame_navigation_identity_is_current(frame, &replacement),
            "generation recreation cannot revive a request either"
        );
    }
}
