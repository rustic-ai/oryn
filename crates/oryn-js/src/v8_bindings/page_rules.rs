//! Native CSS rule state; JavaScript objects carry only opaque identity.
use super::*;

pub(super) mod media_lists;
mod tree;

pub(super) struct Record {
    view: BindingRuleView,
    parent: Option<NodeId>,
    parent_rule: Option<u64>,
    owner: NodeId,
    base_url: String,
    quirks: bool,
    source: Rc<str>,
    index: usize,
    detached_path: Option<Vec<usize>>,
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
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    let Some(identity) = id(scope, object) else {
        return Ok(());
    };
    let (source, path) = mutation_context(scope, state, identity)?;
    if let Some(record) = state.page_rules.borrow_mut().get_mut(&identity) {
        record.source = Rc::from(source);
        record.detached_path = Some(path);
        record.parent = None;
        record.parent_rule = None;
    }
    for label in ["oryn.css.rule.sheet", "oryn.css.rule.parent"] {
        let name = v8::String::new(scope, label)
            .ok_or(V8BindingError::Allocation("CSS detached rule key"))?;
        let key = v8::Private::for_api(scope, Some(name));
        object.delete_private(scope, key);
    }
    Ok(())
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    owner: NodeId,
    view: BindingRuleView,
    base_url: String,
    quirks: bool,
    index: usize,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let object = v8::Object::new(scope);
    let interface = match view.rule_type {
        1 => "CSSStyleRule",
        4 => "CSSMediaRule",
        5 => "CSSFontFaceRule",
        6 => "CSSPageRule",
        10 => "CSSNamespaceRule",
        12 => "CSSSupportsRule",
        18 => "CSSContainerRule",
        13 | 26 => "CSSGroupingRule",
        16 => "CSSLayerBlockRule",
        21 => "CSSScopeRule",
        22 => "CSSStartingStyleRule",
        24 => "CSSNestedDeclarations",
        _ => "CSSRule",
    };
    if let Some(template) = state.instance_templates.borrow().get(interface) {
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
    let sheet = associated_wrapper(scope, state, owner, "CSSStyleSheet")?;
    let name = v8::String::new(scope, "oryn.css.rule.sheet")
        .ok_or(V8BindingError::Allocation("CSS rule parent"))?;
    let key = v8::Private::for_api(scope, Some(name));
    object.set_private(scope, key, sheet);
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
            parent_rule: None,
            owner,
            base_url,
            quirks,
            source: stylesheet_ownership::source_snapshot(state, owner)?,
            index,
            detached_path: None,
            _wrapper: wrapper,
        },
    );
    Ok(object)
}

pub(super) fn is_rule_interface(interface: &str) -> bool {
    matches!(
        interface,
        "CSSMediaRule"
            | "CSSStyleRule"
            | "CSSNestedDeclarations"
            | "CSSFontFaceRule"
            | "CSSPageRule"
            | "CSSRule"
            | "CSSNamespaceRule"
            | "CSSGroupingRule"
            | "CSSConditionRule"
            | "CSSSupportsRule"
            | "CSSContainerRule"
            | "CSSLayerBlockRule"
            | "CSSScopeRule"
            | "CSSStartingStyleRule"
    )
}

fn supports_interface(kind: u16, interface: &str) -> bool {
    match interface {
        "CSSRule" => true,
        "CSSStyleRule" => kind == 1,
        "CSSNestedDeclarations" => kind == 24,
        "CSSFontFaceRule" => kind == 5,
        "CSSPageRule" => kind == 6,
        "CSSMediaRule" => kind == 4,
        "CSSNamespaceRule" => kind == 10,
        "CSSGroupingRule" => matches!(kind, 1 | 4 | 6 | 12 | 13 | 16 | 18 | 21 | 22 | 26),
        "CSSConditionRule" => matches!(kind, 4 | 12 | 18),
        "CSSSupportsRule" => kind == 12,
        "CSSContainerRule" => kind == 18,
        "CSSLayerBlockRule" => kind == 16,
        "CSSScopeRule" => kind == 21,
        "CSSStartingStyleRule" => kind == 22,
        _ => false,
    }
}

/// Generated prototype accessors validate the opaque native receiver, never
/// a mutable JavaScript prototype chain.
pub(super) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let invalid = || V8BindingError::Unsupported {
        interface: interface.into(),
        member: "incompatible receiver".into(),
    };
    let identity = id(scope, object).ok_or_else(invalid)?;
    let (view, parent) = {
        let records = state.page_rules.borrow();
        let record = records.get(&identity).ok_or_else(invalid)?;
        if !supports_interface(record.view.rule_type, interface) {
            return Err(invalid());
        }
        (record.view.clone(), tree::stylesheet_owner(state, identity))
    };
    match member {
        "media" => Ok(media_lists::wrap(scope, state, object)?.into()),
        "matches" => Ok(v8::Boolean::new(
            scope,
            view.properties
                .get("matches")
                .is_some_and(|value| value == "true"),
        )
        .into()),
        "conditions" => {
            let mut values = Vec::with_capacity(view.conditions.len());
            for (name, query) in &view.conditions {
                let condition = v8::Object::new(scope);
                for (key, value) in [("name", name), ("query", query)] {
                    let key = v8::String::new(scope, key)
                        .ok_or(V8BindingError::Allocation("CSS condition key"))?;
                    let value = v8_string(scope, value)?;
                    condition
                        .create_data_property(scope, key.into(), value)
                        .ok_or(V8BindingError::JavaScriptException)?;
                }
                values.push(condition.into());
            }
            let array = v8::Array::new_with_elements(scope, &values);
            array
                .set_integrity_level(scope, v8::IntegrityLevel::Frozen)
                .ok_or(V8BindingError::JavaScriptException)?;
            Ok(array.into())
        }
        "style" => Ok(style_object(scope, state, object)?.into()),
        "selectorText" => v8_string(scope, view.selector_text.as_deref().unwrap_or("")),
        "cssRules" => Ok(tree::children(scope, state, object)?.into()),
        "conditionText" | "name" | "containerName" | "containerQuery" => v8_string(
            scope,
            view.properties.get(member).map_or("", String::as_str),
        ),
        "start" | "end" => match view.properties.get(member) {
            Some(value) => v8_string(scope, value),
            None => Ok(v8::null(scope).into()),
        },
        "cssText" => v8_string(scope, &view.css_text),
        "type" => Ok(v8::Integer::new(
            scope,
            if view.rule_type >= 16 {
                0
            } else {
                i32::from(view.rule_type)
            },
        )
        .into()),
        "parentRule" => tree::parent_rule(scope, object),
        "parentStyleSheet" => match parent {
            Some(owner) => associated_wrapper(scope, state, owner, "CSSStyleSheet"),
            None => Ok(v8::null(scope).into()),
        },
        "prefix" | "namespaceURI" if view.rule_type == 10 => v8_string(
            scope,
            view.properties.get(member).map_or("", String::as_str),
        ),
        _ => Err(invalid()),
    }
}

pub(super) fn group_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let identity = id(scope, args.this()).ok_or_else(|| V8BindingError::Unsupported {
        interface: "CSSGroupingRule".into(),
        member: "incompatible receiver".into(),
    })?;
    let kind = state
        .page_rules
        .borrow()
        .get(&identity)
        .map(|record| record.view.rule_type)
        .ok_or(V8BindingError::Allocation("CSS grouping rule record"))?;
    if !supports_interface(kind, "CSSGroupingRule") {
        return Err(V8BindingError::Unsupported {
            interface: "CSSGroupingRule".into(),
            member: "incompatible receiver".into(),
        });
    }
    // WebIDL conversion can execute author code and mutate the same list.
    // Obtain the native context only after all argument conversions finish.
    let text = if member == "insertRule" {
        cssom_string(scope, args.get(0), false)?
    } else {
        String::new()
    };
    let input = if member == "insertRule" {
        args.get(1)
    } else {
        args.get(0)
    };
    let index = if input.is_undefined() {
        0
    } else {
        input
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)? as usize
    };
    let edit = match member {
        "insertRule" => BindingRuleListEdit::Insert { index, text: &text },
        "deleteRule" => BindingRuleListEdit::Delete { index },
        _ => {
            return Err(V8BindingError::Unsupported {
                interface: "CSSGroupingRule".into(),
                member: member.into(),
            });
        }
    };
    tree::edit(scope, state, args.this(), identity, edit)?;
    Ok(if member == "insertRule" {
        v8::Integer::new_from_unsigned(scope, index as u32).into()
    } else {
        v8::undefined(scope).into()
    })
}

fn mutation_context(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    identity: u64,
) -> Result<(String, Vec<usize>), V8BindingError> {
    let (parent, parent_rule, source, index, detached_path) = {
        let records = state.page_rules.borrow();
        let record = records
            .get(&identity)
            .ok_or(V8BindingError::Allocation("CSS rule record"))?;
        (
            record.parent,
            record.parent_rule,
            record.source.to_string(),
            record.index,
            record.detached_path.clone(),
        )
    };
    if let Some(parent_rule) = parent_rule {
        let (source, mut path) = mutation_context(scope, state, parent_rule)?;
        let index = tree::child_index(scope, state, parent_rule, identity)?.unwrap_or(index);
        path.push(index);
        return Ok((source, path));
    }
    if let Some(owner) = parent {
        let cached = state
            .stylesheet_rule_lists
            .borrow()
            .get(&owner)
            .and_then(|array| array.to_local(scope));
        if let Some(cached) = cached {
            let rules = cached;
            for index in 0..rules.length() {
                if let Some(object) = rules
                    .get_index(scope, index)
                    .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
                    && id(scope, object) == Some(identity)
                {
                    return Ok((
                        state.dom.borrow().arena().stylesheet_text(owner)?,
                        vec![index as usize],
                    ));
                }
            }
        }
    }
    Ok((source, detached_path.unwrap_or_else(|| vec![index])))
}

pub(super) fn set_attribute(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
    input: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    // Validate the native subtype before allowing conversion to execute script.
    let identity = id(scope, object).ok_or_else(|| V8BindingError::Unsupported {
        interface: interface.into(),
        member: "incompatible receiver".into(),
    })?;
    let kind = state
        .page_rules
        .borrow()
        .get(&identity)
        .map(|r| r.view.rule_type)
        .ok_or(V8BindingError::Allocation("CSS rule record"))?;
    if !supports_interface(kind, interface) {
        return Err(V8BindingError::Unsupported {
            interface: interface.into(),
            member: "incompatible receiver".into(),
        });
    }
    if member == "media" {
        let list = media_lists::wrap(scope, state, object)?;
        return media_lists::set_text(scope, state, list, input);
    }
    if member == "style" {
        let declaration = style_object(scope, state, object)?;
        return set_declaration_attribute(scope, declaration, "cssText", input);
    }
    let selector = cssom_string(scope, input, false)?;
    let (base_url, quirks) = {
        let records = state.page_rules.borrow();
        let record = records
            .get(&identity)
            .ok_or(V8BindingError::Allocation("CSS rule record"))?;
        (record.base_url.clone(), record.quirks)
    };
    let (source, path) = mutation_context(scope, state, identity)?;
    let provider = state.layout_provider.borrow().clone();
    if let Some(result) = provider.and_then(|provider| {
        if kind == 6 {
            provider.set_page_rule_selector(&source, &path, &selector, &base_url, quirks)
        } else {
            provider.set_style_rule_selector(&source, &path, &selector, &base_url, quirks)
        }
    }) && let Some(rules) = result.map_err(V8BindingError::EventCallback)?
    {
        publish_rule_transaction(
            scope,
            state,
            identity,
            tree::stylesheet_owner(state, identity),
            path,
            rules,
        )?;
    }
    Ok(())
}

fn publish_rule_transaction(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    identity: u64,
    _parent: Option<NodeId>,
    path: Vec<usize>,
    rules: Vec<BindingRuleView>,
) -> Result<(), V8BindingError> {
    let root = tree::root_identity(state, identity)?;
    let index = *path
        .first()
        .ok_or(V8BindingError::Allocation("CSS rule transaction path"))?;
    let root_path = mutation_context(scope, state, root)?.1;
    let view = tree::view_at(&rules, &root_path)
        .cloned()
        .ok_or(V8BindingError::Allocation("CSS rule transaction result"))?;
    let source: Rc<str> = Rc::from(
        rules
            .iter()
            .map(|r| r.css_text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    );
    if let Some(owner) = tree::stylesheet_owner(state, root) {
        publish_stylesheet_source(state, owner, source.to_string())?;
    }
    tree::refresh_subtree(scope, state, root, view, source, index)?;
    Ok(())
}

fn publish_view(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    identity: u64,
    parent: Option<NodeId>,
    view: BindingRuleView,
) -> Result<(), V8BindingError> {
    if let Some(owner) = parent {
        let cached = state
            .stylesheet_rule_lists
            .borrow()
            .get(&owner)
            .and_then(|array| array.to_local(scope));
        if let Some(cached) = cached {
            let rules = cached;
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
    let (text, base_url, quirks, kind) = {
        let records = state.page_rules.borrow();
        let record = records
            .get(&identity)
            .ok_or(V8BindingError::Allocation("page declaration record"))?;
        (
            record.view.declaration_text.clone().unwrap_or_default(),
            record.base_url.clone(),
            record.quirks,
            record.view.rule_type,
        )
    };
    let provider = state
        .layout_provider
        .borrow()
        .clone()
        .ok_or(V8BindingError::Allocation("page declaration provider"))?;
    let request = BindingDeclarationRequest {
        css_text: &text,
        property,
        value: "",
        priority: "",
        base_url: &base_url,
        quirks,
    };
    let result = if matches!(kind, 1 | 24) {
        provider.read_declaration(&request)
    } else if kind == 5 {
        provider.read_font_face_declaration(&request)
    } else {
        provider.read_page_declaration(&request)
    };
    result
        .ok_or(V8BindingError::Allocation("rule declaration read"))?
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
    let kind = state
        .page_rules
        .borrow()
        .get(&identity)
        .map(|r| r.view.rule_type);
    if matches!(kind, Some(1 | 5 | 6 | 24)) {
        let (source, path) = mutation_context(scope, &state, identity)?;
        let mutation = provider
            .mutate_rule_declaration(
                &BindingDeclarationRequest {
                    css_text: &source,
                    property,
                    value,
                    priority,
                    base_url: &base_url,
                    quirks,
                },
                &path,
                replace,
            )
            .ok_or(V8BindingError::Allocation("CSS declaration mutation"))?
            .map_err(V8BindingError::EventCallback)?;
        if let Some(rules) = mutation.rules {
            publish_rule_transaction(scope, &state, identity, parent, path, rules)?;
        }
        return Ok(mutation.previous_value);
    }
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

pub(super) fn is_declaration(scope: &mut v8::PinScope, object: v8::Local<v8::Object>) -> bool {
    !object.is_proxy() && declaration_rule(scope, object).is_ok()
}

pub(super) fn validate_declaration_interface(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    interface: &str,
) -> Result<(), V8BindingError> {
    let rule = declaration_rule(scope, object)?;
    let identity = id(scope, rule).ok_or(V8BindingError::Allocation("CSS declaration identity"))?;
    let kind = state
        .page_rules
        .borrow()
        .get(&identity)
        .map(|record| record.view.rule_type);
    if matches!(
        (interface, kind),
        ("CSSStyleProperties", Some(1 | 24))
            | ("CSSPageDescriptors", Some(6))
            | ("CSSFontFaceDescriptors", Some(5))
    ) {
        Ok(())
    } else {
        Err(V8BindingError::Unsupported {
            interface: interface.into(),
            member: "incompatible receiver".into(),
        })
    }
}

pub(super) fn declaration_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if member == "parentRule" {
        return Ok(declaration_rule(scope, object)?.into());
    }
    let property = if matches!(member, "cssText" | "length") {
        ""
    } else {
        css_property_name(member)
    };
    let view = read_style(scope, object, property)?;
    match member {
        "cssText" => v8_string(scope, &view.css_text),
        "length" => Ok(v8::Integer::new_from_unsigned(scope, view.names.len() as u32).into()),
        _ => v8_string(scope, &view.value),
    }
}

pub(super) fn set_declaration_attribute(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    member: &str,
    input: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let replace = member == "cssText";
    let value = cssom_string(scope, input, !replace)?;
    mutate_style(
        scope,
        object,
        css_property_name(member),
        &value,
        "",
        replace,
    )?;
    Ok(())
}

pub(super) fn declaration_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    method: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
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
    match method {
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
}

fn style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    rule: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if let Ok(style) = private_object(scope, rule, "oryn.page.declaration") {
        return Ok(style);
    }
    let identity = id(scope, rule).ok_or(V8BindingError::Allocation("CSS declaration rule"))?;
    let kind = state
        .page_rules
        .borrow()
        .get(&identity)
        .map(|record| record.view.rule_type)
        .ok_or(V8BindingError::Allocation("CSS declaration rule record"))?;
    let interface = if matches!(kind, 1 | 24) {
        "CSSStyleProperties"
    } else if kind == 5 {
        "CSSFontFaceDescriptors"
    } else if kind == 6 {
        "CSSPageDescriptors"
    } else {
        "CSSStyleDeclaration"
    };
    let template = state
        .instance_templates
        .borrow()
        .get(interface)
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
