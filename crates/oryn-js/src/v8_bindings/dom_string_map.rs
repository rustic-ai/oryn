//! Live DOMStringMap named properties follow the associated native element.
use super::*;
fn owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<(Rc<V8BindingState>, NodeId), V8BindingError> {
    let key = v8::String::new(scope, "oryn.associated.owner")
        .ok_or(V8BindingError::Allocation("dataset owner key"))?;
    let key = v8::Private::for_api(scope, Some(key));
    let element = object
        .get_private(scope, key)
        .ok_or(V8BindingError::Allocation("dataset owner"))?;
    node_identity::argument(scope, element)?.ok_or(V8BindingError::Allocation("dataset element"))
}
fn key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
) -> Result<Option<String>, V8BindingError> {
    if name.is_symbol() {
        return Ok(None);
    }
    let name = v8::Local::<v8::String>::try_from(name)
        .map_err(|_| V8BindingError::Allocation("dataset name"))?;
    rust_string_preserving_utf16(scope, name).map(Some)
}
fn property(attribute: &str) -> Option<String> {
    let suffix = attribute.strip_prefix("data-")?;
    if suffix.chars().any(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let mut result = String::new();
    let mut chars = suffix.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '-' && chars.peek().is_some_and(|c| c.is_ascii_lowercase()) {
            result.push(chars.next().unwrap().to_ascii_uppercase())
        } else {
            result.push(c)
        }
    }
    Some(result)
}
fn attribute(name: &str) -> String {
    let mut result = String::from("data-");
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            result.push('-');
            result.push(c.to_ascii_lowercase())
        } else {
            result.push(c)
        }
    }
    result
}
fn names(state: &V8BindingState, node: NodeId) -> Result<Vec<String>, V8BindingError> {
    Ok(state
        .dom
        .borrow()
        .arena()
        .get(node)?
        .attributes
        .records()
        .filter(|a| a.namespace.is_none())
        .filter_map(|a| property(&a.name))
        .collect())
}
fn read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: v8::Local<'s, v8::Name>,
) -> Result<Option<String>, V8BindingError> {
    let Some(name) = key(scope, name)? else {
        return Ok(None);
    };
    let (state, node) = owner(scope, object)?;
    if !names(&state, node)?.contains(&name) {
        return Ok(None);
    }
    let value = state
        .dom
        .borrow()
        .attribute_in_null_namespace(node, &attribute(&name))?
        .map(str::to_owned);
    Ok(value)
}
fn write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: Option<v8::Local<'s, v8::Value>>,
) -> Result<(), V8BindingError> {
    owner(scope, object)?;
    let boundary = custom_elements::ReactionBoundary::begin(scope)?;
    let (result, exception) = {
        v8::tc_scope!(let caught,scope);
        let scope = &mut *caught;
        let result = (|| {
            let value = value
                .map(|value| {
                    value
                        .to_string(scope)
                        .ok_or(V8BindingError::JavaScriptException)
                        .and_then(|text| rust_string_preserving_utf16(scope, text))
                })
                .transpose()?;
            if !crate::v8_termination::checkpoint(scope) {
                return Err(V8BindingError::JavaScriptException);
            }
            if value.is_some()
                && name
                    .as_bytes()
                    .windows(2)
                    .any(|pair| pair[0] == b'-' && pair[1].is_ascii_lowercase())
            {
                return Err(V8BindingError::Unsupported {
                    interface: "DOMStringMap".into(),
                    member: "SyntaxError".into(),
                });
            }
            let name = attribute(name);
            let (state, node) = owner(scope, object)?;
            if let Some(value) = value {
                validate_attribute_local_name(&name)?;
                state.dom.borrow_mut().set_attribute(node, &name, value)?;
            } else {
                state.dom.borrow_mut().remove_attribute(node, &name)?;
            }
            Ok(())
        })();
        if caught.has_terminated() || caught.is_execution_terminating() {
            caught.rethrow();
            return Err(V8BindingError::JavaScriptException);
        }
        let exception = caught
            .exception()
            .map(|value| v8::Global::new(caught, value));
        caught.reset();
        (result, exception)
    };
    let completion = boundary.complete_result(scope);
    if let Some(exception) = exception {
        let exception = v8::Local::new(scope, &exception);
        scope.throw_exception(exception);
        return Err(V8BindingError::JavaScriptException);
    }
    result.and(completion)
}
fn getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match read(scope, args.holder(), name)
        .and_then(|value| value.map(|value| v8_string(scope, &value)).transpose())
    {
        Ok(Some(value)) => {
            rv.set(value);
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}
fn setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    let result = key(scope, name).and_then(|key| {
        key.map(|key| write(scope, args.holder(), &key, Some(value)))
            .transpose()
    });
    match result {
        Ok(None) => return v8::Intercepted::kNo,
        Ok(Some(())) => rv.set_bool(true),
        Err(error) => throw_binding_error(scope, &error),
    }
    v8::Intercepted::kYes
}
fn query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    match read(scope, args.holder(), name) {
        Ok(Some(_)) => {
            rv.set_int32(0);
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}
fn descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match read(scope, args.holder(), name)
        .and_then(|value| value.map(|value| v8_string(scope, &value)).transpose())
    {
        Ok(Some(value)) => {
            let descriptor = v8::Object::new(scope);
            for (name, value) in [
                ("value", value),
                ("writable", v8::Boolean::new(scope, true).into()),
                ("enumerable", v8::Boolean::new(scope, true).into()),
                ("configurable", v8::Boolean::new(scope, true).into()),
            ] {
                let name = v8::String::new(scope, name).unwrap();
                descriptor.create_data_property(scope, name.into(), value);
            }
            rv.set(descriptor.into());
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}
fn definer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
    desc: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    if name.is_symbol() {
        return v8::Intercepted::kNo;
    }
    if desc.has_get() || desc.has_set() {
        rv.set_bool(false);
        if args.should_throw_on_error() {
            throw_type_error(scope, "DOMStringMap cannot define a named accessor");
        }
        return v8::Intercepted::kYes;
    }
    let value = if desc.has_value() {
        v8::Local::new(scope, desc.value())
    } else {
        v8::undefined(scope).into()
    };
    setter(scope, name, value, args, rv)
}
fn deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    let result = key(scope, name).and_then(|name| {
        name.map(|name| write(scope, args.holder(), &name, None))
            .transpose()
    });
    match result {
        Ok(None) => return v8::Intercepted::kNo,
        Ok(Some(())) => rv.set_bool(true),
        Err(error) => throw_binding_error(scope, &error),
    }
    v8::Intercepted::kYes
}
fn index_name(name: &str) -> Option<u32> {
    name.parse::<u32>()
        .ok()
        .filter(|index| *index != u32::MAX && index.to_string() == name)
}
fn enumerate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<v8::Array>,
    indexed: bool,
) {
    let result = (|| {
        let (state, node) = owner(scope, args.holder())?;
        let names = names(&state, node)?;
        let mut values = Vec::new();
        for name in names {
            let number = index_name(&name);
            if number.is_some() != indexed {
                continue;
            }
            values.push(if let Some(number) = number {
                v8::Integer::new_from_unsigned(scope, number).into()
            } else {
                v8_string(scope, &name)?
            });
        }
        Ok::<_, V8BindingError>(v8::Array::new_with_elements(scope, &values))
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<v8::Array>,
) {
    enumerate(scope, args, rv, false)
}
fn indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<v8::Array>,
) {
    enumerate(scope, args, rv, true)
}
fn indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue,
) -> v8::Intercepted {
    let name = v8::String::new(scope, &index.to_string()).unwrap();
    getter(scope, name.into(), args, rv)
}
fn indexed_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    let name = v8::String::new(scope, &index.to_string()).unwrap();
    setter(scope, name.into(), value, args, rv)
}
fn indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    let name = v8::String::new(scope, &index.to_string()).unwrap();
    query(scope, name.into(), args, rv)
}
fn indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue,
) -> v8::Intercepted {
    let name = v8::String::new(scope, &index.to_string()).unwrap();
    descriptor(scope, name.into(), args, rv)
}
fn indexed_definer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    desc: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    let name = v8::String::new(scope, &index.to_string()).unwrap();
    definer(scope, name.into(), desc, args, rv)
}
fn indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    let name = v8::String::new(scope, &index.to_string()).unwrap();
    deleter(scope, name.into(), args, rv)
}
pub(super) fn configure(instance: v8::Local<v8::ObjectTemplate>) {
    instance.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(getter)
            .setter(setter)
            .query(query)
            .descriptor(descriptor)
            .enumerator(enumerator)
            .definer(definer)
            .deleter(deleter),
    );
    instance.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(indexed_getter)
            .setter(indexed_setter)
            .query(indexed_query)
            .descriptor(indexed_descriptor)
            .enumerator(indexed_enumerator)
            .definer(indexed_definer)
            .deleter(indexed_deleter),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, code: &str) {
        let code = v8::String::new(scope, code).unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn native_names_preserve_ascii_hyphen_rules() {
        for (input, expected) in [
            ("data-foo-bar", "fooBar"),
            ("data--foo", "Foo"),
            ("data---foo", "-Foo"),
            ("data-", ""),
            ("data-tail-", "tail-"),
            ("data-foo-é", "foo-é"),
        ] {
            assert_eq!(property(input).as_deref(), Some(expected));
            assert_eq!(attribute(expected), input);
        }
        assert_eq!(property("data-UPPER"), None);
        assert_eq!(property("ordinary"), None);
    }
    #[test]
    fn live_map_names_descriptors_symbols_and_reactions() {
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
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            r#"(()=>{const check=(v,m)=>{if(!v)throw Error(m)};const el=document.createElement('div'),map=el.dataset;
   check(map===el.dataset&&map instanceof DOMStringMap&&Object.prototype.toString.call(map)==='[object DOMStringMap]','brand');
   check(!Object.hasOwn(DOMStringMap.prototype,'undefined')&&!Reflect.ownKeys(DOMStringMap.prototype).includes('undefined'),'anonymous operations have no method');
   check(map.undefined===undefined&&Reflect.get(map,'undefined')===undefined&&!('undefined' in map)&&!Object.hasOwn(map,'undefined'),'absent undefined key');
   map.undefined='native';check(el.getAttribute('data-undefined')==='native'&&map.undefined==='native'&&Object.hasOwn(map,'undefined'),'real undefined key setter');
   el.setAttribute('data-undefined','changed');check(Reflect.get(map,'undefined')==='changed','real undefined key getter');
   check(delete map.undefined,'real undefined key deleter');check(!el.hasAttribute('data-undefined')&&map.undefined===undefined&&!('undefined' in map),'deleted undefined key');
   for(const [name,attribute] of [['fooBar','data-foo-bar'],['-Foo','data---foo'],['','data-'],['tail-','data-tail-'],['foo-é','data-foo-é'],['9','data-9']]){map[name]='\uD800';check(el.getAttribute(attribute)==='\uD800'&&map[name]==='\uD800','write');const d=Object.getOwnPropertyDescriptor(map,name);check(d.value==='\uD800'&&d.writable&&d.enumerable&&d.configurable,'descriptor');el.setAttribute(attribute,'changed');check(map[name]==='changed','live read');delete map[name];check(!el.hasAttribute(attribute)&&!Object.hasOwn(map,name),'delete');}
   let syntax=false;try{map['bad-name']='x'}catch(e){syntax=e.name==='SyntaxError'}check(syntax,'syntax');let invalid=false;try{map['bad name']='x'}catch(e){invalid=e.name==='InvalidCharacterError'}check(invalid,'invalid attribute');
   for(const key of ['x','9']){Object.defineProperty(DOMStringMap.prototype,key,{get(){return 'prototype'},set(){throw Error('prototype setter')},configurable:true});check(map[key]==='prototype','prototype fallback');map[key]='native';check(map[key]==='native'&&el.getAttribute('data-'+key)==='native','override builtins');delete DOMStringMap.prototype[key];delete map[key];}
   const symbol=Symbol();map[symbol]=7;check(map[symbol]===7&&Object.keys(map).length===0,'symbol expando');
   map.a='1';map.b='2';check(Object.keys(map).join()==='a,b','attribute order');Object.defineProperty(map,'c',{value:3});check(el.getAttribute('data-c')==='3','define data');
   globalThis.reactions=[];customElements.define('x-dataset',class extends HTMLElement{static get observedAttributes(){return ['data-value']}attributeChangedCallback(n,o,v){reactions.push(v)}});const custom=document.createElement('x-dataset');custom.dataset.value='one';delete custom.dataset.value;check(reactions.join()==='one,','CE reactions');return true;
  })()"#,
        );
    }
    #[test]
    fn association_survives_cross_arena_adoption_and_conversion_reentry() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let a = {
            let scope = &mut v8::ContextScope::new(scope, first);
            install_generated_bindings(
                scope,
                first,
                Rc::new(RefCell::new(
                    oryn_dom::html::parse("<body></body>")
                        .into_runtime()
                        .unwrap(),
                )),
                RealmId(81),
                1,
            )
            .unwrap()
        };
        let b = {
            let scope = &mut v8::ContextScope::new(scope, second);
            install_generated_bindings(
                scope,
                second,
                Rc::new(RefCell::new(
                    oryn_dom::html::parse("<body></body>")
                        .into_runtime()
                        .unwrap(),
                )),
                RealmId(82),
                1,
            )
            .unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let document = wrap_node(scope, &b, b.document_root()).unwrap();
        let name = v8::String::new(scope, "destination").unwrap();
        first
            .global(scope)
            .create_data_property(scope, name.into(), document.into());
        eval(
            scope,
            r#"(()=>{const check=(v,m)=>{if(!v)throw Error(m)};const el=document.createElement('div');document.body.append(el);const map=el.dataset,proto=Object.getPrototypeOf(map);map.original='kept';destination.body.append(el);check(el.dataset===map&&Object.getPrototypeOf(map)===proto&&map.original==='kept','adopted association');let calls=0;map.convert={toString(){calls++;document.body.append(el);return '\uDC00'}};check(calls===1&&el.ownerDocument===document&&el.dataset===map&&el.getAttribute('data-convert')==='\uDC00','conversion owner');const marker={};try{map.convert={toString(){destination.body.append(el);throw marker}}}catch(e){check(e===marker,'exact exception')}check(map.convert==='\uDC00'&&el.dataset===map,'failed conversion retains value');delete map.original;check(!el.hasAttribute('data-original'),'delete after migration');return true})()"#,
        );
        let _ = a;
    }
    #[test]
    fn inherited_receiver_does_not_write_the_holder_element() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            )),
            RealmId(0),
            1,
        )
        .unwrap();
        eval(
            scope,
            r#"(()=>{const el=document.createElement('div'),map=el.dataset,other=Object.create(map);map.present='original';
   if(!Reflect.set(map,'present','receiver',other)||other.present!=='receiver'||map.present!=='original'||el.getAttribute('data-present')!=='original')throw Error('existing receiver');
   other.fresh='own';other[9]='indexed';if(other.fresh!=='own'||other[9]!=='indexed'||el.hasAttribute('data-fresh')||el.hasAttribute('data-9'))throw Error('new receiver');
   if(!Object.hasOwn(other,'present')||!Object.hasOwn(other,'fresh')||!Object.hasOwn(other,'9'))throw Error('own descriptors');
   delete other.present;if(other.present!=='original'||map.present!=='original')throw Error('receiver delete');return true})()"#,
        );
    }
    #[test]
    fn conversion_termination_aborts_mutation_and_next_entry_after_cancel_succeeds() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            )),
            RealmId(0),
            1,
        )
        .unwrap();
        fn halt(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            scope.terminate_execution();
        }
        let halt = v8::Function::new(scope, halt).unwrap();
        let key = v8::String::new(scope, "halt").unwrap();
        context.global(scope).set(scope, key.into(), halt.into());
        eval(
            scope,
            "globalThis.el=document.createElement('div');globalThis.calls=0;el.dataset.value='before';true",
        );
        {
            v8::tc_scope!(let caught,scope);
            let code = v8::String::new(
                caught,
                "el.dataset.value={toString(){calls++;halt();return 'after'}}",
            )
            .unwrap();
            assert!(
                v8::Script::compile(caught, code, None)
                    .unwrap()
                    .run(caught)
                    .is_none()
            );
            assert!(caught.has_terminated());
            caught.cancel_terminate_execution();
        }
        // This is the first script entry after cancellation, not a priming evaluation.
        eval(
            scope,
            "calls===1&&el.dataset.value==='before'&&el.getAttribute('data-value')==='before'",
        );
        eval(
            scope,
            "el.dataset.value='resumed';calls===1&&el.getAttribute('data-value')==='resumed'",
        );
    }
}
