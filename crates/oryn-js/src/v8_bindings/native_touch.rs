//! Native touch-action policy. The contact snapshots this before pointerdown;
//! author changes during the gesture cannot retroactively change its policy.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Policy {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub pinch: bool,
    pub double_tap: bool,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            left: true,
            right: true,
            up: true,
            down: true,
            pinch: true,
            double_tap: true,
        }
    }
}
impl Policy {
    fn none() -> Self {
        Self {
            left: false,
            right: false,
            up: false,
            down: false,
            pinch: false,
            double_tap: false,
        }
    }
    fn parse(value: &str) -> Self {
        if value == "auto" {
            return Self::default();
        }
        if value == "manipulation" {
            return Self {
                double_tap: false,
                ..Self::default()
            };
        }
        let mut result = Self::none();
        for token in value.split_ascii_whitespace() {
            match token {
                "pan-x" => {
                    result.left = true;
                    result.right = true;
                }
                "pan-y" => {
                    result.up = true;
                    result.down = true;
                }
                "pan-left" => result.left = true,
                "pan-right" => result.right = true,
                "pan-up" => result.up = true,
                "pan-down" => result.down = true,
                "pinch-zoom" => result.pinch = true,
                "none" => return Self::none(),
                // A computed value is normally valid. An absent/unknown producer
                // value uses the CSS initial policy rather than granting a subset.
                _ => return Self::default(),
            }
        }
        result
    }
    fn intersect(self, other: Self) -> Self {
        Self {
            left: self.left && other.left,
            right: self.right && other.right,
            up: self.up && other.up,
            down: self.down && other.down,
            pinch: self.pinch && other.pinch,
            double_tap: self.double_tap && other.double_tap,
        }
    }
}

pub(super) fn policy(state: &V8BindingState, target: NodeId) -> Result<Policy, V8BindingError> {
    refresh_layout(state)?;
    let dom = state.dom.borrow();
    let layout = state.layout.borrow();
    let mut cursor = Some(target);
    let mut seen = BTreeSet::new();
    let mut result = Policy::default();
    while let Some(node) = cursor {
        if !seen.insert(node)
            || !dom.is_connected(node)?
            || dom.node_document(node)? != Some(state.document_root())
        {
            break;
        }
        let value = editing_snapshot_property(&dom, &layout, node, "touch-action")?
            .unwrap_or_else(|| "auto".into());
        result = result.intersect(Policy::parse(&value));
        // Panning is bounded by the nearest inclusive scrolling container.
        // Pinch/zoom additionally observes every ancestor to the viewport.
        let scrolling = is_scrolling_document_root(&dom, &layout, state.document_root(), node)
            || ["overflow-x", "overflow-y"].iter().any(|axis| {
                editing_snapshot_property(&dom, &layout, node, axis)
                    .ok()
                    .flatten()
                    .is_some_and(|value| matches!(value.as_str(), "auto" | "scroll" | "hidden"))
            });
        if scrolling {
            let mut ancestor = dom.flat_tree_parent(node)?;
            while let Some(parent) = ancestor {
                if !seen.insert(parent) {
                    break;
                }
                let value = editing_snapshot_property(&dom, &layout, parent, "touch-action")?
                    .unwrap_or_else(|| "auto".into());
                let outer = Policy::parse(&value);
                result.pinch &= outer.pinch;
                result.double_tap &= outer.double_tap;
                ancestor = dom.flat_tree_parent(parent)?;
            }
            break;
        }
        cursor = dom.flat_tree_parent(node)?;
    }
    Ok(result)
}

/// Contact-local recognition never re-reads author policy or geometry. Direction
/// permissions govern the initial pan; an accepted pan may subsequently reverse.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PanState {
    Pending,
    Rejected,
    Accepted { x: bool, y: bool },
}
#[derive(Clone, Debug)]
pub(super) struct Contact {
    identity: Rc<()>,
    pub target: NodeId,
    policy: Policy,
    projection: Option<oryn_common::v4::PaintProjection>,
    origin: (f64, f64),
    previous: (f64, f64),
    state: PanState,
    default_prevented: bool,
}
impl Contact {
    pub fn new(state: &V8BindingState, target: NodeId, x: f64, y: f64, policy: Policy) -> Self {
        Self {
            identity: Rc::new(()),
            target,
            policy,
            projection: state
                .layout
                .borrow()
                .hit_projections
                .get(&node_key(target))
                .cloned(),
            origin: (x, y),
            previous: (x, y),
            state: PanState::Pending,
            default_prevented: false,
        }
    }
    pub fn same_contact(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.identity, &other.identity)
    }
    pub fn prevent_default(&mut self) {
        self.default_prevented = true;
        if self.state == PanState::Pending {
            self.state = PanState::Rejected;
        }
    }
    pub fn active(&self) -> bool {
        matches!(self.state, PanState::Accepted { .. })
    }
    fn local(&self, point: (f64, f64)) -> Option<(f64, f64)> {
        if let Some(projection) = &self.projection {
            projection.viewport_to_local(point.0, point.1)
        } else {
            Some(point)
        }
    }
    /// Returns two viewport endpoints after filtering in the contact's original
    /// local plane. A singular transform rejects the contact rather than silently
    /// applying viewport axes to a differently oriented touch-action surface.
    pub fn movement(&mut self, x: f64, y: f64) -> Option<((f64, f64), (f64, f64))> {
        if !x.is_finite() || !y.is_finite() {
            if !self.active() {
                self.state = PanState::Rejected;
            }
            return None;
        }
        let Some(origin) = self.local(self.origin) else {
            if !self.active() {
                self.state = PanState::Rejected;
            }
            return None;
        };
        let Some(current) = self.local((x, y)) else {
            if !self.active() {
                self.state = PanState::Rejected;
            }
            return None;
        };
        if self.state == PanState::Pending {
            // Recognition slop is measured in viewport CSS pixels, independent
            // of author scale. Direction is measured in the element's plane.
            if (x - self.origin.0).hypot(y - self.origin.1) < 8. {
                return None;
            }
            self.state = recognize(self.policy, current.0 - origin.0, current.1 - origin.1);
        }
        let PanState::Accepted {
            x: allow_x,
            y: allow_y,
        } = self.state
        else {
            return None;
        };
        let previous = self.local(self.previous)?;
        let filtered = (
            if allow_x { current.0 } else { previous.0 },
            if allow_y { current.1 } else { previous.1 },
        );
        let start = self.previous;
        let end = if let Some(projection) = &self.projection {
            project(projection.matrix, filtered)?
        } else {
            filtered
        };
        self.previous = (x, y);
        Some((start, end))
    }
}
/// A pinch candidate owns the exact two contact lifetimes it observed. Pointer
/// IDs alone are insufficient because a released ID can be reused immediately.
#[derive(Clone, Debug)]
pub(super) struct PinchCandidate {
    contacts: (Contact, Contact),
    pinch: visual_viewport::Pinch,
    initial_distance: f64,
    active: bool,
    revision: u64,
}
impl PinchCandidate {
    pub fn begin(
        geometry: visual_viewport::Geometry,
        policy: visual_viewport::ZoomPolicy,
        first: (&Contact, i32, f64, f64),
        second: (&Contact, i32, f64, f64),
    ) -> Option<Self> {
        if first.0.same_contact(second.0)
            || !first.0.policy.pinch
            || !second.0.policy.pinch
            || first.0.default_prevented
            || second.0.default_prevented
        {
            return None;
        }
        let pinch = visual_viewport::Pinch::begin(
            geometry,
            policy,
            (first.1, first.2, first.3),
            (second.1, second.2, second.3),
        )?;
        Some(Self {
            contacts: (first.0.clone(), second.0.clone()),
            pinch,
            initial_distance: (first.2 - second.2).hypot(first.3 - second.3),
            active: false,
            revision: 0,
        })
    }
    pub fn update(
        &mut self,
        geometry: &mut visual_viewport::Geometry,
        first: (&Contact, i32, f64, f64),
        second: (&Contact, i32, f64, f64),
    ) -> Option<bool> {
        if !self.contacts.0.same_contact(first.0)
            || !self.contacts.1.same_contact(second.0)
            || first.0.default_prevented
            || second.0.default_prevented
        {
            return None;
        }
        let distance = (first.2 - second.2).hypot(first.3 - second.3);
        if !distance.is_finite() {
            return None;
        }
        if !self.active && (distance - self.initial_distance).abs() < 8. {
            return Some(false);
        }
        self.revision = self.revision.checked_add(1)?;
        let changed = self.pinch.update(
            geometry,
            (first.1, first.2, first.3),
            (second.1, second.2, second.3),
        )?;
        self.active = true;
        Some(changed)
    }
    pub fn same_update(&self, other: &Self) -> bool {
        self.revision == other.revision
            && self.contacts.0.same_contact(&other.contacts.0)
            && self.contacts.1.same_contact(&other.contacts.1)
    }
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn contains(&self, id: i32) -> bool {
        self.pinch.contains(id)
    }
}

fn recognize(policy: Policy, dx: f64, dy: f64) -> PanState {
    let x = if dx < 0. {
        policy.left
    } else if dx > 0. {
        policy.right
    } else {
        false
    };
    let y = if dy < 0. {
        policy.up
    } else if dy > 0. {
        policy.down
    } else {
        false
    };
    // Reject a disallowed dominant direction for the rest of this contact.
    // Once accepted, retain both permitted axes, including reversal on either.
    if dx.abs() >= dy.abs() && !x || dy.abs() > dx.abs() && !y {
        PanState::Rejected
    } else {
        PanState::Accepted {
            x: policy.left || policy.right,
            y: policy.up || policy.down,
        }
    }
}
fn project(matrix: [f64; 16], point: (f64, f64)) -> Option<(f64, f64)> {
    let w = matrix[3] * point.0 + matrix[7] * point.1 + matrix[15];
    if !w.is_finite() || w <= 0. {
        return None;
    }
    let result = (
        (matrix[0] * point.0 + matrix[4] * point.1 + matrix[12]) / w,
        (matrix[1] * point.0 + matrix[5] * point.1 + matrix[13]) / w,
    );
    (result.0.is_finite() && result.1.is_finite()).then_some(result)
}

/// Consume finger movement through native scroll containers. Each ancestor
/// receives the residual motion in viewport coordinates, then maps it into its
/// own plane. Hidden overflow is programmatically scrollable but not pannable.
struct PanStep {
    node: NodeId,
    parent: Option<NodeId>,
    next: Option<(f64, f64)>,
    remaining: Option<(f64, f64)>,
    picker_epoch: Option<u64>,
    changed: bool,
}
pub(super) struct Pan {
    identity: (NodeId, u64, u64, u64),
    target: NodeId,
    cursor: Option<NodeId>,
    start: (f64, f64),
    end: (f64, f64),
    seen: BTreeSet<NodeId>,
    prepared: bool,
    pending: Option<PanStep>,
    changed: bool,
    done: bool,
}
impl Pan {
    pub(super) fn new(
        state: &V8BindingState,
        target: NodeId,
        start: (f64, f64),
        end: (f64, f64),
    ) -> Self {
        Self {
            identity: (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            ),
            target,
            cursor: Some(target),
            start,
            end,
            seen: BTreeSet::new(),
            prepared: false,
            pending: None,
            changed: false,
            done: false,
        }
    }
    pub(super) fn poll(&mut self, state: &Rc<V8BindingState>) -> Result<bool, V8BindingError> {
        if self.done {
            return Ok(self.changed);
        }
        if state.execution_retired()
            || self.identity
                != (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.document_authority_epoch(),
                )
        {
            self.done = true;
            return Ok(self.changed);
        }
        if !self.prepared {
            refresh_layout(state)?;
            self.prepared = true;
        }
        loop {
            if self.pending.is_none() {
                let Some(node) = self.cursor else {
                    self.done = true;
                    return Ok(self.changed);
                };
                if self.seen.contains(&node)
                    || !state.dom.borrow().is_connected(node)?
                    || state.dom.borrow().node_document(node)? != Some(state.document_root())
                {
                    self.done = true;
                    return Ok(self.changed);
                }
                let parent = state.dom.borrow().flat_tree_parent(node)?;
                if let Some(picker) = select_picker_scroll::snapshot(state, node)
                    && select_picker_scroll::owner_for(state, self.target)? == Some(node)
                    && let Some(projection) = picker.projection.as_ref()
                    && let (Some(a), Some(b), Some(scroll)) = (
                        projection.viewport_to_local(self.start.0, self.start.1),
                        projection.viewport_to_local(self.end.0, self.end.1),
                        projection.scroll.as_ref(),
                    )
                {
                    let eligible = |axis| {
                        picker
                            .style
                            .get(axis)
                            .is_some_and(|value| matches!(value.as_str(), "auto" | "scroll"))
                    };
                    let delta = (a.0 - b.0, a.1 - b.1);
                    let next = (
                        (scroll.offset_x + if eligible("overflow-x") { delta.0 } else { 0. })
                            .clamp(
                                (scroll.scrollable.x - scroll.scrollport.x).min(0.),
                                (scroll.scrollable.right() - scroll.scrollport.right()).max(0.),
                            ),
                        (scroll.offset_y + if eligible("overflow-y") { delta.1 } else { 0. })
                            .clamp(
                                (scroll.scrollable.y - scroll.scrollport.y).min(0.),
                                (scroll.scrollable.bottom() - scroll.scrollport.bottom()).max(0.),
                            ),
                    );
                    let mut remaining = (
                        delta.0 - (next.0 - scroll.offset_x),
                        delta.1 - (next.1 - scroll.offset_y),
                    );
                    if eligible("overflow-x")
                        && picker
                            .style
                            .get("overscroll-behavior-x")
                            .is_some_and(|value| matches!(value.as_str(), "contain" | "none"))
                    {
                        remaining.0 = 0.;
                    }
                    if eligible("overflow-y")
                        && picker
                            .style
                            .get("overscroll-behavior-y")
                            .is_some_and(|value| matches!(value.as_str(), "contain" | "none"))
                    {
                        remaining.1 = 0.;
                    }
                    let epoch = state
                        .dom
                        .borrow()
                        .top_layer()
                        .find(|entry| {
                            entry.node == node && entry.kind == oryn_dom::TopLayerKind::SelectPicker
                        })
                        .map(|entry| entry.picker_epoch);
                    self.pending = Some(PanStep {
                        node,
                        parent,
                        next: Some(next),
                        remaining: project(
                            projection.matrix,
                            (b.0 + remaining.0, b.1 + remaining.1),
                        ),
                        picker_epoch: epoch,
                        changed: next != (scroll.offset_x, scroll.offset_y),
                    });
                }
                if self.pending.is_none() {
                    let plan = {
                        let dom = state.dom.borrow();
                        let layout = state.layout.borrow();
                        let root =
                            is_scrolling_document_root(&dom, &layout, state.document_root(), node);
                        let projection = if root {
                            None
                        } else {
                            layout.hit_projections.get(&node_key(node))
                        };
                        if !root && projection.and_then(|value| value.scroll.as_ref()).is_none() {
                            None
                        } else {
                            let local = |point: (f64, f64)| {
                                projection.map_or(Some(point), |value| {
                                    value.viewport_to_local(point.0, point.1)
                                })
                            };
                            if let (Some(a), Some(b)) = (local(self.start), local(self.end)) {
                                let property = |name: &str| {
                                    if root {
                                        layout
                                            .viewport_scroll
                                            .as_ref()
                                            .and_then(|value| value.style.get(name))
                                            .cloned()
                                    } else {
                                        editing_snapshot_property(&dom, &layout, node, name)
                                            .ok()
                                            .flatten()
                                    }
                                };
                                let permits = |name: &str| {
                                    property(name).map_or(root, |value| {
                                        matches!(value.as_str(), "auto" | "scroll")
                                            || root && value == "visible"
                                    })
                                };
                                let bounds = native_scroll_bounds_in_layout(
                                    &dom,
                                    &layout,
                                    state.document_root(),
                                    node,
                                );
                                let old = state
                                    .scroll_offsets
                                    .borrow()
                                    .get(&scroll_offset_node(state, node))
                                    .copied()
                                    .unwrap_or_default();
                                let next = (
                                    if permits("overflow-x") {
                                        (old.0 + a.0 - b.0).clamp(bounds.min.0, bounds.max.0)
                                    } else {
                                        old.0
                                    },
                                    if permits("overflow-y") {
                                        (old.1 + a.1 - b.1).clamp(bounds.min.1, bounds.max.1)
                                    } else {
                                        old.1
                                    },
                                );
                                let contain = |name: &str| {
                                    property(name).is_some_and(|value| {
                                        matches!(value.as_str(), "contain" | "none")
                                    })
                                };
                                let consumed = (
                                    if permits("overflow-x") && contain("overscroll-behavior-x") {
                                        b.0
                                    } else {
                                        a.0 - (next.0 - old.0)
                                    },
                                    if permits("overflow-y") && contain("overscroll-behavior-y") {
                                        b.1
                                    } else {
                                        a.1 - (next.1 - old.1)
                                    },
                                );
                                Some(PanStep {
                                    node,
                                    parent,
                                    next: (old != next).then_some(next),
                                    remaining: projection.map_or(Some(consumed), |projection| {
                                        project(projection.matrix, consumed)
                                    }),
                                    picker_epoch: None,
                                    changed: old != next,
                                })
                            } else {
                                None
                            }
                        }
                    };
                    self.pending = Some(plan.unwrap_or(PanStep {
                        node,
                        parent,
                        next: None,
                        remaining: Some(self.start),
                        picker_epoch: None,
                        changed: false,
                    }));
                }
            }
            let step = self.pending.as_ref().unwrap();
            if !state.dom.borrow().is_connected(step.node)?
                || state.dom.borrow().node_document(step.node)? != Some(state.document_root())
            {
                self.done = true;
                self.pending.take();
                return Ok(self.changed);
            }
            if let Some(next) = step.next {
                if let Some(epoch) = step.picker_epoch {
                    if !state.dom.borrow().top_layer().any(|entry| {
                        entry.node == step.node
                            && entry.kind == oryn_dom::TopLayerKind::SelectPicker
                            && entry.picker_epoch == epoch
                    }) {
                        self.done = true;
                        self.pending.take();
                        return Ok(self.changed);
                    }
                    if state
                        .dom
                        .borrow_mut()
                        .set_select_picker_scroll(step.node, next)?
                    {
                        state.layout_cache_key.borrow_mut().take();
                    }
                } else {
                    perform_scroll(state, step.node, (Some(next.0), Some(next.1)), false)?;
                }
            }
            let step = self.pending.take().unwrap();
            self.seen.insert(step.node);
            self.cursor = step.parent;
            self.changed |= step.changed;
            let Some(remaining) = step.remaining else {
                self.done = true;
                return Ok(self.changed);
            };
            self.start = remaining;
            if (self.start.0 - self.end.0).abs() < 0.000001
                && (self.start.1 - self.end.1).abs() < 0.000001
            {
                self.done = true;
                return Ok(self.changed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contact_slop_axis_filter_reversal_and_rejection_persist() {
        let dom = html::parse("<div id=target></div>").into_runtime().unwrap();
        let target = dom.find_by_id("target").unwrap();
        let make = |policy| Contact {
            identity: Rc::new(()),
            target,
            policy,
            projection: None,
            origin: (50., 50.),
            previous: (50., 50.),
            state: PanState::Pending,
            default_prevented: false,
        };
        let mut contact = make(Policy::parse("pan-up"));
        assert!(contact.same_contact(&contact.clone()));
        assert!(!contact.same_contact(&make(Policy::parse("pan-up"))));
        assert_eq!(contact.movement(52., 46.), None);
        assert_eq!(contact.movement(53., 38.), Some(((50., 50.), (50., 38.))));
        assert!(contact.active());
        assert_eq!(contact.movement(57., 60.), Some(((53., 38.), (53., 60.))));
        // Bad coordinates cannot reactivate a canceled pointer stream.
        assert_eq!(contact.movement(f64::NAN, 60.), None);
        assert!(contact.active());
        let mut rejected = make(Policy::parse("pan-y"));
        assert_eq!(rejected.movement(65., 52.), None);
        assert_eq!(rejected.movement(50., 10.), None);
        assert!(!rejected.active());
    }
    #[test]
    fn pinch_candidate_owns_both_contacts_and_observes_cancellation() {
        let dom = html::parse("<div id=target></div>").into_runtime().unwrap();
        let target = dom.find_by_id("target").unwrap();
        let make = |policy| Contact {
            identity: Rc::new(()),
            target,
            policy,
            projection: None,
            origin: (0., 0.),
            previous: (0., 0.),
            state: PanState::Pending,
            default_prevented: false,
        };
        let first = make(Policy::default());
        let mut second = make(Policy::default());
        let mut geometry = visual_viewport::Geometry::default();
        geometry.resize(400., 300.);
        let original = geometry;
        let mut candidate = PinchCandidate::begin(
            geometry,
            Default::default(),
            (&first, 1, 100., 100.),
            (&second, 2, 200., 100.),
        )
        .unwrap();
        assert_eq!(
            candidate.update(
                &mut geometry,
                (&first, 1, 98., 100.),
                (&second, 2, 202., 100.)
            ),
            Some(false)
        );
        assert_eq!(geometry, original);
        let prior_update = candidate.clone();
        assert_eq!(
            candidate.update(
                &mut geometry,
                (&first, 1, 50., 100.),
                (&second, 2, 250., 100.)
            ),
            Some(true)
        );
        assert!(!candidate.same_update(&prior_update));
        assert!(candidate.same_update(&candidate.clone()));
        assert!(candidate.active());
        assert!(candidate.contains(2));
        assert_eq!(geometry.scale, 2.);
        let zoomed = geometry;
        let replacement = make(Policy::default());
        assert_eq!(
            candidate.update(
                &mut geometry,
                (&first, 1, 0., 100.),
                (&replacement, 2, 300., 100.)
            ),
            None
        );
        assert_eq!(geometry, zoomed);
        second.prevent_default();
        assert_eq!(
            candidate.update(
                &mut geometry,
                (&first, 1, 0., 100.),
                (&second, 2, 300., 100.)
            ),
            None
        );
        assert!(
            PinchCandidate::begin(
                geometry,
                Default::default(),
                (&first, 1, 100., 100.),
                (&second, 2, 200., 100.)
            )
            .is_none()
        );
        let blocked = make(Policy::parse("pan-x pan-y"));
        assert!(
            PinchCandidate::begin(
                geometry,
                Default::default(),
                (&first, 1, 100., 100.),
                (&blocked, 2, 200., 100.)
            )
            .is_none()
        );
    }
    #[test]
    fn initial_pan_direction_is_locked_but_reversal_is_allowed() {
        let left = Policy::parse("pan-left");
        assert_eq!(
            recognize(left, -12., 2.),
            PanState::Accepted { x: true, y: false }
        );
        assert_eq!(recognize(left, 12., 2.), PanState::Rejected);
        assert_eq!(recognize(left, -2., 12.), PanState::Rejected);
        assert_eq!(
            recognize(Policy::parse("pan-y"), 2., -12.),
            PanState::Accepted { x: false, y: true }
        );
        assert_eq!(
            recognize(Policy::parse("none"), 12., 0.),
            PanState::Rejected
        );
    }
    #[test]
    fn projected_finger_motion_preserves_rotation_and_rejects_horizon() {
        let mut matrix = [0.; 16];
        matrix[1] = 2.;
        matrix[4] = -2.;
        matrix[10] = 1.;
        matrix[15] = 1.;
        assert_eq!(project(matrix, (3., 4.)), Some((-8., 6.)));
        matrix[15] = 0.;
        assert_eq!(project(matrix, (3., 4.)), None);
        matrix[15] = -1.;
        assert_eq!(project(matrix, (3., 4.)), None);
    }
    #[test]
    fn touch_action_intersection_keeps_direction_and_zoom_separate() {
        let policy =
            Policy::parse("pan-left pan-y pinch-zoom").intersect(Policy::parse("pan-x pan-down"));
        assert_eq!(
            policy,
            Policy {
                left: true,
                right: false,
                up: false,
                down: true,
                pinch: false,
                double_tap: false
            }
        );
        assert_eq!(
            Policy::parse("none").intersect(Policy::default()),
            Policy::none()
        );
        assert_eq!(
            Policy::parse("manipulation"),
            Policy {
                double_tap: false,
                ..Policy::default()
            }
        );
        assert_eq!(Policy::parse("auto"), Policy::default());
    }
}
