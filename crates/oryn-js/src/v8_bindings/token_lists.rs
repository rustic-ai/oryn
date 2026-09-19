//! Shared live token-list bindings for class, rel and blocking attributes.
use super::*;

fn tokens(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<Vec<String>, V8BindingError> {
    let state = binding_state_for_receiver(scope, object)
        .ok_or(V8BindingError::Allocation("DOMTokenList owner"))?;
    let node = resolve_receiver(scope, &state, object, "DOMTokenList")?;
    let _ = node;
    values(scope, object)
}

fn getter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match tokens(scope, args.holder()) {
        Ok(values) => match values.get(index as usize) {
            Some(value) => {
                match v8_string(scope, value) {
                    Ok(value) => rv.set(value),
                    Err(error) => throw_binding_error(scope, &error),
                }
                v8::Intercepted::kYes
            }
            None => v8::Intercepted::kNo,
        },
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}

fn query(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    match tokens(scope, args.holder()) {
        Ok(values) if (index as usize) < values.len() => {
            rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
            v8::Intercepted::kYes
        }
        Ok(_) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}

fn enumerator(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Array>,
) {
    match tokens(scope, args.holder()) {
        Ok(values) => {
            let indices = (0..values.len())
                .map(|i| v8::Integer::new_from_unsigned(scope, i as u32).into())
                .collect::<Vec<_>>();
            rv.set(v8::Array::new_with_elements(scope, &indices));
        }
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn reject_write(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    rv.set_bool(false);
    if args.should_throw_on_error() {
        throw_type_error(scope, "DOMTokenList indexed properties are read-only");
    }
    v8::Intercepted::kYes
}
fn setter(
    scope: &mut v8::PinScope,
    _: u32,
    _: v8::Local<v8::Value>,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    reject_write(scope, args, rv)
}
fn definer(
    scope: &mut v8::PinScope,
    _: u32,
    _: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    reject_write(scope, args, rv)
}
fn deleter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    match tokens(scope, args.holder()) {
        Ok(values) => {
            let allowed = index as usize >= values.len();
            rv.set_bool(allowed);
            if !allowed && args.should_throw_on_error() {
                throw_type_error(scope, "DOMTokenList indexed properties cannot be deleted");
            }
        }
        Err(error) => throw_binding_error(scope, &error),
    }
    v8::Intercepted::kYes
}

pub(super) fn configure(instance: v8::Local<v8::ObjectTemplate>) {
    instance.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(getter)
            .query(query)
            .enumerator(enumerator)
            .setter(setter)
            .definer(definer)
            .deleter(deleter),
    );
}

fn attribute_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let key = v8::String::new(scope, "oryn.token-list.attribute")
        .ok_or(V8BindingError::Allocation("token list attribute"))?;
    Ok(v8::Private::for_api(scope, Some(key)))
}
pub(super) fn mark_attribute(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    attribute: &str,
) -> Result<(), V8BindingError> {
    let key = attribute_key(scope)?;
    let value = v8_string(scope, attribute)?;
    if object.set_private(scope, key, value) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(())
}
fn attribute(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<&'static str, V8BindingError> {
    let key = attribute_key(scope)?;
    Ok(
        match object
            .get_private(scope, key)
            .filter(|value| value.is_string())
            .map(|value| value.to_rust_string_lossy(scope))
            .as_deref()
        {
            Some("rel") => "rel",
            Some("for") => "for",
            Some("blocking") => "blocking",
            _ => "class",
        },
    )
}
pub(super) fn values(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<Vec<String>, V8BindingError> {
    let attribute = attribute(scope, object)?;
    let owner = binding_state_for_receiver(scope, object)
        .ok_or(V8BindingError::Allocation("token list owner"))?;
    let node = resolve_receiver(scope, &owner, object, "DOMTokenList")?;
    let dom = owner.dom.borrow();
    let units = oryn_dom::dom_string::units(
        dom.attribute_in_null_namespace(node, attribute)?
            .unwrap_or(""),
    );
    let mut seen = BTreeSet::new();
    Ok(units
        .split(|unit| matches!(unit, 9 | 10 | 12 | 13 | 32))
        .filter(|token| !token.is_empty())
        .map(oryn_dom::dom_string::from_units)
        .filter(|token| seen.insert(token.clone()))
        .collect())
}
pub(super) fn associated_get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let owner = binding_state_for_receiver(scope, object)
        .ok_or(V8BindingError::Allocation("token list owner"))?;
    let node = resolve_receiver(scope, &owner, object, "DOMTokenList")?;
    if member == "length" {
        let length = values(scope, object)?.len();
        return Ok(v8::Integer::new_from_unsigned(scope, length as u32).into());
    }
    let attribute = attribute(scope, object)?;
    let text = owner
        .dom
        .borrow()
        .attribute_in_null_namespace(node, attribute)?
        .unwrap_or("")
        .to_owned();
    v8_string(scope, &text)
}
pub(super) fn associated_set(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let value = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let value = rust_string_preserving_utf16(scope, value)?;
    store(scope, object, &value)
}
fn store(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    value: &str,
) -> Result<(), V8BindingError> {
    let owner = binding_state_for_receiver(scope, object)
        .ok_or(V8BindingError::Allocation("token list owner"))?;
    let node = resolve_receiver(scope, &owner, object, "DOMTokenList")?;
    let attribute = attribute(scope, object)?;
    owner
        .dom
        .borrow_mut()
        .set_attribute_ns(node, None, attribute, value.to_owned())?;
    if attribute == "rel" {
        prepare_native_module_preloads(scope, &owner, node)?;
        prepare_dynamic_stylesheet_subtree(&owner, node)?;
    }
    Ok(())
}
fn validate_token(token: &str) -> Result<(), V8BindingError> {
    let units = oryn_dom::dom_string::units(token);
    let member = if units.is_empty() {
        Some("SyntaxError")
    } else if units
        .iter()
        .any(|unit| matches!(unit, 9 | 10 | 12 | 13 | 32))
    {
        Some("InvalidCharacterError")
    } else {
        None
    };
    match member {
        Some(member) => Err(V8BindingError::Unsupported {
            interface: "DOMTokenList".into(),
            member: member.into(),
        }),
        None => Ok(()),
    }
}
fn serialize_tokens(tokens: &[String]) -> String {
    let mut units = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if index > 0 {
            units.push(32)
        }
        units.extend(oryn_dom::dom_string::units(token));
    }
    oryn_dom::dom_string::from_units(&units)
}
pub(super) fn associated_call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    if member == "toString" {
        return associated_get(scope, object, "value").map(Some);
    }
    if member == "item" {
        let index = args
            .get(0)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)? as usize;
        let values = values(scope, object)?;
        return Ok(Some(match values.get(index) {
            Some(value) => v8_string(scope, value)?,
            None => v8::null(scope).into(),
        }));
    }
    let tokens = match member {
        "add" | "remove" => (0..args.length())
            .map(|index| required_string(scope, args, index))
            .collect::<Result<Vec<_>, _>>()?,
        "replace" => vec![
            required_string(scope, args, 0)?,
            required_string(scope, args, 1)?,
        ],
        _ => vec![required_string(scope, args, 0)?],
    };
    if member == "supports" {
        if matches!(attribute(scope, object)?, "class" | "for") {
            return Err(V8BindingError::Unsupported {
                interface: "DOMTokenList".into(),
                member: "TypeError: attribute has no supported tokens".into(),
            });
        }
        let supported = if attribute(scope, object)? == "blocking" {
            tokens[0] == "render"
        } else {
            let owner = binding_state_for_receiver(scope, object)
                .ok_or(V8BindingError::Allocation("token list owner"))?;
            let node = resolve_receiver(scope, &owner, object, "DOMTokenList")?;
            let dom = owner.dom.borrow();
            let token = tokens[0].to_ascii_lowercase();
            match element_local_name(&dom, node) {
                Some("a" | "area" | "form") => {
                    matches!(token.as_str(), "noopener" | "noreferrer" | "opener")
                }
                Some("link") => matches!(
                    token.as_str(),
                    "alternate"
                        | "canonical"
                        | "dns-prefetch"
                        | "expect"
                        | "icon"
                        | "manifest"
                        | "modulepreload"
                        | "next"
                        | "preconnect"
                        | "prefetch"
                        | "preload"
                        | "prerender"
                        | "stylesheet"
                        | "apple-touch-icon"
                        | "apple-touch-icon-precomposed"
                ),
                _ => false,
            }
        };
        return Ok(Some(v8::Boolean::new(scope, supported).into()));
    }
    if member == "contains" {
        let found = values(scope, object)?.contains(&tokens[0]);
        return Ok(Some(v8::Boolean::new(scope, found).into()));
    }
    if member == "replace" && tokens.iter().any(String::is_empty) {
        validate_token("")?
    }
    for token in &tokens {
        validate_token(token)?
    }
    let mut current = values(scope, object)?;
    let mut boolean = None;
    match member {
        "add" => {
            for token in tokens {
                if !current.contains(&token) {
                    current.push(token)
                }
            }
        }
        "remove" => current.retain(|token| !tokens.contains(token)),
        "replace" => {
            let Some(index) = current.iter().position(|token| token == &tokens[0]) else {
                return Ok(Some(v8::Boolean::new(scope, false).into()));
            };
            current[index] = tokens[1].clone();
            let mut seen = BTreeSet::new();
            current.retain(|token| seen.insert(token.clone()));
            boolean = Some(true)
        }
        "toggle" => {
            let present = current.contains(&tokens[0]);
            let next = if args.length() > 1 && !args.get(1).is_undefined() {
                args.get(1).boolean_value(scope)
            } else {
                !present
            };
            if present == next {
                return Ok(Some(v8::Boolean::new(scope, next).into()));
            }
            if next {
                current.push(tokens[0].clone())
            } else {
                current.retain(|token| token != &tokens[0])
            }
            boolean = Some(next)
        }
        _ => {
            return Err(V8BindingError::Unsupported {
                interface: "DOMTokenList".into(),
                member: member.into(),
            });
        }
    }
    // Token-set update steps preserve an absent attribute when the set is empty.
    // Direct value assignment still creates an empty attribute through store().
    let absent_empty = if current.is_empty() {
        let owner = binding_state_for_receiver(scope, object)
            .ok_or(V8BindingError::Allocation("token list owner"))?;
        let node = resolve_receiver(scope, &owner, object, "DOMTokenList")?;
        let attribute = attribute(scope, object)?;
        let absent = owner
            .dom
            .borrow()
            .attribute_in_null_namespace(node, attribute)?
            .is_none();
        absent
    } else {
        false
    };
    if !absent_empty {
        store(scope, object, &serialize_tokens(&current))?;
    }
    Ok(boolean.map(|value| v8::Boolean::new(scope, value).into()))
}

#[cfg(test)]
mod blocking_tests {
    use super::*;
    #[test]
    fn blocking_lists_are_distinct_live_native_associations() {
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
        let source=v8::String::new(scope,r#"for(const tag of ['script','style','link']){
   const node=document.createElement(tag),list=node.blocking;
   if(!(list instanceof DOMTokenList)||list!==node.blocking)throw Error('blocking brand/identity');
   node.blocking='render unknown render';if(list.length!==2||list[0]!=='render'||!list.supports('render')||list.supports('RENDER')||list.supports('unknown'))throw Error('blocking tokens');
   list.remove('render');if(node.getAttribute('blocking')!=='unknown')throw Error('native update');
   node.setAttribute('blocking','render');if(list.value!=='render'||!list.contains('render'))throw Error('live reflection');
   node.classList.add('different');if(list.contains('different'))throw Error('association collision');
   if(tag==='link'){node.relList.add('stylesheet');if(list.contains('stylesheet')||!node.relList.contains('stylesheet'))throw Error('rel collision');}
   node.blocking='one\vtwo';if(list.length!==1)throw Error('non-HTML whitespace');
  }true"#).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}
