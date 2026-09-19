//! Native form reset owns its reset event and per-control mutation cursor.
use super::retained_dom_event::Event;
use super::*;

pub(super) struct Reset {
    origin: Rc<V8BindingState>,
    lease: Rc<oryn_dom::TrackedNodeLease>,
    object: v8::Global<v8::Object>,
    event: Option<Event>,
    controls: Option<Vec<(v8::Global<v8::Object>, oryn_dom::TrackedNodeLease, String)>>,
    cursor: usize,
    done: bool,
    active: bool,
}
impl Reset {
    pub(super) fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        node: NodeId,
    ) -> Result<Self, V8BindingError> {
        let lease = Rc::new(oryn_dom::TrackedNodeLease::new(&state.dom, node)?);
        let owners = scope
            .get_slot::<BindingStateRegistry>()
            .map(|registry| {
                registry
                    .instances
                    .borrow()
                    .values()
                    .filter_map(RcWeak::upgrade)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![state.clone()]);
        let nested = owners.iter().any(|owner| {
            owner
                .reset_in_progress
                .borrow()
                .iter()
                .any(|active| active.identity() == lease.identity())
        });
        let object = wrap_node(scope, state, node)?;
        let event = if nested {
            None
        } else {
            let event = create_event_object(scope, state, "reset", true, true)?;
            let id = event_id(scope, event)?;
            state
                .events
                .borrow_mut()
                .get_mut(&id)
                .ok_or(V8BindingError::Allocation("reset event"))?
                .is_trusted = true;
            Some(Event::new(scope, state, node, "reset", event)?)
        };
        if !nested {
            state
                .reset_in_progress
                .borrow_mut()
                .try_reserve(1)
                .map_err(|_| V8BindingError::Allocation("form reset guard"))?;
            state.reset_in_progress.borrow_mut().push(lease.clone());
        }
        Ok(Self {
            origin: state.clone(),
            lease,
            object: v8::Global::new(scope, object),
            event,
            controls: None,
            cursor: 0,
            done: nested,
            active: !nested,
        })
    }
    fn finish(&mut self) {
        self.done = true;
        self.event = None;
        if self.active {
            self.origin
                .reset_in_progress
                .borrow_mut()
                .retain(|active| !Rc::ptr_eq(active, &self.lease));
            self.active = false;
        }
    }
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        if self.done {
            return Ok(true);
        }
        if let Some(event) = self.event.as_mut() {
            let owner = event.owner();
            let Some(accepted) = event.poll(scope, &owner)? else {
                return Ok(false);
            };
            self.event = None;
            if !accepted {
                self.finish();
                return Ok(true);
            }
        }
        if self.controls.is_none() {
            let object = v8::Local::new(scope, &self.object);
            let Some((owner, form)) = node_identity::argument(scope, object.into())? else {
                self.finish();
                return Ok(true);
            };
            let controls = {
                let dom = owner.dom.borrow();
                dom.arena()
                    .descendants_inclusive(dom.control_tree_root(form)?)?
                    .into_iter()
                    .filter(|control| {
                        (dom.is_element(*control, "http://www.w3.org/1999/xhtml", "input")
                            || dom.is_element(*control, "http://www.w3.org/1999/xhtml", "textarea")
                            || dom.is_element(*control, "http://www.w3.org/1999/xhtml", "select")
                            || dom.is_element(*control, "http://www.w3.org/1999/xhtml", "output")
                            || dom.is_form_associated_custom_element(*control))
                            && dom.form_owner(*control).ok().flatten() == Some(form)
                    })
                    .map(|control| {
                        let record = dom.arena().get(control)?;
                        let value = if dom.is_element(
                            control,
                            "http://www.w3.org/1999/xhtml",
                            "textarea",
                        ) {
                            dom.arena().text_content(control)?
                        } else {
                            record
                                .attributes
                                .get_null("value")
                                .cloned()
                                .unwrap_or_default()
                        };
                        Ok((control, value))
                    })
                    .collect::<Result<Vec<_>, DomError>>()?
            };
            let mut retained = Vec::new();
            retained
                .try_reserve(controls.len())
                .map_err(|_| V8BindingError::Allocation("reset controls"))?;
            for (node, value) in controls {
                let lease = oryn_dom::TrackedNodeLease::new(&owner.dom, node)?;
                let object = wrap_node(scope, &owner, node)?;
                retained.push((v8::Global::new(scope, object), lease, value));
            }
            self.controls = Some(retained);
        }
        while self.cursor < self.controls.as_ref().unwrap().len() {
            if scope.is_execution_terminating() {
                return Ok(false);
            }
            let (object, _, value) = &self.controls.as_ref().unwrap()[self.cursor];
            let object = v8::Local::new(scope, object);
            let Some((owner, node)) = node_identity::argument(scope, object.into())? else {
                self.cursor += 1;
                continue;
            };
            if owner.execution_suspended() {
                return Ok(false);
            }
            custom_elements::collect_for_migration(scope, &owner)?;
            if owner.dom.borrow().is_form_associated_custom_element(node) {
                custom_elements::enqueue_form_reset(scope, &owner, node)?;
            } else {
                let mut dom = owner.dom.borrow_mut();
                if dom.is_element(node, "http://www.w3.org/1999/xhtml", "output") {
                    dom.reset_output(node)?;
                } else {
                    native_keyboard::invalidate_control_history(&owner, node);
                    if dom.is_element(node, "http://www.w3.org/1999/xhtml", "select") {
                        dom.reset_select(node)?;
                    } else {
                        dom.reset_control_value(node, value.clone())?;
                        owner.text_control_selections.borrow_mut().remove(&node);
                        if dom.is_element(node, "http://www.w3.org/1999/xhtml", "input") {
                            dom.reset_checkedness(node)?;
                        }
                    }
                }
            }
            self.cursor += 1;
        }
        self.finish();
        Ok(true)
    }
    pub(super) fn cancel(&mut self) {
        if let Some(event) = &self.event {
            event.cancel(&event.owner());
        }
        self.finish();
    }
}
impl Drop for Reset {
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
    fn reset_retry_preserves_guard_event_identity_and_defers_mutation() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<form id=f><input id=t value=default><input id=c type=checkbox checked></form>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(225), 1).unwrap();
        eval(
            scope,
            "t.value='changed';c.checked=false;globalThis.log=[];f.addEventListener('reset',e=>{globalThis.original=e;log.push('first');f.reset()},{once:true});f.addEventListener('reset',e=>log.push(e===original?'second':'wrong'))",
        );
        let form = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "f")
            .unwrap()
            .unwrap();
        let mut reset = Reset::new(scope, &state, form).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(reset.poll(scope).is_err());
        assert!(
            eval(
                scope,
                "log.join()==='first'&&t.value==='changed'&&!c.checked"
            )
            .is_true()
        );
        assert_eq!(state.reset_in_progress.borrow().len(), 1);
        assert!(reset.poll(scope).unwrap());
        assert!(reset.poll(scope).unwrap());
        assert!(
            eval(
                scope,
                "log.join()==='first,second'&&t.value==='default'&&c.checked"
            )
            .is_true()
        );
        assert!(state.reset_in_progress.borrow().is_empty());
    }
}
