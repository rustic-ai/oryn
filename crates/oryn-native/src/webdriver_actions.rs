//! Validated native action requests. Parsing performs no input dispatch and
//! grants no privileges; the authenticated session supplies realm authority.
use crate::host::webdriver_script::{ELEMENT_KEY, ScriptError, ScriptValue};
use serde::{Deserialize, Serialize};

pub const MAX_SOURCES: usize = 16;
pub const MAX_ACTIONS_PER_SOURCE: usize = 4096;
const MAX_SAFE: u64 = 9_007_199_254_740_991;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PointerKind {
    Mouse,
    Pen,
    Touch,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    None,
    Key,
    Pointer(PointerKind),
    Wheel,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Origin {
    Viewport,
    Pointer,
    Element(String),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PointerProperties {
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub pressure: Option<f64>,
    pub tangential_pressure: Option<f64>,
    pub tilt_x: Option<i64>,
    pub tilt_y: Option<i64>,
    pub twist: Option<u64>,
    pub altitude_angle: Option<f64>,
    pub azimuth_angle: Option<f64>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsKeyModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
    pub alt_graph: bool,
    pub caps_lock: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OsKey {
    #[serde(default)]
    pub interpretation: Option<u64>,
    #[serde(default)]
    pub composing: bool,
    pub value: Vec<u16>,
    pub code: String,
    pub location: u32,
    pub repeat: bool,
    pub down: bool,
    pub modifiers: OsKeyModifiers,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OsKeyReceipt {
    pub continuation_done: bool,
    pub source: String,
    pub state: Option<oryn_js::composition::StateSnapshot>,
    pub target: Option<crate::dom::NodeId>,
    pub token: u64,
    pub accepted: bool,
    pub interpret: bool,
    pub context: crate::host::webdriver_script::AutomationRealmSnapshot,
}
fn native_key_name(value: &[u16]) -> bool {
    single_code_point(value)
        || [
            "CapsLock",
            "NumLock",
            "ScrollLock",
            "Fn",
            "ContextMenu",
            "Unidentified",
        ]
        .iter()
        .any(|name| value.iter().copied().eq(name.encode_utf16()))
}
impl OsKey {
    fn valid(&self) -> bool {
        native_key_name(&self.value)
            && self.code.len() <= 64
            && self.code.is_ascii()
            && self.location <= 3
            && self
                .interpretation
                .is_none_or(|token| token > 0 && self.down)
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Action {
    OsKey(OsKey),
    OsText {
        token: u64,
        sequence: u64,
        operation: oryn_js::composition::Operation,
    },
    OsModifiers(OsKeyModifiers),
    /// Trusted OS host only; WebDriver JSON parsing never produces this variant.
    OsPointerMove {
        x: f64,
        y: f64,
        screen_x: f64,
        screen_y: f64,
    },
    Pause {
        duration_ms: u64,
    },
    Key {
        down: bool,
        value: Vec<u16>,
    },
    PointerButton {
        down: bool,
        button: u64,
        properties: PointerProperties,
    },
    PointerMove {
        x: i64,
        y: i64,
        origin: Origin,
        duration_ms: Option<u64>,
        properties: PointerProperties,
    },
    PointerCancel,
    Scroll {
        x: i64,
        y: i64,
        delta_x: i64,
        delta_y: i64,
        origin: Origin,
        duration_ms: Option<u64>,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub kind: SourceKind,
    pub actions: Vec<Action>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionsRequest {
    pub sources: Vec<Source>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ActionCommand {
    Perform(ActionsRequest),
    Release,
    Cancel,
}

fn single_code_point(value: &[u16]) -> bool {
    value.len() == 1
        || (value.len() == 2
            && (0xd800..=0xdbff).contains(&value[0])
            && (0xdc00..=0xdfff).contains(&value[1]))
}
impl PointerProperties {
    fn valid(&self) -> bool {
        let bounded = |value: Option<f64>, min: f64, max: f64| {
            value.is_none_or(|value| value.is_finite() && value >= min && value <= max)
        };
        bounded(self.width, 0., MAX_SAFE as f64)
            && bounded(self.height, 0., MAX_SAFE as f64)
            && self.width.is_none_or(|value| value.fract() == 0.)
            && self.height.is_none_or(|value| value.fract() == 0.)
            && bounded(self.pressure, 0., 1.)
            && bounded(self.tangential_pressure, -1., 1.)
            && self.tilt_x.is_none_or(|value| (-90..=90).contains(&value))
            && self.tilt_y.is_none_or(|value| (-90..=90).contains(&value))
            && self.twist.is_none_or(|value| value <= 359)
            && bounded(self.altitude_angle, 0., std::f64::consts::FRAC_PI_2)
            && bounded(self.azimuth_angle, 0., std::f64::consts::TAU)
    }
}

fn object(value: &ScriptValue) -> Result<(), ScriptError> {
    if matches!(value, ScriptValue::Object(_)) {
        Ok(())
    } else {
        Err(ScriptError::invalid("action entry must be an object"))
    }
}
fn string(value: &ScriptValue, key: &str) -> Result<String, ScriptError> {
    value
        .get(key)
        .ok_or_else(|| ScriptError::invalid(format!("missing {key}")))?
        .as_string()
}
fn integer(value: &ScriptValue, key: &str) -> Result<i64, ScriptError> {
    let Some(ScriptValue::Number(number)) = value.get(key) else {
        return Err(ScriptError::invalid(format!("{key} must be an integer")));
    };
    // JSON has one number type: 1, 1.0 and 1e0 all denote an integer.
    number
        .as_i64()
        .or_else(|| {
            number
                .as_f64()
                .filter(|value| {
                    value.is_finite() && value.fract() == 0. && value.abs() <= MAX_SAFE as f64
                })
                .map(|value| value as i64)
        })
        .filter(|number| number.unsigned_abs() <= MAX_SAFE)
        .ok_or_else(|| ScriptError::invalid(format!("{key} must be a safe integer")))
}
fn duration(value: &ScriptValue) -> Result<Option<u64>, ScriptError> {
    value
        .get("duration")
        .map(|_| {
            integer(value, "duration").and_then(|value| {
                u64::try_from(value)
                    .map_err(|_| ScriptError::invalid("duration must be nonnegative"))
            })
        })
        .transpose()
}
fn number(value: &ScriptValue, key: &str, min: f64, max: f64) -> Result<Option<f64>, ScriptError> {
    let Some(value) = value.get(key) else {
        return Ok(None);
    };
    let ScriptValue::Number(value) = value else {
        return Err(ScriptError::invalid(format!("{key} must be a number")));
    };
    value
        .as_f64()
        .filter(|value| value.is_finite() && *value >= min && *value <= max)
        .map(Some)
        .ok_or_else(|| ScriptError::invalid(format!("{key} outside permitted range")))
}
fn properties(value: &ScriptValue) -> Result<PointerProperties, ScriptError> {
    let bounded_integer = |key, min, max| {
        value
            .get(key)
            .map(|_| {
                integer(value, key).and_then(|value| {
                    if value >= min && value <= max {
                        Ok(value)
                    } else {
                        Err(ScriptError::invalid(format!(
                            "{key} outside permitted range"
                        )))
                    }
                })
            })
            .transpose()
    };
    Ok(PointerProperties {
        width: bounded_integer("width", 0, MAX_SAFE as i64)?.map(|value| value as f64),
        height: bounded_integer("height", 0, MAX_SAFE as i64)?.map(|value| value as f64),
        pressure: number(value, "pressure", 0., 1.)?,
        tangential_pressure: number(value, "tangentialPressure", -1., 1.)?,
        tilt_x: bounded_integer("tiltX", -90, 90)?,
        tilt_y: bounded_integer("tiltY", -90, 90)?,
        twist: bounded_integer("twist", 0, 359)?.map(|value| value as u64),
        altitude_angle: number(value, "altitudeAngle", 0., std::f64::consts::FRAC_PI_2)?,
        azimuth_angle: number(value, "azimuthAngle", 0., std::f64::consts::TAU)?,
    })
}
fn origin(value: &ScriptValue, wheel: bool) -> Result<Origin, ScriptError> {
    match value.get("origin") {
        None => Ok(Origin::Viewport),
        Some(ScriptValue::String(value)) if value.iter().copied().eq("viewport".encode_utf16()) => {
            Ok(Origin::Viewport)
        }
        Some(ScriptValue::String(value))
            if !wheel && value.iter().copied().eq("pointer".encode_utf16()) =>
        {
            Ok(Origin::Pointer)
        }
        Some(value @ ScriptValue::Object(_)) => {
            let id = string(value, ELEMENT_KEY)?;
            if id.is_empty() {
                return Err(ScriptError::invalid("empty action element reference"));
            }
            Ok(Origin::Element(id))
        }
        _ => Err(ScriptError::invalid("invalid action origin")),
    }
}
impl ActionsRequest {
    /// Revalidate native/IPC requests too: serde decoding alone does not enforce
    /// source/action compatibility or numeric bounds. Call before reserving any
    /// persistent input identities or dispatching a tick.
    pub fn validate(&self) -> Result<(), ScriptError> {
        if self.sources.len() > MAX_SOURCES {
            return Err(ScriptError::invalid("input source limit"));
        }
        let mut ids = std::collections::BTreeSet::new();
        let safe = |value: i64| value.unsigned_abs() <= MAX_SAFE;
        let origin_valid = |origin: &Origin, wheel: bool| match origin {
            Origin::Viewport => true,
            Origin::Pointer => !wheel,
            Origin::Element(id) => !id.is_empty(),
        };
        for source in &self.sources {
            if !ids.insert(&source.id) {
                return Err(ScriptError::invalid("duplicate input source id"));
            }
            if source.actions.len() > MAX_ACTIONS_PER_SOURCE {
                return Err(ScriptError::invalid("action count limit"));
            }
            for action in &source.actions {
                let valid = match action {
                    Action::OsText {
                        token,
                        sequence,
                        operation,
                    } => {
                        source.kind == SourceKind::Key
                            && *token > 0
                            && *sequence > 0
                            && operation.validate()
                    }
                    Action::OsModifiers(_) => source.kind == SourceKind::Key,
                    Action::OsKey(key) => source.kind == SourceKind::Key && key.valid(),
                    Action::OsPointerMove {
                        x,
                        y,
                        screen_x,
                        screen_y,
                    } => {
                        matches!(source.kind, SourceKind::Pointer(_))
                            && [*x, *y, *screen_x, *screen_y]
                                .iter()
                                .all(|value| value.is_finite() && value.abs() <= 16_777_216.)
                    }
                    Action::Pause { duration_ms } => *duration_ms <= MAX_SAFE,
                    Action::Key { value, .. } => {
                        source.kind == SourceKind::Key && single_code_point(value)
                    }
                    Action::PointerButton {
                        button, properties, ..
                    } => {
                        matches!(source.kind, SourceKind::Pointer(_))
                            && *button <= MAX_SAFE
                            && properties.valid()
                    }
                    Action::PointerMove {
                        x,
                        y,
                        origin,
                        duration_ms,
                        properties,
                    } => {
                        matches!(source.kind, SourceKind::Pointer(_))
                            && safe(*x)
                            && safe(*y)
                            && origin_valid(origin, false)
                            && duration_ms.is_none_or(|value| value <= MAX_SAFE)
                            && properties.valid()
                    }
                    Action::PointerCancel => matches!(source.kind, SourceKind::Pointer(_)),
                    Action::Scroll {
                        x,
                        y,
                        delta_x,
                        delta_y,
                        origin,
                        duration_ms,
                    } => {
                        source.kind == SourceKind::Wheel
                            && safe(*x)
                            && safe(*y)
                            && safe(*delta_x)
                            && safe(*delta_y)
                            && origin_valid(origin, true)
                            && duration_ms.is_none_or(|value| value <= MAX_SAFE)
                    }
                };
                if !valid {
                    return Err(ScriptError::invalid(
                        "invalid action for native input source",
                    ));
                }
            }
        }
        Ok(())
    }
    pub fn parse(value: &ScriptValue) -> Result<Self, ScriptError> {
        object(value)?;
        let Some(ScriptValue::Array(sources)) = value.get("actions") else {
            return Err(ScriptError::invalid("actions must be an array"));
        };
        if sources.len() > MAX_SOURCES {
            return Err(ScriptError::invalid("input source limit"));
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut result = Vec::new();
        for source in sources {
            object(source)?;
            let id = string(source, "id")?;
            if !ids.insert(id.clone()) {
                return Err(ScriptError::invalid("duplicate input source id"));
            }
            let kind = match string(source, "type")?.as_str() {
                "none" => SourceKind::None,
                "key" => SourceKind::Key,
                "wheel" => SourceKind::Wheel,
                "pointer" => {
                    let pointer = if let Some(parameters) = source.get("parameters") {
                        object(parameters)?;
                        parameters
                            .get("pointerType")
                            .map(ScriptValue::as_string)
                            .transpose()?
                            .unwrap_or_else(|| "mouse".into())
                    } else {
                        "mouse".into()
                    };
                    SourceKind::Pointer(match pointer.as_str() {
                        "mouse" => PointerKind::Mouse,
                        "pen" => PointerKind::Pen,
                        "touch" => PointerKind::Touch,
                        _ => return Err(ScriptError::invalid("unknown pointer type")),
                    })
                }
                _ => return Err(ScriptError::invalid("unknown input source type")),
            };
            let Some(ScriptValue::Array(actions)) = source.get("actions") else {
                return Err(ScriptError::invalid("source actions must be an array"));
            };
            if actions.len() > MAX_ACTIONS_PER_SOURCE {
                return Err(ScriptError::invalid("action count limit"));
            }
            let mut parsed = Vec::new();
            for action in actions {
                object(action)?;
                let action_type = string(action, "type")?;
                parsed.push(match (action_type.as_str(), &kind) {
                    ("pause", _) => Action::Pause {
                        duration_ms: duration(action)?.unwrap_or(0),
                    },
                    ("keyDown" | "keyUp", SourceKind::Key) => {
                        let Some(ScriptValue::String(value)) = action.get("value") else {
                            return Err(ScriptError::invalid("key value must be a string"));
                        };
                        if !single_code_point(value) {
                            return Err(ScriptError::invalid(
                                "key value must contain one code point",
                            ));
                        }
                        Action::Key {
                            down: action_type == "keyDown",
                            value: value.clone(),
                        }
                    }
                    ("pointerDown" | "pointerUp", SourceKind::Pointer(_)) => {
                        Action::PointerButton {
                            down: action_type == "pointerDown",
                            button: u64::try_from(integer(action, "button")?)
                                .map_err(|_| ScriptError::invalid("button must be nonnegative"))?,
                            properties: properties(action)?,
                        }
                    }
                    ("pointerMove", SourceKind::Pointer(_)) => Action::PointerMove {
                        x: integer(action, "x")?,
                        y: integer(action, "y")?,
                        origin: origin(action, false)?,
                        duration_ms: duration(action)?,
                        properties: properties(action)?,
                    },
                    ("pointerCancel", SourceKind::Pointer(_)) => Action::PointerCancel,
                    ("scroll", SourceKind::Wheel) => Action::Scroll {
                        x: integer(action, "x")?,
                        y: integer(action, "y")?,
                        delta_x: integer(action, "deltaX")?,
                        delta_y: integer(action, "deltaY")?,
                        origin: origin(action, true)?,
                        duration_ms: duration(action)?,
                    },
                    _ => {
                        return Err(ScriptError::invalid(
                            "action does not match its input source",
                        ));
                    }
                });
            }
            result.push(Source {
                id,
                kind,
                actions: parsed,
            });
        }
        let request = Self { sources: result };
        request.validate()?;
        Ok(request)
    }
    /// Concurrent sources share tick duration; their durations are not summed.
    pub fn tick_duration(&self, tick: usize) -> u64 {
        self.sources
            .iter()
            .filter_map(|source| source.actions.get(tick))
            .map(|action| match action {
                Action::Pause { duration_ms } => *duration_ms,
                Action::PointerMove { duration_ms, .. } | Action::Scroll { duration_ms, .. } => {
                    duration_ms.unwrap_or(0)
                }
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }
}

/// Session-owned source identities survive requests until Release Actions.
/// Admission is atomic: a malformed late source cannot reserve earlier IDs.
#[derive(Default)]
pub struct SourceRegistry {
    kinds: std::collections::BTreeMap<String, SourceKind>,
    pointer_ids: std::collections::BTreeMap<String, i32>,
    next_pointer: i32,
}
impl SourceRegistry {
    pub fn admit(&mut self, request: &ActionsRequest) -> Result<(), ScriptError> {
        request.validate()?;
        let mut proposed = self.kinds.clone();
        for source in &request.sources {
            if let Some(existing) = proposed.get(&source.id) {
                if existing != &source.kind {
                    return Err(ScriptError::invalid("input source id cannot change type"));
                }
            } else {
                proposed.insert(source.id.clone(), source.kind.clone());
            }
        }
        if proposed.len() > MAX_SOURCES {
            return Err(ScriptError::invalid("persistent input source limit"));
        }
        for source in &request.sources {
            if matches!(source.kind, SourceKind::Pointer(_))
                && !self.pointer_ids.contains_key(&source.id)
            {
                self.next_pointer += 1;
                self.pointer_ids
                    .insert(source.id.clone(), self.next_pointer);
            }
        }
        self.kinds = proposed;
        Ok(())
    }
    pub fn clear(&mut self) {
        self.kinds.clear();
        self.pointer_ids.clear();
        self.next_pointer = 0;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum HeldInput {
    Key(Vec<u16>),
    Button(u64),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAction {
    pub source: String,
    pub input: HeldInput,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub alt: bool,
    pub control: bool,
    pub meta: bool,
    pub shift: bool,
}

/// Resolve against one current native layout snapshot. This does not mutate
/// pointer position: only successful dispatch commits that transition.
pub fn resolve_coordinates(
    origin: &Origin,
    x: i64,
    y: i64,
    pointer: (f64, f64),
    viewport: (f64, f64),
    element: Option<(f64, f64, f64, f64)>,
) -> Result<(f64, f64), ScriptError> {
    let outside = || ScriptError {
        code: "move target out of bounds".into(),
        message: "action target is outside the layout viewport".into(),
    };
    if !viewport.0.is_finite()
        || !viewport.1.is_finite()
        || viewport.0 < 0.
        || viewport.1 < 0.
        || x.unsigned_abs() > MAX_SAFE
        || y.unsigned_abs() > MAX_SAFE
    {
        return Err(outside());
    }
    let base = match origin {
        Origin::Viewport => (0., 0.),
        Origin::Pointer => pointer,
        Origin::Element(_) => {
            let (left, top, width, height) = element.ok_or_else(|| ScriptError {
                code: "element not interactable".into(),
                message: "action element has no client rectangle".into(),
            })?;
            if ![left, top, width, height, left + width, top + height]
                .iter()
                .all(|value| value.is_finite())
            {
                return Err(outside());
            }
            let x0 = left.min(left + width).max(0.);
            let x1 = left.max(left + width).min(viewport.0);
            let y0 = top.min(top + height).max(0.);
            let y1 = top.max(top + height).min(viewport.1);
            (((x0 + x1) / 2.).floor(), ((y0 + y1) / 2.).floor())
        }
    };
    let result = (base.0 + x as f64, base.1 + y as f64);
    if !result.0.is_finite()
        || !result.1.is_finite()
        || result.0 < 0.
        || result.1 < 0.
        || result.0 > viewport.0
        || result.1 > viewport.1
    {
        return Err(outside());
    }
    Ok(result)
}

/// Owned by one authenticated automation session. A request's completion does
/// not clear held inputs. The dispatcher updates state just before event
/// delivery, after resolving its target, so callbacks observe the new state.
#[derive(Default)]
pub struct InputState {
    sources: SourceRegistry,
    held: std::collections::BTreeMap<String, std::collections::BTreeSet<HeldInput>>,
    positions: std::collections::BTreeMap<String, (f64, f64)>,
    screen_positions: std::collections::BTreeMap<String, (f64, f64)>,
    native_keys: std::collections::BTreeMap<(String, Vec<u16>), OsKey>,
    native_modifiers: std::collections::BTreeMap<String, OsKeyModifiers>,
    os_receipts: Vec<OsKeyReceipt>,
    os_grants: std::collections::BTreeMap<u64, (OsKeyReceipt, u64)>,
    releases: Vec<ReleaseAction>,
    pointer_lifetimes: std::collections::BTreeMap<String, PointerLifetime>,
    pointer_properties: std::collections::BTreeMap<String, ResolvedPointerProperties>,
}
impl InputState {
    pub fn admit(&mut self, request: &ActionsRequest) -> Result<(), ScriptError> {
        self.sources.admit(request)
    }
    pub fn pointer_id(&self, source: &str) -> Result<i32, ScriptError> {
        self.sources
            .pointer_ids
            .get(source)
            .copied()
            .ok_or_else(|| ScriptError::invalid("unknown pointer source"))
    }
    pub fn source_kind(&self, source: &str) -> Result<&SourceKind, ScriptError> {
        self.sources
            .kinds
            .get(source)
            .ok_or_else(|| ScriptError::invalid("unknown input source"))
    }
    /// WebDriver's global key state is the union of all keyboard sources.
    /// Releasing one physical modifier does not release its opposite-side key
    /// or the same modifier held by another source.
    pub fn modifiers(&self) -> Modifiers {
        let mut result = Modifiers::default();
        for modifiers in self.native_modifiers.values() {
            result.shift |= modifiers.shift;
            result.control |= modifiers.control;
            result.alt |= modifiers.alt;
            result.meta |= modifiers.meta;
        }
        for (source, held) in &self.held {
            if self.sources.kinds.get(source) != Some(&SourceKind::Key) {
                continue;
            }
            for input in held {
                let HeldInput::Key(value) = input else {
                    continue;
                };
                match value.as_slice() {
                    [0xe008] | [0xe050] => result.shift = true,
                    [0xe009] | [0xe051] => result.control = true,
                    [0xe00a] | [0xe052] => result.alt = true,
                    [0xe03d] | [0xe053] => result.meta = true,
                    _ => {}
                }
            }
        }
        result
    }
    /// DOM button bits differ from button indices for middle/right buttons.
    /// Unknown higher buttons stay owned for release, without overflowing the
    /// event's unsigned bit field.
    pub fn buttons(&self, source: &str) -> Result<u32, ScriptError> {
        self.position(source)?;
        Ok(self
            .held
            .get(source)
            .into_iter()
            .flatten()
            .fold(0, |bits, input| {
                let HeldInput::Button(button) = input else {
                    return bits;
                };
                let bit = match button {
                    0 => 1,
                    1 => 4,
                    2 => 2,
                    _ => u32::try_from(*button)
                        .ok()
                        .and_then(|shift| 1u32.checked_shl(shift))
                        .unwrap_or(0),
                };
                bits | bit
            }))
    }
    /// Prepare the native pointercancel target before calling this method. It
    /// clears only that pointer's pressed state and pending releases; keyboard
    /// modifiers, other pointers, identity and current position remain live.
    pub fn cancel_pointer(&mut self, source: &str) -> Result<(), ScriptError> {
        self.position(source)?;
        self.held.remove(source);
        if let Some(lifetime) = self.pointer_lifetimes.get_mut(source) {
            lifetime.active = false;
        }
        self.releases.retain(|action| action.source != source);
        Ok(())
    }
    fn check_input(&self, source: &str, input: &HeldInput) -> Result<(), ScriptError> {
        match input {
            HeldInput::Key(value) if !native_key_name(value) => {
                return Err(ScriptError::invalid("held key must contain one code point"));
            }
            HeldInput::Button(value) if *value > MAX_SAFE => {
                return Err(ScriptError::invalid("button exceeds safe integer range"));
            }
            _ => {}
        }
        let matches = matches!(
            (self.sources.kinds.get(source), input),
            (Some(SourceKind::Key), HeldInput::Key(_))
                | (Some(SourceKind::Pointer(_)), HeldInput::Button(_))
        );
        if matches {
            Ok(())
        } else {
            Err(ScriptError::invalid(
                "input does not belong to an admitted source",
            ))
        }
    }
    /// Returns true for an already-held key/button (keyboard auto-repeat must
    /// not append a second release). A rejected press changes no state.
    pub fn press(&mut self, source: &str, input: HeldInput) -> Result<bool, ScriptError> {
        self.check_input(source, &input)?;
        if self
            .held
            .get(source)
            .is_some_and(|held| held.contains(&input))
        {
            return Ok(true);
        }
        if self.releases.len() >= MAX_ACTIONS_PER_SOURCE {
            return Err(ScriptError::invalid("held input limit"));
        }
        if matches!(input, HeldInput::Button(_))
            && !self.held.get(source).is_some_and(|held| {
                held.iter()
                    .any(|input| matches!(input, HeldInput::Button(_)))
            })
        {
            self.begin_pointer_contact(source)?;
        }
        self.held
            .entry(source.into())
            .or_default()
            .insert(input.clone());
        self.releases.push(ReleaseAction {
            source: source.into(),
            input,
        });
        Ok(false)
    }
    pub fn is_pressed(&self, source: &str, input: &HeldInput) -> bool {
        self.held
            .get(source)
            .is_some_and(|held| held.contains(input))
    }
    /// A normal keyUp/pointerUp removes its pending cancellation action too.
    pub fn release(&mut self, source: &str, input: &HeldInput) -> Result<bool, ScriptError> {
        self.check_input(source, input)?;
        let changed = self
            .held
            .get_mut(source)
            .is_some_and(|held| held.remove(input));
        if changed {
            self.releases
                .retain(|action| action.source != source || &action.input != input);
        }
        if changed
            && matches!(input, HeldInput::Button(_))
            && !self.held.get(source).is_some_and(|held| {
                held.iter()
                    .any(|input| matches!(input, HeldInput::Button(_)))
            })
            && let Some(lifetime) = self.pointer_lifetimes.get_mut(source)
        {
            lifetime.active = false;
        }

        Ok(changed)
    }
    /// Peek rather than drain: failure to prepare a release leaves it owned for
    /// retry. Once the native event is ready, call release immediately before
    /// dispatch so listeners observe the released state.
    pub fn next_release(&self) -> Option<ReleaseAction> {
        self.releases.last().cloned()
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn pointer_sources(&self) -> Vec<String> {
        self.sources
            .kinds
            .iter()
            .filter(|(_, kind)| matches!(kind, SourceKind::Pointer(_)))
            .map(|(id, _)| id.clone())
            .collect()
    }
    pub fn position(&self, source: &str) -> Result<(f64, f64), ScriptError> {
        if !matches!(self.sources.kinds.get(source), Some(SourceKind::Pointer(_))) {
            return Err(ScriptError::invalid(
                "position requires an admitted pointer source",
            ));
        }
        Ok(self.positions.get(source).copied().unwrap_or((0., 0.)))
    }
    pub fn move_pointer(&mut self, source: &str, x: f64, y: f64) -> Result<(), ScriptError> {
        self.position(source)?;
        if !x.is_finite() || !y.is_finite() {
            return Err(ScriptError::invalid("pointer coordinates must be finite"));
        }
        self.positions.insert(source.into(), (x, y));
        self.screen_positions.remove(source);
        Ok(())
    }
    #[cfg(any(feature = "v8-host", test))]
    pub(crate) fn push_os_receipt(&mut self, receipt: OsKeyReceipt) -> Result<(), ScriptError> {
        if self.os_receipts.len() >= 128 {
            return Err(ScriptError::invalid(
                "native interpretation receipt budget exceeded",
            ));
        }
        if receipt.interpret {
            if receipt.target.is_none()
                || self.os_grants.len() >= 128
                || self.os_grants.contains_key(&receipt.token)
            {
                return Err(ScriptError::invalid(
                    "invalid or duplicate native interpretation token",
                ));
            }
            self.os_grants
                .retain(|_, (previous, _)| previous.source != receipt.source);
            self.os_grants.insert(receipt.token, (receipt.clone(), 1));
        }
        self.os_receipts.push(receipt);
        Ok(())
    }
    #[cfg(any(feature = "v8-host", test))]
    pub(crate) fn admit_os_text(
        &mut self,
        source: &str,
        token: u64,
        sequence: u64,
    ) -> Result<OsKeyReceipt, ScriptError> {
        let (grant, next) = self
            .os_grants
            .get_mut(&token)
            .ok_or_else(|| ScriptError::invalid("retired native interpretation token"))?;
        if grant.source != source || *next != sequence {
            return Err(ScriptError::invalid(
                "native interpretation source/sequence mismatch",
            ));
        }
        *next = next
            .checked_add(1)
            .ok_or_else(|| ScriptError::invalid("native interpretation sequence exhausted"))?;
        Ok(grant.clone())
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn record_os_text_state(
        &mut self,
        token: u64,
        state: &oryn_js::composition::StateSnapshot,
    ) {
        if let Some((grant, _)) = self.os_grants.get_mut(&token) {
            grant.state = Some(state.clone());
        }
    }
    #[cfg(any(feature = "v8-host", test))]
    pub(crate) fn finish_os_text(&mut self, token: u64, keep: bool) {
        if !keep {
            self.os_grants.remove(&token);
        }
    }
    #[cfg(any(feature = "v8-host", test))]
    pub(crate) fn take_os_receipts(&mut self) -> Vec<OsKeyReceipt> {
        std::mem::take(&mut self.os_receipts)
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn set_os_modifiers(&mut self, source: &str, modifiers: OsKeyModifiers) {
        self.native_modifiers.insert(source.to_owned(), modifiers);
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn native_key(&self, source: &str, value: &[u16]) -> Option<OsKey> {
        self.native_keys
            .get(&(source.to_owned(), value.to_vec()))
            .cloned()
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn remember_native_key(&mut self, source: &str, key: &OsKey) {
        self.native_modifiers
            .insert(source.to_owned(), key.modifiers);
        if key.down {
            self.native_keys
                .insert((source.to_owned(), key.value.clone()), key.clone());
        } else {
            self.native_keys
                .remove(&(source.to_owned(), key.value.clone()));
        }
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn clear_native_modifiers(&mut self) {
        self.native_modifiers.clear();
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn set_os_screen_position(&mut self, source: &str, x: f64, y: f64) {
        self.screen_positions.insert(source.into(), (x, y));
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn screen_position(&self, source: &str) -> Result<(f64, f64), ScriptError> {
        Ok(self
            .screen_positions
            .get(source)
            .copied()
            .unwrap_or(self.position(source)?))
    }
    pub fn finish_release(&mut self) -> Result<(), ScriptError> {
        if !self.releases.is_empty() {
            return Err(ScriptError::invalid(
                "native input releases are still pending",
            ));
        }
        self.sources.clear();
        self.positions.clear();
        self.screen_positions.clear();
        self.native_keys.clear();
        self.native_modifiers.clear();
        self.os_grants.clear();
        self.os_receipts.clear();
        self.held.clear();
        self.pointer_lifetimes.clear();
        self.pointer_properties.clear();
        Ok(())
    }
    /// A retired document receives no synthetic releases. Its authority is
    /// already gone; discard state only at the owning lifecycle boundary.
    pub fn retire(&mut self) {
        self.releases.clear();
        self.held.clear();
        self.positions.clear();
        self.screen_positions.clear();
        self.native_keys.clear();
        self.native_modifiers.clear();
        self.os_grants.clear();
        self.os_receipts.clear();
        self.sources.clear();
        self.pointer_lifetimes.clear();
        self.pointer_properties.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(text: &str) -> Result<ActionsRequest, ScriptError> {
        ActionsRequest::parse(&ScriptValue::from_json_text(text)?)
    }
    #[test]
    fn pointer_ids_survive_later_lexically_earlier_source_admission() {
        let mut state = InputState::default();
        state
            .admit(&parse(r#"{"actions":[{"id":"z","type":"pointer","actions":[]}]}"#).unwrap())
            .unwrap();
        let first = state.pointer_id("z").unwrap();
        state
            .admit(&parse(r#"{"actions":[{"id":"a","type":"pointer","actions":[]}]}"#).unwrap())
            .unwrap();
        assert_eq!(state.pointer_id("z").unwrap(), first);
        assert_ne!(state.pointer_id("a").unwrap(), first);
        assert!(state.pointer_id("missing").is_err());
    }
    #[test]
    fn action_coordinates_use_clipped_first_fragment_center_and_offsets() {
        let origin = Origin::Element("capability".into());
        assert_eq!(
            resolve_coordinates(
                &origin,
                5,
                -2,
                (900., 900.),
                (100., 80.),
                Some((-20., 10., 80., 31.))
            )
            .unwrap(),
            (35., 23.)
        );
        assert_eq!(
            resolve_coordinates(
                &origin,
                0,
                0,
                (0., 0.),
                (100., 80.),
                Some((60., 41., -80., -31.))
            )
            .unwrap(),
            (30., 25.)
        );
        assert_eq!(
            resolve_coordinates(&Origin::Pointer, 3, -4, (20.5, 30.5), (100., 80.), None).unwrap(),
            (23.5, 26.5)
        );
        assert_eq!(
            resolve_coordinates(&Origin::Viewport, 100, 80, (50., 50.), (100., 80.), None).unwrap(),
            (100., 80.)
        );
        assert_eq!(
            resolve_coordinates(&origin, 0, 0, (0., 0.), (100., 80.), None)
                .unwrap_err()
                .code,
            "element not interactable"
        );
    }

    #[test]
    fn coordinate_rejection_does_not_mutate_persistent_pointer_state() {
        let request = parse(r#"{"actions":[{"id":"p","type":"pointer","actions":[]}]}"#).unwrap();
        let mut state = InputState::default();
        state.admit(&request).unwrap();
        state.move_pointer("p", 20., 30.).unwrap();
        for (origin, x, y, rect) in [
            (Origin::Viewport, -1, 0, None),
            (Origin::Pointer, 81, 0, None),
            (
                Origin::Element("e".into()),
                0,
                0,
                Some((500., 0., 20., 20.)),
            ),
            (
                Origin::Element("e".into()),
                0,
                0,
                Some((f64::NAN, 0., 20., 20.)),
            ),
        ] {
            assert_eq!(
                resolve_coordinates(
                    &origin,
                    x,
                    y,
                    state.position("p").unwrap(),
                    (100., 80.),
                    rect
                )
                .unwrap_err()
                .code,
                "move target out of bounds"
            );
            assert_eq!(state.position("p").unwrap(), (20., 30.));
        }
    }

    #[test]
    fn global_modifiers_preserve_other_keys_and_keyboard_sources() {
        let request=parse(r#"{"actions":[{"id":"a","type":"key","actions":[]},{"id":"b","type":"key","actions":[]}]}"#).unwrap();
        let mut state = InputState::default();
        state.admit(&request).unwrap();
        for code in [
            0xe008, 0xe050, 0xe009, 0xe051, 0xe00a, 0xe052, 0xe03d, 0xe053,
        ] {
            state.press("a", HeldInput::Key(vec![code])).unwrap();
        }
        let all = Modifiers {
            alt: true,
            control: true,
            meta: true,
            shift: true,
        };
        assert_eq!(state.modifiers(), all);
        for code in [0xe008, 0xe009, 0xe00a, 0xe03d] {
            state.release("a", &HeldInput::Key(vec![code])).unwrap();
        }
        assert_eq!(state.modifiers(), all, "right-side modifiers remain held");
        state.press("b", HeldInput::Key(vec![0xe008])).unwrap();
        for code in [0xe050, 0xe051, 0xe052, 0xe053] {
            state.release("a", &HeldInput::Key(vec![code])).unwrap();
        }
        assert_eq!(
            state.modifiers(),
            Modifiers {
                shift: true,
                ..Default::default()
            }
        );
        state.release("b", &HeldInput::Key(vec![0xe008])).unwrap();
        assert_eq!(state.modifiers(), Modifiers::default());
    }

    #[test]
    fn pointer_cancel_preserves_other_sources_and_position() {
        let request=parse(r#"{"actions":[{"id":"k","type":"key","actions":[]},{"id":"p","type":"pointer","actions":[]},{"id":"q","type":"pointer","parameters":{"pointerType":"pen"},"actions":[]}]}"#).unwrap();
        let mut state = InputState::default();
        state.admit(&request).unwrap();
        state.press("k", HeldInput::Key(vec![0xe009])).unwrap();
        state.move_pointer("p", 18., 29.).unwrap();
        for button in [0, 1, 2, 3, 4, MAX_SAFE] {
            state.press("p", HeldInput::Button(button)).unwrap();
        }
        assert_eq!(state.buttons("p").unwrap(), 31);
        state.press("q", HeldInput::Button(2)).unwrap();
        assert_eq!(state.buttons("q").unwrap(), 2);
        state.cancel_pointer("p").unwrap();
        assert_eq!(state.buttons("p").unwrap(), 0);
        assert_eq!(state.position("p").unwrap(), (18., 29.));
        assert_eq!(state.buttons("q").unwrap(), 2);
        assert!(state.modifiers().control);
        assert!(state.cancel_pointer("k").is_err());
        assert_eq!(state.next_release().unwrap().source, "q");
        state.release("q", &HeldInput::Button(2)).unwrap();
        assert_eq!(state.next_release().unwrap().source, "k");
        state.release("k", &HeldInput::Key(vec![0xe009])).unwrap();
        assert!(state.next_release().is_none());
        assert!(matches!(
            state.source_kind("p").unwrap(),
            SourceKind::Pointer(PointerKind::Mouse)
        ));
        state.finish_release().unwrap();
        assert!(state.source_kind("p").is_err());
    }

    #[test]
    fn integer_valued_json_numbers_accept_decimal_and_exponent_spellings() {
        let request=parse(r#"{"actions":[{"id":"p","type":"pointer","actions":[{"type":"pointerMove","x":1.0,"y":-2e0,"duration":3e0},{"type":"pointerDown","button":0.0,"tiltX":90.0}]}]}"#).unwrap();
        assert!(matches!(
            request.sources[0].actions[0],
            Action::PointerMove {
                x: 1,
                y: -2,
                duration_ms: Some(3),
                ..
            }
        ));
        for invalid in ["1.5", "9007199254740992", "-9007199254740992", "1e30"] {
            let json = format!(
                r#"{{"actions":[{{"id":"p","type":"pointer","actions":[{{"type":"pointerMove","x":{invalid},"y":0}}]}}]}}"#
            );
            assert!(parse(&json).is_err(), "{invalid}");
        }
    }

    #[test]
    fn native_request_validation_precedes_atomic_source_admission() {
        let mut state = InputState::default();
        let mut request = ActionsRequest {
            sources: vec![
                Source {
                    id: "first".into(),
                    kind: SourceKind::Key,
                    actions: vec![],
                },
                Source {
                    id: "late".into(),
                    kind: SourceKind::Wheel,
                    actions: vec![Action::PointerCancel],
                },
            ],
        };
        assert!(state.admit(&request).is_err());
        // An invalid late action cannot reserve the earlier source's type.
        request.sources = vec![Source {
            id: "first".into(),
            kind: SourceKind::Pointer(PointerKind::Mouse),
            actions: vec![],
        }];
        state.admit(&request).unwrap();
        assert_eq!(state.position("first").unwrap(), (0., 0.));
        request.sources.push(request.sources[0].clone());
        assert!(
            state.admit(&request).is_err(),
            "duplicate typed source IDs are invalid too"
        );
        assert_eq!(state.position("first").unwrap(), (0., 0.));
    }

    #[test]
    fn typed_actions_reject_invalid_wire_bounds_and_nonfinite_properties() {
        let mut request = ActionsRequest {
            sources: vec![Source {
                id: "p".into(),
                kind: SourceKind::Pointer(PointerKind::Pen),
                actions: vec![],
            }],
        };
        for action in [
            Action::Pause {
                duration_ms: u64::MAX,
            },
            Action::PointerButton {
                down: true,
                button: u64::MAX,
                properties: Default::default(),
            },
            Action::PointerMove {
                x: i64::MIN,
                y: 0,
                origin: Origin::Viewport,
                duration_ms: None,
                properties: Default::default(),
            },
            Action::PointerMove {
                x: 0,
                y: 0,
                origin: Origin::Element(String::new()),
                duration_ms: None,
                properties: Default::default(),
            },
            Action::PointerButton {
                down: true,
                button: 0,
                properties: PointerProperties {
                    pressure: Some(f64::NAN),
                    ..Default::default()
                },
            },
            Action::PointerButton {
                down: true,
                button: 0,
                properties: PointerProperties {
                    width: Some(f64::INFINITY),
                    ..Default::default()
                },
            },
            Action::PointerButton {
                down: true,
                button: 0,
                properties: PointerProperties {
                    twist: Some(360),
                    ..Default::default()
                },
            },
        ] {
            request.sources[0].actions = vec![action];
            assert!(request.validate().is_err());
        }
        request.sources[0].kind = SourceKind::Wheel;
        request.sources[0].actions = vec![Action::Scroll {
            x: 0,
            y: 0,
            delta_x: 0,
            delta_y: 1,
            origin: Origin::Pointer,
            duration_ms: None,
        }];
        assert!(request.validate().is_err());
    }

    #[test]
    fn held_inputs_survive_requests_and_release_in_reverse_press_order() {
        let request=parse(r#"{"actions":[{"id":"k","type":"key","actions":[]},{"id":"p","type":"pointer","actions":[]}]}"#).unwrap();
        let mut state = InputState::default();
        state.admit(&request).unwrap();
        let shift = HeldInput::Key(vec![0xe008]);
        let mouse = HeldInput::Button(0);
        assert!(!state.press("k", shift.clone()).unwrap());
        assert!(state.press("k", shift.clone()).unwrap());
        state.move_pointer("p", 12.5, 40.).unwrap();
        state.press("p", mouse.clone()).unwrap();
        state.admit(&request).unwrap();
        assert_eq!(state.position("p").unwrap(), (12.5, 40.));
        let release = ReleaseAction {
            source: "p".into(),
            input: mouse.clone(),
        };
        assert_eq!(state.next_release(), Some(release.clone()));
        assert!(state.finish_release().is_err());
        assert_eq!(
            state.next_release(),
            Some(release),
            "failed/undelivered release remains owned"
        );
        state.release("p", &mouse).unwrap();
        assert!(state.is_pressed("k", &shift));
        assert_eq!(
            state.next_release(),
            Some(ReleaseAction {
                source: "k".into(),
                input: shift.clone()
            })
        );
        state.release("k", &shift).unwrap();
        assert_eq!(state.next_release(), None);
        state.finish_release().unwrap();
        assert!(state.position("p").is_err());
    }
    #[test]
    fn invalid_input_transition_does_not_mutate_owned_state() {
        let mut state = InputState::default();
        state
            .admit(&parse(r#"{"actions":[{"id":"k","type":"key","actions":[]}]}"#).unwrap())
            .unwrap();
        assert!(state.press("k", HeldInput::Button(0)).is_err());
        assert_eq!(state.next_release(), None);
        state.press("k", HeldInput::Key(vec![97])).unwrap();
        assert!(state.move_pointer("k", 1., 2.).is_err());
        assert!(state.next_release().is_some());
        state.retire();
        assert_eq!(state.next_release(), None);
        assert!(state.sources.kinds.is_empty());
    }
    #[test]
    fn ticks_use_maximum_duration_and_preserve_unicode_key_values() {
        let request=parse(r#"{"actions":[
            {"id":"key","type":"key","actions":[{"type":"keyDown","value":"\ud83d\ude00"},{"type":"pause","duration":20}]},
            {"id":"wheel","type":"wheel","actions":[{"type":"scroll","x":1,"y":2,"deltaX":0,"deltaY":-12,"duration":30},{"type":"pause","duration":10}]}
        ]}"#).unwrap();
        assert_eq!(request.tick_duration(0), 30);
        assert_eq!(request.tick_duration(1), 20);
        assert_eq!(request.tick_duration(2), 0);
        assert_eq!(
            request.sources[0].actions[0],
            Action::Key {
                down: true,
                value: vec![0xd83d, 0xde00]
            }
        );
    }
    #[test]
    fn invalid_late_actions_and_incompatible_origins_are_rejected() {
        for text in [
            r#"{"actions":[{"id":"m","type":"pointer","actions":[{"type":"pointerDown","button":0},{"type":"keyDown","value":"x"}]}]}"#,
            r#"{"actions":[{"id":"w","type":"wheel","actions":[{"type":"scroll","origin":"pointer","x":0,"y":0,"deltaX":0,"deltaY":1}]}]}"#,
            r#"{"actions":[{"id":"k","type":"key","actions":[{"type":"keyDown","value":"xy"}]}]}"#,
            r#"{"actions":[{"id":"m","type":"pointer","actions":[{"type":"pointerMove","x":0,"y":0,"pressure":2}]}]}"#,
            r#"{"actions":[{"id":"m","type":"pointer","actions":[{"type":"pause","duration":-1}]}]}"#,
            r#"{"actions":[{"id":"same","type":"key","actions":[]},{"id":"same","type":"none","actions":[]}]}"#,
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }
    #[test]
    fn source_admission_is_atomic_across_request_boundaries() {
        let mut registry = SourceRegistry::default();
        registry
            .admit(&parse(r#"{"actions":[{"id":"stable","type":"key","actions":[]}]}"#).unwrap())
            .unwrap();
        let invalid=parse(r#"{"actions":[{"id":"new","type":"wheel","actions":[]},{"id":"stable","type":"pointer","actions":[]}]}"#).unwrap();
        assert!(registry.admit(&invalid).is_err());
        assert_eq!(registry.kinds.len(), 1);
        assert!(!registry.kinds.contains_key("new"));
        registry.clear();
        registry.admit(&invalid).unwrap();
        assert_eq!(registry.kinds.len(), 2);
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PointerLifetime {
    active: bool,
    primary: bool,
    in_range: bool,
}
/// Actual normalized event snapshot, retained only for synthesized releases and
/// cancellation. Omitted properties on a later explicit action get fresh defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedPointerProperties {
    pub width: f64,
    pub height: f64,
    pub pressure: f64,
    pub tangential_pressure: f64,
    pub tilt_x: i32,
    pub tilt_y: i32,
    pub twist: u32,
    pub altitude_angle: f64,
    pub azimuth_angle: f64,
}
impl Default for ResolvedPointerProperties {
    fn default() -> Self {
        PointerProperties::default()
            .resolve(0)
            .expect("constant pointer defaults")
    }
}
impl PointerProperties {
    pub fn resolve(&self, buttons: u32) -> Result<ResolvedPointerProperties, ScriptError> {
        if !self.valid() {
            return Err(ScriptError::invalid("invalid native pointer properties"));
        }
        let has_angles = self.altitude_angle.is_some() || self.azimuth_angle.is_some();
        let (mut tilt_x, mut tilt_y) = (
            self.tilt_x.unwrap_or(0) as i32,
            self.tilt_y.unwrap_or(0) as i32,
        );
        let (altitude_angle, azimuth_angle) = if has_angles {
            (
                self.altitude_angle.unwrap_or(std::f64::consts::FRAC_PI_2),
                self.azimuth_angle.unwrap_or(0.),
            )
        } else {
            angles_from_tilt(tilt_x, tilt_y)
        };
        if has_angles {
            let (derived_x, derived_y) = tilt_from_angles(altitude_angle, azimuth_angle);
            if self.tilt_x.is_none() {
                tilt_x = derived_x;
            }
            if self.tilt_y.is_none() {
                tilt_y = derived_y;
            }
        }
        Ok(ResolvedPointerProperties {
            width: self.width.unwrap_or(1.),
            height: self.height.unwrap_or(1.),
            pressure: if buttons == 0 {
                0.
            } else {
                self.pressure.unwrap_or(0.5)
            },
            tangential_pressure: self.tangential_pressure.unwrap_or(0.),
            tilt_x,
            tilt_y,
            twist: self.twist.unwrap_or(0) as u32,
            altitude_angle,
            azimuth_angle,
        })
    }
}
fn tilt_from_angles(altitude: f64, azimuth: f64) -> (i32, i32) {
    let azimuth = azimuth.rem_euclid(std::f64::consts::TAU);
    if altitude == std::f64::consts::FRAC_PI_2 {
        return (0, 0);
    }
    if altitude == 0. {
        let x = if azimuth == std::f64::consts::FRAC_PI_2
            || azimuth == 3. * std::f64::consts::FRAC_PI_2
        {
            0
        } else if azimuth < std::f64::consts::FRAC_PI_2
            || azimuth > 3. * std::f64::consts::FRAC_PI_2
        {
            90
        } else {
            -90
        };
        let y = if azimuth == 0. || azimuth == std::f64::consts::PI {
            0
        } else if azimuth < std::f64::consts::PI {
            90
        } else {
            -90
        };
        return (x, y);
    }
    (
        (azimuth.cos() / altitude.tan()).atan().to_degrees().round() as i32,
        (azimuth.sin() / altitude.tan()).atan().to_degrees().round() as i32,
    )
}
fn angles_from_tilt(x: i32, y: i32) -> (f64, f64) {
    if x == 0 && y == 0 {
        return (std::f64::consts::FRAC_PI_2, 0.);
    }
    let tx = (x as f64).to_radians().tan();
    let ty = (y as f64).to_radians().tan();
    let altitude = if x.abs() == 90 || y.abs() == 90 {
        0.
    } else {
        (1. / tx.hypot(ty)).atan()
    };
    (altitude, ty.atan2(tx).rem_euclid(std::f64::consts::TAU))
}
impl InputState {
    fn begin_pointer_contact(&mut self, source: &str) -> Result<(), ScriptError> {
        let kind = self.source_kind(source)?.clone();
        let primary = match kind {
            SourceKind::Pointer(PointerKind::Mouse) => true,
            SourceKind::Pointer(PointerKind::Pen) => {
                self.observe_pointer_range(source)?;
                self.is_primary_pointer(source)?
            }
            SourceKind::Pointer(PointerKind::Touch) => {
                !self.pointer_lifetimes.iter().any(|(id, lifetime)| {
                    lifetime.active
                        && self.sources.kinds.get(id)
                            == Some(&SourceKind::Pointer(PointerKind::Touch))
                })
            }
            _ => return Err(ScriptError::invalid("contact requires pointer source")),
        };
        self.pointer_lifetimes.insert(
            source.into(),
            PointerLifetime {
                active: true,
                primary,
                in_range: true,
            },
        );
        Ok(())
    }
    pub fn is_primary_pointer(&self, source: &str) -> Result<bool, ScriptError> {
        Ok(match self.source_kind(source)? {
            SourceKind::Pointer(PointerKind::Mouse) => true,
            SourceKind::Pointer(PointerKind::Pen) => self
                .pointer_lifetimes
                .get(source)
                .is_some_and(|lifetime| lifetime.primary),
            SourceKind::Pointer(PointerKind::Touch) => self
                .pointer_lifetimes
                .get(source)
                .is_some_and(|lifetime| lifetime.primary),
            _ => return Err(ScriptError::invalid("primary flag requires pointer source")),
        })
    }
    pub fn observe_pointer_range(&mut self, source: &str) -> Result<(), ScriptError> {
        if self.source_kind(source)? != &SourceKind::Pointer(PointerKind::Pen) {
            return Ok(());
        }
        if self
            .pointer_lifetimes
            .get(source)
            .is_some_and(|lifetime| lifetime.in_range)
        {
            return Ok(());
        }
        let primary = !self.pointer_lifetimes.iter().any(|(id, lifetime)| {
            lifetime.in_range
                && self.sources.kinds.get(id) == Some(&SourceKind::Pointer(PointerKind::Pen))
        });
        self.pointer_lifetimes.insert(
            source.into(),
            PointerLifetime {
                in_range: true,
                primary,
                active: false,
            },
        );
        Ok(())
    }
    pub fn pointer_contact_active(&self, source: &str) -> Result<bool, ScriptError> {
        self.position(source)?;
        Ok(self
            .pointer_lifetimes
            .get(source)
            .is_some_and(|lifetime| lifetime.active))
    }
    pub fn remember_pointer_properties(
        &mut self,
        source: &str,
        properties: ResolvedPointerProperties,
    ) -> Result<(), ScriptError> {
        self.position(source)?;
        self.pointer_properties.insert(source.into(), properties);
        Ok(())
    }
    pub fn last_pointer_properties(
        &self,
        source: &str,
    ) -> Result<ResolvedPointerProperties, ScriptError> {
        self.position(source)?;
        let mut properties = self
            .pointer_properties
            .get(source)
            .copied()
            .unwrap_or_default();
        if self.buttons(source)? == 0 {
            properties.pressure = 0.;
        }
        Ok(properties)
    }
}

#[cfg(test)]
mod pointer_lifetime_tests {
    use super::*;
    fn request() -> ActionsRequest {
        ActionsRequest::parse(&ScriptValue::from_json_text(r#"{"actions":[{"id":"a","type":"pointer","parameters":{"pointerType":"touch"},"actions":[]},{"id":"b","type":"pointer","parameters":{"pointerType":"touch"},"actions":[]},{"id":"c","type":"pointer","parameters":{"pointerType":"touch"},"actions":[]},{"id":"pen1","type":"pointer","parameters":{"pointerType":"pen"},"actions":[]},{"id":"pen2","type":"pointer","parameters":{"pointerType":"pen"},"actions":[]},{"id":"mouse","type":"pointer","actions":[]}]}"#).unwrap()).unwrap()
    }
    #[test]
    fn touch_primary_is_not_promoted_or_reacquired_during_a_live_cohort() {
        let mut state = InputState::default();
        state.admit(&request()).unwrap();
        let button = HeldInput::Button(0);
        state.press("a", button.clone()).unwrap();
        state.press("b", button.clone()).unwrap();
        assert!(state.is_primary_pointer("a").unwrap());
        assert!(!state.is_primary_pointer("b").unwrap());
        assert!(state.is_primary_pointer("mouse").unwrap());
        state.release("a", &button).unwrap();
        assert!(
            state.is_primary_pointer("a").unwrap(),
            "final up retains its primary identity"
        );
        assert!(!state.is_primary_pointer("b").unwrap());
        state.press("a", button.clone()).unwrap();
        assert!(
            !state.is_primary_pointer("a").unwrap(),
            "new contact cannot reuse ended primary lifetime"
        );
        state.cancel_pointer("a").unwrap();
        state.press("c", button.clone()).unwrap();
        assert!(!state.is_primary_pointer("c").unwrap());
        state.release("b", &button).unwrap();
        state.release("c", &button).unwrap();
        state.press("c", button).unwrap();
        assert!(
            state.is_primary_pointer("c").unwrap(),
            "new empty-cohort contact is primary"
        );
    }
    #[test]
    fn pen_primary_follows_first_observed_range_and_survives_button_release() {
        let mut state = InputState::default();
        state.admit(&request()).unwrap();
        state.observe_pointer_range("pen2").unwrap();
        state.observe_pointer_range("pen1").unwrap();
        assert!(state.is_primary_pointer("pen2").unwrap());
        assert!(!state.is_primary_pointer("pen1").unwrap());
        state.press("pen2", HeldInput::Button(0)).unwrap();
        state.release("pen2", &HeldInput::Button(0)).unwrap();
        assert!(state.is_primary_pointer("pen2").unwrap());
        state.finish_release().unwrap();
        state.admit(&request()).unwrap();
        state.observe_pointer_range("pen1").unwrap();
        assert!(state.is_primary_pointer("pen1").unwrap());
    }
    #[test]
    fn orientation_conversion_matches_pinned_pen_and_touch_examples() {
        for (altitude, azimuth, x, y) in [
            (0.3, 0.2419, 72, 38),
            (1., 2., -15, 30),
            (0.5, 1.5, 7, 61),
            (1.2, 6., 20, -6),
            (0.5, 1.8, -23, 61),
        ] {
            let properties = PointerProperties {
                altitude_angle: Some(altitude),
                azimuth_angle: Some(azimuth),
                ..Default::default()
            }
            .resolve(1)
            .unwrap();
            assert_eq!((properties.tilt_x, properties.tilt_y), (x, y));
            assert_eq!(properties.altitude_angle, altitude);
            assert_eq!(properties.azimuth_angle, azimuth);
        }
        for (x, y) in [(0, 0), (21, -8), (-19, 62), (90, 0), (0, -90), (-90, 90)] {
            let properties = PointerProperties {
                tilt_x: Some(x),
                tilt_y: Some(y),
                ..Default::default()
            }
            .resolve(1)
            .unwrap();
            assert_eq!((properties.tilt_x, properties.tilt_y), (x as i32, y as i32));
            assert!(properties.altitude_angle.is_finite() && properties.azimuth_angle.is_finite());
            let (rx, ry) = tilt_from_angles(properties.altitude_angle, properties.azimuth_angle);
            assert_eq!((rx, ry), (x as i32, y as i32));
        }
    }
    #[test]
    fn explicit_action_defaults_differ_from_synthetic_release_snapshot() {
        let mut state = InputState::default();
        state.admit(&request()).unwrap();
        state.press("a", HeldInput::Button(0)).unwrap();
        let custom = PointerProperties {
            width: Some(23.),
            height: Some(31.),
            pressure: Some(0.78),
            twist: Some(355),
            ..Default::default()
        }
        .resolve(1)
        .unwrap();
        state.remember_pointer_properties("a", custom).unwrap();
        let explicit = PointerProperties::default().resolve(1).unwrap();
        assert_eq!(explicit.width, 1.);
        assert_eq!(explicit.pressure, 0.5);
        assert_eq!(explicit.twist, 0);
        state.release("a", &HeldInput::Button(0)).unwrap();
        let release = state.last_pointer_properties("a").unwrap();
        assert_eq!(release.width, 23.);
        assert_eq!(release.height, 31.);
        assert_eq!(release.twist, 355);
        assert_eq!(release.pressure, 0.);
    }
    #[test]
    fn dimension_fraction_is_rejected_before_any_source_admission() {
        for property in ["width", "height"] {
            let json = format!(
                r#"{{"actions":[{{"id":"p","type":"pointer","actions":[{{"type":"pointerDown","button":0,"{property}":0.1}}]}}]}}"#
            );
            assert!(ActionsRequest::parse(&ScriptValue::from_json_text(&json).unwrap()).is_err());
        }
        let invalid = PointerProperties {
            width: Some(0.1),
            ..Default::default()
        };
        assert!(invalid.resolve(1).is_err());
        assert_eq!(
            PointerProperties {
                pressure: Some(0.7),
                ..Default::default()
            }
            .resolve(0)
            .unwrap()
            .pressure,
            0.
        );
    }
}

#[cfg(test)]
mod native_interpretation_tests {
    use super::*;
    fn grant(token: u64) -> OsKeyReceipt {
        let node = oryn_dom::NodeId {
            slot: 1,
            generation: 1,
        };
        OsKeyReceipt {
            continuation_done: false,
            source: "os-key".into(),
            state: None,
            target: Some(node),
            token,
            accepted: true,
            interpret: true,
            context: crate::host::webdriver_script::AutomationRealmSnapshot {
                realm: oryn_js::realm::RealmId(0),
                parent_realm: None,
                frame: None,
                frame_generation: None,
                frame_token: None,
                document_root: node,
                document_generation: 1,
                stream_epoch: 1,
                document_authority_epoch: 0,
                url: "https://example.test/".into(),
                live: true,
            },
        }
    }
    #[test]
    fn native_interpretation_requires_source_sequence_and_live_grant() {
        let mut state = InputState::default();
        state.push_os_receipt(grant(1)).unwrap();
        assert!(state.admit_os_text("other", 1, 1).is_err());
        assert!(state.admit_os_text("os-key", 1, 2).is_err());
        state.admit_os_text("os-key", 1, 1).unwrap();
        assert!(state.admit_os_text("os-key", 1, 1).is_err());
        state.finish_os_text(1, false);
        assert!(state.admit_os_text("os-key", 1, 2).is_err());
    }
    #[test]
    fn superseded_and_retired_input_tokens_cannot_resume_text() {
        let mut state = InputState::default();
        state.push_os_receipt(grant(1)).unwrap();
        state.push_os_receipt(grant(2)).unwrap();
        assert!(state.admit_os_text("os-key", 1, 1).is_err());
        state.admit_os_text("os-key", 2, 1).unwrap();
        state.retire();
        assert!(state.admit_os_text("os-key", 2, 2).is_err());
        assert!(state.take_os_receipts().is_empty());
    }
    #[test]
    fn webdriver_json_cannot_create_native_text_continuation() {
        for kind in ["osText", "OsText", "composition"] {
            let json = format!(
                r#"{{"actions":[{{"id":"os-key","type":"key","actions":[{{"type":"{kind}","token":1,"sequence":1}}]}}]}}"#
            );
            assert!(ActionsRequest::parse(&ScriptValue::from_json_text(&json).unwrap()).is_err());
        }
    }
}
