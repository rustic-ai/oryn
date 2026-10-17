//! Native text-system command receipt. No synthetic key events are introduced.
use super::*;
use crate::composition::{TextCommand, TextDirection};
enum Work {
    Selection(Box<receipt::SelectionJob>),
    Edit(Box<receipt::EditJob>),
    Focus(Box<super::super::retained_focus::Transition>),
}
pub(in super::super) struct Receipt {
    owner: Rc<V8BindingState>,
    target: oryn_dom::TrackedNodeLease,
    generation: u64,
    epoch: u64,
    command: TextCommand,
    work: Option<Work>,
    done: bool,
}
impl Receipt {
    pub(in super::super) fn new(
        state: Rc<V8BindingState>,
        target: NodeId,
        command: TextCommand,
    ) -> Result<Self, V8BindingError> {
        let lease = oryn_dom::TrackedNodeLease::new(&state.dom, target)?;
        Ok(Self {
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            owner: state,
            target: lease,
            command,
            work: None,
            done: false,
        })
    }
    pub(in super::super) fn retry_safe(&self) -> bool {
        true
    }
    pub(in super::super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<bool, V8BindingError> {
        if self.done {
            return Ok(true);
        }
        if self.owner.execution_suspended() || scope.is_execution_terminating() {
            return Ok(false);
        }
        let (dom, target) = self.target.current();
        if !Rc::ptr_eq(&dom, &self.owner.dom)
            || !live(&self.owner, target, self.generation, self.epoch)
        {
            self.cancel();
            return Ok(true);
        }
        if self.work.is_none() {
            match self.command {
                TextCommand::Tab { backward } => {
                    let next = focus_navigation::next_sequential(scope, &self.owner, backward)?;
                    self.work = Some(Work::Focus(Box::new(
                        super::super::retained_focus::Transition::new(&self.owner, next)?,
                    )));
                }
                TextCommand::Move {
                    direction,
                    unit,
                    extend,
                } => {
                    let direction = match direction {
                        TextDirection::Left => "left",
                        TextDirection::Right => "right",
                        TextDirection::Backward => "backward",
                        TextDirection::Forward => "forward",
                    };
                    let alter = if extend { "extend" } else { "move" };
                    if self.owner.dom.borrow().is_editable_text_control(target) {
                        if let Some(plan) = selection_movement::prepare_control_movement(
                            scope,
                            &self.owner,
                            target,
                            alter,
                            direction,
                            text_unit(unit),
                        )? {
                            plan.commit(scope, &self.owner)?;
                        }
                        self.done = true;
                        return Ok(true);
                    }
                    if let Some(plan) = selection_movement::prepare_movement(
                        scope,
                        &self.owner,
                        alter,
                        direction,
                        text_unit(unit),
                    )? {
                        self.work = Some(Work::Selection(Box::new(receipt::SelectionJob::new(
                            &self.owner,
                            plan,
                        ))))
                    } else {
                        self.done = true;
                        return Ok(true);
                    }
                }
                TextCommand::Delete { backward, unit } => {
                    let description = prepare_edit(
                        scope,
                        &self.owner,
                        target,
                        if backward { "Backspace" } else { "Delete" },
                        NativeKeyboardModifiers::default(),
                        Some(unit),
                    )?;
                    if let Some(description) = description {
                        self.work = Some(Work::Edit(Box::new(receipt::EditJob::new(
                            &self.owner,
                            target,
                            description,
                        )?)));
                    } else {
                        self.done = true;
                        return Ok(true);
                    }
                }
                TextCommand::Newline { line_break } => {
                    let description = prepare_edit(
                        scope,
                        &self.owner,
                        target,
                        "Enter",
                        NativeKeyboardModifiers {
                            shift: line_break,
                            ..Default::default()
                        },
                        None,
                    )?;
                    if let Some(description) = description {
                        self.work = Some(Work::Edit(Box::new(receipt::EditJob::new(
                            &self.owner,
                            target,
                            description,
                        )?)));
                    } else {
                        self.done = true;
                        return Ok(true);
                    }
                }
            }
        }
        let complete = match self.work.as_mut().unwrap() {
            Work::Selection(job) => job.poll(scope, &self.owner)?.is_some(),
            Work::Edit(job) => job.poll(scope, &self.owner)?.is_some(),
            Work::Focus(job) => job.poll(scope, &self.owner)?,
        };
        if complete {
            self.work = None;
            self.done = true;
        }
        Ok(complete)
    }
    pub(in super::super) fn cancel(&mut self) {
        if let Some(work) = &mut self.work {
            match work {
                Work::Selection(job) => job.cancel(&self.owner),
                Work::Edit(job) => job.cancel(),
                Work::Focus(job) => job.cancel(&self.owner),
            }
        }
        self.work = None;
        self.done = true;
    }
}
impl Drop for Receipt {
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
    fn rich_word_delete_prepares_static_target_before_native_selection_change() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=editor contenteditable><p>abc def</p></div>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(239), 1).unwrap();
        let node = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "editor")
            .unwrap()
            .unwrap();
        for behavior in ["accept", "cancel", "selection"] {
            eval(
                scope,
                "globalThis.editor=document.getElementById('editor');editor.innerHTML='<p>abc def</p>';editor.focus();globalThis.text=editor.firstChild.firstChild;getSelection().collapse(text,7);globalThis.events=[];globalThis.saved=null;",
            );
            eval(
                scope,
                &format!(
                    "editor.onbeforeinput=e=>{{events.push('before');saved=e.getTargetRanges()[0];if(!e.isTrusted||e.inputType!=='deleteWordBackward'||saved.startContainer!==text||saved.endContainer!==text||saved.startOffset!==4||saved.endOffset!==7||saved!==e.getTargetRanges()[0])throw Error('word target');if(text.data!=='abc def'||getSelection().anchorNode!==text||getSelection().anchorOffset!==7)throw Error('early selection');if({behavior:?}==='cancel')e.preventDefault();if({behavior:?}==='selection')getSelection().collapse(text,1)}};editor.oninput=e=>{{events.push('input');if(e.getTargetRanges().length!==0)throw Error('input ranges')}};"
                ),
            );
            let mut command = Receipt::new(
                state.clone(),
                node,
                TextCommand::Delete {
                    backward: true,
                    unit: crate::composition::TextUnit::Word,
                },
            )
            .unwrap();
            let mut completed = false;
            for _ in 0..128 {
                completed = command.poll(scope).unwrap();
                if completed {
                    break;
                }
            }
            assert!(completed);
            if behavior == "accept" {
                assert!(eval(scope,"!editor.textContent.includes('def')&&editor.textContent.startsWith('abc')&&events.join()==='before,input'&&saved.startOffset===4&&saved.endOffset===7").is_true());
            } else {
                assert!(eval(scope,&format!("text.data==='abc def'&&events.join()==='before'&&getSelection().anchorNode===text&&getSelection().anchorOffset==={}&&saved.startOffset===4&&saved.endOffset===7",if behavior=="cancel" {7}else{1})).is_true());
            }
        }
    }

    #[test]
    fn document_boundary_delete_retains_beforeinput_and_commits_once() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<textarea id=t>ab\ncd</textarea>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(237), 1).unwrap();
        eval(
            scope,
            "t.focus();t.setSelectionRange(2,2);globalThis.log=[];t.addEventListener('beforeinput',e=>{globalThis.original=e;log.push('first')},{once:true});t.addEventListener('beforeinput',e=>log.push(e===original?'second':'wrong'));t.addEventListener('input',()=>log.push('input'))",
        );
        let node = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "t")
            .unwrap()
            .unwrap();
        let mut command = Receipt::new(
            state.clone(),
            node,
            TextCommand::Delete {
                backward: false,
                unit: crate::composition::TextUnit::DocumentBoundary,
            },
        )
        .unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(command.poll(scope).is_err());
        assert!(eval(scope, "t.value==='ab\\ncd'&&log.join()==='first'").is_true());
        assert!(command.poll(scope).unwrap());
        assert!(command.poll(scope).unwrap());
        assert!(eval(scope, "t.value==='ab'&&log.join()==='first,second,input'").is_true());
    }
    #[test]
    fn canceled_delete_never_expands_control_selection() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<textarea id=t>ab\ncd</textarea>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(238), 1).unwrap();
        eval(
            scope,
            "t.focus();t.setSelectionRange(4,4);t.onbeforeinput=e=>e.preventDefault()",
        );
        let node = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "t")
            .unwrap()
            .unwrap();
        let mut command = Receipt::new(
            state,
            node,
            TextCommand::Delete {
                backward: true,
                unit: crate::composition::TextUnit::DocumentBoundary,
            },
        )
        .unwrap();
        assert!(command.poll(scope).unwrap());
        assert!(
            eval(
                scope,
                "t.value==='ab\\ncd'&&t.selectionStart===4&&t.selectionEnd===4"
            )
            .is_true()
        );
    }
}
