//! Native select picker lifecycle. DOM selectedness remains the single value
//! authority; the layout service projects a separate anchored picker box.
use super::*;
const HTML: &str = "http://www.w3.org/1999/xhtml";
pub(super) fn show(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    select: NodeId,
) -> Result<(), V8BindingError> {
    let error = |name: &str| V8BindingError::Unsupported {
        interface: "HTMLSelectElement".into(),
        member: name.into(),
    };
    if state.dom.borrow().control_disabled(select)? {
        return Err(error("InvalidStateError"));
    }
    let window = state
        .global_object
        .to_local(scope)
        .ok_or_else(|| error("InvalidStateError"))?;
    let top = native_window_top(scope, window);
    let top_state = binding_state_for_receiver(scope, top).ok_or_else(|| error("SecurityError"))?;
    if !Rc::ptr_eq(state, &top_state) {
        let origin = current_platform_origin(state);
        if origin == "null" || origin != current_platform_origin(&top_state) {
            return Err(error("SecurityError"));
        }
    }
    if !state.has_transient_user_activation() {
        return Err(error("NotAllowedError"));
    }
    refresh_layout(state)?;
    if !state
        .layout
        .borrow()
        .geometry
        .get(&node_key(select))
        .is_some_and(|geometry| matches!(geometry, BindingGeometry::Available(_)))
    {
        return Err(error("NotSupportedError"));
    }
    if !state.consume_transient_user_activation() {
        return Err(error("NotAllowedError"));
    }
    open(state, select)
}
pub(super) fn open(state: &V8BindingState, select: NodeId) -> Result<(), V8BindingError> {
    let mut dom = state.dom.borrow_mut();
    if !dom.is_element(select, HTML, "select")
        || !dom.is_connected(select)?
        || dom.control_disabled(select)?
        || dom.is_inert(select)?
    {
        return Ok(());
    }
    if dom
        .attribute_in_null_namespace(select, "multiple")?
        .is_some()
        || dom.select_display_size(select)? > 1
    {
        return Ok(());
    }
    let previous = dom
        .top_layer()
        .filter(|entry| {
            matches!(
                entry.kind,
                oryn_dom::TopLayerKind::SelectPicker | oryn_dom::TopLayerKind::TemporalPicker
            ) && entry.node != select
        })
        .map(|entry| entry.node)
        .collect::<Vec<_>>();
    for node in previous {
        dom.remove_from_top_layer(node);
    }
    dom.add_to_top_layer(select, oryn_dom::TopLayerKind::SelectPicker)?;
    let available = dom
        .select_options(select)?
        .into_iter()
        .filter(|option| !dom.option_disabled(*option).unwrap_or(true))
        .collect::<Vec<_>>();
    let active = available
        .iter()
        .copied()
        .find(|option| dom.option_selected(*option).unwrap_or(false))
        .or_else(|| available.first().copied());
    dom.set_select_picker_active(select, active)?;
    if let Some(active) = active {
        state
            .keyboard
            .select_focus
            .borrow_mut()
            .insert(select, (active, active));
    }
    drop(dom);
    state.layout_cache_key.borrow_mut().take();
    Ok(())
}
pub(super) fn prepare_keyboard(
    state: &Rc<V8BindingState>,
    target: NodeId,
    key: &str,
    alt: bool,
) -> Result<(bool, Option<Job>), V8BindingError> {
    if !state.dom.borrow().is_element(target, HTML, "select") {
        return Ok((false, None));
    }
    let opened =
        state.dom.borrow().top_layer().any(|entry| {
            entry.node == target && entry.kind == oryn_dom::TopLayerKind::SelectPicker
        });
    if opened && (key == "Enter" || key == " " || key == "Tab" || (alt && key == "ArrowUp")) {
        let active = state.dom.borrow().select_picker_active(target)?;
        return Ok((
            key != "Tab",
            Some(Job::new(state, target, Mutation::Commit(active))?),
        ));
    }
    if !opened && (key == " " || (alt && key == "ArrowDown")) {
        open(state, target)?;
        return Ok((true, None));
    }
    Ok((false, None))
}
pub(super) fn keyboard(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    key: &str,
    alt: bool,
) -> Result<bool, V8BindingError> {
    let (handled, job) = prepare_keyboard(state, target, key, alt)?;
    if let Some(mut job) = job {
        if !job.poll(scope)? {
            return Err(V8BindingError::JavaScriptException);
        }
    }
    Ok(handled)
}
#[derive(Clone, Copy)]
enum Mutation {
    Commit(Option<NodeId>),
    Listbox {
        option: NodeId,
        ctrl: bool,
        shift: bool,
    },
}
pub(super) struct Job {
    owner: Rc<V8BindingState>,
    target: oryn_dom::TrackedNodeLease,
    generation: u64,
    epoch: u64,
    mutation: Mutation,
    phase: u8,
    event: Option<retained_dom_event::Event>,
    changed: bool,
}
impl Job {
    fn new(
        state: &Rc<V8BindingState>,
        select: NodeId,
        mutation: Mutation,
    ) -> Result<Self, V8BindingError> {
        Ok(Self {
            owner: state.clone(),
            target: oryn_dom::TrackedNodeLease::new(&state.dom, select)?,
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            mutation,
            phase: 0,
            event: None,
            changed: false,
        })
    }
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        loop {
            if self.phase == 3 {
                return Ok(true);
            }
            let state = self.owner.clone();
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            let (dom, select) = self.target.current();
            if !Rc::ptr_eq(&dom, &state.dom)
                || state.execution_retired()
                || state.document_generation() != self.generation
                || state.document_stream_epoch() != self.epoch
                || !state.dom.borrow().is_connected(select)?
            {
                self.cancel();
                return Ok(true);
            }
            if self.phase == 0 {
                self.changed = match self.mutation {
                    Mutation::Commit(option) => {
                        let mut dom = state.dom.borrow_mut();
                        let option = option.filter(|option| {
                            dom.select_options(select)
                                .is_ok_and(|options| options.contains(option))
                                && !dom.option_disabled(*option).unwrap_or(true)
                        });
                        let changed = if let Some(option) = option {
                            if dom.control_disabled(select)? || dom.is_inert(select)? {
                                false
                            } else {
                                let changed = !dom.option_selected(option)?;
                                dom.set_option_selected(option, true)?;
                                changed
                            }
                        } else {
                            false
                        };
                        dom.remove_from_top_layer(select);
                        changed
                    }
                    Mutation::Listbox {
                        option,
                        ctrl,
                        shift,
                    } => listbox_mutation(&state, select, option, ctrl, shift)?,
                };
                if self.changed {
                    state.dom.borrow_mut().set_user_validity(select, true)?;
                }
                self.phase = 1;
                state.layout_cache_key.borrow_mut().take();
            } else {
                if !self.changed {
                    self.phase = 3;
                    continue;
                }
                if self.event.is_none() {
                    let name = if self.phase == 1 { "input" } else { "change" };
                    let event = create_event_object(scope, &state, name, true, false)?;
                    let id = event_id(scope, event)?;
                    {
                        let mut events = state.events.borrow_mut();
                        let record = events
                            .get_mut(&id)
                            .ok_or(V8BindingError::Allocation("picker event"))?;
                        record.is_trusted = true;
                        record.composed = name == "input";
                    }
                    self.event = Some(retained_dom_event::Event::new(
                        scope, &state, select, name, event,
                    )?);
                }
                if self.event.as_mut().unwrap().poll(scope, &state)?.is_none() {
                    return Ok(false);
                }
                self.event = None;
                self.phase += 1;
            }
        }
    }
    pub(super) fn cancel(&mut self) {
        if let Some(event) = self.event.take() {
            event.cancel(&self.owner)
        }
        self.phase = 3;
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel()
    }
}
fn listbox_job(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    select: NodeId,
    option: NodeId,
    event: v8::Local<v8::Object>,
) -> Result<Job, V8BindingError> {
    let ctrl = native_input_property(scope, event, "MouseEvent", "ctrlKey")
        .is_ok_and(|value| value.is_true())
        || native_input_property(scope, event, "MouseEvent", "metaKey")
            .is_ok_and(|value| value.is_true());
    let shift = native_input_property(scope, event, "MouseEvent", "shiftKey")
        .is_ok_and(|value| value.is_true());
    Job::new(
        state,
        select,
        Mutation::Listbox {
            option,
            ctrl,
            shift,
        },
    )
}
fn listbox_mutation(
    state: &Rc<V8BindingState>,
    select: NodeId,
    option: NodeId,
    ctrl: bool,
    shift: bool,
) -> Result<bool, V8BindingError> {
    let changed = {
        let mut dom = state.dom.borrow_mut();
        if dom.control_disabled(select)? || dom.option_disabled(option)? || dom.is_inert(select)? {
            return Ok(false);
        }
        let options = dom.select_options(select)?;
        let multiple = dom
            .attribute_in_null_namespace(select, "multiple")?
            .is_some();
        let before = options
            .iter()
            .copied()
            .filter(|option| dom.option_selected(*option).unwrap_or(false))
            .collect::<BTreeSet<_>>();
        let anchor = state
            .keyboard
            .select_focus
            .borrow()
            .get(&select)
            .map(|entry| entry.0)
            .filter(|anchor| options.contains(anchor))
            .unwrap_or(option);
        let mut selected = if multiple && ctrl {
            before.clone()
        } else {
            BTreeSet::new()
        };
        if multiple && shift {
            let first = options.iter().position(|value| *value == anchor).unwrap();
            let last = options.iter().position(|value| *value == option).unwrap();
            for value in &options[first.min(last)..=first.max(last)] {
                if !dom.option_disabled(*value)? {
                    selected.insert(*value);
                }
            }
        } else if !multiple || !ctrl || !selected.remove(&option) {
            selected.insert(option);
        }
        state
            .keyboard
            .select_focus
            .borrow_mut()
            .insert(select, (if shift { anchor } else { option }, option));
        if selected == before {
            false
        } else {
            for option in options {
                dom.set_option_selected(option, selected.contains(&option))?;
            }
            true
        }
    };
    state.layout_cache_key.borrow_mut().take();
    Ok(changed)
}
pub(super) fn prepare_click(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    event: v8::Local<v8::Object>,
) -> Result<(bool, Option<Job>), V8BindingError> {
    let id = event_id(scope, event)?;
    if !state
        .events
        .borrow()
        .get(&id)
        .is_some_and(|event| event.is_trusted)
    {
        return Ok((false, None));
    }
    let (select, option) = {
        let dom = state.dom.borrow();
        let mut cursor = Some(target);
        let mut option = None;
        let mut select = None;
        while let Some(node) = cursor {
            if dom.is_element(node, HTML, "option") {
                option = Some(node);
            }
            if dom.is_element(node, HTML, "select") {
                select = Some(node);
                break;
            }
            cursor = dom.parent(node)?;
        }
        (select, option)
    };
    let Some(select) = select else {
        return Ok((false, None));
    };
    let opened =
        state.dom.borrow().top_layer().any(|entry| {
            entry.node == select && entry.kind == oryn_dom::TopLayerKind::SelectPicker
        });
    if let Some(option) = option {
        if !opened {
            let listbox = {
                let dom = state.dom.borrow();
                dom.attribute_in_null_namespace(select, "multiple")?
                    .is_some()
                    || dom.select_display_size(select)? > 1
            };
            if !listbox {
                return Ok((false, None));
            }
            return Ok((
                true,
                Some(listbox_job(scope, state, select, option, event)?),
            ));
        }
        return Ok((
            true,
            Some(Job::new(state, select, Mutation::Commit(Some(option)))?),
        ));
    }
    if opened {
        state.dom.borrow_mut().remove_from_top_layer(select);
        state.layout_cache_key.borrow_mut().take();
    } else {
        open(state, select)?;
    }
    Ok((true, None))
}
pub(super) fn click(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    event: v8::Local<v8::Object>,
) -> Result<bool, V8BindingError> {
    let (handled, job) = prepare_click(scope, state, target, event)?;
    if let Some(mut job) = job {
        if !job.poll(scope)? {
            return Err(V8BindingError::JavaScriptException);
        }
    }
    Ok(handled)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_picker_commit_retains_input_suffix_and_one_change() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<select id=s><option id=a>One</option><option id=b>Two</option></select>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state =
            install_generated_bindings(scope, context, dom.clone(), RealmId(245), 1).unwrap();
        let select = dom.borrow().find_by_id("s").unwrap();
        let second = dom.borrow().find_by_id("b").unwrap();
        let source=v8::String::new(scope,"s.focus();globalThis.trace=[];s.addEventListener('input',e=>{globalThis.original=e;trace.push('first')});s.addEventListener('input',e=>{if(e!==original)throw Error('event replaced');trace.push('second')});s.addEventListener('change',()=>trace.push('change'))").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        open(&state, select).unwrap();
        dom.borrow_mut()
            .set_select_picker_active(select, Some(second))
            .unwrap();
        let input = native_keyboard::NativeKeyboardInput {
            key: "Enter".encode_utf16().collect(),
            down: true,
            ..Default::default()
        };
        let mut receipt = native_keyboard::begin_receipt(state.clone(), &input).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(native_keyboard::poll_receipt(scope, &mut receipt).is_err());
        assert!(receipt.retry_safe());
        assert!(dom.borrow().option_selected(second).unwrap());
        state.set_execution_suspended(true);
        assert!(
            native_keyboard::poll_receipt(scope, &mut receipt)
                .unwrap()
                .is_none()
        );
        state.set_execution_suspended(false);
        assert_eq!(
            native_keyboard::poll_receipt(scope, &mut receipt).unwrap(),
            Some(true)
        );
        assert_eq!(
            native_keyboard::poll_receipt(scope, &mut receipt).unwrap(),
            Some(true)
        );
        let source = v8::String::new(scope, "trace.join()==='first,second,change'").unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn native_picker_highlight_commits_on_enter_and_escape_preserves_value() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<select id='s'><option id='a'>One</option><option disabled>Disabled</option><option id='b'>Two</option></select>").into_runtime().unwrap()));
        let select = dom.borrow().find_by_id("s").unwrap();
        let first = dom.borrow().find_by_id("a").unwrap();
        let second = dom.borrow().find_by_id("b").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        dom.borrow_mut().focus(Some(select)).unwrap();
        open(&state, select).unwrap();
        let key = |text: &str| native_keyboard::NativeKeyboardInput {
            key: text.encode_utf16().collect(),
            down: true,
            ..Default::default()
        };
        native_keyboard::dispatch(scope, &state, &key("ArrowDown")).unwrap();
        assert_eq!(
            dom.borrow().select_picker_active(select).unwrap(),
            Some(second)
        );
        assert!(dom.borrow().option_selected(first).unwrap());
        assert!(!dom.borrow().option_selected(second).unwrap());
        native_keyboard::dispatch(scope, &state, &key("Escape")).unwrap();
        assert!(dom.borrow().option_selected(first).unwrap());
        open(&state, select).unwrap();
        native_keyboard::dispatch(scope, &state, &key("ArrowDown")).unwrap();
        native_keyboard::dispatch(scope, &state, &key("Enter")).unwrap();
        assert!(dom.borrow().option_selected(second).unwrap());
        assert!(!dom.borrow().top_layer().any(|entry| entry.node == select));
    }

    #[test]
    fn native_picker_lifecycle_preserves_dom_ancestry_and_ignores_synthetic_clicks() {
        super::super::tests::initialize_v8();
        let parsed = oryn_dom::html::parse(
            "<body><select id='s'><option id='a'>One</option><option id='b'>Two</option></select></body>",
        );
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let root = dom.borrow().document();
        let select = dom.borrow().get_element_by_id(root, "s").unwrap().unwrap();
        let option = dom.borrow().get_element_by_id(root, "b").unwrap().unwrap();
        open(&state, select).unwrap();
        assert_eq!(dom.borrow().parent(option).unwrap(), Some(select));
        assert!(dom.borrow().top_layer().any(
            |entry| entry.node == select && entry.kind == oryn_dom::TopLayerKind::SelectPicker
        ));
        let event = create_event_object(scope, &state, "click", true, true).unwrap();
        assert!(!click(scope, &state, option, event).unwrap());
        assert!(!dom.borrow().option_selected(option).unwrap());
        let event = create_event_object(scope, &state, "click", true, true).unwrap();
        let id = event_id(scope, event).unwrap();
        state.events.borrow_mut().get_mut(&id).unwrap().is_trusted = true;
        assert!(click(scope, &state, option, event).unwrap());
        assert!(dom.borrow().option_selected(option).unwrap());
        assert_eq!(dom.borrow().parent(option).unwrap(), Some(select));
        assert!(!dom.borrow().top_layer().any(|entry| entry.node == select));
    }
}
