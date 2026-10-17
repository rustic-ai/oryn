//! Document-owned animation callback lists, consumed only by rendering steps.
use super::*;

#[derive(Default)]
pub(super) struct State {
    next: u32,
    reveal_identity: Rc<()>,
    reveal: Option<Reveal>,
    revealing: bool,
    history_reveal: Option<(NodeId, u64, u64, u64)>,
    deadline: Option<Instant>,
    callbacks: BTreeMap<u32, v8::Global<v8::Function>>,
}

struct Reveal {
    identity: Rc<()>,
    generation: u64,
    stream: u64,
    epoch: u64,
    event: native_platform::publication::Notification,
}
pub(super) fn reset_reveal(state: &V8BindingState) {
    let mut frames = state.animation_frames.borrow_mut();
    frames.reveal_identity = Rc::new(());
    if let Some(reveal) = frames.reveal.take() {
        reveal.event.cancel(state);
    }
    state.page_revealed.set(false);
}
fn reveal_document(state: &V8BindingState) -> (NodeId, u64, u64, u64) {
    (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
        state.document_authority_epoch(),
    )
}
pub(super) fn prepare_history_reveal(state: &V8BindingState) {
    reset_reveal(state);
    state.animation_frames.borrow_mut().history_reveal = Some(reveal_document(state));
}
pub(super) fn complete_history_reveal(state: &V8BindingState) {
    let mut frames = state.animation_frames.borrow_mut();
    if frames.history_reveal == Some(reveal_document(state)) {
        frames.history_reveal = None;
        frames.deadline = None;
    }
}
fn history_reveal_pending(state: &V8BindingState) -> bool {
    let mut frames = state.animation_frames.borrow_mut();
    if frames
        .history_reveal
        .is_some_and(|identity| identity != reveal_document(state))
    {
        frames.history_reveal = None;
    }
    frames.history_reveal.is_some()
}
// HTML permits an implementation-defined bound on blocking initial rendering.
// This deadline releases only rendering admission; it invents no paint entries.
const RENDER_BLOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
pub(super) fn rendering_allowed(state: &V8BindingState) -> bool {
    if state.execution_retired()
        || state.execution_suspended()
        || state.is_unloading()
        || state.document_prerendering()
        || history_reveal_pending(state)
    {
        return false;
    }
    if state.clock_origin.get().elapsed() >= RENDER_BLOCK_TIMEOUT {
        return true;
    }
    state
        .dom
        .try_borrow_mut()
        .is_ok_and(|mut dom| !dom.render_blocked(state.document_root()))
}
pub(super) fn reveal(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<bool, V8BindingError> {
    if !rendering_allowed(state) {
        return Ok(false);
    }
    if state.page_revealed.get() {
        return Ok(true);
    }
    if state.animation_frames.borrow().revealing {
        return Ok(false);
    }
    let pending = { state.animation_frames.borrow_mut().reveal.take() };
    let mut pending = match pending {
        Some(pending) => pending,
        None => Reveal {
            identity: state.animation_frames.borrow().reveal_identity.clone(),
            generation: state.document_generation(),
            stream: state.document_stream_epoch(),
            epoch: state.document_authority_epoch(),
            event: page_swap::prepare_reveal(scope, state)?,
        },
    };
    let current = |pending: &Reveal| {
        !state.execution_retired()
            && pending.generation == state.document_generation()
            && pending.stream == state.document_stream_epoch()
            && pending.epoch == state.document_authority_epoch()
            && Rc::ptr_eq(
                &pending.identity,
                &state.animation_frames.borrow().reveal_identity,
            )
    };
    if !current(&pending) {
        pending.event.cancel(state);
        return Ok(false);
    }
    state.animation_frames.borrow_mut().revealing = true;
    let result = pending.event.resume(scope, state);
    state.animation_frames.borrow_mut().revealing = false;
    if !current(&pending) {
        pending.event.cancel(state);
        return Ok(false);
    }
    match result {
        Ok(true) => {
            pending.event.finish(scope, state);
            state.page_revealed.set(true);
            Ok(true)
        }
        other => {
            state.animation_frames.borrow_mut().reveal = Some(pending);
            other
        }
    }
}

fn owner(scope: &mut v8::PinScope, object: v8::Local<v8::Object>) -> Option<Rc<V8BindingState>> {
    let state = binding_state_for_receiver(scope, object)?;
    if !global_events::is_global(scope, &state, object) {
        throw_type_error(scope, "animation frame requires a Window receiver");
        return None;
    }
    if !global_events::check_window_access(scope, &state, object) {
        return None;
    }
    Some(state)
}

pub(super) fn request(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = owner(scope, args.this()) else {
        return;
    };
    let Ok(callback) = v8::Local::<v8::Function>::try_from(args.get(0)) else {
        throw_type_error(scope, "requestAnimationFrame requires a callback function");
        return;
    };
    if state.execution_retired() {
        rv.set_uint32(0);
        return;
    }
    let mut frames = state.animation_frames.borrow_mut();
    let Some(id) = frames.next.checked_add(1) else {
        drop(frames);
        throw_binding_error(
            scope,
            &V8BindingError::Allocation("animation callback identity exhausted"),
        );
        return;
    };
    frames.next = id;
    frames
        .callbacks
        .insert(id, v8::Global::new(scope, callback));
    rv.set_uint32(id);
}

pub(super) fn cancel(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Some(state) = owner(scope, args.this()) else {
        return;
    };
    if args.length() == 0 {
        throw_type_error(scope, "cancelAnimationFrame requires a handle");
        return;
    }
    let Some(id) = args.get(0).uint32_value(scope) else {
        return;
    };
    state.animation_frames.borrow_mut().callbacks.remove(&id);
}

pub(super) fn retire(state: &V8BindingState) {
    let mut frames = state.animation_frames.borrow_mut();
    frames.callbacks.clear();
    frames.deadline = None;
    frames.history_reveal = None;
    frames.reveal_identity = Rc::new(());
    if let Some(reveal) = frames.reveal.take() {
        reveal.event.cancel(state);
    }
}

fn sampling_delay(
    deadline: &mut Option<Instant>,
    now: Instant,
    pending: bool,
    urgent: bool,
) -> Option<std::time::Duration> {
    if !pending {
        *deadline = None;
        return None;
    }
    if urgent {
        return Some(std::time::Duration::ZERO);
    }
    let due = *deadline.get_or_insert(now + std::time::Duration::from_millis(16));
    Some(due.saturating_duration_since(now))
}

pub(super) fn delay(state: &V8BindingState) -> Option<std::time::Duration> {
    if state.execution_retired()
        || state.execution_suspended()
        || state.document_prerendering()
        || history_reveal_pending(state)
    {
        return None;
    }
    if !rendering_allowed(state) {
        return Some(RENDER_BLOCK_TIMEOUT.saturating_sub(state.clock_origin.get().elapsed()));
    }
    let playback = state.next_web_animation_delay();
    let css = state
        .layout_provider
        .borrow()
        .as_ref()
        .is_some_and(|provider| provider.animations_active());
    let mut frames = state.animation_frames.borrow_mut();
    let pending = !state.page_revealed.get()
        || playback.is_some()
        || css
        || !frames.callbacks.is_empty()
        || resize_observers::needs_render(state)
        || intersection_observers::needs_render(state);
    sampling_delay(
        &mut frames.deadline,
        Instant::now(),
        pending,
        playback.is_some_and(|delay| delay.is_zero()),
    )
}

pub(super) fn run(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    rendering_time: Instant,
) -> Result<(), V8BindingError> {
    state.animation_frames.borrow_mut().deadline =
        Some(rendering_time + std::time::Duration::from_millis(16));
    if !rendering_allowed(state) {
        return Ok(());
    }
    if !native_platform::rendering_visible(state)? {
        // Playback notifications already queued are tasks even when rendering
        // is suppressed. Consume them so urgent readiness cannot spin forever.
        web_animations::drain_events(scope, state)?;
        return font_loading::drain_events(scope,state);
    }
    if !reveal(scope, state)? {
        return Ok(());
    }
    struct RenderingClockGuard<'a>(&'a V8BindingState);
    impl Drop for RenderingClockGuard<'_> {
        fn drop(&mut self) {
            web_animations::end_rendering(self.0);
        }
    }
    let _rendering_clock = RenderingClockGuard(state);
    {
        let _profile = native_binding_stage(scope, "deep.web_animation_tick");
        web_animations::tick(scope, state, rendering_time)?;
    }
    // A callback registered by a callback waits for the next rendering step.
    // Keep each entry in the list until invocation so another callback can
    // cancel it after this snapshot has been taken.
    let callbacks: Vec<_> = state
        .animation_frames
        .borrow()
        .callbacks
        .keys()
        .copied()
        .collect();
    let elapsed = rendering_time
        .checked_duration_since(state.clock_origin.get())
        .unwrap_or_default();
    let timestamp = (elapsed.as_micros() / 100) as f64 / 10.0;
    for id in callbacks {
        if !rendering_allowed(state) {
            break;
        }
        let callback = state.animation_frames.borrow_mut().callbacks.remove(&id);
        let Some(callback) = callback else {
            continue;
        };
        let callback = v8::Local::new(scope, &callback);
        let receiver = scope.get_current_context().global(scope);
        let timestamp = v8::Number::new(scope, timestamp);
        global_events::invoke(scope, callback.into(), receiver.into(), &[timestamp.into()]);
        if scope.is_execution_terminating() {
            return Err(V8BindingError::JavaScriptException);
        }
        scope.perform_microtask_checkpoint();
        state.finish_platform_task_checkpoint()?;
    }
    // Frame callbacks and their microtasks can edit paused effects without a
    // style query. Publish those edits before the caller paints this frame,
    // while the document timestamp is still frozen for the rendering step.
    if rendering_allowed(state) {
        web_animations::publish(state)?;
    }
    Ok(())
}

#[cfg(test)]
mod scheduling_tests {
    use super::*;
    #[test]
    fn polling_preserves_deadline_and_idle_releases_it() {
        let now = Instant::now();
        let mut deadline = None;
        assert_eq!(
            sampling_delay(&mut deadline, now, true, false),
            Some(std::time::Duration::from_millis(16))
        );
        assert_eq!(
            sampling_delay(
                &mut deadline,
                now + std::time::Duration::from_millis(10),
                true,
                false
            ),
            Some(std::time::Duration::from_millis(6))
        );
        assert_eq!(
            sampling_delay(
                &mut deadline,
                now + std::time::Duration::from_millis(20),
                true,
                false
            ),
            Some(std::time::Duration::ZERO)
        );
        assert_eq!(sampling_delay(&mut deadline, now, false, false), None);
        assert!(deadline.is_none());
        assert_eq!(
            sampling_delay(&mut deadline, now, true, true),
            Some(std::time::Duration::ZERO)
        );
    }
    fn execute(scope: &mut v8::PinScope, source: &str) -> bool {
        let text = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, text, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .is_true()
    }
    fn fixture(scope: &mut v8::PinScope) -> Rc<V8BindingState> {
        let context = scope.get_current_context();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<!doctype html><body></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap()
    }
    #[test]
    fn history_pageshow_gate_blocks_reveal_and_raf_until_native_completion() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = fixture(scope);
        execute(
            scope,
            "globalThis.calls=[];addEventListener('pagereveal',()=>calls.push('reveal'));requestAnimationFrame(()=>calls.push('raf'));true",
        );
        state.prepare_history_reveal();
        assert!(delay(&state).is_none());
        run(scope, &state, Instant::now()).unwrap();
        assert!(execute(scope, "calls.length===0"));
        execute(scope, "calls.push('pageshow');true");
        state.complete_history_reveal();
        assert!(delay(&state).is_some());
        run(scope, &state, Instant::now()).unwrap();
        assert!(execute(scope, "calls.join()==='pageshow,reveal,raf'"));
        run(scope, &state, Instant::now()).unwrap();
        assert!(execute(scope, "calls.length===3"));
    }
    #[test]
    fn replacement_stream_does_not_inherit_history_rendering_gate() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = fixture(scope);
        state.prepare_history_reveal();
        execute(
            scope,
            "document.open();document.write('<body>replacement</body>');document.close();true",
        );
        assert!(!history_reveal_pending(&state));
    }
    #[test]
    fn retained_reveal_finishes_callback_suffix_before_animation_frame() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = fixture(scope);
        execute(
            scope,
            "globalThis.calls=[];addEventListener('pagereveal',()=>calls.push('first'));addEventListener('pagereveal',()=>calls.push('second'));requestAnimationFrame(()=>calls.push('raf'));true",
        );
        native_platform::publication::fail_after_next_callback();
        assert!(run(scope, &state, Instant::now()).is_err());
        assert!(!state.page_revealed.get());
        assert!(delay(&state).is_some());
        assert!(execute(scope, "calls.join()==='first'"));
        run(scope, &state, Instant::now()).unwrap();
        assert!(execute(scope, "calls.join()==='first,second,raf'"));
        assert!(state.page_revealed.get());
        run(scope, &state, Instant::now()).unwrap();
        assert!(execute(scope, "calls.length===3"));
    }
    #[test]
    fn reveal_preparation_reset_cancels_old_suffix_and_requires_new_publication() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = fixture(scope);
        execute(
            scope,
            "globalThis.calls=[];addEventListener('pagereveal',()=>calls.push('first'));addEventListener('pagereveal',()=>calls.push('second'));true",
        );
        native_platform::publication::fail_after_next_callback();
        assert!(reveal(scope, &state).is_err());
        state.prepare_page_reveal();
        assert!(reveal(scope, &state).unwrap());
        assert!(execute(scope, "calls.join()==='first,first,second'"));
        assert!(delay(&state).is_none());
    }
}
