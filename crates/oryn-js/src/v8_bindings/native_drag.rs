//! Host-owned drag session. Author callbacks never run with a DOM or session
//! registry borrowed. The input host supplies hit-test results and owns default
//! editing, source deletion, drag-image capture and platform modifier mapping.
use super::*;
mod advance;
mod begin;
mod cancel;
mod control_move;
mod drop_default;
mod drop_operation;
mod input;
mod rich_move;
pub use input::NativeDragInput;
#[path = "native_drag_cross_frame.rs"]
mod cross_frame;
use crate::data_transfer::{DragEffect, DragNegotiation, EventInit, Mode, Store};
pub use cross_frame::{
    NativeCrossFrameDrag, NativeCrossFrameDragAction, NativeDragAncestor, NativeDragRealm,
    dispatch_cross_frame_drag, dispatch_cross_frame_drag_with_previous, native_drag_session_active,
    native_drag_target_identity,
};

/// Original source ownership is independent of the live Selection. Native
/// mutation records prevent delete-after-callback from removing replacement data.
#[derive(Clone)]
enum SelectedKind {
    Control {
        node: NodeId,
        start: u32,
        end: u32,
        value: String,
    },
    Range {
        range: RangeRecord,
        host: Option<NodeId>,
    },
}
#[derive(Clone)]
struct SelectedSource {
    kind: SelectedKind,
    document: NodeId,
    generation: u64,
    epoch: u64,
    authority: u64,
    cursor: oryn_dom::MutationCursor,
    watched: BTreeSet<NodeId>,
}
impl SelectedSource {
    fn valid(&self, state: &V8BindingState) -> bool {
        if state.execution_retired()
            || (self.document, self.generation, self.epoch, self.authority)
                != (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.document_authority_epoch(),
                )
        {
            return false;
        }
        let dom = state.dom.borrow();
        let batch = dom.mutations_since(self.cursor);
        if batch.truncated {
            return false;
        }
        let changed = batch.mutations.iter().any(|mutation| {
            use oryn_dom::Mutation::*;
            match mutation {
                Inserted { node, parent, .. } => {
                    self.watched.contains(node)
                        || parent.is_some_and(|node| self.watched.contains(&node))
                }
                Removed {
                    node,
                    former_parent,
                    ..
                } => {
                    self.watched.contains(node)
                        || former_parent.is_some_and(|node| self.watched.contains(&node))
                }
                ChildrenReplaced { node, .. }
                | Attribute { node, .. }
                | CharacterData { node, .. }
                | FormState { node }
                | DocumentOwnership { node, .. } => self.watched.contains(node),
                ShadowAttached { host, .. } => self.watched.contains(host),
                _ => false,
            }
        });
        if changed {
            return false;
        }
        match &self.kind {
            SelectedKind::Control { node, value, .. } => {
                dom.is_connected(*node).unwrap_or(false)
                    && dom.control_value(*node).ok().as_ref() == Some(value)
            }
            SelectedKind::Range { range, .. } => [range.start_container, range.end_container]
                .iter()
                .all(|node| {
                    dom.is_connected(*node).unwrap_or(false)
                        && dom.arena().node_document(*node).ok().flatten() == Some(self.document)
                }),
        }
    }
    fn move_target(&self, state: &V8BindingState, target: NodeId) -> bool {
        if !self.valid(state) {
            return false;
        }
        match &self.kind {
            SelectedKind::Control { node, .. } => {
                let dom = state.dom.borrow();
                (dom.is_text_control(target)
                    || editing_host_for_node(
                        &dom,
                        Some(target),
                        state.design_mode_enabled(),
                        state.document_root(),
                    )
                    .ok()
                    .flatten()
                    .is_some())
                    && !dom.control_disabled(*node).unwrap_or(true)
                    && dom.attribute(*node, "readonly").ok().flatten().is_none()
            }
            SelectedKind::Range {
                range,
                host: Some(source),
            } => {
                let dom = state.dom.borrow();
                (dom.is_text_control(target)
                    || editing_host_for_node(
                        &dom,
                        Some(target),
                        state.design_mode_enabled(),
                        state.document_root(),
                    )
                    .ok()
                    .flatten()
                    .is_some())
                    && [range.start_container, range.end_container]
                        .iter()
                        .all(|node| {
                            editing_host_for_node(
                                &dom,
                                Some(*node),
                                state.design_mode_enabled(),
                                state.document_root(),
                            )
                            .ok()
                            .flatten()
                                == Some(*source)
                        })
            }
            _ => false,
        }
    }
}
fn selected_source(
    state: &V8BindingState,
    target: NodeId,
    x: f64,
    y: f64,
) -> Result<Option<(NodeId, Store, Rc<SelectedSource>)>, V8BindingError> {
    let mut store = Store::default();
    let (source, kind, watched) = if state.dom.borrow().is_text_control(target) {
        let dom = state.dom.borrow();
        if dom
            .attribute(target, "type")?
            .is_some_and(|kind| kind.eq_ignore_ascii_case("password"))
        {
            return Ok(None);
        }
        drop(dom);
        let (start, end, value) = clipboard_actions::control_selection(state, target)?;
        if start == end {
            return Ok(None);
        }
        refresh_layout(state)?;
        if !selection_contains_position(
            state,
            &RangeRecord {
                start_container: target,
                start_offset: start,
                end_container: target,
                end_offset: end,
            },
            x,
            y,
        )? {
            return Ok(None);
        }
        if value.len() > crate::clipboard::MAX_CLIPBOARD_BYTES {
            return Ok(None);
        }
        let units = oryn_dom::dom_string::units(&value);
        store.set_data("text/plain", units[start as usize..end as usize].to_vec());
        (
            target,
            SelectedKind::Control {
                node: target,
                start,
                end,
                value,
            },
            BTreeSet::from([target]),
        )
    } else {
        let Some(range) = state
            .selection_range
            .get()
            .and_then(|id| state.ranges.borrow().get(&id).cloned())
        else {
            return Ok(None);
        };
        if range.start_container == range.end_container && range.start_offset == range.end_offset {
            return Ok(None);
        }
        refresh_layout(state)?;
        if !selection_contains_position(state, &range, x, y)? {
            return Ok(None);
        }
        let (text, html) = clipboard_actions::rich_selection(state, &range)?;
        store.set_data("text/plain", text);
        store.set_data("text/html", oryn_dom::dom_string::units(&html));
        let dom = state.dom.borrow();
        let source = if matches!(
            dom.arena().get(range.start_container)?.kind,
            NodeKind::Text { .. } | NodeKind::CDataSection { .. }
        ) {
            dom.parent(range.start_container)?
                .unwrap_or(range.start_container)
        } else {
            range.start_container
        };
        let host = editing_host_for_node(
            &dom,
            Some(range.start_container),
            state.design_mode_enabled(),
            state.document_root(),
        )?;
        let root = host.unwrap_or(common_ancestor(
            &dom,
            range.start_container,
            range.end_container,
        )?);
        let watched = dom
            .arena()
            .descendants_inclusive(root)?
            .into_iter()
            .collect();
        (source, SelectedKind::Range { range, host }, watched)
    };
    let snapshot = Rc::new(SelectedSource {
        kind,
        document: state.document_root(),
        generation: state.document_generation(),
        epoch: state.document_stream_epoch(),
        authority: state.document_authority_epoch(),
        cursor: state.dom.borrow().mutation_cursor(),
        watched,
    });
    Ok(Some((source, store, snapshot)))
}
/// Use the inverse native text paint projection; an axis-aligned bounding box
/// would incorrectly include empty corners around rotated selected text.
fn selection_contains_position(
    state: &V8BindingState,
    range: &RangeRecord,
    x: f64,
    y: f64,
) -> Result<bool, V8BindingError> {
    let dom = state.dom.borrow();
    let layout = state.layout.borrow();
    for node in dom.arena().descendants_inclusive(common_ancestor(
        &dom,
        range.start_container,
        range.end_container,
    )?)? {
        let Some(clusters) = layout.text_clusters.get(&node_key(node)) else {
            continue;
        };
        for cluster in clusters {
            let start = BoundaryPoint {
                container: node,
                offset: cluster.start as u32,
            };
            let end = BoundaryPoint {
                container: node,
                offset: cluster.end as u32,
            };
            if compare_boundary_points(
                &dom,
                (end.container, end.offset),
                (range.start_container, range.start_offset),
            )? <= 0
                || compare_boundary_points(
                    &dom,
                    (start.container, start.offset),
                    (range.end_container, range.end_offset),
                )? >= 0
            {
                continue;
            }
            let m = cluster.matrix;
            // Solve the two projective screen equations for the z=0 text plane.
            let a = m[0] - x * m[3];
            let b = m[4] - x * m[7];
            let c = x * m[15] - m[12];
            let d = m[1] - y * m[3];
            let e = m[5] - y * m[7];
            let f = y * m[15] - m[13];
            let det = a * e - b * d;
            if !det.is_finite() || det.abs() < 1e-12 {
                continue;
            }
            let px = (c * e - b * f) / det;
            let py = (a * f - c * d) / det;
            let w = m[3] * px + m[7] * py + m[15];
            if !w.is_finite() || w <= 0. {
                continue;
            }
            let mut r = cluster.rect;
            let selected_start = if node == range.start_container {
                cluster.start.max(range.start_offset as usize)
            } else {
                cluster.start
            };
            let selected_end = if node == range.end_container {
                cluster.end.min(range.end_offset as usize)
            } else {
                cluster.end
            };
            if selected_start >= selected_end {
                continue;
            }
            let span = (cluster.end - cluster.start).max(1) as f64;
            let mut first = (selected_start - cluster.start) as f64 / span;
            let mut last = (selected_end - cluster.start) as f64 / span;
            let writing = layout
                .styles
                .get(&cluster.layout_node)
                .and_then(|style| style.get("writing-mode"))
                .map(String::as_str)
                .unwrap_or("horizontal-tb");
            if cluster.rtl ^ (writing == "sideways-lr") {
                (first, last) = (1. - last, 1. - first);
            }
            if writing.starts_with("vertical") || writing.starts_with("sideways") {
                r.y += r.height * first;
                r.height *= last - first;
            } else {
                r.x += r.width * first;
                r.width *= last - first;
            }
            if px >= r.x && px <= r.x + r.width && py >= r.y && py <= r.y + r.height {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn selection_image_polygons(
    state: &V8BindingState,
    range: &RangeRecord,
) -> Result<Vec<[f64; 8]>, V8BindingError> {
    refresh_layout(state)?;
    let dom = state.dom.borrow();
    let layout = state.layout.borrow();
    let mut polygons = Vec::new();
    for node in dom.arena().descendants_inclusive(common_ancestor(
        &dom,
        range.start_container,
        range.end_container,
    )?)? {
        let Some(clusters) = layout.text_clusters.get(&node_key(node)) else {
            continue;
        };
        for cluster in clusters {
            if compare_boundary_points(
                &dom,
                (node, cluster.end as u32),
                (range.start_container, range.start_offset),
            )? <= 0
                || compare_boundary_points(
                    &dom,
                    (node, cluster.start as u32),
                    (range.end_container, range.end_offset),
                )? >= 0
            {
                continue;
            }
            let start = if node == range.start_container {
                cluster.start.max(range.start_offset as usize)
            } else {
                cluster.start
            };
            let end = if node == range.end_container {
                cluster.end.min(range.end_offset as usize)
            } else {
                cluster.end
            };
            if start >= end {
                continue;
            }
            let mut first =
                (start - cluster.start) as f64 / (cluster.end - cluster.start).max(1) as f64;
            let mut last =
                (end - cluster.start) as f64 / (cluster.end - cluster.start).max(1) as f64;
            let mode = layout
                .styles
                .get(&cluster.layout_node)
                .and_then(|style| style.get("writing-mode"))
                .map(String::as_str)
                .unwrap_or("horizontal-tb");
            if cluster.rtl ^ (mode == "sideways-lr") {
                (first, last) = (1. - last, 1. - first);
            }
            let mut rect = cluster.rect;
            if mode.starts_with("vertical") || mode.starts_with("sideways") {
                rect.y += rect.height * first;
                rect.height *= last - first;
            } else {
                rect.x += rect.width * first;
                rect.width *= last - first;
            }
            let mut polygon = [0.; 8];
            let m = cluster.matrix;
            let mut valid = true;
            for (index, (x, y)) in [
                (rect.x, rect.y),
                (rect.right(), rect.y),
                (rect.right(), rect.bottom()),
                (rect.x, rect.bottom()),
            ]
            .into_iter()
            .enumerate()
            {
                let w = m[3] * x + m[7] * y + m[15];
                if !w.is_finite() || w <= 0. {
                    valid = false;
                    break;
                }
                polygon[index * 2] = (m[0] * x + m[4] * y + m[12]) / w;
                polygon[index * 2 + 1] = (m[1] * x + m[5] * y + m[13]) / w;
            }
            if valid {
                polygons.push(polygon);
                if polygons.len() > 32768 {
                    return Err(V8BindingError::Allocation("drag selection geometry budget"));
                }
            }
        }
    }
    Ok(polygons)
}

pub struct Session {
    source: NodeId,
    document: NodeId,
    generation: u64,
    epoch: u64,
    authority: u64,
    store: Rc<RefCell<Store>>,
    negotiation: DragNegotiation,
    ended: bool,
    feedback_session: Option<u64>,
    selected: Option<Rc<SelectedSource>>,
    cross_target: Option<cross_frame::Target>,
}
impl Session {
    /// Seeded data contains only native selected bytes/URLs, never path grants.
    /// A canceled dragstart does not create a running drag session.
    pub fn begin(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        source: NodeId,
        seeded: Store,
        init: &EventInit,
    ) -> Result<Option<Self>, V8BindingError> {
        let mut start = begin::Begin::new(scope, state, source, seeded, init)?;
        if start.poll(scope)? {
            Ok(start.take_session())
        } else {
            Err(V8BindingError::Allocation("dragstart suspended"))
        }
    }
    fn publish_image(&mut self, state: &V8BindingState, init: &EventInit) {
        let custom = self
            .store
            .borrow()
            .drag_image_snapshot
            .as_ref()
            .map(|image| (**image).clone());
        let image = custom.or_else(|| {
            let (source, polygons) = if let Some(selected) = self.selected.as_ref() {
                if !selected.valid(state) {
                    return None;
                }
                let range = match &selected.kind {
                    SelectedKind::Control {
                        node, start, end, ..
                    } => RangeRecord {
                        start_container: *node,
                        start_offset: *start,
                        end_container: *node,
                        end_offset: *end,
                    },
                    SelectedKind::Range { range, .. } => range.clone(),
                };
                let polygons = selection_image_polygons(state, &range).ok()?;
                if polygons.is_empty() {
                    return None;
                }
                let dom = state.dom.borrow();
                let root =
                    common_ancestor(&dom, range.start_container, range.end_container).ok()?;
                let root = if matches!(dom.arena().get(root).ok()?.kind, NodeKind::Element { .. }) {
                    root
                } else {
                    dom.parent(root).ok()??
                };
                (root, polygons)
            } else {
                (self.source, Vec::new())
            };
            let mut image = native_drag_image::capture(state, source, (0, 0), polygons).ok()?;
            image.hotspot = (
                (init.client_x as f64 - image.bounds.x) * image.scale,
                (init.client_y as f64 - image.bounds.y) * image.scale,
            );
            Some(image)
        });
        self.feedback_session = Some(native_drag_image::begin(state, image));
    }
    fn live(&self, state: &V8BindingState) -> bool {
        !self.ended
            && !state.execution_retired()
            && state.document_root() == self.document
            && state.document_generation() == self.generation
            && state.document_stream_epoch() == self.epoch
            && state.document_authority_epoch() == self.authority
    }
    fn present(&self, state: &V8BindingState, node: NodeId) -> bool {
        state
            .dom
            .borrow()
            .arena()
            .node_document(node)
            .ok()
            .flatten()
            == Some(self.document)
    }
    fn target_live(&self, state: &V8BindingState, node: NodeId) -> bool {
        self.present(state, node) && state.dom.borrow().is_connected(node).unwrap_or(false)
    }
    fn abandon(&mut self) {
        self.negotiation.finish(false, &mut self.store.borrow_mut());
        self.store.borrow_mut().set_mode(Mode::Detached);
        self.ended = true;
    }
    pub fn operation(&self) -> DragEffect {
        self.negotiation.operation()
    }
    /// Deliver drag source and target transitions. The caller invokes this at
    /// native drag update opportunities, including stationary-pointer updates.
    pub fn advance(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        target: Option<NodeId>,
        preferred: DragEffect,
        native_default: Option<DragEffect>,
        init: &EventInit,
    ) -> Result<DragEffect, V8BindingError> {
        let mut update =
            advance::Advance::new(state, self, target, preferred, native_default, init);
        if !update.poll(scope, self)? {
            return Err(V8BindingError::Allocation("drag advance suspended"));
        }
        if update.cancel_requested() {
            self.cancel(scope, state, init)
        } else {
            Ok(update.result())
        }
    }
    pub fn cancel(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        init: &EventInit,
    ) -> Result<DragEffect, V8BindingError> {
        let mut cancellation = cancel::Cancel::new(state, self, init);
        if cancellation.poll(scope, self)? {
            Ok(DragEffect::None)
        } else {
            Err(V8BindingError::Allocation("drag cancellation suspended"))
        }
    }
    /// The default callback performs native editing only when drop was not
    /// canceled. Source deletion is permitted only for the returned Move result.
    pub fn drop_on_target(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        init: &EventInit,
        default: impl FnOnce(
            &mut v8::PinScope,
            &Rc<V8BindingState>,
            NodeId,
            &Store,
        ) -> Result<bool, V8BindingError>,
    ) -> Result<DragEffect, V8BindingError> {
        let mut operation = drop_operation::DropOperation::new(state, self, init);
        let mut default = Some(default);
        operation
            .poll(scope, self, |scope, state, target, store| {
                default.take().ok_or(V8BindingError::Allocation(
                    "consumed synchronous drop default",
                ))?(scope, state, target, store)
                .map(Some)
            })?
            .ok_or(V8BindingError::Allocation("drop operation suspended"))
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.store.borrow_mut().set_mode(Mode::Detached);
    }
}

fn default_effect(state: &V8BindingState, target: NodeId, store: &Store) -> Option<DragEffect> {
    let dom = state.dom.borrow();
    if !dom.is_connected(target).unwrap_or(false) || dom.control_disabled(target).unwrap_or(true) {
        return None;
    }
    let types = store.types();
    if dom.is_file_input(target) && types.iter().any(|kind| kind == "Files") {
        return Some(DragEffect::Copy);
    }
    if dom.is_text_control(target)
        && dom.attribute(target, "readonly").ok().flatten().is_none()
        && types.iter().any(|kind| kind == "text/plain")
    {
        return Some(DragEffect::Copy);
    }
    if types
        .iter()
        .any(|kind| kind == "text/plain" || kind == "text/html")
        && editing_host_for_node(
            &dom,
            Some(target),
            state.design_mode_enabled(),
            state.document_root(),
        )
        .ok()
        .flatten()
        .is_some()
    {
        return Some(DragEffect::Copy);
    }
    None
}

fn caret_owned_by(state: &V8BindingState, caret: BoundaryPoint, owner: NodeId) -> bool {
    let dom = state.dom.borrow();
    dom.is_connected(caret.container).unwrap_or(false)
        && dom.arena().node_document(caret.container).ok().flatten() == Some(state.document_root())
        && editing_host_for_node(
            &dom,
            Some(caret.container),
            state.design_mode_enabled(),
            state.document_root(),
        )
        .ok()
        .flatten()
            == Some(owner)
        && validate_boundary_offset(&dom, caret.container, caret.offset).is_ok()
}

/// Compatibility entry point; retained native input owns RichMove directly.
#[cfg(test)]
fn move_mixed(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    store: &Store,
    source: &SelectedSource,
    caret: BoundaryPoint,
    text: &str,
) -> Result<bool, V8BindingError> {
    let mut job = rich_move::RichMove::new(state, source, target, store, caret, text);
    job.poll(scope)?
        .ok_or(V8BindingError::Allocation("rich drag move suspended"))
}

fn moved_control_value(
    value: &str,
    start: u32,
    end: u32,
    destination: u32,
    payload: &str,
) -> Option<(String, u32)> {
    let mut units = oryn_dom::dom_string::units(value);
    let (start, end, destination) = (start as usize, end as usize, destination as usize);
    if start >= end
        || end > units.len()
        || destination > units.len()
        || (start..=end).contains(&destination)
    {
        return None;
    }
    units.drain(start..end);
    let destination = if destination > end {
        destination - (end - start)
    } else {
        destination
    };
    let payload = oryn_dom::dom_string::units(payload);
    let position = destination.checked_add(payload.len())?;
    if units.len().checked_add(payload.len())?.checked_mul(2)?
        > crate::clipboard::MAX_CLIPBOARD_BYTES
    {
        return None;
    }
    units.splice(destination..destination, payload);
    Some((
        oryn_dom::dom_string::from_units(&units),
        u32::try_from(position).ok()?,
    ))
}
fn preferred_effect(init: &EventInit, selected: bool) -> DragEffect {
    if selected {
        if init.ctrl_key || init.alt_key {
            DragEffect::Copy
        } else {
            DragEffect::Move
        }
    } else if init.alt_key {
        DragEffect::Link
    } else if init.shift_key {
        DragEffect::Move
    } else {
        DragEffect::Copy
    }
}
fn session_default(
    state: &V8BindingState,
    drag: &Session,
    target: NodeId,
    preferred: DragEffect,
) -> Option<DragEffect> {
    default_effect(state, target, &drag.store.borrow()).map(|effect| {
        if preferred == DragEffect::Move
            && drag
                .selected
                .as_ref()
                .is_some_and(|source| source.move_target(state, target))
        {
            DragEffect::Move
        } else {
            effect
        }
    })
}

/// Realm input bridge. Coordinates/modifiers come from the serialized native
/// action queue; source eligibility and payloads are read from the native DOM.
pub fn element_drag_source(
    state: &V8BindingState,
    target: NodeId,
) -> Result<Option<NodeId>, V8BindingError> {
    if state.execution_retired() {
        return Ok(None);
    }
    let dom = state.dom.borrow();
    // A pointer in a text-editing or select surface belongs to its native
    // selection gesture, even when it or an ancestor is explicitly draggable.
    // Selected-text payload admission runs before this element-source lookup.
    if editing_host_for_node(
        &dom,
        Some(target),
        state.design_mode_enabled(),
        state.document_root(),
    )?
    .is_some()
    {
        return Ok(None);
    }
    let mut cursor = Some(target);
    while let Some(node) = cursor {
        if !dom.is_connected(node)?
            || dom.arena().node_document(node)? != Some(state.document_root())
        {
            break;
        }
        if dom.is_text_control(node)
            || dom.is_element(node, "http://www.w3.org/1999/xhtml", "select")
        {
            return Ok(None);
        }
        let html = matches!(&dom.arena().get(node)?.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml");
        if html {
            let attribute = dom.attribute(node, "draggable")?;
            let explicit = attribute.is_some_and(|value| value.eq_ignore_ascii_case("true"));
            let suppressed = attribute.is_some_and(|value| value.eq_ignore_ascii_case("false"));
            let image = dom.is_element(node, "http://www.w3.org/1999/xhtml", "img");
            let link = dom.is_element(node, "http://www.w3.org/1999/xhtml", "a")
                && dom.attribute(node, "href")?.is_some();
            if explicit || (!suppressed && (image || link)) {
                return Ok(Some(node));
            }
        }
        cursor = dom.parent(node)?;
    }
    Ok(None)
}

pub(super) fn dispatch_input(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    operation: &str,
    target: Option<NodeId>,
    init: &EventInit,
) -> Result<bool, V8BindingError> {
    let mut job = NativeDragInput::new(state, operation, target, init)?;
    job.poll(scope)?
        .ok_or(V8BindingError::Allocation("native drag input suspended"))
}

pub(super) fn input_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let result = (|| {
        let operation = args.get(0).to_rust_string_lossy(scope);
        let target = if args.get(1).is_null_or_undefined() {
            None
        } else {
            let object = args
                .get(1)
                .to_object(scope)
                .ok_or(V8BindingError::Allocation("drag target"))?;
            Some(resolve_receiver(scope, &state, object, "Node")?)
        };
        let x = args.get(2).int32_value(scope).unwrap_or(0);
        let y = args.get(3).int32_value(scope).unwrap_or(0);
        let flags = args.get(4).uint32_value(scope).unwrap_or(0);
        let init = EventInit {
            client_x: x,
            client_y: y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: 1,
            ctrl_key: flags & 1 != 0,
            alt_key: flags & 2 != 0,
            shift_key: flags & 4 != 0,
            meta_key: flags & 8 != 0,
            ..Default::default()
        };
        dispatch_input(scope, &state, &operation, target, &init)
    })();
    match result {
        Ok(active) => rv.set_bool(active),
        Err(error) => throw_binding_error(scope, &error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, code).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn draggable_reflection_defaults_ascii_tokens_and_boolean_conversion() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{for(const tag of ['div','a','img']){const e=document.createElement(tag);if(tag==='a')e.href='/target';const native=tag!=='div';if(e.draggable!==native)throw Error('default '+tag);for(const [token,value] of [['TrUe',true],['FaLsE',false],['false ',native],['ſalse',native],['auto',native],['',native]]){e.setAttribute('draggable',token);if(e.draggable!==value)throw Error(token)}let calls=0;e.draggable={toString(){calls++;throw Error('conversion')}};if(calls||e.getAttribute('draggable')!=='true'||!e.draggable)throw Error('boolean true');e.draggable=0;if(e.getAttribute('draggable')!=='false'||e.draggable)throw Error('boolean false')}return true})()"#).is_true());
    }
    #[test]
    fn interactive_drag_source_matrix_preserves_buttons_and_native_selection() {
        super::super::tests::initialize_v8();
        for (markup, allowed) in [
            ("<button id=inner>x</button>", true),
            ("<input id=inner type=button>", true),
            ("<input id=inner type=image>", true),
            ("<input id=inner type=reset>", true),
            ("<input id=inner type=submit>", true),
            ("<input id=inner type=text value=text>", false),
            ("<input id=inner type=text disabled value=text>", false),
            ("<input id=inner type=text readonly value=text>", false),
            ("<textarea id=inner>text</textarea>", false),
            ("<select id=inner><option>x</option></select>", false),
            (
                "<select id=inner multiple><option>x</option></select>",
                false,
            ),
            ("<div id=inner contenteditable>text</div>", false),
        ] {
            for explicit in [false, true] {
                let markup = if explicit {
                    markup.replace("id=inner", "id=inner draggable=true")
                } else {
                    markup.into()
                };
                let dom = Rc::new(RefCell::new(
                    oryn_dom::html::parse(&format!("<div id=outer draggable=true>{markup}</div>"))
                        .into_runtime()
                        .unwrap(),
                ));
                let mut isolate = v8::Isolate::new(Default::default());
                v8::scope!(let scope,&mut isolate);
                let context = v8::Context::new(scope, Default::default());
                let scope = &mut v8::ContextScope::new(scope, context);
                let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
                let inner = state.dom.borrow().find_by_id("inner").unwrap();
                let outer = state.dom.borrow().find_by_id("outer").unwrap();
                assert_eq!(
                    element_drag_source(&state, inner).unwrap(),
                    allowed.then_some(if explicit { inner } else { outer }),
                    "{markup}"
                );
            }
        }
    }
    #[test]
    fn selected_control_move_uses_original_utf16_span_and_final_payload() {
        assert_eq!(
            moved_control_value("abcdef", 1, 3, 6, "bc"),
            Some(("adefbc".into(), 6))
        );
        assert_eq!(
            moved_control_value("abcdef", 3, 5, 0, "DE"),
            Some(("DEabcf".into(), 2))
        );
        // A dragstart handler may replace the payload without changing source
        // ownership: the original bc is removed and replacement text inserted.
        assert_eq!(
            moved_control_value("abcdef", 1, 3, 6, "XY"),
            Some(("adefXY".into(), 6))
        );
        let value = oryn_dom::dom_string::from_units(&[65, 0xd800, 66, 0xdc00, 67]);
        let payload = oryn_dom::dom_string::from_units(&[0xd800, 66]);
        let (moved, caret) = moved_control_value(&value, 1, 3, 5, &payload).unwrap();
        assert_eq!(
            oryn_dom::dom_string::units(&moved),
            vec![65, 0xdc00, 67, 0xd800, 66]
        );
        assert_eq!(caret, 5);
        for destination in 1..=3 {
            assert!(moved_control_value("abcdef", 1, 3, destination, "bc").is_none());
        }
        assert!(moved_control_value("abcdef", 1, 7, 0, "x").is_none());
    }
    #[test]
    fn drag_source_replacement_is_detected_even_when_value_is_restored() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=source value=abcdef><input id=other>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let node = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "source")
            .unwrap()
            .unwrap();
        let snapshot = SelectedSource {
            kind: SelectedKind::Control {
                node,
                start: 1,
                end: 3,
                value: "abcdef".into(),
            },
            document: state.document_root(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            authority: state.document_authority_epoch(),
            cursor: state.dom.borrow().mutation_cursor(),
            watched: BTreeSet::from([node]),
        };
        assert!(snapshot.valid(&state));
        assert!(eval(scope,"document.getElementById('source').setSelectionRange(4,5);document.getElementById('other').value='independent';true").is_true());
        assert!(
            snapshot.valid(&state),
            "selection changes and unrelated values must not redirect ownership"
        );
        assert!(eval(scope,"document.getElementById('source').value='replacement';document.getElementById('source').value='abcdef';true").is_true());
        assert!(
            !snapshot.valid(&state),
            "equal current bytes cannot erase intervening replacement"
        );
    }
    #[test]
    fn rich_drag_payload_sanitization_preserves_formatting_and_removes_active_nodes() {
        let html = clipboard_actions::sanitized_html(
            "<b style='color:red'>bold</b><script>bad()</script><img src=x onerror='bad()'>",
        )
        .unwrap();
        assert!(html.contains("<b"));
        assert!(html.contains("color:red"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("onerror"));
    }
    #[test]
    fn two_editor_move_undo_redo_and_failed_insert_restore_both_owners() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<div contenteditable id=a>source</div><div contenteditable id=b>target</div>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let a = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "a")
            .unwrap()
            .unwrap();
        let b = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "b")
            .unwrap()
            .unwrap();
        let text_a = state.dom.borrow().children(a).unwrap()[0];
        let text_b = state.dom.borrow().children(b).unwrap()[0];
        set_selection_boundaries(
            &state,
            BoundaryPoint {
                container: text_a,
                offset: 0,
            },
            BoundaryPoint {
                container: text_a,
                offset: 6,
            },
        )
        .unwrap();
        assert!(
            run_compound_edit_transaction(scope, &state, &[a, b], |state| {
                replace_selection_with_text(state, "")?;
                set_selection_boundaries(
                    state,
                    BoundaryPoint {
                        container: text_b,
                        offset: 6,
                    },
                    BoundaryPoint {
                        container: text_b,
                        offset: 6,
                    },
                )?;
                insert_editing_text(state, "source")
            })
            .unwrap()
        );
        assert!(eval(scope,"document.getElementById('a').textContent===''&&document.getElementById('b').textContent==='targetsource'").is_true());
        assert!(undo_edit(&state, false).unwrap());
        assert!(eval(scope,"document.getElementById('a').textContent==='source'&&document.getElementById('b').textContent==='target'").is_true());
        assert!(undo_edit(&state, true).unwrap());
        assert!(eval(scope,"document.getElementById('a').textContent===''&&document.getElementById('b').textContent==='targetsource'").is_true());
        assert_eq!(
            state.selection_anchor.get(),
            Some(BoundaryPoint {
                container: text_b,
                offset: 12
            })
        );
        assert_eq!(
            state.selection_focus.get(),
            Some(BoundaryPoint {
                container: text_b,
                offset: 12
            })
        );
        assert!(undo_edit(&state, false).unwrap());
        assert!(
            run_compound_edit_transaction(scope, &state, &[a, b], |state| {
                state.dom.borrow_mut().set_text(text_a, "damaged")?;
                state.dom.borrow_mut().set_text(text_b, "damaged")?;
                Err(V8BindingError::Allocation("injected insertion failure"))
            })
            .is_err()
        );
        assert!(eval(scope,"document.getElementById('a').textContent==='source'&&document.getElementById('b').textContent==='target'").is_true());
    }
    #[test]
    fn control_move_undo_requires_both_original_owners_and_restores_together() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=a value=abcd><input id=b value=xy>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let a = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "a")
            .unwrap()
            .unwrap();
        let b = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "b")
            .unwrap()
            .unwrap();
        state
            .dom
            .borrow_mut()
            .edit_control_value(a, 1, 3, "")
            .unwrap();
        state
            .dom
            .borrow_mut()
            .edit_control_value(b, 1, 1, "bc")
            .unwrap();
        native_keyboard::record_control_move(
            &state,
            vec![
                (
                    a,
                    "abcd".into(),
                    (1, 3, "none".into()),
                    "ad".into(),
                    (1, 1, "none".into()),
                ),
                (
                    b,
                    "xy".into(),
                    (1, 1, "none".into()),
                    "xbcy".into(),
                    (3, 3, "none".into()),
                ),
            ],
        )
        .unwrap();
        assert!(native_keyboard::undo_control(&state, b, false).unwrap());
        assert_eq!(state.dom.borrow().control_value(a).unwrap(), "abcd");
        assert_eq!(state.dom.borrow().control_value(b).unwrap(), "xy");
        assert!(native_keyboard::undo_control(&state, a, true).unwrap());
        assert_eq!(state.dom.borrow().control_value(a).unwrap(), "ad");
        assert_eq!(state.dom.borrow().control_value(b).unwrap(), "xbcy");
        assert!(
            eval(
                scope,
                "document.getElementById('a').value='replacement';true"
            )
            .is_true()
        );
        assert!(!native_keyboard::undo_control(&state, b, false).unwrap());
        assert_eq!(
            state.dom.borrow().control_value(b).unwrap(),
            "xbcy",
            "one half cannot be undone after the other owner was replaced"
        );
    }
    #[test]
    fn mixed_control_editor_move_has_one_undo_identity_from_either_focus() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=a value=abcd><div contenteditable id=b>xy</div>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let a = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "a")
            .unwrap()
            .unwrap();
        let b = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "b")
            .unwrap()
            .unwrap();
        let text = state.dom.borrow().children(b).unwrap()[0];
        let source = SelectedSource {
            kind: SelectedKind::Control {
                node: a,
                start: 1,
                end: 3,
                value: "abcd".into(),
            },
            document: state.document_root(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            authority: state.document_authority_epoch(),
            cursor: state.dom.borrow().mutation_cursor(),
            watched: BTreeSet::from([a]),
        };
        let mut store = Store::default();
        store.set_data("text/plain", "bc".encode_utf16().collect());
        store.set_mode(Mode::ReadOnly);
        assert!(
            move_mixed(
                scope,
                &state,
                b,
                &store,
                &source,
                BoundaryPoint {
                    container: text,
                    offset: 1
                },
                "bc"
            )
            .unwrap()
        );
        assert!(eval(scope,"document.getElementById('a').value==='ad'&&document.getElementById('b').textContent==='xbcy'").is_true());
        assert!(native_keyboard::undo_control(&state, a, false).unwrap());
        assert!(eval(scope,"document.getElementById('a').value==='abcd'&&document.getElementById('b').textContent==='xy'").is_true());
        assert!(undo_edit(&state, true).unwrap());
        assert!(eval(scope,"document.getElementById('a').value==='ad'&&document.getElementById('b').textContent==='xbcy'").is_true());
        assert!(
            eval(
                scope,
                "document.getElementById('a').value='replacement';true"
            )
            .is_true()
        );
        assert!(
            !undo_edit(&state, false).unwrap(),
            "document undo cannot restore just its editor half after the control marker was invalidated"
        );
        assert!(eval(scope, "document.getElementById('b').textContent==='xbcy'").is_true());
    }
    #[test]
    fn editor_to_control_move_cancellation_and_redo_preserve_source_ownership() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div contenteditable id=a>abcd</div><input id=b value=xy>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let a = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "a")
            .unwrap()
            .unwrap();
        let b = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "b")
            .unwrap()
            .unwrap();
        let text = state.dom.borrow().children(a).unwrap()[0];
        let range = RangeRecord {
            start_container: text,
            start_offset: 1,
            end_container: text,
            end_offset: 3,
        };
        let source = SelectedSource {
            kind: SelectedKind::Range {
                range,
                host: Some(a),
            },
            document: state.document_root(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            authority: state.document_authority_epoch(),
            cursor: state.dom.borrow().mutation_cursor(),
            watched: BTreeSet::from([a, text]),
        };
        let mut store = Store::default();
        store.set_data("text/plain", "bc".encode_utf16().collect());
        store.set_mode(Mode::ReadOnly);
        assert!(eval(scope,"globalThis.cancelDrop=e=>e.preventDefault();document.getElementById('b').addEventListener('beforeinput',cancelDrop);true").is_true());
        assert!(
            !move_mixed(
                scope,
                &state,
                b,
                &store,
                &source,
                BoundaryPoint {
                    container: b,
                    offset: 1
                },
                "bc"
            )
            .unwrap()
        );
        assert!(eval(scope,"document.getElementById('a').textContent==='abcd'&&document.getElementById('b').value==='xy'").is_true());
        assert!(
            eval(
                scope,
                "document.getElementById('b').removeEventListener('beforeinput',cancelDrop);true"
            )
            .is_true()
        );
        assert!(
            move_mixed(
                scope,
                &state,
                b,
                &store,
                &source,
                BoundaryPoint {
                    container: b,
                    offset: 1
                },
                "bc"
            )
            .unwrap()
        );
        assert!(eval(scope,"document.getElementById('a').textContent==='ad'&&document.getElementById('b').value==='xbcy'").is_true());
        assert!(undo_edit(&state, false).unwrap());
        assert!(native_keyboard::undo_control(&state, b, true).unwrap());
        assert!(eval(scope,"document.getElementById('a').textContent==='ad'&&document.getElementById('b').value==='xbcy'").is_true());
    }
    #[test]
    fn beforeinput_cannot_redirect_move_into_an_uncaptured_third_editor() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<input id=a value=abcd><div contenteditable id=b>xy</div><div contenteditable id=c></div>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let a = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "a")
            .unwrap()
            .unwrap();
        let b = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "b")
            .unwrap()
            .unwrap();
        let text = state.dom.borrow().children(b).unwrap()[0];
        let source = SelectedSource {
            kind: SelectedKind::Control {
                node: a,
                start: 1,
                end: 3,
                value: "abcd".into(),
            },
            document: state.document_root(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            authority: state.document_authority_epoch(),
            cursor: state.dom.borrow().mutation_cursor(),
            watched: BTreeSet::from([a]),
        };
        assert!(eval(scope,"document.getElementById('b').addEventListener('beforeinput',()=>document.getElementById('c').appendChild(document.getElementById('b').firstChild));true").is_true());
        let mut store = Store::default();
        store.set_data("text/plain", "bc".encode_utf16().collect());
        store.set_mode(Mode::ReadOnly);
        assert!(
            !move_mixed(
                scope,
                &state,
                b,
                &store,
                &source,
                BoundaryPoint {
                    container: text,
                    offset: 1
                },
                "bc"
            )
            .unwrap()
        );
        assert!(eval(scope,"document.getElementById('a').value==='abcd'&&document.getElementById('c').textContent==='xy'").is_true());
        assert!(state.undo_history.borrow().undo.is_empty());
    }
    #[test]
    fn accepted_native_drag_orders_events_and_detaches_saved_payloads() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=source draggable=true></div><div id=target></div>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "source")
            .unwrap()
            .unwrap();
        let target = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "target")
            .unwrap()
            .unwrap();
        assert!(eval(scope,r#"globalThis.events=[];globalThis.saved=[];
            const source=document.getElementById('source'),target=document.getElementById('target');
            for(const [node,kinds] of [[source,['dragstart','drag','dragend']],[target,['dragenter','dragover','drop']]]) {
                for(const kind of kinds)node.addEventListener(kind,e=>{
                    if(!e.isTrusted)throw Error('untrusted');events.push(kind);saved.push(e.dataTransfer);
                    if(kind==='dragstart'){e.dataTransfer.setData('text','payload');e.dataTransfer.effectAllowed='copyMove';}
                    else if(kind==='dragover'){if(e.dataTransfer.getData('text')!=='')throw Error('protected leak');e.dataTransfer.dropEffect='move';e.preventDefault();}
                    else if(kind==='drop'){if(e.dataTransfer.getData('text')!=='payload')throw Error('missing payload');e.preventDefault();}
                    else if(kind==='dragend'&&e.dataTransfer.dropEffect!=='move')throw Error('wrong operation');
                });
            }true"#).is_true());
        let init = EventInit::default();
        let mut session = Session::begin(scope, &state, source, Store::default(), &init)
            .unwrap()
            .unwrap();
        assert_eq!(
            session
                .advance(scope, &state, Some(target), DragEffect::Copy, None, &init)
                .unwrap(),
            DragEffect::Move
        );
        assert_eq!(
            session
                .drop_on_target(scope, &state, &init, |_, _, _, _| panic!(
                    "canceled drop skips native default"
                ))
                .unwrap(),
            DragEffect::Move
        );
        assert!(eval(scope,"events.join(',')==='dragstart,drag,dragenter,dragover,drop,dragend' && saved.every(dt=>dt.getData('text')===''&&dt.items.length===0)").is_true());
        assert_eq!(
            session
                .advance(scope, &state, Some(target), DragEffect::Copy, None, &init)
                .unwrap(),
            DragEffect::None
        );
        assert!(eval(scope, "events.length===6").is_true());
    }
}
