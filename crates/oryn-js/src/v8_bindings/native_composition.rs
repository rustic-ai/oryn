//! Native marked-text ownership. OS continuations enter through a validated
//! realm/target grant; no script function can create or resume a session.
use super::*;
use crate::composition::{Operation, StateSnapshot, TextRange};
mod finish;
mod receipt;
pub use receipt::NativeComposition;
struct Session {
    native_segments: Vec<editing_inverse::Segment>,
    target: NodeId,
    identity: (NodeId, u64, u64, u64),
    before: Option<EditingSnapshot>,
    before_value: String,
    before_selection: (u32, u32, String),
    before_control_flags: (bool, bool, Option<String>),
    range: TextRange,
    expected: Vec<u16>,
    data: Vec<u16>,
    cursor: oryn_dom::MutationCursor,
}
impl Drop for Session {
    fn drop(&mut self) {
        drop(self.before.take());
    }
}
#[derive(Default)]
pub(super) struct State {
    session: RefCell<Option<Session>>,
    active: RefCell<Option<Rc<()>>>,
    operation_target: Cell<Option<NodeId>>,
}
// Listener registration/removal changes native dispatch metadata, not the
// marked text. All other changes (including equal-value ABA writes) still
// invalidate ownership; a truncated journal cannot prove continuity.
fn composition_mutations_unchanged(
    state: &V8BindingState,
    cursor: oryn_dom::MutationCursor,
) -> bool {
    let dom = state.dom.borrow();
    if dom.mutation_cursor() == cursor {
        return true;
    }
    let changes = dom.mutations_since(cursor);
    !changes.truncated
        && changes
            .mutations
            .iter()
            .all(|change| matches!(change, oryn_dom::Mutation::EventListener { .. }))
}
fn error() -> V8BindingError {
    V8BindingError::Message("native composition target or marked text changed".into())
}
fn identity(state: &V8BindingState) -> (NodeId, u64, u64, u64) {
    (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
        state.document_authority_epoch(),
    )
}
fn live(state: &V8BindingState, target: NodeId, expected: (NodeId, u64, u64, u64)) -> bool {
    !state.execution_retired()
        && identity(state) == expected
        && state.dom.borrow().is_connected(target).unwrap_or(false)
        && state.dom.borrow().node_document(target).ok().flatten() == Some(expected.0)
        && {
            let dom = state.dom.borrow();
            dom.focused() == Some(target)
                || editing_host_for_node(
                    &dom,
                    dom.focused(),
                    state.design_mode_enabled(),
                    state.document_root(),
                )
                .ok()
                .flatten()
                    == Some(target)
        }
}
fn current(
    state: &V8BindingState,
    target: NodeId,
) -> Result<(Vec<u16>, TextRange), V8BindingError> {
    let control = state.dom.borrow().is_editable_text_control(target);
    if control {
        let (start, end, value) = clipboard_actions::control_selection(state, target)?;
        if value.len() > 4 * 1024 * 1024 {
            return Err(error());
        }
        return Ok((
            oryn_dom::dom_string::units(&value),
            TextRange {
                start,
                length: end - start,
            },
        ));
    }
    let text = selection_movement::accessibility_editing_text(state, target)?;
    if text.units.is_empty()
        && state.selection_anchor.get().is_none()
        && state.selection_focus.get().is_none()
    {
        return Ok((
            text.units,
            TextRange {
                start: 0,
                length: 0,
            },
        ));
    }
    let anchor = state
        .selection_anchor
        .get()
        .and_then(|point| text.offset(point))
        .ok_or_else(error)?;
    let focus = state
        .selection_focus
        .get()
        .and_then(|point| text.offset(point))
        .ok_or_else(error)?;
    Ok((
        text.units,
        TextRange {
            start: anchor.min(focus) as u32,
            length: anchor.abs_diff(focus) as u32,
        },
    ))
}
fn select(state: &V8BindingState, target: NodeId, range: TextRange) -> Result<(), V8BindingError> {
    let end = range.end().ok_or_else(error)?;
    if state.dom.borrow().is_editable_text_control(target) {
        let len = oryn_dom::dom_string::len(&state.dom.borrow().control_edit_value(target)?);
        if end as usize > len {
            return Err(error());
        }
        state
            .text_control_selections
            .borrow_mut()
            .insert(target, (range.start, end, "none".into()));
        state
            .selection_revision
            .set(state.selection_revision.get().wrapping_add(1));
    } else {
        let text = selection_movement::accessibility_editing_text(state, target)?;
        let (Some(start), Some(end)) = (
            text.points.get(range.start as usize),
            text.points.get(end as usize),
        ) else {
            return Err(error());
        };
        update_selection_boundaries(state, *start, *end, false)?;
    }
    Ok(())
}
fn snapshot(
    state: &V8BindingState,
    target: NodeId,
    marked: Option<TextRange>,
) -> Result<StateSnapshot, V8BindingError> {
    let (text, selection) = current(state, target)?;
    let protected = {
        let dom = state.dom.borrow();
        dom.is_element(target, "http://www.w3.org/1999/xhtml", "input")
            && dom
                .attribute_in_null_namespace(target, "type")?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("password"))
    };
    Ok(StateSnapshot {
        mutation_revision: state.dom.borrow().mutation_cursor().0,
        aborted: false,
        protected,
        text: if protected { Vec::new() } else { text },
        selection,
        marked,
    })
}
pub(super) fn retire(state: &V8BindingState) {
    state.composition.session.borrow_mut().take();
    state.composition.active.borrow_mut().take();
    state.composition.operation_target.set(None);
}
pub fn apply(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    operation: &Operation,
) -> Result<StateSnapshot, V8BindingError> {
    let mut receipt = NativeComposition::new(state, target, operation)?;
    receipt
        .poll(scope)?
        .ok_or(V8BindingError::Allocation("native composition suspended"))
}

pub fn native_composition_target(state: &V8BindingState) -> Option<NodeId> {
    state
        .composition
        .session
        .borrow()
        .as_ref()
        .map(|session| session.target)
        .or(state.composition.operation_target.get())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn authority_revocation_cannot_restore_or_commit_an_old_marked_session() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<textarea id=t>abc</textarea>")
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
            "t.focus();t.setSelectionRange(1,2);globalThis.ends=0;t.oncompositionend=()=>ends++",
        );
        let target = state.dom.borrow().focused().unwrap();
        apply(
            scope,
            &state,
            target,
            &Operation::SetMarked {
                text: vec![88],
                selected: TextRange {
                    start: 1,
                    length: 0,
                },
                replacement: None,
            },
        )
        .unwrap();
        let identity_before = identity(&state);
        state
            .document_authority_epoch
            .set(state.document_authority_epoch().checked_add(1).unwrap());
        assert_ne!(identity_before, identity(&state));
        let result = apply(scope, &state, target, &Operation::Cancel).unwrap();
        assert!(result.aborted);
        assert!(native_composition_target(&state).is_none());
        eval(
            scope,
            "if(t.value!=='aXc'||ends!==0)throw Error('revoked composition restored old value or emitted end')",
        );
    }
    #[test]
    fn marked_control_updates_are_trusted_utf16_and_cancel_restores_original() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<textarea id=t>abc</textarea>")
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
            "t.focus();t.setSelectionRange(1,2);globalThis.events=[];for(const type of ['compositionstart','compositionupdate','beforeinput','input','compositionend'])t.addEventListener(type,e=>events.push([e.type,e.isTrusted,e.data,e.isComposing,e.cancelable]));",
        );
        let target = state.dom.borrow().focused().unwrap();
        let first = apply(
            scope,
            &state,
            target,
            &Operation::SetMarked {
                text: vec![0xd800, 0xd83d, 0xde00],
                selected: TextRange {
                    start: 1,
                    length: 2,
                },
                replacement: None,
            },
        )
        .unwrap();
        assert_eq!(first.text, vec![97, 0xd800, 0xd83d, 0xde00, 99]);
        assert_eq!(
            first.selection,
            TextRange {
                start: 2,
                length: 2
            }
        );
        let second = apply(
            scope,
            &state,
            target,
            &Operation::SetMarked {
                text: vec![120],
                selected: TextRange {
                    start: 1,
                    length: 0,
                },
                replacement: None,
            },
        )
        .unwrap();
        assert_eq!(second.text, vec![97, 120, 99]);
        let restored = apply(scope, &state, target, &Operation::Cancel).unwrap();
        assert_eq!(restored.text, vec![97, 98, 99]);
        assert_eq!(
            restored.selection,
            TextRange {
                start: 1,
                length: 1
            }
        );
        assert!(native_composition_target(&state).is_none());
        eval(
            scope,
            "if(events.some(e=>!e[1]))throw Error('untrusted native composition');if(events.filter(e=>e[0]==='compositionstart').length!==1||events.at(-1)[0]!=='compositionend')throw Error('composition lifecycle');if(events.filter(e=>e[0]==='beforeinput').some(e=>e[3]!==true||e[4]!==false))throw Error('marked beforeinput flags');",
        );
    }
    #[test]
    fn temporal_marked_field_updates_preserve_partial_value_and_cancel_state() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t type=date required>")
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
            "t.focus();globalThis.ends=[];t.oncompositionend=e=>ends.push(e.data)",
        );
        let target = state.dom.borrow().focused().unwrap();
        let first = apply(
            scope,
            &state,
            target,
            &Operation::SetMarked {
                text: vec![50],
                selected: TextRange {
                    start: 1,
                    length: 0,
                },
                replacement: Some(TextRange {
                    start: 0,
                    length: 4,
                }),
            },
        )
        .unwrap();
        assert_eq!(oryn_dom::dom_string::from_units(&first.text), "0002-mm-dd");
        assert_eq!(
            first.marked,
            Some(TextRange {
                start: 0,
                length: 4
            })
        );
        eval(
            scope,
            "if(t.value!==''||!t.validity.badInput)throw Error('partial temporal composition')",
        );
        let updated = apply(
            scope,
            &state,
            target,
            &Operation::SetMarked {
                text: "2024".encode_utf16().collect(),
                selected: TextRange {
                    start: 4,
                    length: 0,
                },
                replacement: None,
            },
        )
        .unwrap();
        assert_eq!(
            oryn_dom::dom_string::from_units(&updated.text),
            "2024-mm-dd"
        );
        let restored = apply(scope, &state, target, &Operation::Cancel).unwrap();
        assert_eq!(
            oryn_dom::dom_string::from_units(&restored.text),
            "yyyy-mm-dd"
        );
        eval(
            scope,
            "if(t.value!==''||t.validity.badInput||ends.at(-1)!=='')throw Error('temporal cancel state')",
        );
        apply(
            scope,
            &state,
            target,
            &Operation::SetMarked {
                text: vec![50],
                selected: TextRange {
                    start: 1,
                    length: 0,
                },
                replacement: Some(TextRange {
                    start: 0,
                    length: 4,
                }),
            },
        )
        .unwrap();
        apply(scope, &state, target, &Operation::Unmark).unwrap();
        eval(
            scope,
            "if(ends.at(-1)!=='2'||!t.validity.badInput)throw Error('normalized field leaked into composition data')",
        );
    }
    #[test]
    fn tab_commits_marked_text_and_retires_only_original_continuation() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t><input id=b><input id=c>")
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
            "t.focus();globalThis.ends=0;t.addEventListener('compositionend',()=>ends++)",
        );
        let target = state.dom.borrow().focused().unwrap();
        apply(
            scope,
            &state,
            target,
            &Operation::SetMarked {
                text: vec![120],
                selected: TextRange {
                    start: 1,
                    length: 0,
                },
                replacement: None,
            },
        )
        .unwrap();
        let result = apply(
            scope,
            &state,
            target,
            &Operation::Command(crate::composition::TextCommand::Tab { backward: false }),
        )
        .unwrap();
        assert!(result.aborted && result.text.is_empty() && result.marked.is_none());
        assert!(native_composition_target(&state).is_none());
        eval(
            scope,
            "if(t.value!=='x'||document.activeElement!==b||ends!==1)throw Error('composition tab lifecycle');t.focus();t.addEventListener('compositionend',()=>b.focus(),{once:true})",
        );
        apply(
            scope,
            &state,
            target,
            &Operation::SetMarked {
                text: vec![121],
                selected: TextRange {
                    start: 1,
                    length: 0,
                },
                replacement: None,
            },
        )
        .unwrap();
        let result = apply(
            scope,
            &state,
            target,
            &Operation::Command(crate::composition::TextCommand::Tab { backward: false }),
        )
        .unwrap();
        assert!(result.aborted);
        eval(
            scope,
            "if(document.activeElement!==b||c.value!=='')throw Error('continued command after author focus transition')",
        );
    }
    #[test]
    fn ordinary_commit_can_be_canceled_and_password_snapshot_is_private() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t value=abc><input id=p type=password value=secret>")
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
            "t.focus();t.setSelectionRange(1,2);t.onbeforeinput=e=>e.preventDefault()",
        );
        let target = state.dom.borrow().focused().unwrap();
        let read = apply(
            scope,
            &state,
            target,
            &Operation::Commit {
                text: vec![120],
                replacement: None,
            },
        )
        .unwrap();
        assert_eq!(read.text, vec![97, 98, 99]);
        eval(scope, "p.focus();p.setSelectionRange(2,3)");
        let password = state.dom.borrow().focused().unwrap();
        let read = apply(scope, &state, password, &Operation::Read).unwrap();
        assert!(read.protected);
        assert!(read.text.is_empty());
        assert_eq!(
            read.selection,
            TextRange {
                start: 2,
                length: 1
            }
        );
    }
    #[test]
    fn author_mutation_invalidates_composition_without_overwriting_new_value() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<textarea id=t>abc</textarea>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(scope, "t.focus();t.setSelectionRange(0,1)");
        let target = state.dom.borrow().focused().unwrap();
        apply(
            scope,
            &state,
            target,
            &Operation::SetMarked {
                text: vec![120],
                selected: TextRange {
                    start: 1,
                    length: 0,
                },
                replacement: None,
            },
        )
        .unwrap();
        eval(scope, "t.value='author'");
        assert!(
            apply(scope, &state, target, &Operation::Cancel)
                .unwrap()
                .aborted
        );
        eval(
            scope,
            "if(t.value!=='author')throw Error('stale composition overwrite')",
        );
        assert!(native_composition_target(&state).is_none());
    }
    #[test]
    fn native_interpretation_defers_printable_defaults_but_respects_canceled_keydown() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<textarea id=t></textarea>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(scope, "t.focus();globalThis.keys=0;t.onkeydown=()=>keys++");
        let target = state.dom.borrow().focused().unwrap();
        let input = NativeKeyboardInput {
            defer_text: true,
            down: true,
            key: vec![97],
            code: "KeyA".into(),
            ..Default::default()
        };
        assert!(dispatch_native_keyboard(scope, &state, &input).unwrap());
        assert!(native_keyboard_was_deferred(&state));
        assert_eq!(native_keyboard_deferred_target(&state), Some(target));
        eval(
            scope,
            "if(t.value!==''||keys!==1)throw Error('duplicate or early native edit')",
        );
        apply(
            scope,
            &state,
            target,
            &Operation::Commit {
                text: vec![97],
                replacement: None,
            },
        )
        .unwrap();
        apply(
            scope,
            &state,
            target,
            &Operation::Command(crate::composition::TextCommand::Move {
                direction: crate::composition::TextDirection::Backward,
                unit: crate::composition::TextUnit::Character,
                extend: false,
            }),
        )
        .unwrap();
        eval(
            scope,
            "if(t.value!=='a'||keys!==1||t.selectionStart!==0)throw Error('native command emitted duplicate keydown')",
        );
        eval(scope, "t.onkeydown=e=>e.preventDefault()");
        assert!(!dispatch_native_keyboard(scope, &state, &input).unwrap());
        assert!(!native_keyboard_was_deferred(&state));
    }
}
