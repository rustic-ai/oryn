//! Native HTML activation shared by dispatchEvent, device clicks and click().
use super::*;
#[path = "retained_activation_finish.rs"]
mod retained;
pub(super) use retained::Finish;

#[derive(Clone)]
enum Previous {
    Checkbox {
        checked: bool,
        indeterminate: bool,
    },
    Radio {
        selected: Option<NodeId>,
        changed: bool,
    },
    Label,
    Submit,
    Link,
}

#[derive(Clone)]
pub(super) struct Activation {
    lease: Rc<oryn_dom::TrackedNodeLease>,
    selected: Option<Rc<oryn_dom::TrackedNodeLease>>,
    object: v8::Global<v8::Object>,
    previous: Previous,
}
impl Activation {
    pub(super) fn link_target(
        &self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<(Rc<V8BindingState>, NodeId)>, V8BindingError> {
        if !matches!(self.previous, Previous::Link) {
            return Ok(None);
        }
        let object = v8::Local::new(scope, &self.object);
        let resolved = node_identity::argument(scope, object.into())?;
        let (owner, node) = self.lease.current();
        Ok(resolved.filter(|(state, target)| Rc::ptr_eq(&owner, &state.dom) && node == *target))
    }
    pub(super) fn cancel_native(&self) {
        let (owner, node) = self.lease.current();
        let selected = self.selected.as_ref().and_then(|lease| {
            let (dom, node) = lease.current();
            Rc::ptr_eq(&dom, &owner).then_some(node)
        });
        let mut dom = owner.borrow_mut();
        match self.previous {
            Previous::Checkbox {
                checked,
                indeterminate,
            } => {
                if dom
                    .attribute_in_null_namespace(node, "type")
                    .ok()
                    .flatten()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("checkbox"))
                {
                    let _ = dom.set_checkedness(node, checked);
                    let _ = dom.set_indeterminate(node, indeterminate);
                }
            }
            Previous::Radio { .. } if dom.is_radio(node) => {
                let selected = selected.filter(|selected| {
                    *selected == node || dom.same_radio_group(node, *selected).unwrap_or(false)
                });
                let _ = dom.set_checkedness(selected.unwrap_or(node), selected.is_some());
            }
            _ => {}
        }
    }
}

fn retain(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    previous: Previous,
) -> Result<Option<Activation>, V8BindingError> {
    let lease = Rc::new(oryn_dom::TrackedNodeLease::new(&state.dom, node)?);
    let selected = match &previous {
        Previous::Radio {
            selected: Some(node),
            ..
        } => Some(Rc::new(oryn_dom::TrackedNodeLease::new(&state.dom, *node)?)),
        _ => None,
    };
    let object = wrap_node(scope, state, node)?;
    Ok(Some(Activation {
        lease,
        selected,
        object: v8::Global::new(scope, object),
        previous,
    }))
}

pub(super) fn prepare_auxiliary(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    object: v8::Local<v8::Object>,
) -> Result<Option<Activation>, V8BindingError> {
    let id = event_id(scope, object)?;
    let event_owner = binding_state_for_receiver(scope, object)
        .ok_or(V8BindingError::Allocation("auxclick event owner"))?;
    let (bubbles, composed) = {
        let records = event_owner.events.borrow();
        let event = records
            .get(&id)
            .ok_or(V8BindingError::Allocation("auxclick event"))?;
        if event.event_type != "auxclick" || !event.is_trusted {
            return Ok(None);
        }
        (event.bubbles, event.composed)
    };
    if mouse_payload(scope, object).is_none()
        || native_input_property(scope, object, "MouseEvent", "button")?.integer_value(scope)
            != Some(1)
    {
        return Ok(None);
    }
    let related = native_event_related_node(scope, state, object);
    let dom = state.dom.borrow();
    if related.is_some_and(|related| {
        related != target && dom.retarget_node(related, Some(target)).ok() == Some(target)
    }) {
        return Ok(None);
    }
    let root = dom.control_tree_root(target)?;
    let mut cursor = Some(target);
    while let Some(node) = cursor {
        if dom.is_element(node, "http://www.w3.org/1999/xhtml", "a")
            || dom.is_element(node, "http://www.w3.org/1999/xhtml", "area")
            || dom.is_element(node, "http://www.w3.org/2000/svg", "a")
        {
            drop(dom);
            return retain(scope, state, node, Previous::Link);
        }
        if dom.is_interactive_content(node)
            || !bubbles
            || (!composed
                && node == root
                && matches!(dom.arena().get(node)?.kind, NodeKind::ShadowRoot { .. }))
        {
            break;
        }
        let parent = dom.assigned_slot(node, false)?.or(dom.parent(node)?);
        if let (Some(parent), Some(related)) = (parent, related)
            && related != target
            && dom.retarget_node(target, Some(parent))?
                == dom.retarget_node(related, Some(parent))?
        {
            break;
        }
        cursor = parent;
    }
    Ok(None)
}

#[cfg(test)]
thread_local! {static FAIL_PREACTIVATION:Cell<bool>=const{Cell::new(false)};}
#[cfg(test)]
fn fail_preactivation_allocation() -> Result<(), V8BindingError> {
    if FAIL_PREACTIVATION.with(|slot| slot.replace(false)) {
        Err(V8BindingError::Allocation(
            "injected activation custody allocation",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn mouse_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    ui_event_property(scope, object, "UIEvent", "detail").ok()?;
    let payload = object
        .get_internal_field(scope, 2)
        .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())?;
    let kind = payload.get_index(scope, 5)?.to_rust_string_lossy(scope);
    matches!(kind.as_str(), "MouseEvent" | "PointerEvent" | "WheelEvent").then_some(payload)
}

pub(super) fn prepare(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    object: v8::Local<v8::Object>,
    id: u64,
) -> Result<Option<Activation>, V8BindingError> {
    let event_state = binding_state_for_receiver(scope, object)
        .ok_or(V8BindingError::Allocation("event owner"))?;
    let (bubbles, composed) = {
        let events = event_state.events.borrow();
        let event = &events[&id];
        if event.event_type != "click" {
            return Ok(None);
        }
        (event.bubbles, event.composed)
    };
    if mouse_payload(scope, object).is_none() {
        return Ok(None);
    }
    let related_node = native_event_related_node(scope, state, object);
    let dom = state.dom.borrow_mut();
    if let Some(related) = related_node
        && related != target
        && dom.retarget_node(related, Some(target))? == target
    {
        return Ok(None);
    }
    let origin_root = dom.control_tree_root(target)?;
    let mut cursor = Some(target);
    while let Some(node) = cursor {
        if dom.is_element(node, "http://www.w3.org/1999/xhtml", "button")
            || validation::is_submit_button(&dom, node)?
        {
            drop(dom);
            return retain(scope, state, node, Previous::Submit);
        }
        if dom.is_element(node, "http://www.w3.org/1999/xhtml", "input")
            && !dom
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("hidden"))
        {
            let previous = match dom
                .attribute_in_null_namespace(node, "type")?
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "checkbox" => {
                    let checked = dom.checked(node)?;
                    let indeterminate = dom.arena().get(node)?.indeterminate;
                    Previous::Checkbox {
                        checked,
                        indeterminate,
                    }
                }
                "radio" => {
                    let changed = !dom.checked(node)?;
                    let mut selected = None;
                    for (peer, record) in dom.arena().iter() {
                        if record.checked() && (peer == node || dom.same_radio_group(node, peer)?) {
                            selected = Some(peer);
                            break;
                        }
                    }
                    Previous::Radio { selected, changed }
                }
                // The activation target is chosen before dispatch, but its
                // activation behavior observes the type after click handlers.
                _ => Previous::Submit,
            };
            drop(dom);
            // All fallible wrapper/lease allocation precedes legacy checkedness
            // publication. Failure cannot leave an unowned preactivation toggle.
            let activation = retain(scope, state, node, previous)?;
            #[cfg(test)]
            fail_preactivation_allocation()?;
            if let Some(activation) = activation.as_ref() {
                let mut dom = state.dom.borrow_mut();
                match &activation.previous {
                    Previous::Checkbox { checked, .. } => {
                        dom.set_checked(node, !*checked)?;
                        dom.set_indeterminate(node, false)?;
                    }
                    Previous::Radio { .. } => dom.set_checked(node, true)?,
                    _ => {}
                }
            }
            return Ok(activation);
        }
        if dom.is_element(node, "http://www.w3.org/1999/xhtml", "label") {
            drop(dom);
            return retain(scope, state, node, Previous::Label);
        }
        if dom.is_element(node, "http://www.w3.org/1999/xhtml", "a")
            || dom.is_element(node, "http://www.w3.org/1999/xhtml", "area")
            || dom.is_element(node, "http://www.w3.org/2000/svg", "a")
        {
            drop(dom);
            return retain(scope, state, node, Previous::Link);
        }
        if dom.is_interactive_content(node) || !bubbles {
            break;
        }
        if !composed
            && node == origin_root
            && matches!(dom.arena().get(node)?.kind, NodeKind::ShadowRoot { .. })
        {
            break;
        }
        let parent = dom.assigned_slot(node, false)?.or(dom.parent(node)?);
        if let (Some(parent), Some(related)) = (parent, related_node)
            && related != target
            && dom.retarget_node(target, Some(parent))?
                == dom.retarget_node(related, Some(parent))?
        {
            break;
        }
        cursor = parent;
    }
    Ok(None)
}

fn finish_legacy(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    activation: Option<Activation>,
    target: NodeId,
    source: v8::Local<v8::Object>,
    accepted: bool,
) -> Result<(), V8BindingError> {
    if accepted && permission_prompt::click(scope, state, target, source)? {
        return Ok(());
    }
    if accepted && file_picker::click(scope, state, target, source)? {
        return Ok(());
    }
    if accepted && temporal_picker::click(scope, state, target, source)? {
        return Ok(());
    }
    if accepted && select_picker::click(scope, state, target, source)? {
        return Ok(());
    }
    let Some(activation) = activation else {
        return Ok(());
    };
    let object = v8::Local::new(scope, &activation.object);
    let (owner, node) = node_identity::argument(scope, object.into())?
        .ok_or(V8BindingError::Allocation("activation node owner"))?;
    let (target_dom, target_node) = activation.lease.current();
    if target_node != node || !Rc::ptr_eq(&target_dom, &owner.dom) {
        return Err(V8BindingError::Allocation("activation lease owner"));
    }
    let same_target_arena = Rc::ptr_eq(&state.dom, &owner.dom);
    let state = &owner;
    let previous_selected = activation.selected.as_ref().and_then(|lease| {
        let (dom, node) = lease.current();
        Rc::ptr_eq(&dom, &state.dom).then_some(node)
    });
    if !accepted {
        let mut dom = state.dom.borrow_mut();
        match activation.previous {
            Previous::Checkbox {
                checked,
                indeterminate,
            } if dom
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("checkbox")) =>
            {
                dom.set_checkedness(node, checked)?;
                dom.set_indeterminate(node, indeterminate)?;
            }
            Previous::Radio { .. } if dom.is_radio(node) => {
                if let Some(selected) = previous_selected.filter(|peer| {
                    *peer == node || dom.same_radio_group(node, *peer).unwrap_or(false)
                }) {
                    dom.set_checkedness(selected, true)?;
                } else {
                    dom.set_checkedness(node, false)?;
                }
            }
            _ => {}
        }
        return Ok(());
    }
    if matches!(activation.previous, Previous::Link) {
        let source_id = event_id(scope, source)?;
        let source_owner = binding_state_for_receiver(scope, source)
            .ok_or(V8BindingError::Allocation("link activation event owner"))?;
        let trusted = source_owner
            .events
            .borrow()
            .get(&source_id)
            .is_some_and(|event| event.is_trusted);
        let suffix = if same_target_arena
            && state
                .dom
                .borrow()
                .is_element(node, "http://www.w3.org/1999/xhtml", "a")
        {
            image_map_suffix(scope, state, target, source, trusted)?
        } else {
            None
        };
        return submissions::queue_click_with_involvement(
            scope,
            state,
            node,
            trusted,
            suffix.as_deref(),
        );
    }
    if let Some(mut command) = command::Job::new(scope, state, node)? {
        if !command.poll(scope)? {
            return Err(V8BindingError::JavaScriptException);
        }
        return Ok(());
    }
    if top_layer::invoke(scope, state, node)? {
        return Ok(());
    }
    let reset_form_owner = {
        let dom = state.dom.borrow();
        if (dom.is_element(node, "http://www.w3.org/1999/xhtml", "input")
            || dom.is_element(node, "http://www.w3.org/1999/xhtml", "button"))
            && dom
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("reset"))
        {
            if dom.control_disabled(node)? {
                return Ok(());
            }
            Some(dom.form_owner(node)?)
        } else {
            None
        }
    };
    if let Some(form) = reset_form_owner {
        if let Some(form) = form {
            reset_form(scope, state, form)?;
        }
        return Ok(());
    }
    if validation::is_submit_button(&state.dom.borrow(), node)? {
        let form = {
            let dom = state.dom.borrow();
            if dom.control_disabled(node)? || !validation::is_submit_button(&dom, node)? {
                return Ok(());
            }
            dom.form_owner(node)?
        };
        if let Some(form) = form {
            select_image_coordinate(scope, state, node, source)?;
            let source_id = event_id(scope, source)?;
            let source_owner = binding_state_for_receiver(scope, source)
                .ok_or(V8BindingError::Allocation("activation source owner"))?;
            let trusted = source_owner
                .events
                .borrow()
                .get(&source_id)
                .is_some_and(|event| event.is_trusted);
            submissions::request_submit_with_involvement(
                scope,
                state,
                form,
                Some(node),
                submissions::SubmissionSource::Activation,
                trusted,
            )?;
        }
        return Ok(());
    }
    if let Previous::Label = activation.previous {
        let control = {
            let dom = state.dom.borrow();
            let mut cursor = same_target_arena.then_some(target);
            while let Some(current) = cursor.filter(|current| *current != node) {
                if dom.is_interactive_content(current) {
                    return Ok(());
                }
                cursor = dom.parent(current)?;
            }
            dom.label_control(node)?
                .filter(|control| !dom.control_disabled(*control).unwrap_or(true))
        };
        if let Some(control) = control {
            if same_target_arena
                && (control == target
                    || state
                        .dom
                        .borrow()
                        .arena()
                        .ancestors(target)?
                        .contains(&control))
            {
                return Ok(());
            }
            let event = create_synthetic_click_event(scope, state)?;
            if let Some(payload) = mouse_payload(scope, source) {
                event.set_internal_field(2, payload.into());
            }
            let source_id = event_id(scope, source)?;
            let id = event_id(scope, event)?;
            let source_state = binding_state_for_receiver(scope, source)
                .ok_or(V8BindingError::Allocation("source event owner"))?;
            let trusted = source_state.events.borrow()[&source_id].is_trusted;
            state.events.borrow_mut().get_mut(&id).unwrap().is_trusted = trusted;
            if retained_activation::active(state, control)
                || !state.click_in_progress.borrow_mut().insert(control)
            {
                return Ok(());
            }
            let target = wrap_node(scope, state, control)?;
            let result = dispatch_event(scope, state, control, event);
            let (owner, control) = node_identity::argument(scope, target.into())?
                .ok_or(V8BindingError::Allocation("label control owner"))?;
            owner.click_in_progress.borrow_mut().remove(&control);
            result?;
        }
        return Ok(());
    }
    let fire = {
        let dom = state.dom.borrow();
        dom.is_connected(node)?
            && match activation.previous {
                Previous::Checkbox { .. } => dom
                    .attribute_in_null_namespace(node, "type")?
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("checkbox")),
                Previous::Radio { changed, .. } => {
                    changed && dom.is_radio(node) && dom.checked(node)?
                }
                Previous::Label | Previous::Submit | Previous::Link => false,
            }
    };
    if fire {
        state.dom.borrow_mut().set_user_validity(node, true)?;
        for event_type in ["input", "change"] {
            let target = v8::Local::new(scope, &activation.object);
            let (owner, node) = node_identity::argument(scope, target.into())?
                .ok_or(V8BindingError::Allocation("activation event target"))?;
            let state = &owner;
            let event = create_event_object(scope, state, event_type, true, false)?;
            let id = event_id(scope, event)?;
            {
                let mut events = state.events.borrow_mut();
                let record = events.get_mut(&id).unwrap();
                record.is_trusted = true;
                record.composed = event_type == "input";
            }
            dispatch_event(scope, state, node, event)?;
        }
    }
    Ok(())
}

pub(super) fn image_map_suffix(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    event: v8::Local<v8::Object>,
    trusted: bool,
) -> Result<Option<String>, V8BindingError> {
    {
        let dom = state.dom.borrow();
        if !dom.is_element(target, "http://www.w3.org/1999/xhtml", "img")
            || dom.attribute_in_null_namespace(target, "ismap")?.is_none()
        {
            return Ok(None);
        }
    }
    if !trusted {
        return Ok(Some("?0,0".into()));
    }
    let x = native_input_property(scope, event, "MouseEvent", "clientX")?
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let y = native_input_property(scope, event, "MouseEvent", "clientY")?
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let rect = match geometry_for_node(state, target)? {
        BindingGeometry::Available(rect) => rect,
        BindingGeometry::Hidden => return Ok(Some("?0,0".into())),
        BindingGeometry::Unsupported(detail) => {
            return Err(V8BindingError::Unsupported {
                interface: "HTMLImageElement".into(),
                member: format!("image map activation: {detail}"),
            });
        }
    };
    let coordinate = |point: f64, start: f64| (point - start).max(0.).round() as u64;
    Ok(Some(format!(
        "?{},{}",
        coordinate(x, rect.x),
        coordinate(y, rect.y)
    )))
}

fn select_image_coordinate(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    event: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    {
        let dom = state.dom.borrow();
        if !dom.is_element(node, "http://www.w3.org/1999/xhtml", "input")
            || !dom
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|s| s.eq_ignore_ascii_case("image"))
        {
            return Ok(());
        }
    }
    // HTMLElement.click() synthesizes a non-pointing PointerEvent. It must
    // preserve an earlier coordinate, unlike a pointing MouseEvent activation.
    if native_input_property(scope, event, "PointerEvent", "pointerId")
        .ok()
        .and_then(|value| value.int32_value(scope))
        == Some(-1)
    {
        return Ok(());
    }
    let x = native_input_property(scope, event, "MouseEvent", "clientX")?
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let y = native_input_property(scope, event, "MouseEvent", "clientY")?
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let rect = match geometry_for_node(state, node)? {
        BindingGeometry::Available(rect) => rect,
        BindingGeometry::Hidden => return Ok(()),
        BindingGeometry::Unsupported(detail) => {
            return Err(V8BindingError::Unsupported {
                interface: "HTMLInputElement".into(),
                member: format!("image activation: {detail}"),
            });
        }
    };
    let layout = state.layout.borrow();
    let inset = |names: [&str; 2]| -> f64 {
        names
            .into_iter()
            .filter_map(|name| {
                layout
                    .styles
                    .get(&node_key(node))?
                    .get(name)?
                    .strip_suffix("px")?
                    .parse::<f64>()
                    .ok()
            })
            .sum()
    };
    let left = inset(["border-left-width", "padding-left"]);
    let top = inset(["border-top-width", "padding-top"]);
    let coordinate = |point: f64, start: f64, extent: f64, inset: f64| {
        ((point - start).clamp(0., extent.max(0.)) - inset).round() as i32
    };
    let selected = (
        coordinate(x, rect.x, rect.width, left),
        coordinate(y, rect.y, rect.height, top),
    );
    drop(layout);
    state
        .dom
        .borrow_mut()
        .arena_mut()
        .get_mut(node)?
        .image_coordinates = selected;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) {
        run_at(source, None);
    }
    fn run_at(source: &str, url: Option<&str>) {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        if let Some(url) = url {
            state.commit_location(url).unwrap();
        }
        v8::tc_scope!(let scope, scope);
        let source = v8::String::new(scope, source).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        let result = script.run(scope);
        if let Some(error) = scope.exception() {
            panic!("{}", error.to_rust_string_lossy(scope));
        }
        assert_eq!(result.unwrap().to_rust_string_lossy(scope), "ok");
    }

    #[test]
    fn allocation_failure_before_preactivation_preserves_checkbox_and_radio_group() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<input id=c type=checkbox><input id=a type=radio name=g checked><input id=b type=radio name=g>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(224), 1).unwrap();
        for name in ["c", "b"] {
            let target = state
                .dom
                .borrow()
                .get_element_by_id(state.document_root(), name)
                .unwrap()
                .unwrap();
            let before = {
                let dom = state.dom.borrow();
                ["c", "a", "b"].map(|name| {
                    dom.checked(
                        dom.get_element_by_id(state.document_root(), name)
                            .unwrap()
                            .unwrap(),
                    )
                    .unwrap()
                })
            };
            let event = create_synthetic_click_event(scope, &state).unwrap();
            let id = event_id(scope, event).unwrap();
            FAIL_PREACTIVATION.with(|slot| slot.set(true));
            assert!(prepare(scope, &state, target, event, id).is_err());
            let after = {
                let dom = state.dom.borrow();
                ["c", "a", "b"].map(|name| {
                    dom.checked(
                        dom.get_element_by_id(state.document_root(), name)
                            .unwrap()
                            .unwrap(),
                    )
                    .unwrap()
                })
            };
            assert_eq!(before, after);
        }
    }

    #[test]
    fn submission_properties_reflect_native_content_attributes() {
        run_at(
            r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const form=document.createElement('form'); document.body.append(form);
            for(const [property,attribute,value] of [
                ['target','target','destination'], ['enctype','enctype','multipart/form-data'],
                ['encoding','enctype','text/plain'], ['acceptCharset','accept-charset','UTF-8'],
                ['name','name','named-form'], ['rel','rel','noreferrer'],
                ['autocomplete','autocomplete','off'], ['method','method','post'],
            ]) {
                check(!!Object.getOwnPropertyDescriptor(HTMLFormElement.prototype,property),property+' IDL');
                form[property]=value;
                check(form.getAttribute(attribute)===value,property+' content write');
                check(form[property]===value,property+' content read');
                form.removeAttribute(attribute);
            }
            check(form.method==='get'&&form.enctype==='application/x-www-form-urlencoded'&&form.autocomplete==='on','missing defaults');
            form.method='PoSt';check(form.method==='post'&&form.getAttribute('method')==='PoSt','method canonicalization');
            form.enctype='invalid';check(form.enctype==='application/x-www-form-urlencoded','invalid enctype');
            form.noValidate=true;check(form.hasAttribute('novalidate')&&form.noValidate,'noValidate true');
            form.noValidate=false;check(!form.hasAttribute('novalidate')&&!form.noValidate,'noValidate false');
            const base=document.createElement('base');base.href='https://base.test/path/';document.body.prepend(base);
            check(base instanceof HTMLBaseElement&&base.getAttribute('href')==='https://base.test/path/','base reflection');
            const otherBase=document.createElement('base');
            check(otherBase.href==='https://forms.test/document','missing base href');
            otherBase.href='';check(otherBase.href==='https://forms.test/document','empty base href');
            otherBase.href='relative';check(otherBase.href==='https://forms.test/relative','base ignores other base');
            check(form.action==='https://forms.test/document','missing action uses document URL');
            form.action='';check(form.action==='https://forms.test/document','empty action ignores base');
            form.action='submit';check(form.action==='https://base.test/path/submit','relative action uses base');
            const marker={};let thrown;
            try{form.target={toString(){throw marker}}}catch(error){thrown=error}
            check(thrown===marker&&!form.hasAttribute('target'),'conversion exception leaves content unchanged');
            form.target='\uD800';check(form.target==='\uD800'&&form.getAttribute('target')==='\uD800','DOMString preserves surrogate');
            form.action='/\uD800';check(form.getAttribute('action')==='/\uFFFD','USVString replaces surrogate');
            for(const tag of ['input','button']) {
                const submitter=document.createElement(tag);form.append(submitter);
                check(submitter.formAction==='https://forms.test/document',tag+' missing action');
                submitter.formAction='';check(submitter.formAction==='https://forms.test/document',tag+' empty action');
                submitter.formAction='next';check(submitter.formAction==='https://base.test/path/next',tag+' relative action');
                for(const [property,attribute,value] of [
                    ['formAction','formaction','https://forms.test/submit'],
                    ['formMethod','formmethod','post'], ['formEnctype','formenctype','text/plain'],
                    ['formTarget','formtarget','destination'],
                ]) {
                    submitter[property]=value;
                    check(submitter.getAttribute(attribute)===value&&submitter[property]===value,tag+' '+property);
                    submitter.removeAttribute(attribute);
                }
                check(submitter.formMethod===''&&submitter.formEnctype==='','missing overrides');
                submitter.formMethod='bad';check(submitter.formMethod==='get','invalid override');
                submitter.formNoValidate=true;check(submitter.hasAttribute('formnovalidate'),'submitter bypass');
                submitter.formNoValidate=false;check(!submitter.hasAttribute('formnovalidate'),'submitter bypass removed');
                submitter.setAttributeNS('urn:not-html','formtarget','foreign');check(submitter.formTarget==='','namespace isolation');
            }
            'ok'
        "#,
            Some("https://forms.test/document"),
        );
    }

    #[test]
    fn activation_uses_input_type_after_click_handlers() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            for(const tag of ['input','button'])for(const type of ['text','search','tel','url','email','password','date','month','week','time','datetime-local','number','range','color','checkbox','radio','file','image','reset','button']) {
                const f=document.createElement('form'),b=document.createElement(tag);b.type=type;f.append(b);document.body.append(f);
                let submitted=0;
                f.addEventListener('submit',e=>{e.preventDefault();submitted++;check(e.submitter===b,'submitter')});
                b.addEventListener('click',()=>{b.type='submit'});
                b.click();check(submitted===1,tag+' '+type+' becomes submit');f.remove();
            }
            'ok'
        "#);
    }

    #[test]
    fn user_validity_submission_reset_and_script_changes_share_native_state() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const f=document.createElement('form');f.innerHTML='<input required><select required><option value="">Choose</option></select><button>Go</button>';document.body.append(f);
            const i=f.querySelector('input'),s=f.querySelector('select');f.addEventListener('submit',e=>e.preventDefault());
            check(!i.matches(':user-invalid')&&!i.matches(':user-valid'),'initial');
            i.value='script';check(!i.matches(':user-valid'),'script assignment does not interact');i.value='';
            i.addEventListener('invalid',e=>e.preventDefault());
            i.checkValidity();check(!i.matches(':user-invalid'),'checkValidity preserves untouched state');
            i.addEventListener('invalid',()=>check(i.matches(':user-invalid'),'state before invalid callback'));
            f.requestSubmit();check(i.matches(':user-invalid')&&s.matches(':user-invalid'),'submission marks controls');
            i.value='ok';check(i.matches(':user-valid'),'subsequent script changes preserve eligibility');
            f.addEventListener('reset',e=>e.preventDefault(),{once:true});f.reset();check(i.matches(':user-valid'),'cancelled reset preserves state');
            f.reset();check(!i.matches(':user-valid')&&!i.matches(':user-invalid')&&!s.matches(':user-invalid'),'reset clears user validity');
            f.noValidate=true;f.requestSubmit();check(i.matches(':user-invalid'),'novalidate still records submission');
            'ok'
        "#);
    }

    #[test]
    fn click_and_request_submit_share_validation_and_reentrancy() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            for(const outer of ['click','request'])for(const inner of ['click','request'])for(const required of [false,true]) {
                const f=document.createElement('form'),b=document.createElement('button'),s=document.createElement('select');
                f.append(b,s);document.body.append(f);s.required=required;
                let count=0;const invoke=mode=>mode==='click'?b.click():f.requestSubmit(b);
                (required?s:f).addEventListener(required?'invalid':'submit',e=>{
                    e.preventDefault();count++;if(count<3)invoke(inner);
                    if(!required)check(e instanceof SubmitEvent&&e.submitter===b,'click submitter');
                });
                invoke(outer);check(count===1,outer+' '+inner+' '+required);f.remove();
            }
            const f=document.createElement('form'),b=document.createElement('button'),span=document.createElement('span');
            b.append(span);f.append(b);document.body.append(f);let count=0;
            f.addEventListener('submit',e=>{count++;e.preventDefault()});
            span.click();check(count===1,'descendant activation');
            b.addEventListener('click',e=>e.preventDefault(),{once:true});b.click();check(count===1,'canceled click');
            b.addEventListener('click',()=>b.type='button',{once:true});b.click();check(count===1,'type changed during click');
            b.type='submit';b.disabled=true;b.click();check(count===1,'disabled click');
            b.dispatchEvent(new Event('click',{bubbles:true}));check(count===1,'untyped event no activation');
            'ok'
        "#);
    }

    #[test]
    fn submit_event_constructor_and_request_submit_preserve_submitter_identity() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const button=document.createElement('button');
            for(const init of [undefined,null,{}, {submitter:null},{submitter:undefined}])check(new SubmitEvent('submit',init).submitter===null,'default null');
            let order=[];const init={};
            for(const [name,value] of [['bubbles',true],['cancelable',true],['composed',true],['submitter',button]])Object.defineProperty(init,name,{get(){order.push(name);return value}});
            const event=new SubmitEvent({toString(){order.push('type');return 'submit'}},init);
            check(order.join()==='type,bubbles,cancelable,composed,submitter','conversion order');
            check(event instanceof Event&&event instanceof SubmitEvent&&event.submitter===button&&event.bubbles&&event.cancelable&&event.composed&&!event.isTrusted,'typed event');
            event.initEvent('other',false,false);check(event.submitter===button,'initEvent retains submitter');
            class Derived extends SubmitEvent{};check(new Derived('submit',{submitter:button}) instanceof Derived,'subclass');
            const getter=Object.getOwnPropertyDescriptor(SubmitEvent.prototype,'submitter').get;
            for(const receiver of [{},new Event('submit'),Object.create(event),new Proxy(event,{})]) {
                let error;try{getter.call(receiver)}catch(e){error=e}check(error instanceof TypeError,'receiver');
            }
            for(const value of [{},document,document.createTextNode('x'),document.createElementNS('http://www.w3.org/2000/svg','svg'),new Proxy(button,{})]) {
                let error;try{new SubmitEvent('submit',{submitter:value})}catch(e){error=e}check(error instanceof TypeError,'HTMLElement conversion');
            }
            const marker={};let error;try{new SubmitEvent('submit',{get submitter(){throw marker}})}catch(e){error=e}check(error===marker,'exception preservation');
            const form=document.createElement('form');document.body.append(form);form.append(button);
            let saved;form.addEventListener('submit',e=>{saved=e;e.preventDefault()});
            form.requestSubmit(button);check(saved instanceof SubmitEvent&&saved.submitter===button&&saved.isTrusted,'native submitter');
            button.remove();check(saved.submitter===button,'detached submitter');
            form.requestSubmit();check(saved.submitter===null,'form submitter null');
            'ok'
        "#);
    }

    #[test]
    fn request_submit_checks_submitter_validation_bypass_and_reentrancy() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const f=document.createElement('form');f.id='submission-owner';document.body.append(f);
            const s=document.createElement('select');s.required=true;f.append(s);
            let invalid=0,submitted=0;
            s.addEventListener('invalid',()=>{invalid++;if(invalid<3)f.requestSubmit()});
            f.addEventListener('submit',e=>{submitted++;check(e.isTrusted&&e.bubbles&&e.cancelable,'submit metadata');e.preventDefault();if(submitted<3)f.requestSubmit()});
            f.requestSubmit();check(invalid===1&&submitted===0,'invalid reentrancy');
            s.required=false;f.requestSubmit();check(submitted===1,'submit reentrancy');
            f.requestSubmit(null);f.requestSubmit(undefined);check(submitted===3,'guard cleared');
            const button=document.createElement('button');button.setAttribute('form',f.id);document.body.prepend(button);
            s.required=true;button.setAttribute('formnovalidate','');button.disabled=true;
            f.requestSubmit(button);check(submitted===4&&invalid===1,'disabled submitter bypass');
            button.removeAttribute('formnovalidate');button.setAttributeNS('urn:test','formnovalidate','');
            f.requestSubmit(button);check(submitted===4&&invalid===2,'namespaced bypass ignored');
            f.setAttribute('novalidate','');f.requestSubmit();check(submitted===5,'form bypass');f.removeAttribute('novalidate');
            for(const bad of [document.body,document.createElement('input'),{},new Proxy(button,{})]) {
                let error;try{f.requestSubmit(bad)}catch(e){error=e}check(error instanceof TypeError,'invalid submitter type');
            }
            button.remove();let error;try{f.requestSubmit(button)}catch(e){error=e}check(error&&error.name==='NotFoundError','foreign submitter');
            f.submit();check(submitted===5&&invalid===2,'submit method bypasses events and validation');
            f.remove();const old=invalid;f.requestSubmit();check(invalid===old&&submitted===5,'disconnected');
            'ok'
        "#);
    }

    #[test]
    fn form_validation_uses_mixed_tree_order_and_does_not_add_new_invalid_controls() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const f=document.createElement('form');document.body.append(f);
            const select=document.createElement('select'),input=document.createElement('input'),textarea=document.createElement('textarea'),later=document.createElement('select');
            f.append(select,textarea,input,later);
            for(const c of [select,textarea,input])c.required=true;
            let seen=[];
            for(const [c,name] of [[select,'select'],[textarea,'textarea'],[input,'input'],[later,'later']])c.addEventListener('invalid',()=>seen.push(name));
            select.addEventListener('invalid',()=>{later.required=true},{once:true});
            check(!f.checkValidity()&&seen.join()==='select,textarea,input','snapshot mixed tree order');
            seen=[];check(!f.checkValidity()&&seen.join()==='select,textarea,input,later','next validation sees new invalid control');
            for(const c of [select,textarea,input,later])c.required=false;
            const host=document.createElement('div');f.append(host);
            const shadow=host.attachShadow({mode:'open'}),hidden=document.createElement('select');hidden.required=true;shadow.append(hidden);
            check(f.checkValidity(),'shadow control excluded');
            'ok'
        "#);
    }

    #[test]
    fn email_submission_blocks_invalid_values_and_allows_recovery() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const f=document.createElement('form'),e=document.createElement('input');
            e.type='email';f.append(e);document.body.append(f);
            let submits=0,invalid=0;
            f.onsubmit=event=>{event.preventDefault();submits++};
            e.oninvalid=()=>invalid++;
            e.value='not-an-email';f.requestSubmit();
            check(submits===0&&invalid===1,'invalid email submitted');
            e.value='a+b@example.test';f.requestSubmit();
            check(submits===1&&invalid===1,'valid recovery');
            e.value='bad';e.readOnly=true;f.requestSubmit();
            check(submits===2&&invalid===1,'readonly barred');
            e.readOnly=false;e.disabled=true;f.requestSubmit();
            check(submits===3&&invalid===1,'disabled barred');
            e.disabled=false;e.value='a@b,c@d';e.setAttribute('multiple','');
            f.requestSubmit();check(submits===4,'multiple addresses');
            e.value='a@b,';f.requestSubmit();check(submits===4&&invalid===2,'empty list entry');
            e.value='';f.requestSubmit();check(submits===5,'optional empty');
            e.required=true;f.requestSubmit();check(submits===5&&invalid===3,'required empty');
            e.type='hidden';f.requestSubmit();check(submits===6,'hidden barred');
            'ok'
        "#);
    }

    #[test]
    fn form_validation_snapshots_owned_controls_before_invalid_events() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const f=document.createElement('form');f.id='validation-owner';
            const before=document.createElement('select'),inside=document.createElement('select'),after=document.createElement('select');
            before.setAttribute('form',f.id);after.setAttribute('form',f.id);
            document.body.append(before,f,after);f.append(inside);
            const other=document.createElement('form');other.id='other-owner';document.body.append(other);
            const excluded=document.createElement('select');excluded.required=true;excluded.setAttribute('form',other.id);f.append(excluded);
            for(const s of [before,inside,after])s.required=true;
            let events=[],bubbled=0;f.addEventListener('invalid',()=>bubbled++);
            for(const [s,name] of [[before,'before'],[inside,'inside'],[after,'after']])s.addEventListener('invalid',e=>{
                check(e.isTrusted&&e.cancelable&&!e.bubbles&&!e.composed&&e.target===s,'event metadata');
                e.preventDefault();events.push(name);
            });
            before.addEventListener('invalid',()=>{inside.required=false;after.remove();},{once:true});
            check(!f.checkValidity(),'snapshot remains invalid');
            check(events.join()==='before,inside,after'&&bubbled===0,'tree order and detached snapshot');
            before.required=false;events=[];check(f.checkValidity()&&events.length===0,'recompute ownership and validity');
            inside.add(new Option('',''));inside.add(new Option('',''));inside.required=true;inside.selectedIndex=1;
            check(f.checkValidity(),'non-placeholder empty option valid');
            inside.setCustomValidity('custom');check(!f.checkValidity(),'custom error');
            inside.disabled=true;check(f.checkValidity(),'barred custom error');inside.disabled=false;
            inside.setCustomValidity('');inside.selectedIndex=0;check(!f.checkValidity(),'placeholder');
            f.remove();check(!f.checkValidity(),'detached form descendants');
            'ok'
        "#);
    }

    #[test]
    fn select_validation_ignores_namespaced_control_attributes() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const s=document.createElement('select');s.setAttributeNS('urn:test','required','');
            check(s.validity.valid&&!s.validity.valueMissing,'namespaced required');
            s.removeAttributeNS('urn:test','required');s.required=true;
            s.add(new Option(''));check(s.validity.valueMissing,'real required');
            s.setAttributeNS('urn:test','size','2');s.setAttributeNS('urn:test','multiple','');
            check(s.validity.valueMissing,'namespaced size and multiple');
            s.setAttributeNS('urn:test','disabled','');check(s.willValidate,'namespaced disabled');
            const f=document.createElement('fieldset');f.setAttributeNS('urn:test','disabled','');f.append(s);
            check(s.willValidate,'namespaced fieldset disabled');
            f.removeAttributeNS('urn:test','disabled');f.disabled=true;check(!s.willValidate,'real disabled fieldset');
            const empty=document.createElement('select');empty.add(new Option('chosen'));
            empty.selectedIndex=-1;
            for(const name of ['size','multiple']) {
                empty.setAttributeNS('urn:test',name,'2');
                check(empty.selectedIndex===-1,'namespaced set preserves empty selection '+name);
                empty.getAttributeNodeNS('urn:test',name).value='3';
                check(empty.selectedIndex===-1,'namespaced Attr write preserves empty selection '+name);
                empty.removeAttributeNS('urn:test',name);
                check(empty.selectedIndex===-1,'namespaced remove preserves empty selection '+name);
            }
            'ok'
        "#);
    }

    #[test]
    fn select_validation_state_and_invalid_event_are_live() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const s=document.createElement('select'),v=s.validity;document.body.append(s);
            check(v===s.validity&&v instanceof ValidityState&&v.valid,'state identity');
            let events=0,bubbled=0;s.addEventListener('invalid',e=>{
                events++;check(e.target===s&&e.cancelable&&!e.bubbles&&!e.composed&&e.isTrusted,'invalid event');e.preventDefault();
            });document.body.addEventListener('invalid',()=>bubbled++);
            s.required=true;check(v.valueMissing&&!v.valid&&!s.checkValidity()&&events===1&&bubbled===0,'missing');
            s.add(new Option('chosen','chosen'));check(v.valid&&s.checkValidity()&&events===1,'valid');
            s.setCustomValidity('custom');check(v.customError&&!v.valid&&s.validationMessage==='custom'&&!s.checkValidity(),'custom');
            s.disabled=true;check(!s.willValidate&&s.checkValidity()&&v.customError&&s.validationMessage==='','barred');
            s.disabled=false;s.setCustomValidity('');check(v.valid&&!v.customError&&s.validationMessage==='','clear');
            let error;try{s.setCustomValidity()}catch(e){error=e}check(error instanceof TypeError,'required message');
            const sentinel={};error=null;try{s.setCustomValidity({toString(){throw sentinel}})}catch(e){error=e}check(error===sentinel&&v.valid,'conversion exception');
            s.setCustomValidity('\ud800');check(s.validationMessage.charCodeAt(0)===0xd800,'DOMString preservation');
            const copy=s.cloneNode(true);check(!copy.validity.customError,'clone does not copy custom error');
            for(const receiver of [{},Object.create(v),new Proxy(v,{})]) {
                error=null;try{Object.getOwnPropertyDescriptor(ValidityState.prototype,'valid').get.call(receiver)}catch(e){error=e}
                check(error instanceof TypeError,'brand');
            }
            'ok'
        "#);
    }

    #[test]
    fn select_validation_eligibility_is_live() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const s=document.createElement('select');check(s.willValidate,'detached candidate');
            s.setAttribute('readonly','');check(s.willValidate,'readonly ignored');
            s.disabled=true;check(!s.willValidate,'disabled');s.disabled=false;
            const f=document.createElement('fieldset');f.disabled=true;f.append(s);check(!s.willValidate,'fieldset');
            const l=document.createElement('legend');f.prepend(l);l.append(s);check(s.willValidate,'first legend exemption');
            const d=document.createElement('datalist');d.append(s);check(!s.willValidate,'datalist');
            s.remove();check(s.willValidate,'eligibility restored');'ok'
        "#);
    }

    #[test]
    fn indexed_option_replacement_records_adoption_then_compound_change() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            for(const mode of ['fresh','previous','next','self','other']) {
                const s=document.createElement('select'),other=document.createElement('select');
                const a=new Option('a'),b=new Option('b'),c=new Option('c');s.append(a,b,c);
                const fresh=new Option('fresh');if(mode==='other')other.append(fresh);
                const incoming=mode==='previous'?a:mode==='next'?c:mode==='self'?b:fresh;
                const observer=new MutationObserver(()=>{});observer.observe(s,{childList:true});observer.observe(other,{childList:true});
                s.options[1]=incoming;
                const records=observer.takeRecords();check(records.length===(mode==='fresh'?1:2),'count '+mode);
                if(mode!=='fresh')check(records[0].addedNodes.length===0&&records[0].removedNodes[0]===incoming&&records[0].target===(mode==='other'?other:s),'adoption '+mode);
                const record=records[records.length-1];
                check(record.target===s&&record.addedNodes.length===1&&record.addedNodes[0]===incoming,'added '+mode);
                check(record.removedNodes.length===(mode==='self'?0:1),'removed count '+mode);
                if(mode!=='self')check(record.removedNodes[0]===b,'removed identity '+mode);
                check(record.previousSibling===a&&record.nextSibling===(mode==='next'?null:c),'siblings '+mode);
                observer.disconnect();
            }
            'ok'
        "#);
    }

    #[test]
    fn blank_option_growth_is_one_child_list_batch() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            for(const indexed of [false,true]) {
                const s=document.createElement('select');document.body.append(s);
                const group=document.createElement('optgroup');s.append(group);
                const range=document.createRange();range.setStart(s,1);range.collapse(true);
                const observer=new MutationObserver(()=>{});observer.observe(s,{childList:true,subtree:true});
                const last=new Option('last');
                if(indexed)s.options[3]=last;else s.length=3;
                const records=observer.takeRecords();
                check(records.length===(indexed?2:1),'record count');
                const batch=records[0];check(batch.target===s&&batch.type==='childList','target');
                check(batch.addedNodes.length===3&&batch.removedNodes.length===0,'batch members');
                check(batch.previousSibling===group&&batch.nextSibling===null,'siblings');
                for(let i=0;i<3;i++)check(batch.addedNodes[i]===s.options[i]&&s.options[i].parentNode===s&&s.options[i].ownerDocument===document,'members');
                check(range.startContainer===s&&range.startOffset===1,'range at append boundary');
                check(s.selectedIndex===0&&s.options[0].selected,'normalization');
                if(indexed)check(records[1].addedNodes[0]===last&&records[1].previousSibling===s.options[2],'separate supplied option');
                s.length=s.length;check(observer.takeRecords().length===0,'no-op length');
                observer.disconnect();s.remove();
            }
            'ok'
        "#);
    }

    #[test]
    fn options_collection_indexed_mutation_and_descriptors() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            for(const target of ['options','select']) {
            const s=document.createElement('select'),o=target==='options'?s.options:s;
            const a=new Option('a'),b=new Option('b');o[0]=a;
            check(o.length===1&&o[0]===a,'append');o[0]=b;
            check(o.length===1&&o[0]===b&&a.parentNode===null,'replace');
            o[3]=a;check(o.length===4&&o[3]===a&&o[1].text===''&&o[2].text==='','padding');
            o[0]=null;o[0]=undefined;check(o.length===2&&o[1]===a,'nullable removal');
            o[99]=null;check(o.length===2,'null does not pad');
            for(const value of [1,'x',{},document.createElement('optgroup')]) {
                let error;try{o[0]=value}catch(e){error=e}check(error instanceof TypeError&&o.length===2,'type check');
            }
            Object.defineProperty(o,'0',{value:b,writable:false,enumerable:false,configurable:false});
            const d=Object.getOwnPropertyDescriptor(o,'0');check(d.value===b&&d.writable&&d.enumerable&&d.configurable,'virtual descriptor');
            check(!Reflect.defineProperty(o,'0',{get(){return a}}),'accessor rejected');
            check(!Reflect.defineProperty(o,'0',{}),'generic descriptor rejected');
            check(Reflect.defineProperty(o,'0',{writable:true})&&o.length===1&&o[0]===a,'missing value converts undefined');
            const receiver={};check(Reflect.set(o,'0',b,receiver)&&receiver[0]===b&&o[0]===a,'different receiver');
            const child=Object.create(o);child[0]=b;check(child[0]===b&&o[0]===a,'prototype receiver');
            check(!Reflect.deleteProperty(o,'0')&&Reflect.deleteProperty(o,'99'),'delete rules');
            const g=document.createElement('optgroup');g.append(b);s.append(g);o[1]=new Option('c');
            check(g.firstChild.text==='c'&&b.parentNode===null,'replacement in group');
            o[0]=o[0];check(o[0]===a,'self replacement');
            let error;try{o[0]=new Proxy(a,{})}catch(e){error=e}check(error instanceof TypeError,'proxy option rejected');
            const outer=new Option('ancestor');outer.append(s);
            error=null;try{o[5]=outer}catch(e){error=e}
            check(error?.name==='HierarchyRequestError'&&o.length===5,'padding precedes hierarchy failure');
            const keys=Object.keys(o).filter(k=>/^\d+$/.test(k));check(keys.join(',')==='0,1,2,3,4','indices enumerate');
            }
            'ok'
        "#);
    }

    #[test]
    fn select_collection_methods_share_live_membership_and_remove_overloads() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const s=document.createElement('select');document.body.append(s);
            const a=new Option('a'),b=new Option('b');a.id='key';s.add(a);s.add(b);
            check(s.length===2&&s.item(0)===a&&s.namedItem('key')===a,'mirrors');
            check(s.item(-1)===null&&s.namedItem('')===null,'missing');
            s.remove(undefined);check(s.parentNode===document.body&&s.item(0)===b,'explicit undefined removes option');
            s.remove(-1);s.remove(99);check(s.length===1,'out of range');
            s.length={valueOf(){s.add(new Option('side'));return 3}};
            check(s.length===3&&s.options.length===3&&s.item(1).text==='side','length conversion');
            check(s.item({valueOf(){s.remove(0);return 0}}).text==='side','item conversion before lookup');
            check(s.namedItem({toString(){s.item(0).id='late';return 'late'}})===s.item(0),'name conversion');
            const sentinel={};for(const method of ['item','namedItem']) {
                let caught;try{s[method]()}catch(e){caught=e}check(caught instanceof TypeError,'required '+method);
                caught=null;try{s[method]({valueOf(){throw sentinel},toString(){throw sentinel}})}catch(e){caught=e}
                check(caught===sentinel,'exception '+method);
            }
            s.length=1;check(s.options.length===1,'trim');s.remove();
            check(s.parentNode===null&&s.length===1,'no argument removes select');
            'ok'
        "#);
    }

    #[test]
    fn options_collection_is_live_branded_and_mutates_its_select() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const s=document.createElement('select'),list=s.options;
            check(list===s.options&&list instanceof HTMLOptionsCollection&&list instanceof HTMLCollection,'identity and inheritance');
            check(list.length===0&&list.item(0)===null&&list.selectedIndex===-1,'empty');
            const a=new Option('a'),b=new Option('b');a.id='name';list.add(a);list.add(b,0);
            check(list[0]===b&&list[1]===a&&list.namedItem('name')===a&&list.name===a,'live order/names');
            list.selectedIndex=0;check(b.selected&&s.selectedIndex===0,'selected index');
            s.selectedIndex=-1;check(list.selectedIndex===-1,'selection shared');
            const group=document.createElement('optgroup');group.append(new Option('c'));list.add(group);
            list.length=5;check(list.length===5&&s.lastChild.localName==='option'&&s.lastChild.parentNode===s,'grow outside group');
            check(list[3].attributes.length===0&&list[3].childNodes.length===0,'blank option');
            list.length=2;check(list.length===2&&group.parentNode===s&&group.children.length===0,'trim preserves group');
            list.remove(-1);list.remove(99);check(list.length===2,'out of range no-op');
            list.remove(0);check(list.length===1&&list[0]===a,'remove indexed member');
            list.length=100001;check(list.length===1,'specified growth ceiling');
            list.length={valueOf(){list.add(new Option('side'));return 3}};
            check(list.length===3&&list[1].text==='side','conversion before membership');
            const sentinel={};let caught;try{list.length={valueOf(){throw sentinel}}}catch(e){caught=e}
            check(caught===sentinel&&list.length===3,'conversion failure');
            caught=null;try{list.remove()}catch(e){caught=e}check(caught instanceof TypeError,'required remove argument');
            caught=null;try{Object.getOwnPropertyDescriptor(HTMLOptionsCollection.prototype,'length').get.call(s.children)}catch(e){caught=e}
            check(caught instanceof TypeError,'incompatible receiver');
            const excluded=document.createElement('datalist');excluded.append(new Option('excluded'));s.append(excluded);
            check(list.length===3,'native option list filter');
            'ok'
        "#);
    }

    #[test]
    fn select_size_reflects_numeric_conversion_and_controls_reset() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const s=document.createElement('select');
            check(s.size===0&&s.type==='select-one','defaults');
            s.multiple=true;check(s.type==='select-multiple','multiple type');s.multiple=false;
            for(const [input,expected] of [['',0],['junk',0],[' +3junk',3],['2.9',2],['-1',0],['2147483647',2147483647],['2147483648',0],['4294967296',0],['\u00a02',0]]) {
                s.setAttribute('size',input);check(s.size===expected,'attribute '+input);
            }
            for(const [input,expected] of [[2.9,2],[-1,0],[2147483648,0],[4294967298,2],[null,0],[NaN,0],[Infinity,0]]) {
                s.size=input;check(s.size===expected&&s.getAttribute('size')===String(expected),'setter '+input);
            }
            const a=new Option('a'),b=new Option('b');s.append(a,b);b.selected=true;
            s.size={valueOf(){check(b.selected,'conversion before mutation');return 2}};
            b.remove();check(s.selectedIndex===-1&&!a.selected,'listbox removal has no fallback');
            s.size=1;check(a.selected,'dropdown fallback');
            s.size=2;a.selected=false;check(s.selectedIndex===-1,'listbox selectedness change');
            const sentinel={};let caught;try{s.size={valueOf(){throw sentinel}}}catch(e){caught=e}
            check(caught===sentinel&&s.size===2,'exception leaves attribute unchanged');
            s.removeAttribute('size');s.setAttributeNS('urn:test','size','7');check(s.size===0,'namespace');
            'ok'
        "#);
    }

    #[test]
    fn selected_option_insertion_wins_at_every_position() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            for(let index=0;index<3;index++) {
                const s=document.createElement('select');s.innerHTML='<option>A</option><option selected>C</option><option>D</option>';
                const b=new Option('B','B',false,true);s.insertBefore(b,s.children[index]);
                check(s.value==='B'&&s.selectedOptions.length===1&&s.selectedOptions[0]===b,'position '+index);
                check(!b.defaultSelected,'insertion did not reflect selectedness');
            }
            const s=document.createElement('select');s.innerHTML='<option selected>old</option>';
            const group=document.createElement('optgroup'),a=new Option('a','a',true,true),b=new Option('b','b',true,true);
            group.append(a,b);s.insertBefore(group,s.firstChild);
            check(s.value==='b'&&!a.selected&&b.selected,'last incoming selected in group wins');
            b.removeAttribute('selected');check(!b.selected,'insertion did not dirty default state');
            const other=document.createElement('select');other.append(new Option('other','other',false,true));
            s.insertBefore(other,s.firstChild);check(s.selectedOptions[0]!==other.firstChild,'nested select unaffected');
            'ok'
        "#);
    }

    #[test]
    fn selected_options_is_a_live_same_object_native_collection() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const s=document.createElement('select');s.multiple=true;
            const view=s.selectedOptions;
            check(view instanceof HTMLCollection&&view===s.selectedOptions&&view.length===0,'brand identity empty');
            const a=new Option('a','a',false,true),b=new Option('b','b',false,true);a.id='named';
            s.append(a,b);check(view.length===2&&view[0]===a&&view.namedItem('named')===a,'live membership');
            a.selected=false;check(view.length===1&&view[0]===b&&view.namedItem('named')===null,'state update');
            a.selected=true;s.insertBefore(b,a);check(view[0]===b&&view[1]===a,'tree order');
            for(const tag of ['option','hr','datalist','select']){
                const root=document.createElement(tag);s.append(root);root.append(new Option('excluded','x',false,true));
                check(view.length===2,'excluded '+tag);root.remove();
            }
            s.selectedIndex=-1;check(view.length===0&&view.item(0)===null&&view[0]===undefined,'explicit empty');
            b.selected=true;b.remove();check(view.length===0,'removal');
            check(s.selectedOptions===view,'identity after changes');
            'ok'
        "#);
    }

    #[test]
    fn option_text_label_index_and_form_follow_live_native_state() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const f=document.createElement('form');f.id='owner';document.body.append(f);
            const s=document.createElement('select');s.setAttribute('form','owner');document.body.append(s);
            const a=new Option('a'),b=new Option('b');s.append(a,b);
            check(a.form===f&&b.form===f&&b.index===1,'external owner and index');
            s.insertBefore(b,a);check(b.index===0&&a.index===1,'live index');
            const old=b.firstChild;b.text=' \uD800\t x ';
            check(old.data==='b'&&!old.isConnected&&old!==b.firstChild,'text replacement');
            check(b.text==='\uD800 x'&&b.value==='\uD800 x'&&b.label==='\uD800 x','text fallback');
            b.label='';check(b.label==='','empty label does not fall back');
            b.label='\uDC00';check(b.label==='\uDC00','lossless label');
            b.removeAttribute('label');b.setAttributeNS('urn:test','label','not HTML');
            b.setAttributeNS('urn:test','value','not HTML');
            check(b.label==='\uD800 x'&&b.value==='\uD800 x','namespaced attributes ignored');
            b.text='';check(b.firstChild===null&&b.text==='','empty replacement');
            const sentinel={};let caught;try{b.text={toString(){throw sentinel}}}catch(e){caught=e}
            check(caught===sentinel&&b.firstChild===null,'conversion exception');
            for(const tag of ['datalist','hr','option']){
                const root=document.createElement(tag);s.append(root);root.append(b);
                check(b.form===null&&b.index===0,'barrier '+tag);root.remove();
            }
            const outer=document.createElement('optgroup'),inner=document.createElement('optgroup');
            s.append(outer);outer.append(inner);inner.append(b);
            check(b.form===null&&b.index===0,'nested optgroup barrier');
            outer.append(b);check(b.form===f&&b.index===1,'one group allowed');
            b.remove();check(b.form===null&&b.index===0,'detached option');
            'ok'
        "#);
    }

    #[test]
    fn select_add_handles_groups_references_and_conversion_side_effects() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)},raises=(fn,name)=>{let e;try{fn()}catch(x){e=x}check(e&&e.name===name,'expected '+name)};
            const s=document.createElement('select'),a=new Option('a','a'),b=new Option('b','b');
            s.add(a);s.add(b);check(s.options.length===2&&s.value==='a','helper setup');
            const group=document.createElement('optgroup');check(group instanceof HTMLOptGroupElement,'group brand');
            group.label='\uD800';group.disabled=true;
            check(group.label==='\uD800'&&group.getAttribute('label')==='\uD800'&&group.disabled,'group reflection');
            group.disabled=false;
            const c=new Option('c','c');group.append(c);s.add(group,0);
            check(s.options[0]===c&&s.options[1]===a,'group insertion');
            const d=new Option('d','d');s.add(d,c);check(d.parentNode===group&&s.options[0]===d,'reference parent');
            s.add(c,c);check(group.children.length===2,'same element no-op');
            s.add(a,-1);check(s.options[3]===a,'negative append');
            s.selectedIndex={valueOf(){return 4294967296}};check(s.selectedIndex===0,'WebIDL long index');
            d.value='\uDC00';s.value='\uDC00';check(s.value==='\uDC00'&&d.selected,'UTF16 setters');
            const e=new Option('e','e');s.add(e,{valueOf(){b.remove();return 2}});
            check(s.options[2]===e&&s.options[3]===a,'conversion before live indexing');
            const missing=new Option();raises(()=>s.add(missing,new Option()),'NotFoundError');
            raises(()=>s.add(document.createElement('div')),'TypeError');
            raises(()=>s.add(),'TypeError');
            const ancestor=document.createElement('optgroup');ancestor.append(s);
            raises(()=>s.add(ancestor),'HierarchyRequestError');
            const sentinel={};let caught;try{s.add(new Option(),{valueOf(){throw sentinel}})}catch(x){caught=x}
            check(caught===sentinel,'numeric exception');
            'ok'
        "#);
    }

    #[test]
    fn option_factory_converts_arguments_and_preserves_native_state() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const empty=new Option();
            check(empty instanceof HTMLOptionElement&&empty instanceof Option&&empty.ownerDocument===document,'brand/document');
            check(empty.childNodes.length===0&&!empty.hasAttribute('value')&&!empty.selected,'empty defaults');
            const o=new Option('text','value',true,false);
            check(o.textContent==='text'&&o.value==='value'&&o.defaultSelected&&!o.selected,'independent defaults');
            o.defaultSelected=true;check(o.selected,'factory is not dirty');
            const units=new Option('\uD800','\uDC00',false,true);
            check(units.textContent==='\uD800'&&units.value==='\uDC00'&&units.selected&&!units.defaultSelected,'UTF16');
            const order=[];const converted=new Option({toString(){order.push('text');return 't'}},{toString(){order.push('value');return 'v'}},{valueOf(){throw Error('boolean conversion')}},{});
            check(order.join()==='text,value'&&converted.defaultSelected&&converted.selected,'conversion order');
            const sentinel={};let caught;
            try{new Option({toString(){throw sentinel}})}catch(e){caught=e}check(caught===sentinel,'exception identity');
            let rejected=false;try{Option()}catch(e){rejected=e instanceof TypeError}check(rejected,'requires new');
            class Derived extends Option{};const derived=new Derived('t');check(derived instanceof Derived&&derived instanceof HTMLOptionElement,'subclass');
            const descriptor=Object.getOwnPropertyDescriptor(Option,'prototype');
            check(Option.length===0&&Option.name==='Option'&&Option.prototype===HTMLOptionElement.prototype&&!descriptor.writable&&!descriptor.enumerable&&!descriptor.configurable,'factory descriptors');
            const undef=new Option(undefined,undefined);check(undef.textContent===''&&!undef.hasAttribute('value'),'undefined defaults');
            const nulls=new Option(null,null);check(nulls.textContent==='null'&&nulls.value==='null','null conversion');
            const prototypeOrder=[];
            const target=new Proxy(function(){},{get(target,key){if(key==='prototype'){prototypeOrder.push('prototype');return HTMLOptionElement.prototype}return Reflect.get(target,key)}});
            Reflect.construct(Option,[{toString(){prototypeOrder.push('text');return 't'}}],target);
            check(prototypeOrder.join()==='text,prototype','conversion before prototype lookup: '+prototypeOrder);
            const abrupt=new Proxy(function(){},{get(target,key){if(key==='prototype')throw sentinel;return Reflect.get(target,key)}});
            caught=null;try{Reflect.construct(Option,[],abrupt)}catch(e){caught=e}check(caught===sentinel,'prototype exception');
            'ok'
        "#);
    }

    #[test]
    fn native_select_state_is_live_and_independent_of_defaults() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const s=document.createElement('select'),a=document.createElement('option'),b=document.createElement('option');
            a.textContent=' a ';b.textContent='b';s.append(a,b);
            check(s.selectedIndex===0&&s.value==='a'&&a.selected&&!a.defaultSelected,'initial state');
            b.selected=true;check(s.selectedIndex===1&&!a.selected&&b.selected&&!b.defaultSelected,'selected property');
            b.defaultSelected=true;b.selected=false;
            check(a.selected&&!b.selected&&b.defaultSelected,'reset request and dirty default');
            s.value='absent';check(s.selectedIndex===-1&&s.value===''&&!a.selected&&!b.selected,'explicit empty');
            s.className='unrelated';s.append(a);check(s.selectedIndex===-1,'same-owner move preserves empty');
            a.value='x';b.value='x';s.value='x';check(b.selected&&!a.selected,'first matching tree order');
            s.selectedIndex=-1;check(s.selectedIndex===-1,'negative index');
            s.multiple=true;a.selected=true;b.selected=true;check(a.selected&&b.selected,'multiple');
            s.multiple=false;check(a.selected&&!b.selected,'last selected tree order');
            const other=document.createElement('select');other.append(a);check(other.value==='x'&&b.selected,'owner changes');
            check(a.matches(':checked')&&!a.hasAttribute('selected'),'selected selector');
            const form=document.createElement('form');form.id='selection-form';document.body.append(form);
            other.name='choice';other.setAttribute('form',form.id);document.body.append(other);
            check(new FormData(form).get('choice')==='x','external associated select');
            other.selectedIndex=-1;check(!new FormData(form).has('choice'),'empty selection omitted');
            a.defaultSelected=true;other.selectedIndex=-1;form.reset();check(a.selected,'reset native selection');
            other.multiple=true;const c=document.createElement('option');c.value='second';other.append(c);c.selected=true;
            check(new FormData(form).getAll('choice').join()==='x,second','multiple values');
            c.disabled=true;check(new FormData(form).getAll('choice').join()==='x','disabled option omitted');
            'ok'
        "#);
    }

    #[test]
    fn form_elements_are_live_branded_and_use_form_ownership() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            document.body.innerHTML='<input id=before form=f><form id=f><input id=a name=g><input id=b name=g type=radio value=B><input type=image></form><input id=after form=f>';
            const f=document.getElementById('f'),a=document.getElementById('a'),b=document.getElementById('b'),before=document.getElementById('before'),after=document.getElementById('after');
            const list=f.elements;
            check(list instanceof HTMLFormControlsCollection&&list instanceof HTMLCollection,'collection inheritance');
            check(list===f.elements&&list.length===4&&f.length===4,'identity/length');
            check([...list].map(e=>e.id).join()==='before,a,b,after','tree order');
            check(HTMLCollection.prototype.item.call(list,1)===a,'inherited item');
            const radios=list.namedItem('g');
            check(radios instanceof RadioNodeList&&radios instanceof NodeList&&radios.length===2,'named brand');
            check(list.g instanceof RadioNodeList&&list.namedItem('before')===before&&list.namedItem('missing')===null,'named lookup');
            check(HTMLCollection.prototype.namedItem.call(list,'g')===a,'parent algorithm');
            check(NodeList.prototype.item.call(radios,0)===a&&radios.value==='','inherited radio item/value');
            radios.value='B';check(b.checked&&radios.value==='B','radio setter');
            a.remove();check(radios.length===1&&radios[0]===b&&list.namedItem('g')===b,'held live radio list');
            b.name='other';check(radios.length===0&&radios.value==='','name mutation');
            before.removeAttribute('form');check(list.length===2,'external reassociation');
            f.remove();check(list.length===1&&list[0]===b,'detached owner');
            'ok'
        "#);
    }

    #[test]
    fn radio_node_list_value_conversion_and_receiver_brands() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)},reject=fn=>{let threw=false;try{fn()}catch(e){threw=e instanceof TypeError}check(threw,'brand rejection')};
            document.body.innerHTML='<form id=f><input name=g id=a type=radio><input name=g id=b type=radio value=""><input name=g id=c type=radio value=x><button name=g value=x></button></form>';
            const f=document.getElementById('f'),a=document.getElementById('a'),b=document.getElementById('b'),c=document.getElementById('c'),list=f.elements.namedItem('g');
            let events=0;f.oninput=f.onchange=()=>events++;
            list.value='on';check(a.checked&&list.value==='on','missing value default');
            list.value='';check(b.checked&&!a.checked&&list.value==='','empty value');
            list.value={toString(){c.value='changed';return 'changed'}};check(c.checked&&list.value==='changed','conversion before live query');
            list.value='absent';check(c.checked&&events===0,'no match and no events');
            reject(()=>Object.getOwnPropertyDescriptor(RadioNodeList.prototype,'value').get.call(f.elements));
            reject(()=>HTMLFormControlsCollection.prototype.namedItem.call(document.children,'g'));
            reject(()=>NodeList.prototype.item.call(f.elements,0));
            reject(()=>new RadioNodeList());reject(()=>new HTMLFormControlsCollection());
            'ok'
        "#);
    }

    #[test]
    fn fieldset_elements_are_live_descendants_not_form_members() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            document.body.innerHTML='<form id=f><fieldset id=fs disabled name=group><legend><input id=a name=g></legend><fieldset id=nested><input id=b name=g form=other type=image></fieldset><object id=o></object><output id=out></output><select id=s></select><textarea id=t></textarea><button id=bt></button><keygen></keygen><meter></meter><div id=host></div></fieldset></form><form id=other></form><input form=f id=external>';
            const fs=document.getElementById('fs'),a=document.getElementById('a'),b=document.getElementById('b'),f=document.getElementById('f');
            const list=fs.elements;
            check(fs instanceof HTMLFieldSetElement&&fs.type==='fieldset'&&fs.form===f,'fieldset brand/type/owner');
            check(fs.disabled&&fs.name==='group','reflection');fs.disabled=false;fs.name='changed';
            check(!fs.hasAttribute('disabled')&&fs.getAttribute('name')==='changed','reflection setters');
            check(list===fs.elements&&list instanceof HTMLCollection&&!(list instanceof HTMLFormControlsCollection),'collection identity/brand');
            check([...list].map(x=>x.id).join()==='a,nested,b,o,out,s,t,bt','listed tree order');
            check(list.namedItem('g')===a&&list.g===a,'duplicate names return first element, not radio list');
            b.setAttribute('form','absent');check(list.length===8,'form ownership irrelevant');
            fs.prepend(b);check(list[0]===b&&list.namedItem('g')===b,'live reorder');
            for(const mode of ['open','closed']) {
                const host=document.createElement('div');fs.append(host);
                host.attachShadow({mode}).append(document.createElement('input'));
            }
            check(list.length===8,'no shadow descendants');
            fs.remove();check(list.length===8&&fs.form===null,'detached membership');
            b.remove();check(list.length===7&&list.namedItem('g')===a,'live removal');
            let rejected=false;try{Object.getOwnPropertyDescriptor(HTMLFieldSetElement.prototype,'elements').get.call(f)}catch(e){rejected=e instanceof TypeError}
            check(rejected,'receiver brand');
            'ok'
        "#);
    }

    #[test]
    fn form_collection_domstring_conversion_preserves_utf16() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const f=document.createElement('form');document.body.append(f);
            const a=document.createElement('input'),b=document.createElement('input');
            a.type=b.type='radio';f.append(a,b);
            for(const value of ['\uD800','\uDC00','a\uD800b','\uD83D\uDE00','\uFFFD']) {
                a.name=b.name=value;a.value=value;b.value='other';
                const list=f.elements.namedItem(value);
                check(list instanceof RadioNodeList&&list.length===2,'name code units');
                check(f.elements[value] instanceof RadioNodeList,'named property code units');
                check(Object.getOwnPropertyDescriptor(f.elements,value).value.length===2,'descriptor code units');
                check(Object.getOwnPropertyNames(f.elements).includes(value),'enumerated code units');
                check(HTMLCollection.prototype.namedItem.call(f.elements,value)===a,'base namedItem code units');
                b.checked=true;list.value=value;
                check(a.checked&&!b.checked&&list.value===value,'value code units');
                b.checked=true;list.value={toString(){return value}};
                check(a.checked&&list.value===value,'object conversion');
            }
            a.name=b.name='g';a.value='\uD800';b.value='\uFFFD';
            const list=f.elements.namedItem('g');list.value='\uD800';
            check(a.checked&&!b.checked,'lone surrogate distinct from replacement');
            list.value='\uFFFD';check(b.checked&&!a.checked,'replacement distinct from surrogate');
            const sentinel={};let caught;
            try{list.value={toString(){throw sentinel}}}catch(e){caught=e}
            check(caught===sentinel&&b.checked,'conversion exception preserved');
            'ok'
        "#);
    }

    #[test]
    fn tabindex_reflection_uses_html_integer_parsing_and_webidl_conversion() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            const div=document.createElement('div');
            check(div.tabIndex===-1,'default div');
            for(const tag of ['a','area','button','frame','iframe','input','object','select','textarea'])check(document.createElement(tag).tabIndex===0,tag);
            for(const [value,expected] of [[' +12junk',12],['-2.5',-2],['0x10',0],['',-1],['Infinity',-1],['2147483648',-1],['\u00a01',-1],['-2147483648',-2147483648]]) {
                div.setAttribute('tabindex',value);check(div.tabIndex===expected,value);
            }
            div.tabIndex=4294967295;check(div.tabIndex===-1&&div.getAttribute('tabindex')==='-1','WebIDL long');
            div.tabIndex=2.9;check(div.tabIndex===2,'truncate');
            let converted=0;div.tabIndex={valueOf(){converted++;return 3}};check(div.tabIndex===3&&converted===1,'numeric conversion');
            const details=document.createElement('details'),a=document.createElement('summary'),b=document.createElement('summary');details.append(a,b);
            check(a.tabIndex===0&&b.tabIndex===-1,'first summary');a.remove();check(b.tabIndex===0,'live summary');
            'ok'
        "#);
    }

    #[test]
    fn label_interfaces_and_live_lists_follow_tree_and_type_changes() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            document.body.innerHTML='<form id=f><label id=l><input id=c></label></form><label id=e for=c></label>';
            const f=document.getElementById('f'),l=document.getElementById('l'),c=document.getElementById('c'),e=document.getElementById('e');
            const labels=c.labels;
            check(l instanceof HTMLLabelElement && labels instanceof NodeList,'brands');
            check(labels===c.labels && labels.length===2 && labels[0]===l && labels[1]===e,'live ordered identity');
            check(l.control===c && e.control===c && c.form===f && e.form===f,'associations');
            c.type='hidden';check(c.labels===null && labels.length===0 && l.control===null,'hidden');
            c.type='text';check(c.labels===labels && labels.length===2,'restored identity');
            const first=document.createElement('input');l.prepend(first);
            check(l.control===first && labels.length===1 && first.labels[0]===l,'first descendant');
            first.remove();l.remove();check(labels.length===1 && labels[0]===l && e.control===null && l.form===null,'detached');
            const inner=document.createElement('label');inner.htmlFor='c';l.append(inner);
            check(inner.getAttribute('for')==='c' && inner.control===c && labels.length===2,'detached explicit');
            inner.htmlFor='';check(inner.control===null && labels.length===1,'empty for');
            const host=document.createElement('div');document.body.append(host);
            const shadow=host.attachShadow({mode:'closed'});shadow.append(l);
            check(labels.length===1 && e.control===null,'shadow isolation');
            const blocker=document.createElement('span');blocker.id='c';shadow.prepend(blocker);
            inner.htmlFor='c';check(inner.control===null,'first duplicate ID must be labelable');
            blocker.remove();check(inner.control===c && labels.length===2,'duplicate removal');
            c.remove();check(labels.length===0,'owner root changes');
            for(const tag of ['button','meter','output','progress','select','textarea']) {
                const label=document.createElement('label'),control=document.createElement(tag);
                label.append(control);f.append(label);
                check(control.labels instanceof NodeList && control.labels[0]===label && label.control===control,tag+' labels');
                check(label.form===(tag==='meter'||tag==='progress'?null:f),tag+' form');
            }
            let rejected=false;try{Object.getOwnPropertyDescriptor(HTMLLabelElement.prototype,'control').get.call(c)}catch(e){rejected=true}
            check(rejected,'receiver brand');
            'ok'
        "#);
    }

    #[test]
    fn checkbox_activation_cancellation_event_order_and_dirty_state() {
        run(r#"
            const c = document.createElement('input');c.type='checkbox';document.body.append(c);
            const log=[]; const check=(v,m)=>{if(!v)throw Error(m)};
            c.indeterminate=true;
            c.onclick=e=>{log.push('click');check(c.checked&&!c.indeterminate,'preactivation');c.click()};
            c.oninput=e=>{check(e.isTrusted&&e.bubbles&&!e.cancelable&&e.composed,'input flags');log.push('input')};
            c.onchange=e=>{check(e.isTrusted&&e.bubbles&&!e.cancelable&&!e.composed,'change flags');log.push('change')};
            c.click();check(log.join()==='click,input,change','event order/recursion');
            check(c.checked&&!c.defaultChecked&&!c.hasAttribute('checked'),'attribute separation');
            c.onclick=e=>{check(!c.checked&&!c.indeterminate,'cancel preactivation');e.preventDefault()};
            c.indeterminate=true;c.click();check(c.checked&&c.indeterminate,'restoration');
            check(log.length===3,'no canceled input/change');
            c.onclick=null;c.dispatchEvent(new Event('click'));check(c.checked,'plain Event is not activation');
            c.dispatchEvent(new MouseEvent('click',{cancelable:true}));check(!c.checked,'MouseEvent activation');
            c.defaultChecked=true;check(!c.checked,'dirty flag');
            const copy=c.cloneNode();check(!copy.checked&&copy.defaultChecked,'clone state');
            'ok'
        "#);
    }

    #[test]
    fn radio_activation_restores_prior_selection_and_observes_new_group() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            document.body.innerHTML='<input id=a type=radio name=g checked><input id=b type=radio name=g>';
            const a=document.getElementById('a'),b=document.getElementById('b');
            b.onclick=e=>{check(!a.checked&&b.checked,'preactivation group');e.preventDefault()};
            b.click();check(a.checked&&!b.checked,'restore group');
            b.onclick=e=>{a.name='elsewhere';e.preventDefault()};
            b.click();check(!a.checked&&!b.checked,'do not restore former group');
            let changes=0;b.onchange=()=>changes++;b.onclick=null;b.click();b.click();
            check(b.checked&&changes===1,'selected radio no duplicate change');
            'ok'
        "#);
    }

    #[test]
    fn label_forwarding_preserves_metadata_and_initiated_inert_activation() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            document.body.innerHTML='<div id=outer><label id=l><span id=s>choose</span><input id=c type=checkbox><button id=b>other</button></label></div>';
            const outer=document.getElementById('outer'),l=document.getElementById('l'),c=document.getElementById('c'),s=document.getElementById('s'),b=document.getElementById('b');
            let clicks=0;c.onclick=e=>{clicks++;check(!e.isTrusted,'synthetic trust');check(e.ctrlKey&&e.pointerId===42,'metadata')};
            outer.onclick=()=>outer.inert=true;
            s.dispatchEvent(new PointerEvent('click',{bubbles:true,cancelable:true,ctrlKey:true,pointerId:42}));
            check(c.checked&&clicks===1&&outer.inert,'initiated label activation');
            c.onclick=()=>clicks++;b.click();check(clicks===1,'interactive descendant');
            l.onclick=e=>e.preventDefault();l.click();check(clicks===1,'canceled label');
            l.onclick=null;c.disabled=true;l.click();check(clicks===1,'disabled control');
            'ok'
        "#);
    }

    #[test]
    fn hidden_inputs_and_objects_do_not_suppress_label_forwarding() {
        run(r#"
            for (const markup of ['<input type=hidden>', '<input type=hidden tabindex=0>', '<object usemap=""></object>']) {
                document.body.innerHTML='<label><button id=b></button>'+markup+'</label>';
                const button=document.getElementById('b'),target=button.nextElementSibling;
                let clicks=0;button.onclick=()=>clicks++;
                target.dispatchEvent(new MouseEvent('click',{bubbles:true}));
                const span=document.createElement('span');target.append(span);span.click();
                if(clicks!==2)throw Error(markup+': '+clicks);
            }
            'ok'
        "#);
    }

    #[test]
    fn reset_controls_share_native_reset_and_cancelable_activation() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            for(const tag of ['button','input']){
                document.body.innerHTML=`<form id=f><input id=value value=seed><${tag} id=reset type=reset></${tag}></form>`;
                const f=document.getElementById('f'),value=document.getElementById('value'),button=document.getElementById('reset');
                let calls=0;f.addEventListener('reset',()=>calls++);
                f.reset=()=>{throw Error('author reset method invoked')};
                value.value='edited';button.click();check(value.value==='seed'&&calls===1,'native reset activation');
                value.value='edited';button.addEventListener('click',e=>e.preventDefault(),{once:true});button.click();
                check(value.value==='edited'&&calls===1,'cancelled click');
                f.addEventListener('reset',e=>e.preventDefault(),{once:true});button.click();
                check(value.value==='edited'&&calls===2,'cancelled reset');
                button.disabled=true;button.dispatchEvent(new MouseEvent('click',{bubbles:true,cancelable:true}));
                check(value.value==='edited'&&calls===2,'disabled reset');
                button.disabled=false;button.dispatchEvent(new MouseEvent('click',{bubbles:true,cancelable:true}));
                check(value.value==='seed'&&calls===3,'dispatch activation');
            }
            'ok'
        "#);
    }

    #[test]
    fn form_reset_restores_defaults_after_cancelable_event_and_external_controls() {
        run(r#"
            const check=(v,m)=>{if(!v)throw Error(m)};
            document.body.innerHTML='<form id=f><input id=c name=c type=checkbox checked value=yes></form><input id=x form=f type=checkbox checked>';
            const f=document.getElementById('f'),c=document.getElementById('c'),x=document.getElementById('x');
            c.checked=false;x.checked=false;
            f.onreset=e=>{check(!c.checked&&e.cancelable,'reset before change');e.preventDefault()};
            f.reset();check(!c.checked&&!x.checked,'canceled reset');
            f.onreset=()=>f.reset();f.reset();check(c.checked&&x.checked,'reset defaults and recursion');
            c.defaultChecked=false;check(!c.checked,'reset clears dirty');
            c.checked=true;check(new FormData(f).get('c')==='yes','FormData live checkedness');
            c.checked=false;check(!new FormData(f).has('c'),'FormData excludes unchecked default');
            'ok'
        "#);
    }
}

// Script dispatch has no caller-held native receipt; retain its interrupted
// default separately after the dispatch itself has completed.
enum DefaultJob {
    Activation(Box<Finish>),
    Reset(Box<retained_form_reset::Reset>),
    ForcedClick(Box<retained_activation::Click>),
}
impl DefaultJob {
    fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        match self {
            Self::Activation(job) => job.poll(scope),
            Self::Reset(job) => job.poll(scope),
            Self::ForcedClick(job) => job.poll(scope).map(|result| result.is_some()),
        }
    }
    fn cancel(&mut self) {
        match self {
            Self::Activation(job) => job.cancel(),
            Self::Reset(job) => job.cancel(),
            Self::ForcedClick(job) => job.cancel(),
        }
    }
    fn retry_safe(&self) -> bool {
        match self {
            Self::Activation(job) => job.retry_safe(),
            Self::Reset(_) => true,
            Self::ForcedClick(job) => job.retry_safe(),
        }
    }
}
#[derive(Default)]
pub(super) struct Registry {
    reserved: usize,
    queue: VecDeque<(u64, u64, DefaultJob)>,
    retry: Option<std::time::Instant>,
    polling: bool,
}
fn is_click(
    scope: &mut v8::PinScope,
    source: v8::Local<v8::Object>,
) -> Result<bool, V8BindingError> {
    let id = event_id(scope, source)?;
    let owner = binding_state_for_receiver(scope, source)
        .ok_or(V8BindingError::Allocation("activation event owner"))?;
    let click = owner
        .events
        .borrow()
        .get(&id)
        .is_some_and(|event| event.event_type == "click");
    Ok(click && mouse_payload(scope, source).is_some())
}
// Keep the continuation off the native callback stack. Each synchronous
// nested click retains this owner through its NavigateEvent callbacks.
pub(super) struct ScriptDefault {
    owner: Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    job: Option<Box<Finish>>,
}
impl ScriptDefault {
    pub(super) fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        target: NodeId,
        source: v8::Local<v8::Object>,
    ) -> Result<Option<Self>, V8BindingError> {
        if !is_click(scope, source)? {
            return Ok(None);
        }
        {
            let mut registry = state.activation_jobs.borrow_mut();
            if registry.queue.len().saturating_add(registry.reserved) >= 64 {
                return Err(V8BindingError::Allocation("retained activation limit"));
            }
            let additional = registry.reserved + 1;
            registry
                .queue
                .try_reserve(additional)
                .map_err(|_| V8BindingError::Allocation("retained activation queue"))?;
            registry.reserved += 1;
        }
        let mut pending = Self {
            owner: state.clone(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            job: None,
        };
        pending.job = Some(Box::new(Finish::new(
            scope, state, None, target, source, false,
        )?));
        Ok(Some(pending))
    }
    pub(super) fn activation(&mut self, activation: Option<Activation>) {
        self.job.as_mut().unwrap().set_activation(activation);
    }
    pub(super) fn finish(
        mut self,
        scope: &mut v8::PinScope,
        accepted: bool,
    ) -> Result<(), V8BindingError> {
        let mut job = self.job.take().unwrap();
        job.set_accepted(accepted);
        let result = job.poll(scope);
        if !matches!(result, Ok(true))
            && job.retry_safe()
            && queue_owner_current(&self.owner, self.generation, self.epoch)
        {
            let mut registry = self.owner.activation_jobs.borrow_mut();
            registry
                .queue
                .push_back((self.generation, self.epoch, DefaultJob::Activation(job)));
            registry.retry = Some(std::time::Instant::now() + std::time::Duration::from_millis(10));
        }
        match result {
            Ok(true) => Ok(()),
            Ok(false) => Err(V8BindingError::JavaScriptException),
            Err(error) => Err(error),
        }
    }
}
impl Drop for ScriptDefault {
    fn drop(&mut self) {
        self.owner.activation_jobs.borrow_mut().reserved -= 1;
        if let Some(job) = &mut self.job {
            job.cancel();
        }
    }
}
/// Script reset shares the exact control/event continuation used by trusted
/// activation. Reserve the retry slot before dispatch can run author code.
pub(super) fn reset(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    {
        let mut registry = state.activation_jobs.borrow_mut();
        if registry.queue.len().saturating_add(registry.reserved) >= 64 {
            return Err(V8BindingError::Allocation("retained activation limit"));
        }
        let additional = registry.reserved + 1;
        registry
            .queue
            .try_reserve(additional)
            .map_err(|_| V8BindingError::Allocation("retained activation queue"))?;
        registry.reserved += 1;
    }
    struct Reservation<'a>(&'a V8BindingState);
    impl Drop for Reservation<'_> {
        fn drop(&mut self) {
            self.0.activation_jobs.borrow_mut().reserved -= 1;
        }
    }
    let _reservation = Reservation(state);
    let generation = state.document_generation();
    let epoch = state.document_stream_epoch();
    let mut job = retained_form_reset::Reset::new(scope, state, node)?;
    let result = job.poll(scope);
    if !matches!(result, Ok(true)) && queue_owner_current(state, generation, epoch) {
        let mut registry = state.activation_jobs.borrow_mut();
        registry
            .queue
            .push_back((generation, epoch, DefaultJob::Reset(Box::new(job))));
        registry.retry = Some(std::time::Instant::now() + std::time::Duration::from_millis(10));
    }
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(V8BindingError::JavaScriptException),
        Err(error) => Err(error),
    }
}
/// Explicit driver force bypasses geometry, never native identity or disabledness.
/// The exact click event/default cursor survives interrupted publication.
pub(super) fn forced_click(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    if !queue_owner_current(
        state,
        state.document_generation(),
        state.document_stream_epoch(),
    ) || state.execution_suspended()
    {
        return Err(V8BindingError::Allocation(
            "retired forced activation document",
        ));
    }
    {
        let dom = state.dom.borrow();
        if !dom.is_connected(node)?
            || dom.node_document(node)? != Some(state.document_root())
            || dom.control_disabled(node)?
        {
            return Err(V8BindingError::Allocation(
                "invalid forced activation target",
            ));
        }
    }
    {
        let mut registry = state.activation_jobs.borrow_mut();
        if registry.queue.len().saturating_add(registry.reserved) >= 64 {
            return Err(V8BindingError::Allocation("retained activation limit"));
        }
        let additional = registry.reserved + 1;
        registry
            .queue
            .try_reserve(additional)
            .map_err(|_| V8BindingError::Allocation("retained activation queue"))?;
        registry.reserved += 1;
    }
    struct Reservation<'a>(&'a V8BindingState);
    impl Drop for Reservation<'_> {
        fn drop(&mut self) {
            self.0.activation_jobs.borrow_mut().reserved -= 1;
        }
    }
    let _reservation = Reservation(state);
    let generation = state.document_generation();
    let epoch = state.document_stream_epoch();
    let Some(job) = retained_activation::Click::new(scope, state, node)? else {
        return Ok(());
    };
    let mut job = Box::new(job);
    state.notify_user_activation();
    let result = job.poll(scope);
    if !matches!(result, Ok(Some(_)))
        && job.retry_safe()
        && queue_owner_current(state, generation, epoch)
    {
        let mut registry = state.activation_jobs.borrow_mut();
        registry
            .queue
            .push_back((generation, epoch, DefaultJob::ForcedClick(job)));
        registry.retry = Some(std::time::Instant::now() + std::time::Duration::from_millis(10));
    }
    match result {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(V8BindingError::JavaScriptException),
        Err(error) => Err(error),
    }
}
fn queue_owner_current(state: &V8BindingState, generation: u64, epoch: u64) -> bool {
    !state.execution_retired()
        && !state.execution_tasks_stopped()
        && state.document_generation() == generation
        && state.document_stream_epoch() == epoch
}
pub(super) fn delay(state: &V8BindingState) -> Option<std::time::Duration> {
    let registry = state.activation_jobs.borrow();
    (!registry.queue.is_empty() && !state.execution_suspended()).then(|| {
        registry
            .retry
            .map_or(std::time::Duration::ZERO, |deadline| {
                deadline.saturating_duration_since(std::time::Instant::now())
            })
    })
}
// Exact empty-queue check; future retries and other owners remain active.
pub(super) fn queue_empty(state: &V8BindingState) -> bool {
    state.activation_jobs.borrow().queue.is_empty()
}

pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.activation_jobs.borrow().polling || !delay(state).is_some_and(|delay| delay.is_zero())
    {
        return Ok(0);
    }
    let Some((generation, epoch, mut job)) = state.activation_jobs.borrow_mut().queue.pop_front()
    else {
        return Ok(0);
    };
    if !queue_owner_current(state, generation, epoch) {
        job.cancel();
        return Ok(1);
    }
    state.activation_jobs.borrow_mut().polling = true;
    struct Guard<'a>(&'a V8BindingState);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.activation_jobs.borrow_mut().polling = false
        }
    }
    let _guard = Guard(state);
    match job.poll(scope) {
        Ok(true) => {
            state.activation_jobs.borrow_mut().retry = None;
            Ok(1)
        }
        result => {
            if job.retry_safe() && queue_owner_current(state, generation, epoch) {
                let mut registry = state.activation_jobs.borrow_mut();
                registry.queue.push_front((generation, epoch, job));
                registry.retry =
                    Some(std::time::Instant::now() + std::time::Duration::from_millis(10));
            }
            result.map(|_| 0)
        }
    }
}
pub(super) fn retire(state: &V8BindingState) {
    let jobs = std::mem::take(&mut state.activation_jobs.borrow_mut().queue);
    for (_, _, mut job) in jobs {
        job.cancel();
    }
    state.activation_jobs.borrow_mut().retry = None;
}

#[cfg(test)]
mod script_default_retention_tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) {
        super::super::test_eval::eval(scope, source);
    }
    #[test]
    fn nested_script_links_keep_synchronous_order_and_latest_native_admission() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<a id=a href=/a>A</a><a id=b href=/b>B</a><a id=c href=/c>C</a>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(253), 1).unwrap();
        state.commit_location("https://links.test/start").unwrap();
        crate::navigation_api::sync_history(
            scope,
            vec![crate::navigation_api::NavigationHistorySnapshot {
                scroll: Default::default(),
                key: "initial-key".into(),
                id: "initial-entry".into(),
                url: "https://links.test/start".into(),
                document_id: "initial-document".into(),
                state: None,
                history_state: None,
            }],
            0,
            "push",
        )
        .unwrap();
        eval(
            scope,
            "globalThis.order=[];globalThis.errors=[];addEventListener('error',e=>errors.push(e.message));navigation.onnavigate=e=>{const id=e.sourceElement.id;order.push('enter:'+id);if(id==='a')b.click();else if(id==='b')c.click();order.push('leave:'+id)};a.click()",
        );
        assert!(super::super::test_eval::eval(scope,"order.join(',')==='enter:a,enter:b,enter:c,leave:c,leave:b,leave:a'&&errors.length===0").is_true());
        assert_eq!(state.activation_jobs.borrow().reserved, 0);
        assert!(state.activation_jobs.borrow().queue.is_empty());
        assert_eq!(state.reserved_default_actions.get(), 0);
        let actions = state.pending_default_actions.borrow();
        assert_eq!(actions.len(), 1);
        let crate::v8_bindings::PendingDefaultAction::Click(target) = actions.front().unwrap()
        else {
            panic!("missing latest link")
        };
        let navigation = target.navigation.as_ref().unwrap();
        assert_eq!(navigation.url, "https://links.test/c");
        let admission = navigation.admission.as_ref().unwrap();
        assert!(admission.operation.as_ref().is_some_and(|token| {
            crate::navigation_api::native_navigation_matches(&state, token)
        }));
        drop(actions);
        state.pending_default_actions.borrow_mut().clear();
        eval(
            scope,
            "navigation.onnavigate=e=>e.preventDefault();a.click()",
        );
        assert!(state.pending_default_actions.borrow().is_empty());
        assert_eq!(state.activation_jobs.borrow().reserved, 0);
    }
    #[test]
    fn script_form_reset_retains_event_and_control_suffix_without_replay() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<form id=f><input id=t value=default><input id=c type=checkbox checked></form>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(254), 1).unwrap();
        eval(
            scope,
            "t.value='changed';c.checked=false;globalThis.trace=[];f.addEventListener('reset',e=>{globalThis.original=e;trace.push('first');f.reset()});f.addEventListener('reset',e=>{if(e!==original)throw Error('reset event replaced');trace.push('second')})",
        );
        native_platform::publication::fail_after_next_callback();
        eval(
            scope,
            "try{f.reset()}catch{};if(t.value!=='changed'||c.checked||trace.join()!=='first')throw Error('reset advanced after interruption')",
        );
        assert_eq!(state.activation_jobs.borrow().queue.len(), 1);
        assert_eq!(state.activation_jobs.borrow().reserved, 0);
        assert_eq!(state.reset_in_progress.borrow().len(), 1);
        state.activation_jobs.borrow_mut().retry = None;
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert_eq!(poll(scope, &state).unwrap(), 0);
        eval(
            scope,
            "if(t.value!=='default'||!c.checked||trace.join()!=='first,second')throw Error('reset suffix lost or repeated')",
        );
        assert!(state.reset_in_progress.borrow().is_empty());
    }
    struct RetireResetOwner(Rc<V8BindingState>);
    fn retire_reset_owner(
        scope: &mut v8::PinScope,
        _args: v8::FunctionCallbackArguments,
        _rv: v8::ReturnValue,
    ) {
        scope
            .get_current_context()
            .get_slot::<RetireResetOwner>()
            .unwrap()
            .0
            .retire_execution();
    }
    #[test]
    fn callback_retirement_cannot_requeue_an_interrupted_reset() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<form id=f><input id=t value=default></form>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(254), 1).unwrap();
        context.set_slot(Rc::new(RetireResetOwner(state.clone())));
        let callback = v8::Function::new(scope, retire_reset_owner).unwrap();
        let key = v8::String::new(scope, "retireResetOwner").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), callback.into());
        eval(
            scope,
            "t.value='changed';f.addEventListener('reset',retireResetOwner)",
        );
        native_platform::publication::fail_after_next_callback();
        let form = state.dom.borrow().find_by_id("f").unwrap();
        assert!(reset(scope, &state, form).is_err());
        assert!(state.execution_retired());
        assert!(state.activation_jobs.borrow().queue.is_empty());
        assert_eq!(state.activation_jobs.borrow().reserved, 0);
        assert!(state.reset_in_progress.borrow().is_empty());
        let control = state.dom.borrow().find_by_id("t").unwrap();
        assert_eq!(
            state.dom.borrow().control_value(control).unwrap(),
            "changed"
        );
    }
    #[test]
    fn cancelled_script_reset_releases_guard_and_never_applies_control_suffix() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<form id=f><input id=t value=default></form>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(254), 1).unwrap();
        eval(
            scope,
            "t.value='changed';globalThis.calls=0;f.addEventListener('reset',()=>calls++,{once:true})",
        );
        native_platform::publication::fail_after_next_callback();
        eval(scope, "try{f.reset()}catch{}");
        assert_eq!(state.activation_jobs.borrow().queue.len(), 1);
        retire(&state);
        assert!(state.activation_jobs.borrow().queue.is_empty());
        assert!(state.reset_in_progress.borrow().is_empty());
        assert_eq!(poll(scope, &state).unwrap(), 0);
        eval(
            scope,
            "if(t.value!=='changed'||calls!==1)throw Error('retired reset committed');f.reset();if(t.value!=='default'||calls!==1)throw Error('reset guard leaked')",
        );
    }
    #[test]
    fn unrelated_trusted_input_cannot_open_file_picker() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t type=file>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(252), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        let event = create_event_object(scope, &state, "input", true, false).unwrap();
        let id = event_id(scope, event).unwrap();
        state.events.borrow_mut().get_mut(&id).unwrap().is_trusted = true;
        dispatch_event(scope, &state, target, event).unwrap();
        assert!(
            state
                .dom
                .borrow()
                .file_picker_snapshot(target)
                .unwrap()
                .is_none()
        );
        assert!(state.activation_jobs.borrow().queue.is_empty());
    }
    #[test]
    fn default_slot_failure_precedes_checkbox_preactivation_and_listeners() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t type=checkbox>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(253), 1).unwrap();
        eval(scope, "globalThis.clicks=0;t.onclick=()=>clicks++");
        state.activation_jobs.borrow_mut().reserved = 64;
        eval(
            scope,
            "try{t.click()}catch{};if(t.checked||clicks)throw Error('preflight was after activation')",
        );
        state.activation_jobs.borrow_mut().reserved = 0;
    }
    #[test]
    fn details_interface_reflects_native_state() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<details id=d></details>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(254), 1).unwrap();
        eval(
            scope,
            "if(!(d instanceof HTMLDetailsElement)||d.open||d.name!=='')throw Error('details defaults');d.open=true;if(!d.hasAttribute('open'))throw Error('open reflected');d.removeAttribute('open');if(d.open)throw Error('open content reflection');d.name='group';if(d.getAttribute('name')!=='group')throw Error('name reflected')",
        );
    }
    #[test]
    fn script_command_default_retains_suffix_after_dispatch_has_returned() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<button id=b type=button command=toggle commandfor=d>toggle</button><details id=d></details>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(254), 1).unwrap();
        eval(
            scope,
            "globalThis.trace=[];d.addEventListener('command',e=>{globalThis.original=e;trace.push('first')});d.addEventListener('command',e=>{if(e!==original)throw Error('event replaced');trace.push('second')})",
        );
        native_platform::publication::fail_after_next_callback();
        eval(
            scope,
            "try{b.click()}catch{};if(d.open||trace.join()!=='first')throw Error('command default advanced')",
        );
        assert_eq!(state.activation_jobs.borrow().queue.len(), 1);
        assert_eq!(state.activation_jobs.borrow().reserved, 0);
        state.activation_jobs.borrow_mut().retry = None;
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert_eq!(poll(scope, &state).unwrap(), 0);
        eval(
            scope,
            "if(!d.open||trace.join()!=='first,second')throw Error('script default replay')",
        );
    }
}

#[cfg(test)]
mod forced_activation_tests {
    use super::*;
    #[test]
    fn forced_hidden_target_uses_native_event_and_preserves_canceled_defaults() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<input id=target type=checkbox hidden><button id=disabled disabled></button><button id=outside></button>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let target = dom.borrow().find_by_id("target").unwrap();
        let disabled = dom.borrow().find_by_id("disabled").unwrap();
        super::super::test_eval::eval(
            scope,
            "globalThis.events=[];target.click=()=>{throw Error('author method invoked')};target.addEventListener('click',e=>{events.push([e.isTrusted,target.checked,e.target===target].join(':'));e.preventDefault()});outside.onclick=()=>{throw Error('wrong target')}",
        );
        forced_click(scope, &state, target).unwrap();
        assert!(
            super::super::test_eval::eval(
                scope,
                "events.join()==='true:true:true'&&!target.checked"
            )
            .is_true()
        );
        assert!(forced_click(scope, &state, disabled).is_err());
        super::super::test_eval::eval(scope, "target.remove()");
        assert!(forced_click(scope, &state, target).is_err());
    }
    #[test]
    fn forced_interruption_keeps_original_click_and_remaining_listeners() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<button id=b hidden></button>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let b = dom.borrow().find_by_id("b").unwrap();
        super::super::test_eval::eval(
            scope,
            "globalThis.log=[];b.addEventListener('click',e=>{globalThis.first=e;log.push('first')});b.addEventListener('click',e=>{if(e!==first)throw Error('replayed click');log.push('second')})",
        );
        native_platform::publication::fail_after_next_callback();
        assert!(forced_click(scope, &state, b).is_err());
        assert!(super::super::test_eval::eval(scope, "log.join()==='first'").is_true());
        state.activation_jobs.borrow_mut().retry = None;
        poll(scope, &state).unwrap();
        assert!(super::super::test_eval::eval(scope, "log.join()==='first,second'").is_true());
        assert_eq!(poll(scope, &state).unwrap(), 0);
    }
}
