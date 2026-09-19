//! Parent-produced violations retain native payloads and trusted dispatch.
use super::*;
const MEMBERS: [&str; 12] = [
    "blockedURI",
    "columnNumber",
    "disposition",
    "documentURI",
    "effectiveDirective",
    "lineNumber",
    "originalPolicy",
    "referrer",
    "sample",
    "sourceFile",
    "statusCode",
    "violatedDirective",
];
fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "SecurityPolicyViolationEvent".into(),
        member: "TypeError: incompatible CSP event".into(),
    }
}
fn store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    values: &[v8::Local<'s, v8::Value>],
) -> Result<(), V8BindingError> {
    let brand = v8_string(scope, "SecurityPolicyViolationEvent")?;
    let mut items = vec![brand];
    items.extend_from_slice(values);
    let payload = v8::Array::new_with_elements(scope, &items);
    let null = v8::null(scope);
    if payload.set_prototype(scope, null.into()) != Some(true) {
        return Err(invalid());
    }
    event.set_internal_field(2, payload.into());
    Ok(())
}
pub(super) fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Result<(), V8BindingError> {
    let mut values = Vec::new();
    for name in &MEMBERS[..12] {
        let value =
            event_init_member(scope, init, name).ok_or(V8BindingError::JavaScriptException)?;
        let converted = if matches!(*name, "columnNumber" | "lineNumber" | "statusCode") {
            let value = if value.is_undefined() {
                0.
            } else {
                value
                    .number_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?
            };
            let modulo = if *name == "statusCode" {
                65536.
            } else {
                4294967296.
            };
            let value = if value.is_finite() {
                value.trunc().rem_euclid(modulo)
            } else {
                0.
            };
            v8::Number::new(scope, value).into()
        } else {
            let text = if value.is_undefined() {
                if *name == "disposition" {
                    "enforce".into()
                } else {
                    String::new()
                }
            } else {
                {
                    let string = value
                        .to_string(scope)
                        .ok_or(V8BindingError::JavaScriptException)?;
                    rust_string_preserving_utf16(scope, string)?
                }
            };
            if *name == "disposition" && !matches!(text.as_str(), "enforce" | "report") {
                return Err(invalid());
            }
            if matches!(
                *name,
                "documentURI" | "referrer" | "blockedURI" | "sourceFile"
            ) {
                let scalar = String::from_utf16_lossy(&oryn_dom::dom_string::units(&text));
                v8::String::new(scope, &scalar)
                    .ok_or(V8BindingError::Allocation("CSP USVString"))?
                    .into()
            } else {
                v8_string(scope, &text)?
            }
        };
        values.push(converted);
    }
    store(scope, event, &values)
}
pub(super) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if event.is_proxy() {
        return Err(invalid());
    }
    event_id(scope, event)?;
    let payload = event
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(invalid)?;
    if !payload.get_index(scope, 0).is_some_and(|value| {
        value.is_string() && value.to_rust_string_lossy(scope) == "SecurityPolicyViolationEvent"
    }) {
        return Err(invalid());
    }
    let index = MEMBERS[..12]
        .iter()
        .position(|member| *member == name)
        .ok_or_else(invalid)?;
    payload
        .get_index(scope, index as u32 + 1)
        .ok_or_else(invalid)
}
pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    value: &crate::platform_control::CspEvent,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let event = create_event_object_with_interface(
        scope,
        state,
        "SecurityPolicyViolationEvent",
        "securitypolicyviolation",
        true,
        false,
    )?;
    let mut values = Vec::new();
    for name in &MEMBERS[..12] {
        values.push(match *name {
            "columnNumber" => v8::Number::new(scope, value.column_number as f64).into(),
            "lineNumber" => v8::Number::new(scope, value.line_number as f64).into(),
            "statusCode" => v8::Number::new(scope, value.status_code as f64).into(),
            name => {
                let text = match name {
                    "blockedURI" => &value.blocked_uri,
                    "disposition" => &value.disposition,
                    "documentURI" => &value.document_uri,
                    "effectiveDirective" | "violatedDirective" => &value.effective_directive,
                    "originalPolicy" => &value.original_policy,
                    "referrer" => &value.referrer,
                    "sample" => &value.sample,
                    "sourceFile" => &value.source_file,
                    _ => return Err(invalid()),
                };
                if matches!(
                    name,
                    "documentURI" | "referrer" | "blockedURI" | "sourceFile"
                ) {
                    let scalar = String::from_utf16_lossy(&oryn_dom::dom_string::units(text));
                    v8::String::new(scope, &scalar)
                        .ok_or(V8BindingError::Allocation("CSP USVString"))?
                        .into()
                } else {
                    v8_string(scope, text)?
                }
            }
        });
    }
    store(scope, event, &values)?;
    let id = event_id(scope, event)?;
    state
        .events
        .borrow_mut()
        .get_mut(&id)
        .ok_or_else(invalid)?
        .is_trusted = true;
    Ok(event)
}
#[cfg(test)]
pub(super) fn dispatch(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: &crate::platform_control::CspEvent,
) -> Result<(), V8BindingError> {
    let event = create(scope, state, value)?;
    dispatch_created(scope, state, event)
}
#[cfg(test)]
pub(super) fn dispatch_created<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    event: v8::Local<'s, v8::Object>,
) -> Result<(), V8BindingError> {
    if state.binding_global == BindingGlobal::Window {
        dispatch_event(scope, state, state.document_root(), event).map(|_| ())
    } else {
        global_events::dispatch(scope, state, event).map(|_| ())
    }
}

pub(super) fn poll_agent(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.execution_retired()
        || state.binding_global == BindingGlobal::Window
        || state.native_platform.borrow().publishing
    {
        return Ok(0);
    }
    let creator = state.native_worker_creator()?;
    if !matches!(creator, crate::worker_wire::Creator::Agent(_)) {
        return Ok(0);
    }
    let transport = state
        .worker_transport
        .borrow()
        .clone()
        .ok_or(V8BindingError::Allocation("worker CSP transport"))?;
    let mut count = native_platform::publication::drain(scope, state)?;
    if !state.native_platform.borrow().publications.is_empty()
        || state.execution_retired()
        || scope.is_execution_terminating()
    {
        return Ok(count);
    }
    let after = state.native_platform.borrow().csp_cursor;
    let response = transport
        .request(crate::worker_wire::AgentServiceRequest::CspEvents { after })
        .map_err(|_| V8BindingError::Allocation("worker CSP event polling"))?;
    let crate::worker_wire::AgentServiceResponse::CspEvents(events) = response else {
        return Err(V8BindingError::Allocation("worker CSP response"));
    };
    let mut previous = after;
    for event in &events {
        if event.id <= previous {
            return Err(V8BindingError::Allocation("worker CSP cursor regression"));
        }
        previous = event.id;
    }
    for event in events {
        native_platform::publication::enqueue(
            state,
            creator.clone(),
            native_platform::publication::Payload::Csp(event),
        )?;
    }
    count += native_platform::publication::drain(scope, state)?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, text: &str) -> v8::Local<'s, v8::Value> {
        let text = v8::String::new(scope, text).unwrap();
        v8::Script::compile(scope, text, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn csp_event_dictionary_preserves_domstrings_and_numeric_conversions() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const fields=['blockedURI','documentURI','effectiveDirective','originalPolicy','referrer','sample','sourceFile','violatedDirective'];
            for(const value of ['\uD800','\uDC00','\uD83D\uDE00','\0']){
                const init=Object.fromEntries(fields.map(field=>[field,{toString(){return value}}]));
                const event=new SecurityPolicyViolationEvent('x',init);
                const scalar=value==='\uD800'||value==='\uDC00'?'\uFFFD':value;
                if(!fields.every(field=>event[field]===(['blockedURI','documentURI','referrer','sourceFile'].includes(field)?scalar:value)))throw Error('CSP string category');
            }
            const event=new SecurityPolicyViolationEvent('x',{columnNumber:-1,lineNumber:4294967297,statusCode:65537});
            if(event.columnNumber!==4294967295||event.lineNumber!==1||event.statusCode!==1)throw Error('integer conversion');
            for(const field of fields){const event=new SecurityPolicyViolationEvent('x',{[field]:null});if(event[field]!=='null')throw Error('DOMString null');}
            let rejected=false;try{new SecurityPolicyViolationEvent('x',{disposition:'other'})}catch(error){rejected=error instanceof TypeError}
            return rejected;
        })()"#).is_true());
    }
    #[test]
    fn native_csp_event_payload_brand_and_trust_survive_author_constructor_replacement() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>csp</p>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"globalThis.Csp=SecurityPolicyViolationEvent;const e=new Csp('custom',{blockedURI:'test',statusCode:65537});let rejected=false;try{Object.getOwnPropertyDescriptor(Csp.prototype,'blockedURI').get.call(new Event('x'))}catch(e){rejected=e instanceof TypeError}e.blockedURI==='test'&&e.statusCode===1&&!e.isTrusted&&rejected"#).is_true());
        eval(
            scope,
            r#"globalThis.seen=[];document.addEventListener('securitypolicyviolation',e=>seen.push([e instanceof Csp,e.isTrusted,e.blockedURI,e.effectiveDirective,e.target===document,e.bubbles,e.cancelable]));globalThis.SecurityPolicyViolationEvent=()=>{throw Error('author constructor')};document.dispatchEvent=()=>{throw Error('author dispatch')}"#,
        );
        dispatch(
            scope,
            &state,
            &crate::platform_control::CspEvent {
                id: 7,
                document_uri: "https://example.test/".into(),
                referrer: String::new(),
                blocked_uri: "https://example.test/filter.svg".into(),
                effective_directive: "img-src".into(),
                original_policy: "img-src 'none'".into(),
                source_file: String::new(),
                sample: String::new(),
                disposition: "enforce".into(),
                status_code: 200,
                line_number: 0,
                column_number: 0,
            },
        )
        .unwrap();
        assert_eq!(
            eval(scope, "JSON.stringify(seen)").to_rust_string_lossy(scope),
            r#"[[true,true,"https://example.test/filter.svg","img-src",true,true,false]]"#
        );
    }
}

#[cfg(test)]
mod worker_tests {
    use super::*;
    #[test]
    fn worker_global_receives_native_violation_once_without_document_target() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let document = dom.borrow().document();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings_with_root_for_global(
            scope,
            context,
            dom,
            RealmId(0),
            1,
            document,
            BindingGlobal::Worker,
        )
        .unwrap();
        struct Transport;
        impl crate::worker_wire::WorkerTransport for Transport {
            fn request(
                &self,
                request: crate::worker_wire::AgentServiceRequest,
            ) -> Result<crate::worker_wire::AgentServiceResponse, crate::worker_wire::WorkerError>
            {
                match request {
                    crate::worker_wire::AgentServiceRequest::CspEvents { after } => Ok(
                        crate::worker_wire::AgentServiceResponse::CspEvents(if after == 0 {
                            vec![crate::platform_control::CspEvent {
                                id: 1,
                                document_uri: "https://worker.test/main.js".into(),
                                referrer: String::new(),
                                blocked_uri: "https://worker.test/import.js".into(),
                                effective_directive: "script-src-elem".into(),
                                original_policy: "script-src 'none'".into(),
                                source_file: String::new(),
                                sample: String::new(),
                                disposition: "enforce".into(),
                                status_code: 0,
                                line_number: 0,
                                column_number: 0,
                            }]
                        } else {
                            Vec::new()
                        }),
                    ),
                    _ => Ok(crate::worker_wire::AgentServiceResponse::Done),
                }
            }
        }
        state.set_worker_transport(
            std::sync::Arc::new(Transport),
            Some(crate::worker_wire::Creator::Agent(
                crate::worker_wire::AgentId(7),
            )),
            Some("https://worker.test/main.js".into()),
        );
        let source=v8::String::new(scope,"let seen=0;onsecuritypolicyviolation=e=>{if(!(e instanceof SecurityPolicyViolationEvent)||!e.isTrusted||e.target!==globalThis||e.effectiveDirective!=='script-src-elem')throw Error('worker violation');seen++}").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert_eq!(poll_agent(scope, &state).unwrap(), 1);
        assert_eq!(poll_agent(scope, &state).unwrap(), 0);
        let source = v8::String::new(scope, "seen===1").unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}
