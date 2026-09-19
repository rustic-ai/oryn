//! Native visual viewport geometry. Layout CSS size is unchanged by pinch;
//! zoom changes the visible rectangle and its offset within that layout view.
//! Input and paint consumers use the same transform in opposite directions.
use super::*;

pub(super) use oryn_common::visual_viewport::Geometry;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ZoomPolicy {
    pub minimum: f64,
    pub maximum: f64,
    pub allowed: bool,
}
impl Default for ZoomPolicy {
    fn default() -> Self {
        Self {
            minimum: 1.,
            maximum: 10.,
            allowed: true,
        }
    }
}
impl ZoomPolicy {
    fn valid(self) -> bool {
        self.minimum.is_finite()
            && self.maximum.is_finite()
            && self.minimum > 0.
            && self.maximum >= self.minimum
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Pinch {
    ids: (i32, i32),
    initial_distance: f64,
    initial_scale: f64,
    anchor: (f64, f64),
    policy: ZoomPolicy,
}
impl Pinch {
    pub fn begin(
        geometry: Geometry,
        policy: ZoomPolicy,
        first: (i32, f64, f64),
        second: (i32, f64, f64),
    ) -> Option<Self> {
        if !policy.allowed || !policy.valid() || first.0 == second.0 {
            return None;
        }
        let (distance, center) = pair(first, second)?;
        // Coincident fingers cannot define an initial scale ratio. Wait for
        // another sample rather than inventing distance or allowing NaN state.
        if distance <= f64::EPSILON {
            return None;
        }
        let anchor = geometry.visual_to_layout(center.0, center.1)?;
        Some(Self {
            ids: (first.0, second.0),
            initial_distance: distance,
            initial_scale: geometry.scale,
            anchor,
            policy,
        })
    }
    pub fn update(
        self,
        geometry: &mut Geometry,
        first: (i32, f64, f64),
        second: (i32, f64, f64),
    ) -> Option<bool> {
        if (first.0, second.0) != self.ids {
            return None;
        }
        let (distance, center) = pair(first, second)?;
        let raw_scale = self.initial_scale * (distance / self.initial_distance);
        if !raw_scale.is_finite() {
            return None;
        }
        let next_scale = raw_scale.clamp(self.policy.minimum, self.policy.maximum);
        let old = *geometry;
        geometry.scale = next_scale;
        let port = geometry.content_rect();
        geometry.offset_x = self.anchor.0 - port.x - (center.0 - port.x) / next_scale;
        geometry.offset_y = self.anchor.1 - port.y - (center.1 - port.y) / next_scale;
        geometry.clamp();
        Some(old != *geometry)
    }
    pub fn contains(self, id: i32) -> bool {
        self.ids.0 == id || self.ids.1 == id
    }
}
fn pair(first: (i32, f64, f64), second: (i32, f64, f64)) -> Option<(f64, (f64, f64))> {
    if ![first.1, first.2, second.1, second.2]
        .iter()
        .all(|value| value.is_finite())
    {
        return None;
    }
    let center = (first.1 / 2. + second.1 / 2., first.2 / 2. + second.2 / 2.);
    let distance = (first.1 - second.1).hypot(first.2 - second.2);
    distance.is_finite().then_some((distance, center))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinch_extreme_pan_excludes_classic_gutters_and_keeps_anchor_basis() {
        let port = oryn_common::v4::CssRect {
            x: 16.,
            y: 0.,
            width: 184.,
            height: 84.,
        };
        let mut geometry = Geometry::default();
        geometry.resize(200., 100.);
        geometry.set_scrollport(Some(port));
        let pinch = Pinch::begin(
            geometry,
            ZoomPolicy::default(),
            (1, 150., 70.),
            (2, 190., 70.),
        )
        .unwrap();
        assert_eq!(
            pinch.update(&mut geometry, (1, -30., -100.), (2, 50., -100.)),
            Some(true)
        );
        assert_eq!(
            (
                geometry.scale,
                geometry.width(),
                geometry.height(),
                geometry.offset_x,
                geometry.offset_y
            ),
            (2., 92., 42., 92., 42.)
        );
        assert!(geometry.is_valid());
        assert_eq!(
            geometry.visual_to_layout(port.right(), port.bottom()),
            Some((port.right(), port.bottom()))
        );
    }
    #[test]
    fn pinch_preserves_content_under_midpoint_and_transform_round_trips() {
        let mut geometry = Geometry::default();
        assert!(geometry.resize(400., 300.));
        let pinch = Pinch::begin(
            geometry,
            ZoomPolicy::default(),
            (1, 150., 150.),
            (2, 250., 150.),
        )
        .unwrap();
        assert_eq!(
            pinch.update(&mut geometry, (1, 100., 150.), (2, 300., 150.)),
            Some(true)
        );
        assert_eq!(
            (geometry.scale, geometry.width(), geometry.height()),
            (2., 200., 150.)
        );
        assert_eq!(geometry.visual_to_layout(200., 150.), Some((200., 150.)));
        let content = geometry.visual_to_layout(30., 40.).unwrap();
        assert_eq!(
            geometry.layout_to_visual(content.0, content.1),
            Some((30., 40.))
        );
        assert_eq!(
            (geometry.layout_width, geometry.layout_height),
            (400., 300.)
        );
        assert!(pinch.contains(1));
        assert!(!pinch.contains(3));
    }
    #[test]
    fn viewport_pan_consumes_only_available_range_and_preserves_residual() {
        let mut geometry = Geometry {
            scrollport: None,
            layout_width: 400.,
            layout_height: 300.,
            scale: 2.,
            offset_x: 100.,
            offset_y: 75.,
        };
        assert_eq!(geometry.pan(300., -200.), Some((100., -50.)));
        assert_eq!((geometry.offset_x, geometry.offset_y), (200., 0.));
        assert_eq!(geometry.pan(-500., 0.), Some((-100., 0.)));
        assert_eq!(geometry.offset_x, 0.);
    }
    #[test]
    fn pinch_policy_identity_and_nonfinite_samples_do_not_corrupt_geometry() {
        let mut geometry = Geometry::default();
        geometry.resize(200., 100.);
        assert!(
            Pinch::begin(
                geometry,
                ZoomPolicy {
                    allowed: false,
                    ..Default::default()
                },
                (1, 10., 20.),
                (2, 30., 20.)
            )
            .is_none()
        );
        assert!(
            Pinch::begin(
                geometry,
                ZoomPolicy::default(),
                (1, 10., 20.),
                (2, 10., 20.)
            )
            .is_none()
        );
        let pinch = Pinch::begin(
            geometry,
            ZoomPolicy {
                maximum: 3.,
                ..Default::default()
            },
            (1, 10., 20.),
            (2, 30., 20.),
        )
        .unwrap();
        assert_eq!(
            pinch.update(&mut geometry, (1, 0., 20.), (2, 200., 20.)),
            Some(true)
        );
        assert_eq!(geometry.scale, 3.);
        let old = geometry;
        assert_eq!(
            pinch.update(&mut geometry, (1, f64::NAN, 20.), (2, 200., 20.)),
            None
        );
        assert_eq!(geometry, old);
        assert_eq!(
            pinch.update(&mut geometry, (3, 0., 20.), (2, 200., 20.)),
            None
        );
        assert_eq!(geometry, old);
        assert!(!geometry.resize(f64::INFINITY, 100.));
        assert_eq!(geometry, old);
        geometry.resize(0., 0.);
        assert_eq!((geometry.offset_x, geometry.offset_y), (0., 0.));
    }
}

pub(super) fn object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let existing = state
        .platform_objects
        .borrow()
        .iter()
        .find_map(|(id, record)| {
            matches!(record, PlatformObjectRecord::VisualViewport).then_some(*id)
        });
    let id = existing
        .unwrap_or_else(|| register_platform_object(state, PlatformObjectRecord::VisualViewport));
    if state.visual_viewport_observed.get().is_none() {
        state
            .visual_viewport_observed
            .set(Some(observation(state)?));
    }
    new_platform_object(scope, state, "VisualViewport", id)
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    if !matches!(
        &*platform_object(state, id)?,
        PlatformObjectRecord::VisualViewport
    ) {
        return Err(V8BindingError::Allocation("VisualViewport receiver"));
    }
    if state.execution_retired() {
        return Ok(v8::Number::new(scope, 0.).into());
    }
    let geometry = state.native_visual_viewport_geometry()?;
    let root = state
        .dom
        .borrow()
        .first_element_child(state.document_root())?;
    let scroll = root
        .and_then(|root| state.scroll_offsets.borrow().get(&root).copied())
        .unwrap_or_default();
    let value = match member {
        "width" => geometry.width(),
        "height" => geometry.height(),
        "scale" => geometry.scale,
        "offsetLeft" => geometry.offset_x,
        "offsetTop" => geometry.offset_y,
        "pageLeft" => scroll.0 + geometry.offset_x,
        "pageTop" => scroll.1 + geometry.offset_y,
        _ => return Err(V8BindingError::Allocation("VisualViewport attribute")),
    };
    Ok(v8::Number::new(scope, value).into())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Observation {
    geometry: Geometry,
    page: (f64, f64),
}
fn observation(state: &V8BindingState) -> Result<Observation, V8BindingError> {
    let geometry = state.native_visual_viewport_geometry()?;
    let root = state
        .dom
        .borrow()
        .first_element_child(state.document_root())?;
    let scroll = root
        .and_then(|root| state.scroll_offsets.borrow().get(&root).copied())
        .unwrap_or_default();
    Ok(Observation {
        geometry,
        page: (scroll.0 + geometry.offset_x, scroll.1 + geometry.offset_y),
    })
}
/// Called at native rendering/task checkpoints. Commit the observation before
/// callbacks so reentrant changes belong to the next checkpoint, not this one.
pub fn poll(scope: &mut v8::PinScope, state: &Rc<V8BindingState>) -> Result<usize, V8BindingError> {
    if state.execution_retired() {
        state.visual_viewport_observed.set(None);
        state.visual_viewport_scroll_end.set(false);
        return Ok(0);
    }
    let id = state
        .platform_objects
        .borrow()
        .iter()
        .find_map(|(id, record)| {
            matches!(record, PlatformObjectRecord::VisualViewport).then_some(*id)
        });
    let Some(id) = id else {
        return Ok(0);
    };
    let next = observation(state)?;
    let Some(previous) = state.visual_viewport_observed.replace(Some(next)) else {
        return Ok(0);
    };
    let resize = (
        previous.geometry.width(),
        previous.geometry.height(),
        previous.geometry.scale,
    ) != (
        next.geometry.width(),
        next.geometry.height(),
        next.geometry.scale,
    );
    let scroll = previous.page != next.page
        || (previous.geometry.offset_x, previous.geometry.offset_y)
            != (next.geometry.offset_x, next.geometry.offset_y);
    let end = state.visual_viewport_scroll_end.get()
        && !scroll
        && !native_pointer::touch_gesture_active(state);
    if scroll {
        state.visual_viewport_scroll_end.set(true);
    } else if end {
        state.visual_viewport_scroll_end.set(false);
    }
    let identity = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
    );
    let mut delivered = 0;
    for (name, changed) in [("resize", resize), ("scroll", scroll), ("scrollend", end)] {
        if !changed {
            continue;
        }
        if state.execution_retired()
            || identity
                != (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                )
        {
            break;
        }
        let event = create_event_object(scope, state, name, false, false)?;
        let event_id = event_id(scope, event)?;
        if let Some(record) = state.events.borrow_mut().get_mut(&event_id) {
            record.is_trusted = true;
        }
        dispatch_platform_event(scope, state, id, name, Some(event))?;
        delivered += 1;
    }
    Ok(delivered)
}
