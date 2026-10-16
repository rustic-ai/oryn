//! Retained focus transition used by navigation completion. Native state changes
//! and each event are separate stages; author focus changes supersede the tail.
use super::*;
pub(in super::super) struct Transition {
    previous: Option<NodeId>,
    target: NodeId,
    revision: Option<u64>,
    stage: u8,
    event: Option<publication::Notification>,
    root: bool,
}
impl Transition {
    pub(in super::super) fn new(state: &V8BindingState, target: NodeId) -> Self {
        Self {
            previous: state.dom.borrow().focused(),
            target,
            revision: None,
            stage: 0,
            event: None,
            root: target == state.document_root(),
        }
    }
    pub(in super::super) fn cancel(&self, state: &V8BindingState) {
        if let Some(event) = &self.event {
            event.cancel(state);
        }
    }
    pub(in super::super) fn resume(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<bool, V8BindingError> {
        if self.previous == Some(self.target) {
            return Ok(true);
        }
        loop {
            if scope.is_execution_terminating() {
                return Ok(false);
            }
            if state.execution_retired() {
                return Ok(true);
            }
            if let Some(event) = self.event.as_mut() {
                if !event.resume(scope, state)? {
                    return Ok(false);
                }
                event.finish(scope, state);
                self.event = None;
                self.stage += 1;
                continue;
            }
            match self.stage {
                0 => {
                    let revision = state.focus_transition_revision.get().wrapping_add(1);
                    state.dom.borrow_mut().focus(None)?;
                    state.focus_transition_revision.set(revision);
                    self.revision = Some(revision);
                    self.stage = 1;
                }
                1 => {
                    if let Some(previous) =
                        self.previous.filter(|node| *node != state.document_root())
                    {
                        // Retain event and wrapper before consuming the dirty flag.
                        let wrapper = wrap_node(scope, state, previous)?;
                        let event = create_event_object(scope, state, "change", true, false)?;
                        let pending = publication::pending_dom_dispatch(
                            scope, state, wrapper, "change", event,
                        );
                        if state.dom.borrow_mut().take_control_change(previous)? {
                            self.event = Some(pending);
                            continue;
                        }
                    }
                    self.stage = 2;
                }
                2 | 3 => {
                    if let Some(previous) =
                        self.previous.filter(|node| *node != state.document_root())
                    {
                        let name = if self.stage == 2 { "blur" } else { "focusout" };
                        let wrapper = wrap_node(scope, state, previous)?;
                        let event = create_focus_event_object(
                            scope,
                            state,
                            name,
                            (!self.root).then_some(self.target),
                        )?;
                        self.event = Some(publication::pending_dom_dispatch(
                            scope, state, wrapper, name, event,
                        ));
                    } else {
                        self.stage += 1;
                    }
                }
                4 => {
                    if self.revision != Some(state.focus_transition_revision.get())
                        || !state.dom.borrow().is_connected(self.target)?
                        || state.dom.borrow().is_inert(self.target)?
                        || !self.root && css_suppresses_focus_in_scope(scope, state, self.target)?
                    {
                        return Ok(true);
                    }
                    let selected_control = selection_text::prepare_control(state, self.target)?;
                    state.dom.borrow_mut().focus(Some(self.target))?;
                    if let Some(control) = selected_control {
                        *state.selected_text_control.borrow_mut() = Some(control);
                    }
                    self.stage = 5;
                }
                5 | 6 => {
                    if self.root
                        || self.revision != Some(state.focus_transition_revision.get())
                        || state.dom.borrow().focused() != Some(self.target)
                    {
                        return Ok(true);
                    }
                    let name = if self.stage == 5 { "focus" } else { "focusin" };
                    let wrapper = wrap_node(scope, state, self.target)?;
                    let event = create_focus_event_object(scope, state, name, self.previous)?;
                    self.event = Some(publication::pending_dom_dispatch(
                        scope, state, wrapper, name, event,
                    ));
                }
                _ => {
                    if !self.root
                        && self.revision == Some(state.focus_transition_revision.get())
                        && state.dom.borrow().focused() == Some(self.target)
                        && focus_navigation::delegated_target(scope, state, self.target)?
                            == Some(self.target)
                    {
                        // The target is already focused. This runs the native caret
                        // initialization tail without starting another transition.
                        focus_html_element(scope, state, self.target, false)?;
                    }
                    return Ok(true);
                }
            }
        }
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
    fn focus_transition_retries_remaining_callbacks_without_repeating_blur() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><input id=a><input id=b></body>")
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
            "globalThis.a=document.getElementById('a');globalThis.b=document.getElementById('b');a.value='old';a.select();b.value='new';b.setSelectionRange(0,3);globalThis.calls=[];globalThis.saved=null;a.addEventListener('blur',e=>{saved=e;calls.push('blur1')},{once:true});a.addEventListener('blur',e=>{if(e!==saved)throw Error('identity');calls.push('blur2')});a.addEventListener('focusout',()=>calls.push('out'));b.addEventListener('focus',()=>{if(String(getSelection())!=='new')throw Error('focus published stale control selection');calls.push('focus')});b.addEventListener('focusin',()=>calls.push('in'))",
        );
        let target = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "b")
            .unwrap()
            .unwrap();
        let mut transition = Transition::new(&state, target);
        publication::fail_after_next_callback();
        assert!(transition.resume(scope, &state).is_err());
        assert!(eval(scope, "calls.join(',')==='blur1'").is_true());
        assert!(transition.resume(scope, &state).unwrap());
        assert!(eval(scope,"calls.join(',')==='blur1,blur2,out,focus,in'&&document.activeElement===b&&saved.currentTarget===null&&String(getSelection())==='new'").is_true());
    }
    #[test]
    fn author_focus_during_retained_blur_supersedes_automatic_target() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><input id=a><input id=b><input id=c></body>")
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
            "globalThis.a=document.getElementById('a');globalThis.b=document.getElementById('b');globalThis.c=document.getElementById('c');a.focus();a.addEventListener('blur',()=>c.focus(),{once:true})",
        );
        let target = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "b")
            .unwrap()
            .unwrap();
        let mut transition = Transition::new(&state, target);
        publication::fail_after_next_callback();
        assert!(transition.resume(scope, &state).is_err());
        assert!(transition.resume(scope, &state).unwrap());
        assert!(eval(scope, "document.activeElement===c").is_true());
    }
}
