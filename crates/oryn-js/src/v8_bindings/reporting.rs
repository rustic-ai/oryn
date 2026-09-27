//! Native ReportingObserver queues. Only native producers can enqueue reports.
use super::*;
#[derive(Clone)]
struct Report {
    value: serde_json::Value,
    serialized: Option<String>,
}
impl Report {
    fn text(&self) -> String {
        self.serialized
            .clone()
            .unwrap_or_else(|| self.value.to_string())
    }
}
struct Observer {
    object: v8::Weak<v8::Object>,
    keepalive: Option<v8::Global<v8::Object>>,
    types: BTreeSet<String>,
    active: bool,
    buffered: bool,
    queue: Vec<Report>,
}
#[derive(Default)]
pub(super) struct State {
    next: u64,
    observers: BTreeMap<u64, Observer>,
    buffer: Vec<Report>,
}
fn queue_report(state: &V8BindingState, report: Report) {
    let kind = report.value["type"].as_str().unwrap_or("");
    let mut registry = state.reporting.borrow_mut();
    // The retained history limit is per type. Live observer queues must keep
    // already admitted records, including when disconnected before delivery.
    if registry
        .buffer
        .iter()
        .filter(|old| old.value["type"].as_str() == Some(kind))
        .count()
        >= 100
        && let Some(index) = registry
            .buffer
            .iter()
            .position(|old| old.value["type"].as_str() == Some(kind))
    {
        registry.buffer.remove(index);
    }
    registry.buffer.push(report.clone());
    for observer in registry.observers.values_mut() {
        if observer.active && (observer.types.is_empty() || observer.types.contains(kind)) {
            observer.queue.push(report.clone());
        }
    }
}
fn failure() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "ReportingObserver".into(),
        member: "TypeError".into(),
    }
}
fn key<'s>(scope: &mut v8::PinScope<'s, '_>, name: &str) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, name).unwrap();
    v8::Private::for_api(scope, Some(name))
}
fn owner(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<(Rc<V8BindingState>, u64), V8BindingError> {
    if object.is_proxy() {
        return Err(failure());
    }
    let owner_key = key(scope, "oryn.reporting.owner");
    let global = object
        .get_private(scope, owner_key)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(failure)?;
    let state = global
        .get_creation_context(scope)
        .and_then(|context| context.get_slot::<V8BindingState>())
        .ok_or_else(failure)?;
    let key = key(scope, "oryn.reporting.observer");
    let id = object
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0)
        .ok_or_else(failure)?;
    if !state
        .reporting
        .borrow()
        .observers
        .get(&id)
        .and_then(|observer| observer.object.to_local(scope))
        .is_some_and(|registered| registered.strict_equals(object.into()))
    {
        return Err(failure());
    }
    Ok((state, id))
}
fn json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let text = v8::String::new(scope, value).ok_or_else(failure)?;
    v8::json::parse(scope, text).ok_or_else(failure)
}
fn to_json(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let key = key(scope, "oryn.reporting.json");
    let result = args
        .this()
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .and_then(|text| v8::json::parse(scope, text));
    if let Some(value) = result {
        rv.set(value)
    } else {
        throw_type_error(scope, "Report receiver required")
    }
}
fn object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    report: &Report,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let serialized = report.text();
    let value = json(scope, &serialized)?;
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| failure())?;
    let key = key(scope, "oryn.reporting.json");
    let text = v8::String::new(scope, &serialized).ok_or_else(failure)?;
    if object.set_private(scope, key, text.into()) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    let name = v8::String::new(scope, "toJSON").unwrap();
    let function = v8::Function::new(scope, to_json).ok_or_else(failure)?;
    if object.define_own_property(
        scope,
        name.into(),
        function.into(),
        v8::PropertyAttribute::DONT_ENUM,
    ) != Some(true)
    {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(object.into())
}
#[cfg(test)]
thread_local! {static FAIL_ARRAY_ONCE:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
fn array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reports: &[Report],
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    #[cfg(test)]
    if FAIL_ARRAY_ONCE.with(|flag| flag.replace(false)) {
        return Err(V8BindingError::Allocation("report array injection"));
    }
    let values = v8::Array::new(scope, reports.len() as i32);
    for (index, report) in reports.iter().enumerate() {
        let value = object(scope, report)?;
        let key = v8::String::new(scope, &index.to_string()).ok_or_else(failure)?;
        if values.create_data_property(scope, key.into(), value) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
    }
    Ok(values.into())
}

fn construct(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let result = (|| {
        if !args.is_construct_call() {
            return Err(failure());
        }
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or_else(failure)?;
        let callback = v8::Local::<v8::Function>::try_from(args.get(0)).map_err(|_| failure())?;
        let mut types = BTreeSet::new();
        let mut buffered = false;
        if !args.get(1).is_null_or_undefined() {
            let options = v8::Local::<v8::Object>::try_from(args.get(1)).map_err(|_| failure())?;
            let name = v8::String::new(scope, "buffered").unwrap();
            buffered = options
                .get(scope, name.into())
                .ok_or(V8BindingError::JavaScriptException)?
                .boolean_value(scope);
            let name = v8::String::new(scope, "types").unwrap();
            let value = options
                .get(scope, name.into())
                .ok_or(V8BindingError::JavaScriptException)?;
            if !value.is_undefined() {
                let sequence = v8::Local::<v8::Object>::try_from(value).map_err(|_| failure())?;
                let iterator = v8::Symbol::get_iterator(scope);
                let method = sequence
                    .get(scope, iterator.into())
                    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                    .ok_or_else(failure)?;
                iterable_conversion::consume(scope, sequence, method, |scope, value| {
                    if types.len() >= 64 {
                        return Err(failure());
                    }
                    let text = value
                        .to_string(scope)
                        .ok_or(V8BindingError::JavaScriptException)?
                        .to_rust_string_lossy(scope);
                    if text.len() > 256 {
                        return Err(failure());
                    }
                    types.insert(text);
                    Ok(())
                })?;
            }
        }
        // The native registration belongs to the constructor's executing global,
        // independently of a foreign newTarget/prototype or borrowed method realm.
        // Keep that identity as a traced private edge, not a native strong callback.
        let owner_key = key(scope, "oryn.reporting.owner");
        let context = scope.get_current_context();
        let global = context.global(scope);
        if args.this().set_private(scope, owner_key, global.into()) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
        let callback_key = key(scope, "oryn.reporting.callback");
        if args
            .this()
            .set_private(scope, callback_key, callback.into())
            != Some(true)
        {
            return Err(V8BindingError::JavaScriptException);
        }
        let mut registry = state.reporting.borrow_mut();
        registry
            .observers
            .retain(|_, observer| observer.object.to_local(scope).is_some());
        if registry.observers.len() >= 256 {
            return Err(failure());
        }
        let id = registry.next.checked_add(1).ok_or_else(failure)?;
        let brand = key(scope, "oryn.reporting.observer");
        let value = v8::BigInt::new_from_u64(scope, id);
        if args.this().set_private(scope, brand, value.into()) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
        registry.next = id;
        registry.observers.insert(
            id,
            Observer {
                object: v8::Weak::new(scope, args.this()),
                keepalive: None,
                types,
                active: false,
                buffered,
                queue: Vec::new(),
            },
        );
        Ok(())
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error)
    }
}
fn call(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| {
        let (state, id) = owner(scope, args.this())?;
        let index = args.data().int32_value(scope).unwrap_or(-1);
        let mut registry = state.reporting.borrow_mut();
        let buffer = registry.buffer.clone();
        let observer = registry.observers.get_mut(&id).ok_or_else(failure)?;
        match index {
            0 => {
                observer.active = true;
                observer.keepalive = Some(v8::Global::new(scope, args.this()));
                if observer.buffered {
                    observer.buffered = false;
                    for report in buffer {
                        if observer.types.is_empty()
                            || observer
                                .types
                                .contains(report.value["type"].as_str().unwrap_or(""))
                        {
                            observer.queue.push(report)
                        }
                    }
                }
                Ok(None)
            }
            1 => {
                observer.active = false;
                if observer.queue.is_empty() {
                    observer.keepalive = None;
                }
                Ok(None)
            }
            2 => {
                let queue = observer.queue.clone();
                drop(registry);
                let value = array(scope, &queue)?;
                let mut registry = state.reporting.borrow_mut();
                let observer = registry.observers.get_mut(&id).ok_or_else(failure)?;
                observer.queue.drain(..queue.len());
                if !observer.active && observer.queue.is_empty() {
                    observer.keepalive = None;
                }
                Ok(Some(value))
            }
            _ => Err(failure()),
        }
    })();
    match result {
        Ok(Some(value)) => rv.set(value),
        Ok(None) => {}
        Err(error) => throw_binding_error(scope, &error),
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let function = v8::Function::builder(construct)
        .length(1)
        .build(scope)
        .ok_or_else(failure)?;
    let name = v8::String::new(scope, "prototype").ok_or_else(failure)?;
    let prototype = function
        .get(scope, name.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(failure)?;
    for (name, index) in [("observe", 0), ("disconnect", 1), ("takeRecords", 2)] {
        let data = v8::Integer::new(scope, index);
        let method = v8::Function::builder(call)
            .data(data.into())
            .length(0)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or_else(failure)?;
        let name = v8::String::new(scope, name).ok_or_else(failure)?;
        method.set_name(name);
        if prototype.define_own_property(
            scope,
            name.into(),
            method.into(),
            v8::PropertyAttribute::NONE,
        ) != Some(true)
        {
            return Err(V8BindingError::JavaScriptException);
        }
    }
    let tag = v8::Symbol::get_to_string_tag(scope);
    let name = v8::String::new(scope, "ReportingObserver").ok_or_else(failure)?;
    if prototype.define_own_property(
        scope,
        tag.into(),
        name.into(),
        v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
    ) != Some(true)
    {
        return Err(V8BindingError::JavaScriptException);
    }
    function.set_name(name);
    if context
        .global(scope)
        .create_data_property(scope, name.into(), function.into())
        != Some(true)
    {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(())
}

pub(super) fn permission(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    feature: &str,
    disposition: &str,
) -> Result<(), V8BindingError> {
    let url = document_url(scope, state)?;
    let frame = v8::StackTrace::current_stack_trace(scope, 1).and_then(|trace| {
        if trace.get_frame_count() > 0 {
            trace.get_frame(scope, 0)
        } else {
            None
        }
    });
    let source = frame
        .and_then(|frame| frame.get_script_name(scope))
        .map(|name| name.to_rust_string_lossy(scope))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| url.clone());
    let line = frame.map(|frame| frame.get_line_number()).unwrap_or(0);
    let column = frame.map(|frame| frame.get_column()).unwrap_or(0);
    let report = Report {
        serialized: None,
        value: serde_json::json!({"type":"permissions-policy-violation","url":url,"body":{"featureId":feature,"disposition":disposition,"sourceFile":source,"lineNumber":line,"columnNumber":column}}),
    };
    queue_report(state, report);
    Ok(())
}
// Backend admission has no entered V8 stack. Preserve native document source
// identity and use the API's unknown source-location values.
pub(super) fn permission_native(state: &V8BindingState, feature: &str, disposition: &str) {
    if state.execution_retired() {
        return;
    }
    let url = state
        .dom()
        .borrow()
        .arena()
        .get(state.document_root())
        .ok()
        .and_then(|node| node.document_metadata.as_ref())
        .and_then(|metadata| metadata.url.clone())
        .unwrap_or_default();
    let report = Report {
        serialized: None,
        value: serde_json::json!({"type":"permissions-policy-violation","url":url,"body":{"featureId":feature,"disposition":disposition,"sourceFile":url,"lineNumber":0,"columnNumber":0}}),
    };
    queue_report(state, report);
}
// CSP fields are DOMStrings. Serialize their exact UTF-16 units instead of
// allowing Rust's scalar-only JSON strings to replace lone surrogates.
fn csp_string(value: &str) -> String {
    use std::fmt::Write;
    let mut text = String::from("\"");
    for unit in oryn_dom::dom_string::units(value) {
        write!(&mut text, "\\u{unit:04x}").expect("String writing");
    }
    text.push('"');
    text
}
fn csp_url_string(value: &str) -> String {
    serde_json::to_string(&String::from_utf16_lossy(&oryn_dom::dom_string::units(
        value,
    )))
    .expect("string JSON")
}
pub(super) fn csp(state: &V8BindingState, event: &crate::platform_control::CspEvent) {
    if state.execution_retired() {
        return;
    }
    let mut fields = Vec::new();
    for (name, value) in [
        ("documentURL", &event.document_uri),
        ("referrer", &event.referrer),
        ("blockedURL", &event.blocked_uri),
        ("effectiveDirective", &event.effective_directive),
        ("originalPolicy", &event.original_policy),
        ("sample", &event.sample),
        ("disposition", &event.disposition),
    ] {
        fields.push(format!(
            "\"{name}\":{}",
            if matches!(name, "documentURL" | "referrer" | "blockedURL") {
                csp_url_string(value)
            } else {
                csp_string(value)
            }
        ));
    }
    fields.push(format!(
        "\"sourceFile\":{}",
        if event.source_file.is_empty() {
            "null".into()
        } else {
            csp_url_string(&event.source_file)
        }
    ));
    for (name, value) in [
        ("lineNumber", event.line_number),
        ("columnNumber", event.column_number),
    ] {
        fields.push(format!(
            "\"{name}\":{}",
            if value == 0 {
                "null".into()
            } else {
                value.to_string()
            }
        ));
    }
    fields.push(format!("\"statusCode\":{}", event.status_code));
    let serialized = format!(
        "{{\"type\":\"csp-violation\",\"url\":{},\"body\":{{{}}}}}",
        csp_string(&event.document_uri),
        fields.join(",")
    );
    let report = Report {
        value: serde_json::json!({"type":"csp-violation"}),
        serialized: Some(serialized),
    };
    queue_report(state, report);
}

// Exact local no-work witness, independent of suspension/readiness policy.
pub(super) fn queues_empty(state: &V8BindingState) -> bool {
    state
        .reporting
        .borrow()
        .observers
        .values()
        .all(|observer| observer.queue.is_empty())
}
pub(super) fn ready(state: &V8BindingState) -> bool {
    !state.execution_retired()
        && !state.execution_suspended()
        && state
            .reporting
            .borrow()
            .observers
            .values()
            .any(|observer| !observer.queue.is_empty())
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    let ids = state
        .reporting
        .borrow()
        .observers
        .iter()
        .filter(|(_, observer)| !observer.queue.is_empty())
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    let mut count = 0;
    for id in ids {
        if state.execution_retired() || state.execution_suspended() {
            break;
        }
        if scope.is_execution_terminating() {
            return Err(V8BindingError::JavaScriptException);
        }
        let value = {
            let registry = state.reporting.borrow();
            let Some(observer) = registry.observers.get(&id) else {
                continue;
            };
            let object = observer
                .object
                .to_local(scope)
                .ok_or(V8BindingError::Allocation("report observer owner"))?;
            let callback_key = key(scope, "oryn.reporting.callback");
            let callback = object
                .get_private(scope, callback_key)
                .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                .ok_or(V8BindingError::Allocation("report observer callback"))?;
            (object, callback, observer.queue.clone())
        };
        if value.2.is_empty() {
            continue;
        }
        // Fresh native data properties cannot invoke author Array prototype setters.
        // Keep the authoritative queue until all fallible argument construction ends.
        // Web IDL callback arguments are converted in the callback's realm. The
        // observer still belongs to its registration document; a foreign callback
        // must receive that callback realm's Array and dictionary prototypes.
        let callback_context = value.1.get_creation_context(scope).ok_or_else(failure)?;
        let scope = &mut v8::ContextScope::new(scope, callback_context);
        let reports = array(scope, &value.2)?;
        let object = value.0;
        let callback = value.1;
        if scope.is_execution_terminating() {
            return Err(V8BindingError::JavaScriptException);
        }
        {
            let mut registry = state.reporting.borrow_mut();
            let Some(observer) = registry.observers.get_mut(&id) else {
                continue;
            };
            observer.queue.drain(..value.2.len());
            if !observer.active && observer.queue.is_empty() {
                observer.keepalive = None;
            }
        }
        v8::tc_scope!(let tc,scope);
        let receiver = v8::undefined(tc);
        callback.call(tc, receiver.into(), &[reports, object.into()]);
        count += 1;
        if tc.is_execution_terminating() {
            return Err(V8BindingError::JavaScriptException);
        }
        if let Some(exception) = tc.exception() {
            let message = tc.message();
            tc.reset();
            let _ = report_exception(tc, exception, message);
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pause_document(
        scope: &mut v8::PinScope,
        _args: v8::FunctionCallbackArguments,
        _rv: v8::ReturnValue,
    ) {
        scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .unwrap()
            .execution_suspended
            .set(true);
    }
    #[test]
    fn foreign_new_target_and_borrowed_methods_keep_registration_owner() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, text: &str) -> v8::Local<'s, v8::Value> {
            let text = v8::String::new(scope, text).unwrap();
            v8::Script::compile(scope, text, None)
                .unwrap()
                .run(scope)
                .unwrap()
        }
        let (first_state, constructor) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap();
            (state, eval(scope, "ReportingObserver"))
        };
        let second_state = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap();
            let name = v8::String::new(scope, "ForeignObserver").unwrap();
            second.global(scope).set(scope, name.into(), constructor);
            eval(
                scope,
                "globalThis.calls=0;function Alternate(){};Alternate.prototype=ReportingObserver.prototype;globalThis.observer=Reflect.construct(ForeignObserver,[()=>calls++],Alternate);ReportingObserver.prototype.observe.call(observer)",
            );
            state
        };
        assert_eq!(first_state.reporting.borrow().observers.len(), 1);
        assert!(second_state.reporting.borrow().observers.is_empty());
        permission_native(&first_state, "first", "enforce");
        permission_native(&second_state, "second", "enforce");
        {
            let scope = &mut v8::ContextScope::new(scope, second);
            assert!(eval(scope,"(()=>{const records=ReportingObserver.prototype.takeRecords.call(observer);ReportingObserver.prototype.disconnect.call(observer);return records.length===1&&records[0].body.featureId==='first'&&calls===0})()").is_true());
        }
        assert!(!ready(&first_state));
        assert!(!ready(&second_state));
    }
    #[test]
    fn foreign_callback_receives_its_own_realm_report_values() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let owner_context = v8::Context::new(scope, Default::default());
        let callback_context = v8::Context::new(scope, Default::default());
        let callback = {
            let scope = &mut v8::ContextScope::new(scope, callback_context);
            let code=v8::String::new(scope,"globalThis.calls=0;(batch,observer)=>{calls++;globalThis.valid=batch instanceof Array&&Object.getPrototypeOf(batch[0])===Object.prototype&&Object.getPrototypeOf(batch[0].body)===Object.prototype&&observer===globalThis.expected}").unwrap();
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
        };
        let state = {
            let scope = &mut v8::ContextScope::new(scope, owner_context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state =
                install_generated_bindings(scope, owner_context, dom, RealmId(0), 1).unwrap();
            let name = v8::String::new(scope, "foreignCallback").unwrap();
            owner_context
                .global(scope)
                .set(scope, name.into(), callback);
            let code=v8::String::new(scope,"globalThis.observer=new ReportingObserver(foreignCallback);observer.observe();observer").unwrap();
            let observer = v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap();
            let scope = &mut v8::ContextScope::new(scope, callback_context);
            let name = v8::String::new(scope, "expected").unwrap();
            assert_eq!(
                callback_context
                    .global(scope)
                    .create_data_property(scope, name.into(), observer),
                Some(true)
            );
            state
        };
        {
            let scope = &mut v8::ContextScope::new(scope, owner_context);
            permission_native(&state, "geolocation", "enforce");
            FAIL_ARRAY_ONCE.with(|flag| flag.set(true));
            assert!(poll(scope, &state).is_err());
            assert!(ready(&state));
            assert_eq!(poll(scope, &state).unwrap(), 1);
            assert!(!ready(&state));
        }
        {
            let scope = &mut v8::ContextScope::new(scope, callback_context);
            let code = v8::String::new(
                scope,
                "JSON.stringify({calls,valid,expectedType:typeof expected})",
            )
            .unwrap();
            let diagnostic = v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .to_rust_string_lossy(scope);
            let code = v8::String::new(scope, "calls===1&&valid").unwrap();
            assert!(
                v8::Script::compile(scope, code, None)
                    .unwrap()
                    .run(scope)
                    .unwrap()
                    .is_true(),
                "{diagnostic}"
            );
        }
    }
    #[test]
    fn reporting_preserves_queues_across_suspension_and_argument_failure() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(150), 1).unwrap();
        let pause = v8::Function::new(scope, pause_document).unwrap();
        let key = v8::String::new(scope, "pauseNativeDocument").unwrap();
        context.global(scope).set(scope, key.into(), pause.into());
        fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
            let code = v8::String::new(scope, code).unwrap();
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
        }
        eval(
            scope,
            "globalThis.calls='';globalThis.setterCalls=0;globalThis.first=new ReportingObserver(batch=>{calls+='a'+batch.length;pauseNativeDocument();});globalThis.second=new ReportingObserver(batch=>{calls+='b'+batch.length;});first.observe();second.observe();Object.defineProperty(Array.prototype,'0',{configurable:true,set(){setterCalls++;}})",
        );
        permission(scope, &state, "geolocation", "enforce").unwrap();
        state.execution_suspended.set(true);
        assert!(!ready(&state));
        assert_eq!(poll(scope, &state).unwrap(), 0);
        assert!(
            state
                .reporting
                .borrow()
                .observers
                .values()
                .all(|observer| observer.queue.len() == 1)
        );
        state.execution_suspended.set(false);
        FAIL_ARRAY_ONCE.with(|flag| flag.set(true));
        assert!(poll(scope, &state).is_err());
        assert!(
            state
                .reporting
                .borrow()
                .observers
                .values()
                .all(|observer| observer.queue.len() == 1)
        );
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert!(!ready(&state));
        assert_eq!(
            state
                .reporting
                .borrow()
                .observers
                .values()
                .map(|observer| observer.queue.len())
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        state.execution_suspended.set(false);
        assert!(ready(&state));
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert!(eval(scope, "calls==='a1b1'&&setterCalls===0").is_true());
        permission(scope, &state, "geolocation", "report").unwrap();
        FAIL_ARRAY_ONCE.with(|flag| flag.set(true));
        assert!(
            eval(
                scope,
                "(()=>{try{second.takeRecords();return false}catch(error){return true}})()"
            )
            .is_true()
        );
        assert!(eval(scope, "second.takeRecords().length===1").is_true());
        eval(
            scope,
            "delete Array.prototype[0];first.takeRecords();first.disconnect();second.disconnect()",
        );
        assert!(!ready(&state));
    }
    #[test]
    fn disconnect_preserves_admitted_delivery_and_buffers_are_per_type() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
            let code = v8::String::new(scope, code).unwrap();
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
        }
        eval(
            scope,
            "globalThis.batches=[];globalThis.observer=new ReportingObserver(batch=>batches.push(batch));observer.observe()",
        );
        let event = crate::platform_control::CspEvent {
            id: 1,
            document_uri: "https://example.test/".into(),
            referrer: String::new(),
            blocked_uri: "inline".into(),
            effective_directive: "script-src".into(),
            original_policy: "script-src 'none'".into(),
            source_file: String::new(),
            sample: String::new(),
            disposition: "enforce".into(),
            status_code: 200,
            line_number: 0,
            column_number: 0,
        };
        csp(&state, &event);
        for index in 0..110 {
            permission_native(&state, &index.to_string(), "enforce");
        }
        eval(scope, "observer.disconnect()");
        assert!(ready(&state));
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert!(eval(scope,"batches.length===1&&batches[0].length===111&&batches[0][0].type==='csp-violation'&&batches[0][1].body.featureId==='0'&&batches[0][110].body.featureId==='109'").is_true());
        assert!(eval(scope,"(()=>{const late=new ReportingObserver(()=>{},{buffered:true});late.observe();const records=late.takeRecords();return records.length===101&&records[0].type==='csp-violation'&&records[1].body.featureId==='10'&&records[100].body.featureId==='109'})()").is_true());
        permission_native(&state, "new", "enforce");
        assert!(eval(scope, "observer.takeRecords().length===0").is_true());
    }

    #[test]
    fn reporting_observer_native_interface_and_conversion_contract() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let code=v8::String::new(scope,r#"(()=>{
   if(ReportingObserver.length!==1)throw Error('constructor arity');
   const observer=new ReportingObserver(()=>{});
   if(Object.prototype.toString.call(observer)!=='[object ReportingObserver]')throw Error('tag');
   for(const name of ['observe','disconnect','takeRecords']){
    const descriptor=Object.getOwnPropertyDescriptor(ReportingObserver.prototype,name);
    if(!descriptor.enumerable||!descriptor.configurable||!descriptor.writable||descriptor.value.name!==name||descriptor.value.length!==0)throw Error('method descriptor');
    for(const action of [()=>new descriptor.value(),()=>descriptor.value.call({}),()=>descriptor.value.call(new Proxy(observer,{}))]){
     let rejected=false;try{action()}catch(error){rejected=error instanceof TypeError}if(!rejected)throw Error('native receiver/constructor');
    }
   }
   let order=[];const marker={};
   try{new ReportingObserver(()=>{},{get buffered(){order.push('buffered');return true},get types(){order.push('types');return {[Symbol.iterator](){order.push('iterator');return {next(){throw marker}}}}}})}catch(error){if(error!==marker)throw Error('exception identity')}
   return order.join(',')==='buffered,types,iterator';
  })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }

    #[test]
    fn reporting_observer_cycles_release_when_idle_but_keep_pending_delivery() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
            let code = v8::String::new(scope, code).unwrap();
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
        }
        let idle = {
            v8::scope!(let inner,scope);
            let object = eval(
                inner,
                "(()=>{const observer=new ReportingObserver(()=>observer);return observer})()",
            )
            .try_cast::<v8::Object>()
            .unwrap();
            v8::Weak::new(inner, object)
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(idle.to_local(scope).is_none());
        let active = {
            v8::scope!(let inner,scope);
            let object=eval(inner,"(()=>{const observer=new ReportingObserver(()=>observer);observer.observe();return observer})()").try_cast::<v8::Object>().unwrap();
            v8::Weak::new(inner, object)
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        {
            v8::scope!(let inner,scope);
            let object = active
                .to_local(inner)
                .expect("registered observer retained");
            let name = v8::String::new(inner, "disconnect").unwrap();
            let method = object
                .get(inner, name.into())
                .unwrap()
                .try_cast::<v8::Function>()
                .unwrap();
            method.call(inner, object.into(), &[]).unwrap();
        }
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(active.to_local(scope).is_none());
        eval(scope, "globalThis.deliveries=0");
        let pending = {
            v8::scope!(let inner,scope);
            let object=eval(inner,"(()=>{const observer=new ReportingObserver(batch=>{deliveries+=batch.length;return observer});observer.observe();return observer})()").try_cast::<v8::Object>().unwrap();
            permission_native(&state, "camera", "enforce");
            let name = v8::String::new(inner, "disconnect").unwrap();
            let method = object
                .get(inner, name.into())
                .unwrap()
                .try_cast::<v8::Function>()
                .unwrap();
            method.call(inner, object.into(), &[]).unwrap();
            v8::Weak::new(inner, object)
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        {
            v8::scope!(let inner,scope);
            assert!(pending.to_local(inner).is_some());
            assert_eq!(poll(inner, &state).unwrap(), 1);
        }
        assert!(eval(scope, "deliveries===1").is_true());
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(pending.to_local(scope).is_none());
        // New admission prunes dead native records instead of exhausting the limit.
        eval(scope, "new ReportingObserver(()=>{})");
        assert_eq!(state.reporting.borrow().observers.len(), 1);
    }

    #[test]
    fn native_reports_are_queued_filterable_serializable_and_retired_with_the_document() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
            let code = v8::String::new(scope, code).unwrap();
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
        }
        eval(
            scope,
            "globalThis.reports=[];globalThis.observer=new ReportingObserver((batch,owner)=>{if(owner!==observer)throw Error('owner');reports.push(...batch)},{types:['permissions-policy-violation']});observer.observe();globalThis.ignored=new ReportingObserver(()=>{throw Error('unrelated type')},{types:['deprecation']});ignored.observe()",
        );
        let location = trusted_source_location(scope);
        assert!(location.source_file.is_empty());
        assert_eq!((location.line, location.column), (0, 0));
        permission(scope, &state, "bluetooth", "enforce").unwrap();
        assert!(eval(scope, "reports.length===0").is_true());
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert!(eval(scope,"reports.length===1&&reports[0].body.featureId==='bluetooth'&&reports[0].body.disposition==='enforce'&&reports[0].body.lineNumber===0&&reports[0].body.columnNumber===0&&reports[0].body.sourceFile===reports[0].url&&JSON.stringify(reports[0])===JSON.stringify(reports[0].toJSON())").is_true());
        eval(
            scope,
            "observer.disconnect();globalThis.buffered=new ReportingObserver(()=>{throw Error('takeRecords drains queue')},{buffered:true,types:['permissions-policy-violation']});buffered.observe();globalThis.taken=buffered.takeRecords()",
        );
        assert!(eval(scope, "taken.length===1").is_true());
        assert_eq!(poll(scope, &state).unwrap(), 0);
        state.retire_native_platform();
        assert!(!ready(&state));
    }
}
