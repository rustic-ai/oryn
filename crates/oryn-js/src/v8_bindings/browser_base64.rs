//! HTML binary-string base64 operations shared by Window and workers.
use super::*;

pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    for (name, encode) in [("btoa", true), ("atob", false)] {
        let data = v8::Boolean::new(scope, encode);
        let function = v8::Function::builder(convert)
            .data(data.into())
            .length(1)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("base64 function"))?;
        let name = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("base64 name"))?;
        function.set_name(name);
        context.global(scope).define_own_property(
            scope,
            name.into(),
            function.into(),
            v8::PropertyAttribute::NONE,
        );
    }
    Ok(())
}

fn convert(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| {
        if args.length() == 0 {
            return Err(V8BindingError::Unsupported {
                interface: "WindowOrWorkerGlobalScope".into(),
                member: "TypeError".into(),
            });
        }
        let string = args
            .get(0)
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        // HTML uses binary strings: preserve UTF-16 until every code unit has
        // been checked. UTF-8 conversion would lose lone surrogates and bytes.
        let mut units = vec![0; string.length()];
        string.write_v2(scope, 0, &mut units, v8::WriteFlags::default());
        let invalid = || V8BindingError::Unsupported {
            interface: "WindowOrWorkerGlobalScope".into(),
            member: "InvalidCharacterError".into(),
        };
        let mut bytes = units
            .into_iter()
            .map(|unit| u8::try_from(unit).map_err(|_| invalid()))
            .collect::<Result<Vec<_>, _>>()?;
        if args.data().boolean_value(scope) {
            return v8_string(scope, &BASE64.encode(bytes));
        }
        // Infra forgiving-base64 removes only ASCII whitespace, permits
        // omitted padding and ignores discarded bits in the final sextet.
        bytes.retain(|b| !matches!(b, b'\t' | b'\n' | b'\x0c' | b'\r' | b' '));
        if bytes.len() % 4 == 0 {
            for _ in 0..2 {
                if bytes.last() == Some(&b'=') {
                    bytes.pop();
                }
            }
        }
        use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
        let decoder = GeneralPurpose::new(
            &base64::alphabet::STANDARD,
            GeneralPurposeConfig::new()
                .with_decode_padding_mode(DecodePaddingMode::RequireNone)
                .with_decode_allow_trailing_bits(true),
        );
        let decoded = decoder.decode(&bytes).map_err(|_| invalid())?;
        v8::String::new_from_one_byte(scope, &decoded, v8::NewStringType::Normal)
            .map(Into::into)
            .ok_or(V8BindingError::Allocation("decoded base64"))
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base64_globals_preserve_binary_strings_and_webidl_conversion() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        for global in [BindingGlobal::Window, BindingGlobal::Worker] {
            v8::scope!(let scope, &mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(DomRuntime::new()));
            let document = dom.borrow().document();
            install_generated_bindings_with_root_for_global(
                scope,
                context,
                dom,
                RealmId(991),
                1,
                document,
                global,
            )
            .unwrap();
            let script = v8::String::new(scope, r#"(()=>{
                const check=(v,m)=>{if(!v)throw Error(m)};
                check(typeof btoa==='function' && typeof atob==='function','native global exposure');
                check(btoa.length===1 && atob.length===1,'arity');
                const binary=Array.from({length:256},(_,i)=>String.fromCharCode(i)).join('');
                check(atob(btoa(binary))===binary,'all Latin-1 bytes including NUL');
                check(btoa(undefined)==='dW5kZWZpbmVk','explicit undefined');
                check(atob(' Z h ==\t\n\f\r')==='f','forgiving alphabet and discarded pad bits');
                check(atob('Zm9')==='fo','omitted padding');
                for(const input of ['a','a===','Zg=','Zg===','Zg==x','Zg==\v','Zg==\u00a0','-_','\u0100']) {
                    let e;try{atob(input)}catch(x){e=x}check(e instanceof DOMException&&e.name==='InvalidCharacterError','decode '+input);
                }
                for(const input of ['\u0100','\ud800','\udc00','\ud800\udc00']) {
                    let e;try{btoa(input)}catch(x){e=x}check(e instanceof DOMException&&e.name==='InvalidCharacterError','nonbyte encoding');
                }
                for(const fn of [btoa,atob]) {
                    let e;try{fn()}catch(x){e=x}check(e instanceof TypeError,'required argument');
                    e=null;try{fn(Symbol())}catch(x){e=x}check(e instanceof TypeError,'symbol conversion');
                    const sentinel={};let calls=0;e=null;
                    try{fn({toString(){calls++;throw sentinel}})}catch(x){e=x}
                    check(e===sentinel&&calls===1,'preserve conversion exception');
                    e=null;try{new fn('')}catch(x){e=x}check(e instanceof TypeError,'not constructible');
                }
                return true;
            })()"#).unwrap();
            assert!(
                v8::Script::compile(scope, script, None)
                    .unwrap()
                    .run(scope)
                    .unwrap()
                    .is_true()
            );
        }
    }
}
