//! Native Web Animations objects, promises and renderer samples. Every time is
//! derived from the owning document clock shared with performance and rAF.
type TimelineCallback<'s> = (v8::Local<'s, v8::Object>, v8::Local<'s, v8::Function>);

use super::animation_timing::{Direction, Easing, Fill, Timing};
use super::*;
#[derive(Default)]
pub(super) struct State {
    timelines: BTreeMap<NodeId, u64>,
    rendering_time: Option<f64>,
    dirty_samples: bool,
    observed_css: BTreeMap<u64, ObservedCss>,
    contributors: Vec<RcWeak<RefCell<Animation>>>,
    settlements: VecDeque<Settlement>,
    css: BTreeMap<u64, u64>,
    active: BTreeMap<u64, v8::Global<v8::Object>>,
    events: VecDeque<native_platform::publication::Notification>,
}
pub(super) struct Timeline {
    owner: RcWeak<V8BindingState>,
    document: NodeId,
    pub origin: f64,
    pub id: Cell<u64>,
}
impl Timeline {
    fn now(&self) -> Option<f64> {
        let state = self.owner.upgrade()?;
        (!state.execution_retired() && state.document_root() == self.document)
            .then(|| document_time(&state) - self.origin)
    }
}
#[derive(Clone)]
struct Target {
    owner: RcWeak<V8BindingState>,
    node: NodeId,
    wrapper: Rc<v8::Weak<v8::Object>>,
}
pub(super) struct Effect {
    pub id: u64,
    owner: RcWeak<V8BindingState>,
    target: Option<Target>,
    pseudo: Option<String>,
    frames: Vec<BindingAnimationKeyframe>,
    offsets: Vec<Option<f64>>,
    timing: Timing,
    easing: String,
    fill: String,
    composite: String,
    iteration_composite: String,
    animation: Option<RcWeak<RefCell<Animation>>>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    Play,
    Pause,
}
struct Settlement {
    resolver: Rc<v8::Weak<v8::PromiseResolver>>,
    value: v8::Global<v8::Value>,
    reject: bool,
}
struct PromiseCell {
    owner: RcWeak<V8BindingState>,
    resolver: Rc<v8::Weak<v8::PromiseResolver>>,
    settled: bool,
}
impl PromiseCell {
    fn new<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        owner: &Rc<V8BindingState>,
    ) -> Result<(Self, v8::Local<'s, v8::Object>), V8BindingError> {
        let resolver = v8::PromiseResolver::new(scope).ok_or_else(failed)?;
        let object: v8::Local<v8::Object> = resolver.into();
        Ok((
            Self {
                owner: Rc::downgrade(owner),
                resolver: Rc::new(v8::Weak::new(scope, resolver)),
                settled: false,
            },
            object,
        ))
    }
    fn local<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::PromiseResolver>> {
        self.resolver.to_local(scope)
    }
    fn settle(&mut self, scope: &mut v8::PinScope, value: v8::Local<v8::Value>, reject: bool) {
        if self.settled {
            return;
        }
        self.settled = true;
        if let Some(owner) = self.owner.upgrade() {
            owner
                .web_animations
                .borrow_mut()
                .settlements
                .push_back(Settlement {
                    resolver: self.resolver.clone(),
                    value: v8::Global::new(scope, value),
                    reject,
                });
        }
    }
}
pub(super) struct Animation {
    pub id: u64,
    render_id: u64,
    owner: RcWeak<V8BindingState>,
    name: String,
    effect: Option<Rc<RefCell<Effect>>>,
    timeline: Option<Rc<Timeline>>,
    rate: f64,
    start: Option<f64>,
    hold: Option<f64>,
    pending: Option<Pending>,
    ready: PromiseCell,
    finished: PromiseCell,
    finished_notified: bool,
    persisted: bool,
    css: Option<BindingCssAnimation>,
}
impl Animation {
    pub(super) fn interface(&self) -> &'static str {
        match self.css.as_ref() {
            Some(css) if css.transition => "CSSTransition",
            Some(_) => "CSSAnimation",
            None => "Animation",
        }
    }
    fn now(&self) -> Option<f64> {
        self.timeline.as_ref().and_then(|timeline| timeline.now())
    }
    fn end(&self) -> f64 {
        self.css.as_ref().map_or_else(
            || {
                self.effect
                    .as_ref()
                    .map_or(0., |effect| effect.borrow().timing.end_time())
            },
            |css| css.duration_ms,
        )
    }
    fn time(&self) -> Option<f64> {
        self.hold
            .or_else(|| Some((self.now()? - self.start?) * self.rate))
    }
    fn play_state(&self) -> &'static str {
        if let Some(css) = &self.css {
            return match css.play_state.as_str() {
                "running" => "running",
                "paused" => "paused",
                "finished" => "finished",
                _ => "idle",
            };
        }
        if self.time().is_none() && self.pending.is_none() {
            "idle"
        } else if self.pending == Some(Pending::Pause)
            || (self.start.is_none() && self.pending != Some(Pending::Play))
        {
            "paused"
        } else if self.time().is_some_and(|time| {
            self.rate > 0. && time >= self.end() || self.rate < 0. && time <= 0.
        }) {
            "finished"
        } else {
            "running"
        }
    }
}
fn drain_settlements(scope: &mut v8::PinScope, state: &Rc<V8BindingState>) {
    // Promise resolution reads an authored then property. No native animation
    // borrow may survive this boundary; keep admitted values alive independently.
    while !scope.is_execution_terminating() {
        let next = state.web_animations.borrow_mut().settlements.pop_front();
        let Some(next) = next else { break };
        if let Some(resolver) = next.resolver.to_local(scope) {
            let value = v8::Local::new(scope, &next.value);
            if next.reject {
                resolver.reject(scope, value);
            } else {
                resolver.resolve(scope, value);
            }
        }
    }
}
fn next_render_id() -> Result<u64, V8BindingError> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |id| id.checked_add(1),
    )
    .map_err(|_| failed())
}
fn invalidate_samples(state: &V8BindingState) {
    state.layout_cache_key.borrow_mut().take();
    if state.layout_provider.borrow().is_some() {
        state.web_animations.borrow_mut().dirty_samples = true;
    }
}
fn invalidate_effect_target(effect: &Effect) {
    if let Some(owner) = effect
        .target
        .as_ref()
        .and_then(|target| target.owner.upgrade())
    {
        invalidate_samples(&owner);
    }
}
fn link_target(animation: &Rc<RefCell<Animation>>) {
    let target = {
        let animation = animation.borrow();
        animation.effect.as_ref().and_then(|effect| {
            effect
                .borrow()
                .target
                .as_ref()
                .and_then(|target| target.owner.upgrade())
        })
    };
    if let Some(target) = target {
        let mut registry = target.web_animations.borrow_mut();
        registry.contributors.retain(|weak| weak.strong_count() > 0);
        if !registry
            .contributors
            .iter()
            .filter_map(RcWeak::upgrade)
            .any(|value| Rc::ptr_eq(&value, animation))
        {
            registry.contributors.push(Rc::downgrade(animation));
        }
        drop(registry);
        invalidate_samples(&target);
    }
}
fn contributing_records(state: &V8BindingState) -> Vec<Rc<RefCell<Animation>>> {
    let mut records = state
        .platform_objects
        .borrow()
        .values()
        .filter_map(|record| match record {
            PlatformObjectRecord::WebAnimation(animation) => Some(animation.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    records.extend(
        state
            .web_animations
            .borrow()
            .contributors
            .iter()
            .filter_map(RcWeak::upgrade),
    );
    let mut seen = BTreeSet::new();
    records.retain(|record| seen.insert(record.borrow().render_id));
    records.sort_by_key(|record| record.borrow().render_id);
    records
}
pub(super) fn migrate_targets(
    scope: &mut v8::PinScope,
    source: &Rc<V8BindingState>,
    destination: &Rc<V8BindingState>,
    nodes: &BTreeMap<NodeId, NodeId>,
) {
    let states = scope
        .get_slot::<BindingStateRegistry>()
        .map(|registry| {
            registry
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for state in states {
        let effects = state
            .platform_objects
            .borrow()
            .values()
            .filter_map(|record| match record {
                PlatformObjectRecord::WebAnimationEffect(effect) => Some(effect.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        for effect in effects {
            let linked = {
                let mut effect = effect.borrow_mut();
                let Some(target) = effect.target.as_mut() else {
                    continue;
                };
                if !target
                    .owner
                    .upgrade()
                    .is_some_and(|owner| Rc::ptr_eq(&owner.dom, &source.dom))
                {
                    continue;
                }
                let Some(node) = nodes.get(&target.node) else {
                    continue;
                };
                target.node = *node;
                target.owner = Rc::downgrade(destination);
                effect.animation.as_ref().and_then(RcWeak::upgrade)
            };
            if let Some(animation) = linked {
                link_target(&animation);
            }
        }
    }
    invalidate_samples(source);
    invalidate_samples(destination);
}
fn failed() -> V8BindingError {
    V8BindingError::Allocation("native animation")
}
fn type_error() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Animation".into(),
        member: "TypeError".into(),
    }
}
fn dom_error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Animation".into(),
        member: name.into(),
    }
}
fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: Option<v8::Local<v8::Object>>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = v8::String::new(scope, name).ok_or_else(failed)?;
    match object {
        Some(object) => object
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException),
        None => Ok(v8::undefined(scope).into()),
    }
}
fn text(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<String, V8BindingError> {
    let value = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    rust_string_preserving_utf16(scope, value)
}
fn object(value: v8::Local<v8::Value>) -> Result<Option<v8::Local<v8::Object>>, V8BindingError> {
    if value.is_null_or_undefined() {
        Ok(None)
    } else {
        v8::Local::<v8::Object>::try_from(value)
            .map(Some)
            .map_err(|_| type_error())
    }
}
fn set(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    set_object_property(scope, object, name, value)
}
fn string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    v8_string(scope, value)
}
fn number(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    nullable: bool,
) -> Result<Option<f64>, V8BindingError> {
    if nullable && value.is_null_or_undefined() {
        return Ok(None);
    }
    let value = value
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    if !value.is_finite() {
        return Err(type_error());
    }
    Ok(Some(value))
}
fn target(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Option<Target>, V8BindingError> {
    let Some((owner, node)) = node_identity::argument(scope, value)? else {
        return Ok(None);
    };
    if !matches!(
        owner.dom.borrow().arena().get(node)?.kind,
        NodeKind::Element { .. }
    ) {
        return Err(type_error());
    }
    let wrapper = v8::Local::<v8::Object>::try_from(value).map_err(|_| type_error())?;
    Ok(Some(Target {
        owner: Rc::downgrade(&owner),
        node,
        wrapper: Rc::new(v8::Weak::new(scope, wrapper)),
    }))
}
fn dictionary(
    value: v8::Local<v8::Value>,
) -> Result<Option<v8::Local<v8::Object>>, V8BindingError> {
    object(value)
}
fn timing(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    existing: Option<&Effect>,
) -> Result<(Timing, String, String), V8BindingError> {
    let mut timing = existing.map_or_else(Timing::default, |effect| effect.timing.clone());
    let mut easing = existing.map_or_else(|| "linear".into(), |effect| effect.easing.clone());
    let mut fill = existing.map_or_else(|| "auto".into(), |effect| effect.fill.clone());
    let options = if value.is_number() {
        let duration = value
            .number_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        if duration.is_nan() || duration < 0. {
            return Err(type_error());
        }
        timing.duration = Some(duration);
        return Ok((timing, easing, fill));
    } else {
        dictionary(value)?
    };
    for name in [
        "delay",
        "direction",
        "duration",
        "easing",
        "endDelay",
        "fill",
        "iterationStart",
        "iterations",
    ] {
        let value = get(scope, options, name)?;
        if value.is_undefined() {
            continue;
        }
        match name {
            "delay" | "endDelay" => {
                let value = number(scope, value, false)?.unwrap();
                if name == "delay" {
                    timing.delay = value
                } else {
                    timing.end_delay = value
                }
            }
            "duration" => {
                if value.is_string() {
                    if text(scope, value)? != "auto" {
                        return Err(type_error());
                    }
                    timing.duration = None
                } else {
                    let value = value
                        .number_value(scope)
                        .ok_or(V8BindingError::JavaScriptException)?;
                    if value.is_nan() || value < 0. {
                        return Err(type_error());
                    }
                    timing.duration = Some(value)
                }
            }
            "iterationStart" | "iterations" => {
                let value = value
                    .number_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if value.is_nan() || value < 0. || name == "iterationStart" && !value.is_finite() {
                    return Err(type_error());
                }
                if name == "iterations" {
                    timing.iterations = value
                } else {
                    timing.iteration_start = value
                }
            }
            "easing" => {
                easing = text(scope, value)?;
                timing.easing = Easing::parse(&easing).ok_or_else(type_error)?
            }
            "direction" => {
                timing.direction = match text(scope, value)?.as_str() {
                    "normal" => Direction::Normal,
                    "reverse" => Direction::Reverse,
                    "alternate" => Direction::Alternate,
                    "alternate-reverse" => Direction::AlternateReverse,
                    _ => return Err(type_error()),
                }
            }
            "fill" => {
                fill = text(scope, value)?;
                timing.fill = match fill.as_str() {
                    "auto" | "none" => Fill::None,
                    "backwards" => Fill::Backwards,
                    "forwards" => Fill::Forwards,
                    "both" => Fill::Both,
                    _ => return Err(type_error()),
                }
            }
            _ => unreachable!(),
        }
    }
    Ok((timing, easing, fill))
}
fn pseudo(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Option<String>, V8BindingError> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    let value = text(scope, value)?;
    if !value.starts_with("::") || value.len() < 3 || value.contains(['{', '}', ';']) {
        return Err(dom_error("SyntaxError"));
    }
    Ok(Some(value))
}
fn composite(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    auto: bool,
) -> Result<String, V8BindingError> {
    let value = text(scope, value)?;
    if matches!(value.as_str(), "replace" | "add" | "accumulate") || auto && value == "auto" {
        Ok(value)
    } else {
        Err(type_error())
    }
}
fn css_name(name: &str) -> Option<String> {
    if matches!(name, "offset" | "easing" | "composite") {
        return None;
    }
    if name.starts_with("--") {
        return Some(name.into());
    }
    if name == "cssFloat" {
        return Some("float".into());
    }
    if name == "cssOffset" {
        return Some("offset".into());
    }
    if name.contains('-') || name.chars().next().is_some_and(char::is_uppercase) {
        return None;
    }
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase())
        } else {
            out.push(c)
        }
    }
    Some(out)
}
fn names(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<Vec<String>, V8BindingError> {
    let keys = object
        .get_own_property_names(
            scope,
            v8::GetPropertyNamesArgs {
                property_filter: v8::PropertyFilter::ONLY_ENUMERABLE
                    | v8::PropertyFilter::SKIP_SYMBOLS,
                key_conversion: v8::KeyConversionMode::ConvertToString,
                ..Default::default()
            },
        )
        .ok_or(V8BindingError::JavaScriptException)?;
    let mut names = Vec::new();
    for index in 0..keys.length() {
        let key = keys
            .get_index(scope, index)
            .ok_or(V8BindingError::JavaScriptException)?;
        names.push(text(scope, key)?);
    }
    names.sort();
    Ok(names)
}
fn iterable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<TimelineCallback<'s>>, V8BindingError> {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok(None);
    };
    let key = v8::Symbol::get_iterator(scope);
    let method = object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    if method.is_null_or_undefined() {
        return Ok(None);
    }
    Ok(Some((
        object,
        v8::Local::<v8::Function>::try_from(method).map_err(|_| type_error())?,
    )))
}
fn frame(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<(Option<f64>, BindingAnimationKeyframe), V8BindingError> {
    let frame = dictionary(value)?;
    let mut result = BindingAnimationKeyframe {
        offset: 0.,
        easing: "linear".into(),
        composite: "auto".into(),
        properties: BTreeMap::new(),
    };
    let value = get(scope, frame, "composite")?;
    if !value.is_undefined() {
        result.composite = composite(scope, value, true)?
    }
    let value = get(scope, frame, "easing")?;
    if !value.is_undefined() {
        result.easing = text(scope, value)?;
        Easing::parse(&result.easing).ok_or_else(type_error)?;
    }
    let value = get(scope, frame, "offset")?;
    let offset = number(scope, value, true)?;
    if offset.is_some_and(|offset| !(0. ..=1.).contains(&offset)) {
        return Err(type_error());
    }
    if let Some(frame) = frame {
        for name in names(scope, frame)? {
            if let Some(property) = css_name(&name) {
                let value = get(scope, Some(frame), &name)?;
                result.properties.insert(property, text(scope, value)?);
            }
        }
    }
    Ok((offset, result))
}
fn sequence_values<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    mut convert: impl FnMut(
        &mut v8::PinScope<'s, '_>,
        v8::Local<'s, v8::Value>,
    ) -> Result<T, V8BindingError>,
) -> Result<Vec<T>, V8BindingError> {
    let mut values = Vec::new();
    if let Some((object, method)) = iterable(scope, value)? {
        iterable_conversion::consume(scope, object, method, |scope, value| {
            values.push(convert(scope, value)?);
            Ok(())
        })?;
    } else {
        values.push(convert(scope, value)?);
    }
    Ok(values)
}
fn distribute_offsets(
    frames: &mut [(Option<f64>, BindingAnimationKeyframe)],
) -> Result<(), V8BindingError> {
    let mut previous = 0.;
    for (offset, _) in frames.iter() {
        if let Some(offset) = offset {
            if !(0. ..=1.).contains(offset) || *offset < previous {
                return Err(type_error());
            }
            previous = *offset;
        }
    }
    let count = frames.len();
    if count == 0 {
        return Ok(());
    }
    if frames[count - 1].0.is_none() {
        frames[count - 1].0 = Some(1.)
    }
    if count > 1 && frames[0].0.is_none() {
        frames[0].0 = Some(0.)
    }
    let mut start = 0;
    while start < count {
        let begin = frames[start].0.unwrap_or(1.);
        frames[start].1.offset = begin;
        let Some(end) = ((start + 1)..count).find(|index| frames[*index].0.is_some()) else {
            break;
        };
        let finish = frames[end].0.unwrap();
        for (index, frame) in frames.iter_mut().enumerate().take(end).skip(start + 1) {
            frame.1.offset =
                begin + (finish - begin) * (index - start) as f64 / (end - start) as f64;
        }
        start = end;
    }
    Ok(())
}
fn keyframes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<(Vec<BindingAnimationKeyframe>, Vec<Option<f64>>), V8BindingError> {
    if value.is_null_or_undefined() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut frames = Vec::new();
    let specified;
    if let Some((object, method)) = iterable(scope, value)? {
        iterable_conversion::consume(scope, object, method, |scope, value| {
            frames.push(frame(scope, value)?);
            Ok(())
        })?;
        specified = frames.iter().map(|(offset, _)| *offset).collect();
        distribute_offsets(&mut frames)?;
    } else {
        let object = dictionary(value)?.ok_or_else(type_error)?;
        let value = get(scope, Some(object), "composite")?;
        let composites = if value.is_undefined() {
            Vec::new()
        } else {
            sequence_values(scope, value, |scope, value| composite(scope, value, true))?
        };
        let value = get(scope, Some(object), "easing")?;
        let easings = if value.is_undefined() {
            Vec::new()
        } else {
            sequence_values(scope, value, |scope, value| {
                let value = text(scope, value)?;
                Easing::parse(&value).ok_or_else(type_error)?;
                Ok(value)
            })?
        };
        let value = get(scope, Some(object), "offset")?;
        let offsets = if value.is_undefined() {
            Vec::new()
        } else {
            sequence_values(scope, value, |scope, value| number(scope, value, true))?
        };
        for name in names(scope, object)? {
            if let Some(property) = css_name(&name) {
                let value = get(scope, Some(object), &name)?;
                let values = sequence_values(scope, value, text)?;
                let count = values.len();
                for (index, value) in values.into_iter().enumerate() {
                    let offset = if count <= 1 {
                        1.
                    } else {
                        index as f64 / (count - 1) as f64
                    };
                    if let Some((_, frame)) = frames.iter_mut().find(|(at, _)| *at == Some(offset))
                    {
                        frame.properties.insert(property.clone(), value);
                    } else {
                        frames.push((
                            Some(offset),
                            BindingAnimationKeyframe {
                                offset,
                                easing: "linear".into(),
                                composite: "auto".into(),
                                properties: BTreeMap::from([(property.clone(), value)]),
                            },
                        ));
                    }
                }
            }
        }
        frames.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        if offsets.len() > frames.len() {
            return Err(type_error());
        }
        specified = (0..frames.len())
            .map(|index| offsets.get(index).copied().flatten())
            .collect::<Vec<_>>();
        if !offsets.is_empty() {
            for (index, (offset, _)) in frames.iter_mut().enumerate() {
                *offset = specified[index];
            }
            distribute_offsets(&mut frames)?;
        }
        for (index, (_, frame)) in frames.iter_mut().enumerate() {
            if !easings.is_empty() {
                frame.easing = easings[index % easings.len()].clone();
            }
            if !composites.is_empty() {
                frame.composite = composites[index % composites.len()].clone();
            }
        }
    }
    Ok((
        frames.into_iter().map(|(_, frame)| frame).collect(),
        specified,
    ))
}
fn idl_css_name(name: &str) -> String {
    if name.starts_with("--") {
        return name.into();
    }
    if name == "float" {
        return "cssFloat".into();
    }
    if name == "offset" {
        return "cssOffset".into();
    }
    let mut result = String::new();
    let mut uppercase = false;
    for character in name.chars() {
        if character == '-' {
            uppercase = true
        } else if uppercase {
            result.push(character.to_ascii_uppercase());
            uppercase = false
        } else {
            result.push(character)
        }
    }
    result
}

fn record(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Object>,
) -> Result<(Rc<V8BindingState>, u64), V8BindingError> {
    let owner = binding_state_for_receiver(scope, value).ok_or_else(type_error)?;
    let id = stateful_id(scope, value).map_err(|_| type_error())?;
    Ok((owner, id))
}
fn animation(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Object>,
) -> Result<(Rc<V8BindingState>, Rc<RefCell<Animation>>), V8BindingError> {
    let (owner, id) = record(scope, value)?;
    let record = match owner.platform_objects.borrow().get(&id) {
        Some(PlatformObjectRecord::WebAnimation(animation)) => animation.clone(),
        _ => return Err(type_error()),
    };
    Ok((owner, record))
}
fn effect(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Option<Rc<RefCell<Effect>>>, V8BindingError> {
    let Some(value) = object(value)? else {
        return Ok(None);
    };
    let (owner, id) = record(scope, value)?;
    let effect = match owner.platform_objects.borrow().get(&id) {
        Some(PlatformObjectRecord::WebAnimationEffect(effect)) => effect.clone(),
        _ => return Err(type_error()),
    };
    Ok(Some(effect))
}
fn timeline(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Option<Rc<Timeline>>, V8BindingError> {
    let Some(value) = object(value)? else {
        return Ok(None);
    };
    let (owner, id) = record(scope, value)?;
    let timeline = match owner.platform_objects.borrow().get(&id) {
        Some(PlatformObjectRecord::WebAnimationTimeline(timeline)) => timeline.clone(),
        _ => return Err(type_error()),
    };
    Ok(Some(timeline))
}
fn default_timeline(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<Rc<Timeline>, V8BindingError> {
    document_timeline(scope, state, state.document_root())
}
fn document_timeline(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    document: NodeId,
) -> Result<Rc<Timeline>, V8BindingError> {
    if let Some(id) = state
        .web_animations
        .borrow()
        .timelines
        .get(&document)
        .copied()
        && let Some(PlatformObjectRecord::WebAnimationTimeline(timeline)) =
            state.platform_objects.borrow().get(&id)
    {
        return Ok(timeline.clone());
    }
    let timeline = Rc::new(Timeline {
        owner: Rc::downgrade(state),
        document,
        origin: 0.,
        id: Cell::new(0),
    });
    let id = register_platform_object(
        state,
        PlatformObjectRecord::WebAnimationTimeline(timeline.clone()),
    );
    timeline.id.set(id);
    new_platform_object(scope, state, "DocumentTimeline", id)?;
    state
        .web_animations
        .borrow_mut()
        .timelines
        .insert(document, id);
    Ok(timeline)
}
fn trace_effect(
    scope: &mut v8::PinScope,
    wrapper: v8::Local<v8::Object>,
    effect: &Effect,
) -> Result<(), V8BindingError> {
    let value = effect
        .target
        .as_ref()
        .and_then(|target| target.wrapper.to_local(scope))
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    let name = v8::String::new(scope, "oryn.animation.target").ok_or_else(failed)?;
    let key = v8::Private::for_api(scope, Some(name));
    wrapper
        .set_private(scope, key, value)
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(())
}
fn document_time(state: &V8BindingState) -> f64 {
    state
        .web_animations
        .borrow()
        .rendering_time
        .unwrap_or_else(|| state.performance_now())
}
pub(super) fn end_rendering(state: &V8BindingState) {
    state.web_animations.borrow_mut().rendering_time = None;
}
fn trace(
    scope: &mut v8::PinScope,
    _state: &Rc<V8BindingState>,
    wrapper: v8::Local<v8::Object>,
    animation: &Animation,
    extra: &[v8::Local<v8::Value>],
) -> Result<(), V8BindingError> {
    let mut values = extra.to_vec();
    if let Some(effect) = &animation.effect {
        let effect = effect.borrow();
        if let Some(owner) = effect.owner.upgrade() {
            let wrapper = new_platform_object(scope, &owner, "KeyframeEffect", effect.id)?;
            trace_effect(scope, wrapper, &effect)?;
            values.push(wrapper.into())
        }
    }
    if let Some(timeline) = &animation.timeline
        && let Some(owner) = timeline.owner.upgrade()
    {
        values
            .push(new_platform_object(scope, &owner, "DocumentTimeline", timeline.id.get())?.into())
    }
    for promise in [&animation.ready, &animation.finished] {
        if let Some(value) = promise.resolver.to_local(scope) {
            values.push(value.into());
        }
    }
    let values = v8::Array::new_with_elements(scope, &values);
    let name = v8::String::new(scope, "oryn.animation.edges").ok_or_else(failed)?;
    let key = v8::Private::for_api(scope, Some(name));
    wrapper
        .set_private(scope, key, values.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(())
}
fn create_animation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    effect: Option<Rc<RefCell<Effect>>>,
    timeline: Option<Rc<Timeline>>,
    receiver: Option<v8::Local<'s, v8::Object>>,
    css: Option<BindingCssAnimation>,
) -> Result<(v8::Local<'s, v8::Object>, Rc<RefCell<Animation>>), V8BindingError> {
    let (ready, ready_root) = PromiseCell::new(scope, state)?;
    let (finished, finished_root) = PromiseCell::new(scope, state)?;
    let animation = Rc::new(RefCell::new(Animation {
        id: 0,
        render_id: next_render_id()?,
        owner: Rc::downgrade(state),
        name: String::new(),
        effect: effect.clone(),
        timeline,
        rate: 1.,
        start: None,
        hold: None,
        pending: None,
        ready,
        finished,
        finished_notified: false,
        persisted: false,
        css,
    }));
    let id = register_platform_object(state, PlatformObjectRecord::WebAnimation(animation.clone()));
    animation.borrow_mut().id = id;
    let wrapper = match receiver {
        Some(wrapper) => {
            bind_platform_object(scope, state, id, wrapper);
            wrapper
        }
        None => new_platform_object(scope, state, animation.borrow().interface(), id)?,
    };
    if let Some(effect) = effect {
        if let Some(old) = effect
            .borrow_mut()
            .animation
            .take()
            .and_then(|old| old.upgrade())
        {
            old.borrow_mut().effect = None;
        }
        effect.borrow_mut().animation = Some(Rc::downgrade(&animation));
    }
    trace(
        scope,
        state,
        wrapper,
        &animation.borrow(),
        &[ready_root.into(), finished_root.into()],
    )?;
    animation
        .borrow_mut()
        .ready
        .settle(scope, wrapper.into(), false);
    link_target(&animation);
    Ok((wrapper, animation))
}
fn convert_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    animate: bool,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if value.is_number() {
        return Ok(value);
    }
    let source = dictionary(value)?;
    let output = v8::Object::new(scope);
    for member in [
        "composite",
        "delay",
        "direction",
        "duration",
        "easing",
        "endDelay",
        "fill",
        "id",
        "iterationComposite",
        "iterationStart",
        "iterations",
        "pseudoElement",
        "timeline",
    ] {
        if !animate && matches!(member, "id" | "timeline") {
            continue;
        }
        let raw = get(scope, source, member)?;
        if raw.is_undefined() {
            continue;
        }
        let converted = match member {
            "timeline" => {
                timeline(scope, raw)?;
                raw
            }
            "pseudoElement" => {
                if raw.is_null() {
                    raw
                } else {
                    let value = text(scope, raw)?;
                    string(scope, &value)?
                }
            }
            "delay" | "endDelay" | "iterationStart" | "iterations" => {
                let value = raw
                    .number_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                v8::Number::new(scope, value).into()
            }
            "duration" if !raw.is_string() => {
                let value = raw
                    .number_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                v8::Number::new(scope, value).into()
            }
            _ => {
                let value = text(scope, raw)?;
                string(scope, &value)?
            }
        };
        set(scope, output, member, converted)?;
    }
    Ok(output.into())
}
fn new_effect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    target_value: v8::Local<'s, v8::Value>,
    frames_value: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> Result<Rc<RefCell<Effect>>, V8BindingError> {
    target(scope, target_value)?;
    let options = convert_options(scope, options, false)?;
    let (timing, easing, fill) = timing(scope, options, None)?;
    let dictionary = if options.is_number() {
        None
    } else {
        dictionary(options)?
    };
    let pseudo_value = get(scope, dictionary, "pseudoElement")?;
    let pseudo = pseudo(scope, pseudo_value)?;
    let value = get(scope, dictionary, "composite")?;
    let composite = if value.is_undefined() {
        "replace".into()
    } else {
        composite(scope, value, false)?
    };
    let value = get(scope, dictionary, "iterationComposite")?;
    let iteration_composite = if value.is_undefined() {
        "replace".into()
    } else {
        let value = text(scope, value)?;
        if !matches!(value.as_str(), "replace" | "accumulate") {
            return Err(type_error());
        }
        value
    };
    let (frames, offsets) = keyframes(scope, frames_value)?;
    // Re-resolve after all author dictionary/keyframe callbacks (including adoption).
    let target = target(scope, target_value)?;
    let effect = Rc::new(RefCell::new(Effect {
        id: 0,
        owner: Rc::downgrade(state),
        target,
        pseudo,
        frames,
        offsets,
        timing,
        easing,
        fill,
        composite,
        iteration_composite,
        animation: None,
    }));
    let id = register_platform_object(
        state,
        PlatformObjectRecord::WebAnimationEffect(effect.clone()),
    );
    effect.borrow_mut().id = id;
    Ok(effect)
}
pub(super) fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    name: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<(), V8BindingError> {
    match name {
        "Animation" => {
            let effect = effect(scope, args.get(0))?;
            let timeline = if args.get(1).is_undefined() {
                Some(default_timeline(scope, state)?)
            } else {
                timeline(scope, args.get(1))?
            };
            create_animation(scope, state, effect, timeline, Some(args.this()), None)?;
        }
        "KeyframeEffect" => {
            if args.length() == 0 {
                return Err(type_error());
            }
            let effect = if args.length() == 1 {
                let source = effect(scope, args.get(0))?.ok_or_else(type_error)?;
                let source = source.borrow();
                let result = Rc::new(RefCell::new(Effect {
                    id: 0,
                    owner: Rc::downgrade(state),
                    target: source.target.clone(),
                    pseudo: source.pseudo.clone(),
                    frames: source.frames.clone(),
                    offsets: source.offsets.clone(),
                    timing: source.timing.clone(),
                    easing: source.easing.clone(),
                    fill: source.fill.clone(),
                    composite: source.composite.clone(),
                    iteration_composite: source.iteration_composite.clone(),
                    animation: None,
                }));
                let id = register_platform_object(
                    state,
                    PlatformObjectRecord::WebAnimationEffect(result.clone()),
                );
                result.borrow_mut().id = id;
                result
            } else {
                new_effect(scope, state, args.get(0), args.get(1), args.get(2))?
            };
            bind_platform_object(scope, state, effect.borrow().id, args.this());
            trace_effect(scope, args.this(), &effect.borrow())?;
        }
        "DocumentTimeline" => {
            let options = dictionary(args.get(0))?;
            let origin = get(scope, options, "originTime")?;
            let origin = if origin.is_undefined() {
                0.
            } else {
                number(scope, origin, false)?.unwrap()
            };
            let timeline = Rc::new(Timeline {
                owner: Rc::downgrade(state),
                document: state.document_root(),
                origin,
                id: Cell::new(0),
            });
            let id = register_platform_object(
                state,
                PlatformObjectRecord::WebAnimationTimeline(timeline.clone()),
            );
            timeline.id.set(id);
            bind_platform_object(scope, state, id, args.this());
        }
        _ => return Err(type_error()),
    }
    drain_settlements(scope, state);
    Ok(())
}
fn renew_promise(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    wrapper: v8::Local<v8::Object>,
    animation: &mut Animation,
    ready: bool,
) -> Result<(), V8BindingError> {
    let (promise, root) = PromiseCell::new(scope, state)?;
    if ready {
        animation.ready = promise
    } else {
        animation.finished = promise;
        animation.finished_notified = false
    }
    trace(scope, state, wrapper, animation, &[root.into()])
}
fn update_root(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    wrapper: v8::Local<v8::Object>,
    animation: &Animation,
) {
    let active = animation.pending == Some(Pending::Pause)
        || animation.now().is_some()
            && (animation.pending == Some(Pending::Play)
                || animation.play_state() == "running" && animation.rate != 0.)
        || animation.play_state() == "finished" && !animation.finished_notified;
    if active {
        state
            .web_animations
            .borrow_mut()
            .active
            .insert(animation.id, v8::Global::new(scope, wrapper));
    } else {
        state
            .web_animations
            .borrow_mut()
            .active
            .remove(&animation.id);
    }
    invalidate_samples(state);
    if let Some(effect) = &animation.effect {
        invalidate_effect_target(&effect.borrow());
    }
}
fn play(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    wrapper: v8::Local<v8::Object>,
    animation: &mut Animation,
    rewind: bool,
) -> Result<(), V8BindingError> {
    if animation.rate < 0. && animation.end().is_infinite() {
        return Err(dom_error("InvalidStateError"));
    }
    let current = animation.time();
    if current.is_none()
        || rewind
            && current.is_some_and(|time| {
                animation.rate >= 0. && (time < 0. || time >= animation.end())
                    || animation.rate < 0. && (time <= 0. || time > animation.end())
            })
    {
        animation.hold = Some(if animation.rate >= 0. {
            0.
        } else {
            animation.end()
        });
        animation.start = None;
    }
    if animation.finished.settled {
        renew_promise(scope, state, wrapper, animation, false)?;
    }
    if animation.ready.settled {
        renew_promise(scope, state, wrapper, animation, true)?;
    }
    animation.pending = Some(Pending::Play);
    update_root(scope, state, wrapper, animation);
    Ok(())
}
fn queue_event(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    animation: &Animation,
    name: &str,
) -> Result<(), V8BindingError> {
    let event = create_event_object_with_interface(
        scope,
        state,
        "AnimationPlaybackEvent",
        name,
        false,
        false,
    )?;
    let id = event_id(scope, event)?;
    if let Some(record) = state.events.borrow_mut().get_mut(&id) {
        record.is_trusted = true;
    }
    for (key, value) in [
        ("currentTime", animation.time()),
        ("timelineTime", animation.now()),
    ] {
        let value = value
            .map(|value| v8::Number::new(scope, value).into())
            .unwrap_or_else(|| v8::null(scope).into());
        event_private(scope, event, "AnimationPlaybackEvent", key, value)?;
    }
    state.web_animations.borrow_mut().events.push_back(
        native_platform::publication::pending_platform_dispatch(
            scope,
            state,
            animation.id,
            name,
            event,
        ),
    );
    Ok(())
}
#[derive(Clone)]
struct ObservedCss {
    snapshot: BindingCssAnimation,
    target: v8::Global<v8::Object>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum CssPhase {
    Before,
    Active,
    After,
}
fn css_active_duration(value: &BindingCssAnimation) -> f64 {
    if value.iteration_duration_ms == 0. || value.iterations == 0. {
        0.
    } else {
        value.iteration_duration_ms * value.iterations
    }
}
fn css_phase(value: &BindingCssAnimation) -> CssPhase {
    if value.current_time_ms < value.delay_ms {
        CssPhase::Before
    } else if value.current_time_ms < value.delay_ms + css_active_duration(value) {
        CssPhase::Active
    } else {
        CssPhase::After
    }
}
fn css_elapsed(value: &BindingCssAnimation) -> f64 {
    (value.current_time_ms - value.delay_ms)
        .max(0.)
        .min(css_active_duration(value))
        / 1000.
}
fn css_event_changes(
    old: Option<&BindingCssAnimation>,
    new: Option<&BindingCssAnimation>,
) -> Vec<(&'static str, f64)> {
    let mut result = Vec::new();
    let Some(value) = new else {
        if let Some(old) = old
            && css_phase(old) != CssPhase::After
        {
            result.push((
                if old.transition {
                    "transitioncancel"
                } else {
                    "animationcancel"
                },
                css_elapsed(old),
            ));
        }
        return result;
    };
    let before = old.map(css_phase).unwrap_or(CssPhase::Before);
    let after = css_phase(value);
    if value.transition && old.is_none() {
        result.push((
            "transitionrun",
            (-value.delay_ms).max(0.).min(value.iteration_duration_ms) / 1000.,
        ));
    }
    if before == CssPhase::Before && after != CssPhase::Before
        || before == CssPhase::After && after != CssPhase::After
    {
        result.push((
            if value.transition {
                "transitionstart"
            } else {
                "animationstart"
            },
            if before == CssPhase::After {
                css_active_duration(value) / 1000.
            } else {
                (-value.delay_ms).max(0.).min(css_active_duration(value)) / 1000.
            },
        ));
    }
    if !value.transition
        && before == CssPhase::Active
        && after == CssPhase::Active
        && value.iteration_duration_ms > 0.
        && let Some(old) = old
    {
        let previous = ((old.current_time_ms - old.delay_ms) / old.iteration_duration_ms).floor();
        let current =
            ((value.current_time_ms - value.delay_ms) / value.iteration_duration_ms).floor();
        if previous != current {
            result.push((
                "animationiteration",
                if current > previous {
                    current
                } else {
                    current + 1.
                } * value.iteration_duration_ms
                    / 1000.,
            ));
        }
    }
    if before != CssPhase::After && after == CssPhase::After
        || before != CssPhase::Before && after == CssPhase::Before
    {
        result.push((
            if value.transition {
                "transitionend"
            } else {
                "animationend"
            },
            if after == CssPhase::Before {
                0.
            } else {
                css_active_duration(value) / 1000.
            },
        ));
    }
    result
}
const HANDLER_TYPES: &[&str] = &[
    "animationstart",
    "animationend",
    "animationiteration",
    "animationcancel",
    "transitionrun",
    "transitionstart",
    "transitionend",
    "transitioncancel",
    "webkitAnimationStart",
    "webkitAnimationEnd",
    "webkitAnimationIteration",
    "webkitTransitionEnd",
];
pub(super) fn handler_property(name: &str) -> String {
    if matches!(
        name,
        "webkitAnimationStart"
            | "webkitAnimationEnd"
            | "webkitAnimationIteration"
            | "webkitTransitionEnd"
    ) {
        format!("on{}", name.to_ascii_lowercase())
    } else {
        format!("on{name}")
    }
}
fn legacy_type(name: &str) -> Option<&'static str> {
    match name {
        "animationstart" => Some("webkitAnimationStart"),
        "animationend" => Some("webkitAnimationEnd"),
        "animationiteration" => Some("webkitAnimationIteration"),
        "transitionend" => Some("webkitTransitionEnd"),
        _ => None,
    }
}
fn handler_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key =
        v8::String::new(scope, &format!("oryn.animation.handler.{name}")).ok_or_else(failed)?;
    let key = v8::Private::for_api(scope, Some(key));
    let value = object
        .get_private(scope, key)
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(if value.is_undefined() {
        v8::null(scope).into()
    } else {
        value
    })
}
pub(super) fn is_animation_event(name: &str) -> bool {
    HANDLER_TYPES.contains(&name)
}
fn node_handler_invoke<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue,
) {
    let result = (|| -> Result<(), V8BindingError> {
        let name = text(scope, args.data())?;
        let callback = handler_value(scope, args.this(), &name)?;
        if !callback.is_function() {
            return Ok(());
        }
        let event = args.get(0);
        let returned = global_events::invoke(scope, callback, args.this().into(), &[event]);
        if returned.is_some_and(|value| value.is_false()) {
            let event = event.to_object(scope).ok_or_else(failed)?;
            let owner = binding_state_for_receiver(scope, event).ok_or_else(failed)?;
            let id = event_id(scope, event)?;
            let mut events = owner.events.borrow_mut();
            if let Some(event) = events.get_mut(&id)
                && event.cancelable
            {
                event.default_prevented = true;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error)
    }
}
fn set_node_handler(
    scope: &mut v8::PinScope,
    receiver: v8::Local<v8::Object>,
    name: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let (owner, node) = node_identity::argument(scope, receiver.into())?.ok_or_else(type_error)?;
    let value = if value.is_function() {
        value
    } else {
        v8::null(scope).into()
    };
    let key =
        v8::String::new(scope, &format!("oryn.animation.handler.{name}")).ok_or_else(failed)?;
    let key = v8::Private::for_api(scope, Some(key));
    let registration = v8::String::new(scope, &format!("oryn.animation.registration.{name}"))
        .ok_or_else(failed)?;
    let registration = v8::Private::for_api(scope, Some(registration));
    let old = receiver
        .get_private(scope, registration)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0);
    let exists = old.is_some_and(|id| {
        owner
            .listeners
            .borrow()
            .get(&(node, name.into()))
            .is_some_and(|entries| entries.iter().any(|entry| entry.registration_id == id))
    });
    if value.is_function() && !exists {
        let id = owner.next_event_listener_id.get();
        let next = id.checked_add(1).ok_or_else(failed)?;
        let data = v8::String::new(scope, name).ok_or_else(failed)?;
        let callback = v8::Function::builder(node_handler_invoke)
            .data(data.into())
            .build(scope)
            .ok_or_else(failed)?;
        let stored = v8::BigInt::new_from_u64(scope, id);
        receiver
            .set_private(scope, registration, stored.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        owner.next_event_listener_id.set(next);
        owner
            .listeners
            .borrow_mut()
            .entry((node, name.into()))
            .or_default()
            .push(EventListener {
                registration_id: id,
                callback: v8::Global::new(scope, v8::Local::<v8::Value>::from(callback)),
                capture: false,
                once: false,
                passive: false,
                signal: None,
            });
    } else if !value.is_function()
        && let Some(id) = old
        && let Some(entries) = owner.listeners.borrow_mut().get_mut(&(node, name.into()))
    {
        entries.retain(|entry| entry.registration_id != id);
    }
    receiver
        .set_private(scope, key, value)
        .ok_or(V8BindingError::JavaScriptException)?;
    let present = owner
        .listeners
        .borrow()
        .get(&(node, name.into()))
        .is_some_and(|entries| !entries.is_empty());
    owner
        .dom
        .borrow_mut()
        .set_event_listener_presence(node, name, present)?;
    Ok(())
}

pub(super) fn clear_node_handlers(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    nodes: &BTreeSet<NodeId>,
) -> Result<(), V8BindingError> {
    let objects = {
        let wrappers = state.wrappers.borrow();
        state
            .objects
            .borrow()
            .iter()
            .filter_map(|(id, object)| {
                if !wrappers
                    .metadata(*id)
                    .is_some_and(|metadata| nodes.contains(&metadata.node))
                {
                    return None;
                }
                object
                    .handle
                    .to_local(scope)
                    .map(|object| v8::Global::new(scope, object))
            })
            .collect::<Vec<_>>()
    };
    for object in objects {
        let object = v8::Local::new(scope, &object);
        for name in HANDLER_TYPES {
            for prefix in ["handler", "registration"] {
                let key = v8::String::new(scope, &format!("oryn.animation.{prefix}.{name}"))
                    .ok_or_else(failed)?;
                let key = v8::Private::for_api(scope, Some(key));
                object
                    .delete_private(scope, key)
                    .ok_or(V8BindingError::JavaScriptException)?;
            }
        }
    }
    Ok(())
}
pub(super) fn invocation_type(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    receiver: v8::Local<v8::Object>,
    event_state: &Rc<V8BindingState>,
    event_id: u64,
    name: &str,
) -> Result<String, V8BindingError> {
    let Some(prefixed) = legacy_type(name) else {
        return Ok(name.into());
    };
    if !event_state
        .events
        .borrow()
        .get(&event_id)
        .is_some_and(|event| event.is_trusted)
    {
        return Ok(name.into());
    }
    let matching = if global_events::is_global(scope, state, receiver) {
        state
            .platform_listeners
            .borrow()
            .get(&(global_events::GLOBAL_TARGET, name.into()))
            .is_some_and(|list| !list.is_empty())
    } else if let Some((owner, node)) = node_identity::argument(scope, receiver.into())? {
        let registered = owner
            .listeners
            .borrow()
            .get(&(node, name.into()))
            .is_some_and(|list| !list.is_empty());
        registered || handler_value(scope, receiver, name)?.is_function()
    } else {
        true
    };
    Ok(if matching { name } else { prefixed }.into())
}
pub(super) struct InvocationTypeGuard {
    state: Rc<V8BindingState>,
    id: u64,
    original: String,
}
impl InvocationTypeGuard {
    pub(super) fn new(state: &Rc<V8BindingState>, id: u64, name: &str) -> Self {
        let original = {
            let mut events = state.events.borrow_mut();
            let event = events.get_mut(&id).expect("live dispatched event");
            std::mem::replace(&mut event.event_type, name.into())
        };
        Self {
            state: state.clone(),
            id,
            original,
        }
    }
}
impl Drop for InvocationTypeGuard {
    fn drop(&mut self) {
        if let Some(event) = self.state.events.borrow_mut().get_mut(&self.id) {
            event.event_type = std::mem::take(&mut self.original);
        }
    }
}

fn event_private(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let key = v8::String::new(scope, &format!("oryn.{interface}.{member}")).ok_or_else(failed)?;
    let key = v8::Private::for_api(scope, Some(key));
    event
        .set_private(scope, key, value)
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(())
}
fn event_field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    event_id(scope, event).map_err(|_| type_error())?;
    let key = v8::String::new(scope, &format!("oryn.{interface}.{member}")).ok_or_else(failed)?;
    let key = v8::Private::for_api(scope, Some(key));
    let value = event
        .get_private(scope, key)
        .ok_or(V8BindingError::JavaScriptException)?;
    if value.is_undefined() {
        return Err(type_error());
    }
    Ok(value)
}
pub(super) fn event_name_prefix(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    name: &str,
    init: Option<v8::Local<v8::Object>>,
) -> Result<(), V8BindingError> {
    if name == "AnimationEvent" {
        let value = get(scope, init, "animationName")?;
        let value = if value.is_undefined() {
            String::new()
        } else {
            text(scope, value)?
        };
        let value = string(scope, &value)?;
        event_private(scope, event, name, "animationName", value)?;
    }
    Ok(())
}
pub(super) fn initialize_event(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    name: &str,
    init: Option<v8::Local<v8::Object>>,
) -> Result<(), V8BindingError> {
    if name == "AnimationPlaybackEvent" {
        for member in ["currentTime", "timelineTime"] {
            let value = get(scope, init, member)?;
            let value = number(scope, value, true)?;
            let value = current_time(scope, value);
            event_private(scope, event, name, member, value)?;
        }
    } else if matches!(name, "AnimationEvent" | "TransitionEvent") {
        let value = get(scope, init, "elapsedTime")?;
        let value = if value.is_undefined() {
            0.
        } else {
            number(scope, value, false)?.unwrap()
        };
        let value = v8::Number::new(scope, value);
        event_private(scope, event, name, "elapsedTime", value.into())?;
        for member in if name == "TransitionEvent" {
            &["propertyName", "pseudoElement"][..]
        } else {
            &["pseudoElement"][..]
        } {
            let value = get(scope, init, member)?;
            let value = if value.is_undefined() {
                String::new()
            } else {
                text(scope, value)?
            };
            let value = string(scope, &value)?;
            event_private(scope, event, name, member, value)?;
        }
    }
    Ok(())
}
fn collect_css_events(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    {
        let _profile = native_binding_stage(scope, "deep.animation_layout_flush");
        refresh_layout(state)?;
    }
    let snapshots = state
        .layout_provider
        .borrow()
        .as_ref()
        .map(|provider| provider.css_animations())
        .transpose()
        .map_err(|_| failed())?
        .unwrap_or_default();
    let previous = state.web_animations.borrow().observed_css.clone();
    let mut next = BTreeMap::new();
    let mut notifications = Vec::new();
    for snapshot in snapshots {
        let target = wrap_node(scope, state, snapshot.target)?;
        let changes = css_event_changes(
            previous.get(&snapshot.id).map(|old| &old.snapshot),
            Some(&snapshot),
        );
        for (name, elapsed) in changes {
            notifications.push(css_notification(
                scope, state, target, &snapshot, name, elapsed,
            )?);
        }
        next.insert(
            snapshot.id,
            ObservedCss {
                snapshot,
                target: v8::Global::new(scope, target),
            },
        );
    }
    for (id, old) in &previous {
        if !next.contains_key(id) {
            let target = v8::Local::new(scope, &old.target);
            for (name, elapsed) in css_event_changes(Some(&old.snapshot), None) {
                notifications.push(css_notification(
                    scope,
                    state,
                    target,
                    &old.snapshot,
                    name,
                    elapsed,
                )?);
            }
        }
    }
    // No author callbacks run during preparation. Publish the observation cursor
    // only after the whole batch owns its event payloads; retries cannot replay.
    let mut animations = state.web_animations.borrow_mut();
    animations.observed_css = next;
    animations.events.extend(notifications);
    Ok(())
}
fn css_notification(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: v8::Local<v8::Object>,
    snapshot: &BindingCssAnimation,
    name: &str,
    elapsed: f64,
) -> Result<native_platform::publication::Notification, V8BindingError> {
    let interface = if snapshot.transition {
        "TransitionEvent"
    } else {
        "AnimationEvent"
    };
    let event = create_event_object_with_interface(scope, state, interface, name, true, false)?;
    let id = event_id(scope, event)?;
    if let Some(record) = state.events.borrow_mut().get_mut(&id) {
        record.is_trusted = true;
    }
    let value = string(scope, &snapshot.name)?;
    event_private(
        scope,
        event,
        interface,
        if snapshot.transition {
            "propertyName"
        } else {
            "animationName"
        },
        value,
    )?;
    let value = string(scope, snapshot.pseudo.as_deref().unwrap_or(""))?;
    event_private(scope, event, interface, "pseudoElement", value)?;
    let value = v8::Number::new(scope, elapsed);
    event_private(scope, event, interface, "elapsedTime", value.into())?;
    Ok(native_platform::publication::pending_dom_dispatch(
        scope, state, target, name, event,
    ))
}

fn sync_effect_css(effect: &Rc<RefCell<Effect>>) -> Result<(), V8BindingError> {
    let animation = effect.borrow().animation.as_ref().and_then(RcWeak::upgrade);
    if let Some(animation) = animation {
        let owner = animation.borrow().owner.upgrade();
        if let Some(owner) = owner {
            sync_css(&owner, &mut animation.borrow_mut())?;
        }
    }
    Ok(())
}
fn sync_css(state: &V8BindingState, animation: &mut Animation) -> Result<(), V8BindingError> {
    let Some(css) = animation.css.as_mut() else {
        return Ok(());
    };
    let provider = state.layout_provider.borrow().clone().ok_or_else(failed)?;
    provider.set_animation_time(document_time(state));
    let snapshots = provider.css_animations().map_err(|_| failed())?;
    if let Some(current) = snapshots.into_iter().find(|current| current.id == css.id) {
        animation.hold = Some(current.current_time_ms);
        if let Some(effect) = &animation.effect {
            let mut effect = effect.borrow_mut();
            effect.timing.delay = current.delay_ms;
            effect.timing.duration = Some(current.iteration_duration_ms);
            effect.timing.iterations = current.iterations;
            effect.timing.direction = match current.timing_direction.as_str() {
                "reverse" => Direction::Reverse,
                "alternate" => Direction::Alternate,
                "alternate-reverse" => Direction::AlternateReverse,
                _ => Direction::Normal,
            };
            effect.fill = current.timing_fill.clone();
            effect.timing.fill = match current.timing_fill.as_str() {
                "both" => Fill::Both,
                "forwards" => Fill::Forwards,
                "backwards" => Fill::Backwards,
                _ => Fill::None,
            };
            effect.easing = current.timing_easing.clone();
            effect.timing.easing = Easing::parse(&current.timing_easing).unwrap_or(Easing::Linear);
        }
        *css = current
    } else {
        css.play_state = "idle".into();
        animation.hold = None;
    }
    Ok(())
}
fn current_time<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<f64>,
) -> v8::Local<'s, v8::Value> {
    value
        .map(|value| v8::Number::new(scope, value).into())
        .unwrap_or_else(|| v8::null(scope).into())
}
fn getter(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| -> Result<v8::Local<v8::Value>, V8BindingError> {
        let name = text(scope, args.data())?;
        if name == "Document:timeline" {
            let state = binding_state_for_receiver(scope, args.this()).ok_or_else(type_error)?;
            let node = argument_node(scope, &state, args.this().into())?;
            if !matches!(
                state.dom.borrow().arena().get(node)?.kind,
                NodeKind::Document
            ) {
                return Err(type_error());
            }
            let timeline = document_timeline(scope, &state, node)?;
            return Ok(
                new_platform_object(scope, &state, "DocumentTimeline", timeline.id.get())?.into(),
            );
        }
        if name == "AnimationTimeline:currentTime" {
            let timeline = timeline(scope, args.this().into())?.ok_or_else(type_error)?;
            return Ok(current_time(scope, timeline.now()));
        }
        let (interface, member) = name.split_once(':').ok_or_else(failed)?;
        if matches!(
            interface,
            "AnimationPlaybackEvent" | "AnimationEvent" | "TransitionEvent"
        ) {
            return event_field(scope, args.this(), interface, member);
        }
        if let Some(name) = member.strip_prefix("handler-") {
            node_identity::argument(scope, args.this().into())?.ok_or_else(type_error)?;
            return handler_value(scope, args.this(), name);
        }
        if interface == "KeyframeEffect" {
            let effect = effect(scope, args.this().into())?.ok_or_else(type_error)?;
            let effect = effect.borrow();
            return match member {
                "target" => Ok(effect
                    .target
                    .as_ref()
                    .and_then(|target| target.wrapper.to_local(scope).map(Into::into))
                    .unwrap_or_else(|| v8::null(scope).into())),
                "pseudoElement" => effect
                    .pseudo
                    .as_ref()
                    .map(|pseudo| string(scope, pseudo))
                    .unwrap_or_else(|| Ok(v8::null(scope).into())),
                "composite" => string(scope, &effect.composite),
                "iterationComposite" => string(scope, &effect.iteration_composite),
                _ => Err(type_error()),
            };
        }
        let (state, animation) = animation(scope, args.this())?;
        let mut animation = animation.borrow_mut();
        sync_css(&state, &mut animation)?;
        match member {
            "animationName" if animation.css.as_ref().is_some_and(|css| !css.transition) => {
                string(scope, &animation.css.as_ref().unwrap().name)
            }
            "transitionProperty" if animation.css.as_ref().is_some_and(|css| css.transition) => {
                string(scope, &animation.css.as_ref().unwrap().name)
            }
            "id" => string(scope, &animation.name),
            "currentTime" => Ok(current_time(scope, animation.time())),
            "startTime" => Ok(current_time(scope, animation.start)),
            "playbackRate" => Ok(v8::Number::new(scope, animation.rate).into()),
            "pending" => Ok(v8::Boolean::new(scope, animation.pending.is_some()).into()),
            "playState" => string(scope, animation.play_state()),
            "replaceState" => string(
                scope,
                if animation.persisted {
                    "persisted"
                } else {
                    "active"
                },
            ),
            "effect" => {
                if let Some(effect) = &animation.effect {
                    let effect = effect.borrow();
                    let owner = effect.owner.upgrade().ok_or_else(failed)?;
                    Ok(new_platform_object(scope, &owner, "KeyframeEffect", effect.id)?.into())
                } else {
                    Ok(v8::null(scope).into())
                }
            }
            "timeline" => {
                if let Some(timeline) = &animation.timeline {
                    let owner = timeline.owner.upgrade().ok_or_else(failed)?;
                    Ok(
                        new_platform_object(scope, &owner, "DocumentTimeline", timeline.id.get())?
                            .into(),
                    )
                } else {
                    Ok(v8::null(scope).into())
                }
            }
            "ready" | "finished" => {
                let promise = if member == "ready" {
                    &animation.ready
                } else {
                    &animation.finished
                };
                let resolver = promise.local(scope).ok_or_else(failed)?;
                Ok(resolver.get_promise(scope).into())
            }
            _ => Err(type_error()),
        }
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn setter(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let result = (|| -> Result<(), V8BindingError> {
        let name = text(scope, args.data())?;
        let (interface, member) = name.split_once(':').ok_or_else(failed)?;
        if let Some(name) = member.strip_prefix("handler-") {
            return set_node_handler(scope, args.this(), name, args.get(0));
        }
        if interface == "KeyframeEffect" {
            let effect = effect(scope, args.this().into())?.ok_or_else(type_error)?;
            match member {
                "target" => {
                    let target = target(scope, args.get(0))?;
                    invalidate_effect_target(&effect.borrow());
                    effect.borrow_mut().target = target;
                    trace_effect(scope, args.this(), &effect.borrow())?
                }
                "pseudoElement" => {
                    let pseudo = pseudo(scope, args.get(0))?;
                    effect.borrow_mut().pseudo = pseudo
                }
                "composite" => {
                    let value = composite(scope, args.get(0), false)?;
                    effect.borrow_mut().composite = value
                }
                "iterationComposite" => {
                    let value = text(scope, args.get(0))?;
                    if !matches!(value.as_str(), "replace" | "accumulate") {
                        return Err(type_error());
                    }
                    effect.borrow_mut().iteration_composite = value
                }
                _ => return Err(type_error()),
            }
            if let Some(animation) = effect.borrow().animation.as_ref().and_then(RcWeak::upgrade) {
                link_target(&animation);
            }
            if let Some(owner) = effect.borrow().owner.upgrade() {
                invalidate_samples(&owner);
            }
            return Ok(());
        }
        let (state, record) = animation(scope, args.this())?;
        // Finish authored conversion before taking a mutable native record borrow.
        enum Converted {
            Text(String),
            Number(Option<f64>),
            Effect(Option<Rc<RefCell<Effect>>>),
            Timeline(Option<Rc<Timeline>>),
        }
        let converted = match member {
            "id" => Converted::Text(text(scope, args.get(0))?),
            "effect" => Converted::Effect(effect(scope, args.get(0))?),
            "timeline" => Converted::Timeline(timeline(scope, args.get(0))?),
            _ => Converted::Number(number(scope, args.get(0), member != "playbackRate")?),
        };
        let mut animation = record.borrow_mut();
        sync_css(&state, &mut animation)?;
        match converted {
            Converted::Text(value) => animation.name = value,
            Converted::Effect(value) => {
                if let Some(old) = animation.effect.take() {
                    invalidate_effect_target(&old.borrow());
                    old.borrow_mut().animation = None;
                }
                if let Some(effect) = &value {
                    if let Some(old) = effect
                        .borrow_mut()
                        .animation
                        .take()
                        .and_then(|old| old.upgrade())
                        && !Rc::ptr_eq(&old, &record)
                    {
                        old.borrow_mut().effect = None;
                    }
                    effect.borrow_mut().animation = Some(Rc::downgrade(&record));
                }
                animation.effect = value;
            }
            Converted::Timeline(value) => {
                let time = animation.time();
                animation.timeline = value;
                if let (Some(now), Some(time)) = (animation.now(), time) {
                    if animation.start.is_some() && animation.rate != 0. {
                        animation.start = Some(now - time / animation.rate)
                    }
                } else {
                    animation.hold = time;
                    animation.start = None;
                }
            }
            Converted::Number(value) => match member {
                "currentTime" => {
                    if value.is_none() && animation.time().is_some() {
                        return Err(type_error());
                    }
                    if let Some(css) = &animation.css {
                        state
                            .layout_provider
                            .borrow()
                            .as_ref()
                            .ok_or_else(failed)?
                            .control_css_animation(
                                css.id,
                                BindingCssAnimationControl::SetCurrentTime,
                                value,
                            )
                            .map_err(|_| failed())?;
                        animation.hold = value;
                    } else if animation.start.is_some() && animation.rate != 0. {
                        if let (Some(now), Some(value)) = (animation.now(), value) {
                            animation.start = Some(now - value / animation.rate);
                            animation.hold = None
                        } else {
                            animation.hold = value;
                        }
                    } else {
                        animation.hold = value;
                    }
                    if animation.pending == Some(Pending::Pause) {
                        animation.pending = None;
                        animation.start = None;
                        animation.ready.settle(scope, args.this().into(), false);
                    }
                    if animation.finished.settled && animation.play_state() != "finished" {
                        renew_promise(scope, &state, args.this(), &mut animation, false)?;
                    }
                }
                "startTime" => {
                    let time = animation.time();
                    animation.start = value;
                    if value.is_some() && animation.rate != 0. {
                        animation.hold = None
                    } else {
                        animation.hold = time;
                    }
                    if animation.pending.take().is_some() {
                        animation.ready.settle(scope, args.this().into(), false);
                    }
                }
                "playbackRate" => animation.rate = value.unwrap_or(0.),
                _ => return Err(type_error()),
            },
        }
        trace(scope, &state, args.this(), &animation, &[])?;
        update_root(scope, &state, args.this(), &animation);
        drop(animation);
        link_target(&record);
        Ok(())
    })();
    if result.is_ok()
        && let Some(state) = binding_state_for_receiver(scope, args.this())
    {
        drain_settlements(scope, &state);
    }
    if let Err(error) = result {
        throw_binding_error(scope, &error)
    }
}
fn timing_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    effect: &Effect,
    computed: bool,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let object = v8::Object::new(scope);
    let timing = &effect.timing;
    for (name, value) in [
        ("delay", timing.delay),
        ("endDelay", timing.end_delay),
        ("iterationStart", timing.iteration_start),
        ("iterations", timing.iterations),
    ] {
        let value = v8::Number::new(scope, value);
        set(scope, object, name, value.into())?;
    }
    let duration = if let Some(duration) = timing.duration {
        v8::Number::new(scope, duration).into()
    } else if computed {
        v8::Number::new(scope, 0.).into()
    } else {
        string(scope, "auto")?
    };
    set(scope, object, "duration", duration)?;
    for (name, value) in [
        (
            "fill",
            if computed && effect.fill == "auto" {
                "none"
            } else {
                effect.fill.as_str()
            },
        ),
        ("easing", effect.easing.as_str()),
        (
            "direction",
            match timing.direction {
                Direction::Normal => "normal",
                Direction::Reverse => "reverse",
                Direction::Alternate => "alternate",
                Direction::AlternateReverse => "alternate-reverse",
            },
        ),
    ] {
        let value = string(scope, value)?;
        set(scope, object, name, value)?;
    }
    if computed {
        let animation = effect.animation.as_ref().and_then(RcWeak::upgrade);
        let (time, rate) = animation
            .map(|animation| {
                let animation = animation.borrow();
                (animation.time(), animation.rate)
            })
            .unwrap_or((None, 1.));
        let sample = timing.sample(time, rate);
        for (name, value) in [
            ("endTime", Some(sample.end_time)),
            ("activeDuration", Some(sample.active_duration)),
            ("localTime", time),
            ("progress", sample.progress),
            ("currentIteration", sample.iteration),
        ] {
            let value = current_time(scope, value);
            set(scope, object, name, value)?;
        }
    }
    Ok(object)
}
fn finish_animation(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    wrapper: v8::Local<v8::Object>,
    animation: &mut Animation,
) -> Result<(), V8BindingError> {
    if animation.rate == 0. || animation.rate > 0. && animation.end().is_infinite() {
        return Err(dom_error("InvalidStateError"));
    }
    if let Some(css) = &animation.css {
        state
            .layout_provider
            .borrow()
            .as_ref()
            .ok_or_else(failed)?
            .control_css_animation(css.id, BindingCssAnimationControl::Finish, None)
            .map_err(|_| failed())?;
    }
    animation.hold = Some(if animation.rate > 0. {
        animation.end()
    } else {
        0.
    });
    if let Some(now) = animation.now() {
        animation.start = Some(now - animation.hold.unwrap() / animation.rate)
    }
    animation.pending = None;
    animation.ready.settle(scope, wrapper.into(), false);
    animation.finished.settle(scope, wrapper.into(), false);
    if !animation.finished_notified {
        queue_event(scope, state, animation, "finish")?;
        animation.finished_notified = true;
    }
    update_root(scope, state, wrapper, animation);
    Ok(())
}
fn commit_styles(
    _scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    record: &Rc<RefCell<Animation>>,
) -> Result<(), V8BindingError> {
    let (id, css, target, pseudo) = {
        let animation = record.borrow();
        if let Some(css) = &animation.css {
            (
                css.id,
                true,
                Some((state.clone(), css.target)),
                css.pseudo.clone(),
            )
        } else {
            let effect = animation
                .effect
                .as_ref()
                .ok_or_else(|| dom_error("InvalidStateError"))?
                .borrow();
            let target = effect
                .target
                .as_ref()
                .and_then(|target| target.owner.upgrade().map(|owner| (owner, target.node)));
            (animation.render_id, false, target, effect.pseudo.clone())
        }
    };
    let (owner, node) = target.ok_or_else(|| dom_error("InvalidStateError"))?;
    if pseudo.is_some() || !owner.dom.borrow().is_connected(node)? {
        return Err(dom_error("NoModificationAllowedError"));
    }
    refresh_layout(&owner)?;
    let values = owner
        .layout_provider
        .borrow()
        .as_ref()
        .ok_or_else(failed)?
        .animation_style_values(id, css)
        .map_err(|_| failed())?;
    let current = owner
        .dom
        .borrow()
        .arena()
        .get(node)?
        .attributes
        .get_null("style")
        .cloned()
        .unwrap_or_default();
    let mut entries = inline_style_entries(&current);
    for (name, value) in values {
        entries.insert(name, value);
    }
    let serialized = entries
        .into_iter()
        .map(|(name, value)| format!("{name}: {value};"))
        .collect::<Vec<_>>()
        .join(" ");
    owner
        .dom
        .borrow_mut()
        .set_attribute_ns(node, None, "style", serialized)?;
    owner
        .dom
        .borrow_mut()
        .arena_mut()
        .get_mut(node)?
        .style_attribute_cssom = true;
    owner.layout_cache_key.borrow_mut().take();
    Ok(())
}
fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let result = (|| -> Result<Option<v8::Local<v8::Value>>, V8BindingError> {
        let name = text(scope, args.data())?;
        let (interface, member) = name.split_once(':').ok_or_else(failed)?;
        if matches!(interface, "Element" | "Document") {
            let owner = binding_state_for_receiver(scope, args.this()).ok_or_else(type_error)?;
            let node = argument_node(scope, &owner, args.this().into())?;
            if member == "getAnimations" {
                let subtree = if interface == "Document" {
                    true
                } else {
                    let options = dictionary(args.get(0))?;
                    get(scope, options, "subtree")?.boolean_value(scope)
                };
                return Ok(Some(get_animations(scope, &owner, node, subtree)?.into()));
            }
            if !matches!(
                owner.dom.borrow().arena().get(node)?.kind,
                NodeKind::Element { .. }
            ) {
                return Err(type_error());
            }
            if args.length() == 0 {
                return Err(type_error());
            }
            let converted = convert_options(scope, args.get(1), true)?;
            let effect = new_effect(scope, &owner, args.this().into(), args.get(0), converted)?;
            let options = if converted.is_number() {
                None
            } else {
                dictionary(converted)?
            };
            let id = get(scope, options, "id")?;
            let id = if id.is_undefined() {
                String::new()
            } else {
                text(scope, id)?
            };
            let timeline_value = get(scope, options, "timeline")?;
            let timeline = if timeline_value.is_undefined() {
                let document = owner_document_for_node(&owner, node)?;
                Some(document_timeline(scope, &owner, document)?)
            } else {
                timeline(scope, timeline_value)?
            };
            let (wrapper, animation) =
                create_animation(scope, &owner, Some(effect), timeline, None, None)?;
            animation.borrow_mut().name = id;
            play(scope, &owner, wrapper, &mut animation.borrow_mut(), true)?;
            return Ok(Some(wrapper.into()));
        }
        if matches!(interface, "AnimationEffect" | "KeyframeEffect") {
            let effect = effect(scope, args.this().into())?.ok_or_else(type_error)?;
            sync_effect_css(&effect)?;
            match member {
                "getTiming" => {
                    return Ok(Some(timing_object(scope, &effect.borrow(), false)?.into()));
                }
                "getComputedTiming" => {
                    return Ok(Some(timing_object(scope, &effect.borrow(), true)?.into()));
                }
                "updateTiming" => {
                    let snapshot = {
                        let effect = effect.borrow();
                        Effect {
                            id: effect.id,
                            owner: effect.owner.clone(),
                            target: effect.target.clone(),
                            pseudo: effect.pseudo.clone(),
                            frames: effect.frames.clone(),
                            offsets: effect.offsets.clone(),
                            timing: effect.timing.clone(),
                            easing: effect.easing.clone(),
                            fill: effect.fill.clone(),
                            composite: effect.composite.clone(),
                            iteration_composite: effect.iteration_composite.clone(),
                            animation: None,
                        }
                    };
                    let (timing, easing, fill) = timing(scope, args.get(0), Some(&snapshot))?;
                    let mut effect = effect.borrow_mut();
                    effect.timing = timing;
                    effect.easing = easing;
                    effect.fill = fill;
                }
                "setKeyframes" => {
                    let (frames, offsets) = keyframes(scope, args.get(0))?;
                    let mut effect = effect.borrow_mut();
                    effect.frames = frames;
                    effect.offsets = offsets;
                }
                "getKeyframes" => {
                    let (frames, offsets) = {
                        let effect = effect.borrow();
                        (effect.frames.clone(), effect.offsets.clone())
                    };
                    let mut values = Vec::new();
                    for (index, frame) in frames.into_iter().enumerate() {
                        let object = v8::Object::new(scope);
                        for (name, value) in frame.properties {
                            let value = string(scope, &value)?;
                            set(scope, object, &idl_css_name(&name), value)?;
                        }
                        let value = current_time(scope, offsets[index]);
                        set(scope, object, "offset", value)?;
                        let value = v8::Number::new(scope, frame.offset);
                        set(scope, object, "computedOffset", value.into())?;
                        let easing = string(scope, &frame.easing)?;
                        set(scope, object, "easing", easing)?;
                        let composite = string(scope, &frame.composite)?;
                        set(scope, object, "composite", composite)?;
                        values.push(object.into());
                    }
                    return Ok(Some(v8::Array::new_with_elements(scope, &values).into()));
                }
                _ => return Err(type_error()),
            }
            if let Some(animation) = effect.borrow().animation.as_ref().and_then(RcWeak::upgrade) {
                link_target(&animation);
            }
            if let Some(owner) = effect.borrow().owner.upgrade() {
                invalidate_samples(&owner);
            }
            return Ok(None);
        }
        let (state, record) = animation(scope, args.this())?;
        if member == "commitStyles" {
            let boundary = custom_elements::ReactionBoundary::begin(scope)?;
            let result = commit_styles(scope, &state, &record);
            boundary.complete(scope);
            result?;
            return Ok(None);
        }
        let new_rate = if member == "updatePlaybackRate" {
            Some(number(scope, args.get(0), false)?.unwrap())
        } else {
            None
        };
        let mut animation = record.borrow_mut();
        sync_css(&state, &mut animation)?;
        if let Some(css) = &animation.css {
            let action = match member {
                "pause" => Some(BindingCssAnimationControl::Pause),
                "play" => Some(BindingCssAnimationControl::Play),
                "cancel" => Some(BindingCssAnimationControl::Cancel),
                _ => None,
            };
            if let Some(action) = action {
                state
                    .layout_provider
                    .borrow()
                    .as_ref()
                    .ok_or_else(failed)?
                    .control_css_animation(css.id, action, None)
                    .map_err(|_| failed())?;
            }
        }
        match member {
            "play" => play(scope, &state, args.this(), &mut animation, true)?,
            "pause" => {
                let time = animation.time().unwrap_or_else(|| {
                    if animation.rate >= 0. {
                        0.
                    } else {
                        animation.end()
                    }
                });
                if !time.is_finite() {
                    return Err(dom_error("InvalidStateError"));
                }
                animation.hold = Some(time);
                animation.start = None;
                if animation.ready.settled {
                    renew_promise(scope, &state, args.this(), &mut animation, true)?;
                }
                animation.pending = Some(Pending::Pause);
            }
            "finish" => finish_animation(scope, &state, args.this(), &mut animation)?,
            "cancel" => {
                let active = animation.play_state() != "idle";
                let error = dom_exception_value(scope, "AbortError")?;
                if animation.pending.is_some() {
                    animation.ready.settle(scope, error, true);
                    renew_promise(scope, &state, args.this(), &mut animation, true)?;
                    animation.ready.settle(scope, args.this().into(), false);
                }
                animation.finished.settle(scope, error, true);
                renew_promise(scope, &state, args.this(), &mut animation, false)?;
                animation.pending = None;
                animation.hold = None;
                animation.start = None;
                if active {
                    queue_event(scope, &state, &animation, "cancel")?;
                }
            }
            "reverse" => {
                if animation.now().is_none() {
                    return Err(dom_error("InvalidStateError"));
                }
                let time = animation.time();
                animation.rate = -animation.rate;
                animation.hold = time;
                animation.start = None;
                play(scope, &state, args.this(), &mut animation, true)?;
            }
            "updatePlaybackRate" => {
                let time = animation.time();
                let running = animation.play_state() == "running";
                animation.rate = new_rate.unwrap();
                animation.hold = time;
                animation.start = None;
                if running {
                    play(scope, &state, args.this(), &mut animation, false)?;
                }
            }
            "persist" => animation.persisted = true,
            _ => return Err(type_error()),
        }
        trace(scope, &state, args.this(), &animation, &[])?;
        update_root(scope, &state, args.this(), &animation);
        Ok(None)
    })();
    if result.is_ok()
        && let Some(state) = binding_state_for_receiver(scope, args.this())
    {
        drain_settlements(scope, &state);
    }
    match result {
        Ok(Some(value)) => rv.set(value),
        Ok(None) => {}
        Err(error) => throw_binding_error(scope, &error),
    }
}
pub(super) fn publish(state: &V8BindingState) -> Result<(), V8BindingError> {
    let provider = state.layout_provider.borrow().clone();
    let Some(provider) = provider else {
        return Ok(());
    };
    provider.set_animation_time(document_time(state));
    let records = contributing_records(state);
    let mut effects = Vec::new();
    for record in records {
        let animation = record.borrow();
        if animation.css.is_some() {
            continue;
        }
        let Some(effect) = &animation.effect else {
            continue;
        };
        let effect = effect.borrow();
        let Some(target) = &effect.target else {
            continue;
        };
        let Some(owner) = target.owner.upgrade() else {
            continue;
        };
        if !Rc::ptr_eq(&owner.dom, &state.dom) || owner.document_root() != state.document_root() {
            continue;
        }
        let sample = effect.timing.sample(animation.time(), animation.rate);
        effects.push(BindingAnimationEffect {
            id: animation.render_id,
            target: target.node,
            pseudo: effect.pseudo.clone(),
            keyframes: effect.frames.clone(),
            progress: sample.progress,
            current_iteration: sample.iteration.unwrap_or(0.),
            composite: effect.composite.clone(),
        });
    }
    provider
        .publish_animation_effects(&effects)
        .map_err(|_| failed())?;
    state.web_animations.borrow_mut().dirty_samples = false;
    Ok(())
}
fn css_effect(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    css: &BindingCssAnimation,
) -> Result<Rc<RefCell<Effect>>, V8BindingError> {
    let wrapper = wrap_node(scope, state, css.target)?;
    let options = v8::Object::new(scope);
    for (name, value) in [
        ("delay", css.delay_ms),
        ("duration", css.iteration_duration_ms),
        ("iterations", css.iterations),
    ] {
        let value = v8::Number::new(scope, value);
        set(scope, options, name, value.into())?;
    }
    for (name, value) in [
        ("direction", css.timing_direction.as_str()),
        ("fill", css.timing_fill.as_str()),
        ("easing", css.timing_easing.as_str()),
    ] {
        let value = string(scope, value)?;
        set(scope, options, name, value)?;
    }
    if let Some(pseudo) = &css.pseudo {
        let value = string(scope, pseudo)?;
        set(scope, options, "pseudoElement", value)?;
    }
    let frames = v8::null(scope);
    new_effect(scope, state, wrapper.into(), frames.into(), options.into())
}
fn get_animations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    subtree: bool,
) -> Result<v8::Local<'s, v8::Array>, V8BindingError> {
    refresh_layout(state)?;
    let snapshots = state
        .layout_provider
        .borrow()
        .as_ref()
        .map(|provider| provider.css_animations())
        .transpose()
        .map_err(|_| failed())?
        .unwrap_or_default();
    for css in snapshots {
        let existing = state.web_animations.borrow().css.get(&css.id).copied();
        if let Some(id) = existing
            && let Some(PlatformObjectRecord::WebAnimation(animation)) =
                state.platform_objects.borrow().get(&id)
        {
            animation.borrow_mut().css = Some(css);
            continue;
        }
        let timeline = default_timeline(scope, state)?;
        let effect = css_effect(scope, state, &css)?;
        let (wrapper, animation) = create_animation(
            scope,
            state,
            Some(effect),
            Some(timeline),
            None,
            Some(css.clone()),
        )?;
        let id = animation.borrow().id;
        state.web_animations.borrow_mut().css.insert(css.id, id);
        {
            let mut animation = animation.borrow_mut();
            animation.hold = Some(css.current_time_ms);
            animation.css = Some(css);
            update_root(scope, state, wrapper, &animation);
        }
    }
    let records = contributing_records(state);
    let mut values = Vec::new();
    for record in records {
        let mut animation = record.borrow_mut();
        sync_css(state, &mut animation)?;
        let target = if let Some(css) = &animation.css {
            if !css.in_effect && css.play_state != "running" && animation.pending.is_none() {
                continue;
            }
            Some(css.target)
        } else {
            animation.effect.as_ref().and_then(|effect| {
                let effect = effect.borrow();
                let sample = effect.timing.sample(animation.time(), animation.rate);
                if sample.progress.is_none() && animation.play_state() != "running" {
                    return None;
                }
                effect.target.as_ref().and_then(|target| {
                    target
                        .owner
                        .upgrade()
                        .filter(|owner| Rc::ptr_eq(&owner.dom, &state.dom))
                        .map(|_| target.node)
                })
            })
        };
        let Some(target) = target else { continue };
        if animation.play_state() == "idle" {
            continue;
        }
        let includes = target == node
            || subtree
                && state
                    .dom
                    .borrow()
                    .arena()
                    .ancestors(target)?
                    .contains(&node);
        if includes {
            let owner = animation.owner.upgrade().ok_or_else(failed)?;
            values.push(
                new_platform_object(scope, &owner, animation.interface(), animation.id)?.into(),
            );
        }
    }
    Ok(v8::Array::new_with_elements(scope, &values))
}
pub(super) fn publish_rendering_samples(
    state: &V8BindingState,
    rendering_time: Instant,
) -> Result<(), V8BindingError> {
    if state.execution_tasks_stopped() {
        return Ok(());
    }
    struct Clock<'a> {
        state: &'a V8BindingState,
        previous: Option<f64>,
    }
    impl Drop for Clock<'_> {
        fn drop(&mut self) {
            self.state.web_animations.borrow_mut().rendering_time = self.previous;
        }
    }
    let elapsed = rendering_time
        .checked_duration_since(state.clock_origin.get())
        .unwrap_or_default();
    let previous = state
        .web_animations
        .borrow_mut()
        .rendering_time
        .replace((elapsed.as_micros() / 100) as f64 / 10.);
    let _clock = Clock { state, previous };
    publish(state)
}
pub(super) fn tick(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    rendering_time: Instant,
) -> Result<(), V8BindingError> {
    if state.execution_tasks_stopped() {
        return Ok(());
    }
    let elapsed = rendering_time
        .checked_duration_since(state.clock_origin.get())
        .unwrap_or_default();
    state.web_animations.borrow_mut().rendering_time =
        Some((elapsed.as_micros() / 100) as f64 / 10.);
    let roots = state
        .web_animations
        .borrow()
        .active
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for root in roots {
        let wrapper = v8::Local::new(scope, &root);
        let (_, record) = animation(scope, wrapper)?;
        let mut animation = record.borrow_mut();
        sync_css(state, &mut animation)?;
        if let Some(pending) = animation.pending {
            let now = animation.now();
            if pending == Pending::Pause || now.is_some() {
                if pending == Pending::Play {
                    if animation.rate != 0. {
                        animation.start =
                            Some(now.unwrap() - animation.hold.unwrap_or(0.) / animation.rate);
                        animation.hold = None;
                    }
                } else {
                    animation.start = None;
                }
                animation.pending = None;
                animation.ready.settle(scope, wrapper.into(), false);
            }
        }
        if animation.play_state() == "finished" && !animation.finished_notified {
            finish_animation(scope, state, wrapper, &mut animation)?;
        }
        update_root(scope, state, wrapper, &animation);
    }
    drain_settlements(scope, state);
    scope.perform_microtask_checkpoint();
    if scope.is_execution_terminating() {
        return Err(V8BindingError::JavaScriptException);
    }
    state.finish_platform_task_checkpoint()?;
    if state.execution_tasks_stopped() {
        return Ok(());
    }
    publish(state)?;
    collect_css_events(scope, state)?;
    drain_events(scope, state)
}
pub(super) fn drain_events(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    drain_settlements(scope, state);
    let admitted = state.web_animations.borrow().events.len();
    for _ in 0..admitted {
        let next = state.web_animations.borrow_mut().events.pop_front();
        let Some(mut notification) = next else { break };
        match notification.resume(scope, state) {
            Ok(true) => notification.finish(scope, state),
            Ok(false) => {
                state
                    .web_animations
                    .borrow_mut()
                    .events
                    .push_front(notification);
                break;
            }
            Err(error) => {
                state
                    .web_animations
                    .borrow_mut()
                    .events
                    .push_front(notification);
                return Err(error);
            }
        }
        if state.execution_tasks_stopped() {
            break;
        }
    }
    Ok(())
}
pub(super) fn retire(state: &V8BindingState) {
    let records = state
        .platform_objects
        .borrow()
        .values()
        .filter_map(|record| match record {
            PlatformObjectRecord::WebAnimation(animation) => Some(animation.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for animation in records {
        let animation = animation.borrow();
        if let Some(target) = animation.effect.as_ref().and_then(|effect| {
            effect
                .borrow()
                .target
                .as_ref()
                .and_then(|target| target.owner.upgrade())
        }) && target.instance_id != state.instance_id
        {
            invalidate_samples(&target);
        }
    }
    let mut animations = state.web_animations.borrow_mut();
    animations.active.clear();
    animations.dirty_samples = false;
    animations.observed_css.clear();
    animations.settlements.clear();
    for notification in animations.events.drain(..) {
        notification.cancel(state);
    }
}
fn accessor(
    scope: &mut v8::PinScope,
    prototype: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
    writable: bool,
) -> Result<(), V8BindingError> {
    let name = v8::String::new(scope, member).ok_or_else(failed)?;
    let data = v8::String::new(scope, &format!("{interface}:{member}")).ok_or_else(failed)?;
    let get = v8::Function::builder(getter)
        .data(data.into())
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or_else(failed)?;
    let set: v8::Local<v8::Value> = if writable {
        v8::Function::builder(setter)
            .data(data.into())
            .length(1)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or_else(failed)?
            .into()
    } else {
        v8::undefined(scope).into()
    };
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(get.into(), set);
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    prototype
        .define_property(scope, name.into(), &descriptor)
        .ok_or_else(failed)?;
    Ok(())
}
fn method(
    scope: &mut v8::PinScope,
    prototype: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
    length: i32,
) -> Result<(), V8BindingError> {
    let data = v8::String::new(scope, &format!("{interface}:{member}")).ok_or_else(failed)?;
    let function = v8::Function::builder(operation)
        .data(data.into())
        .length(length)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or_else(failed)?;
    set(scope, prototype, member, function.into())
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    prototype: v8::Local<v8::Object>,
    interface: &str,
) -> Result<(), V8BindingError> {
    if matches!(interface, "Element" | "Document") {
        for name in HANDLER_TYPES {
            let member = format!("handler-{name}");
            accessor(scope, prototype, interface, &member, true)?;
            let from = v8::String::new(scope, &member).ok_or_else(failed)?;
            let descriptor = prototype
                .get_own_property_descriptor(scope, from.into())
                .ok_or_else(failed)?;
            let descriptor = descriptor.to_object(scope).ok_or_else(failed)?;
            let getter = get(scope, Some(descriptor), "get")?;
            let setter = get(scope, Some(descriptor), "set")?;
            let mut desc = v8::PropertyDescriptor::new_from_get_set(getter, setter);
            desc.set_enumerable(true);
            desc.set_configurable(true);
            let key = v8::String::new(scope, &handler_property(name)).ok_or_else(failed)?;
            prototype
                .define_property(scope, key.into(), &desc)
                .ok_or_else(failed)?;
            prototype.delete(scope, from.into());
        }
    }
    match interface {
        "Element" => {
            method(scope, prototype, interface, "animate", 1)?;
            method(scope, prototype, interface, "getAnimations", 0)?;
        }
        "Document" => {
            method(scope, prototype, interface, "getAnimations", 0)?;
            accessor(scope, prototype, interface, "timeline", false)?;
        }
        "Animation" => {
            for member in [
                "id",
                "effect",
                "timeline",
                "startTime",
                "currentTime",
                "playbackRate",
            ] {
                accessor(scope, prototype, interface, member, true)?;
            }
            for member in ["playState", "pending", "replaceState", "ready", "finished"] {
                accessor(scope, prototype, interface, member, false)?;
            }
            for member in [
                "cancel",
                "finish",
                "play",
                "pause",
                "reverse",
                "commitStyles",
                "persist",
            ] {
                method(scope, prototype, interface, member, 0)?;
            }
            method(scope, prototype, interface, "updatePlaybackRate", 1)?;
        }
        "CSSAnimation" => accessor(scope, prototype, interface, "animationName", false)?,
        "CSSTransition" => accessor(scope, prototype, interface, "transitionProperty", false)?,
        "AnimationPlaybackEvent" => {
            for member in ["currentTime", "timelineTime"] {
                accessor(scope, prototype, interface, member, false)?;
            }
        }
        "AnimationEvent" | "TransitionEvent" => {
            for member in [
                if interface == "AnimationEvent" {
                    "animationName"
                } else {
                    "propertyName"
                },
                "elapsedTime",
                "pseudoElement",
            ] {
                accessor(scope, prototype, interface, member, false)?;
            }
        }
        "AnimationTimeline" => accessor(scope, prototype, interface, "currentTime", false)?,
        "AnimationEffect" => {
            for member in ["getTiming", "getComputedTiming", "updateTiming"] {
                method(scope, prototype, interface, member, 0)?;
            }
        }
        "KeyframeEffect" => {
            for member in ["target", "pseudoElement", "composite", "iterationComposite"] {
                accessor(scope, prototype, interface, member, true)?;
            }
            for member in ["getKeyframes", "setKeyframes"] {
                method(scope, prototype, interface, member, 0)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn css_observation_pending(state: &V8BindingState) -> bool {
    let provider = state.layout_provider.borrow().clone();
    let Some(provider) = provider else {
        return false;
    };
    let Ok(snapshots) = provider.css_animations() else {
        return true;
    };
    let observed = state.web_animations.borrow();
    if snapshots.len() != observed.observed_css.len() {
        return true;
    }
    snapshots.iter().any(|snapshot| {
        observed
            .observed_css
            .get(&snapshot.id)
            .is_none_or(|old| !css_event_changes(Some(&old.snapshot), Some(snapshot)).is_empty())
    })
}
pub(super) fn has_pending(state: &V8BindingState) -> bool {
    if css_observation_pending(state) {
        return true;
    }
    let animations = state.web_animations.borrow();
    animations.dirty_samples
        || !animations.active.is_empty()
        || !animations.events.is_empty()
        || !animations.settlements.is_empty()
}
pub(super) fn delay(state: &V8BindingState) -> Option<std::time::Duration> {
    let css = css_observation_pending(state);
    let animations = state.web_animations.borrow();
    if !animations.events.is_empty() || !animations.settlements.is_empty() {
        Some(std::time::Duration::ZERO)
    } else if css || animations.dirty_samples || !animations.active.is_empty() {
        Some(std::time::Duration::from_millis(16))
    } else {
        None
    }
}
pub(super) fn release_collected(state: &V8BindingState, id: u64) {
    let removable = matches!(
        state.platform_objects.borrow().get(&id),
        Some(
            PlatformObjectRecord::WebAnimation(_)
                | PlatformObjectRecord::WebAnimationEffect(_)
                | PlatformObjectRecord::WebAnimationTimeline(_)
        )
    );
    if removable {
        state.platform_objects.borrow_mut().remove(&id);
        state
            .web_animations
            .borrow_mut()
            .css
            .retain(|_, native| *native != id);
        state
            .web_animations
            .borrow_mut()
            .timelines
            .retain(|_, timeline| *timeline != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn native_animation_objects_promises_and_pseudo_samples_follow_playback() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id='target'></div>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{globalThis.a=document.getElementById('target').animate([{opacity:'0'},{opacity:'1'}],{duration:100,fill:'both',pseudoElement:'::picker-icon'});if(!(a instanceof Animation)||!(a.effect instanceof KeyframeEffect)||a.timeline!==document.timeline||!a.pending)throw Error('objects');a.pause();a.currentTime=50;if(a.playState!=='paused'||a.pending||a.effect.pseudoElement!=='::picker-icon'||a.effect.getComputedTiming().progress!==0.5)throw Error('paused sample');globalThis.finishes=0;a.onfinish=e=>{if(!(e instanceof AnimationPlaybackEvent)||!e.isTrusted||e.currentTime!==100)throw Error('finish event');finishes++};a.finish();return a.playState==='finished'&&finishes===0})()"#).is_true());
        assert_eq!(delay(&state), Some(std::time::Duration::ZERO));
        drain_events(scope, &state).unwrap();
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "finishes===1").is_true());
        assert_eq!(delay(&state), None);
    }
    #[test]
    fn animation_conversion_observes_options_before_keyframes_and_preserves_exceptions() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{let log=[];let a=document.body.animate([{get opacity(){log.push('frame');return 0}}],{get duration(){log.push('duration');return{valueOf(){log.push('number');return 100}}},get id(){log.push('id');return'example'}});if(log.join()!=='duration,number,id,frame'||a.id!=='example')throw Error(log);a.cancel();const marker={};let closed=false;const frames={[Symbol.iterator](){return{next(){return{done:false,value:{get offset(){throw marker}}}},return(){closed=true;return{}}}}};try{new KeyframeEffect(document.body,frames);throw Error('accepted')}catch(e){if(e!==marker||!closed)throw e}const setter=Object.getOwnPropertyDescriptor(Animation.prototype,'id').set;setter.call(a);return a.id==='undefined'})()"#).is_true());
    }
    #[test]
    fn animation_event_dictionaries_are_native_readonly_and_branded() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{let order=[];const e=new AnimationEvent('animationstart',{get animationName(){order.push('name');return'grow'},get bubbles(){order.push('bubbles');return true},get elapsedTime(){order.push('elapsed');return 0.25},pseudoElement:'::before'});if(order.join()!=='name,bubbles,elapsed'||e.animationName!=='grow'||e.elapsedTime!==0.25||e.pseudoElement!=='::before'||!e.bubbles||e.isTrusted)throw Error('event');const p=new AnimationPlaybackEvent('finish');if(p.currentTime!==null||p.timelineTime!==null)throw Error('nullable');const t=new TransitionEvent('transitionend',{propertyName:'opacity',elapsedTime:2});if(t.propertyName!=='opacity'||t.elapsedTime!==2)throw Error('transition');try{Object.getOwnPropertyDescriptor(AnimationEvent.prototype,'animationName').get.call(p);return false}catch(e){return e instanceof TypeError}})()"#).is_true());
    }
    #[test]
    fn css_lifecycle_crossings_include_skipped_active_interval_and_cancellation() {
        let dom = oryn_dom::html::parse("<body>").into_runtime().unwrap();
        let mut snapshot = BindingCssAnimation {
            timing_direction: "normal".into(),
            timing_fill: "none".into(),
            timing_easing: "linear".into(),
            id: 1,
            target: dom.document(),
            pseudo: None,
            name: "grow".into(),
            current_time_ms: -10.,
            duration_ms: 210.,
            play_state: "running".into(),
            transition: false,
            delay_ms: 10.,
            iteration_duration_ms: 100.,
            iterations: 2.,
            in_effect: false,
        };
        assert!(css_event_changes(None, Some(&snapshot)).is_empty());
        let before = snapshot.clone();
        snapshot.current_time_ms = 210.;
        assert_eq!(
            css_event_changes(Some(&before), Some(&snapshot)),
            vec![("animationstart", 0.), ("animationend", 0.2)]
        );
        assert!(css_event_changes(Some(&snapshot), None).is_empty());
        snapshot.current_time_ms = 160.;
        assert_eq!(
            css_event_changes(Some(&snapshot), None),
            vec![("animationcancel", 0.15)]
        );
        let active = snapshot.clone();
        snapshot.current_time_ms = 60.;
        assert_eq!(
            css_event_changes(Some(&active), Some(&snapshot)),
            vec![("animationiteration", 0.1)]
        );
    }
    #[derive(Default)]
    struct Provider {
        time: Cell<f64>,
        fail: Cell<bool>,
        css: RefCell<Vec<BindingCssAnimation>>,
        samples: RefCell<Vec<BindingAnimationEffect>>,
    }
    impl BindingLayoutProvider for Provider {
        fn compute_layout(
            &self,
            _: &BTreeMap<NodeId, BTreeSet<String>>,
        ) -> Result<BindingLayout, String> {
            Ok(BindingLayout::default())
        }
        fn set_animation_time(&self, time: f64) {
            self.time.set(time)
        }
        fn css_animations(&self) -> Result<Vec<BindingCssAnimation>, String> {
            Ok(self.css.borrow().clone())
        }
        fn publish_animation_effects(
            &self,
            effects: &[BindingAnimationEffect],
        ) -> Result<(), String> {
            if self.fail.replace(false) {
                return Err("injected animation publication".into());
            }
            *self.samples.borrow_mut() = effects.to_vec();
            Ok(())
        }
    }
    #[test]
    fn keyframe_sequences_preserve_specified_offsets_and_rendered_progress() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let provider = Rc::new(Provider::default());
        state.set_layout_provider(provider.clone());
        assert!(eval(scope,r#"(()=>{globalThis.a=document.body.animate({backgroundColor:['red','green','blue'],offset:[0,null,1],easing:['linear','steps(1,end)']},{duration:100,fill:'both'});a.pause();a.currentTime=50;const f=a.effect.getKeyframes();if(f.length!==3||f[1].offset!==null||f[1].computedOffset!==0.5||f[1].easing!=='steps(1,end)'||f[1].backgroundColor!=='green')throw Error('frames');return a.effect.getComputedTiming().progress===0.5})()"#).is_true());
        publish(&state).unwrap();
        assert_eq!(provider.samples.borrow()[0].progress, Some(0.5));
        assert_eq!(delay(&state), None);
        eval(
            scope,
            "a.effect.setKeyframes([{opacity:0},{opacity:1}]);a.effect.target=null",
        );
        assert!(state.web_animations.borrow().dirty_samples);
        provider.fail.set(true);
        assert!(publish(&state).is_err());
        assert!(state.web_animations.borrow().dirty_samples);
        publish(&state).unwrap();
        assert!(provider.samples.borrow().is_empty());
    }
    #[test]
    fn rendering_clock_is_stable_through_callbacks_and_reentrant_events_wait() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let provider = Rc::new(Provider::default());
        state.set_layout_provider(provider.clone());
        state.page_revealed.set(true);
        eval(
            scope,
            "globalThis.times=[];requestAnimationFrame(t=>{times.push(t,document.timeline.currentTime);throw Error('callback')});requestAnimationFrame(t=>times.push(t,document.timeline.currentTime))",
        );
        let rendering = state.clock_origin.get() + std::time::Duration::from_millis(125);
        animation_frames::run(scope, &state, rendering).unwrap();
        assert!(eval(scope, "times.length===4&&times.every(t=>t===125)").is_true());
        assert!(state.web_animations.borrow().rendering_time.is_none());
        assert_eq!(provider.time.get(), 125.);
        eval(
            scope,
            "globalThis.a=document.body.animate([],100);globalThis.calls=0;a.onfinish=()=>{calls++;if(calls===1){a.cancel();a.finish()}};a.finish()",
        );
        drain_events(scope, &state).unwrap();
        assert!(eval(scope, "calls===1").is_true());
        drain_events(scope, &state).unwrap();
        assert!(eval(scope, "calls===2").is_true());
        assert!(state.web_animations.borrow().dirty_samples);
        publish(&state).unwrap();
        assert_eq!(delay(&state), None);
    }

    #[test]
    fn trusted_css_fallback_is_per_target_and_synthetic_events_keep_their_type() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id='child'></div>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.child=document.getElementById('child');globalThis.seen=[];if(child.onanimationstart!==null||child.onwebkitanimationstart!==null)throw Error('default');child.onwebkitanimationstart=e=>seen.push('child:'+e.type);document.body.onanimationstart=e=>seen.push('body:'+e.type);child.dispatchEvent(new AnimationEvent('animationstart',{bubbles:true}));if(seen.join()!=='body:animationstart')throw Error('synthetic fallback');seen=[]",
        );
        let target = eval(scope, "child").to_object(scope).unwrap();
        let node = argument_node(scope, &state, target.into()).unwrap();
        let snapshot = BindingCssAnimation {
            timing_direction: "normal".into(),
            timing_fill: "none".into(),
            timing_easing: "linear".into(),
            id: 1,
            target: node,
            pseudo: None,
            name: "grow".into(),
            current_time_ms: 0.,
            duration_ms: 100.,
            play_state: "running".into(),
            transition: false,
            delay_ms: 0.,
            iteration_duration_ms: 100.,
            iterations: 1.,
            in_effect: true,
        };
        let event =
            css_notification(scope, &state, target, &snapshot, "animationstart", 0.).unwrap();
        state.web_animations.borrow_mut().events.push_back(event);
        drain_events(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "seen.join()==='child:webkitAnimationStart,body:animationstart'"
            )
            .is_true()
        );
        eval(
            scope,
            "seen=[];child.addEventListener('animationstart',e=>seen.push('listener:'+e.type))",
        );
        let event =
            css_notification(scope, &state, target, &snapshot, "animationstart", 0.).unwrap();
        state.web_animations.borrow_mut().events.push_back(event);
        drain_events(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "seen.join()==='listener:animationstart,body:animationstart'"
            )
            .is_true()
        );
        clear_document_stream_listeners(scope, &state, state.document_root()).unwrap();
        assert!(
            eval(
                scope,
                "child.onwebkitanimationstart===null&&document.body.onanimationstart===null"
            )
            .is_true()
        );
    }
    #[test]
    fn adopted_effects_keep_original_animation_identity_and_publish_in_destination() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let (first_state, animation, target) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><div id='moving'></div>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap();
            let target = eval(scope, "document.getElementById('moving')")
                .to_object(scope)
                .unwrap();
            let animation=eval(scope,"globalThis.a=document.getElementById('moving').animate([{opacity:0},{opacity:1}],{duration:100,fill:'both'});a.pause();a.currentTime=25;a").to_object(scope).unwrap();
            (
                state,
                v8::Global::new(scope, animation),
                v8::Global::new(scope, target),
            )
        };
        let scope = &mut v8::ContextScope::new(scope, second);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap();
        let provider = Rc::new(Provider::default());
        state.set_layout_provider(provider.clone());
        let target = v8::Local::new(scope, &target);
        let animation = v8::Local::new(scope, &animation);
        let global = second.global(scope);
        set_object_property(scope, global, "moving", target.into()).unwrap();
        set_object_property(scope, global, "foreignAnimation", animation.into()).unwrap();
        assert!(eval(scope,"(()=>{try{document.body.moveBefore(moving,null);return false}catch(e){return e.name==='HierarchyRequestError'&&moving.ownerDocument!==document}})()").is_true());
        assert!(eval(scope,"document.body.appendChild(document.adoptNode(moving));moving.getAnimations()[0]===foreignAnimation&&foreignAnimation.effect.target===moving").is_true());
        publish(&state).unwrap();
        assert_eq!(provider.samples.borrow().len(), 1);
        assert_eq!(provider.samples.borrow()[0].progress, Some(0.25));
        let source_provider = Rc::new(Provider::default());
        first_state.set_layout_provider(source_provider.clone());
        publish(&first_state).unwrap();
        assert!(source_provider.samples.borrow().is_empty());
    }

    #[test]
    fn rendering_failure_and_termination_preserve_event_cursor_and_release_clock() {
        fn terminate(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            scope.terminate_execution();
        }
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.page_revealed.set(true);
        eval(
            scope,
            "globalThis.a=document.body.animate([],100);globalThis.seen=[];a.addEventListener('finish',()=>seen.push('first'),{once:true});a.addEventListener('finish',()=>seen.push('second'));a.finish()",
        );
        native_platform::publication::fail_after_next_callback();
        assert!(animation_frames::run(scope, &state, Instant::now()).is_err());
        assert!(state.web_animations.borrow().rendering_time.is_none());
        assert!(eval(scope, "seen.join()==='first'").is_true());
        drain_events(scope, &state).unwrap();
        assert!(eval(scope, "seen.join()==='first,second'").is_true());
        let function = v8::Function::new(scope, terminate).unwrap();
        let global = context.global(scope);
        set_object_property(scope, global, "terminateAnimationTask", function.into()).unwrap();
        eval(
            scope,
            "a.cancel();seen=[];a.addEventListener('finish',()=>{seen.push('terminating');terminateAnimationTask()},{once:true});a.addEventListener('finish',()=>seen.push('after'));a.finish()",
        );
        let _ = animation_frames::run(scope, &state, Instant::now());
        assert!(scope.is_execution_terminating());
        assert!(state.web_animations.borrow().rendering_time.is_none());
        scope.cancel_terminate_execution();
        drain_events(scope, &state).unwrap();
        assert!(eval(scope, "seen.join()==='second,terminating,after'").is_true());
        assert_eq!(delay(&state), None);
    }

    #[test]
    fn hidden_document_drains_playback_events_without_running_animation_frames() {
        struct Hidden;
        impl crate::platform_control::Backend for Hidden {
            fn request(
                &self,
                _: crate::worker_wire::Creator,
                _: crate::platform_control::Request,
            ) -> Result<crate::platform_control::Response, String> {
                Ok(crate::platform_control::Response::WindowState {
                    hidden: true,
                    revision: 1,
                })
            }
        }
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.finished=0;globalThis.frames=0;globalThis.a=document.body.animate([],100);a.onfinish=()=>finished++;a.finish();requestAnimationFrame(()=>frames++)",
        );
        state.set_native_platform_backend(std::sync::Arc::new(Hidden));
        animation_frames::run(scope, &state, Instant::now()).unwrap();
        assert!(eval(scope, "finished===1&&frames===0").is_true());
        assert_eq!(delay(&state), None);
        assert!(state.web_animations.borrow().rendering_time.is_none());
    }

    #[test]
    fn promise_then_access_can_reenter_animation_after_native_borrows_are_released() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{let reads=0;Object.defineProperty(Animation.prototype,'then',{configurable:true,get(){reads++;this.id='reentered';this.effect?.getComputedTiming();return undefined}});globalThis.a=document.body.animate([{opacity:0},{opacity:1}],100);a.pause();a.currentTime=50;a.finish();delete Animation.prototype.then;return reads>=2&&a.id==='reentered'&&a.playState==='finished'})()"#).is_true());
        drain_events(scope, &state).unwrap();
    }

    #[test]
    fn playback_ready_microtasks_precede_animation_frame_callbacks() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.page_revealed.set(true);
        eval(
            scope,
            "globalThis.order=[];globalThis.a=document.body.animate([{opacity:0},{opacity:1}],10000);a.ready.then(()=>order.push('ready'));requestAnimationFrame(()=>order.push('frame'))",
        );
        animation_frames::run(scope, &state, Instant::now()).unwrap();
        assert!(
            eval(
                scope,
                "order.join()==='ready,frame'&&!a.pending&&a.playState==='running'"
            )
            .is_true()
        );
    }

    #[test]
    fn inert_document_timeline_is_stable_inactive_and_does_not_busy_tick() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,"globalThis.inert=document.implementation.createHTMLDocument();globalThis.a=inert.body.animate([{opacity:0},{opacity:1}],100);inert.timeline===inert.timeline&&inert.timeline!==document.timeline&&inert.timeline.currentTime===null&&a.timeline===inert.timeline").is_true());
        assert_eq!(delay(&state), None);
    }

    #[test]
    fn paused_css_start_and_removal_each_request_only_one_observation() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.page_revealed.set(true);
        let provider = Rc::new(Provider::default());
        state.set_layout_provider(provider.clone());
        let body = eval(
            scope,
            "globalThis.events=[];document.body.onanimationstart=e=>events.push(e.type);document.body.onanimationcancel=e=>events.push(e.type);document.body",
        );
        let node = argument_node(scope, &state, body).unwrap();
        provider.css.borrow_mut().push(BindingCssAnimation {
            timing_direction: "normal".into(),
            timing_fill: "both".into(),
            timing_easing: "linear".into(),
            id: 1,
            target: node,
            pseudo: None,
            name: "paused".into(),
            current_time_ms: 0.,
            duration_ms: 100.,
            play_state: "paused".into(),
            transition: false,
            delay_ms: 0.,
            iteration_duration_ms: 100.,
            iterations: 1.,
            in_effect: true,
        });
        assert_eq!(delay(&state), Some(std::time::Duration::from_millis(16)));
        animation_frames::run(scope, &state, Instant::now()).unwrap();
        assert_eq!(delay(&state), None);
        provider.css.borrow_mut().clear();
        assert!(delay(&state).is_some());
        animation_frames::run(scope, &state, Instant::now()).unwrap();
        assert_eq!(delay(&state), None);
        assert!(eval(scope, "events.join()==='animationstart,animationcancel'").is_true());
    }
}
