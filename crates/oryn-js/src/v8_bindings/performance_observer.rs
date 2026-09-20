//! Native Performance Timeline observers; callback batches are private snapshots.
use super::resource_timing::Entry;
use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Single,
    Multiple,
}
pub(super) struct Observer {
    callback: v8::Global<v8::Function>,
    object: Option<v8::Global<v8::Object>>,
    mode: Option<Mode>,
    types: BTreeSet<String>,
    entries: Vec<Entry>,
    dropped: u64,
}
#[derive(Default)]
pub(super) struct State {
    registered: BTreeMap<u64, Rc<RefCell<Observer>>>,
    pending: VecDeque<u64>,
    failure: Option<(String, u64)>,
    retry: Option<std::time::Instant>,
    #[cfg(test)]
    fail_preparation: bool,
}
fn supported(state: &V8BindingState, kind: &str) -> bool {
    kind == "resource"
        || (matches!(kind, "paint" | "navigation") && state.binding_global == BindingGlobal::Window)
}
fn error(message: &str) -> V8BindingError {
    V8BindingError::Message(format!("TypeError: {message}"))
}
pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    let callback = v8::Local::<v8::Function>::try_from(args.get(0))
        .map_err(|_| error("PerformanceObserver callback must be callable"))?;
    let record = Rc::new(RefCell::new(Observer {
        callback: v8::Global::new(scope, callback),
        object: None,
        mode: None,
        types: BTreeSet::new(),
        entries: Vec::new(),
        dropped: 0,
    }));
    let id = register_platform_object(state, PlatformObjectRecord::PerformanceObserver(record));
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}
pub(super) fn supported_types<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let mut values = Vec::new();
    if supported(state, "navigation") {
        values.push(v8_string(scope, "navigation")?);
    }
    if supported(state, "paint") {
        values.push(v8_string(scope, "paint")?);
    }
    values.push(v8_string(scope, "resource")?);
    let array = v8::Array::new_with_elements(scope, &values);
    array
        .set_integrity_level(scope, v8::IntegrityLevel::Frozen)
        .ok_or(V8BindingError::Allocation("performance supported types"))?;
    Ok(array.into())
}
pub(super) fn publish(state: &V8BindingState, entry: &Entry) {
    let mut schedule = Vec::new();
    {
        let observers = state.performance_observers.borrow();
        for (id, observer) in &observers.registered {
            let mut observer = observer.borrow_mut();
            if !observer.types.contains(entry.kind) {
                continue;
            }
            if observer.entries.len() < 4096 {
                observer.entries.push(entry.clone())
            } else {
                observer.dropped = observer.dropped.saturating_add(1);
            }
            schedule.push(*id);
        }
    }
    let mut observers = state.performance_observers.borrow_mut();
    for id in schedule {
        if !observers.pending.contains(&id) {
            observers.pending.push_back(id);
        }
    }
}
pub(super) fn delay(state: &V8BindingState) -> Option<std::time::Duration> {
    if state.execution_retired() || state.execution_suspended() || state.execution_tasks_stopped() {
        return None;
    }
    let observers = state.performance_observers.borrow();
    if observers.pending.is_empty() {
        return None;
    }
    Some(observers.retry.map_or(std::time::Duration::ZERO, |retry| {
        retry.saturating_duration_since(std::time::Instant::now())
    }))
}
pub(super) fn diagnostic(state: &V8BindingState) -> Option<String> {
    state
        .performance_observers
        .borrow_mut()
        .failure
        .take()
        .map(|(message, count)| {
            format!("PerformanceObserver preparation failed {count} time(s): {message}")
        })
}
pub(super) fn deliver(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<bool, V8BindingError> {
    match poll(scope, state) {
        Ok(done) => Ok(done),
        Err(error) => {
            let mut observers = state.performance_observers.borrow_mut();
            if let Some((_, count)) = &mut observers.failure {
                *count = count.saturating_add(1)
            } else {
                observers.failure = Some((error.to_string(), 1));
            }
            Ok(false)
        }
    }
}
fn array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    entries: &[Entry],
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let mut entries = entries.to_vec();
    entries.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut values = Vec::with_capacity(entries.len());
    for entry in entries {
        values.push(resource_timing::entry_object(scope, state, entry)?.into());
    }
    Ok(v8::Array::new_with_elements(scope, &values).into())
}
fn text(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<String, V8BindingError> {
    let text = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    rust_string_preserving_utf16(scope, text)
}
fn option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("observer option"))?;
    object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)
}
pub(super) fn call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    // Retain native values, never the registry borrow, across author conversion
    // or wrapper allocation (which can itself register platform objects).
    let (entries, observer) = {
        let record = platform_object(state, id)?;
        match &*record {
            PlatformObjectRecord::PerformanceObserverEntryList(entries) => {
                (Some(entries.clone()), None)
            }
            PlatformObjectRecord::PerformanceObserver(observer) => (None, Some(observer.clone())),
            _ => return Err(error("PerformanceObserver receiver")),
        }
    };
    if let Some(entries) = entries {
        let name = if member == "getEntries" {
            None
        } else {
            Some(text(scope, args.get(0))?)
        };
        let kind = if member == "getEntriesByName" && !args.get(1).is_undefined() {
            Some(text(scope, args.get(1))?)
        } else {
            None
        };
        let filtered = entries
            .iter()
            .filter(|entry| match member {
                "getEntries" => true,
                "getEntriesByType" => name.as_deref() == Some(entry.kind),
                "getEntriesByName" => {
                    name.as_deref().is_some_and(|name| entry.name_matches(name))
                        && kind.as_deref().is_none_or(|kind| kind == entry.kind)
                }
                _ => false,
            })
            .cloned()
            .collect::<Vec<_>>();
        return Ok(Some(array(scope, state, &filtered)?));
    }
    let observer = observer.expect("validated observer record");
    match member {
        "disconnect" => {
            state
                .performance_observers
                .borrow_mut()
                .registered
                .remove(&id);
            state
                .performance_observers
                .borrow_mut()
                .pending
                .retain(|entry| *entry != id);
            let mut observer = observer.borrow_mut();
            observer.object = None;
            observer.entries.clear();
            observer.types.clear();
            Ok(None)
        }
        "takeRecords" => {
            let entries = observer.borrow().entries.clone();
            let result = array(scope, state, &entries)?;
            observer.borrow_mut().entries.clear();
            state
                .performance_observers
                .borrow_mut()
                .pending
                .retain(|entry| *entry != id);
            Ok(Some(result))
        }
        "observe" => {
            let options = args.get(0);
            if !options.is_object() {
                return Err(error("observe requires an options dictionary"));
            }
            let options = options
                .to_object(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            let identity = (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            );
            let buffered_value = option(scope, options, "buffered")?;
            let buffered = buffered_value.boolean_value(scope);
            let threshold = option(scope, options, "durationThreshold")?;
            if !threshold.is_undefined() {
                let number = threshold
                    .number_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if !number.is_finite() {
                    return Err(error("durationThreshold must be finite"));
                }
            }
            let multiple = option(scope, options, "entryTypes")?;
            let types = if multiple.is_undefined() {
                None
            } else {
                Some(dom_string_sequence_from_iterable(scope, multiple)?)
            };
            let single = option(scope, options, "type")?;
            let single = if single.is_undefined() {
                None
            } else {
                let text = single
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                Some(rust_string_preserving_utf16(scope, text)?)
            };
            if types.is_some() == single.is_some() {
                return Err(error("specify exactly one of entryTypes and type"));
            }
            if types.is_some() && (!buffered_value.is_undefined() || !threshold.is_undefined()) {
                return Err(error(
                    "entryTypes cannot be combined with buffered or durationThreshold",
                ));
            }
            if state.execution_retired()
                || identity
                    != (
                        state.document_root(),
                        state.document_generation(),
                        state.document_stream_epoch(),
                        state.document_authority_epoch(),
                    )
            {
                return Ok(None);
            }
            let mode = if single.is_some() {
                Mode::Single
            } else {
                Mode::Multiple
            };
            let mut observed = observer.borrow_mut();
            if observed.mode.is_some_and(|previous| previous != mode) {
                return Err(V8BindingError::Message(
                    "InvalidModificationError: observer mode cannot change".into(),
                ));
            }
            observed.mode = Some(mode);
            let requested = types
                .unwrap_or_else(|| single.iter().cloned().collect())
                .into_iter()
                .filter(|kind| supported(state, kind))
                .collect::<BTreeSet<_>>();
            if requested.is_empty() {
                return Ok(None);
            }
            if !state
                .performance_observers
                .borrow()
                .registered
                .contains_key(&id)
                && state.performance_observers.borrow().registered.len() >= 1024
            {
                return Err(V8BindingError::Allocation(
                    "performance observer registration limit",
                ));
            }
            observed.mode = Some(mode);
            if mode == Mode::Multiple {
                observed.types = requested.clone()
            } else {
                observed.types.extend(requested.iter().cloned())
            }
            if buffered {
                for entry in resource_timing::buffered(state)
                    .into_iter()
                    .filter(|entry| requested.contains(entry.kind))
                {
                    if observed.entries.len() < 4096 {
                        observed.entries.push(entry)
                    } else {
                        observed.dropped = observed.dropped.saturating_add(1);
                    }
                }
            }
            observed.object = Some(v8::Global::new(scope, object));
            let pending = !observed.entries.is_empty();
            drop(observed);
            state
                .performance_observers
                .borrow_mut()
                .registered
                .insert(id, observer.clone());
            if pending {
                let mut observers = state.performance_observers.borrow_mut();
                if !observers.pending.contains(&id) {
                    observers.pending.push_back(id);
                }
            }
            Ok(None)
        }
        _ => Err(error("PerformanceObserver operation")),
    }
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<bool, V8BindingError> {
    if delay(state).is_none_or(|delay| !delay.is_zero()) || scope.is_execution_terminating() {
        return Ok(false);
    }
    let selected = {
        let observers = state.performance_observers.borrow();
        observers
            .pending
            .front()
            .and_then(|id| observers.registered.get(id))
            .cloned()
    };
    let Some(observer) = selected else {
        state.performance_observers.borrow_mut().pending.pop_front();
        return Ok(true);
    };
    let prepared = (|| {
        let (entries, callback, object, dropped) = {
            let record = observer.borrow();
            (
                record.entries.clone(),
                v8::Local::new(scope, &record.callback),
                v8::Local::new(
                    scope,
                    record
                        .object
                        .as_ref()
                        .ok_or(V8BindingError::Allocation("observer owner"))?,
                ),
                record.dropped,
            )
        };
        let options = v8::Object::new(scope);
        let key = v8::String::new(scope, "droppedEntriesCount")
            .ok_or(V8BindingError::Allocation("observer dropped count"))?;
        let count = v8::Number::new(scope, dropped as f64);
        options
            .create_data_property(scope, key.into(), count.into())
            .ok_or(V8BindingError::Allocation("observer dropped count"))?;
        #[cfg(test)]
        {
            let failed = {
                let mut observers = state.performance_observers.borrow_mut();
                std::mem::take(&mut observers.fail_preparation)
            };
            if failed {
                return Err(V8BindingError::Allocation("injected observer preparation"));
            }
        }
        let id = register_platform_object(
            state,
            PlatformObjectRecord::PerformanceObserverEntryList(entries),
        );
        let list = match new_platform_object(scope, state, "PerformanceObserverEntryList", id) {
            Ok(list) => list,
            Err(error) => {
                state.platform_objects.borrow_mut().remove(&id);
                state.platform_wrappers.borrow_mut().remove(&id);
                return Err(error);
            }
        };
        Ok::<_, V8BindingError>((callback, object, list, options))
    })();
    let (callback, object, list, options) = match prepared {
        Ok(values) => values,
        Err(error) => {
            state.performance_observers.borrow_mut().retry =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(10));
            return Err(error);
        }
    };
    // All fallible native preparation precedes acceptance. Once entered, author
    // exceptions/termination never replay this callback or its snapshot.
    {
        let mut observer = observer.borrow_mut();
        observer.entries.clear();
        observer.dropped = 0;
    }
    {
        let mut observers = state.performance_observers.borrow_mut();
        observers.pending.pop_front();
        observers.retry = None;
    }
    global_events::invoke(
        scope,
        callback.into(),
        object.into(),
        &[list.into(), object.into(), options.into()],
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) -> bool {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .is_true()
    }
    #[test]
    fn buffered_paint_snapshot_brand_filter_and_disconnect_are_native() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>content</body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        resource_timing::paint(&state, true);
        assert!(eval(
            scope,
            "globalThis.calls=0;globalThis.observer=new PerformanceObserver((list,owner,options)=>{if(!(list instanceof PerformanceObserverEntryList)||owner!==observer||list.getEntriesByType('paint').length!==2||list.getEntriesByName('first-paint','paint').length!==1||options.droppedEntriesCount!==0)throw Error('observer snapshot');globalThis.saved=list;calls++;observer.disconnect()});observer.observe({type:'paint',buffered:true});calls===0"
        ));
        assert!(poll(scope, &state).unwrap());
        assert!(eval(
            scope,
            "calls===1&&saved.getEntries().length===2&&observer.takeRecords().length===0&&Object.isFrozen(PerformanceObserver.supportedEntryTypes)"
        ));
        assert!(delay(&state).is_none());
    }
    #[test]
    fn conversions_and_entry_snapshots_release_registry_before_reentry() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>content</body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        resource_timing::paint(&state, true);
        assert!(eval(
            scope,
            r#"globalThis.calls=0;globalThis.conversions=0;
   globalThis.observer=new PerformanceObserver(list=>{globalThis.saved=list;calls++;});
   observer.observe({get buffered(){conversions++;new PerformanceObserver(()=>{});observer.disconnect();return true},type:'paint'});
   globalThis.taken=observer.takeRecords();
   taken.length===2&&observer.takeRecords().length===0&&conversions===1"#
        ));
        assert!(!poll(scope, &state).unwrap());
        assert!(eval(
            scope,
            "observer.observe({type:'paint',buffered:true});calls===0"
        ));
        assert!(poll(scope, &state).unwrap());
        assert!(eval(
            scope,
            r#"(()=>{
   const first=saved.getEntriesByName({toString(){conversions++;new PerformanceObserver(()=>{});observer.disconnect();return 'first-paint'}},{toString(){conversions++;new PerformanceObserver(()=>{});return 'paint'}});
   const byType=saved.getEntriesByType({toString(){new PerformanceObserver(()=>{});return 'paint'}});
   return calls===1&&conversions===3&&first.length===1&&first[0].name==='first-paint'&&byType.length===2&&saved.getEntries().length===2&&taken.length===2&&observer.takeRecords().length===0;
  })()"#
        ));
        assert!(delay(&state).is_none());
    }
    #[test]
    fn reentrant_observer_does_not_starve_an_already_queued_sibling() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        resource_timing::paint(&state, true);
        eval(
            scope,
            "globalThis.order=[];globalThis.a=new PerformanceObserver(()=>{order.push('a');a.observe({type:'paint',buffered:true})});globalThis.b=new PerformanceObserver(()=>order.push('b'));a.observe({type:'paint',buffered:true});b.observe({type:'paint',buffered:true});true",
        );
        poll(scope, &state).unwrap();
        poll(scope, &state).unwrap();
        assert!(eval(scope, "order.join()==='a,b'"));
    }
    #[test]
    fn resource_observation_is_independent_of_resource_buffer_capacity() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
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
            "performance.setResourceTimingBufferSize(0);globalThis.received=0;new PerformanceObserver(list=>received+=list.getEntries().length).observe({entryTypes:['resource']});true",
        );
        resource_timing::external(
            &state,
            "https://example.test/a.css".into(),
            "link".into(),
            1.,
            2.,
            None,
        );
        poll(scope, &state).unwrap();
        assert!(eval(
            scope,
            "received===1&&performance.getEntriesByType('resource').length===0"
        ));
    }
    #[test]
    fn observer_preparation_failure_and_parking_preserve_one_batch() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
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
            "globalThis.calls=0;globalThis.count=0;new PerformanceObserver(list=>{calls++;count=list.getEntries().length}).observe({type:'paint'});true",
        );
        resource_timing::paint(&state, true);
        state.performance_observers.borrow_mut().fail_preparation = true;
        let objects = state.platform_objects.borrow().len();
        assert!(!deliver(scope, &state).unwrap());
        assert_eq!(state.platform_objects.borrow().len(), objects);
        assert!(delay(&state).is_some_and(|delay| !delay.is_zero()));
        assert!(
            diagnostic(&state)
                .unwrap()
                .contains("injected observer preparation")
        );
        assert!(eval(scope, "calls===0"));
        state.set_execution_suspended(true);
        state.performance_observers.borrow_mut().retry = None;
        assert!(delay(&state).is_none());
        assert!(!poll(scope, &state).unwrap());
        assert_eq!(state.performance_observers.borrow().pending.len(), 1);
        state.set_execution_suspended(false);
        assert!(poll(scope, &state).unwrap());
        assert!(eval(scope, "calls===1&&count===2"));
        assert!(delay(&state).is_none());
    }
}
