//! Native intersection observation snapshots and deferred callback tasks.
use super::*;
#[derive(Clone, Copy, Debug, Default)]
struct Margin {
    value: f64,
    percent: bool,
}
fn margins(text: &str) -> Result<[Margin; 4], V8BindingError> {
    let mut values = Vec::new();
    for part in text.split_ascii_whitespace() {
        let part = part.to_ascii_lowercase();
        let (number, percent, factor) = if let Some(number) = part.strip_suffix('%') {
            (number, true, 1.)
        } else {
            let (unit, factor) = [
                ("px", 1.),
                ("cm", 96. / 2.54),
                ("mm", 96. / 25.4),
                ("q", 96. / 101.6),
                ("in", 96.),
                ("pt", 96. / 72.),
                ("pc", 16.),
            ]
            .into_iter()
            .find(|(unit, _)| part.ends_with(unit))
            .ok_or_else(|| {
                error(
                    "SyntaxError",
                    "intersection margin requires absolute lengths or percentages",
                )
            })?;
            (part.strip_suffix(unit).unwrap(), false, factor)
        };
        let value = number
            .parse::<f64>()
            .map_err(|_| error("SyntaxError", "invalid intersection margin"))?
            * factor;
        if !value.is_finite() || values.len() == 4 {
            return Err(error("SyntaxError", "invalid intersection margin"));
        }
        values.push(Margin {
            value: if percent { value } else { value.trunc() },
            percent,
        });
    }
    Ok(match values.as_slice() {
        [] => [Margin::default(); 4],
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return Err(error("SyntaxError", "invalid intersection margin")),
    })
}
fn margin_text(values: &[Margin; 4]) -> String {
    values
        .iter()
        .map(|value| format!("{}{}", value.value, if value.percent { "%" } else { "px" }))
        .collect::<Vec<_>>()
        .join(" ")
}
struct Target {
    object: v8::Global<v8::Object>,
    previous: Option<(usize, bool, bool)>,
    last: Option<std::time::Instant>,
}
pub(super) struct Observer {
    object: Option<v8::Global<v8::Object>>,
    root: Option<v8::Weak<v8::Object>>,
    root_margin: [Margin; 4],
    scroll_margin: [Margin; 4],
    thresholds: Vec<f64>,
    delay: f64,
    track_visibility: bool,
    targets: Vec<Rc<RefCell<Target>>>,
    entries: Vec<Entry>,
}
#[derive(Clone)]
pub(super) struct Entry {
    target: Option<Rc<v8::Weak<v8::Object>>>,
    time: f64,
    root: Option<BindingRect>,
    bounding: BindingRect,
    intersection: BindingRect,
    ratio: f64,
    intersecting: bool,
    visible: bool,
}

fn entry_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = v8::String::new(scope, "oryn.observer_entry.target")
        .ok_or(V8BindingError::Allocation("entry target key"))?;
    let key = v8::Private::for_api(scope, Some(key));
    object
        .get_private(scope, key)
        .ok_or(V8BindingError::JavaScriptException)
}
fn set_entry_target(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    target: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    let key = v8::String::new(scope, "oryn.observer_entry.target")
        .ok_or(V8BindingError::Allocation("entry target key"))?;
    let key = v8::Private::for_api(scope, Some(key));
    if object.set_private(scope, key, target.into()) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(())
}
#[derive(Default)]
pub(super) struct State {
    observers: BTreeMap<u64, Rc<RefCell<Observer>>>,
    pending: VecDeque<u64>,
    dirty: bool,
    retry: Option<std::time::Instant>,
    dependencies: Vec<(std::rc::Weak<V8BindingState>, u64)>,
    #[cfg(test)]
    fail_prepare: bool,
}
fn error(kind: &str, message: &str) -> V8BindingError {
    if matches!(kind, "SyntaxError" | "RangeError") {
        // The shared typed route creates DOMException SyntaxError and the realm's
        // intrinsic RangeError. A prefixed Message is always a TypeError there.
        V8BindingError::Unsupported {
            interface: "IntersectionObserver".into(),
            member: kind.into(),
        }
    } else {
        V8BindingError::Message(format!("{kind}: {message}"))
    }
}
fn option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: Option<v8::Local<v8::Object>>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let Some(options) = options else {
        return Ok(v8::undefined(scope).into());
    };
    let key =
        v8::String::new(scope, name).ok_or(V8BindingError::Allocation("intersection option"))?;
    options
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)
}
fn text(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    default: &str,
) -> Result<String, V8BindingError> {
    if value.is_undefined() {
        return Ok(default.into());
    }
    let value = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    rust_string_preserving_utf16(scope, value)
}
fn node(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    element: bool,
) -> Result<(Rc<V8BindingState>, NodeId), V8BindingError> {
    let (owner, node) = node_identity::argument(scope, object.into())?
        .ok_or_else(|| error("TypeError", "intersection target requires a native node"))?;
    let valid = matches!(
        &owner.dom.borrow().arena().get(node)?.kind,
        NodeKind::Element { .. }
    ) || !element
        && matches!(
            &owner.dom.borrow().arena().get(node)?.kind,
            NodeKind::Document
        );
    if !valid {
        return Err(error(
            "TypeError",
            "intersection target requires an Element",
        ));
    }
    Ok((owner, node))
}
pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    let callback = v8::Local::<v8::Function>::try_from(args.get(0)).map_err(|_| {
        error(
            "TypeError",
            "IntersectionObserver callback must be callable",
        )
    })?;
    let options = if args.get(1).is_null_or_undefined() {
        None
    } else {
        Some(
            v8::Local::<v8::Object>::try_from(args.get(1))
                .map_err(|_| error("TypeError", "intersection options must be a dictionary"))?,
        )
    };
    // Dictionary members are read in Web IDL lexicographic order.
    let delay = option(scope, options, "delay")?;
    let delay = if delay.is_undefined() {
        0.
    } else {
        delay
            .int32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)? as f64
    };
    let root = option(scope, options, "root")?;
    let root = if root.is_null_or_undefined() {
        None
    } else {
        let root = v8::Local::<v8::Object>::try_from(root)
            .map_err(|_| error("TypeError", "intersection root must be Element or Document"))?;
        node(scope, root, false)?;
        Some(root)
    };
    let value = option(scope, options, "rootMargin")?;
    let root_margin = text(scope, value, "0px")?;
    let value = option(scope, options, "scrollMargin")?;
    let scroll_margin = text(scope, value, "0px")?;
    let value = option(scope, options, "threshold")?;
    let mut thresholds = Vec::new();
    let mut convert = |scope: &mut v8::PinScope, value: v8::Local<v8::Value>| {
        if thresholds.len() >= 4096 {
            return Err(V8BindingError::Allocation("intersection thresholds"));
        }
        let value = value
            .number_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        if !value.is_finite() {
            return Err(error("TypeError", "intersection threshold must be finite"));
        }
        thresholds.push(value);
        Ok(())
    };
    if value.is_undefined() {
        thresholds.push(0.)
    } else if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
        let key = v8::Symbol::get_iterator(scope);
        let method = object
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if method.is_null_or_undefined() {
            convert(scope, value)?
        } else {
            let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| {
                error(
                    "TypeError",
                    "intersection threshold iterator must be callable",
                )
            })?;
            iterable_conversion::consume(scope, object, method, |scope, value| {
                convert(scope, value)
            })?;
        }
    } else {
        convert(scope, value)?;
    }
    let track_visibility = option(scope, options, "trackVisibility")?.boolean_value(scope);
    let delay = if track_visibility {
        delay.max(100.)
    } else {
        delay
    };
    let root_margin = margins(&root_margin)?;
    let scroll_margin = margins(&scroll_margin)?;
    if thresholds.iter().any(|value| !(0. ..=1.).contains(value)) {
        return Err(error("RangeError", "intersection threshold outside [0,1]"));
    }
    thresholds.sort_by(f64::total_cmp);
    if thresholds.is_empty() {
        thresholds.push(0.)
    }

    if let Some(root) = root {
        node(scope, root, false)?;
        let key = v8::String::new(scope, "oryn.intersection.root")
            .ok_or(V8BindingError::Allocation("intersection root key"))?;
        let key = v8::Private::for_api(scope, Some(key));
        if args.this().set_private(scope, key, root.into()) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
    }
    let root = root.map(|root| v8::Weak::new(scope, root));
    // A callback may capture its observer. Retain it through a private JS edge,
    // allowing an inactive unreachable cycle to be collected with its wrapper.
    let key = v8::String::new(scope, "oryn.intersection_observers.callback")
        .ok_or(V8BindingError::Allocation("observer callback key"))?;
    let key = v8::Private::for_api(scope, Some(key));
    if args.this().set_private(scope, key, callback.into()) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    let observer = Rc::new(RefCell::new(Observer {
        object: None,
        root,
        root_margin,
        scroll_margin,
        thresholds,
        delay,
        track_visibility,
        targets: Vec::new(),
        entries: Vec::new(),
    }));
    let id = register_platform_object(state, PlatformObjectRecord::IntersectionObserver(observer));
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}
#[cfg(test)]
fn rect_intersection(a: BindingRect, b: BindingRect) -> (BindingRect, bool) {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (
        BindingRect {
            x,
            y,
            width: (right - x).max(0.),
            height: (bottom - y).max(0.),
        },
        right >= x && bottom >= y,
    )
}
fn expand(mut rect: BindingRect, margin: [Margin; 4]) -> BindingRect {
    let value = |index: usize| {
        if margin[index].percent {
            margin[index].value * rect.width / 100.
        } else {
            margin[index].value
        }
    };
    let (top, right, bottom, left) = (value(0), value(1), value(2), value(3));
    rect.x -= left;
    rect.y -= top;
    rect.width = (rect.width + left + right).max(0.);
    rect.height = (rect.height + top + bottom).max(0.);
    rect
}
fn project(rect: BindingRect, matrix: [f64; 16]) -> Option<BindingRect> {
    let mut bounds = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for (x, y) in [
        (rect.x, rect.y),
        (rect.x + rect.width, rect.y),
        (rect.x, rect.y + rect.height),
        (rect.x + rect.width, rect.y + rect.height),
    ] {
        let w = matrix[3] * x + matrix[7] * y + matrix[15];
        if !w.is_finite() || w <= 0. {
            return None;
        }
        let x = (matrix[0] * x + matrix[4] * y + matrix[12]) / w;
        let y = (matrix[1] * x + matrix[5] * y + matrix[13]) / w;
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        bounds = (
            bounds.0.min(x),
            bounds.1.min(y),
            bounds.2.max(x),
            bounds.3.max(y),
        );
    }
    Some(BindingRect {
        x: bounds.0,
        y: bounds.1,
        width: bounds.2 - bounds.0,
        height: bounds.3 - bounds.1,
    })
}
fn css_rect(rect: oryn_common::v4::CssRect) -> BindingRect {
    BindingRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}
fn identity() -> [f64; 16] {
    [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ]
}
fn bounding(state: &V8BindingState, node: NodeId) -> BindingRect {
    let layout = state.layout.borrow();
    if let Some(rects) = layout.client_rects.get(&node_key(node)) {
        return bounding_client_rect(rects);
    }
    match layout.geometry.get(&node_key(node)) {
        Some(BindingGeometry::Available(rect)) => *rect,
        _ => BindingRect::default(),
    }
}
fn viewport(state: &V8BindingState) -> BindingRect {
    let (width, height) = state.layout.borrow().viewport.unwrap_or((0., 0.));
    BindingRect {
        x: 0.,
        y: 0.,
        width,
        height,
    }
}
fn same_origin(a: &V8BindingState, b: &V8BindingState) -> bool {
    std::ptr::eq(a, b) || {
        let a = current_platform_origin(a);
        a != "null" && a == current_platform_origin(b)
    }
}

/// Intersections are calculated in the clip's own plane. Applying a viewport
/// axis-aligned bounding box first incorrectly clips rotated scroll containers.
#[cfg(test)]
fn apply_clip(
    rect: BindingRect,
    clip: &oryn_common::v4::PaintClip,
    margin: [Margin; 4],
) -> Option<(BindingRect, bool)> {
    let inverse = inverse_matrix_components(clip.matrix)?;
    let mut local = project(rect, inverse)?;
    if let Some(shape) = &clip.shape {
        let Some(clipped) = shape.intersection_bounds(oryn_common::v4::CssRect {
            x: local.x,
            y: local.y,
            width: local.width,
            height: local.height,
        }) else {
            return Some((BindingRect::default(), false));
        };
        local = css_rect(clipped);
    }
    let mut applied = margin;
    if !clip.scroll_y {
        applied[0] = Margin::default();
        applied[2] = Margin::default();
    }
    if !clip.scroll_x {
        applied[1] = Margin::default();
        applied[3] = Margin::default();
    }
    let bounds = expand(css_rect(clip.rect), applied);
    let x = if clip.clip_x {
        local.x.max(bounds.x)
    } else {
        local.x
    };
    let y = if clip.clip_y {
        local.y.max(bounds.y)
    } else {
        local.y
    };
    let right = if clip.clip_x {
        (local.x + local.width).min(bounds.x + bounds.width)
    } else {
        local.x + local.width
    };
    let bottom = if clip.clip_y {
        (local.y + local.height).min(bounds.y + bounds.height)
    } else {
        local.y + local.height
    };
    let intersects = right >= x && bottom >= y;
    let result = project(
        BindingRect {
            x,
            y,
            width: (right - x).max(0.),
            height: (bottom - y).max(0.),
        },
        clip.matrix,
    )?;
    Some((result, intersects))
}
fn top_document(state: &Rc<V8BindingState>) -> Result<Rc<V8BindingState>, V8BindingError> {
    let mut current = state.clone();
    let mut seen = BTreeSet::new();
    for _ in 0..1024 {
        if !seen.insert(current.instance_id) {
            break;
        }
        let relation = current.frame_viewport_parent.borrow().clone();
        let Some((parent, frame, connection)) = relation else {
            return Ok(current);
        };
        let Some(parent) = parent.upgrade() else {
            return Ok(current);
        };
        if parent.execution_retired()
            || parent.dom.borrow().frame_context_generation(frame) != connection
        {
            return Ok(current);
        }
        current = parent;
    }
    Err(V8BindingError::Allocation("intersection root ancestry"))
}
fn region_rect(rect: BindingRect) -> oryn_common::basic_shape::Shape {
    oryn_common::basic_shape::Shape::Polygon {
        points: vec![
            [rect.x, rect.y],
            [rect.x + rect.width, rect.y],
            [rect.x + rect.width, rect.y + rect.height],
            [rect.x, rect.y + rect.height],
        ],
        even_odd: false,
    }
}
struct Geometry {
    root: Option<BindingRect>,
    bounding: BindingRect,
    intersection: BindingRect,
    intersecting: bool,
    dependencies: Vec<(std::rc::Weak<V8BindingState>, u64)>,
}
/// Resolve the native embedding chain without entering layout or invoking author
/// code. Every step validates the original frame connection and document owner.
/// Detached/replaced frames never inherit the replacement document's viewport.
fn geometry(
    scope: &mut v8::PinScope,
    observer_state: &Rc<V8BindingState>,
    observer: &Observer,
    target: &Target,
) -> Result<Geometry, V8BindingError> {
    let object = v8::Local::new(scope, &target.object);
    let (owner, target_node) = node(scope, object, true)?;
    let bounds = bounding(&owner, target_node);
    let mut result = Geometry {
        root: None,
        bounding: bounds,
        intersection: BindingRect::default(),
        intersecting: false,
        dependencies: vec![(Rc::downgrade(&owner), owner.native_paint_revision()?)],
    };
    let root = if let Some(root) = &observer.root {
        let object = root
            .to_local(scope)
            .ok_or(V8BindingError::Allocation("intersection root lifetime"))?;
        Some(node(scope, object, false)?)
    } else {
        None
    };
    let implicit_root = top_document(observer_state)?;
    result.dependencies.push((
        Rc::downgrade(&implicit_root),
        implicit_root.native_paint_revision()?,
    ));
    if let Some((root, _)) = &root {
        result
            .dependencies
            .push((Rc::downgrade(root), root.native_paint_revision()?));
    }
    let mut current = owner.clone();
    let mut cursor = target_node;
    let mut transform = identity();
    let mut regions = vec![(region_rect(bounds), identity())];
    let mut visited = BTreeSet::new();
    let same = match &root {
        Some((root, _)) => same_origin(&owner, root),
        None => same_origin(&owner, &implicit_root),
    };
    let margins = if same {
        observer.scroll_margin
    } else {
        [Margin::default(); 4]
    };
    if same {
        result.root = Some(BindingRect::default());
    }
    if root
        .as_ref()
        .is_some_and(|(root, _)| !Rc::ptr_eq(root, &owner))
    {
        result.bounding = BindingRect::default();
        return Ok(result);
    }
    let has_box = {
        let layout = owner.layout.borrow();
        layout.hit_projections.contains_key(&node_key(target_node))
            || matches!(
                layout.geometry.get(&node_key(target_node)),
                Some(BindingGeometry::Available(_))
            )
            || layout
                .client_rects
                .get(&node_key(target_node))
                .is_some_and(|rects| !rects.is_empty())
    };
    if !has_box || !owner.dom.borrow().is_connected(target_node)? {
        result.bounding = BindingRect::default();
        return Ok(result);
    }
    for _ in 0..1024 {
        if !visited.insert(current.instance_id)
            || current.execution_retired()
            || current.execution_suspended()
            || !current.dom.borrow().is_connected(cursor)?
        {
            return Ok(result);
        }
        result
            .dependencies
            .push((Rc::downgrade(&current), current.native_paint_revision()?));
        let mut allowed_clips = None;
        let reached = if let Some((root_state, root_node)) = &root {
            if Rc::ptr_eq(&current, root_state) {
                // An explicit element root only observes descendants in its own document.
                let mut ancestor = Some(cursor);
                let mut found = false;
                let mut below = BTreeSet::new();
                for _ in 0..65536 {
                    let Some(value) = ancestor else { break };
                    if value == *root_node {
                        found = true;
                        break;
                    }
                    below.insert(node_key(value));
                    ancestor = current.dom.borrow().flat_tree_parent(value)?;
                }
                if !found {
                    return Ok(result);
                }
                if *root_node != current.document_root() {
                    allowed_clips = Some(below);
                }
                Some(*root_node)
            } else {
                None
            }
        } else if Rc::ptr_eq(&current, &implicit_root) {
            Some(current.document_root())
        } else {
            None
        };
        {
            let layout = current.layout.borrow();
            if let Some(projection) = layout.hit_projections.get(&node_key(cursor)) {
                for clip in &projection.clips {
                    if reached == Some(current.document_root()) && clip.owner.is_none() {
                        continue;
                    }
                    if allowed_clips.as_ref().is_some_and(|allowed| {
                        clip.owner.is_none_or(|owner| !allowed.contains(&owner))
                    }) {
                        continue;
                    }
                    let Some(inverse) = inverse_matrix_components(transform) else {
                        return Ok(result);
                    };
                    let map = multiply_matrix_components(inverse, clip.matrix);
                    let shape = if let Some(shape) = &clip.shape {
                        shape.clone()
                    } else {
                        let mut margin = margins;
                        if !clip.scroll_y {
                            margin[0] = Margin::default();
                            margin[2] = Margin::default();
                        }
                        if !clip.scroll_x {
                            margin[1] = Margin::default();
                            margin[3] = Margin::default();
                        }
                        let mut rect = expand(css_rect(clip.rect), margin);
                        if !clip.clip_x || !clip.clip_y {
                            let Some(inverse) = inverse_matrix_components(map) else {
                                return Ok(result);
                            };
                            let Some(target) = project(bounds, inverse) else {
                                return Ok(result);
                            };
                            if !clip.clip_x {
                                rect.x = target.x;
                                rect.width = target.width;
                            }
                            if !clip.clip_y {
                                rect.y = target.y;
                                rect.height = target.height;
                            }
                        }
                        region_rect(rect)
                    };
                    if regions.len() >= 4096 {
                        return Err(V8BindingError::Allocation(
                            "intersection clip region budget",
                        ));
                    }
                    regions.push((shape, map));
                }
            }
        }
        if let Some(root_node) = reached {
            let mut root_rect = if root_node == current.document_root() {
                viewport(&current)
            } else {
                let layout = current.layout.borrow();
                layout
                    .hit_projections
                    .get(&node_key(root_node))
                    .and_then(|projection| {
                        let clips = layout
                            .styles
                            .get(&node_key(root_node))
                            .is_some_and(|style| {
                                ["overflow-x", "overflow-y"].iter().any(|name| {
                                    style.get(*name).is_some_and(|value| value != "visible")
                                })
                            });
                        project(
                            css_rect(if clips {
                                projection.box_model.padding_box
                            } else {
                                projection.box_model.border_box
                            }),
                            projection.matrix,
                        )
                    })
                    .unwrap_or_else(|| bounding(&current, root_node))
            };
            if same {
                let scrollable = root_node == current.document_root()
                    || current
                        .layout
                        .borrow()
                        .hit_projections
                        .get(&node_key(root_node))
                        .is_some_and(|projection| projection.scroll.is_some());
                let combined = std::array::from_fn(|index| {
                    let root = observer.root_margin[index];
                    let scroll = if scrollable {
                        observer.scroll_margin[index]
                    } else {
                        Margin::default()
                    };
                    Margin {
                        value: (if root.percent {
                            root.value * root_rect.width / 100.
                        } else {
                            root.value
                        }) + (if scroll.percent {
                            scroll.value * root_rect.width / 100.
                        } else {
                            scroll.value
                        }),
                        percent: false,
                    }
                });
                root_rect = expand(root_rect, combined);
            }
            let Some(inverse) = inverse_matrix_components(transform) else {
                return Ok(result);
            };
            regions.push((region_rect(root_rect), inverse));
            let intersection =
                oryn_common::basic_shape::intersect_regions(&regions).map_err(|error| {
                    V8BindingError::Message(format!("native intersection geometry: {error}"))
                })?;
            result.root = if same { Some(root_rect) } else { None };
            result.intersection = intersection.map(css_rect).unwrap_or_default();
            result.intersecting = intersection.is_some();
            return Ok(result);
        }
        let relation = current.frame_viewport_parent.borrow().clone();
        let Some((parent, frame, connection)) = relation else {
            return Ok(result);
        };
        let Some(parent) = parent.upgrade() else {
            return Ok(result);
        };
        if parent.dom.borrow().frame_context_generation(frame) != connection
            || parent.dom.borrow().node_document(frame)? != Some(parent.document_root())
        {
            return Ok(result);
        }
        let Some(inverse) = inverse_matrix_components(transform) else {
            return Ok(result);
        };
        regions.push((region_rect(expand(viewport(&current), margins)), inverse));
        let projection = parent
            .layout
            .borrow()
            .hit_projections
            .get(&node_key(frame))
            .cloned();
        let Some(projection) = projection else {
            return Ok(result);
        };
        let mut translation = identity();
        translation[12] = projection.box_model.content_box.x;
        translation[13] = projection.box_model.content_box.y;
        let step = multiply_matrix_components(projection.matrix, translation);
        transform = multiply_matrix_components(step, transform);
        current = parent;
        cursor = frame;
    }
    Err(V8BindingError::Allocation("intersection embedding depth"))
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    #[test]
    fn retired_queued_observer_keeps_records_only_through_its_wrapper() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let target = state.dom.borrow().find_by_id("target").unwrap();
        state.layout.borrow_mut().viewport = Some((100., 100.));
        state.layout.borrow_mut().geometry.insert(
            node_key(target),
            BindingGeometry::Available(BindingRect {
                x: 0.,
                y: 0.,
                width: 10.,
                height: 10.,
            }),
        );
        let weak = {
            v8::scope!(let inner,scope);
            let code=v8::String::new(inner,"globalThis.o=new IntersectionObserver(()=>{});(()=>{const target=document.getElementById('target');target.owner=o;o.observe(target);return o})()").unwrap();
            let object = v8::Script::compile(inner, code, None)
                .unwrap()
                .run(inner)
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap();
            sample(inner, &state).unwrap();
            retire(&state);
            let weak = v8::Weak::new(inner, object);
            let code = v8::String::new(
                inner,
                "document.getElementById('target').remove();delete globalThis.o;true",
            )
            .unwrap();
            assert!(
                v8::Script::compile(inner, code, None)
                    .unwrap()
                    .run(inner)
                    .unwrap()
                    .is_true()
            );
            weak
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(weak.to_local(scope).is_none());
    }
    #[test]
    fn constructed_entry_and_target_cycles_are_collectable() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body/>").into_runtime().unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let weak = {
            v8::scope!(let inner,scope);
            let source=v8::String::new(inner,"(()=>{const target=document.createElement('div');const entry=new IntersectionObserverEntry({time:0,rootBounds:null,boundingClientRect:{},intersectionRect:{},intersectionRatio:0,isIntersecting:false,isVisible:false,target});target.saved=entry;if(entry.target!==target)throw Error('target identity');return entry})()").unwrap();
            let object = v8::Script::compile(inner, source, None)
                .unwrap()
                .run(inner)
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap();
            v8::Weak::new(inner, object)
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(weak.to_local(scope).is_none());
    }
    #[test]
    fn inactive_callback_cycles_collect_and_active_observation_keeps_callback_alive() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        fn value<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Object> {
            let source = v8::String::new(scope, source).unwrap();
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap()
        }
        let idle = {
            v8::scope!(let inner,scope);
            let object = value(
                inner,
                "(()=>{const observer=new IntersectionObserver(()=>observer);return observer})()",
            );
            v8::Weak::new(inner, object)
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(idle.to_local(scope).is_none());
        let active = {
            v8::scope!(let inner,scope);
            let object = value(
                inner,
                "(()=>{const observer=new IntersectionObserver(()=>observer);observer.observe(document.getElementById('target'));return observer})()",
            );
            v8::Weak::new(inner, object)
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        {
            v8::scope!(let inner,scope);
            let object = active
                .to_local(inner)
                .expect("native observation owns observer");
            let key = v8::String::new(inner, "disconnect").unwrap();
            let method = object
                .get(inner, key.into())
                .unwrap()
                .try_cast::<v8::Function>()
                .unwrap();
            method.call(inner, object.into(), &[]).unwrap();
        }
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(active.to_local(scope).is_none());
    }
    #[test]
    fn edge_contact_is_intersecting_even_with_zero_area() {
        let (a, yes) = rect_intersection(
            BindingRect {
                x: 0.,
                y: 0.,
                width: 10.,
                height: 10.,
            },
            BindingRect {
                x: 10.,
                y: 0.,
                width: 10.,
                height: 10.,
            },
        );
        assert!(yes);
        assert_eq!(a.width, 0.);
        let (_, no) = rect_intersection(
            a,
            BindingRect {
                x: 11.,
                y: 0.,
                width: 10.,
                height: 10.,
            },
        );
        assert!(!no);
    }
    #[test]
    fn percentages_use_undilated_width_for_all_four_sides() {
        let rect = expand(
            BindingRect {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 20.,
            },
            margins("10% 20%").unwrap(),
        );
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (-20., -10., 140., 40.)
        );
        assert!(margins("auto").is_err());
        assert!(margins("1px 2px 3px 4px 5px").is_err());
    }
    #[test]
    fn pinned_absolute_units_and_empty_margin_are_normalized() {
        assert_eq!(margin_text(&margins(" ").unwrap()), "0px 0px 0px 0px");
        for (unit, expected) in [
            ("cm", 377.),
            ("mm", 37.),
            ("q", 9.),
            ("in", 960.),
            ("pt", 13.),
            ("pc", 160.),
            ("PX", 10.),
        ] {
            assert_eq!(margins(&format!("10{unit}")).unwrap()[0].value, expected);
        }
        assert!(margins("1em").is_err());
    }
    #[test]
    fn single_axis_clipping_does_not_clip_the_other_axis() {
        let clip = oryn_common::v4::PaintClip {
            shape: None,
            owner: None,
            scroll_x: false,
            scroll_y: false,
            matrix: identity(),
            rect: oryn_common::v4::CssRect {
                x: 0.,
                y: 0.,
                width: 10.,
                height: 10.,
            },
            clip_x: true,
            clip_y: false,
        };
        let (rect, yes) = apply_clip(
            BindingRect {
                x: 5.,
                y: -40.,
                width: 10.,
                height: 100.,
            },
            &clip,
            [Margin::default(); 4],
        )
        .unwrap();
        assert!(yes);
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (5., -40., 5., 100.)
        );
    }
    #[test]
    fn scroll_margin_never_expands_overflow_clip() {
        let mut clip = oryn_common::v4::PaintClip {
            shape: None,
            owner: Some(1),
            scroll_x: false,
            scroll_y: false,
            matrix: identity(),
            rect: oryn_common::v4::CssRect {
                x: 0.,
                y: 0.,
                width: 10.,
                height: 10.,
            },
            clip_x: true,
            clip_y: true,
        };
        let target = BindingRect {
            x: 11.,
            y: 1.,
            width: 1.,
            height: 1.,
        };
        assert!(
            !apply_clip(target, &clip, margins("2px").unwrap())
                .unwrap()
                .1
        );
        clip.scroll_x = true;
        assert!(
            apply_clip(target, &clip, margins("2px").unwrap())
                .unwrap()
                .1
        );
    }
    #[test]
    fn ellipse_clipping_uses_curve_extrema_not_bounding_box() {
        let shape = oryn_common::basic_shape::parse(
            "circle(50px at center)",
            oryn_common::v4::CssRect {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 100.,
            },
            16.,
            (100., 100.),
        )
        .unwrap()
        .unwrap();
        let clip = oryn_common::v4::PaintClip {
            shape: Some(shape),
            owner: Some(1),
            scroll_x: false,
            scroll_y: false,
            matrix: identity(),
            rect: oryn_common::v4::CssRect {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 100.,
            },
            clip_x: true,
            clip_y: true,
        };
        let (_, outside) = apply_clip(
            BindingRect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
            &clip,
            [Margin::default(); 4],
        )
        .unwrap();
        assert!(!outside);
        let (rect, inside) = apply_clip(
            BindingRect {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 10.,
            },
            &clip,
            [Margin::default(); 4],
        )
        .unwrap();
        assert!(inside);
        assert!((rect.x - 20.).abs() < 1e-10);
        assert!((rect.width - 60.).abs() < 1e-10);
    }
    #[test]
    fn singular_clip_cannot_publish_an_unclipped_intersection() {
        let clip = oryn_common::v4::PaintClip {
            shape: None,
            owner: None,
            scroll_x: false,
            scroll_y: false,
            matrix: [0.; 16],
            rect: oryn_common::v4::CssRect {
                x: 0.,
                y: 0.,
                width: 10.,
                height: 10.,
            },
            clip_x: true,
            clip_y: true,
        };
        assert!(apply_clip(BindingRect::default(), &clip, [Margin::default(); 4]).is_none());
    }
}

fn pin_queued_targets(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    targets: &[v8::Local<v8::Value>],
) -> Result<(), V8BindingError> {
    let key = v8::String::new(scope, "oryn.intersection.queue_targets")
        .ok_or(V8BindingError::Allocation("queued target key"))?;
    let key = v8::Private::for_api(scope, Some(key));
    let array = v8::Array::new_with_elements(scope, targets);
    if object.set_private(scope, key, array.into()) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(())
}
fn entries_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    entries: &[Entry],
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let mut ids = Vec::new();
    let result = (|| {
        let mut values = Vec::new();
        values
            .try_reserve(entries.len())
            .map_err(|_| V8BindingError::Allocation("intersection entries"))?;
        for entry in entries {
            let mut public = entry.clone();
            let target = public
                .target
                .as_ref()
                .and_then(|target| target.to_local(scope))
                .ok_or(V8BindingError::Allocation("pending entry target"))?;
            public.target = None;
            let id = register_platform_object(
                state,
                PlatformObjectRecord::IntersectionObserverEntry(public),
            );
            ids.push(id);
            let object = new_platform_object(scope, state, "IntersectionObserverEntry", id)?;
            set_entry_target(scope, object, target)?;
            values.push(object.into());
        }
        Ok(v8::Array::new_with_elements(scope, &values).into())
    })();
    if result.is_err() {
        for id in ids {
            state.platform_objects.borrow_mut().remove(&id);
            state.platform_wrappers.borrow_mut().remove(&id);
        }
    }
    result
}
pub(super) fn retire(state: &V8BindingState) {
    let mut native = state.intersection_observers.borrow_mut();
    for observer in native.observers.values() {
        let mut observer = observer.borrow_mut();
        observer.targets.clear();
        observer.object = None;
    }
    // Entry records belong to the observer object, not the retired task queue.
    native.observers.clear();
    native.pending.clear();
    native.dependencies.clear();
    native.retry = None;
    native.dirty = false;
}
fn release_idle(state: &V8BindingState, id: u64, observer: &Rc<RefCell<Observer>>) {
    if observer.borrow().targets.is_empty() && observer.borrow().entries.is_empty() {
        observer.borrow_mut().object = None;
        let mut native = state.intersection_observers.borrow_mut();
        native.observers.remove(&id);
        native.pending.retain(|pending| *pending != id);
        if native.observers.is_empty() {
            native.dependencies.clear();
            native.dirty = false;
        }
        if native.pending.is_empty() {
            native.retry = None;
        }
    }
}
pub(super) fn call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    // Entry wrappers register new platform records. Keep the observer itself,
    // never the registry borrow, across allocation and publication.
    let observer = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::IntersectionObserver(observer) = &*record else {
            return Err(error("TypeError", "IntersectionObserver receiver"));
        };
        observer.clone()
    };
    if member == "takeRecords" {
        let entries = observer.borrow().entries.clone();
        let array = entries_array(scope, state, &entries)?;
        pin_queued_targets(scope, object, &[])?;
        observer.borrow_mut().entries.clear();
        state
            .intersection_observers
            .borrow_mut()
            .pending
            .retain(|pending| *pending != id);
        release_idle(state, id, &observer);
        return Ok(Some(array));
    }
    if member == "disconnect" {
        observer.borrow_mut().targets.clear();
        release_idle(state, id, &observer);
        return Ok(Some(v8::undefined(scope).into()));
    }
    let target = v8::Local::<v8::Object>::try_from(args.get(0))
        .map_err(|_| error("TypeError", "intersection target must be an Element"))?;
    node(scope, target, true)?;
    let existing = observer
        .borrow()
        .targets
        .iter()
        .position(|entry| v8::Local::new(scope, &entry.borrow().object) == target);
    if member == "unobserve" {
        if let Some(index) = existing {
            observer.borrow_mut().targets.remove(index);
        }
        release_idle(state, id, &observer);
        return Ok(Some(v8::undefined(scope).into()));
    }
    if member != "observe" {
        return Ok(None);
    }
    if existing.is_some() {
        return Ok(Some(v8::undefined(scope).into()));
    }
    {
        let observers = state.intersection_observers.borrow();
        if !observers.observers.contains_key(&id) && observers.observers.len() >= 1024 {
            return Err(V8BindingError::Allocation("intersection observers"));
        }
    }
    {
        let mut observer = observer.borrow_mut();
        if observer.targets.len() >= 4096 {
            return Err(V8BindingError::Allocation("intersection targets"));
        }
        observer
            .targets
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("intersection targets"))?;
        observer.targets.push(Rc::new(RefCell::new(Target {
            object: v8::Global::new(scope, target),
            previous: None,
            last: None,
        })));
        observer.object = Some(v8::Global::new(scope, object));
    }
    if state.execution_retired() {
        let mut observer = observer.borrow_mut();
        observer.targets.clear();
        observer.object = None;
        return Ok(Some(v8::undefined(scope).into()));
    }
    let mut observers = state.intersection_observers.borrow_mut();
    observers.observers.insert(id, observer.clone());
    observers.dirty = true;
    Ok(Some(v8::undefined(scope).into()))
}
fn rect_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    rect: BindingRect,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let template = state
        .instance_templates
        .borrow()
        .get("DOMRectReadOnly")
        .and_then(|template| template.to_global(scope))
        .ok_or(V8BindingError::MissingInterface("DOMRectReadOnly"))?;
    let object = v8::Local::new(scope, &template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("intersection rectangle"))?;
    initialize_rect_components(
        scope,
        object,
        [rect.x, rect.y, rect.width, rect.height],
        true,
    )?;
    Ok(object.into())
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    let record = platform_object(state, id)?;
    match &*record {
        PlatformObjectRecord::IntersectionObserver(observer) => {
            let observer = observer.borrow();
            match member {
                "root" => Ok(observer
                    .root
                    .as_ref()
                    .and_then(|root| root.to_local(scope).map(Into::into))
                    .unwrap_or_else(|| v8::null(scope).into())),
                "rootMargin" => v8_string(scope, &margin_text(&observer.root_margin)),
                "scrollMargin" => v8_string(scope, &margin_text(&observer.scroll_margin)),
                "delay" => Ok(v8::Number::new(scope, observer.delay).into()),
                "trackVisibility" => Ok(v8::Boolean::new(scope, observer.track_visibility).into()),
                "thresholds" => {
                    let values = observer
                        .thresholds
                        .iter()
                        .map(|value| v8::Number::new(scope, *value).into())
                        .collect::<Vec<_>>();
                    let array = v8::Array::new_with_elements(scope, &values);
                    array
                        .set_integrity_level(scope, v8::IntegrityLevel::Frozen)
                        .ok_or(V8BindingError::Allocation("intersection thresholds"))?;
                    Ok(array.into())
                }
                _ => Err(error("TypeError", "intersection observer member")),
            }
        }
        PlatformObjectRecord::IntersectionObserverEntry(entry) => match member {
            "target" => entry_target(scope, object),
            "time" => Ok(v8::Number::new(scope, entry.time).into()),
            "intersectionRatio" => Ok(v8::Number::new(scope, entry.ratio).into()),
            "isIntersecting" => Ok(v8::Boolean::new(scope, entry.intersecting).into()),
            "isVisible" => Ok(v8::Boolean::new(scope, entry.visible).into()),
            "rootBounds" => match entry.root {
                Some(rect) => rect_object(scope, state, rect),
                None => Ok(v8::null(scope).into()),
            },
            "boundingClientRect" => rect_object(scope, state, entry.bounding),
            "intersectionRect" => rect_object(scope, state, entry.intersection),
            _ => Err(error("TypeError", "intersection entry member")),
        },
        _ => Err(error("TypeError", "intersection receiver")),
    }
}
/// Called only after all geometry in the document tree is current. Native
/// preparation of the complete sample batch precedes updates to threshold state.
pub(super) fn sample(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    if !state.document_rendering_allowed() {
        return Ok(());
    }
    let now = std::time::Instant::now();
    let observers = state
        .intersection_observers
        .borrow()
        .observers
        .iter()
        .map(|(id, observer)| (*id, observer.clone()))
        .collect::<Vec<_>>();
    let mut samples = Vec::new();
    let mut dependencies = Vec::new();
    let mut deferred = false;
    for (id, observer) in observers {
        let record = observer.borrow();
        for target in &record.targets {
            let target_record = target.borrow();
            if target_record
                .last
                .is_some_and(|last| now.duration_since(last).as_secs_f64() * 1000. < record.delay)
            {
                deferred = true;
                continue;
            }
            let geometry = geometry(scope, state, &record, &target_record)?;
            dependencies.extend(geometry.dependencies);
            let area = geometry.bounding.width * geometry.bounding.height;
            let ratio = if area > 0. {
                (geometry.intersection.width * geometry.intersection.height / area).clamp(0., 1.)
            } else if geometry.intersecting {
                1.
            } else {
                0.
            };
            let visible = if record.track_visibility && geometry.intersecting {
                let object = v8::Local::new(scope, &target_record.object);
                let (owner, node) = node(scope, object, true)?;
                visible_in_tree(&owner, node, geometry.intersection)?
            } else {
                false
            };
            let index = record
                .thresholds
                .partition_point(|threshold| *threshold <= ratio);
            let next = (index, geometry.intersecting, visible);
            let entry = (target_record.previous != Some(next)).then(|| {
                let object = v8::Local::new(scope, &target_record.object);
                Entry {
                    target: Some(Rc::new(v8::Weak::new(scope, object))),
                    time: state.clock_origin.get().elapsed().as_secs_f64() * 1000.,
                    root: geometry.root,
                    bounding: geometry.bounding,
                    intersection: geometry.intersection,
                    ratio,
                    intersecting: geometry.intersecting,
                    visible,
                }
            });
            samples.push((id, observer.clone(), target.clone(), next, entry));
        }
    }
    let mut counts = BTreeMap::new();
    for (id, _, _, _, entry) in &samples {
        if entry.is_some() {
            *counts.entry(*id).or_insert(0usize) += 1;
        }
    }
    for (id, count) in counts {
        let observer = samples
            .iter()
            .find(|sample| sample.0 == id)
            .unwrap()
            .1
            .clone();
        let mut observer = observer.borrow_mut();
        if observer.entries.len().saturating_add(count) > 4096 {
            return Err(V8BindingError::Allocation("intersection pending entries"));
        }
        observer
            .entries
            .try_reserve(count)
            .map_err(|_| V8BindingError::Allocation("intersection pending entries"))?;
    }
    // Retain queue targets through the observer's JS wrapper, not a native
    // global that would root observer-target cycles after document retirement.
    let mut pinned = BTreeSet::new();
    for (id, observer, _, _, _) in &samples {
        if !pinned.insert(*id) {
            continue;
        }
        let observer = observer.borrow();
        let object = v8::Local::new(
            scope,
            observer
                .object
                .as_ref()
                .ok_or(V8BindingError::Allocation("observer queue owner"))?,
        );
        let mut targets = Vec::new();
        for entry in observer.entries.iter().chain(
            samples
                .iter()
                .filter(|sample| sample.0 == *id)
                .filter_map(|sample| sample.4.as_ref()),
        ) {
            let target = entry
                .target
                .as_ref()
                .and_then(|target| target.to_local(scope))
                .ok_or(V8BindingError::Allocation("queued target"))?;
            targets.push(target.into());
        }
        pin_queued_targets(scope, object, &targets)?;
    }
    let mut state_record = state.intersection_observers.borrow_mut();
    state_record
        .pending
        .try_reserve(samples.len())
        .map_err(|_| V8BindingError::Allocation("intersection pending callbacks"))?;
    for (id, observer, target, next, entry) in samples {
        let mut target = target.borrow_mut();
        target.previous = Some(next);
        target.last = Some(now);
        if let Some(entry) = entry {
            observer.borrow_mut().entries.push(entry);
            if !state_record.pending.contains(&id) {
                state_record.pending.push_back(id);
            }
        }
    }
    state_record.dependencies = dependencies;
    state_record.dirty = deferred;
    Ok(())
}
pub(super) fn needs_render(state: &V8BindingState) -> bool {
    let observers = state.intersection_observers.borrow();
    observers.dirty
        || observers.dependencies.iter().any(|(owner, revision)| {
            owner
                .upgrade()
                .is_none_or(|owner| owner.native_paint_revision().ok() != Some(*revision))
        })
}
pub(super) fn delay(state: &V8BindingState) -> Option<std::time::Duration> {
    if state.execution_retired() || state.execution_suspended() || state.execution_tasks_stopped() {
        return None;
    }
    let observers = state.intersection_observers.borrow();
    if observers.pending.is_empty() {
        None
    } else {
        Some(observers.retry.map_or(std::time::Duration::ZERO, |at| {
            at.saturating_duration_since(std::time::Instant::now())
        }))
    }
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<bool, V8BindingError> {
    if delay(state).is_none_or(|delay| !delay.is_zero()) || scope.is_execution_terminating() {
        return Ok(false);
    }
    let observer = {
        let observers = state.intersection_observers.borrow();
        observers.pending.front().and_then(|id| {
            observers
                .observers
                .get(id)
                .map(|observer| (*id, observer.clone()))
        })
    };
    let Some((id, observer)) = observer else {
        state
            .intersection_observers
            .borrow_mut()
            .pending
            .pop_front();
        return Ok(true);
    };
    let prepared = (|| {
        #[cfg(test)]
        if std::mem::take(&mut state.intersection_observers.borrow_mut().fail_prepare) {
            return Err(V8BindingError::Allocation(
                "injected intersection entry preparation",
            ));
        }
        let (entries, object) = {
            let record = observer.borrow();
            (
                record.entries.clone(),
                v8::Local::new(
                    scope,
                    record
                        .object
                        .as_ref()
                        .ok_or(V8BindingError::Allocation("observer owner"))?,
                ),
            )
        };
        let array = entries_array(scope, state, &entries)?;
        let key = v8::String::new(scope, "oryn.intersection_observers.callback")
            .ok_or(V8BindingError::Allocation("observer callback key"))?;
        let key = v8::Private::for_api(scope, Some(key));
        let callback = object
            .get_private(scope, key)
            .and_then(|value| value.try_cast::<v8::Function>().ok())
            .ok_or(V8BindingError::Allocation("observer callback"))?;
        Ok((array, callback, object))
    })();
    let (array, callback, object) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            state.intersection_observers.borrow_mut().retry =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(10));
            return Err(error);
        }
    };
    pin_queued_targets(scope, object, &[])?;
    observer.borrow_mut().entries.clear();
    {
        let mut observers = state.intersection_observers.borrow_mut();
        observers.pending.pop_front();
        observers.retry = None;
    }
    release_idle(state, id, &observer);
    global_events::invoke(
        scope,
        callback.into(),
        object.into(),
        &[array, object.into()],
    );
    Ok(true)
}

fn visible_in_tree(
    state: &Rc<V8BindingState>,
    node: NodeId,
    rect: BindingRect,
) -> Result<bool, V8BindingError> {
    let mut current = state.clone();
    let mut node = node;
    let mut seen = BTreeSet::new();
    let mut polygon = oryn_common::visibility::rectangle(oryn_common::v4::CssRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    });
    for _ in 0..1024 {
        if !seen.insert(current.instance_id)
            || current.execution_retired()
            || current.execution_suspended()
            || !permission_visibility::intersection_region_visible(
                &current,
                node,
                std::slice::from_ref(&polygon),
            )?
        {
            return Ok(false);
        }
        let relation = current.frame_viewport_parent.borrow().clone();
        let Some((parent, frame, connection)) = relation else {
            return Ok(true);
        };
        let Some(parent) = parent.upgrade() else {
            return Ok(false);
        };
        if parent.dom.borrow().frame_context_generation(frame) != connection {
            return Ok(false);
        }
        let projection = parent
            .layout
            .borrow()
            .hit_projections
            .get(&node_key(frame))
            .cloned();
        let Some(projection) = projection else {
            return Ok(false);
        };
        let mut translation = identity();
        translation[12] = projection.box_model.content_box.x;
        translation[13] = projection.box_model.content_box.y;
        let matrix = multiply_matrix_components(projection.matrix, translation);
        for point in &mut polygon {
            let [x, y] = *point;
            let w = matrix[3] * x + matrix[7] * y + matrix[15];
            if !w.is_finite() || w <= 0. {
                return Ok(false);
            }
            *point = [
                (matrix[0] * x + matrix[4] * y + matrix[12]) / w,
                (matrix[1] * x + matrix[5] * y + matrix[13]) / w,
            ];
            if !point.iter().all(|value| value.is_finite()) {
                return Ok(false);
            }
        }
        current = parent;
        node = frame;
    }
    Ok(false)
}
fn required<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: Option<v8::Local<v8::Object>>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let value = option(scope, options, name)?;
    if value.is_undefined() {
        Err(error(
            "TypeError",
            &format!("missing IntersectionObserverEntry {name}"),
        ))
    } else {
        Ok(value)
    }
}
fn init_rect(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<BindingRect, V8BindingError> {
    let object = if value.is_null_or_undefined() {
        None
    } else {
        Some(
            v8::Local::<v8::Object>::try_from(value)
                .map_err(|_| error("TypeError", "rectangle must be a dictionary"))?,
        )
    };
    let mut values = [0.; 4];
    for (name, index) in [("height", 3), ("width", 2), ("x", 0), ("y", 1)] {
        let value = option(scope, object, name)?;
        if !value.is_undefined() {
            values[index] = value
                .number_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
        }
    }
    Ok(BindingRect {
        x: values[0],
        y: values[1],
        width: values[2],
        height: values[3],
    })
}
pub(super) fn construct_entry(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    let options = Some(v8::Local::<v8::Object>::try_from(args.get(0)).map_err(|_| {
        error(
            "TypeError",
            "IntersectionObserverEntry requires a dictionary",
        )
    })?);
    let value = required(scope, options, "boundingClientRect")?;
    let bounding = init_rect(scope, value)?;
    let value = required(scope, options, "intersectionRatio")?;
    let ratio = value
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    if !ratio.is_finite() {
        return Err(error("TypeError", "intersectionRatio must be finite"));
    }
    let value = required(scope, options, "intersectionRect")?;
    let intersection = init_rect(scope, value)?;
    let intersecting = required(scope, options, "isIntersecting")?.boolean_value(scope);
    let visible = required(scope, options, "isVisible")?.boolean_value(scope);
    let value = required(scope, options, "rootBounds")?;
    let root = if value.is_null() {
        None
    } else {
        Some(init_rect(scope, value)?)
    };
    let value = required(scope, options, "target")?;
    let target = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| error("TypeError", "entry target must be an Element"))?;
    node(scope, target, true)?;
    let value = required(scope, options, "time")?;
    let time = value
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    if !time.is_finite() {
        return Err(error("TypeError", "entry time must be finite"));
    }
    node(scope, target, true)?;
    set_entry_target(scope, args.this(), target)?;
    let record = Entry {
        target: None,
        time,
        root,
        bounding,
        intersection,
        ratio,
        intersecting,
        visible,
    };
    let id = register_platform_object(
        state,
        PlatformObjectRecord::IntersectionObserverEntry(record),
    );
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}

#[cfg(test)]
mod native_tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) -> bool {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .is_true()
    }
    #[test]
    fn take_records_releases_registry_borrows_and_commits_only_its_queue() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let target = state.dom.borrow().find_by_id("target").unwrap();
        state.layout.borrow_mut().viewport = Some((100., 100.));
        state.layout.borrow_mut().geometry.insert(
            node_key(target),
            BindingGeometry::Available(BindingRect {
                x: 0.,
                y: 0.,
                width: 10.,
                height: 10.,
            }),
        );
        assert!(eval(
            scope,
            "globalThis.calls=0;globalThis.target=document.getElementById('target');globalThis.a=new IntersectionObserver(()=>{throw Error('drained observer called')});globalThis.b=new IntersectionObserver((entries,owner)=>{calls++;if(owner!==b||entries.length!==1||entries[0].target!==target||owner.takeRecords().length!==0)throw Error('callback custody');globalThis.created=new Response('callback');});a.observe(target);b.observe(target);true"
        ));
        sample(scope, &state).unwrap();
        assert_eq!(state.intersection_observers.borrow().pending.len(), 2);
        assert!(eval(
            scope,
            "a.disconnect();globalThis.records=a.takeRecords();records.length===1&&records[0].target===target&&records[0].boundingClientRect.width===10&&a.takeRecords().length===0&&calls===0"
        ));
        assert_eq!(state.intersection_observers.borrow().pending.len(), 1);
        assert!(poll(scope, &state).unwrap());
        assert!(eval(
            scope,
            "calls===1&&created instanceof Response&&records[0].isIntersecting"
        ));
        assert!(!poll(scope, &state).unwrap());
    }
    #[test]
    fn native_threshold_tasks_retain_preparation_and_take_records() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let target = state.dom.borrow().find_by_id("target").unwrap();
        state.layout.borrow_mut().viewport = Some((100., 100.));
        state.layout.borrow_mut().geometry.insert(
            node_key(target),
            BindingGeometry::Available(BindingRect {
                x: 90.,
                y: 0.,
                width: 20.,
                height: 20.,
            }),
        );
        assert!(eval(
            scope,
            "globalThis.calls=0;globalThis.observer=new IntersectionObserver((entries,owner)=>{calls++;globalThis.saved=entries;if(owner!==observer)throw Error('receiver')},{threshold:[0,.5,1],rootMargin:'0px'});observer.callback=()=>{throw Error('forged callback')};observer.observe(document.getElementById('target'));calls===0&&Object.isFrozen(observer.thresholds)"
        ));
        sample(scope, &state).unwrap();
        assert!(eval(scope, "calls===0"));
        state.intersection_observers.borrow_mut().fail_prepare = true;
        assert!(poll(scope, &state).is_err());
        assert!(eval(scope, "calls===0"));
        state.intersection_observers.borrow_mut().retry = None;
        assert!(poll(scope, &state).unwrap());
        assert!(eval(
            scope,
            "calls===1&&saved[0] instanceof IntersectionObserverEntry&&saved[0].intersectionRatio===.5&&saved[0].isIntersecting&&saved[0].boundingClientRect instanceof DOMRectReadOnly"
        ));
        sample(scope, &state).unwrap();
        assert!(delay(&state).is_none());
        state.layout.borrow_mut().geometry.insert(
            node_key(target),
            BindingGeometry::Available(BindingRect {
                x: 100.,
                y: 0.,
                width: 20.,
                height: 20.,
            }),
        );
        sample(scope, &state).unwrap();
        assert!(eval(
            scope,
            "globalThis.records=observer.takeRecords();records.length===1&&records[0].isIntersecting&&records[0].intersectionRatio===0&&calls===1"
        ));
        assert!(delay(&state).is_none());
        state.layout.borrow_mut().geometry.insert(
            node_key(target),
            BindingGeometry::Available(BindingRect {
                x: 101.,
                y: 0.,
                width: 20.,
                height: 20.,
            }),
        );
        sample(scope, &state).unwrap();
        assert!(eval(scope, "observer.disconnect();true"));
        assert!(poll(scope, &state).unwrap());
        assert!(eval(scope, "calls===2&&!saved[0].isIntersecting"));
    }
    #[test]
    fn constructor_dictionary_order_and_entry_private_slots() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=t></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(
            scope,
            "(()=>{const order=[];const options=new Proxy({delay:4294967297,root:null,rootMargin:'1px 2%',scrollMargin:'3px',threshold:[1,0,.5],trackVisibility:false},{get(t,k){order.push(k);return t[k]}});const o=new IntersectionObserver(()=>{},options);return order.join(',')==='delay,root,rootMargin,scrollMargin,threshold,trackVisibility'&&o.delay===1&&o.rootMargin==='1px 2% 1px 2%'&&o.thresholds.join(',')==='0,0.5,1'})()"
        ));
        assert!(eval(
            scope,
            "(()=>{const order=[];try{new IntersectionObserver(()=>{},new Proxy({rootMargin:'bad',threshold:2},{get(t,k){order.push(k);return t[k]}}))}catch(e){return e.name==='SyntaxError'&&order.join(',')==='delay,root,rootMargin,scrollMargin,threshold,trackVisibility'}return false})()"
        ));
        assert!(eval(
            scope,
            "(()=>{try{new IntersectionObserver(()=>{},{rootMargin:'bad'})}catch(e){return e instanceof DOMException&&e.name==='SyntaxError'&&e.code===12}return false})()"
        ));
        assert!(eval(
            scope,
            "(()=>{try{new IntersectionObserver(()=>{},{threshold:2})}catch(e){return e instanceof RangeError&&!(e instanceof DOMException)}return false})()"
        ));
        assert!(eval(
            scope,
            "(()=>{const sentinel={};try{new IntersectionObserver(()=>{},{get rootMargin(){throw sentinel}})}catch(e){return e===sentinel}return false})()"
        ));
        assert!(eval(
            scope,
            "(()=>{const target=document.getElementById('t');const e=new IntersectionObserverEntry({time:1,rootBounds:null,boundingClientRect:{x:2,width:3},intersectionRect:{},intersectionRatio:.25,isIntersecting:true,isVisible:false,target});return e.target===target&&e.rootBounds===null&&e.boundingClientRect.x===2&&e.intersectionRatio===.25&&!e.isVisible})()"
        ));
    }
    #[test]
    fn implicit_root_cannot_be_replaced_by_an_unrelated_target_window() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles,&mut isolate);
        let first = v8::Context::new(handles, Default::default());
        let second = v8::Context::new(handles, Default::default());
        let foreign = {
            let scope = &mut v8::ContextScope::new(handles, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><div id=t></div></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let owner = install_generated_bindings(scope, second, dom, RealmId(1), 2).unwrap();
            let target = owner.dom.borrow().find_by_id("t").unwrap();
            owner.layout.borrow_mut().viewport = Some((100., 100.));
            owner.layout.borrow_mut().geometry.insert(
                node_key(target),
                BindingGeometry::Available(BindingRect {
                    x: 0.,
                    y: 0.,
                    width: 10.,
                    height: 10.,
                }),
            );
            let source = v8::String::new(scope, "document.getElementById('t')").unwrap();
            let value = v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap();
            (v8::Global::new(scope, value), owner)
        };
        let scope = &mut v8::ContextScope::new(handles, first);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap();
        state.layout.borrow_mut().viewport = Some((100., 100.));
        let key = v8::String::new(scope, "foreign").unwrap();
        let value = v8::Local::new(scope, &foreign.0);
        first
            .global(scope)
            .create_data_property(scope, key.into(), value)
            .unwrap();
        assert!(eval(
            scope,
            "globalThis.o=new IntersectionObserver(()=>{});o.observe(foreign);true"
        ));
        sample(scope, &state).unwrap();
        assert!(eval(
            scope,
            "(()=>{const entries=o.takeRecords();return entries.length===1&&!entries[0].isIntersecting&&entries[0].intersectionRatio===0})()"
        ));
        drop(foreign);
    }
    #[test]
    fn document_open_preserves_queued_snapshot_and_rechecks_detached_target() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=t></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        state.layout.borrow_mut().viewport = Some((100., 100.));
        state.layout.borrow_mut().geometry.insert(
            node_key(target),
            BindingGeometry::Available(BindingRect {
                x: 10.,
                y: 10.,
                width: 20.,
                height: 20.,
            }),
        );
        assert!(eval(
            scope,
            "globalThis.oldTarget=document.getElementById('t');globalThis.o=new IntersectionObserver(()=>{});o.observe(oldTarget);true"
        ));
        sample(scope, &state).unwrap();
        assert!(eval(
            scope,
            "document.open();document.write('<body><div id=newTarget></div>');document.close();const entries=o.takeRecords();entries.length===1&&entries[0].target===oldTarget&&entries[0].isIntersecting&&!oldTarget.isConnected"
        ));
        sample(scope, &state).unwrap();
        assert!(eval(
            scope,
            "const detached=o.takeRecords();detached.length===1&&detached[0].target===oldTarget&&!detached[0].isIntersecting&&detached[0].boundingClientRect.width===0"
        ));
    }
    #[test]
    fn retirement_stops_delivery_but_preserves_script_held_entry_snapshots() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=t></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.layout.borrow_mut().viewport = Some((100., 100.));
        assert!(eval(
            scope,
            "globalThis.n=0;globalThis.target=document.getElementById('t');globalThis.o=new IntersectionObserver(()=>n++);o.observe(target);true"
        ));
        sample(scope, &state).unwrap();
        let record = state
            .intersection_observers
            .borrow()
            .observers
            .values()
            .next()
            .unwrap()
            .clone();
        assert_eq!(record.borrow().entries.len(), 1);
        state.retire_execution();
        assert!(delay(&state).is_none());
        assert!(!poll(scope, &state).unwrap());
        assert!(record.borrow().object.is_none());
        assert_eq!(record.borrow().entries.len(), 1);
        assert!(eval(
            scope,
            "const saved=o.takeRecords();saved.length===1&&saved[0].target===target&&n===0"
        ));
        assert!(eval(
            scope,
            "o.disconnect();o.observe(target);o.thresholds[0]===0"
        ));
        assert!(state.intersection_observers.borrow().observers.is_empty());
        assert!(record.borrow().object.is_none());
    }
    #[test]
    fn disconnected_observer_releases_idle_self_root_after_pending_delivery_and_can_reobserve() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=t></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.layout.borrow_mut().viewport = Some((100., 100.));
        assert!(eval(
            scope,
            "globalThis.n=0;globalThis.o=new IntersectionObserver(()=>n++);o.observe(document.getElementById('t'));true"
        ));
        sample(scope, &state).unwrap();
        let record = state
            .intersection_observers
            .borrow()
            .observers
            .values()
            .next()
            .unwrap()
            .clone();
        assert!(eval(scope, "o.disconnect();true"));
        assert!(record.borrow().object.is_some());
        assert_eq!(state.intersection_observers.borrow().pending.len(), 1);
        assert!(poll(scope, &state).unwrap());
        assert!(record.borrow().object.is_none());
        assert!(state.intersection_observers.borrow().observers.is_empty());
        assert!(eval(scope, "n===1"));
        assert!(eval(scope, "o.observe(document.getElementById('t'));true"));
        assert!(record.borrow().object.is_some());
        assert_eq!(state.intersection_observers.borrow().observers.len(), 1);
        assert!(eval(
            scope,
            "o.unobserve(document.getElementById('t'));true"
        ));
        assert!(record.borrow().object.is_none());
        assert!(state.intersection_observers.borrow().observers.is_empty());
    }
    #[test]
    fn parked_document_preserves_pending_callback_without_runnable_task() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=t></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.layout.borrow_mut().viewport = Some((100., 100.));
        assert!(eval(
            scope,
            "globalThis.n=0;globalThis.o=new IntersectionObserver(()=>n++);o.observe(document.getElementById('t'));true"
        ));
        sample(scope, &state).unwrap();
        state.execution_suspended.set(true);
        assert!(delay(&state).is_none());
        assert!(!poll(scope, &state).unwrap());
        assert!(eval(scope, "n===0"));
        state.execution_suspended.set(false);
        assert!(poll(scope, &state).unwrap());
        assert!(eval(scope, "n===1"));
    }
}

/// An explicit native layout handoff is a new geometry sample opportunity.
pub(super) fn layout_changed(state: &V8BindingState) {
    let mut observers = state.intersection_observers.borrow_mut();
    if !observers.observers.is_empty() {
        observers.dirty = true;
    }
}
