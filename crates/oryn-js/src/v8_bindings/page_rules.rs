//! Native page-rule state; JavaScript objects carry only opaque identity.
use super::*;

pub(super) struct Record {
    view: BindingRuleView,
    parent: Option<NodeId>,
    base_url: String,
    quirks: bool,
    _wrapper: v8::Weak<v8::Object>,
}

fn id(scope: &mut v8::PinScope, object: v8::Local<v8::Object>) -> Option<u64> {
    if object.is_proxy() {
        return None;
    }
    let name = v8::String::new(scope, "oryn.page.rule")?;
    let key = v8::Private::for_api(scope, Some(name));
    let value = object.get_private(scope, key)?;
    Some(v8::Local::<v8::BigInt>::try_from(value).ok()?.u64_value().0)
}

pub(super) fn detach(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) {
    if let Some(id) = id(scope, object)
        && let Some(record) = state.page_rules.borrow_mut().get_mut(&id)
    {
        record.parent = None;
    }
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    owner: NodeId,
    view: BindingRuleView,
    base_url: String,
    quirks: bool,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let object = v8::Object::new(scope);
    if let Some(template) = state.instance_templates.borrow().get("CSSPageRule") {
        let template = v8::Local::new(scope, template);
        if let Some(instance) = template.new_instance(scope)
            && let Some(prototype) = instance.get_prototype(scope)
        {
            object.set_prototype(scope, prototype);
        }
    }
    let identity = state.next_page_rule_id.get();
    state.next_page_rule_id.set(identity + 1);
    let name = v8::String::new(scope, "oryn.page.rule")
        .ok_or(V8BindingError::Allocation("page rule identity"))?;
    let key = v8::Private::for_api(scope, Some(name));
    object.set_private(scope, key, v8::BigInt::new_from_u64(scope, identity).into());
    let node = wrap_node(scope, state, owner)?;
    let name = v8::String::new(scope, "oryn.associated.owner")
        .ok_or(V8BindingError::Allocation("page rule owner"))?;
    let key = v8::Private::for_api(scope, Some(name));
    object.set_private(scope, key, node.into());
    for name in [
        "cssText",
        "type",
        "selectorText",
        "parentStyleSheet",
        "parentRule",
        "style",
    ] {
        let key =
            v8::String::new(scope, name).ok_or(V8BindingError::Allocation("page property"))?;
        let get = v8::Function::builder(getter)
            .data(key.into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("page getter"))?;
        let set: v8::Local<v8::Value> = if name == "style" {
            v8::Function::builder(forward_style)
                .build(scope)
                .ok_or(V8BindingError::Allocation("page style setter"))?
                .into()
        } else if name == "selectorText" {
            v8::Function::builder(setter)
                .build(scope)
                .ok_or(V8BindingError::Allocation("page setter"))?
                .into()
        } else {
            v8::undefined(scope).into()
        };
        let mut descriptor = v8::PropertyDescriptor::new_from_get_set(get.into(), set);
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        object.define_property(scope, key.into(), &descriptor);
    }
    let weak_state = Rc::downgrade(state);
    let wrapper = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_| {
            if let Some(state) = weak_state.upgrade() {
                state.page_rules.borrow_mut().remove(&identity);
            }
        }),
    );
    state.page_rules.borrow_mut().insert(
        identity,
        Record {
            view,
            parent: Some(owner),
            base_url,
            quirks,
            _wrapper: wrapper,
        },
    );
    Ok(object)
}

fn getter(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| {
        let state = binding_state_for_receiver(scope, args.this())
            .ok_or(V8BindingError::Allocation("page rule state"))?;
        let identity =
            id(scope, args.this()).ok_or(V8BindingError::Allocation("page rule receiver"))?;
        let (view, parent) = {
            let records = state.page_rules.borrow();
            let record = records
                .get(&identity)
                .ok_or(V8BindingError::Allocation("page rule record"))?;
            (record.view.clone(), record.parent)
        };
        let name = args.data().to_rust_string_lossy(scope);
        match name.as_str() {
            "style" => Ok(style_object(scope, &state, args.this())?.into()),
            "cssText" => v8_string(scope, &view.css_text),
            "selectorText" => v8_string(scope, view.selector_text.as_deref().unwrap_or("")),
            "type" => Ok(v8::Integer::new(scope, 6).into()),
            "parentStyleSheet" => match parent {
                Some(owner) => associated_wrapper(scope, &state, owner, "CSSStyleSheet"),
                None => Ok(v8::null(scope).into()),
            },
            _ => Ok(v8::null(scope).into()),
        }
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn setter(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let result = (|| {
        let selector = cssom_string(scope, args.get(0), false)?;
        let state = binding_state_for_receiver(scope, args.this())
            .ok_or(V8BindingError::Allocation("page rule state"))?;
        let identity =
            id(scope, args.this()).ok_or(V8BindingError::Allocation("page rule receiver"))?;
        let (source, parent, base_url, quirks) = {
            let records = state.page_rules.borrow();
            let record = records
                .get(&identity)
                .ok_or(V8BindingError::Allocation("page rule record"))?;
            (
                record.view.css_text.clone(),
                record.parent,
                record.base_url.clone(),
                record.quirks,
            )
        };
        let provider = state.layout_provider.borrow().clone();
        let Some(result) =
            provider.and_then(|p| p.set_page_selector(&source, &selector, &base_url, quirks))
        else {
            return Ok(());
        };
        let Some(view) = result.map_err(V8BindingError::EventCallback)? else {
            return Ok(());
        };
        publish_view(scope, &state, identity, parent, view)?;
        Ok(())
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}

fn publish_view(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    identity: u64,
    parent: Option<NodeId>,
    view: BindingRuleView,
) -> Result<(), V8BindingError> {
    if let Some(owner) = parent {
        let cached = state.stylesheet_rule_lists.borrow().get(&owner).cloned();
        if let Some(cached) = cached {
            let rules = v8::Local::new(scope, cached);
            let source = state.dom.borrow().arena().stylesheet_text(owner)?;
            let mut texts = stylesheet_rules_for_owner(scope, state, owner, &source, true)?;
            for index in 0..rules.length() {
                let object = rules
                    .get_index(scope, index)
                    .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
                    .ok_or(V8BindingError::Allocation("page sibling rule"))?;
                if id(scope, object) == Some(identity) {
                    if let Some(text) = texts.get_mut(index as usize) {
                        *text = view.css_text.clone();
                    }
                    break;
                }
            }
            publish_stylesheet_source(state, owner, texts.join("\n"))?;
        }
    }
    if let Some(record) = state.page_rules.borrow_mut().get_mut(&identity) {
        record.view = view;
    }
    Ok(())
}

fn private_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    key: &str,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let name =
        v8::String::new(scope, key).ok_or(V8BindingError::Allocation("page declaration key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    object
        .get_private(scope, key)
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
        .ok_or(V8BindingError::Allocation("page declaration receiver"))
}

fn declaration_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    private_object(scope, object, "oryn.page.declaration.rule")
}

fn read_style(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    property: &str,
) -> Result<BindingDeclarationView, V8BindingError> {
    let rule = declaration_rule(scope, object)?;
    let state = binding_state_for_receiver(scope, rule)
        .ok_or(V8BindingError::Allocation("page declaration state"))?;
    let identity =
        id(scope, rule).ok_or(V8BindingError::Allocation("page declaration identity"))?;
    let (text, base_url, quirks) = {
        let records = state.page_rules.borrow();
        let record = records
            .get(&identity)
            .ok_or(V8BindingError::Allocation("page declaration record"))?;
        (
            record.view.declaration_text.clone().unwrap_or_default(),
            record.base_url.clone(),
            record.quirks,
        )
    };
    let provider = state
        .layout_provider
        .borrow()
        .clone()
        .ok_or(V8BindingError::Allocation("page declaration provider"))?;
    provider
        .read_page_declaration(&BindingDeclarationRequest {
            css_text: &text,
            property,
            value: "",
            priority: "",
            base_url: &base_url,
            quirks,
        })
        .ok_or(V8BindingError::Allocation("page declaration read"))?
        .map_err(V8BindingError::EventCallback)
}

fn mutate_style(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    property: &str,
    value: &str,
    priority: &str,
    replace: bool,
) -> Result<String, V8BindingError> {
    let rule = declaration_rule(scope, object)?;
    let state = binding_state_for_receiver(scope, rule)
        .ok_or(V8BindingError::Allocation("page declaration state"))?;
    let identity =
        id(scope, rule).ok_or(V8BindingError::Allocation("page declaration identity"))?;
    let (source, parent, base_url, quirks) = {
        let records = state.page_rules.borrow();
        let record = records
            .get(&identity)
            .ok_or(V8BindingError::Allocation("page declaration record"))?;
        (
            record.view.css_text.clone(),
            record.parent,
            record.base_url.clone(),
            record.quirks,
        )
    };
    let provider = state
        .layout_provider
        .borrow()
        .clone()
        .ok_or(V8BindingError::Allocation("page declaration provider"))?;
    let mutation = provider
        .mutate_page_declaration(
            &BindingDeclarationRequest {
                css_text: &source,
                property,
                value,
                priority,
                base_url: &base_url,
                quirks,
            },
            replace,
        )
        .ok_or(V8BindingError::Allocation("page declaration mutation"))?
        .map_err(V8BindingError::EventCallback)?;
    if let Some(view) = mutation.rule {
        publish_view(scope, &state, identity, parent, view)?;
    }
    Ok(mutation.previous_value)
}

fn style_getter(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| {
        let name = args.data().to_rust_string_lossy(scope);
        if name == "@parentRule" {
            return Ok(declaration_rule(scope, args.this())?.into());
        }
        let view = read_style(
            scope,
            args.this(),
            if name.starts_with('@') { "" } else { &name },
        )?;
        match name.as_str() {
            "@cssText" => v8_string(scope, &view.css_text),
            "@length" => Ok(v8::Integer::new_from_unsigned(scope, view.names.len() as u32).into()),
            _ => v8_string(scope, &view.value),
        }
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn style_setter(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let result = (|| {
        let property = args.data().to_rust_string_lossy(scope);
        let replace = property == "@cssText";
        let value = cssom_string(scope, args.get(0), !replace)?;
        mutate_style(scope, args.this(), &property, &value, "", replace)?;
        Ok(())
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}

fn style_method(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let name = args.data().to_rust_string_lossy(scope);
    let required = if name == "setProperty" { 2 } else { 1 };
    if args.length() < required {
        throw_type_error(
            scope,
            "CSSStyleDeclaration method called with too few arguments",
        );
        return;
    }
    let result = (|| {
        let method = args.data().to_rust_string_lossy(scope);
        if method == "item" {
            let index = args
                .get(0)
                .uint32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            let view = read_style(scope, args.this(), "")?;
            return v8_string(
                scope,
                view.names.get(index as usize).map_or("", String::as_str),
            );
        }
        let property = cssom_string(scope, args.get(0), false)?;
        match method.as_str() {
            "getPropertyValue" => {
                let view = read_style(scope, args.this(), &property)?;
                v8_string(scope, &view.value)
            }
            "getPropertyPriority" => {
                let view = read_style(scope, args.this(), &property)?;
                v8_string(scope, if view.important { "important" } else { "" })
            }
            "removeProperty" => {
                let previous = mutate_style(scope, args.this(), &property, "", "", false)?;
                v8_string(scope, &previous)
            }
            _ => {
                let value = cssom_string(scope, args.get(1), true)?;
                let priority = if args.length() < 3 || args.get(2).is_undefined() {
                    String::new()
                } else {
                    cssom_string(scope, args.get(2), true)?
                };
                mutate_style(scope, args.this(), &property, &value, &priority, false)?;
                Ok(v8::undefined(scope).into())
            }
        }
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    rule: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if let Ok(style) = private_object(scope, rule, "oryn.page.declaration") {
        return Ok(style);
    }
    let template = state
        .instance_templates
        .borrow()
        .get("CSSStyleDeclaration")
        .cloned()
        .ok_or(V8BindingError::MissingInterface("CSSStyleDeclaration"))?;
    let template = v8::Local::new(scope, template);
    let style = template
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("page declaration instance"))?;
    let key = v8::String::new(scope, "oryn.page.declaration.rule")
        .ok_or(V8BindingError::Allocation("declaration owner"))?;
    let key = v8::Private::for_api(scope, Some(key));
    style.set_private(scope, key, rule.into());
    let provider = state
        .layout_provider
        .borrow()
        .clone()
        .ok_or(V8BindingError::Allocation("declaration provider"))?;
    let mut names = BTreeMap::new();
    for (name, canonical) in provider.supported_css_properties() {
        for attribute in inline_styles::attribute_names(&name) {
            names.insert(attribute, canonical.clone());
        }
    }
    for name in ["cssText", "length", "parentRule"] {
        names.insert(name.into(), format!("@{name}"));
    }
    for (name, property) in names {
        let key = v8::String::new(scope, &name)
            .ok_or(V8BindingError::Allocation("declaration property"))?;
        let data = v8::String::new(scope, &property)
            .ok_or(V8BindingError::Allocation("declaration property data"))?;
        let get = v8::Function::builder(style_getter)
            .data(data.into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("declaration getter"))?;
        let set: v8::Local<v8::Value> = if property == "@length" || property == "@parentRule" {
            v8::undefined(scope).into()
        } else {
            v8::Function::builder(style_setter)
                .data(data.into())
                .build(scope)
                .ok_or(V8BindingError::Allocation("declaration setter"))?
                .into()
        };
        let mut descriptor = v8::PropertyDescriptor::new_from_get_set(get.into(), set);
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        style.define_property(scope, key.into(), &descriptor);
    }
    for name in [
        "item",
        "getPropertyValue",
        "getPropertyPriority",
        "setProperty",
        "removeProperty",
    ] {
        let data = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("declaration method name"))?;
        let method = v8::Function::builder(style_method)
            .data(data.into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("declaration method"))?;
        set_object_property(scope, style, name, method.into())?;
    }
    let key = v8::String::new(scope, "oryn.page.declaration")
        .ok_or(V8BindingError::Allocation("declaration cache"))?;
    let key = v8::Private::for_api(scope, Some(key));
    rule.set_private(scope, key, style.into());
    Ok(style)
}

/// Let the shared CSSStyleDeclaration index handlers read this native block.
pub(super) fn declaration_names(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<Option<Vec<String>>, V8BindingError> {
    if object.is_proxy() || declaration_rule(scope, object).is_err() {
        return Ok(None);
    }
    Ok(Some(read_style(scope, object, "")?.names))
}

fn forward_style(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let result = (|| {
        let value = cssom_string(scope, args.get(0), false)?;
        let state = binding_state_for_receiver(scope, args.this())
            .ok_or(V8BindingError::Allocation("page rule state"))?;
        let style = style_object(scope, &state, args.this())?;
        mutate_style(scope, style, "", &value, "", true)?;
        Ok(())
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}
