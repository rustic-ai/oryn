//! Authenticated host keyboard dispatch. No script-visible activation/grant entry.
struct AfterKeyInputs<'a> {
    input: &'a NativeKeyboardInput,
    target: NodeId,
    key: String,
    code: String,
    location: u32,
    generation: u64,
    epoch: u64,
    accepted: bool,
}

struct KeyboardEditInputs<'a> {
    target: NodeId,
    key: &'a str,
    mods: NativeKeyboardModifiers,
    generation: u64,
    epoch: u64,
    deletion_unit: Option<crate::composition::TextUnit>,
}

use super::*;
use unicode_segmentation::UnicodeSegmentation;
#[path = "native_keyboard_receipt.rs"]
mod receipt;
#[path = "native_keyboard/text_command.rs"]
mod text_command;
pub use receipt::Receipt as NativeKeyboardReceipt;
pub(super) use text_command::Receipt as TextCommandReceipt;
pub(super) fn begin_receipt(
    state: Rc<V8BindingState>,
    input: &NativeKeyboardInput,
) -> Result<NativeKeyboardReceipt, V8BindingError> {
    receipt::Receipt::new(state, input)
}
pub(super) fn poll_receipt(
    scope: &mut v8::PinScope,
    receipt: &mut NativeKeyboardReceipt,
) -> Result<Option<bool>, V8BindingError> {
    receipt.poll(scope)
}
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeKeyboardModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
    pub alt_graph: bool,
    pub caps_lock: bool,
}
#[derive(Clone, Debug, Default)]
pub struct NativeKeyboardInput {
    pub defer_text: bool,
    pub composing: bool,
    pub key: Vec<u16>,
    pub code: String,
    pub location: u32,
    pub repeat: bool,
    pub modifiers: NativeKeyboardModifiers,
    pub down: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeKeyboardPlatform {
    MacOs,
    Other,
}
impl Default for NativeKeyboardPlatform {
    fn default() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Other
        }
    }
}
#[derive(Default)]
pub(super) struct State {
    deferred: Cell<bool>,
    deferred_target: Cell<Option<NodeId>>,
    platform: Cell<NativeKeyboardPlatform>,
    space_activation: Cell<Option<(NodeId, u64, u64)>>,
    control_history: RefCell<BTreeMap<NodeId, ControlHistory>>,
    pub(super) select_focus: RefCell<BTreeMap<NodeId, (NodeId, NodeId)>>,
    select_search: RefCell<BTreeMap<NodeId, (String, std::time::Instant)>>,
}
fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "KeyboardEvent".into(),
        member: "InvalidStateError".into(),
    }
}
fn normalized(input: &NativeKeyboardInput) -> (String, String, u32) {
    let special = if input.key.len() == 1 {
        match input.key[0] {
            0xe000 => Some(("Unidentified", 0)),
            0xe001 => Some(("Cancel", 0)),
            0xe002 => Some(("Help", 0)),
            0xe003 => Some(("Backspace", 0)),
            0xe004 => Some(("Tab", 0)),
            0xe005 => Some(("Clear", 3)),
            0xe006 => Some(("Enter", 0)),
            0xe007 => Some(("Enter", 3)),
            0xe008 => Some(("Shift", 1)),
            0xe009 => Some(("Control", 1)),
            0xe00a => Some(("Alt", 1)),
            0xe00b => Some(("Pause", 0)),
            0xe00c => Some(("Escape", 0)),
            0xe00d => Some((" ", 0)),
            0xe00e => Some(("PageUp", 0)),
            0xe00f => Some(("PageDown", 0)),
            0xe010 => Some(("End", 0)),
            0xe011 => Some(("Home", 0)),
            0xe012 => Some(("ArrowLeft", 0)),
            0xe013 => Some(("ArrowUp", 0)),
            0xe014 => Some(("ArrowRight", 0)),
            0xe015 => Some(("ArrowDown", 0)),
            0xe016 => Some(("Insert", 0)),
            0xe017 => Some(("Delete", 0)),
            0xe018 => Some((";", 0)),
            0xe019 => Some(("=", 0)),
            0xe01a => Some(("0", 3)),
            0xe01b => Some(("1", 3)),
            0xe01c => Some(("2", 3)),
            0xe01d => Some(("3", 3)),
            0xe01e => Some(("4", 3)),
            0xe01f => Some(("5", 3)),
            0xe020 => Some(("6", 3)),
            0xe021 => Some(("7", 3)),
            0xe022 => Some(("8", 3)),
            0xe023 => Some(("9", 3)),
            0xe024 => Some(("*", 3)),
            0xe025 => Some(("+", 3)),
            0xe026 => Some((",", 3)),
            0xe027 => Some(("-", 3)),
            0xe028 => Some((".", 3)),
            0xe029 => Some(("/", 3)),
            0xe031 => Some(("F1", 0)),
            0xe032 => Some(("F2", 0)),
            0xe033 => Some(("F3", 0)),
            0xe034 => Some(("F4", 0)),
            0xe035 => Some(("F5", 0)),
            0xe036 => Some(("F6", 0)),
            0xe037 => Some(("F7", 0)),
            0xe038 => Some(("F8", 0)),
            0xe039 => Some(("F9", 0)),
            0xe03a => Some(("F10", 0)),
            0xe03b => Some(("F11", 0)),
            0xe03c => Some(("F12", 0)),
            0xe03d => Some(("Meta", 1)),
            0xe040 => Some(("ZenkakuHankaku", 0)),
            0xe050 => Some(("Shift", 2)),
            0xe051 => Some(("Control", 2)),
            0xe052 => Some(("Alt", 2)),
            0xe053 => Some(("Meta", 2)),
            0xe054 => Some(("PageUp", 3)),
            0xe055 => Some(("PageDown", 3)),
            0xe056 => Some(("End", 3)),
            0xe057 => Some(("Home", 3)),
            0xe058 => Some(("ArrowLeft", 3)),
            0xe059 => Some(("ArrowUp", 3)),
            0xe05a => Some(("ArrowRight", 3)),
            0xe05b => Some(("ArrowDown", 3)),
            0xe05c => Some(("Insert", 3)),
            0xe05d => Some(("Delete", 3)),
            _ => None,
        }
    } else {
        None
    };
    let (key, location) = special
        .map(|(key, location)| (key.to_string(), location))
        .unwrap_or_else(|| (oryn_dom::dom_string::from_units(&input.key), input.location));
    let code = if !input.code.is_empty() {
        input.code.clone()
    } else {
        match key.as_str() {
            "Shift" | "Control" | "Alt" | "Meta" => {
                format!("{key}{}", if location == 2 { "Right" } else { "Left" })
            }
            " " => "Space".into(),
            "Enter" if location == 3 => "NumpadEnter".into(),
            key if location == 3 => format!(
                "Numpad{}",
                match key {
                    "*" => "Multiply",
                    "+" => "Add",
                    "," => "Comma",
                    "-" => "Subtract",
                    "." => "Decimal",
                    "/" => "Divide",
                    "PageUp" => "9",
                    "PageDown" => "3",
                    "End" => "1",
                    "Home" => "7",
                    "ArrowLeft" => "4",
                    "ArrowUp" => "8",
                    "ArrowRight" => "6",
                    "ArrowDown" => "2",
                    "Insert" => "0",
                    "Delete" => "Decimal",
                    "Clear" => "5",
                    key => key,
                }
            ),
            key if key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() => {
                format!("Key{}", key.to_ascii_uppercase())
            }
            key if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() => format!("Digit{key}"),
            ";" | ":" => "Semicolon".into(),
            "=" | "+" => "Equal".into(),
            "," | "<" => "Comma".into(),
            "-" | "_" => "Minus".into(),
            "." | ">" => "Period".into(),
            "/" | "?" => "Slash".into(),
            "`" | "~" => "Backquote".into(),
            "[" | "{" => "BracketLeft".into(),
            "]" | "}" => "BracketRight".into(),
            "\\" | "|" => "Backslash".into(),
            "'" | "\"" => "Quote".into(),
            _ if printable(&key) || key.starts_with(WTF16_STRING_PREFIX) => String::new(),
            _ => key.clone(),
        }
    };
    (key, code, location)
}
fn key_code(key: &str) -> u32 {
    match key {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" => 13,
        "Shift" => 16,
        "Control" => 17,
        "Alt" => 18,
        "Pause" => 19,
        "CapsLock" => 20,
        "Escape" => 27,
        " " => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Insert" => 45,
        "Delete" => 46,
        "Meta" => 91,
        "NumLock" => 144,
        "ScrollLock" => 145,
        ";" | ":" => 186,
        "=" | "+" => 187,
        "," | "<" => 188,
        "-" | "_" => 189,
        "." | ">" => 190,
        "/" | "?" => 191,
        "`" | "~" => 192,
        "[" | "{" => 219,
        "\\" | "|" => 220,
        "]" | "}" => 221,
        "'" | "\"" => 222,
        _ if key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric() => {
            key.as_bytes()[0].to_ascii_uppercase() as u32
        }
        _ => key
            .strip_prefix('F')
            .and_then(|n| n.parse::<u32>().ok())
            .filter(|n| (1..=12).contains(n))
            .map_or(0, |n| 111 + n),
    }
}
fn event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    input: &NativeKeyboardInput,
    key: &str,
    code: &str,
    location: u32,
    kind: &str,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let units = oryn_dom::dom_string::units(key);
    let code = code.encode_utf16().collect::<Vec<_>>();
    let mods = input.modifiers;
    let char_code = if kind == "keypress" {
        if key == "Enter" {
            13
        } else {
            String::from_utf16(&units)
                .ok()
                .and_then(|key| key.chars().next())
                .map_or(0, |c| c as u32)
        }
    } else {
        0
    };
    create_native_input_event(
        scope,
        state,
        NativeInputEventInputs {
            interface: "KeyboardEvent",
            event_type: kind,
            bubbles: true,
            cancelable: true,
            composed: true,
            detail: 0,
            related: None,
            fields: &[
                ("key", NativeInputValue::String(&units)),
                ("code", NativeInputValue::String(&code)),
                ("location", NativeInputValue::Number(location as f64)),
                (
                    "repeat",
                    NativeInputValue::Bool(input.repeat && kind != "keyup"),
                ),
                ("isComposing", NativeInputValue::Bool(input.composing)),
                ("ctrlKey", NativeInputValue::Bool(mods.control)),
                ("shiftKey", NativeInputValue::Bool(mods.shift)),
                ("altKey", NativeInputValue::Bool(mods.alt)),
                ("metaKey", NativeInputValue::Bool(mods.meta)),
                ("modifierAltGraph", NativeInputValue::Bool(mods.alt_graph)),
                ("modifierCapsLock", NativeInputValue::Bool(mods.caps_lock)),
                (
                    "keyCode",
                    NativeInputValue::Number(if kind == "keypress" {
                        char_code
                    } else {
                        key_code(key)
                    } as f64),
                ),
                ("charCode", NativeInputValue::Number(char_code as f64)),
            ],
        },
    )
}
fn focus_target(state: &V8BindingState) -> Result<NodeId, V8BindingError> {
    let dom = state.dom.borrow();
    let document = state.document_root();
    if let Some(node) = dom.focused() {
        if dom.arena().node_document(node)? != Some(document) {
            return Err(invalid());
        }
        if dom.is_document_connected(node)? {
            return Ok(node);
        }
    }
    for node in dom.arena().descendants_inclusive(document)? {
        if dom.arena().node_document(node)? == Some(document)
            && dom.is_element(node, "http://www.w3.org/1999/xhtml", "body")
        {
            return Ok(node);
        }
    }
    Ok(document)
}
fn live(state: &V8BindingState, node: NodeId, generation: u64, epoch: u64) -> bool {
    !state.execution_retired()
        && state.document_generation() == generation
        && state.document_stream_epoch() == epoch
        && state
            .dom
            .borrow()
            .is_document_connected(node)
            .unwrap_or(false)
        && state
            .dom
            .borrow()
            .arena()
            .node_document(node)
            .ok()
            .flatten()
            == Some(state.document_root())
}
fn printable(key: &str) -> bool {
    let units = oryn_dom::dom_string::units(key);
    String::from_utf16(&units).is_ok_and(|text| {
        text.graphemes(true).count() == 1
            && !text.chars().any(char::is_control)
            && !text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c))
    })
}
fn activate(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
) -> Result<(), V8BindingError> {
    if state.dom.borrow().control_disabled(target)?
        || retained_activation::active(state, target)
        || !state.click_in_progress.borrow_mut().insert(target)
    {
        return Ok(());
    }
    let event = create_synthetic_click_event(scope, state)?;
    let id = event_id(scope, event)?;
    if let Some(record) = state.events.borrow_mut().get_mut(&id) {
        record.is_trusted = true;
    }
    let object = wrap_node(scope, state, target)?;
    let result = dispatch_event(scope, state, target, event);
    state.click_in_progress.borrow_mut().remove(&target);
    let (owner, target) = node_identity::argument(scope, object.into())?
        .ok_or(V8BindingError::Allocation("keyboard activation owner"))?;
    owner.click_in_progress.borrow_mut().remove(&target);
    if result? {
        permission_elements::activate(&owner, target, true)?;
    }
    Ok(())
}
fn activatable(state: &V8BindingState, node: NodeId, space: bool) -> Result<bool, V8BindingError> {
    let dom = state.dom.borrow();
    if dom.control_disabled(node)? {
        return Ok(false);
    }
    let html = "http://www.w3.org/1999/xhtml";
    Ok(dom.arena().get(node)?.submit_behavior.is_some()
        || dom.is_element(node, html, "button")
        || (dom.is_element(node, html, "input")
            && dom
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|kind| {
                    matches!(
                        kind.to_ascii_lowercase().as_str(),
                        "button" | "submit" | "reset" | "checkbox" | "radio"
                    )
                }))
        || (!space
            && dom.is_element(node, html, "a")
            && dom.attribute_in_null_namespace(node, "href")?.is_some()))
}
fn admit_selection(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    anchor: BoundaryPoint,
    focus: BoundaryPoint,
) -> Result<bool, V8BindingError> {
    synchronize_selection_range(state)?;
    if anchor == focus || state.selection_anchor.get() != state.selection_focus.get() {
        return Ok(true);
    }
    let original = selection_snapshot(state);
    let associated = state.selection_range.get();
    let generation = state.document_generation();
    let epoch = state.document_stream_epoch();
    let boundaries = state.dom.borrow().boundary_revision();
    let target = state
        .selection_anchor
        .get()
        .map_or(anchor.container, |point| point.container);
    let event = create_event_object(scope, state, "selectstart", true, true)?;
    let id = event_id(scope, event)?;
    state
        .events
        .borrow_mut()
        .get_mut(&id)
        .ok_or(V8BindingError::Allocation("selectstart event"))?
        .is_trusted = true;
    if !dispatch_event(scope, state, target, event)? {
        return Ok(false);
    }
    synchronize_selection_range(state)?;
    // Author changes supersede this prepared native movement. Never overwrite
    // their selection, or apply an offset calculated before a tree mutation.
    Ok(!state.execution_retired()
        && !state.execution_suspended()
        && state.document_generation() == generation
        && state.document_stream_epoch() == epoch
        && state.dom.borrow().boundary_revision() == boundaries
        && state.selection_range.get() == associated
        && selection_snapshot(state) == original)
}
fn select_all(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
) -> Result<(), V8BindingError> {
    if state.dom.borrow().is_editable_text_control(target) {
        let selected = selection_text::prepare_control(state, target)?;
        let length =
            oryn_dom::dom_string::len(&state.dom.borrow().control_edit_value(target)?) as u32;
        let next = (0, length, "none".into());
        if state.text_control_selections.borrow().get(&target) != Some(&next) {
            queue_control_select(scope, state, target)?;
            state
                .text_control_selections
                .borrow_mut()
                .insert(target, next);
        }
        if let Some(selected) = selected {
            *state.selected_text_control.borrow_mut() = Some(selected);
        }
        return Ok(());
    }
    let root = editing_host(state)?.unwrap_or(state.document_root());
    let length = light_children(&state.dom.borrow(), root)?.len() as u32;
    let anchor = BoundaryPoint {
        container: root,
        offset: 0,
    };
    let focus = BoundaryPoint {
        container: root,
        offset: length,
    };
    if admit_selection(scope, state, anchor, focus)? {
        set_selection_boundaries(state, anchor, focus)?;
    }
    Ok(())
}
fn movement_parameters(
    state: &V8BindingState,
    key: &str,
    mods: NativeKeyboardModifiers,
) -> Option<(&'static str, &'static str, &'static str)> {
    let mac = state.keyboard.platform.get() == NativeKeyboardPlatform::MacOs;
    let word = if mac { mods.alt } else { mods.control };
    let document = if mac { mods.meta } else { mods.control };
    let (direction, granularity) = match key {
        "PageUp" => ("backward", "page"),
        "PageDown" => ("forward", "page"),
        "ArrowLeft" => (
            "left",
            if mac && mods.meta {
                "lineboundary"
            } else if word {
                "word"
            } else {
                "character"
            },
        ),
        "ArrowRight" => (
            "right",
            if mac && mods.meta {
                "lineboundary"
            } else if word {
                "word"
            } else {
                "character"
            },
        ),
        "ArrowUp" => (
            "backward",
            if document {
                "documentboundary"
            } else if mac && mods.alt {
                "paragraph"
            } else {
                "line"
            },
        ),
        "ArrowDown" => (
            "forward",
            if document {
                "documentboundary"
            } else if mac && mods.alt {
                "paragraph"
            } else {
                "line"
            },
        ),
        "Home" => (
            "backward",
            if document {
                "documentboundary"
            } else {
                "lineboundary"
            },
        ),
        "End" => (
            "forward",
            if document {
                "documentboundary"
            } else {
                "lineboundary"
            },
        ),
        _ => return None,
    };
    Some((
        if mods.shift { "extend" } else { "move" },
        direction,
        granularity,
    ))
}
fn movement(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    key: &str,
    mods: NativeKeyboardModifiers,
) -> Result<bool, V8BindingError> {
    let Some((alter, direction, granularity)) = movement_parameters(state, key, mods) else {
        return Ok(false);
    };
    if !selection_movement::modify_control(scope, state, alter, direction, granularity)?
        && let Some(plan) =
            selection_movement::prepare_movement(scope, state, alter, direction, granularity)?
    {
        if let Some((anchor, focus)) = plan.boundaries
            && !admit_selection(scope, state, anchor, focus)?
        {
            return Ok(true);
        }
        plan.commit(state)?;
    }
    Ok(true)
}

fn edit(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    key: &str,
    mods: NativeKeyboardModifiers,
    generation: u64,
    epoch: u64,
) -> Result<(), V8BindingError> {
    edit_with_deletion_unit(
        scope,
        state,
        KeyboardEditInputs {
            target,
            key,
            mods,
            generation,
            epoch,
            deletion_unit: None,
        },
    )
}
struct EditDescription {
    deletion_unit: Option<crate::composition::TextUnit>,
    is_control: bool,
    backward: bool,
    deletion: bool,
    line: bool,
    word_delete: bool,
    line_delete: bool,
    hard_line_delete: bool,
    input_type: String,
    replacement: String,
    enter: bool,
}
fn prepare_edit(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    key: &str,
    mods: NativeKeyboardModifiers,
    deletion_unit: Option<crate::composition::TextUnit>,
) -> Result<Option<EditDescription>, V8BindingError> {
    if !clipboard_actions::editable(state, target)?
        || css_suppresses_focus_in_scope(scope, state, target)?
    {
        return Ok(None);
    }
    let is_control = state.dom.borrow().is_editable_text_control(target);
    let backward = key == "Backspace";
    let deletion = backward || key == "Delete";
    let line = key == "Enter"
        && (mods.shift
            || editing_host(state)?
                .is_some_and(|host| host_is_plaintext_only(&state.dom.borrow(), host)));
    let mac = state.keyboard.platform.get() == NativeKeyboardPlatform::MacOs;
    let word_delete = deletion
        && deletion_unit.map_or(if mac { mods.alt } else { mods.control }, |unit| {
            unit == crate::composition::TextUnit::Word
        });
    let line_delete = deletion
        && deletion_unit.map_or(mac && mods.meta, |unit| {
            unit == crate::composition::TextUnit::LineBoundary
        });
    let hard_line_delete =
        deletion && deletion_unit == Some(crate::composition::TextUnit::ParagraphBoundary);
    let input_type = if hard_line_delete {
        if backward {
            "deleteHardLineBackward"
        } else {
            "deleteHardLineForward"
        }
    } else if word_delete {
        if backward {
            "deleteWordBackward"
        } else {
            "deleteWordForward"
        }
    } else if line_delete {
        if backward {
            "deleteSoftLineBackward"
        } else {
            "deleteSoftLineForward"
        }
    } else if backward {
        "deleteContentBackward"
    } else if deletion {
        "deleteContentForward"
    } else if key == "Enter" {
        if line || is_control {
            "insertLineBreak"
        } else {
            "insertParagraph"
        }
    } else {
        "insertText"
    };
    let replacement = if deletion {
        ""
    } else if key == "Enter" {
        "\n"
    } else {
        key
    };
    Ok(Some(EditDescription {
        deletion_unit,
        is_control,
        backward,
        deletion,
        line,
        word_delete,
        line_delete,
        hard_line_delete,
        input_type: input_type.into(),
        replacement: replacement.into(),
        enter: key == "Enter",
    }))
}
fn text_unit(unit: crate::composition::TextUnit) -> &'static str {
    use crate::composition::TextUnit;
    match unit {
        TextUnit::Character => "character",
        TextUnit::Word => "word",
        TextUnit::Line => "line",
        TextUnit::LineBoundary => "lineboundary",
        TextUnit::Paragraph => "paragraph",
        TextUnit::ParagraphBoundary => "paragraphboundary",
        TextUnit::DocumentBoundary => "documentboundary",
    }
}
fn deletion_granularity(description: &EditDescription) -> Option<&'static str> {
    description
        .deletion_unit
        .filter(|unit| *unit != crate::composition::TextUnit::Character)
        .map(text_unit)
        .or({
            if description.word_delete {
                Some("word")
            } else if description.hard_line_delete {
                Some("paragraphboundary")
            } else if description.line_delete {
                Some("lineboundary")
            } else {
                None
            }
        })
}
fn apply_edit(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    description: &EditDescription,
) -> Result<bool, V8BindingError> {
    let EditDescription {
        is_control,
        backward,
        deletion,
        line,
        word_delete,
        line_delete,
        hard_line_delete,
        ..
    } = *description;
    let input_type = description.input_type.as_str();
    let replacement = description.replacement.as_str();
    let key = if description.enter { "Enter" } else { "" };
    let changed = if is_control {
        let (mut start, mut end, before) = clipboard_actions::control_selection(state, target)?;
        if deletion && start == end {
            let initial_selection = (start, end, before.clone());
            let initial_generation = state.document_generation();
            let initial_authority = state.document_authority_epoch();
            let units = oryn_dom::dom_string::units(&before);
            let text = String::from_utf16_lossy(&units);
            let mut offset = 0;
            let mut boundaries = vec![0];
            for grapheme in text.graphemes(true) {
                offset += grapheme.encode_utf16().count() as u32;
                boundaries.push(offset);
            }
            if let Some(unit) = description.deletion_unit {
                let Some(plan) = selection_movement::prepare_control_movement(
                    scope,
                    state,
                    target,
                    "extend",
                    if backward { "backward" } else { "forward" },
                    text_unit(unit),
                )?
                else {
                    return Ok(false);
                };
                if state.execution_retired()
                    || state.execution_suspended()
                    || state.document_generation() != initial_generation
                    || state.document_authority_epoch() != initial_authority
                    || clipboard_actions::control_selection(state, target)? != initial_selection
                {
                    return Ok(false);
                }
                let boundaries = plan.boundaries();
                start = boundaries.0;
                end = boundaries.1;
            } else if hard_line_delete {
                if backward {
                    start = units[..start as usize]
                        .iter()
                        .rposition(|unit| *unit == 10)
                        .map_or(0, |index| index as u32 + 1);
                } else {
                    end = units[end as usize..]
                        .iter()
                        .position(|unit| *unit == 10)
                        .map_or(units.len() as u32, |index| end + index as u32);
                }
            } else if line_delete {
                let Some(boundary) = selection_movement::control_line_boundary(
                    scope, state, target, &before, start, !backward,
                )?
                else {
                    return Ok(false);
                };
                if state.execution_retired()
                    || state.execution_suspended()
                    || state.document_generation() != initial_generation
                    || state.document_authority_epoch() != initial_authority
                    || clipboard_actions::control_selection(state, target)? != initial_selection
                {
                    return Ok(false);
                }
                if backward {
                    start = boundary;
                } else {
                    end = boundary;
                }
            } else if word_delete {
                if backward {
                    let prefix = String::from_utf16_lossy(&units[..start as usize]);
                    start = prefix
                        .unicode_word_indices()
                        .next_back()
                        .map_or(0, |(index, _)| {
                            prefix[..index].encode_utf16().count() as u32
                        });
                } else {
                    let suffix = String::from_utf16_lossy(&units[end as usize..]);
                    end = suffix.unicode_word_indices().next().map_or(
                        units.len() as u32,
                        |(index, word)| {
                            end + suffix[..index + word.len()].encode_utf16().count() as u32
                        },
                    );
                }
            } else if backward {
                start = boundaries
                    .into_iter()
                    .rfind(|offset| *offset < start)
                    .unwrap_or(0);
            } else {
                end = boundaries
                    .into_iter()
                    .find(|offset| *offset > end)
                    .unwrap_or(units.len() as u32);
            }
        }
        if start == end && replacement.is_empty() {
            return Ok(false);
        }
        edit_native_control(state, target, start, end, replacement)?;
        before != state.dom.borrow().control_edit_value(target)? || start != end
    } else {
        editing_commands::run(
            scope,
            state,
            input_type,
            if deletion {
                editing_commands::Command::Delete {
                    forward: !backward,
                    unit: deletion_granularity(description).map(str::to_owned),
                }
            } else if key == "Enter" {
                if line {
                    editing_commands::Command::LineBreak
                } else {
                    editing_commands::Command::Paragraph
                }
            } else {
                editing_commands::Command::Text(replacement.into())
            },
        )?
    };
    Ok(changed)
}
fn edit_with_deletion_unit(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    params: KeyboardEditInputs<'_>,
) -> Result<(), V8BindingError> {
    let KeyboardEditInputs {
        target,
        key,
        mods,
        generation,
        epoch,
        deletion_unit,
    } = params;

    let Some(description) = prepare_edit(scope, state, target, key, mods, deletion_unit)? else {
        return Ok(());
    };
    let data =
        (!description.deletion && !description.enter).then_some(description.replacement.as_str());
    let planned = if description.deletion
        && !description.is_control
        && state.selection_anchor.get() == state.selection_focus.get()
        && let Some(unit) = deletion_granularity(&description)
    {
        let Some(plan) = selection_movement::prepare_movement(
            scope,
            state,
            "extend",
            if description.backward {
                "backward"
            } else {
                "forward"
            },
            unit,
        )?
        else {
            return Ok(());
        };
        if !live(state, target, generation, epoch) || !clipboard_actions::editable(state, target)? {
            return Ok(());
        }
        let _retained_scope = state.enter_retained_allocation(scope)?;
        let allocation = state
            .retained_allocation
            .borrow()
            .clone()
            .map(|allocation| {
                allocation.reserve(
                    std::mem::size_of::<selection_movement::PreparedMovement>()
                        + 2 * std::mem::size_of::<usize>(),
                )
            })
            .transpose()
            .map_err(V8BindingError::EventCallback)?;
        Some((
            Rc::new(plan),
            (
                selection_snapshot(state),
                state.selection_range.get(),
                state.dom.borrow().boundary_revision(),
            ),
            allocation,
        ))
    } else {
        None
    };
    if let Some((plan, original, _allocation)) = planned {
        let Some(range) = plan.ordered_range(state)? else {
            return Ok(());
        };
        if !clipboard_actions::dispatch_prepared_editing(
            scope,
            state,
            target,
            &description.input_type,
            range,
        )? || !live(state, target, generation, epoch)
            || !clipboard_actions::editable(state, target)?
            || state.dom.borrow().is_inert(target)?
            || css_suppresses_focus_in_scope(scope, state, target)?
            || state.dom.borrow().is_editable_text_control(target) != description.is_control
            || original.0 != selection_snapshot(state)
            || original.1 != state.selection_range.get()
            || original.2 != state.dom.borrow().boundary_revision()
        {
            return Ok(());
        }
        let changed = editing_commands::run(
            scope,
            state,
            &description.input_type,
            editing_commands::Command::DeletePrepared {
                forward: !description.backward,
                plan,
                original,
                allocation: _allocation,
            },
        )?;
        if changed && live(state, target, generation, epoch) {
            clipboard_actions::fire_editing(
                scope,
                state,
                target,
                "input",
                &description.input_type,
                None,
                None,
            )?;
        }
        return Ok(());
    }
    if !clipboard_actions::fire_editing(
        scope,
        state,
        target,
        "beforeinput",
        &description.input_type,
        data,
        None,
    )? || !live(state, target, generation, epoch)
        || !clipboard_actions::editable(state, target)?
        || state.dom.borrow().is_inert(target)?
        || css_suppresses_focus_in_scope(scope, state, target)?
        || state.dom.borrow().is_editable_text_control(target) != description.is_control
    {
        return Ok(());
    }
    let changed = apply_edit(scope, state, target, &description)?;
    if changed && live(state, target, generation, epoch) {
        clipboard_actions::fire_editing(
            scope,
            state,
            target,
            "input",
            &description.input_type,
            data,
            None,
        )?;
    }
    Ok(())
}

fn implicit_submit(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
) -> Result<(), V8BindingError> {
    let form = state.dom.borrow().form_owner(target)?;
    let Some(form) = form else {
        return Ok(());
    };
    let mut submitter = None;
    let mut blockers = 0;
    {
        let dom = state.dom.borrow();
        for node in dom.arena().descendants_inclusive(state.document_root())? {
            if dom.form_owner(node)? != Some(form) {
                continue;
            }
            if dom.is_submit_button(node)? && submitter.is_none() {
                submitter = Some(node);
            }
            if dom.is_element(node, "http://www.w3.org/1999/xhtml", "input")
                && dom
                    .attribute_in_null_namespace(node, "type")?
                    .is_none_or(|kind| {
                        matches!(
                            kind.to_ascii_lowercase().as_str(),
                            "text"
                                | "search"
                                | "tel"
                                | "url"
                                | "email"
                                | "password"
                                | "date"
                                | "month"
                                | "week"
                                | "time"
                                | "datetime-local"
                                | "number"
                        )
                    })
            {
                blockers += 1;
            }
        }
    }
    if let Some(submitter) = submitter {
        activate(scope, state, submitter)?;
    } else if blockers <= 1 {
        submissions::request_submit(scope, state, form, None)?;
    }
    Ok(())
}
/// Dispatch exactly one host key transition in the selected native document.
/// The host owns pressed-key state; supplied modifier/repeat snapshots are never read from JS.
pub fn dispatch(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    input: &NativeKeyboardInput,
) -> Result<bool, V8BindingError> {
    state.keyboard.deferred.set(false);
    state.keyboard.deferred_target.set(None);
    if state.execution_retired() || input.location > 3 {
        return Err(invalid());
    }
    update_native_pointer_modifiers(state, &input.modifiers);
    fixup_focus_eligibility(scope, state)?;
    if state.execution_retired() {
        return Err(invalid());
    }
    let generation = state.document_generation();
    let epoch = state.document_stream_epoch();
    let target = focus_target(state)?;
    let (key, code, location) = normalized(input);
    let mods = input.modifiers;
    if input.down && !mods.control && !mods.alt && !mods.meta {
        state.dom.borrow_mut().set_focus_visible(true);
    }
    if input.down && key != "Escape" {
        state.notify_user_activation();
    }
    if state.permission_elements.borrow().prompt.is_some() {
        if input.down {
            permission_prompt::keyboard(scope, state, &key)?;
        }
        return Ok(true);
    }
    let key_event = event(
        scope,
        state,
        input,
        &key,
        &code,
        location,
        if input.down { "keydown" } else { "keyup" },
    )?;
    let accepted = dispatch_event(scope, state, target, key_event)?;
    dispatch_after_key(
        scope,
        state,
        AfterKeyInputs {
            input,
            target,
            key,
            code,
            location,
            generation,
            epoch,
            accepted,
        },
    )
}
fn dispatch_after_key(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    params: AfterKeyInputs<'_>,
) -> Result<bool, V8BindingError> {
    let AfterKeyInputs {
        input,
        mut target,
        mut key,
        code,
        location,
        generation,
        epoch,
        accepted,
    } = params;

    let mods = input.modifiers;
    if !input.down {
        if key == " "
            && let Some((armed, generation, epoch)) = state.keyboard.space_activation.take()
            && accepted
            && armed == target
            && live(state, target, generation, epoch)
            && activatable(state, target, true)?
        {
            activate(scope, state, target)?;
        }
        return Ok(accepted);
    }
    if !accepted || !live(state, target, generation, epoch) {
        return Ok(accepted);
    }
    target = focus_target(state)?;
    if permission_prompt::keyboard(scope, state, &key)? {
        return Ok(true);
    }
    if !mods.control
        && !mods.meta
        && !mods.alt
        && matches!(key.as_str(), "Enter" | " ")
        && state.dom.borrow().is_file_input(target)
    {
        file_picker::open(scope, state, target)?;
        return Ok(true);
    }
    if !mods.control
        && !mods.meta
        && temporal_picker::keyboard(scope, state, target, &key, mods.alt)?
    {
        return Ok(true);
    }
    if state.dom.borrow().is_temporal_control(target) && !mods.control && !mods.meta && !mods.alt {
        if printable(&key) {
            let press = event(scope, state, input, &key, &code, location, "keypress")?;
            if !dispatch_event(scope, state, target, press)? {
                return Ok(false);
            }
        }
        if temporal_key(scope, state, target, &key, mods.shift, generation, epoch)? {
            return Ok(true);
        }
    }
    if input.defer_text
        && (input.composing || key == "Unidentified" || printable(&key))
        && !mods.control
        && !mods.meta
    {
        target = focus_target(state)?;
        let editable = {
            let dom = state.dom.borrow();
            if dom.is_editable_text_control(target) {
                !dom.control_disabled(target)?
                    && dom
                        .attribute_in_null_namespace(target, "readonly")?
                        .is_none()
            } else {
                editing_host_for_node(
                    &dom,
                    Some(target),
                    state.design_mode_enabled(),
                    state.document_root(),
                )?
                .is_some()
            }
        };
        if editable {
            if !input.composing && printable(&key) {
                let press = event(scope, state, input, &key, &code, location, "keypress")?;
                if !dispatch_event(scope, state, target, press)? {
                    return Ok(false);
                }
            }
            if live(state, target, generation, epoch) {
                state.keyboard.deferred.set(true);
                let dom = state.dom.borrow();
                state
                    .keyboard
                    .deferred_target
                    .set(Some(if dom.is_editable_text_control(target) {
                        target
                    } else {
                        editing_host_for_node(
                            &dom,
                            Some(target),
                            state.design_mode_enabled(),
                            state.document_root(),
                        )?
                        .unwrap_or(target)
                    }));
            }
            return Ok(true);
        }
    }
    if key == "Escape"
        && native_pointer::cancel_drag_from_keyboard(
            scope,
            state,
            mods.control,
            mods.alt,
            mods.shift,
            mods.meta,
        )?
    {
        return Ok(true);
    }
    if key == "Escape" && (interest::escape(scope, state)? || top_layer::escape(scope, state)?) {
        return Ok(true);
    }
    target = focus_target(state)?;
    if !mods.control && !mods.meta && select_picker::keyboard(scope, state, target, &key, mods.alt)?
    {
        return Ok(true);
    }
    if key == "ContextMenu"
        || (key == "F10" && mods.shift && !mods.control && !mods.alt && !mods.meta)
    {
        native_pointer::context_menu_from_keyboard(
            scope,
            state,
            target,
            mods.control,
            mods.alt,
            mods.shift,
            mods.meta,
        )?;
        return Ok(true);
    }
    if matches!(
        key.as_str(),
        "Shift" | "Control" | "Alt" | "Meta" | "AltGraph" | "CapsLock" | "NumLock" | "ScrollLock"
    ) {
        return Ok(true);
    }
    let mac = state.keyboard.platform.get() == NativeKeyboardPlatform::MacOs;
    let shortcut = (if mac { mods.meta } else { mods.control }) && !mods.alt_graph;
    if mac && mods.control && !mods.alt_graph && !mods.meta && !mods.alt {
        key = match key.as_str() {
            "a" => "Home",
            "e" => "End",
            "b" => "ArrowLeft",
            "f" => "ArrowRight",
            "p" => "ArrowUp",
            "n" => "ArrowDown",
            "h" => "Backspace",
            "d" => "Delete",
            _ => return Ok(true),
        }
        .into();
    }
    if shortcut {
        match key.to_ascii_lowercase().as_str() {
            "c" | "x" | "v" => {
                let action = match key.to_ascii_lowercase().as_str() {
                    "c" => crate::clipboard::ClipboardAction::Copy,
                    "x" => crate::clipboard::ClipboardAction::Cut,
                    _ => crate::clipboard::ClipboardAction::Paste,
                };
                let mut grant = grant_native_clipboard_action(state, target, action)?;
                native_clipboard_action(scope, state, &mut grant)?;
                return Ok(true);
            }
            "a" => {
                select_all(scope, state, target)?;
                return Ok(true);
            }
            "z" | "y" => {
                let redo = mods.shift || key.eq_ignore_ascii_case("y");
                let kind = if redo { "historyRedo" } else { "historyUndo" };
                if clipboard_actions::editable(state, target)?
                    && clipboard_actions::fire_editing(
                        scope,
                        state,
                        target,
                        "beforeinput",
                        kind,
                        None,
                        None,
                    )?
                    && live(state, target, generation, epoch)
                    && (if state.dom.borrow().is_editable_text_control(target) {
                        undo_control(state, target, redo)?
                    } else {
                        editing_commands::paragraph_history::dispatch(scope, state, redo)?
                    })
                {
                    clipboard_actions::fire_editing(
                        scope, state, target, "input", kind, None, None,
                    )?;
                }
                return Ok(true);
            }
            _ => {}
        }
    }
    if key == "Tab" && !shortcut && !mods.alt {
        focus_navigation::advance(scope, state, mods.shift)?;
        return Ok(true);
    }
    if matches!(
        key.as_str(),
        "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown"
    ) && state.dom.borrow().is_radio(target)
    {
        if !shortcut && !mods.alt {
            focus_navigation::radio_arrow(scope, state, target, &key)?;
        }
        return Ok(true);
    }
    if !mac && mods.alt && !mods.alt_graph {
        return Ok(true);
    }
    if (!printable(&key) || shortcut) && control_step(scope, state, target, &key, mods)? {
        return Ok(true);
    }
    if key != " " && scroll_default(scope, state, target, &key, mods)? {
        return Ok(true);
    }
    if movement(scope, state, &key, mods)? {
        return Ok(true);
    }
    let deletion = matches!(key.as_str(), "Backspace" | "Delete");
    if (shortcut && !deletion)
        || (mods.alt && !mods.alt_graph && !(mac && deletion))
        || (!mac && mods.meta)
    {
        return Ok(true);
    }
    if printable(&key) || key == "Enter" {
        let press = event(scope, state, input, &key, &code, location, "keypress")?;
        if !dispatch_event(scope, state, target, press)? {
            return Ok(false);
        }
        if !live(state, target, generation, epoch) {
            return Ok(true);
        }
    }
    if printable(&key) && control_step(scope, state, target, &key, mods)? {
        return Ok(true);
    }
    if key == "Enter" && activatable(state, target, false)? {
        activate(scope, state, target)?;
        return Ok(true);
    }
    if key == " " && activatable(state, target, true)? {
        if !input.repeat {
            state
                .keyboard
                .space_activation
                .set(Some((target, generation, epoch)));
        }
        return Ok(true);
    }
    if key == " " && scroll_default(scope, state, target, &key, mods)? {
        return Ok(true);
    }
    if key == "Enter"
        && state
            .dom
            .borrow()
            .is_element(target, "http://www.w3.org/1999/xhtml", "input")
    {
        implicit_submit(scope, state, target)?;
        return Ok(true);
    }
    if printable(&key) || matches!(key.as_str(), "Enter" | "Backspace" | "Delete") {
        if edit_context::keyboard(scope,state,target,&key)? {return Ok(true);}
        edit(scope, state, target, &key, mods, generation, epoch)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(key: &str, down: bool) -> NativeKeyboardInput {
        NativeKeyboardInput {
            key: key.encode_utf16().collect(),
            down,
            ..Default::default()
        }
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, code).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn deletion_does_not_use_geometry_after_provider_changes_control_value() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=x value=abcdef>")
                .into_runtime()
                .unwrap(),
        ));
        let node = dom.borrow().find_by_id("x").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(221), 1).unwrap();
        struct Provider {
            state: std::rc::Weak<V8BindingState>,
            node: NodeId,
            calls: Cell<usize>,
        }
        impl BindingLayoutProvider for Provider {
            fn compute_layout(
                &self,
                _: &BTreeMap<NodeId, BTreeSet<String>>,
            ) -> Result<BindingLayout, String> {
                self.calls.set(self.calls.get() + 1);
                let state = self.state.upgrade().ok_or("owner retired")?;
                state
                    .dom
                    .borrow_mut()
                    .restore_control_edit_value(self.node, "author".into(), false)
                    .map_err(|e| e.to_string())?;
                state
                    .text_control_selections
                    .borrow_mut()
                    .insert(self.node, (3, 3, "none".into()));
                Ok(BindingLayout::default())
            }
        }
        state
            .text_control_selections
            .borrow_mut()
            .insert(node, (2, 2, "none".into()));
        let provider = Rc::new(Provider {
            state: Rc::downgrade(&state),
            node,
            calls: Cell::new(0),
        });
        state.set_layout_provider(provider.clone());
        let description = EditDescription {
            deletion_unit: Some(crate::composition::TextUnit::Line),
            is_control: true,
            backward: true,
            deletion: true,
            line: false,
            word_delete: false,
            line_delete: false,
            hard_line_delete: false,
            input_type: "deleteContentBackward".into(),
            replacement: String::new(),
            enter: false,
        };
        assert!(!apply_edit(scope, &state, node, &description).unwrap());
        assert_eq!(provider.calls.get(), 1);
        assert_eq!(
            state.dom.borrow().control_edit_value(node).unwrap(),
            "author"
        );
        assert_eq!(
            state.text_control_selections.borrow().get(&node).unwrap().0,
            3
        );
    }
    #[test]
    fn native_keyboard_selectstart_precedes_extension_and_cancellation_preserves_range() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=editor contenteditable>abc</div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(216), 1).unwrap();
        set_platform(&state, NativeKeyboardPlatform::Other);
        eval(
            scope,
            "globalThis.s=getSelection();globalThis.text=editor.firstChild;editor.focus();s.collapse(text,0);globalThis.count=0;globalThis.cancel=true;globalThis.reenter=false;document.addEventListener('selectstart',e=>{count++;if(!e.isTrusted||!e.bubbles||!e.cancelable||!s.isCollapsed)throw Error('selectstart ordering');if(cancel)e.preventDefault();if(reenter)s.collapse(text,2)});globalThis.original=s.getRangeAt(0)",
        );
        let mut key = input("ArrowRight", true);
        key.modifiers.shift = true;
        dispatch(scope, &state, &key).unwrap();
        assert!(
            eval(
                scope,
                "count===1&&s.getRangeAt(0)===original&&s.isCollapsed&&s.anchorOffset===0"
            )
            .is_true()
        );
        eval(scope, "cancel=false");
        dispatch(scope, &state, &key).unwrap();
        assert!(
            eval(
                scope,
                "count===2&&s.anchorOffset===0&&s.focusOffset===1&&!s.isCollapsed"
            )
            .is_true()
        );
        dispatch(scope, &state, &key).unwrap();
        assert!(eval(scope, "count===2&&s.focusOffset===2").is_true());
        eval(scope, "s.collapse(text,3)");
        dispatch(scope, &state, &key).unwrap();
        assert!(eval(scope, "count===2&&s.isCollapsed&&s.focusOffset===3").is_true());
        eval(scope, "s.collapse(text,0);reenter=true");
        dispatch(scope, &state, &key).unwrap();
        assert!(eval(scope, "count===3&&s.isCollapsed&&s.focusOffset===2").is_true());
        eval(
            scope,
            "reenter=false;cancel=true;s.collapse(text,1);original=s.getRangeAt(0)",
        );
        let mut all = input("a", true);
        all.modifiers.control = true;
        dispatch(scope, &state, &all).unwrap();
        assert!(
            eval(
                scope,
                "count===4&&s.getRangeAt(0)===original&&s.isCollapsed&&s.focusOffset===1"
            )
            .is_true()
        );
        eval(scope, "cancel=false");
        dispatch(scope, &state, &all).unwrap();
        assert!(eval(scope, "count===5&&String(s)==='abc'").is_true());
    }
    #[test]
    fn webdriver_keys_have_native_location_and_lossless_character_identity() {
        let mut key = input("\u{e007}", true);
        assert_eq!(normalized(&key), ("Enter".into(), "NumpadEnter".into(), 3));
        key.key = vec![0xe050];
        assert_eq!(normalized(&key), ("Shift".into(), "ShiftRight".into(), 2));
        key.key = "😀".encode_utf16().collect();
        assert_eq!(normalized(&key).0, "😀");
        assert!(printable("😀"));
        assert!(!printable("ArrowRight"));
        key.key = vec![0xd800];
        assert_eq!(
            oryn_dom::dom_string::units(&normalized(&key).0),
            vec![0xd800]
        );
    }
    #[test]
    fn keyboard_context_menu_uses_focus_and_obeys_canceled_keydown() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<button id=t>menu</button>")
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
            "globalThis.menus=[];t.focus();t.addEventListener('contextmenu',e=>{menus.push(e.target===t&&e.isTrusted&&e.cancelable&&e.composed&&e.pointerId===-1&&e.pointerType===''&&e.buttons===0);e.preventDefault()});document.elementFromPoint=()=>{throw Error('author hit test')}",
        );
        let target = state.dom.borrow().find_by_id("t").unwrap();
        state.set_layout(BindingLayout {
            viewport: Some((400., 300.)),
            geometry: BTreeMap::from([(
                node_key(target),
                BindingGeometry::Available(BindingRect {
                    x: 10.,
                    y: 10.,
                    width: 80.,
                    height: 30.,
                }),
            )]),
            ..Default::default()
        });
        let mut key = input("F10", true);
        key.modifiers.shift = true;
        dispatch(scope, &state, &key).unwrap();
        eval(
            scope,
            "t.addEventListener('keydown',e=>e.preventDefault(),{once:true})",
        );
        dispatch(scope, &state, &key).unwrap();
        dispatch(scope, &state, &input("ContextMenu", true)).unwrap();
        assert!(eval(scope, "menus.length===2&&menus.every(Boolean)").is_true());
    }
    #[test]
    fn number_keys_keep_bad_input_text_and_report_native_validity() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<form id=f><input id=n type=number placeholder=Number><button id=b type=button>Next</button></form>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(scope, "n.focus();globalThis.edits=0;n.oninput=()=>edits++");
        dispatch(scope, &state, &input("1", true)).unwrap();
        dispatch(scope, &state, &input("e", true)).unwrap();
        assert!(eval(scope,"n.value===''&&n.validity.badInput&&!n.validity.valid&&n.validationMessage!==''&&n.matches(':invalid')&&!n.matches(':placeholder-shown')&&edits===2").is_true());
        dispatch(scope, &state, &input("2", true)).unwrap();
        assert!(
            eval(
                scope,
                "n.value==='1e2'&&n.valueAsNumber===100&&!n.validity.badInput"
            )
            .is_true()
        );
        dispatch(scope, &state, &input("Backspace", true)).unwrap();
        assert!(eval(scope, "n.validity.badInput").is_true());
        dispatch(scope, &state, &input("Tab", true)).unwrap();
        assert!(eval(scope, "n.matches(':user-invalid')").is_true());
        assert!(
            eval(
                scope,
                "n.value='invalid';!n.validity.badInput&&n.value===''"
            )
            .is_true()
        );
        eval(scope, "f.reset()");
        assert!(eval(scope, "!n.validity.badInput&&!n.matches(':user-invalid')").is_true());
    }
    #[test]
    fn native_key_events_bypass_author_constructors_and_cancel_real_defaults() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t><button id=b>go</button>")
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
            r#"globalThis.events=[];t.focus();globalThis.originalKeyboard=KeyboardEvent;globalThis.KeyboardEvent=function(){throw Error('author constructor')};for(const kind of ['keydown','keypress','keyup'])t.addEventListener(kind,e=>{if(!(e instanceof originalKeyboard)||!e.isTrusted||!e.composed||!e.bubbles)throw Error('native event');events.push(kind+':'+e.key)});t.addEventListener('keydown',e=>{if(e.key==='x')e.preventDefault()});"#,
        );
        assert!(dispatch(scope, &state, &input("😀", true)).unwrap());
        assert!(dispatch(scope, &state, &input("😀", false)).unwrap());
        assert!(
            eval(
                scope,
                "t.value==='😀' && events.join('|')==='keydown:😀|keypress:😀|keyup:😀'"
            )
            .is_true()
        );
        assert!(!dispatch(scope, &state, &input("x", true)).unwrap());
        assert!(eval(scope, "t.value==='😀'").is_true());
        assert!(dispatch(scope, &state, &input("Backspace", true)).unwrap());
        assert!(eval(scope, "t.value===''").is_true());
        eval(
            scope,
            "t.addEventListener('beforeinput',e=>e.preventDefault(),{once:true})",
        );
        dispatch(scope, &state, &input("a", true)).unwrap();
        assert!(eval(scope, "t.value===''").is_true());
        state.retire_execution();
        assert!(dispatch(scope, &state, &input("b", true)).is_err());
    }
    #[test]
    fn space_activates_button_on_release_and_tab_changes_native_focus() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<button id=a>A</button><button id=b>B</button>")
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
            "globalThis.clicks=0;a.addEventListener('click',e=>{if(!e.isTrusted)throw Error('click trust');clicks++});a.focus()",
        );
        dispatch(scope, &state, &input(" ", true)).unwrap();
        assert!(eval(scope, "clicks===0").is_true());
        dispatch(scope, &state, &input(" ", false)).unwrap();
        assert!(eval(scope, "clicks===1").is_true());
        dispatch(scope, &state, &input(" ", false)).unwrap();
        assert!(eval(scope, "clicks===1").is_true());
        dispatch(scope, &state, &input("Tab", true)).unwrap();
        assert!(eval(scope, "document.activeElement===b").is_true());
        let mut reverse = input("Tab", true);
        reverse.modifiers.shift = true;
        dispatch(scope, &state, &reverse).unwrap();
        assert!(eval(scope, "document.activeElement===a").is_true());
    }
    #[test]
    fn canceled_shortcut_never_obtains_clipboard_default() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t value=abc>")
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
            "t.focus();t.select();t.addEventListener('keydown',e=>e.preventDefault())",
        );
        let mut copy = input("c", true);
        copy.modifiers.control = true;
        // No backend installed: any attempted clipboard default would error.
        assert!(!dispatch(scope, &state, &copy).unwrap());
        assert!(eval(scope, "t.value==='abc'").is_true());
    }
}

fn temporal_key(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    key: &str,
    shift: bool,
    generation: u64,
    epoch: u64,
) -> Result<bool, V8BindingError> {
    use oryn_dom::TemporalAction;
    if !live(state, node, generation, epoch)
        || state.dom.borrow().focused() != Some(node)
        || !state.dom.borrow().is_temporal_control(node)
        || css_suppresses_focus_in_scope(scope, state, node)?
    {
        return Ok(false);
    }
    // Preserve established whole-value stepping until the user selects a field.
    if matches!(key, "ArrowUp" | "ArrowDown")
        && state
            .dom
            .borrow()
            .arena()
            .get(node)?
            .temporal_edit
            .is_none()
        && !state.dom.borrow().control_value(node)?.is_empty()
    {
        return Ok(false);
    }
    let digit = state.dom.borrow().temporal_digit(node, key)?;
    let mut action = match key {
        "ArrowLeft" => TemporalAction::Move(-1),
        "ArrowRight" => TemporalAction::Move(1),
        "Tab" => TemporalAction::Move(if shift { -1 } else { 1 }),
        "Home" => TemporalAction::First,
        "End" => TemporalAction::Last,
        "ArrowUp" => TemporalAction::Increment(1),
        "ArrowDown" => TemporalAction::Increment(-1),
        "Backspace" | "Delete" => TemporalAction::Clear,
        "-" | "/" | ":" | "." | "T" | " " => TemporalAction::Separator(key.chars().next().unwrap()),
        _ if digit.is_some() => TemporalAction::Digit(digit.unwrap()),
        _ => return Ok(printable(key)),
    };
    let editing = matches!(
        action,
        TemporalAction::Digit(_)
            | TemporalAction::Separator(_)
            | TemporalAction::Clear
            | TemporalAction::Increment(_)
    );
    if editing && !clipboard_actions::editable(state, node)? {
        return Ok(true);
    }
    let input_type = if matches!(action, TemporalAction::Clear) {
        "deleteContentBackward"
    } else if matches!(
        action,
        TemporalAction::Digit(_) | TemporalAction::Separator(_)
    ) {
        "insertText"
    } else {
        "insertReplacementText"
    };
    let data = matches!(
        action,
        TemporalAction::Digit(_) | TemporalAction::Separator(_)
    )
    .then_some(key);
    if editing
        && (!clipboard_actions::fire_editing(
            scope,
            state,
            node,
            "beforeinput",
            input_type,
            data,
            None,
        )? || !live(state, node, generation, epoch)
            || !clipboard_actions::editable(state, node)?
            || state.dom.borrow().focused() != Some(node)
            || !state.dom.borrow().is_temporal_control(node)
            || state.dom.borrow().is_inert(node)?
            || css_suppresses_focus_in_scope(scope, state, node)?)
    {
        return Ok(true);
    }
    let before = state.dom.borrow().control_edit_value(node)?;
    let selection = state.text_control_selections.borrow().get(&node).cloned();
    let all = selection.as_ref().is_some_and(|(start, end, _)| {
        *start == 0 && *end as usize == oryn_dom::dom_string::len(&before)
    });
    if all {
        if matches!(action, TemporalAction::Clear) {
            action = TemporalAction::ClearAll;
        } else if matches!(action, TemporalAction::Digit(_)) {
            state
                .dom
                .borrow_mut()
                .edit_temporal_control(node, TemporalAction::ClearAll)?;
        }
    }

    if state
        .dom
        .borrow()
        .arena()
        .get(node)?
        .temporal_edit
        .is_some()
        && let Some((start, _, _)) = &selection
    {
        state
            .dom
            .borrow_mut()
            .align_temporal_selection(node, *start)?;
    }
    let Some(outcome) = state.dom.borrow_mut().edit_temporal_control(node, action)? else {
        return Ok(true);
    };
    let after_selection = (outcome.selection.0, outcome.selection.1, "none".into());
    state
        .text_control_selections
        .borrow_mut()
        .insert(node, after_selection.clone());
    state
        .selection_revision
        .set(state.selection_revision.get().wrapping_add(1));
    state.control_modify_goal.borrow_mut().take();
    if outcome.changed {
        let after = state.dom.borrow().control_edit_value(node)?;
        record_control_edit(
            state,
            node,
            before,
            selection.unwrap_or((outcome.selection.0, outcome.selection.1, "none".into())),
            after,
            after_selection,
        );
        clipboard_actions::fire_editing(scope, state, node, "input", input_type, data, None)?;
    }
    Ok(key != "Tab" || outcome.moved)
}

pub(super) fn control_step(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    key: &str,
    mods: NativeKeyboardModifiers,
) -> Result<bool, V8BindingError> {
    let (select, kind, disabled) = {
        let dom = state.dom.borrow();
        (
            dom.is_element(node, "http://www.w3.org/1999/xhtml", "select"),
            dom.attribute_in_null_namespace(node, "type")?
                .unwrap_or_default()
                .to_ascii_lowercase(),
            dom.control_disabled(node)?,
        )
    };
    if disabled {
        return Ok(false);
    }
    if select {
        return select_step(scope, state, node, key, mods);
    }
    if mods.control || mods.meta || mods.alt {
        return Ok(false);
    }
    let generation = state.document_generation();
    let epoch = state.document_stream_epoch();
    let changed = if matches!(
        kind.as_str(),
        "number" | "range" | "date" | "month" | "week" | "time" | "datetime-local"
    ) && matches!(
        key,
        "ArrowUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
            | "Home"
            | "End"
            | "PageUp"
            | "PageDown"
    ) {
        if kind != "range" && !matches!(key, "ArrowUp" | "ArrowDown") {
            return Ok(false);
        }
        if kind != "range"
            && state
                .dom
                .borrow()
                .attribute_in_null_namespace(node, "readonly")?
                .is_some()
        {
            return Ok(true);
        }
        let before = state.dom.borrow().control_edit_value(node)?;
        let mut down = matches!(key, "ArrowDown" | "ArrowLeft");
        if kind == "range" && matches!(key, "ArrowLeft" | "ArrowRight") {
            refresh_layout_in_scope(scope, state)?;
            let rtl = editing_snapshot_property(
                &state.dom.borrow(),
                &state.layout.borrow(),
                node,
                "direction",
            )?
            .as_deref()
                == Some("rtl");
            if rtl {
                down = !down;
            }
        }
        if kind == "range" && matches!(key, "Home" | "End") {
            let bound = {
                let dom = state.dom.borrow();
                dom.attribute_in_null_namespace(node, if key == "Home" { "min" } else { "max" })?
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| value.is_finite())
                    .unwrap_or(if key == "Home" { 0. } else { 100. })
            };
            state
                .dom
                .borrow_mut()
                .set_control_value_as_number(node, bound)?;
        } else {
            state.dom.borrow_mut().step_control(
                node,
                if matches!(key, "PageUp" | "PageDown") {
                    10
                } else {
                    1
                },
                down || key == "PageDown",
            )?;
        }
        before != state.dom.borrow().control_edit_value(node)?
    } else {
        return Ok(false);
    };
    if changed {
        state.dom.borrow_mut().set_user_validity(node, true)?;
        for name in ["input", "change"] {
            if !live(state, node, generation, epoch) {
                break;
            }
            let event = create_event_object(scope, state, name, true, false)?;
            let id = event_id(scope, event)?;
            if let Some(record) = state.events.borrow_mut().get_mut(&id) {
                record.is_trusted = true;
                record.composed = name == "input";
            }
            dispatch_event(scope, state, node, event)?;
        }
    }
    Ok(true)
}
fn scroll_default(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    key: &str,
    mods: NativeKeyboardModifiers,
) -> Result<bool, V8BindingError> {
    if mods.control
        || mods.alt
        || mods.meta
        || clipboard_actions::editable(state, node)?
        || (key == " " && activatable(state, node, true)?)
    {
        return Ok(false);
    }
    refresh_layout_in_scope(scope, state)?;
    let viewport = state.layout.borrow().viewport;
    let Some((_, height)) = viewport else {
        return Ok(false);
    };
    let page = height * 0.875;
    let line = editing_snapshot_property(
        &state.dom.borrow(),
        &state.layout.borrow(),
        node,
        "line-height",
    )?
    .and_then(|value| {
        value
            .strip_suffix("px")
            .and_then(|value| value.parse::<f64>().ok())
    });
    let (x, y) = match key {
        "PageDown" => (0., page),
        "PageUp" => (0., -page),
        " " => (0., if mods.shift { -page } else { page }),
        "ArrowDown" => (0., line.unwrap_or(0.)),
        "ArrowUp" => (0., -line.unwrap_or(0.)),
        "ArrowRight" => (line.unwrap_or(0.), 0.),
        "ArrowLeft" => (-line.unwrap_or(0.), 0.),
        _ => return Ok(false),
    };
    apply_pixel_wheel(scope, state, node, x, y)?;
    Ok(true)
}

#[derive(Clone)]
struct ControlSnapshot {
    value: String,
    selection: (u32, u32, String),
}
#[derive(Clone)]
struct ControlUndo {
    before: ControlSnapshot,
    after: ControlSnapshot,
    generation: u64,
    epoch: u64,
    group: Option<Rc<ControlUndoGroup>>,
    document_unit: Option<Rc<()>>,
}
struct ControlUndoGroup {
    changes: Vec<(NodeId, ControlSnapshot, ControlSnapshot)>,
}
#[derive(Default)]
struct ControlHistory {
    undo: Vec<ControlUndo>,
    redo: Vec<ControlUndo>,
}
const MAX_CONTROL_UNDO_BYTES: usize = 1024 * 1024;
fn control_undo_bytes(entry: &ControlUndo) -> usize {
    let direct = entry
        .before
        .value
        .len()
        .saturating_add(entry.after.value.len());
    entry.group.as_ref().map_or(direct, |group| {
        group
            .changes
            .iter()
            .fold(direct, |bytes, (_, before, after)| {
                bytes
                    .saturating_add(before.value.len())
                    .saturating_add(after.value.len())
            })
    })
}
fn trim_control_histories(histories: &mut BTreeMap<NodeId, ControlHistory>) {
    for history in histories.values_mut() {
        while history.undo.len() > 100
            || history
                .undo
                .iter()
                .chain(history.redo.iter())
                .map(control_undo_bytes)
                .sum::<usize>()
                > MAX_CONTROL_UNDO_BYTES
        {
            if !history.undo.is_empty() {
                history.undo.remove(0);
            } else if !history.redo.is_empty() {
                history.redo.remove(0);
            } else {
                break;
            }
        }
    }
    while histories
        .values()
        .flat_map(|history| history.undo.iter().chain(history.redo.iter()))
        .map(control_undo_bytes)
        .sum::<usize>()
        > 4 * MAX_CONTROL_UNDO_BYTES
        || histories.len() > 1024
    {
        let Some(oldest) = histories.keys().next().copied() else {
            break;
        };
        histories.remove(&oldest);
    }
}

pub(super) fn record_control_edit(
    state: &V8BindingState,
    node: NodeId,
    before: String,
    before_selection: (u32, u32, String),
    after: String,
    after_selection: (u32, u32, String),
) {
    if before == after {
        return;
    }
    let mut histories = state.keyboard.control_history.borrow_mut();
    // Generational NodeIds are never recycled into another control's history.
    histories.retain(|node, _| state.dom.borrow().arena().get(*node).is_ok());
    let history = histories.entry(node).or_default();
    if history.undo.last().is_some_and(|entry| {
        entry.after.value != before
            || entry.generation != state.document_generation()
            || entry.epoch != state.document_stream_epoch()
    }) {
        history.undo.clear();
    }
    history.redo.clear();
    if before.len().saturating_add(after.len()) > MAX_CONTROL_UNDO_BYTES {
        history.undo.clear();
        return;
    }
    history.undo.push(ControlUndo {
        before: ControlSnapshot {
            value: before,
            selection: before_selection,
        },
        after: ControlSnapshot {
            value: after,
            selection: after_selection,
        },
        generation: state.document_generation(),
        epoch: state.document_stream_epoch(),
        group: None,
        document_unit: None,
    });
    trim_control_histories(&mut histories);
}
pub(super) fn undo_control(
    state: &V8BindingState,
    node: NodeId,
    redo: bool,
) -> Result<bool, V8BindingError> {
    let current = state.dom.borrow().control_edit_value(node)?;
    let mut histories = state.keyboard.control_history.borrow_mut();
    let Some(history) = histories.get_mut(&node) else {
        return Ok(false);
    };
    let source = if redo {
        &mut history.redo
    } else {
        &mut history.undo
    };
    let Some(entry) = source.last().cloned() else {
        return Ok(false);
    };
    if let Some(identity) = entry.document_unit.clone() {
        drop(histories);
        return undo_document_edit(state, redo, &identity);
    }
    if let Some(group) = entry.group.clone() {
        drop(histories);
        return undo_control_group(state, &group, redo, entry.generation, entry.epoch);
    }
    let expected = if redo { &entry.before } else { &entry.after };
    if current != expected.value || !live(state, node, entry.generation, entry.epoch) {
        history.undo.clear();
        history.redo.clear();
        return Ok(false);
    }
    let restore = if redo { &entry.after } else { &entry.before };
    {
        let mut dom = state.dom.borrow_mut();
        let sensitive = dom.arena().get(node)?.sensitive_value;
        dom.restore_control_edit_value(node, restore.value.clone(), sensitive)?;
        let record = dom.arena_mut().get_mut(node)?;
        record.user_edited_value = true;
        if record.pending_edit_value.is_none() {
            record.pending_edit_value = Some(current.clone());
        }
    }
    state
        .text_control_selections
        .borrow_mut()
        .insert(node, restore.selection.clone());
    state.control_modify_goal.borrow_mut().take();
    source.pop();
    if redo {
        history.undo.push(entry);
    } else {
        history.redo.push(entry);
    }
    Ok(true)
}

/// Two native controls share the same undo identity. Undo is admitted only
/// when every owner still has this group at the top and the expected value.
fn undo_control_group(
    state: &V8BindingState,
    group: &Rc<ControlUndoGroup>,
    redo: bool,
    generation: u64,
    epoch: u64,
) -> Result<bool, V8BindingError> {
    let mut histories = state.keyboard.control_history.borrow_mut();
    for (node, before, after) in &group.changes {
        let Some(history) = histories.get(node) else {
            return Ok(false);
        };
        let Some(entry) = (if redo {
            history.redo.last()
        } else {
            history.undo.last()
        }) else {
            return Ok(false);
        };
        if !entry
            .group
            .as_ref()
            .is_some_and(|candidate| Rc::ptr_eq(candidate, group))
            || !live(state, *node, generation, epoch)
            || state.dom.borrow().control_edit_value(*node)?
                != (if redo { &before.value } else { &after.value }).as_str()
        {
            return Ok(false);
        }
    }
    // All generational nodes and source values are checked before any write.
    // Native setters do not run author callbacks; rollback is never a script edit.
    let mut applied: Vec<(NodeId, ControlSnapshot)> = Vec::new();
    for (node, before, after) in &group.changes {
        let (restore, current) = if redo {
            (after, before)
        } else {
            (before, after)
        };
        let result = (|| {
            let mut dom = state.dom.borrow_mut();
            let sensitive = dom.arena().get(*node)?.sensitive_value;
            dom.restore_control_edit_value(*node, restore.value.clone(), sensitive)?;
            let record = dom.arena_mut().get_mut(*node)?;
            record.user_edited_value = true;
            if record.pending_edit_value.is_none() {
                record.pending_edit_value = Some(current.value.clone());
            }
            Ok::<_, V8BindingError>(())
        })();
        if let Err(error) = result {
            for (node, snapshot) in applied {
                let mut dom = state.dom.borrow_mut();
                let sensitive = dom.arena().get(node)?.sensitive_value;
                dom.restore_control_edit_value(node, snapshot.value, sensitive)?;
                state
                    .text_control_selections
                    .borrow_mut()
                    .insert(node, snapshot.selection);
            }
            return Err(error);
        }
        state
            .text_control_selections
            .borrow_mut()
            .insert(*node, restore.selection.clone());
        applied.push((*node, current.clone()));
    }
    for (node, _, _) in &group.changes {
        let history = histories
            .get_mut(node)
            .expect("prevalidated control undo owner");
        let entry = (if redo {
            &mut history.redo
        } else {
            &mut history.undo
        })
        .pop()
        .expect("prevalidated undo group");
        if redo {
            history.undo.push(entry);
        } else {
            history.redo.push(entry);
        }
    }
    state.control_modify_goal.borrow_mut().take();
    Ok(true)
}
type DocumentControlChange = ControlEdit;
pub(super) fn document_edit_budget(
    changes: &[DocumentControlChange],
) -> Result<(), V8BindingError> {
    if changes
        .iter()
        .fold(0usize, |bytes, (_, before, _, after, _)| {
            bytes
                .saturating_add(before.len())
                .saturating_add(after.len())
        })
        > MAX_CONTROL_UNDO_BYTES / 2
    {
        return Err(V8BindingError::Allocation(
            "compound document/control undo budget",
        ));
    }
    Ok(())
}
/// Segmented document undo already owns bounded native control inverses. Avoid
/// duplicating their payload in the independent control stack when that stack's
/// smaller budget cannot admit the complete group; never publish partial markers.
pub(super) fn segmented_document_changes(
    changes: Vec<DocumentControlChange>,
) -> Vec<DocumentControlChange> {
    if document_edit_budget(&changes).is_ok() {
        changes
    } else {
        Vec::new()
    }
}
fn reserve_compound_history(state: &V8BindingState, owners: &[NodeId]) {
    let mut histories = state.keyboard.control_history.borrow_mut();
    loop {
        let bytes = histories
            .values()
            .flat_map(|history| history.undo.iter().chain(history.redo.iter()))
            .map(control_undo_bytes)
            .sum::<usize>();
        if bytes <= 2 * MAX_CONTROL_UNDO_BYTES {
            break;
        }
        if let Some(victim) = histories
            .keys()
            .find(|node| !owners.contains(node))
            .copied()
        {
            histories.remove(&victim);
        } else {
            histories.clear();
            break;
        }
    }
}
pub(super) fn record_document_edit(
    state: &V8BindingState,
    identity: &Rc<()>,
    changes: Vec<DocumentControlChange>,
) {
    reserve_compound_history(
        state,
        &changes.iter().map(|change| change.0).collect::<Vec<_>>(),
    );
    for (node, before, before_selection, after, after_selection) in changes {
        record_control_edit(
            state,
            node,
            before,
            before_selection,
            after,
            after_selection,
        );
        if let Some(entry) = state
            .keyboard
            .control_history
            .borrow_mut()
            .get_mut(&node)
            .and_then(|history| history.undo.last_mut())
        {
            entry.document_unit = Some(identity.clone());
        }
    }
}
pub(super) fn document_undo_owner_current(
    state: &V8BindingState,
    identity: &Rc<()>,
    owners: &[NodeId],
    redo: bool,
) -> bool {
    let histories = state.keyboard.control_history.borrow();
    owners.iter().all(|node| {
        histories
            .get(node)
            .and_then(|history| {
                if redo {
                    history.redo.last()
                } else {
                    history.undo.last()
                }
            })
            .is_some_and(|entry| {
                entry
                    .document_unit
                    .as_ref()
                    .is_some_and(|token| Rc::ptr_eq(token, identity))
                    && live(state, *node, entry.generation, entry.epoch)
            })
    })
}
pub(super) fn can_document_undo(
    state: &V8BindingState,
    identity: &Rc<()>,
    owners: &[NodeId],
    redo: bool,
) -> Result<bool, V8BindingError> {
    let histories = state.keyboard.control_history.borrow();
    for node in owners {
        let Some(history) = histories.get(node) else {
            return Ok(false);
        };
        let Some(entry) = (if redo {
            history.redo.last()
        } else {
            history.undo.last()
        }) else {
            return Ok(false);
        };
        if !entry
            .document_unit
            .as_ref()
            .is_some_and(|token| Rc::ptr_eq(token, identity))
            || !live(state, *node, entry.generation, entry.epoch)
            || state.dom.borrow().control_edit_value(*node)?
                != (if redo {
                    &entry.before.value
                } else {
                    &entry.after.value
                })
                .as_str()
        {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(super) fn finish_document_undo(state: &V8BindingState, owners: &[NodeId], redo: bool) {
    let mut histories = state.keyboard.control_history.borrow_mut();
    for node in owners {
        let history = histories
            .get_mut(node)
            .expect("checked compound undo owner");
        let entry = (if redo {
            &mut history.redo
        } else {
            &mut history.undo
        })
        .pop()
        .expect("checked compound undo marker");
        if redo {
            history.undo.push(entry);
        } else {
            history.redo.push(entry);
        }
    }
}

pub(super) fn record_control_move(
    state: &V8BindingState,
    changes: Vec<ControlEdit>,
) -> Result<(), V8BindingError> {
    let bytes = changes
        .iter()
        .fold(0usize, |bytes, (_, before, _, after, _)| {
            bytes
                .saturating_add(before.len())
                .saturating_add(after.len())
        });
    if bytes > MAX_CONTROL_UNDO_BYTES / 2 {
        return Err(V8BindingError::Allocation("compound control undo budget"));
    }
    reserve_compound_history(
        state,
        &changes.iter().map(|change| change.0).collect::<Vec<_>>(),
    );
    let group = Rc::new(ControlUndoGroup {
        changes: changes
            .into_iter()
            .filter(|(_, before, _, after, _)| before != after)
            .map(|(node, before, before_selection, after, after_selection)| {
                (
                    node,
                    ControlSnapshot {
                        value: before,
                        selection: before_selection,
                    },
                    ControlSnapshot {
                        value: after,
                        selection: after_selection,
                    },
                )
            })
            .collect(),
    });
    for (node, before, after) in &group.changes {
        record_control_edit(
            state,
            *node,
            before.value.clone(),
            before.selection.clone(),
            after.value.clone(),
            after.selection.clone(),
        );
        if let Some(entry) = state
            .keyboard
            .control_history
            .borrow_mut()
            .get_mut(node)
            .and_then(|history| history.undo.last_mut())
        {
            entry.group = Some(group.clone());
        }
    }
    trim_control_histories(&mut state.keyboard.control_history.borrow_mut());
    Ok(())
}
pub(super) fn finish_control_move_selection(state: &V8BindingState, node: NodeId, position: u32) {
    let selection = (position, position, "none".into());
    state
        .text_control_selections
        .borrow_mut()
        .insert(node, selection.clone());
    if let Some(entry) = state
        .keyboard
        .control_history
        .borrow_mut()
        .get_mut(&node)
        .and_then(|history| history.undo.last_mut())
    {
        entry.after.selection = selection;
    }
}

pub(super) fn invalidate_control_history(state: &V8BindingState, node: NodeId) {
    state.keyboard.control_history.borrow_mut().remove(&node);
}

pub(super) fn set_platform(state: &V8BindingState, platform: NativeKeyboardPlatform) {
    state.keyboard.platform.set(platform);
}
fn select_step(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    key: &str,
    mods: NativeKeyboardModifiers,
) -> Result<bool, V8BindingError> {
    if mods.alt
        || mods.meta
        || (!matches!(key, "ArrowUp" | "ArrowDown" | "Home" | "End" | " ")
            && (!printable(key) || mods.control))
    {
        return Ok(false);
    }
    let (options, available, multiple, current, anchor, before) = {
        let dom = state.dom.borrow();
        let options = dom.select_options(node)?;
        let available = options
            .iter()
            .copied()
            .filter(|option| !dom.option_disabled(*option).unwrap_or(true))
            .collect::<Vec<_>>();
        let before = options
            .iter()
            .copied()
            .filter(|option| dom.option_selected(*option).unwrap_or(false))
            .collect::<BTreeSet<_>>();
        let previous = state.keyboard.select_focus.borrow().get(&node).copied();
        let current = previous
            .map(|(_, focus)| focus)
            .filter(|focus| options.contains(focus))
            .or_else(|| {
                options
                    .iter()
                    .copied()
                    .find(|option| before.contains(option))
            });
        let anchor = previous
            .map(|(anchor, _)| anchor)
            .filter(|anchor| options.contains(anchor))
            .or(current);
        let multiple = dom.attribute_in_null_namespace(node, "multiple")?.is_some();
        (options, available, multiple, current, anchor, before)
    };
    if available.is_empty() {
        return Ok(true);
    }
    if key == " " && (!multiple || !mods.control) {
        return Ok(true);
    }
    let current_index =
        current.and_then(|current| available.iter().position(|node| *node == current));
    let typed = printable(key) && key != " ";
    let typed_target = if typed {
        let now = std::time::Instant::now();
        let lower = key.to_lowercase();
        let previous = state
            .keyboard
            .select_search
            .borrow()
            .get(&node)
            .cloned()
            .filter(|(_, time)| {
                now.saturating_duration_since(*time) < std::time::Duration::from_secs(1)
            });
        let cycling = previous
            .as_ref()
            .is_some_and(|(prefix, _)| prefix == &lower);
        let prefix = if cycling {
            lower.clone()
        } else {
            previous
                .as_ref()
                .map_or(lower.clone(), |(prefix, _)| format!("{prefix}{lower}"))
        };
        state
            .keyboard
            .select_search
            .borrow_mut()
            .insert(node, (prefix.clone(), now));
        let start = if previous.is_some() && !cycling {
            current_index.unwrap_or(0)
        } else {
            current_index.map_or(0, |index| (index + 1) % available.len())
        };
        let dom = state.dom.borrow();
        (0..available.len())
            .map(|offset| available[(start + offset) % available.len()])
            .find(|option| {
                dom.option_label(*option)
                    .is_ok_and(|label| label.trim_start().to_lowercase().starts_with(&prefix))
            })
    } else {
        None
    };
    if typed && typed_target.is_none() {
        return Ok(true);
    }
    let next = if let Some(target) = typed_target {
        target
    } else {
        match key {
            "Home" => available.first().copied(),
            "End" => available.last().copied(),
            "ArrowUp" => current_index
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| available.get(index).copied())
                .or(current),
            "ArrowDown" => available
                .get(current_index.map_or(0, |index| index + 1))
                .copied()
                .or(current),
            _ => current,
        }
        .unwrap_or(available[0])
    };
    let anchor = if mods.shift {
        anchor.unwrap_or(next)
    } else {
        next
    };
    state
        .keyboard
        .select_focus
        .borrow_mut()
        .insert(node, (anchor, next));
    if state
        .dom
        .borrow()
        .top_layer()
        .any(|entry| entry.node == node && entry.kind == oryn_dom::TopLayerKind::SelectPicker)
    {
        state
            .dom
            .borrow_mut()
            .set_select_picker_active(node, Some(next))?;
        state.layout_cache_key.borrow_mut().take();
        scroll_element_into_view_in_scope(scope, state, next, "nearest", "nearest", false)?;
        return Ok(true);
    }
    let mut selected = if multiple && mods.control {
        before.clone()
    } else {
        BTreeSet::new()
    };
    if key == " " {
        if !selected.remove(&next) {
            selected.insert(next);
        }
    } else if mods.shift && multiple {
        let first = options.iter().position(|node| *node == anchor).unwrap_or(0);
        let last = options
            .iter()
            .position(|node| *node == next)
            .unwrap_or(first);
        for option in &options[first.min(last)..=first.max(last)] {
            if available.contains(option) {
                selected.insert(*option);
            }
        }
    } else if !mods.control || !multiple {
        selected.insert(next);
    }
    if selected == before {
        return Ok(true);
    }
    {
        let mut dom = state.dom.borrow_mut();
        for option in options {
            dom.set_option_selected(option, selected.contains(&option))?;
        }
    }
    state.dom.borrow_mut().set_user_validity(node, true)?;
    let generation = state.document_generation();
    let epoch = state.document_stream_epoch();
    for name in ["input", "change"] {
        if !live(state, node, generation, epoch) {
            break;
        }
        let event = create_event_object(scope, state, name, true, false)?;
        let id = event_id(scope, event)?;
        if let Some(record) = state.events.borrow_mut().get_mut(&id) {
            record.is_trusted = true;
            record.composed = name == "input";
        }
        dispatch_event(scope, state, node, event)?;
    }
    Ok(true)
}
#[cfg(test)]
mod defaults_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, code).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    fn key(key: &str) -> NativeKeyboardInput {
        NativeKeyboardInput {
            key: key.encode_utf16().collect(),
            down: true,
            ..Default::default()
        }
    }
    #[test]
    fn text_system_delete_preserves_intent_and_canceled_selection() {
        use crate::composition::{TextCommand, TextUnit};
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t value='one two'>")
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
            "t.focus();t.setSelectionRange(7,7);globalThis.kinds=[];t.addEventListener('beforeinput',e=>{kinds.push([e.inputType,e.isTrusted,t.selectionStart,t.selectionEnd]);e.preventDefault()},{once:true})",
        );
        let target = state.dom.borrow().focused().unwrap();
        text_system_command(
            scope,
            &state,
            target,
            TextCommand::Delete {
                backward: true,
                unit: TextUnit::Word,
            },
        )
        .unwrap();
        assert!(eval(scope,r#"t.value==='one two'&&t.selectionStart===7&&t.selectionEnd===7&&JSON.stringify(kinds)==='[["deleteWordBackward",true,7,7]]'"#).is_true());
        eval(
            scope,
            "t.addEventListener('input',e=>kinds.push([e.inputType,e.isTrusted]))",
        );
        text_system_command(
            scope,
            &state,
            target,
            TextCommand::Delete {
                backward: true,
                unit: TextUnit::Word,
            },
        )
        .unwrap();
        assert!(eval(scope,"t.value==='one '&&t.selectionStart===4&&kinds[1][0]==='deleteWordBackward'&&kinds[1][1]").is_true());
    }
    #[test]
    fn text_system_soft_line_delete_uses_shaped_wrap_and_original_undo_selection() {
        use crate::composition::{TextCommand, TextUnit};
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<textarea id=t>abcdefghijkl</textarea>")
                .into_runtime()
                .unwrap(),
        ));
        let target = dom.borrow().find_by_id("t").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "t.focus();t.setSelectionRange(6,6);globalThis.kind='';t.addEventListener('input',e=>kind=e.inputType)",
        );
        let matrix = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        state.set_layout(BindingLayout {
            text_clusters: BTreeMap::from([(
                node_key(target),
                (0..12)
                    .map(|offset| oryn_common::v4::TextClusterGeometry {
                        layout_node: node_key(target),
                        start: offset,
                        end: offset + 1,
                        rect: oryn_common::v4::CssRect {
                            x: (offset % 4) as f64 * 10.,
                            y: (offset / 4) as f64 * 20.,
                            width: 10.,
                            height: 16.,
                        },
                        matrix,
                        line: offset / 4,
                        rtl: false,
                    })
                    .collect(),
            )]),
            ..Default::default()
        });
        text_system_command(
            scope,
            &state,
            target,
            TextCommand::Delete {
                backward: true,
                unit: TextUnit::LineBoundary,
            },
        )
        .unwrap();
        assert!(
            eval(
                scope,
                "t.value==='abcdghijkl'&&t.selectionStart===4&&kind==='deleteSoftLineBackward'"
            )
            .is_true()
        );
        eval(scope, "document.execCommand('undo')");
        assert!(
            eval(
                scope,
                "t.value==='abcdefghijkl'&&t.selectionStart===6&&t.selectionEnd===6"
            )
            .is_true()
        );
    }
    #[test]
    fn text_system_paragraph_commands_use_hard_boundaries_and_native_event_intent() {
        use crate::composition::{TextCommand, TextDirection, TextUnit};
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<textarea id=t>one\ntwo three</textarea>")
                .into_runtime()
                .unwrap(),
        ));
        let target = dom.borrow().find_by_id("t").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "t.focus();t.setSelectionRange(7,7);globalThis.kind='';t.addEventListener('input',e=>kind=e.inputType)",
        );
        text_system_command(
            scope,
            &state,
            target,
            TextCommand::Delete {
                backward: true,
                unit: TextUnit::ParagraphBoundary,
            },
        )
        .unwrap();
        assert!(
            eval(
                scope,
                r#"t.value==='one\n three'&&t.selectionStart===4&&kind==='deleteHardLineBackward'"#
            )
            .is_true()
        );
        text_system_command(
            scope,
            &state,
            target,
            TextCommand::Move {
                direction: TextDirection::Forward,
                unit: TextUnit::ParagraphBoundary,
                extend: true,
            },
        )
        .unwrap();
        assert!(eval(scope, "t.selectionStart===4&&t.selectionEnd===10").is_true());
    }
    #[test]
    fn user_control_edits_share_undo_but_script_assignments_invalidate_it() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<form id=f><input id=t value=seed></form>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        set_platform(&state, NativeKeyboardPlatform::Other);
        eval(
            scope,
            "t.focus();t.setSelectionRange(4,4);globalThis.historyEvents=[];t.addEventListener('input',e=>historyEvents.push(e.inputType))",
        );
        dispatch(scope, &state, &key("😀")).unwrap();
        assert!(
            eval(
                scope,
                "t.value==='seed😀' && document.queryCommandEnabled('undo')"
            )
            .is_true()
        );
        let mut undo = key("z");
        undo.modifiers.control = true;
        dispatch(scope, &state, &undo).unwrap();
        assert!(
            eval(
                scope,
                "t.value==='seed' && t.selectionStart===4 && historyEvents.includes('historyUndo')"
            )
            .is_true()
        );
        let mut redo = key("y");
        redo.modifiers.control = true;
        dispatch(scope, &state, &redo).unwrap();
        assert!(eval(scope, "t.value==='seed😀' && t.selectionStart===6").is_true());
        eval(scope, "t.value='script'");
        dispatch(scope, &state, &undo).unwrap();
        assert!(eval(scope, "t.value==='script'").is_true());
        dispatch(scope, &state, &key("x")).unwrap();
        eval(scope, "f.reset()");
        dispatch(scope, &state, &undo).unwrap();
        assert!(eval(scope, "t.value==='seed'").is_true());
    }
    #[test]
    fn range_and_select_defaults_use_native_bounds_and_disabled_options() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<input id=r type=range min=2 max=8 step=2 value=4><select id=s><option>A</option><option disabled>B</option><option>C</option></select>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "r.focus();globalThis.changes=0;r.addEventListener('change',e=>{if(!e.isTrusted)throw Error('trust');changes++})",
        );
        dispatch(scope, &state, &key("ArrowRight")).unwrap();
        assert!(eval(scope, "r.value==='6' && changes===1").is_true());
        dispatch(scope, &state, &key("End")).unwrap();
        assert!(eval(scope, "r.value==='8'").is_true());
        dispatch(scope, &state, &key("Home")).unwrap();
        assert!(eval(scope, "r.value==='2'").is_true());
        eval(scope, "s.focus()");
        dispatch(scope, &state, &key("ArrowDown")).unwrap();
        assert!(eval(scope, "s.selectedIndex===2").is_true());
        dispatch(scope, &state, &key("ArrowUp")).unwrap();
        assert!(eval(scope, "s.selectedIndex===0").is_true());
        eval(
            scope,
            "s.addEventListener('keypress',e=>e.preventDefault(),{once:true})",
        );
        assert!(!dispatch(scope, &state, &key("C")).unwrap());
        assert!(eval(scope, "s.selectedIndex===0").is_true());
        dispatch(scope, &state, &key("C")).unwrap();
        assert!(eval(scope, "s.selectedIndex===2").is_true());
    }
    #[test]
    fn native_platform_setting_controls_command_modifiers() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t value=abcdef>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        set_platform(&state, NativeKeyboardPlatform::MacOs);
        eval(
            scope,
            "t.focus();t.setSelectionRange(6,6);globalThis.navigator={platform:'Windows'}",
        );
        let mut all = key("a");
        all.modifiers.meta = true;
        dispatch(scope, &state, &all).unwrap();
        assert!(eval(scope, "t.selectionStart===0 && t.selectionEnd===6").is_true());
    }
    #[test]
    fn native_word_delete_multi_select_and_date_steps_keep_actual_state() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<input id=t value='one two'><select id=s multiple><option selected>A</option><option disabled>B</option><option>C</option><option>D</option></select><input id=d type=date value=2024-02-28>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        set_platform(&state, NativeKeyboardPlatform::Other);
        eval(
            scope,
            "t.focus();t.setSelectionRange(7,7);globalThis.kind='';t.addEventListener('beforeinput',e=>kind=e.inputType)",
        );
        let mut delete = key("Backspace");
        delete.modifiers.control = true;
        dispatch(scope, &state, &delete).unwrap();
        assert!(eval(scope, "t.value==='one ' && kind==='deleteWordBackward'").is_true());
        eval(scope, "s.focus()");
        let mut extend = key("ArrowDown");
        extend.modifiers.shift = true;
        dispatch(scope, &state, &extend).unwrap();
        assert!(eval(scope,"s.options[0].selected && !s.options[1].selected && s.options[2].selected && !s.options[3].selected").is_true());
        let mut next = key("ArrowDown");
        next.modifiers.control = true;
        dispatch(scope, &state, &next).unwrap();
        assert!(eval(scope, "!s.options[3].selected").is_true());
        let mut toggle = key(" ");
        toggle.modifiers.control = true;
        dispatch(scope, &state, &toggle).unwrap();
        assert!(eval(scope, "s.options[3].selected").is_true());
        eval(scope, "d.focus()");
        dispatch(scope, &state, &key("ArrowUp")).unwrap();
        assert!(eval(scope, "d.value==='2024-02-29'").is_true());
        dispatch(scope, &state, &key("ArrowUp")).unwrap();
        assert!(eval(scope, "d.value==='2024-03-01'").is_true());
    }
}

pub(super) fn control_history_available(state: &V8BindingState, node: NodeId, redo: bool) -> bool {
    let histories = state.keyboard.control_history.borrow();
    let Some(history) = histories.get(&node) else {
        return false;
    };
    let Some(entry) = (if redo {
        history.redo.last()
    } else {
        history.undo.last()
    }) else {
        return false;
    };
    if let Some(identity) = &entry.document_unit {
        let undo = state.undo_history.borrow();
        let Some(unit) = (if redo {
            undo.redo.back()
        } else {
            undo.undo.back()
        }) else {
            return false;
        };
        if !Rc::ptr_eq(&unit.identity, identity) {
            return false;
        }
        for owner in &unit.control_owners {
            let Some(history) = histories.get(owner) else {
                return false;
            };
            let Some(marker) = (if redo {
                history.redo.last()
            } else {
                history.undo.last()
            }) else {
                return false;
            };
            if !marker
                .document_unit
                .as_ref()
                .is_some_and(|token| Rc::ptr_eq(token, identity))
                || !live(state, *owner, marker.generation, marker.epoch)
                || !state
                    .dom
                    .borrow()
                    .control_edit_value(*owner)
                    .is_ok_and(|value| {
                        value.as_str()
                            == if redo {
                                marker.before.value.as_str()
                            } else {
                                marker.after.value.as_str()
                            }
                    })
            {
                return false;
            }
        }
    }
    if let Some(group) = &entry.group {
        for (owner, before, after) in &group.changes {
            let Some(history) = histories.get(owner) else {
                return false;
            };
            let Some(marker) = (if redo {
                history.redo.last()
            } else {
                history.undo.last()
            }) else {
                return false;
            };
            if !marker
                .group
                .as_ref()
                .is_some_and(|candidate| Rc::ptr_eq(candidate, group))
                || !live(state, *owner, marker.generation, marker.epoch)
                || !state
                    .dom
                    .borrow()
                    .control_edit_value(*owner)
                    .is_ok_and(|value| {
                        value.as_str()
                            == if redo {
                                before.value.as_str()
                            } else {
                                after.value.as_str()
                            }
                    })
            {
                return false;
            }
        }
    }
    live(state, node, entry.generation, entry.epoch)
        && state
            .dom
            .borrow()
            .control_edit_value(node)
            .is_ok_and(|value| {
                value.as_str()
                    == if redo {
                        entry.before.value.as_str()
                    } else {
                        entry.after.value.as_str()
                    }
            })
}

/// Native lifecycle cancellation disarms Space before synthetic key releases;
/// undo history and authored selection remain owned by their live controls.
pub fn cancel_native_keyboard_defaults(state: &V8BindingState) {
    state.keyboard.space_activation.set(None);
    state.keyboard.select_search.borrow_mut().clear();
}

pub fn native_keyboard_was_deferred(state: &V8BindingState) -> bool {
    state.keyboard.deferred.get()
}

pub fn native_keyboard_deferred_target(state: &V8BindingState) -> Option<NodeId> {
    state.keyboard.deferred_target.get()
}

/// Apply an AppKit command only after an accepted native keydown continuation.
/// No second keydown is generated; editing retains native beforeinput/input.
#[cfg(test)]
pub(super) fn text_system_command(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    command: crate::composition::TextCommand,
) -> Result<(), V8BindingError> {
    let mut receipt = TextCommandReceipt::new(state.clone(), target, command)?;
    if !receipt.poll(scope)? {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(())
}
