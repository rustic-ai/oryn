//! Command invoker reflection, native event payload and default activation.
use super::*;
const BUILTINS: &[&str] = &[
    "toggle-popover",
    "show-popover",
    "hide-popover",
    "show-modal",
    "close",
    "request-close",
    "toggle-fullscreen",
    "request-fullscreen",
    "exit-fullscreen",
    "play-pause",
    "play",
    "pause",
    "mute",
    "unmute",
    "toggle-muted",
    "seek-forward",
    "seek-backward",
    "toggle",
    "open",
    "step-up",
    "step-down",
    "page-up",
    "page-down",
    "page-left",
    "page-right",
    "page-block-start",
    "page-block-end",
    "page-inline-start",
    "page-inline-end",
];
pub(super) fn value(state: &V8BindingState, node: NodeId) -> Result<String, V8BindingError> {
    let dom = state.dom.borrow();
    let value = dom
        .attribute_in_null_namespace(node, "command")?
        .unwrap_or("");
    if oryn_dom::dom_string::units(value).starts_with(&[45, 45]) {
        return Ok(value.into());
    }
    let value = value.to_ascii_lowercase();
    Ok(if BUILTINS.contains(&value.as_str()) {
        value
    } else {
        String::new()
    })
}
fn private<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    field: &str,
) -> Option<v8::Local<'s, v8::Private>> {
    let name = v8::String::new(scope, &format!("oryn.command.{field}"))?;
    Some(v8::Private::for_api(scope, Some(name)))
}
pub(super) fn event_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<v8::Object>,
    field: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = private(scope, field)?;
    event
        .get_private(scope, key)
        .filter(|value| !value.is_undefined())
}
pub(super) fn event_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    event_value(scope, event, "source")
}
pub(super) fn set_event_source(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    value: v8::Local<v8::Value>,
) -> bool {
    let Some(key) = private(scope, "source") else {
        return false;
    };
    if event
        .get_private(scope, key)
        .is_none_or(|value| value.is_undefined())
    {
        return false;
    }
    event.set_private(scope, key, value) == Some(true)
}
fn payload(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    command: v8::Local<v8::Value>,
    source: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    for (field, value) in [("command", command), ("source", source)] {
        let key = private(scope, field).ok_or(V8BindingError::Allocation("command event field"))?;
        event
            .set_private(scope, key, value)
            .filter(|set| *set)
            .ok_or(V8BindingError::JavaScriptException)?;
    }
    Ok(())
}
pub(super) fn initialize_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Result<(), V8BindingError> {
    let command =
        event_init_member(scope, init, "command").ok_or(V8BindingError::JavaScriptException)?;
    let command = if command.is_undefined() {
        v8_string(scope, "")?
    } else {
        command
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .into()
    };
    let source =
        event_init_member(scope, init, "source").ok_or(V8BindingError::JavaScriptException)?;
    let source = if source.is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        let identity = node_identity::argument(scope, source).map_err(|_| {
            throw_type_error(scope, "CommandEvent source must be an Element");
            V8BindingError::JavaScriptException
        })?;
        let Some((owner, node)) = identity else {
            throw_type_error(scope, "CommandEvent source must be an Element");
            return Err(V8BindingError::JavaScriptException);
        };
        if !matches!(
            owner.dom.borrow().arena().get(node)?.kind,
            NodeKind::Element { .. }
        ) {
            throw_type_error(scope, "CommandEvent source must be an Element");
            return Err(V8BindingError::JavaScriptException);
        }
        source
    };
    payload(scope, event, command, source)
}
pub(super) fn target(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    source: NodeId,
) -> Result<Option<NodeId>, V8BindingError> {
    aria::reconcile_node(scope, state, source)?;
    Ok(state.dom.borrow().command_for_element(source)?)
}
pub(super) fn set_target(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    source: NodeId,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    set_reference_target(scope, state, source, value, "commandfor", "commandForElement")
}
pub(super) fn set_reference_target(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    source: NodeId,
    value: v8::Local<v8::Value>,
    attribute: &str,
    member: &str,
) -> Result<(), V8BindingError> {
    let object = wrap_node(scope, state, source)?;
    let key = aria::migration_key(scope, attribute, true)?;
    if value.is_null_or_undefined() {
        state
            .dom
            .borrow_mut()
            .set_accessibility_references(source, attribute, false, None)?;
        object.delete_private(scope, key);
        return Ok(());
    }
    let Some((owner, node)) = node_identity::argument(scope, value)? else {
        throw_type_error(scope, &format!("{member} requires an Element"));
        return Err(V8BindingError::JavaScriptException);
    };
    if !matches!(
        owner.dom.borrow().arena().get(node)?.kind,
        NodeKind::Element { .. }
    ) {
        throw_type_error(scope, &format!("{member} requires an Element"));
        return Err(V8BindingError::JavaScriptException);
    }
    let track = {
        let mut tracked = state.aria.migrated.borrow_mut();
        tracked.retain(|weak| !weak.is_empty());
        let missing = !tracked.iter().any(|weak| {
            weak.to_local(scope)
                .is_some_and(|previous| previous.strict_equals(object.into()))
        });
        if missing {
            tracked
                .try_reserve(1)
                .map_err(|_| V8BindingError::Allocation("command relationship custody"))?;
        }
        missing
    };
    let values = v8::Array::new_with_elements(scope, &[value]);
    object
        .set_private(scope, key, values.into())
        .filter(|set| *set)
        .ok_or(V8BindingError::JavaScriptException)?;
    state.dom.borrow_mut().set_accessibility_references(
        source,
        attribute,
        false,
        Some(if Rc::ptr_eq(&owner.dom, &state.dom) {
            vec![node]
        } else {
            Vec::new()
        }),
    )?;
    if track {
        state
            .aria
            .migrated
            .borrow_mut()
            .push(v8::Weak::new(scope, object));
    }
    Ok(())
}

/// Owns the command event and its default independently of click dispatch.
/// The target and command are frozen before author command listeners run.
pub(super) struct Job {
    owner: Rc<V8BindingState>,
    source: NodeId,
    target: NodeId,
    command: String,
    epoch: (NodeId, u64, u64),
    _leases: Vec<oryn_dom::TrackedNodeLease>,
    event: Option<retained_dom_event::Event>,
    popover: Option<top_layer::PendingToggle>,
    dialog: Option<dialog::Job>,
    phase: u8,
    toggle: Option<(&'static str, &'static str)>,
}
fn supports(state: &V8BindingState, node: NodeId, command: &str) -> Result<bool, V8BindingError> {
    if command.starts_with("--") {
        return Ok(true);
    }
    let dom = state.dom.borrow();
    let html = |name| dom.is_element(node, "http://www.w3.org/1999/xhtml", name);
    Ok(match command {
        "toggle-popover" | "show-popover" | "hide-popover" => {
            dom.attribute_in_null_namespace(node, "popover")?.is_some()
        }
        "show-modal" | "request-close" => html("dialog"),
        "close" => html("dialog") || html("details"),
        "toggle" | "open" => html("details"),
        "play-pause" | "play" | "pause" | "mute" | "unmute" | "toggle-muted" | "seek-forward"
        | "seek-backward" => html("audio") || html("video"),
        "step-up" | "step-down" => {
            html("input")
                && dom
                    .attribute_in_null_namespace(node, "type")?
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("number"))
        }
        "toggle-fullscreen" | "request-fullscreen" | "exit-fullscreen" => true,
        "page-up" | "page-down" | "page-left" | "page-right" | "page-block-start"
        | "page-block-end" | "page-inline-start" | "page-inline-end" => true,
        _ => false,
    })
}
fn page_scroll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    command: &str,
) -> Result<bool, V8BindingError> {
    refresh_layout_in_scope(scope, state)?;
    let delta = {
        let layout = state.layout.borrow();
        let dom = state.dom.borrow();
        let key = node_key(node);
        let viewport = is_scrolling_document_root(&dom, &layout, state.document_root(), node);
        let size = if viewport {
            layout
                .viewport_scroll
                .as_ref()
                .map(|view| (view.scroll.scrollport.width, view.scroll.scrollport.height))
                .or(layout.viewport)
        } else {
            layout.hit_projections.get(&key).map(|projection| {
                projection.scroll.as_ref().map_or(
                    (
                        projection.box_model.padding_box.width,
                        projection.box_model.padding_box.height,
                    ),
                    |scroll| (scroll.scrollport.width, scroll.scrollport.height),
                )
            })
        };
        let Some((width, height)) = size else {
            return Ok(false);
        };
        let width = width * 0.875;
        let height = height * 0.875;
        let style = layout.styles.get(&key);
        let writing = style
            .and_then(|style| style.get("writing-mode"))
            .map(String::as_str)
            .unwrap_or("horizontal-tb");
        let rtl = style
            .and_then(|style| style.get("direction"))
            .is_some_and(|value| value == "rtl");
        let block = if writing.ends_with("-rl") {
            (-width, 0.)
        } else if writing == "vertical-lr" || writing == "sideways-lr" {
            (width, 0.)
        } else {
            (0., height)
        };
        let inline = if writing == "horizontal-tb" {
            (if rtl { -width } else { width }, 0.)
        } else {
            (
                0.,
                if rtl ^ (writing == "sideways-lr") {
                    -height
                } else {
                    height
                },
            )
        };
        match command {
            "page-up" => (0., -height),
            "page-down" => (0., height),
            "page-left" => (-width, 0.),
            "page-right" => (width, 0.),
            "page-block-end" => block,
            "page-block-start" => (-block.0, -block.1),
            "page-inline-end" => inline,
            "page-inline-start" => (-inline.0, -inline.1),
            _ => return Ok(false),
        }
    };
    let bounds = programmatic_scroll_bounds_in_layout(
        &state.dom.borrow(),
        &state.layout.borrow(),
        state.document_root(),
        node,
    );
    let offset_node = scroll_offset_node(state, node);
    let previous = state
        .scroll_offsets
        .borrow()
        .get(&offset_node)
        .copied()
        .unwrap_or_default();
    let next = (
        (previous.0 + delta.0).clamp(bounds.min.0, bounds.max.0),
        (previous.1 + delta.1).clamp(bounds.min.1, bounds.max.1),
    );
    if next == previous {
        return Ok(false);
    }
    state.scroll_offsets.borrow_mut().insert(offset_node, next);
    state.layout_cache_key.borrow_mut().take();
    Ok(true)
}
impl Job {
    pub(super) fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        source: NodeId,
    ) -> Result<Option<Self>, V8BindingError> {
        {
            let dom = state.dom.borrow();
            if !dom.is_element(source, "http://www.w3.org/1999/xhtml", "button")
                || dom.control_disabled(source)?
                || !dom.is_connected(source)?
                || dom.is_inert(source)?
            {
                return Ok(None);
            }
            // With a form owner only the explicit Button state invokes commands.
            // Auto is neither a submit button nor a command invoker in this case.
            if dom.form_owner(source)?.is_some()
                && !dom
                    .attribute_in_null_namespace(source, "type")?
                    .is_some_and(|value| value.eq_ignore_ascii_case("button"))
            {
                return Ok(None);
            }
        }
        let command = value(state, source)?;
        if command.is_empty() {
            return Ok(None);
        }
        let Some(target) = target(scope, state, source)? else {
            return Ok(None);
        };
        if !supports(state, target, &command)? {
            return Ok(None);
        }
        let leases = vec![
            oryn_dom::TrackedNodeLease::new(&state.dom, source)?,
            oryn_dom::TrackedNodeLease::new(&state.dom, target)?,
        ];
        let event = create_event_object_with_interface(
            scope,
            state,
            "CommandEvent",
            "command",
            false,
            true,
        )?;
        let command_value = v8_string(scope, &command)?;
        let source_value = wrap_node(scope, state, source)?;
        payload(scope, event, command_value, source_value.into())?;
        let id = event_id(scope, event)?;
        {
            let mut events = state.events.borrow_mut();
            let record = events
                .get_mut(&id)
                .ok_or(V8BindingError::Allocation("command event"))?;
            record.is_trusted = true;
            record.composed = true;
        }
        let event = retained_dom_event::Event::new(scope, state, target, "command", event)?;
        Ok(Some(Self {
            owner: state.clone(),
            source,
            target,
            command,
            epoch: (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
            ),
            _leases: leases,
            event: Some(event),
            popover: None,
            dialog: None,
            phase: 0,
            toggle: None,
        }))
    }
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        loop {
            if self.phase == 9 {
                return Ok(true);
            }
            if self.owner.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            if self.owner.execution_retired()
                || self.epoch
                    != (
                        self.owner.document_root(),
                        self.owner.document_generation(),
                        self.owner.document_stream_epoch(),
                    )
                || self._leases.iter().any(|lease| {
                    let (dom, node) = lease.current();
                    !Rc::ptr_eq(&dom, &self.owner.dom)
                        || self.owner.dom.borrow().node_document(node).ok().flatten()
                            != Some(self.epoch.0)
                })
            {
                self.cancel();
                return Ok(true);
            }
            match self.phase {
                0 => {
                    let Some(accepted) = self.event.as_mut().unwrap().poll(scope, &self.owner)?
                    else {
                        return Ok(false);
                    };
                    self.event = None;
                    if !accepted {
                        self.phase = 9;
                        return Ok(true);
                    }
                    self.phase = 1;
                }
                1 => {
                    if self.command.starts_with("--") {
                        self.phase = 9;
                        return Ok(true);
                    }
                    if matches!(
                        self.command.as_str(),
                        "toggle-popover" | "show-popover" | "hide-popover"
                    ) {
                        if !self.owner.dom.borrow().is_connected(self.target)?
                            || self
                                .owner
                                .dom
                                .borrow()
                                .attribute_in_null_namespace(self.target, "popover")?
                                .is_none()
                        {
                            self.phase = 9;
                            return Ok(true);
                        }
                        let force = match self.command.as_str() {
                            "show-popover" => Some(true),
                            "hide-popover" => Some(false),
                            _ => None,
                        };
                        self.popover = Some(top_layer::PendingToggle::new(
                            &self.owner,
                            self.target,
                            force,
                            Some(self.source),
                        )?);
                        self.phase = 2;
                        continue;
                    }
                    if !supports(&self.owner, self.target, &self.command)? {
                        self.phase = 9;
                        return Ok(true);
                    }
                    if self.owner.dom.borrow().is_element(
                        self.target,
                        "http://www.w3.org/1999/xhtml",
                        "dialog",
                    ) && matches!(
                        self.command.as_str(),
                        "show-modal" | "close" | "request-close"
                    ) {
                        let value = self
                            .owner
                            .dom
                            .borrow()
                            .attribute_in_null_namespace(self.source, "value")?
                            .map(str::to_owned);
                        let action = match self.command.as_str() {
                            "show-modal" => dialog::Action::ShowModal,
                            "request-close" => dialog::Action::RequestClose,
                            _ => dialog::Action::Close,
                        };
                        self.dialog = Some(dialog::Job::new(
                            &self.owner,
                            self.target,
                            action,
                            value,
                            Some(self.source),
                        )?);
                        self.phase = 5;
                        continue;
                    }
                    if matches!(self.command.as_str(), "toggle" | "open" | "close")
                        && self.owner.dom.borrow().is_element(
                            self.target,
                            "http://www.w3.org/1999/xhtml",
                            "details",
                        )
                    {
                        let was = self
                            .owner
                            .dom
                            .borrow()
                            .attribute_in_null_namespace(self.target, "open")?
                            .is_some();
                        let show = match self.command.as_str() {
                            "open" => true,
                            "close" => false,
                            _ => !was,
                        };
                        if show == was {
                            self.phase = 9;
                            return Ok(true);
                        }
                        if show {
                            self.owner.dom.borrow_mut().set_attribute_ns(
                                self.target,
                                None,
                                "open",
                                "",
                            )?;
                        } else {
                            self.owner.dom.borrow_mut().remove_attribute_ns(
                                self.target,
                                None,
                                "open",
                            )?;
                        }
                        self.toggle = Some((
                            if was { "open" } else { "closed" },
                            if show { "open" } else { "closed" },
                        ));
                        self.phase = 3;
                        continue;
                    }
                    if matches!(self.command.as_str(), "step-up" | "step-down") {
                        self.owner.dom.borrow_mut().step_control(
                            self.target,
                            1,
                            self.command == "step-down",
                        )?;
                        self.phase = 9;
                        return Ok(true);
                    }
                    if self.command.starts_with("page-") {
                        if page_scroll(scope, &self.owner, self.target, &self.command)? {
                            self.phase = 4;
                            continue;
                        }
                        self.phase = 9;
                        return Ok(true);
                    }
                    if fullscreen::command(scope, &self.owner, self.target, &self.command)? {
                        self.phase = 9;
                        return Ok(true);
                    }
                    media_element::command(scope, &self.owner, self.target, &self.command)?;
                    self.phase = 9;
                    return Ok(true);
                }
                2 => {
                    if !self.popover.as_mut().unwrap().resume(scope, &self.owner)? {
                        return Ok(false);
                    }
                    self.popover = None;
                    self.phase = 9;
                }
                3 => {
                    let (old, new) = self.toggle.unwrap();
                    top_layer::queue(scope, &self.owner, self.target, old, new, Some(self.source))?;
                    self.toggle = None;
                    self.phase = 9;
                }
                4 => {
                    queue_scroll_events(&self.owner, self.target)?;
                    self.phase = 9;
                }
                5 => {
                    if !self.dialog.as_mut().unwrap().poll(scope)? {
                        return Ok(false);
                    }
                    self.dialog = None;
                    self.phase = 9;
                }
                _ => unreachable!(),
            }
        }
    }
    pub(super) fn cancel(&mut self) {
        if let Some(event) = self.event.take() {
            event.cancel(&self.owner)
        }
        if let Some(mut popover) = self.popover.take() {
            popover.release(&self.owner)
        }
        self.dialog = None;
        self.phase = 9;
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel()
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
    fn command_suffix_keeps_frozen_target_payload_and_cancellation() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<button id=b commandfor=t command=--Custom></button><div id=t></div><div id=other></div>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(228), 1).unwrap();
        eval(
            scope,
            "globalThis.log=[];t.addEventListener('command',e=>{globalThis.original=e;log.push('first');b.command='--Changed';b.commandForElement=other},{once:true});t.addEventListener('command',e=>{log.push(e===original&&e.source===b&&e.command==='--Custom'&&e.isTrusted&&!e.bubbles&&e.composed?'second':'wrong');e.preventDefault()});other.oncommand=()=>log.push('wrong-target')",
        );
        let button = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "b")
            .unwrap()
            .unwrap();
        let mut job = Job::new(scope, &state, button).unwrap().unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(job.poll(scope).is_err());
        assert!(eval(scope, "log.join()==='first'").is_true());
        assert!(job.poll(scope).unwrap());
        assert!(job.poll(scope).unwrap());
        assert!(eval(scope, "log.join()==='first,second'").is_true());
    }
    #[test]
    fn only_explicit_button_state_invokes_with_form_owner() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<form><button id=b commandfor=t command=--custom></button></form><div id=t></div>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(229), 1).unwrap();
        let button = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "b")
            .unwrap()
            .unwrap();
        for kind in ["", "invalid", "submit", "reset"] {
            state
                .dom
                .borrow_mut()
                .set_attribute(button, "type", kind)
                .unwrap();
            assert!(Job::new(scope, &state, button).unwrap().is_none());
        }
        state
            .dom
            .borrow_mut()
            .set_attribute(button, "type", "button")
            .unwrap();
        assert!(Job::new(scope, &state, button).unwrap().is_some());
    }
}
