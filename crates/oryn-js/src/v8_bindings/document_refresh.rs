//! Retained native declarative navigation; no authored timer/Location lookup.
use super::*;
#[derive(Clone, Default)]
pub(super) struct State {
    epoch: Option<(u64, u64)>,
    accepted: bool,
    next: u64,
    pending: Option<Pending>,
}
#[derive(Clone)]
struct Pending {
    id: u64,
    generation: u64,
    epoch: u64,
    url: Url,
    delay_ms: u64,
    scheduled: bool,
    due: bool,
}
pub(super) fn schedule(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    content: &str,
) -> Result<(), V8BindingError> {
    if state.execution_retired() || !state.automatic_features_allowed.get() {
        return Ok(());
    }
    let Some(parsed) = oryn_dom::document_refresh::parse(content) else {
        return Ok(());
    };
    let identity = (state.document_generation(), state.document_stream_epoch());
    {
        let mut refresh = state.document_refresh.borrow_mut();
        if refresh.epoch != Some(identity) {
            refresh.epoch = Some(identity);
            refresh.accepted = false;
            refresh.pending = None;
        }
        if refresh.accepted {
            return Ok(());
        }
    }
    let url = match parsed.target {
        None => Url::parse(&state.request_initiator_url()?).ok(),
        Some(target) => {
            let (base, encoding) = hyperlinks::document_base(scope, state, state.document_root())?;
            Url::parse(&base)
                .ok()
                .and_then(|base| hyperlinks::parse(&target, Some(&base), encoding).ok())
        }
    };
    let Some(url) = url.filter(|url| url.scheme() != "javascript") else {
        return Ok(());
    };
    let mut refresh = state.document_refresh.borrow_mut();
    let id = refresh
        .next
        .checked_add(1)
        .ok_or(V8BindingError::Allocation("refresh identity exhausted"))?;
    refresh.next = id;
    refresh.accepted = true;
    refresh.pending = Some(Pending {
        id,
        generation: identity.0,
        epoch: identity.1,
        url,
        delay_ms: parsed.delay_seconds.saturating_mul(1000),
        scheduled: false,
        due: false,
    });
    Ok(())
}
pub(super) fn ready(state: &V8BindingState) -> bool {
    !state.execution_retired()
        && !state.execution_suspended()
        && state.document_completely_loaded.get()
        && state
            .document_refresh
            .borrow()
            .pending
            .as_ref()
            .is_some_and(|pending| !pending.scheduled || pending.due)
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if !ready(state) {
        return Ok(0);
    }
    let Some(mut pending) = state.document_refresh.borrow_mut().pending.take() else {
        return Ok(0);
    };
    if pending.generation != state.document_generation()
        || pending.epoch != state.document_stream_epoch()
        || !state.automatic_features_allowed.get()
    {
        return Ok(0);
    }
    if pending.due {
        // This is a native navigation task in the owner context. Location's native
        // admission applies current navigation policy, without reading DOM attributes.
        let result = location::navigate(
            scope,
            state,
            pending.url.clone(),
            pending.delay_ms <= 1000,
            false,
        );
        if result.is_err() {
            state.document_refresh.borrow_mut().pending = Some(pending);
        }
        return result.map(|_| 1);
    }
    let result = (|| {
        let scheduler = state
            .native_timeout_scheduler
            .get()
            .ok_or(V8BindingError::Allocation("native refresh scheduler"))?;
        let global = state
            .global_object
            .to_local(scope)
            .ok_or(V8BindingError::Allocation("refresh owner"))?;
        let id = v8::BigInt::new_from_u64(scope, pending.id);
        let data = v8::Array::new_with_elements(scope, &[global.into(), id.into()]);
        let callback = v8::Function::builder(expired)
            .data(data.into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("native refresh callback"))?;
        scheduler(scope, state, callback, pending.delay_ms)?;
        Ok::<(), V8BindingError>(())
    })();
    if result.is_ok() {
        pending.scheduled = true;
    }
    state.document_refresh.borrow_mut().pending = Some(pending);
    result.map(|_| 1)
}
fn expired(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let Ok(data) = v8::Local::<v8::Array>::try_from(args.data()) else {
        return;
    };
    let Some(global) = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(state) = binding_state_for_receiver(scope, global) else {
        return;
    };
    let Some(id) = data
        .get_index(scope, 1)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|id| id.u64_value().0)
    else {
        return;
    };
    if state.execution_retired() {
        return;
    }
    if let Some(pending) = state.document_refresh.borrow_mut().pending.as_mut() {
        if pending.id == id
            && pending.generation == state.document_generation()
            && pending.epoch == state.document_stream_epoch()
        {
            pending.due = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    thread_local! {static TIMERS:RefCell<Vec<(u64,v8::Global<v8::Function>)>>=const{RefCell::new(Vec::new())};}
    fn capture(
        scope: &mut v8::PinScope,
        _state: &V8BindingState,
        callback: v8::Local<v8::Function>,
        delay: u64,
    ) -> Result<(), V8BindingError> {
        TIMERS.with(|timers| {
            timers
                .borrow_mut()
                .push((delay, v8::Global::new(scope, callback)))
        });
        Ok(())
    }
    fn eval(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    macro_rules! fixture{($scope:ident,$state:ident)=>{super::super::tests::initialize_v8();let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body>").into_runtime().unwrap()));let mut isolate=v8::Isolate::new(Default::default());v8::scope!(let $scope,&mut isolate);let context=v8::Context::new($scope,Default::default());let $scope=&mut v8::ContextScope::new($scope,context);let $state=install_generated_bindings($scope,context,dom,RealmId(0),1).unwrap();$state.commit_location("https://example.test/page").unwrap();$state.set_native_timeout_scheduler(capture);}}
    #[test]
    fn delay_begins_after_load_and_ignores_authored_timer_properties() {
        fixture!(scope, state);
        eval(
            scope,
            "globalThis.setTimeout=()=>{throw Error('author timer called')}",
        );
        state
            .dom
            .borrow_mut()
            .arena_mut()
            .get_mut(state.document_root())
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .encoding = "windows-1251".into();
        schedule(scope, &state, "2.9, URL=?ÿ").unwrap();
        schedule(scope, &state, "0;url=/replacement").unwrap();
        assert!(!ready(&state));
        assert_eq!(poll(scope, &state).unwrap(), 0);
        state.document_completely_loaded.set(true);
        assert_eq!(poll(scope, &state).unwrap(), 1);
        let pending = state.document_refresh.borrow();
        let pending = pending.pending.as_ref().unwrap();
        assert!(pending.scheduled);
        assert_eq!(
            pending.url.as_str(),
            "https://example.test/page?%26%23255%3B"
        );
        TIMERS.with(|timers| {
            let mut timers = timers.borrow_mut();
            assert_eq!(timers.len(), 1);
            assert_eq!(timers[0].0, 2000);
            timers.clear();
        });
    }
    #[test]
    fn parser_replay_restores_header_baseline_and_discards_provisional_meta() {
        fixture!(scope, state);
        let empty = state.capture_refresh_checkpoint();
        schedule(scope, &state, "0;url=/provisional").unwrap();
        state.restore_refresh_checkpoint(&empty).unwrap();
        assert!(state.document_refresh.borrow().pending.is_none());
        schedule(scope, &state, "3;url=/header").unwrap();
        let header = state.capture_refresh_checkpoint();
        state
            .document_refresh
            .borrow_mut()
            .pending
            .as_mut()
            .unwrap()
            .url = Url::parse("https://example.test/wrong").unwrap();
        state.restore_refresh_checkpoint(&header).unwrap();
        assert_eq!(
            state
                .document_refresh
                .borrow()
                .pending
                .as_ref()
                .unwrap()
                .url
                .path(),
            "/header"
        );
    }
    #[test]
    fn stream_rollover_invalidates_expiry_and_sandbox_blocks_admission() {
        fixture!(scope, state);
        state.document_completely_loaded.set(true);
        state.set_automatic_features_allowed(false);
        schedule(scope, &state, "0;url=/blocked").unwrap();
        assert!(state.document_refresh.borrow().pending.is_none());
        state.set_automatic_features_allowed(true);
        schedule(scope, &state, "0;url=/old").unwrap();
        poll(scope, &state).unwrap();
        let callback = TIMERS.with(|timers| timers.borrow_mut().pop().unwrap().1);
        state
            .document_stream_epoch
            .set(state.document_stream_epoch() + 1);
        schedule(scope, &state, "4;url=/new").unwrap();
        let callback = v8::Local::new(scope, callback);
        let receiver = v8::undefined(scope);
        callback.call(scope, receiver.into(), &[]).unwrap();
        let pending = state.document_refresh.borrow();
        let pending = pending.pending.as_ref().unwrap();
        assert!(!pending.due);
        assert_eq!(pending.url.path(), "/new");
    }
}

/// Opaque checkpoint for parser re-decoding before authored execution.
#[derive(Clone)]
pub struct Snapshot {
    generation: u64,
    epoch: u64,
    state: State,
}
impl V8BindingState {
    pub fn refresh_checkpoint_bytes(&self) -> usize {
        self.document_refresh
            .borrow()
            .pending
            .as_ref()
            .map_or(128, |pending| {
                128usize.saturating_add(pending.url.as_str().len())
            })
    }
    pub fn capture_refresh_checkpoint(&self) -> Snapshot {
        Snapshot {
            generation: self.document_generation(),
            epoch: self.document_stream_epoch(),
            state: self.document_refresh.borrow().clone(),
        }
    }
    pub fn restore_refresh_checkpoint(&self, snapshot: &Snapshot) -> Result<(), V8BindingError> {
        if self.execution_retired()
            || self.document_generation() != snapshot.generation
            || self.document_stream_epoch() != snapshot.epoch
        {
            return Err(V8BindingError::Allocation(
                "refresh checkpoint owner changed",
            ));
        }
        let mut restored = snapshot.state.clone();
        restored.next = restored.next.max(self.document_refresh.borrow().next);
        *self.document_refresh.borrow_mut() = restored;
        Ok(())
    }
}
