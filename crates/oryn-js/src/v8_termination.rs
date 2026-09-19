//! Service pending V8 termination at a native author-callback boundary.
//!
//! In pinned V8, TerminateExecution requests a stack-guard interrupt; it does
//! not immediately set the termination exception. A private JavaScript entry
//! services that request before the next authored callback is consumed. Call
//! inside the caller's TryCatch, then ReThrow an actual caught termination.
//! No author context, globals, tasks, policy, or microtasks are consulted.
use std::{cell::RefCell, rc::Rc};
struct Checkpoint {
    // Drop persistent script/context handles before their private queue.
    script: RefCell<Option<v8::Global<v8::Script>>>,
    context: v8::Global<v8::Context>,
    _microtasks: v8::UniqueRef<v8::MicrotaskQueue>,
}
/// The caller must first preserve any ordinary caught exception and must not
/// use this as a replacement for consuming termination at its host boundary.
pub fn checkpoint(scope: &mut v8::PinScope) -> bool {
    if scope.is_execution_terminating() {
        return false;
    }
    let owner = if let Some(owner) = scope.get_slot::<Rc<Checkpoint>>() {
        owner.clone()
    } else {
        let mut queue = v8::MicrotaskQueue::new(scope, v8::MicrotasksPolicy::Explicit);
        let context = v8::Context::new(
            scope,
            v8::ContextOptions {
                microtask_queue: Some(&mut *queue),
                ..Default::default()
            },
        );
        let owner = Rc::new(Checkpoint {
            script: RefCell::new(None),
            context: v8::Global::new(scope, context),
            _microtasks: queue,
        });
        scope.set_slot(owner.clone());
        owner
    };
    let context = v8::Local::new(scope, &owner.context);
    let scope = &mut v8::ContextScope::new(scope, context);
    if owner.script.borrow().is_none() {
        let Some(source) = v8::String::new(scope, "void 0") else {
            return false;
        };
        let Some(script) = v8::Script::compile(scope, source, None) else {
            return false;
        };
        *owner.script.borrow_mut() = Some(v8::Global::new(scope, script));
    }
    let script = {
        let saved = owner.script.borrow();
        v8::Local::new(scope, saved.as_ref().unwrap())
    };
    script.run(scope).is_some()
}
