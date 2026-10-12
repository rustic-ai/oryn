//! URL static parsing shares constructor conversion and the native URL record.
use super::*;

pub(super) fn parse_arguments(
    scope: &mut v8::PinScope,
    args: &v8::FunctionCallbackArguments,
) -> Result<Option<Url>, V8BindingError> {
    if args.length() == 0 {
        return Err(V8BindingError::Unsupported {
            interface: "URL".into(),
            member: "TypeError".into(),
        });
    }
    // V8 ToString preserves exceptions (including Symbols); lossy UTF-16 to
    // Rust conversion implements USVString's unpaired-surrogate replacement.
    let input = args
        .get(0)
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope);
    let base = if args.length() > 1 && !args.get(1).is_undefined() {
        Some(
            args.get(1)
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope),
        )
    } else {
        None
    };
    let parsed = if let Some(base) = base {
        Url::parse(&base).and_then(|base| base.join(&input))
    } else {
        Url::parse(&input)
    };
    Ok(parsed.ok())
}
fn parse(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| {
        let Some(url) = parse_arguments(scope, &args)? else {
            return Ok(None);
        };
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or(V8BindingError::Allocation("URL.parse binding state"))?;
        let id = register_platform_object(&state, PlatformObjectRecord::Url(url));
        new_platform_object(scope, &state, "URL", id).map(Some)
    })();
    match result {
        Ok(Some(url)) => rv.set(url.into()),
        Ok(None) => rv.set(v8::null(scope).into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn can_parse(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match parse_arguments(scope, &args) {
        Ok(url) => rv.set_bool(url.is_some()),
        Err(error) => throw_binding_error(scope, &error),
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    constructor: v8::Local<v8::Function>,
) -> Result<(), V8BindingError> {
    let parse = v8::Function::builder(parse)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("URL.parse"))?;
    let can_parse = v8::Function::builder(can_parse)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("URL.canParse"))?;
    for (name, callback) in [("parse", parse), ("canParse", can_parse)] {
        let name =
            v8::String::new(scope, name).ok_or(V8BindingError::Allocation("URL static name"))?;
        callback.set_name(name);
        if constructor.create_data_property(scope, name.into(), callback.into()) != Some(true) {
            return Err(V8BindingError::Allocation("URL static property"));
        }
    }
    Ok(())
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
    fn static_url_conversion_parsing_and_fresh_query_records_on_window_and_workers() {
        super::super::tests::initialize_v8();
        for global in [BindingGlobal::Window, BindingGlobal::Worker] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope, &mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
            let root = dom.borrow().document();
            let _state = install_generated_bindings_with_root_for_global(
                scope,
                context,
                dom,
                RealmId(0),
                1,
                root,
                global,
            )
            .unwrap();
            assert!(eval(scope, r#"(()=>{
                for(const method of ['parse','canParse']) {
                    const f=URL[method], d=Object.getOwnPropertyDescriptor(URL,method);
                    if(f.length!==1 || f.name!==method || !d.enumerable || !d.writable || !d.configurable)throw Error('static descriptor');
                    for(const args of [[],[Symbol()],[{},Symbol()]]) {
                        try { f(...args); throw Error('missing conversion error'); } catch(e) { if(!(e instanceof TypeError))throw e; }
                    }
                    try { new f('https://example.test/'); throw Error('constructible static'); } catch(e) { if(!(e instanceof TypeError))throw e; }
                    const marker={}, order=[];
                    try { f({toString(){order.push('url');return 'invalid'}},{toString(){order.push('base');throw marker}}); throw Error('conversion swallowed'); } catch(e) { if(e!==marker)throw e; }
                    if(order.join(',')!=='url,base')throw Error('argument order');
                    const stop=[];
                    try { f({toString(){throw marker}},{toString(){stop.push('base');return 'https://base.test/'}}); throw Error('first conversion swallowed'); } catch(e) { if(e!==marker||stop.length)throw Error('first conversion order'); }
                }
                for(const args of [['https://example.test/a?x=1#part'],['../b','https://user:pass@example.test:443/a/'],['https://example.test/',undefined],['x',undefined],['https://example.test/','invalid'],['https://example.test/',null],['https://example.test/\ud800'],['x','https://example.test/\ud800'],[null], [undefined], ['http://[broken']]) {
                    let expected=null; try { expected=new URL(...args); } catch(e) { if(!(e instanceof TypeError))throw e; }
                    const parsed=URL.parse(...args);
                    if(URL.canParse(...args)!==(expected!==null))throw Error('canParse constructor parity');
                    if(expected===null) { if(parsed!==null)throw Error('invalid parse must be null'); }
                    else if(!(parsed instanceof URL)||parsed.href!==expected.href)throw Error('parse constructor parity');
                }
                const a=URL.parse('/path?x=1','https://example.test/'), b=URL.parse('/path?x=1','https://example.test/');
                if(a===b || a.searchParams===b.searchParams || a.searchParams!==a.searchParams)throw Error('fresh URL/query identity');
                a.searchParams.set('x','2'); if(a.search!=='?x=2'||b.search!=='?x=1')throw Error('query aliases');
                a.href='https://next.test/?z=3'; if(a.searchParams.get('z')!=='3'||b.host!=='example.test')throw Error('href aliases');
                const native=URL, parse=native.parse; globalThis.URL=function(){throw Error('public constructor invoked')};
                try { const parsed=parse.call(null,'https://fresh.test/'); if(Object.getPrototypeOf(parsed)!==native.prototype||parsed.href!=='https://fresh.test/')throw Error('intrinsic wrapper'); }
                finally { globalThis.URL=native; }
                return true;
            })()"#).is_true());
        }
    }
    #[test]
    fn borrowed_parse_allocates_in_function_realm_without_using_this_constructor() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let _first = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(1), 1).unwrap()
        };
        let second = v8::Context::new(scope, Default::default());
        let _second = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(2), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        // Borrowing a constructor across page realms requires same-origin
        // access, represented by the shared native token, not relaxed checks.
        let token = v8::String::new(scope, "https://same-origin.test").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let other = second.global(scope);
        let name = v8::String::new(scope, "other").unwrap();
        first.global(scope).set(scope, name.into(), other.into());
        assert!(eval(scope, "(()=>{const parsed=other.URL.parse.call(URL,'https://realm.test/');return Object.getPrototypeOf(parsed)===other.URL.prototype&&Object.getPrototypeOf(parsed)!==URL.prototype&&parsed.href==='https://realm.test/'})()").is_true());
    }
}
