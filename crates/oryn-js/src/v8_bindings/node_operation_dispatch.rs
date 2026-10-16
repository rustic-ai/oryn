//! Each operation has an independent native frame, including in debug builds.
//! The parent dispatcher retains routing order and shares no operation locals.
use super::*;

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlslotelement_assign<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let mut nodes = Vec::new();
        for index in 0..args.length() {
            let child = argument_node(scope, state, args.get(index))?;
            if !state.dom.borrow().is_slottable(child)? {
                return Err(V8BindingError::Unsupported {
                    interface: "HTMLSlotElement".into(),
                    member: "TypeError".into(),
                });
            }
            nodes.push(child);
        }
        state.dom.borrow_mut().assign_slot_nodes(node, &nodes)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlslotelement_assignednodes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let options = args.get(0);
        let flatten = if options.is_null_or_undefined() {
            false
        } else {
            if !options.is_object() {
                return Err(V8BindingError::Unsupported {
                    interface: "HTMLSlotElement".into(),
                    member: "TypeError".into(),
                });
            }
            let options = options
                .to_object(scope)
                .ok_or(V8BindingError::Allocation("slot options"))?;
            let key = v8::String::new(scope, "flatten")
                .ok_or(V8BindingError::Allocation("slot option name"))?;
            options
                .get(scope, key.into())
                .ok_or(V8BindingError::JavaScriptException)?
                .boolean_value(scope)
        };
        let nodes = {
            let dom = state.dom.borrow();
            let mut nodes = dom.assigned_nodes(node, flatten)?;
            if member == "assignedElements" {
                nodes.retain(|node| {
                    dom.arena()
                        .get(*node)
                        .is_ok_and(|record| matches!(record.kind, NodeKind::Element { .. }))
                });
            }
            nodes
        };
        let array = v8::Array::new(scope, nodes.len() as i32);
        for (index, node) in nodes.into_iter().enumerate() {
            let wrapper = wrap_node(scope, state, node)?;
            array.set_index(scope, index as u32, wrapper.into());
        }
        Ok(Some(array.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_node_lookupnamespaceuri<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let value = if args.get(0).is_null_or_undefined() {
            None
        } else {
            Some(required_string(scope, args, 0)?)
        };
        if member == "isDefaultNamespace" {
            return Ok(Some(
                v8::Boolean::new(
                    scope,
                    state
                        .dom
                        .borrow()
                        .is_default_namespace(node, value.as_deref())?,
                )
                .into(),
            ));
        }
        let value = if member == "lookupPrefix" {
            state.dom.borrow().lookup_prefix(node, value.as_deref())?
        } else {
            state
                .dom
                .borrow()
                .lookup_namespace_uri(node, value.as_deref())?
        };
        Ok(Some(match value {
            Some(value) => v8_string(scope, &value)?,
            None => v8::null(scope).into(),
        }))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_node_normalize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        script_children_changed::normalize(scope, state, node)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_characterdata_substringdata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let offset = args
            .get(0)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let count = args
            .get(1)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let dom = state.dom.borrow();
        let data = match &dom.arena().get(node)?.kind {
            NodeKind::Text { data }
            | NodeKind::CDataSection { data }
            | NodeKind::ProcessingInstruction { data, .. }
            | NodeKind::Comment { data } => data,
            _ => return Err(DomError::StaleNode(node).into()),
        };
        if offset as usize > oryn_dom::dom_string::len(data) {
            return Err(V8BindingError::Unsupported {
                interface: "CharacterData".into(),
                member: "IndexSizeError".into(),
            });
        }
        Ok(Some(v8_string(
            scope,
            &utf16_slice(data, offset, offset.saturating_add(count)),
        )?))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_characterdata_appenddata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let (offset, count, replacement) = match member {
            "appendData" => (0, 0, required_string(scope, args, 0)?),
            "insertData" => (
                args.get(0)
                    .uint32_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?,
                0,
                required_string(scope, args, 1)?,
            ),
            "deleteData" => (
                args.get(0)
                    .uint32_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?,
                args.get(1)
                    .uint32_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?,
                String::new(),
            ),
            _ => (
                args.get(0)
                    .uint32_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?,
                args.get(1)
                    .uint32_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?,
                required_string(scope, args, 2)?,
            ),
        };
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        // WebIDL coercion may run script and mutate this CharacterData.
        // Read the current native length only after every argument converts.
        let length = {
            let dom = state.dom.borrow();
            match &dom.arena().get(node)?.kind {
                NodeKind::Text { data }
                | NodeKind::CDataSection { data }
                | NodeKind::ProcessingInstruction { data, .. }
                | NodeKind::Comment { data } => oryn_dom::dom_string::len(data) as u32,
                _ => return Err(DomError::StaleNode(node).into()),
            }
        };
        let offset = if member == "appendData" {
            length
        } else {
            offset
        };
        if offset > length {
            return Err(V8BindingError::Unsupported {
                interface: "CharacterData".into(),
                member: "IndexSizeError".into(),
            });
        }
        let preparation = script_children_changed::character_data(scope, state, node)?;
        let result =
            state
                .dom
                .borrow_mut()
                .replace_character_data(node, offset, count, replacement);
        preparation.finish_mutation(scope, result)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_text_splittext<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let offset = args
            .get(0)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let length = {
            let dom = state.dom.borrow();
            match &dom.arena().get(node)?.kind {
                NodeKind::Text { data } | NodeKind::CDataSection { data } => {
                    oryn_dom::dom_string::len(data)
                }
                _ => return Err(DomError::StaleNode(node).into()),
            }
        };
        if offset as usize > length {
            return Err(V8BindingError::Unsupported {
                interface: "Text".into(),
                member: "IndexSizeError".into(),
            });
        }
        let preparation = script_children_changed::character_data(scope, state, node)?;
        let result = state.dom.borrow_mut().split_text(node, offset);
        // Capture the new object's identity before children-changed script
        // can move it to another arena, and publish range state first.
        let result = result.map_err(V8BindingError::from).and_then(|created| {
            let wrapper = wrap_node(scope, state, created)?;
            synchronize_live_ranges(state)?;
            Ok(wrapper)
        });
        let wrapper = preparation.finish_result(scope, result)?;
        Ok(Some(wrapper.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_eventtarget_addeventlistener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        node_event_listeners::change(scope, args, member == "addEventListener")?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_eventtarget_dispatchevent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let event = v8::Local::<v8::Object>::try_from(args.get(0))
            .map_err(|_| V8BindingError::Allocation("event object"))?;
        let event_type = prepare_author_event_dispatch(scope, event)?;
        let accepted = dispatch_event(scope, state, node, event)?;
        let (owner, node) = node_identity::argument(scope, args.this().into())?
            .ok_or(V8BindingError::Allocation("dispatch target owner"))?;
        let state = &owner;
        if accepted && event_type.eq_ignore_ascii_case("click") {
            permission_elements::activate(state, node, false)?;
        }
        Ok(Some(v8::Boolean::new(scope, accepted).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_adoptnode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let (source, source_node) =
            node_identity::argument(scope, args.get(0))?.ok_or(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "TypeError".into(),
            })?;
        let kind = source.dom.borrow().arena().get(source_node)?.kind.clone();
        if matches!(kind, NodeKind::Document | NodeKind::ShadowRoot { .. }) {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: if matches!(kind, NodeKind::Document) {
                    "NotSupportedError"
                } else {
                    "HierarchyRequestError"
                }
                .into(),
            });
        }
        node_migration::adopt(scope, state, node, &source, source_node)?;
        wrapper_graphs::refresh_agent(scope, state);
        Ok(Some(args.get(0)))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_importnode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        // Convert the Node argument before the options dictionary, but do
        // not retain its arena-local identity across authored getters.
        node_identity::argument(scope, args.get(0))?.ok_or(V8BindingError::Unsupported {
            interface: "Document".into(),
            member: "TypeError".into(),
        })?;
        let (deep, fallback) = custom_elements::import_options(scope, args.get(1))?;
        let (source, source_node) =
            node_identity::argument(scope, args.get(0))?.ok_or(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "TypeError".into(),
            })?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        if matches!(
            source.dom.borrow().arena().get(source_node)?.kind,
            NodeKind::Document | NodeKind::ShadowRoot { .. }
        ) {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "NotSupportedError".into(),
            });
        }
        let imported = if Rc::ptr_eq(&source.dom, &state.dom) {
            state.dom.borrow_mut().clone_node(source_node, deep)?
        } else {
            let source = source.dom.borrow();
            let mut destination = state.dom.borrow_mut();
            destination.import_node_from(source.arena(), source_node, deep)?
        };
        state
            .dom
            .borrow_mut()
            .initialize_cloned_node_document(imported, node)?;
        template_links::complete_clone(scope, &source, source_node, state, imported, deep)?;
        custom_elements::clone_registry_tree(scope, state, imported, node, fallback, true)?;
        declarative_stylesheets::prepare_declarative_stylesheets(scope, state, imported)?;
        custom_elements::upgrade_for_document(scope, state, imported)?;
        Ok(Some(wrap_node(scope, state, imported)?.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_createattribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        document_factory_arguments::require(
            args,
            if member == "createAttributeNS" { 2 } else { 1 },
        )?;
        let (namespace, prefix, local_name, name) = if member == "createAttributeNS" {
            let namespace = if args.get(0).is_null_or_undefined() {
                String::new()
            } else {
                required_string(scope, args, 0)?
            };
            let name = required_string(scope, args, 1)?;
            validate_node_namespace(&namespace, &name, is_valid_attribute_local_name)?;
            let (prefix, local) = name
                .split_once(':')
                .map_or((None, name.clone()), |(prefix, local)| {
                    (Some(prefix.to_string()), local.to_string())
                });
            (
                (!namespace.is_empty()).then_some(namespace),
                prefix,
                local,
                name,
            )
        } else {
            let mut name = required_string(scope, args, 0)?;
            validate_attribute_local_name(&name)?;
            if state
                .dom
                .borrow()
                .arena()
                .get(node)?
                .document_metadata
                .as_ref()
                .is_some_and(|m| m.is_html)
            {
                name = oryn_dom::dom_string::ascii_lowercase(&name);
            }
            (None, None, name.clone(), name)
        };
        let created = state.dom.borrow_mut().create(NodeKind::Attribute {
            data: std::sync::Arc::new(oryn_dom::ContentAttribute {
                name,
                local_name,
                namespace,
                prefix,
                value: String::new(),
            }),
            owner: None,
        });
        state.dom.borrow_mut().set_node_document(created, node)?;
        Ok(Some(wrap_node(scope, state, created)?.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_createelement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        document_factory_arguments::require(args, 1)?;
        let name = required_string(scope, args, 0)?;
        let options = custom_elements::creation_options(scope, args.get(1))?;
        if !is_valid_element_local_name(&name) {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "InvalidCharacterError".into(),
            });
        }
        let mut dom = state.dom.borrow_mut();
        let metadata = dom
            .arena()
            .get(node)?
            .document_metadata
            .as_ref()
            .expect("document metadata");
        let html = metadata.is_html;
        let namespace = if html || metadata.content_type == "application/xhtml+xml" {
            "http://www.w3.org/1999/xhtml"
        } else {
            ""
        };
        let local_name = if html {
            oryn_dom::dom_string::ascii_lowercase(&name)
        } else {
            name
        };
        let created = dom.create_element_with_local_name(namespace, &local_name);
        drop(dom);
        state.dom.borrow_mut().set_node_document(created, node)?;
        custom_elements::initialize_created_element(state, node, created, options)?;
        let (owner, created) = custom_elements::create_synchronously(scope, state, created)?;
        Ok(Some(wrap_node(scope, &owner, created)?.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_createelementns<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        document_factory_arguments::require(args, 2)?;
        let namespace = if args.get(0).is_null_or_undefined() {
            String::new()
        } else {
            required_string(scope, args, 0)?
        };
        let name = required_string(scope, args, 1)?;
        let options = custom_elements::creation_options(scope, args.get(2))?;
        validate_element_namespace(&namespace, &name)?;
        let created = state.dom.borrow_mut().create_element_ns(&namespace, &name);
        state.dom.borrow_mut().set_node_document(created, node)?;
        custom_elements::initialize_created_element(state, node, created, options)?;
        let (owner, created) = custom_elements::create_synchronously(scope, state, created)?;
        Ok(Some(wrap_node(scope, &owner, created)?.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_createtextnode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        document_factory_arguments::require(args, 1)?;
        let data = required_string(scope, args, 0)?;
        let created = state.dom.borrow_mut().create_text(data);
        state.dom.borrow_mut().set_node_document(created, node)?;
        Ok(Some(wrap_node(scope, state, created)?.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_createcomment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        document_factory_arguments::require(args, 1)?;
        let data = required_string(scope, args, 0)?;
        let created = state.dom.borrow_mut().create_comment(data);
        state.dom.borrow_mut().set_node_document(created, node)?;
        Ok(Some(wrap_node(scope, state, created)?.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_createcdatasection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        document_factory_arguments::require(args, 1)?;
        let data = required_string(scope, args, 0)?;
        if state
            .dom
            .borrow()
            .arena()
            .get(node)?
            .document_metadata
            .as_ref()
            .expect("document metadata")
            .is_html
        {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "NotSupportedError".into(),
            });
        }
        if oryn_dom::dom_string::units(&data)
            .windows(3)
            .any(|units| units == [93, 93, 62])
        {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "InvalidCharacterError".into(),
            });
        }
        let created = state.dom.borrow_mut().create_cdata_section(data);
        state.dom.borrow_mut().set_node_document(created, node)?;
        Ok(Some(wrap_node(scope, state, created)?.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_createprocessinginstruction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    document_processing_instruction::create(scope, state, node, args)
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_createdocumentfragment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let created = state.dom.borrow_mut().create_fragment();
        state.dom.borrow_mut().set_node_document(created, node)?;
        Ok(Some(wrap_node(scope, state, created)?.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_createevent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let requested = required_string(scope, args, 0)?.to_ascii_lowercase();
        let interface = if requested == "customevent" {
            "CustomEvent"
        } else if requested == "messageevent" {
            "MessageEvent"
        } else if matches!(requested.as_str(), "uievent" | "uievents") {
            "UIEvent"
        } else if requested == "focusevent" {
            "FocusEvent"
        } else if matches!(requested.as_str(), "mouseevent" | "mouseevents") {
            "MouseEvent"
        } else if requested == "keyboardevent" {
            "KeyboardEvent"
        } else if matches!(
            requested.as_str(),
            "event" | "events" | "htmlevents" | "svgevents"
        ) {
            "Event"
        } else {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "NotSupportedError".into(),
            });
        };
        let event = if matches!(interface, "MouseEvent" | "KeyboardEvent") {
            let mut fields = Vec::new();
            for name in [
                "ctrlKey",
                "shiftKey",
                "altKey",
                "metaKey",
                "modifierAltGraph",
                "modifierCapsLock",
            ] {
                fields.push((name, NativeInputValue::Bool(false)));
            }
            if interface == "KeyboardEvent" {
                for name in ["repeat", "isComposing"] {
                    fields.push((name, NativeInputValue::Bool(false)));
                }
                for name in ["location", "keyCode", "charCode", "which"] {
                    fields.push((name, NativeInputValue::Number(0.0)));
                }
                for name in ["key", "code"] {
                    fields.push((name, NativeInputValue::String(&[])));
                }
            } else {
                for name in [
                    "screenX",
                    "screenY",
                    "clientX",
                    "clientY",
                    "movementX",
                    "movementY",
                    "button",
                    "buttons",
                    "which",
                ] {
                    fields.push((name, NativeInputValue::Number(0.0)));
                }
            }
            let event = create_native_input_event(
                scope,
                state,
                NativeInputEventInputs {
                    interface,
                    event_type: "",
                    bubbles: false,
                    cancelable: false,
                    composed: false,
                    detail: 0,
                    related: None,
                    fields: &fields,
                },
            )?;
            let payload = event
                .get_internal_field(scope, 2)
                .and_then(|value| value.try_cast::<v8::Array>().ok())
                .ok_or(V8BindingError::Allocation("legacy input event payload"))?;
            let null = v8::null(scope);
            payload.set_index(scope, 0, null.into());
            event
        } else {
            create_event_object_with_interface(scope, state, interface, "", false, false)?
        };
        if interface == "MessageEvent" {
            message_events::initialize_empty(scope, state, event)
                .ok_or(V8BindingError::Allocation("MessageEvent payload"))?;
        }
        if matches!(interface, "UIEvent" | "FocusEvent") {
            let data = v8::Array::new(scope, 4);
            let null = v8::null(scope);
            data.set_index(scope, 0, null.into());
            let zero = v8::Integer::new(scope, 0);
            data.set_index(scope, 1, zero.into());
            data.set_index(scope, 2, null.into());
            let focus = v8::Boolean::new(scope, interface == "FocusEvent");
            data.set_index(scope, 3, focus.into());
            event.set_internal_field(2, data.into());
            tag_ui_event_object(scope, state, event);
        }
        let id = event_id(scope, event)?;
        let mut events = state.events.borrow_mut();
        let record = events.get_mut(&id).unwrap();
        record.initialized = false;
        record.is_trusted = false;
        Ok(Some(event.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_execcommand<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let command = required_string(scope, args, 0)?.to_ascii_lowercase();
        let html_document = state
            .dom
            .borrow()
            .arena()
            .get(node)?
            .document_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.is_html);
        if !html_document {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "InvalidStateError".into(),
            });
        }
        // Capability support is an HTML document interface query, not an
        // executable-document mutation. Conversion and XML rejection above
        // still precede this lookup; inert mutations remain guarded below.
        if member == "queryCommandSupported" {
            let supported = matches!(command.as_str(), "copy" | "cut" | "paste")
                || edit_command_supported(&command);
            return Ok(Some(v8::Boolean::new(scope, supported).into()));
        }
        if active_document_state_for_node(scope, state, node).is_none() {
            return Ok(Some(if member == "queryCommandValue" {
                v8_string(scope, "")?
            } else {
                v8::Boolean::new(scope, false).into()
            }));
        }
        let invocation = if member == "execCommand" {
            let Some(invocation) = editing_commands::admit_invocation(scope, state, node)? else {
                return Ok(Some(v8::Boolean::new(scope, false).into()));
            };
            Some(invocation)
        } else {
            None
        };
        if matches!(command.as_str(), "copy" | "cut" | "paste") {
            let value = match member {
                "queryCommandEnabled" => async_clipboard::exec(scope, state, &command, false)?,
                "execCommand" => async_clipboard::exec(scope, state, &command, true)?,
                "queryCommandValue" => return Ok(Some(v8_string(scope, "")?)),
                _ => false,
            };
            return Ok(Some(v8::Boolean::new(scope, value).into()));
        }
        let supported = edit_command_supported(&command);
        if member == "queryCommandEnabled" {
            let focused_control = state
                .dom
                .borrow()
                .focused()
                .filter(|node| state.dom.borrow().is_text_control(*node));
            let history_enabled = match command.as_str() {
                "undo" | "redo" if focused_control.is_some() => {
                    native_keyboard::control_history_available(
                        state,
                        focused_control.unwrap(),
                        command == "redo",
                    )
                }
                "undo" => !state.undo_history.borrow().undo.is_empty(),
                "redo" => !state.undo_history.borrow().redo.is_empty(),
                _ => true,
            };
            return Ok(Some(
                v8::Boolean::new(
                    scope,
                    supported
                        && history_enabled
                        && (edit_command_enabled_without_selection(&command)
                            || document_is_editable(state)
                            || (matches!(
                                command.as_str(),
                                "undo"
                                    | "redo"
                                    | "inserttext"
                                    | "delete"
                                    | "forwarddelete"
                                    | "selectall"
                            ) && focused_control.is_some_and(|node| {
                                let dom = state.dom.borrow();
                                !dom.control_disabled(node).unwrap_or(true)
                                    && dom
                                        .attribute_in_null_namespace(node, "readonly")
                                        .is_ok_and(|value| value.is_none())
                            }))),
                )
                .into(),
            ));
        }
        if matches!(member, "queryCommandIndeterm" | "queryCommandState" | "queryCommandValue")
            && matches!(command.as_str(), "justifyleft" | "justifyright" | "justifycenter" | "justifyfull")
        {
            refresh_layout_in_scope(scope, state)?;
        }
        if member == "queryCommandIndeterm" {
            let alignment = match command.as_str() {
                "justifyleft" => Some("left"),
                "justifyright" => Some("right"),
                "justifycenter" => Some("center"),
                "justifyfull" => Some("justify"),
                _ => None,
            };
            let mixed = if let Some(expected) = alignment {
                let values = selection_block_alignments(state)?;
                values.iter().any(|value| value == expected)
                    && values.iter().any(|value| value != expected)
            } else {
                false
            };
            return Ok(Some(v8::Boolean::new(scope, mixed).into()));
        }
        if member == "queryCommandState" {
            let active = match command.as_str() {
                "stylewithcss" => state.style_with_css.get(),
                "justifyleft" => {
                    state.selection_range.get().is_some() && selection_has_alignment(state, "left")?
                }
                "justifycenter" => {
                    state.selection_range.get().is_some()
                        && selection_has_alignment(state, "center")?
                }
                "justifyright" => {
                    state.selection_range.get().is_some()
                        && selection_has_alignment(state, "right")?
                }
                "justifyfull" => {
                    state.selection_range.get().is_some()
                        && selection_has_alignment(state, "justify")?
                }
                "bold" | "italic" | "underline" | "strikethrough" | "subscript" | "superscript" => {
                    selection_format_value(state, &command).is_some()
                }
                _ => false,
            };
            return Ok(Some(v8::Boolean::new(scope, active).into()));
        }
        if member == "queryCommandValue" {
            let value = match command.as_str() {
                "stylewithcss" => String::new(),
                "defaultparagraphseparator" => state.default_paragraph_separator.borrow().clone(),
                "justifyleft" | "justifycenter" | "justifyright" | "justifyfull" => {
                    if state.selection_range.get().is_some() {
                        selection_block_alignment(state)?
                    } else {
                        String::new()
                    }
                }
                "backcolor" | "hilitecolor" | "forecolor" | "fontname" | "fontsize"
                | "formatblock" => selection_format_value(state, &command).unwrap_or_default(),
                _ => String::new(),
            };
            return Ok(Some(v8_string(scope, &value)?));
        }
        let document_command = edit_command_enabled_without_selection(&command);
        let focused_control = state
            .dom
            .borrow()
            .focused()
            .filter(|node| state.dom.borrow().is_text_control(*node));
        if let Some(control) = focused_control
            && matches!(command.as_str(), "undo" | "redo")
        {
            let editable = {
                let dom = state.dom.borrow();
                !dom.control_disabled(control)?
                    && dom
                        .attribute_in_null_namespace(control, "readonly")?
                        .is_none()
            };
            let changed =
                editable && native_keyboard::undo_control(state, control, command == "redo")?;
            if changed {
                clipboard_actions::fire_editing(
                    scope,
                    state,
                    control,
                    "input",
                    if command == "redo" {
                        "historyRedo"
                    } else {
                        "historyUndo"
                    },
                    None,
                    None,
                )?;
            }
            return Ok(Some(v8::Boolean::new(scope, changed).into()));
        }
        if let Some(control) = focused_control
            && matches!(
                command.as_str(),
                "inserttext" | "inserthtml" | "delete" | "forwarddelete" | "selectall"
            )
        {
            let length =
                oryn_dom::dom_string::len(&state.dom.borrow().control_value(control)?) as u32;
            let (mut start, mut end, _) = state
                .text_control_selections
                .borrow()
                .get(&control)
                .cloned()
                .unwrap_or((length, length, "none".into()));
            start = start.min(length);
            end = end.min(length).max(start);
            if command == "selectall" {
                state
                    .text_control_selections
                    .borrow_mut()
                    .insert(control, (0, length, "none".into()));
                return Ok(Some(v8::Boolean::new(scope, true).into()));
            }
            let value = if command == "delete" || command == "forwarddelete" {
                if start == end {
                    if command == "delete" {
                        start = start.saturating_sub(1);
                    } else {
                        end = (end + 1).min(length);
                    }
                }
                String::new()
            } else if args.get(2).is_undefined() {
                String::new()
            } else {
                required_string(scope, args, 2)?
            };
            let value = if command == "inserthtml" {
                let parsed = oryn_dom::html::parse_fragment_dom_string(&value, "div");
                parsed.dom.text_content(parsed.document)?
            } else {
                value
            };
            let before = state.dom.borrow().control_value(control)?;
            edit_native_control(state, control, start, end, &value)?;
            if before != state.dom.borrow().control_value(control)? {
                let input_type = match command.as_str() {
                    "delete" => "deleteContentBackward",
                    "forwarddelete" => "deleteContentForward",
                    "inserthtml" => "insertFromPaste",
                    _ => "insertText",
                };
                let event = create_input_event_object(
                    scope,
                    state,
                    "input",
                    input_type,
                    Some(&value),
                    false,
                )?;
                dispatch_event(scope, state, control, event)?;
            }
            return Ok(Some(v8::Boolean::new(scope, true).into()));
        }
        if !supported || (!document_is_editable(state) && !document_command) {
            return Ok(Some(v8::Boolean::new(scope, false).into()));
        }
        let settings_owner = if matches!(
            command.as_str(),
            "stylewithcss" | "usecss" | "defaultparagraphseparator"
        ) {
            active_document_state_for_node(scope, state, node).map(|owner| {
                let identity = (
                    owner.document_root(),
                    owner.document_generation(),
                    owner.document_authority_epoch(),
                );
                (owner, identity)
            })
        } else {
            None
        };
        let value = if args.get(2).is_undefined() {
            String::new()
        } else {
            required_string(scope, args, 2)?
        };
        if matches!(
            command.as_str(),
            "stylewithcss" | "usecss" | "defaultparagraphseparator"
        ) {
            let current = settings_owner.as_ref().is_some_and(|(owner, identity)| {
                !owner.execution_retired()
                    && (
                        owner.document_root(),
                        owner.document_generation(),
                        owner.document_authority_epoch(),
                    ) == *identity
                    && active_document_state_for_node(scope, state, node)
                        .is_some_and(|current| Rc::ptr_eq(&current, owner))
            });
            if !current {
                return Ok(Some(v8::Boolean::new(scope, false).into()));
            }
        }
        let plaintext = editing_host(state)?
            .is_some_and(|host| host_is_plaintext_only(&state.dom.borrow(), host));
        let formatting = matches!(
            command.as_str(),
            "backcolor"
                | "bold"
                | "createlink"
                | "fontname"
                | "fontsize"
                | "forecolor"
                | "formatblock"
                | "hilitecolor"
                | "indent"
                | "insertorderedlist"
                | "insertunorderedlist"
                | "italic"
                | "justifycenter"
                | "justifyfull"
                | "justifyleft"
                | "justifyright"
                | "outdent"
                | "removeformat"
                | "strikethrough"
                | "subscript"
                | "superscript"
                | "underline"
                | "unlink"
        );
        if plaintext && formatting {
            return Ok(Some(v8::Boolean::new(scope, false).into()));
        }
        let executed = match command.as_str() {
            "stylewithcss" => {
                state
                    .style_with_css
                    .set(!value.eq_ignore_ascii_case("false"));
                true
            }
            "usecss" => {
                state
                    .style_with_css
                    .set(value.eq_ignore_ascii_case("false"));
                true
            }
            "defaultparagraphseparator" => {
                let separator = value.to_ascii_lowercase();
                if matches!(separator.as_str(), "div" | "p") {
                    *state.default_paragraph_separator.borrow_mut() = separator;
                    true
                } else {
                    false
                }
            }
            "selectall" => {
                let root = state
                    .dom
                    .borrow()
                    .focused()
                    .and_then(|focused| {
                        let dom = state.dom.borrow();
                        if let Ok(Some(host)) =
                            editing_host_for_node(&dom, Some(focused), false, state.document_root())
                        {
                            return Some(host);
                        }
                        let mut current = Some(focused);
                        while let Some(candidate) = current {
                            let record = dom.arena().get(candidate).ok()?;
                            if matches!(record.kind, NodeKind::ShadowRoot { .. }) {
                                return Some(candidate);
                            }
                            current = record.parent;
                        }
                        Some(focused)
                    })
                    .or_else(|| {
                        state
                            .dom
                            .borrow()
                            .query_selector_all(node, "body")
                            .ok()
                            .and_then(|nodes| nodes.first().copied())
                    })
                    .unwrap_or(node);
                let end = boundary_length(&state.dom.borrow(), root)?;
                set_selection_boundaries(
                    state,
                    BoundaryPoint {
                        container: root,
                        offset: 0,
                    },
                    BoundaryPoint {
                        container: root,
                        offset: end,
                    },
                )?;
                true
            }
            "undo" | "redo" => {
                let changed = undo_edit(state, command == "redo")?;
                if changed {
                    dispatch_editing_input(
                        scope,
                        state,
                        if command == "redo" {
                            "historyRedo"
                        } else {
                            "historyUndo"
                        },
                        None,
                    )?;
                }
                true
            }
            _ => {
                let changed = editing_commands::run(
                    scope,
                    state,
                    &command,
                    editing_commands::Command::Execute {
                        command: command.clone(),
                        value: value.clone(),
                        plaintext,
                        invocation: invocation.as_ref().expect("exec invocation").clone(),
                    },
                )?;
                // These commands are accepted for an editable selection
                // even when there is no resulting markup diff. The input
                // transaction is still observable.
                let accepted = changed
                    || (state.selection_anchor.get().is_some()
                        && state.selection_anchor.get() == state.selection_focus.get()
                        && matches!(
                            command.as_str(),
                            "bold"
                                | "italic"
                                | "underline"
                                | "strikethrough"
                                | "subscript"
                                | "superscript"
                                | "fontname"
                                | "fontsize"
                                | "forecolor"
                                | "backcolor"
                                | "hilitecolor"
                        ))
                    || matches!(
                        command.as_str(),
                        "delete"
                            | "forwarddelete"
                            | "formatblock"
                            | "justifyleft"
                            | "justifycenter"
                            | "justifyright"
                            | "justifyfull"
                    );
                if accepted {
                    let input_type = match command.as_str() {
                        "delete" => "deleteContentBackward",
                        "forwarddelete" => "deleteContentForward",
                        "inserttext" => "insertText",
                        "inserthtml" => "insertFromPaste",
                        "insertlinebreak" => "insertLineBreak",
                        "insertparagraph" => "insertParagraph",
                        "bold" => "formatBold",
                        "italic" => "formatItalic",
                        "underline" => "formatUnderline",
                        "strikethrough" => "formatStrikeThrough",
                        "justifycenter" => "formatJustifyCenter",
                        "justifyfull" => "formatJustifyFull",
                        "justifyleft" => "formatJustifyLeft",
                        "justifyright" => "formatJustifyRight",
                        "indent" => "formatIndent",
                        "outdent" => "formatOutdent",
                        "removeformat" => "formatRemove",
                        "insertorderedlist" => "insertOrderedList",
                        "insertunorderedlist" => "insertUnorderedList",
                        _ if formatting => "formatSetBlockTextDirection",
                        _ => "insertReplacementText",
                    };
                    dispatch_editing_input(
                        scope,
                        state,
                        input_type,
                        (command == "inserttext").then_some(value.as_str()),
                    )?;
                }
                accepted
            }
        };
        Ok(Some(v8::Boolean::new(scope, executed).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlinputelement_htmltextareaelement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let original = wrap_node(scope, state, node)?;
        if args.length() == 0 || args.length() == 2 {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: "TypeError".into(),
            });
        }
        let replacement = required_string(scope, args, 0)?;
        let explicit_offsets = if args.length() >= 3 {
            Some((
                args.get(1)
                    .uint32_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?,
                args.get(2)
                    .uint32_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?,
            ))
        } else {
            None
        };
        let mode = if args.get(3).is_undefined() {
            "preserve".into()
        } else {
            args.get(3)
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope)
        };
        if !matches!(mode.as_str(), "select" | "start" | "end" | "preserve") {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: "TypeError".into(),
            });
        }
        let (owner, node) =
            node_identity::argument(scope, original.into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        if !state.dom.borrow().supports_selection_range(node)? {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: "InvalidStateError".into(),
            });
        }
        // Read live value and selection after argument conversions, which
        // may execute author code and change the control.
        let length = oryn_dom::dom_string::len(&state.dom.borrow().control_value(node)?) as u32;
        let (selection_start, selection_end, _) = state
            .text_control_selections
            .borrow()
            .get(&node)
            .cloned()
            .unwrap_or((0, 0, "none".into()));
        let (start, end) =
            explicit_offsets.unwrap_or((selection_start.min(length), selection_end.min(length)));
        if start > end {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: "IndexSizeError".into(),
            });
        }
        let start = start.min(length);
        let end = end.min(length);
        let inserted = oryn_dom::dom_string::len(&replacement) as u32;
        let new_end = start + inserted;
        state
            .dom
            .borrow_mut()
            .replace_control_value(node, start, end, &replacement)?;
        let (new_start, new_selection_end) = match mode.as_str() {
            "select" => (start, new_end),
            "start" => (start, start),
            "end" => (new_end, new_end),
            _ => {
                let adjust = |point: u32, inside: u32| {
                    let point = point.min(length);
                    if point > end {
                        point - (end - start) + inserted
                    } else if point > start {
                        inside
                    } else {
                        point
                    }
                };
                (
                    adjust(selection_start, start),
                    adjust(selection_end, new_end),
                )
            }
        };
        state
            .text_control_selections
            .borrow_mut()
            .insert(node, (new_start, new_selection_end, "none".into()));
        if (selection_start, selection_end) != (new_start, new_selection_end) {
            queue_control_select(scope, state, node)?;
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlinputelement_htmltextareaelement_22<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let original = wrap_node(scope, state, node)?;
        let start = args
            .get(0)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let end = args
            .get(1)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let (owner, node) =
            node_identity::argument(scope, original.into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        Ok(Some(
            opaque_ranges::create(scope, state, node, start, end)?.into(),
        ))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_createrange<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let id = register_live_range(
            state,
            RangeRecord {
                start_container: node,
                start_offset: 0,
                end_container: node,
                end_offset: 0,
            },
        )?;
        Ok(Some(new_stateful_object(scope, state, "Range", id)?.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_getselection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        if let Some(object) = state.selection_object.borrow().as_ref() {
            return object
                .to_local(scope)
                .map(|object| Some(object.into()))
                .ok_or(V8BindingError::Allocation("selection context"));
        }
        let object = new_stateful_object(scope, state, "Selection", 1)?;
        let retained = context_intrinsics::retain_object_for(scope, state, "selection", object)?;
        state.selection_object.borrow_mut().replace(retained);
        Ok(Some(object.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_createnodeiterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        node_identity::canonical(scope, args.get(0))?.ok_or(V8BindingError::Unsupported {
            interface: "NodeIterator".into(),
            member: "TypeError".into(),
        })?;
        let what_to_show = if args.get(1).is_undefined() {
            u32::MAX
        } else {
            args.get(1)
                .uint32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?
        };
        if member == "createTreeWalker" {
            let (root_owner, root) = node_identity::argument(scope, args.get(0))?.ok_or(
                V8BindingError::Unsupported {
                    interface: "TreeWalker".into(),
                    member: "TypeError".into(),
                },
            )?;
            return Ok(Some(
                tree_walkers::create(scope, state, &root_owner, root, what_to_show, args.get(2))?
                    .into(),
            ));
        }
        Ok(Some(
            node_iterators::create(scope, state, args.get(0), what_to_show, args.get(2))?.into(),
        ))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_elementinternals<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    custom_elements::internals_call(scope, state, node, member, args)
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlelement_attachinternals<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    custom_elements::attach_internals(scope, state, node, args.get(0)).map(Some)
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_customstateset_keys<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    custom_state_iterator::create(scope, args.this(), member == "entries").map(Some)
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_customstateset_foreach<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let callback = v8::Local::<v8::Function>::try_from(args.get(0)).map_err(|_| {
            V8BindingError::Unsupported {
                interface: "CustomStateSet".into(),
                member: "TypeError".into(),
            }
        })?;
        let mut index = 0;
        loop {
            let entry = state
                .custom_state_order
                .borrow()
                .get(&node)
                .and_then(|order| order.next_from(index));
            let Some((next, value)) = entry else { break };
            index = next;
            let value = v8_string(scope, &value)?;
            callback
                .call(scope, args.get(1), &[value, value, args.this().into()])
                .ok_or(V8BindingError::JavaScriptException)?;
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_customstateset_add<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        if member == "clear" {
            let changed = state
                .custom_states
                .borrow_mut()
                .remove(&node)
                .is_some_and(|states| !states.is_empty());
            if changed {
                custom_elements::mark_internals_changed(state, node);
            }
            if let Some(order) = state.custom_state_order.borrow_mut().get_mut(&node) {
                order.clear();
            }
            return Ok(None);
        }
        let original = wrap_node(scope, state, node)?;
        let value = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, original.into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        match member {
            "add" => {
                let present = state
                    .custom_states
                    .borrow()
                    .get(&node)
                    .is_some_and(|states| states.contains(&value));
                if !present {
                    state
                        .custom_state_order
                        .borrow_mut()
                        .entry(node)
                        .or_default()
                        .insert(value.clone())?;
                    state
                        .custom_states
                        .borrow_mut()
                        .entry(node)
                        .or_default()
                        .insert(value);
                    custom_elements::mark_internals_changed(state, node);
                }
                Ok(Some(args.this().into()))
            }
            "delete" => {
                let removed = state
                    .custom_states
                    .borrow_mut()
                    .get_mut(&node)
                    .is_some_and(|states| states.remove(&value));
                if removed {
                    custom_elements::mark_internals_changed(state, node);
                    if let Some(order) = state.custom_state_order.borrow_mut().get_mut(&node) {
                        order.remove(&value);
                    }
                }
                Ok(Some(v8::Boolean::new(scope, removed).into()))
            }
            _ => {
                let present = state
                    .custom_states
                    .borrow()
                    .get(&node)
                    .is_some_and(|states| states.contains(&value));
                Ok(Some(v8::Boolean::new(scope, present).into()))
            }
        }
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_shadowroot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        sanitizer::insert_html(scope, state, node, args, member)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_shadowroot_32<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        Ok(Some(
            html_stream::create(scope, state, node, args.get(0), member)?.into(),
        ))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_shadowroot_33<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        sanitizer::set_html(scope, state, node, args, member == "setHTML")?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_shadowroot_34<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let text = template_links::get_html(scope, state, node, args.get(0))?;
        Ok(Some(v8_string(scope, &text)?))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_attachshadow<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let original = wrap_node(scope, state, node)?;
        let value = args.get(0);
        if !value.is_object() {
            throw_type_error(scope, "attachShadow requires a ShadowRootInit dictionary");
            return Err(V8BindingError::JavaScriptException);
        }
        let options = value
            .to_object(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        // Web IDL dictionaries read members in lexicographic order, before
        // validating or mutating the host. Getters may reenter the DOM.
        let clonable_key = v8::String::new(scope, "clonable")
            .ok_or(V8BindingError::Allocation("ShadowRootInit.clonable"))?;
        let clonable = options
            .get(scope, clonable_key.into())
            .ok_or(V8BindingError::JavaScriptException)?
            .boolean_value(scope);
        let registry = custom_elements::registry_option(scope, options)?;
        let delegates_key = v8::String::new(scope, "delegatesFocus")
            .ok_or(V8BindingError::Allocation("delegatesFocus option"))?;
        let delegates_focus = options
            .get(scope, delegates_key.into())
            .ok_or(V8BindingError::JavaScriptException)?
            .boolean_value(scope);
        let mode_key = v8::String::new(scope, "mode")
            .ok_or(V8BindingError::Allocation("ShadowRootInit.mode"))?;
        let mode_value = options
            .get(scope, mode_key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if mode_value.is_undefined() {
            throw_type_error(scope, "ShadowRootInit.mode is required");
            return Err(V8BindingError::JavaScriptException);
        }
        let mode = mode_value
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope);
        let mode = match mode.as_str() {
            "open" => ShadowMode::Open,
            "closed" => ShadowMode::Closed,
            _ => {
                throw_type_error(scope, "invalid ShadowRootMode");
                return Err(V8BindingError::JavaScriptException);
            }
        };
        let reference_key = v8_string(scope, "referenceTarget")?;
        let reference = options
            .get(scope, reference_key)
            .ok_or(V8BindingError::JavaScriptException)?;
        let reference_target = if reference.is_null_or_undefined() {
            None
        } else {
            let reference = reference
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            Some(rust_string_preserving_utf16(scope, reference)?)
        };
        let serializable_key = v8_string(scope, "serializable")?;
        let serializable = options
            .get(scope, serializable_key)
            .ok_or(V8BindingError::JavaScriptException)?
            .boolean_value(scope);
        let assignment_key = v8::String::new(scope, "slotAssignment")
            .ok_or(V8BindingError::Allocation("ShadowRootInit.slotAssignment"))?;
        let assignment_value = options
            .get(scope, assignment_key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        let assignment = if assignment_value.is_undefined() {
            "named".into()
        } else {
            assignment_value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope)
        };
        if !matches!(assignment.as_str(), "named" | "manual") {
            throw_type_error(scope, "invalid SlotAssignmentMode");
            return Err(V8BindingError::JavaScriptException);
        }
        let (owner, node) =
            node_identity::argument(scope, original.into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        if custom_elements::shadow_disabled(scope, state, node)? {
            return Err(V8BindingError::Unsupported {
                interface: "Element".into(),
                member: "NotSupportedError".into(),
            });
        }
        let created = {
            let mut dom = state.dom.borrow_mut();
            let allowed = match &dom.arena().get(node)?.kind {
                NodeKind::Element { name } => {
                    name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                        && (matches!(
                            name.local.as_ref(),
                            "article"
                                | "aside"
                                | "blockquote"
                                | "body"
                                | "div"
                                | "footer"
                                | "h1"
                                | "h2"
                                | "h3"
                                | "h4"
                                | "h5"
                                | "h6"
                                | "header"
                                | "main"
                                | "nav"
                                | "p"
                                | "section"
                                | "span"
                        ) || is_valid_custom_element_name(name.local.as_ref()))
                }
                _ => false,
            };
            if !allowed {
                return Err(V8BindingError::Unsupported {
                    interface: "Element".into(),
                    member: "NotSupportedError".into(),
                });
            }
            if let Some(existing) = dom
                .arena()
                .get(node)?
                .children
                .iter()
                .copied()
                .find(|child| {
                    dom.arena()
                        .get(*child)
                        .is_ok_and(|record| matches!(record.kind, NodeKind::ShadowRoot { .. }))
                })
            {
                let reusable = dom.arena().get(existing)?.declarative_shadow_root
                    && matches!(dom.arena().get(existing)?.kind,NodeKind::ShadowRoot{mode:existing_mode,..}if existing_mode==mode);
                if reusable {
                    let children = dom.children(existing)?.to_vec();
                    for child in children {
                        dom.remove_child(existing, child)?;
                    }
                    dom.arena_mut().get_mut(existing)?.declarative_shadow_root = false;
                    drop(dom);
                    return Ok(Some(wrap_node(scope, state, existing)?.into()));
                }
            }
            if dom.arena().get(node)?.children.iter().any(|child| {
                matches!(
                    dom.arena().get(*child).ok().map(|record| &record.kind),
                    Some(NodeKind::ShadowRoot { .. })
                )
            }) {
                return Err(V8BindingError::Unsupported {
                    interface: "Element".into(),
                    member: "NotSupportedError".into(),
                });
            }
            let root = dom.create(NodeKind::ShadowRoot { mode, clonable });
            dom.arena_mut().get_mut(root)?.delegates_focus = delegates_focus;
            dom.arena_mut().get_mut(root)?.shadow_serializable = serializable;
            dom.set_shadow_reference_target(root, reference_target)?;
            if assignment == "manual" {
                dom.set_manual_slot_assignment(root)?;
            }
            dom.append_child(node, root)?;
            root
        };
        custom_elements::initialize_shadow_registry(state, node, created, registry)?;
        Ok(Some(wrap_node(scope, state, created)?.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_getattributenodens<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let namespace = if args.get(0).is_null_or_undefined() {
            None
        } else {
            Some(required_string(scope, args, 0)?)
        };
        let local_name = required_string(scope, args, 1)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let attribute = namespaced_attribute_node(state, node, namespace.as_deref(), &local_name)?;
        optional_node(scope, state, attribute).map(Some)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_getattributenode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let name = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let name = normalized_attribute_name(state, node, name)?;
        let attribute = state.dom.borrow_mut().attribute_node(node, &name)?;
        optional_node(scope, state, attribute).map(Some)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_insertadjacenthtml<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let position = required_string(scope, args, 0)?;
        let markup = trusted_string(
            scope,
            state,
            args.get(1),
            TrustedKind::Html,
            "Element insertAdjacentHTML",
        )?;
        let (owner, node) = node_identity::argument(scope, args.this().into())?.ok_or(
            V8BindingError::Unsupported {
                interface: "Element".into(),
                member: "TypeError".into(),
            },
        )?;
        let (parent, reference) = adjacent_insertion_point(&owner.dom.borrow(), node, &position)?;
        let context = {
            let dom = owner.dom.borrow();
            if matches!(dom.arena().get(parent)?.kind, NodeKind::Document) {
                return Err(V8BindingError::Unsupported {
                    interface: "Element".into(),
                    member: "NoModificationAllowedError".into(),
                });
            }
            if matches!(dom.arena().get(parent)?.kind, NodeKind::Element { .. })
                && !dom.is_element(parent, "http://www.w3.org/1999/xhtml", "html")
            {
                oryn_dom::sanitizer::FragmentContext::capture(&dom, parent)?
            } else {
                oryn_dom::sanitizer::FragmentContext::html_body()
            }
        };
        let (fragment, root) =
            sanitizer::prepare_legacy_fragment(&owner, parent, &markup, context)?;
        post_connection_scripts::check_fragment_capacity(scope, &owner, parent, &fragment, root)?;
        let staging = {
            let mut dom = owner.dom.borrow_mut();
            let staging = dom.create(NodeKind::DocumentFragment);
            let document = dom
                .node_document(parent)?
                .ok_or(DomError::StaleNode(parent))?;
            dom.set_node_document(staging, document)?;
            for &child in &fragment.arena().get(root)?.children {
                let imported = dom.import_subtree(fragment.arena(), child)?;
                dom.append_child(staging, imported)?;
            }
            staging
        };
        let _lease = oryn_dom::TrackedNodeLease::new(&owner.dom, staging)?;
        let inserted = light_children(&owner.dom.borrow(), staging)?;
        for &child in &inserted {
            custom_elements::initialize_fragment_registry_from(&owner, child, &owner, parent)?;
            owner.mark_script_subtree_inert(child)?;
        }
        let preparation =
            post_connection_scripts::capture_insertion(scope, &owner, parent, staging)?;
        let mutation = owner
            .dom
            .borrow_mut()
            .insert_dom_node(parent, staging, reference);
        preparation.finish_mutation(scope, mutation)?;
        for child in inserted {
            upgrade_custom_element_subtree(scope, &owner, child)?;
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_insertadjacentelement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let position = required_string(scope, args, 0)?;
        let text = if member == "insertAdjacentText" {
            Some(required_string(scope, args, 1)?)
        } else {
            let (source, inserted) = node_identity::argument(scope, args.get(1))?.ok_or(
                V8BindingError::Unsupported {
                    interface: "Element".into(),
                    member: "TypeError".into(),
                },
            )?;
            if !matches!(
                source.dom.borrow().arena().get(inserted)?.kind,
                NodeKind::Element { .. }
            ) {
                return Err(V8BindingError::Unsupported {
                    interface: "Element".into(),
                    member: "TypeError".into(),
                });
            }
            None
        };
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        if matches!(
            position.to_ascii_lowercase().as_str(),
            "beforebegin" | "afterend"
        ) && owner.dom.borrow().parent(node)?.is_none()
        {
            return Ok(if member == "insertAdjacentElement" {
                Some(v8::null(scope).into())
            } else {
                None
            });
        }
        let (parent, reference) = adjacent_insertion_point(&owner.dom.borrow(), node, &position)?;
        if let Some(text) = text {
            let inserted = owner.dom.borrow_mut().create_text(text);
            let preparation =
                post_connection_scripts::capture_insertion(scope, &owner, parent, inserted)?;
            let result = owner
                .dom
                .borrow_mut()
                .insert_dom_node(parent, inserted, reference);
            preparation.finish_mutation(scope, result)?;
            Ok(None)
        } else {
            let insertion = node_migration::insertion_argument(
                scope,
                &owner,
                parent,
                args.get(1),
                reference,
                None,
            )?;
            let preparation = post_connection_scripts::capture_insertion(
                scope,
                &insertion.owner,
                insertion.parent,
                insertion.node,
            )?;
            let mutation = insertion.owner.dom.borrow_mut().insert_dom_node(
                insertion.parent,
                insertion.node,
                insertion.reference,
            );
            preparation.finish_mutation(scope, mutation)?;
            Ok(Some(args.get(1)))
        }
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_shadowroot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let id = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let found = state.dom.borrow().get_element_by_id(node, &id)?;
        optional_node(scope, state, found).map(Some)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_open<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let owner =
            active_document_state_for_node(scope, state, node).unwrap_or_else(|| state.clone());
        let state = &owner;
        document_domain::require_strict_origin(scope, state, node)?;
        let context = state
            .global_object
            .to_local(scope)
            .and_then(|global| global.get_creation_context(scope))
            .ok_or(V8BindingError::Allocation("document parser realm"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        require_html_document_stream(state, node)?;
        if node == state.document_root()
            && (state.parser_script_depth.get() > 0 || state.document_write_depth.get() > 0)
        {
            return Ok(Some(wrap_node(scope, state, node)?.into()));
        }
        open_document_write_stream(scope, state, node)?;
        Ok(Some(wrap_node(scope, state, node)?.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    call_document_write(scope, state, node, member, args)
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let owner =
            active_document_state_for_node(scope, state, node).unwrap_or_else(|| state.clone());
        let state = &owner;
        let context = state
            .global_object
            .to_local(scope)
            .and_then(|global| global.get_creation_context(scope))
            .ok_or(V8BindingError::Allocation("document parser realm"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        require_html_document_stream(state, node)?;
        let gate = state.document_parser_gate.borrow().clone().filter(|gate| {
            gate.document() == node && gate.epoch() == state.document_stream_epoch() && gate.live()
        });
        if let Some(gate) = gate.filter(|gate| gate.owned()) {
            gate.close();
            if gate.can_finish_locally() {
                let (ready, error) =
                    document_parser_gate::with_parser_construction(scope, state, || {
                        gate.prepare_finish()
                    });
                if let Some(error) = error {
                    return Err(error);
                }
                if !ready {
                    return Ok(None);
                }
                let _ = gate.finish();
                state.parser_input_active.set(false);
                state.closed_document_stream.set(Some((
                    state.document_generation(),
                    state.document_stream_epoch(),
                )));
            }
            if state.document_write_roots.borrow_mut().remove(&node) {
                state.dom.borrow_mut().release_native(node)?;
            }
            return Ok(None);
        }
        if let Some((parser, _)) = state.detached_document_parsers.borrow_mut().remove(&node) {
            let _ = parser.finish();
        }
        if state.document_write_roots.borrow_mut().remove(&node) {
            state.dom.borrow_mut().release_native(node)?;
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_appendchild_insertbefore<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let reference = if member == "insertBefore" {
            optional_argument_node(scope, state, args.get(1))?
        } else {
            None
        };
        let insertion =
            node_migration::insertion_argument(scope, state, node, args.get(0), reference, None)?;
        let preparation = post_connection_scripts::capture_insertion(
            scope,
            &insertion.owner,
            insertion.parent,
            insertion.node,
        )?;
        let mutation = insertion.owner.dom.borrow_mut().insert_dom_node(
            insertion.parent,
            insertion.node,
            insertion.reference,
        );
        preparation.finish_mutation(scope, mutation)?;
        Ok(Some(args.get(0)))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_removechild<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let child = argument_node(scope, state, args.get(0))?;
        if state.dom.borrow().parent(child)? != Some(node) {
            return Err(DomError::NotAChild(child, node).into());
        }
        let preparation = script_children_changed::parent_change(scope, state, Some(node))?;
        let result = state.dom.borrow_mut().remove_child(node, child);
        preparation.finish_mutation(scope, result)?;
        // Children-changed scripts may adopt the removed node. Return the
        // original argument identity instead of consulting its former arena.
        Ok(Some(args.get(0)))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_replacechild<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let child = argument_node(scope, state, args.get(1))?;
        let insertion = node_migration::insertion_argument(
            scope,
            state,
            node,
            args.get(0),
            Some(child),
            Some(child),
        )?;
        if insertion.replacement_removed {
            let preparation = post_connection_scripts::capture_insertion(
                scope,
                &insertion.owner,
                insertion.parent,
                insertion.node,
            )?;
            let mutation = insertion.owner.dom.borrow_mut().insert_dom_node(
                insertion.parent,
                insertion.node,
                insertion.reference,
            );
            preparation.finish_mutation(scope, mutation)?;
            return Ok(Some(args.get(1)));
        }
        let state = &insertion.owner;
        let node = insertion.parent;
        let replacement = insertion.node;
        let child = insertion.replaced.ok_or(DomError::NotAChild(child, node))?;
        let reference = {
            let dom = state.dom.borrow();
            let siblings = light_children(&dom, node)?;
            let index = siblings
                .iter()
                .position(|candidate| *candidate == child)
                .ok_or(DomError::NotAChild(child, node))?;
            siblings[index + 1..]
                .iter()
                .copied()
                .find(|candidate| *candidate != replacement)
        };
        let preparation =
            post_connection_scripts::capture_insertion(scope, state, node, replacement)?;
        let mutation =
            state
                .dom
                .borrow_mut()
                .replace_dom_child(node, replacement, child, reference);
        preparation.finish_mutation(scope, mutation)?;
        Ok(Some(args.get(1)))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_movebefore<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let (moved_owner, moved) =
            node_identity::argument(scope, args.get(0))?.ok_or(V8BindingError::Unsupported {
                interface: "Node".into(),
                member: "TypeError".into(),
            })?;
        if !Rc::ptr_eq(&moved_owner.dom, &state.dom) {
            return Err(hierarchy_request_error());
        }
        let reference = optional_argument_node(scope, state, args.get(1))?;
        validate_tree_insertion(&state.dom.borrow(), node, moved)?;
        {
            let dom = state.dom.borrow();
            if let Some(reference) = reference
                && dom.arena().get(reference)?.parent != Some(node)
            {
                return Err(V8BindingError::Unsupported {
                    interface: "Node".into(),
                    member: "NotFoundError".into(),
                });
            }
            if !matches!(
                dom.arena().get(moved)?.kind,
                NodeKind::Element { .. }
                    | NodeKind::Text { .. }
                    | NodeKind::CDataSection { .. }
                    | NodeKind::ProcessingInstruction { .. }
                    | NodeKind::Comment { .. }
            ) || dom.arena().node_document(node)? != dom.arena().node_document(moved)?
                || dom.is_connected(node)? != dom.is_connected(moved)?
            {
                return Err(hierarchy_request_error());
            }

            if matches!(dom.arena().get(node)?.kind, NodeKind::Document) {
                let mut proposed = dom.children(node)?.to_vec();
                proposed.retain(|candidate| *candidate != moved);
                let index = reference
                    .and_then(|reference| proposed.iter().position(|item| *item == reference))
                    .unwrap_or(proposed.len());
                proposed.insert(index, moved);
                validate_child_sequence(&dom, node, &proposed)?;
            }
        }
        // Preserve native focus while reparenting the same NodeId. The
        // journal still records the old/new parentage for other consumers.
        state.dom.borrow_mut().move_before(node, moved, reference)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_clonenode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        if matches!(
            state.dom.borrow().arena().get(node)?.kind,
            NodeKind::ShadowRoot { .. }
        ) {
            return Err(V8BindingError::Unsupported {
                interface: "Node".into(),
                member: "NotSupportedError".into(),
            });
        }
        let deep = args.get(0).boolean_value(scope);
        let owner = owner_document_for_node(state, node)?;
        let clone = state.dom.borrow_mut().clone_node(node, deep)?;
        if matches!(
            state.dom.borrow().arena().get(node)?.kind,
            NodeKind::Document
        ) {
            state.derive_document_provenance(node, clone)?;
        }
        let owner = if matches!(
            state.dom.borrow().arena().get(clone)?.kind,
            NodeKind::Document
        ) {
            clone
        } else {
            owner
        };
        // The arena is shared by iframe realms. Its primary document is
        // not necessarily the source node's document. Preserve ownership
        // for the whole clone, including descendants later detached from it.
        state
            .dom
            .borrow_mut()
            .initialize_cloned_node_document(clone, owner)?;
        template_links::complete_clone(scope, state, node, state, clone, deep)?;
        let document = state
            .dom
            .borrow()
            .arena()
            .node_document(clone)?
            .unwrap_or(state.document_root());
        custom_elements::clone_registry_tree(scope, state, clone, document, None, false)?;
        declarative_stylesheets::prepare_declarative_stylesheets(scope, state, clone)?;
        custom_elements::upgrade_for_document(scope, state, clone)?;
        Ok(Some(wrap_node(scope, state, clone)?.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_node_append<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        node_mutator_bindings::operation(scope, state, node, member, args)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_node_remove<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let parent = state.dom.borrow().parent(node)?;
        let preparation = script_children_changed::parent_change(scope, state, parent)?;
        let result = state.dom.borrow_mut().remove(node);
        preparation.finish_mutation(scope, result)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_node_issamenode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let same = node_identity::same(scope, state, node, args.get(0))?;
        Ok(Some(v8::Boolean::new(scope, same).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_node_contains<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let Some((owner, other)) = node_identity::argument(scope, args.get(0))? else {
            return Ok(Some(v8::Boolean::new(scope, false).into()));
        };
        if !Rc::ptr_eq(&owner.dom, &state.dom) {
            return Ok(Some(v8::Boolean::new(scope, false).into()));
        }
        let dom = state.dom.borrow();
        let contains = node == other
            || (tree_root(&dom, node)? == tree_root(&dom, other)?
                && dom.arena().ancestors(other)?.contains(&node));
        Ok(Some(v8::Boolean::new(scope, contains).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_node_isequalnode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let Some((owner, other)) = node_identity::argument(scope, args.get(0))? else {
            return Ok(Some(v8::Boolean::new(scope, false).into()));
        };
        let dom = state.dom.borrow();
        let equal = dom.is_equal_node_from(node, &owner.dom.borrow(), other)?;
        Ok(Some(v8::Boolean::new(scope, equal).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_node_comparedocumentposition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let (owner, other) =
            node_identity::argument(scope, args.get(0))?.ok_or(V8BindingError::Unsupported {
                interface: "Node".into(),
                member: "TypeError".into(),
            })?;
        if !Rc::ptr_eq(&owner.dom, &state.dom) {
            // A stable implementation-specific ordering for disconnected
            // arenas, independent of colliding realm-local NodeIds.
            let order = if (Rc::as_ptr(&state.dom) as usize) < (Rc::as_ptr(&owner.dom) as usize) {
                4
            } else {
                2
            };
            return Ok(Some(v8::Integer::new(scope, 1 | 32 | order).into()));
        }
        if node == other {
            return Ok(Some(v8::Integer::new(scope, 0).into()));
        }
        let dom = state.dom.borrow();
        let node_root = tree_root(&dom, node)?;
        let other_root = tree_root(&dom, other)?;
        let position = if node_root != other_root {
            let order = if node < other { 4 } else { 2 };
            1 | 32 | order
        } else if dom.arena().ancestors(node)?.contains(&other) {
            8 | 2
        } else if dom.arena().ancestors(other)?.contains(&node) {
            16 | 4
        } else if traversal_position(&dom, node)? < traversal_position(&dom, other)? {
            4
        } else {
            2
        };
        Ok(Some(v8::Integer::new(scope, position).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_node_haschildnodes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let has_children = !state.dom.borrow().arena().get(node)?.children.is_empty();
        Ok(Some(v8::Boolean::new(scope, has_children).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_node_getrootnode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    node_identity::root(scope, args).map(Some)
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_setattributenode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    node_lists::attributes::element_operation(scope, state, node, member, args).map(Some)
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_getattribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let name = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let name = normalized_attribute_name(state, node, name)?;
        match state.dom.borrow().attribute(node, &name)? {
            Some(value) => Ok(Some(v8_string(scope, value)?)),
            None => Ok(Some(v8::null(scope).into())),
        }
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_getattributens<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let namespace = if args.get(0).is_null_or_undefined() {
            None
        } else {
            Some(required_string(scope, args, 0)?)
        };
        let local_name = required_string(scope, args, 1)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let dom = state.dom.borrow();
        let value = dom.attribute_ns(node, namespace.as_deref(), &local_name)?;
        match value {
            Some(value) => Ok(Some(v8_string(scope, value)?)),
            None => Ok(Some(v8::null(scope).into())),
        }
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_getattributenames<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let names = state
            .dom
            .borrow()
            .arena()
            .get(node)?
            .attributes
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let values = names
            .iter()
            .map(|name| v8_string(scope, name))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(v8::Array::new_with_elements(scope, &values).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_setpointercapture<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let id = args
            .get(0)
            .int32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let captured = native_pointer::capture(state, node, id, member)?;
        if member == "hasPointerCapture" {
            Ok(Some(v8::Boolean::new(scope, captured).into()))
        } else {
            Ok(Some(v8::undefined(scope).into()))
        }
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_setattribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let name = required_string(scope, args, 0)?;
        let converted = trusted_union_string_input(scope, args.get(1))?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let name = normalized_attribute_name(state, node, name)?;
        validate_attribute_local_name(&name)?;
        let required = native_trusted_attribute_type(state, node, &name, None)?;
        let value = match required {
            Some(kind) => trusted_string(
                scope,
                state,
                converted,
                kind,
                &native_trusted_attribute_sink(state, node, &name)?,
            )?,
            None => {
                let string = converted
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                rust_string_preserving_utf16(scope, string)?
            }
        };
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        state.dom.borrow_mut().set_attribute(node, &name, value)?;
        attribute_resource_changed(scope, state, node, None, &name)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_setattributens<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let namespace = if args.get(0).is_null_or_undefined() {
            None
        } else {
            let value = required_string(scope, args, 0)?;
            (!value.is_empty()).then_some(value)
        };
        let qualified_name = required_string(scope, args, 1)?;
        let converted = trusted_union_string_input(scope, args.get(2))?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        validate_node_namespace(
            namespace.as_deref().unwrap_or(""),
            &qualified_name,
            is_valid_attribute_local_name,
        )?;
        let local_name = qualified_name.rsplit(':').next().unwrap_or(&qualified_name);
        let required =
            native_trusted_attribute_type(state, node, local_name, namespace.as_deref())?;
        let value = match required {
            Some(kind) => trusted_string(
                scope,
                state,
                converted,
                kind,
                &native_trusted_attribute_sink(state, node, local_name)?,
            )?,
            None => {
                let string = converted
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                rust_string_preserving_utf16(scope, string)?
            }
        };
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        state.dom.borrow_mut().set_attribute_ns(
            node,
            namespace.clone(),
            qualified_name.clone(),
            value,
        )?;
        attribute_resource_changed(scope, state, node, namespace.as_deref(), local_name)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_removeattribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let name = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let name = normalized_attribute_name(state, node, name)?;
        let present = state.dom.borrow().attribute(node, &name)?.is_some();
        state.dom.borrow_mut().remove_attribute(node, &name)?;
        if present {
            attribute_resource_changed(scope, state, node, None, &name)?;
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_removeattributens<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let namespace = if args.get(0).is_null_or_undefined() {
            None
        } else {
            Some(required_string(scope, args, 0)?)
        };
        let local_name = required_string(scope, args, 1)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        if state
            .dom
            .borrow()
            .attribute_ns(node, namespace.as_deref(), &local_name)?
            .is_some()
        {
            state
                .dom
                .borrow_mut()
                .remove_attribute_ns(node, namespace.as_deref(), &local_name)?;
            attribute_resource_changed(scope, state, node, namespace.as_deref(), &local_name)?;
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_hasattribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let name = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let name = normalized_attribute_name(state, node, name)?;
        let present = state.dom.borrow().attribute(node, &name)?.is_some();
        Ok(Some(v8::Boolean::new(scope, present).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_hasattributens<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let namespace = if args.get(0).is_null_or_undefined() {
            None
        } else {
            Some(required_string(scope, args, 0)?)
        };
        let local_name = required_string(scope, args, 1)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let present = state
            .dom
            .borrow()
            .attribute_ns(node, namespace.as_deref(), &local_name)?
            .is_some();
        Ok(Some(v8::Boolean::new(scope, present).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_hasattributes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let has_attributes = !state.dom.borrow().arena().get(node)?.attributes.is_empty();
        Ok(Some(v8::Boolean::new(scope, has_attributes).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_toggleattribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let name = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let name = normalized_attribute_name(state, node, name)?;
        validate_attribute_local_name(&name)?;
        let present = state.dom.borrow().attribute(node, &name)?.is_some();
        let force = (!args.get(1).is_undefined()).then(|| args.get(1).boolean_value(scope));
        let next = force.unwrap_or(!present);
        if next {
            if !present {
                state.dom.borrow_mut().set_attribute(node, &name, "")?;
            }
        } else if present {
            state.dom.borrow_mut().remove_attribute(node, &name)?;
        }
        if next != present {
            attribute_resource_changed(scope, state, node, None, &name)?;
        }
        Ok(Some(v8::Boolean::new(scope, next).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_queryselector<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let selector = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let found = query_selector_nodes(state, node, &selector, SelectorOperation::First)?;
        optional_node(scope, state, found.first().copied()).map(Some)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_queryselectorall<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let selector = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let found = query_selector_nodes(state, node, &selector, SelectorOperation::All)?;
        node_lists::snapshot(scope, state, &found).map(Some)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let name = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let document = owner_document_for_node(state, node)?;
        let html = state
            .dom
            .borrow()
            .arena()
            .get(document)?
            .document_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.is_html);
        node_lists::elements(
            scope,
            state,
            node,
            node_lists::ElementFilter::Tag { name, html },
        )
        .map(Some)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_element_73<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let namespace = if args.get(0).is_null_or_undefined() {
            None
        } else {
            Some(required_string(scope, args, 0)?)
        };
        let local = required_string(scope, args, 1)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        node_lists::elements(
            scope,
            state,
            node,
            node_lists::ElementFilter::Namespace { namespace, local },
        )
        .map(Some)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_element_74<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let classes = required_string(scope, args, 0)?
            .split_ascii_whitespace()
            .map(str::to_owned)
            .collect();
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let document = owner_document_for_node(state, node)?;
        let quirks = state
            .dom
            .borrow()
            .arena()
            .get(document)?
            .document_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.quirks);
        node_lists::elements(
            scope,
            state,
            node,
            node_lists::ElementFilter::Classes {
                names: classes,
                quirks,
            },
        )
        .map(Some)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_getelementsbyname<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let name = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        node_lists::named(scope, state, node, name).map(Some)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_elementfrompoint<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let x = args
            .get(0)
            .number_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        if !x.is_finite() {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "TypeError".into(),
            });
        }
        let y = args
            .get(1)
            .number_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        if !y.is_finite() {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "TypeError".into(),
            });
        }
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        refresh_layout_in_scope(scope, state)?;
        let hits = native_pointer::hit_test_snapshot(state, node, x, y, true)?;
        let context = state.realm_context.to_local(scope)
            .ok_or(V8BindingError::Allocation("geometry result owner context expired"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let _retained_scope = state.enter_retained_allocation(scope)?;
        if member == "elementFromPoint" {
            optional_node(scope, state, hits.first().copied()).map(Some)
        } else {
            node_array(scope, state, &hits).map(Some)
        }
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_remove<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let parent = state.dom.borrow().parent(node)?;
        let preparation = script_children_changed::parent_change(scope, state, parent)?;
        let result = state.dom.borrow_mut().remove(node);
        preparation.finish_mutation(scope, result)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_matches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let selector = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let found = query_selector_nodes(state, node, &selector, SelectorOperation::Matches)?;
        Ok(Some(v8::Boolean::new(scope, !found.is_empty()).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_closest<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let selector = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let found = query_selector_nodes(state, node, &selector, SelectorOperation::Closest)?;
        optional_node(scope, state, found.first().copied()).map(Some)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_getboundingclientrect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let rects = client_rects_for_node(scope, state, node)?;
        let context = state.realm_context.to_local(scope)
            .ok_or(V8BindingError::Allocation("geometry result owner context expired"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let _retained_scope = state.enter_retained_allocation(scope)?;
        Ok(Some(
            dom_rect_object(scope, bounding_client_rect(&rects))?.into(),
        ))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_getclientrects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let rects = client_rects_for_node(scope, state, node)?;
        let context = state.realm_context.to_local(scope)
            .ok_or(V8BindingError::Allocation("geometry result owner context expired"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let _retained_scope = state.enter_retained_allocation(scope)?;
        let mut values = Vec::new();
        values.try_reserve_exact(rects.len())
            .map_err(|_| V8BindingError::Allocation("Element client rectangle results"))?;
        for rect in rects {
            values.push(dom_rect_object(scope, rect)?.into());
        }
        Ok(Some(rect_lists::wrap(scope, state, &values)?.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_cssstylesheet_insertrule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let rule = required_string(scope, args, 0)?;
        let index = if args.length() > 1 && !args.get(1).is_undefined() {
            args.get(1)
                .uint32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)? as usize
        } else {
            0
        };
        edit_stylesheet_rule_list(
            scope,
            state,
            node,
            BindingRuleListEdit::Insert { index, text: &rule },
        )?;
        Ok(Some(
            v8::Integer::new_from_unsigned(scope, index as u32).into(),
        ))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_cssstylesheet_deleterule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let index = if args.length() > 0 && !args.get(0).is_undefined() {
            args.get(0)
                .uint32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)? as usize
        } else {
            0
        };
        edit_stylesheet_rule_list(scope, state, node, BindingRuleListEdit::Delete { index })?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_cssstylesheet_addrule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let selector = if args.length() > 0 {
            cssom_string(scope, args.get(0), false)?
        } else {
            "undefined".into()
        };
        let declarations = if args.length() > 1 {
            cssom_string(scope, args.get(1), false)?
        } else {
            "undefined".into()
        };
        let index = if args.length() > 2 && !args.get(2).is_undefined() {
            args.get(2)
                .uint32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)? as usize
        } else {
            let current = state.dom.borrow().arena().stylesheet_text(node)?;
            stylesheet_rules_for_owner(scope, state, node, &current, true)?.len()
        };
        let rule = format!("{selector} {{ {declarations} }}");
        edit_stylesheet_rule_list(
            scope,
            state,
            node,
            BindingRuleListEdit::Insert { index, text: &rule },
        )?;
        Ok(Some(v8::Integer::new(scope, -1).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_cssstylesheet_replacesync<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let source = required_string(scope, args, 0)?;
        if !state.constructed_stylesheets.borrow().contains(&node) {
            if member == "replace" {
                let resolver = v8::PromiseResolver::new(scope)
                    .ok_or(V8BindingError::Allocation("stylesheet replacement promise"))?;
                let exception = dom_exception_value(scope, "NotAllowedError")?;
                resolver.reject(scope, exception);
                return Ok(Some(resolver.get_promise(scope).into()));
            }
            return Err(V8BindingError::Unsupported {
                interface: "CSSStyleSheet".into(),
                member: "NotAllowedError".into(),
            });
        }
        // Stylesheet parsing recovers from invalid input. An empty parsed
        // result replaces the previous rules with an empty stylesheet.
        let rules = stylesheet_rules_for_owner(scope, state, node, &source, false)?;
        if let Some(array) = state
            .stylesheet_rule_lists
            .borrow()
            .get(&node)
            .and_then(|array| array.to_local(scope))
        {
            for index in 0..array.length() {
                if let Some(object) = array
                    .get_index(scope, index)
                    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                {
                    page_rules::detach(scope, state, object)?;
                }
            }
        }
        publish_stylesheet_source(state, node, rules.join("\n"))?;
        replace_cached_stylesheet_rules(scope, state, node, &rules)?;
        if member == "replace" {
            let receiver = args.this();
            return Ok(Some(resolved_promise(scope, receiver.into())?.into()));
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_cssstyledeclaration_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let Some(index) = args.get(0).uint32_value(scope) else {
            return Ok(None);
        };
        let names = if let Some(view) = read_inline_declaration(scope, state, node, "", None)? {
            view.names
        } else {
            inline_style_entries(
                state
                    .dom
                    .borrow()
                    .arena()
                    .get(node)?
                    .attributes
                    .get_null("style")
                    .map_or("", String::as_str),
            )
            .into_keys()
            .collect()
        };
        Ok(Some(v8_string(
            scope,
            names.get(index as usize).map_or("", String::as_str),
        )?))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_cssstyledeclaration_getpropertyvalue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let property = required_string(scope, args, 0)?;
        if let Some(view) = read_inline_declaration(scope, state, node, &property, None)? {
            return Ok(Some(v8_string(scope, &view.value)?));
        }
        let property = property.to_ascii_lowercase();
        let dom = state.dom.borrow();
        let style = dom
            .arena()
            .get(node)?
            .attributes
            .get_null("style")
            .cloned()
            .unwrap_or_default();
        let raw_value = inline_style_entries(&style)
            .remove(&property)
            .unwrap_or_default();
        let trimmed = raw_value.trim_end();
        let value = trimmed
            .strip_suffix("!important")
            .unwrap_or(trimmed)
            .trim_end();
        Ok(Some(v8_string(scope, value)?))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_cssstyledeclaration_getpropertypriority<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let property = required_string(scope, args, 0)?;
        if let Some(view) = read_inline_declaration(scope, state, node, &property, None)? {
            return Ok(Some(v8_string(
                scope,
                if view.important { "important" } else { "" },
            )?));
        }
        let property = property.to_ascii_lowercase();
        let dom = state.dom.borrow();
        let style = dom
            .arena()
            .get(node)?
            .attributes
            .get_null("style")
            .cloned()
            .unwrap_or_default();
        let important = inline_style_entries(&style)
            .get(&property)
            .is_some_and(|value| value.trim_end().ends_with("!important"));
        Ok(Some(v8_string(
            scope,
            if important { "important" } else { "" },
        )?))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_cssstyledeclaration_setproperty<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let property = required_string(scope, args, 0)?;
        let value = cssom_string(scope, args.get(1), true)?;
        let priority = if args.length() > 2 && !args.get(2).is_undefined() {
            cssom_string(scope, args.get(2), true)?
        } else {
            String::new()
        };
        mutate_inline_declaration(scope, state, node, &property, &value, &priority)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_cssstyledeclaration_removeproperty<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let property = required_string(scope, args, 0)?;
        let previous = mutate_inline_declaration(scope, state, node, &property, "", "")?;
        Ok(Some(v8_string(scope, &previous)?))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmldialogelement_show<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        dialog::invoke(
            scope,
            state,
            node,
            if member == "showModal" {
                dialog::Action::ShowModal
            } else {
                dialog::Action::Show
            },
            None,
        )?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_requestfullscreen<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let receiver = wrap_node(scope, state, node)?;
        let receiver = v8::Global::new(scope, receiver);
        let options = args.get(0);
        if !options.is_null_or_undefined() {
            let options = options
                .to_object(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            let key = v8_string(scope, "navigationUI")?;
            let value = options
                .get(scope, key)
                .ok_or(V8BindingError::JavaScriptException)?;
            if !value.is_undefined() {
                let value = value
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?
                    .to_rust_string_lossy(scope);
                if !matches!(value.as_str(), "auto" | "show" | "hide") {
                    throw_type_error(scope, "Invalid fullscreen navigationUI");
                    return Err(V8BindingError::JavaScriptException);
                }
            }
        }
        let receiver = v8::Local::new(scope, receiver);
        let (owner, node) = node_identity::argument(scope, receiver.into())?
            .ok_or(V8BindingError::Allocation("fullscreen receiver"))?;
        Ok(Some(fullscreen::promise(scope, &owner, node, false)?))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_document_exitfullscreen<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    Ok(Some(fullscreen::promise(scope, state, node, true)?))
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlelement_showpopover<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let force = if member == "showPopover" {
            Some(true)
        } else if member == "hidePopover" {
            Some(false)
        } else if args.length() == 0 || args.get(0).is_null_or_undefined() {
            None
        } else {
            let value = args.get(0);
            if value.is_object() {
                let object = value
                    .to_object(scope)
                    .ok_or(V8BindingError::Allocation("popover options"))?;
                let key = v8_string(scope, "force")?;
                let force = object
                    .get(scope, key)
                    .ok_or(V8BindingError::Allocation("popover force"))?;
                if force.is_undefined() {
                    None
                } else {
                    Some(force.boolean_value(scope))
                }
            } else {
                Some(value.boolean_value(scope))
            }
        };
        let source = if member != "hidePopover" && args.get(0).is_object() {
            let object = args
                .get(0)
                .to_object(scope)
                .ok_or(V8BindingError::Allocation("popover options"))?;
            let key = v8_string(scope, "source")?;
            let value = object
                .get(scope, key)
                .ok_or(V8BindingError::Allocation("popover source"))?;
            if value.is_null_or_undefined() {
                None
            } else {
                let object = value
                    .try_cast::<v8::Object>()
                    .map_err(|_| V8BindingError::Allocation("popover source Element"))?;
                Some(resolve_receiver(scope, state, object, "Element")?)
            }
        } else {
            None
        };
        let opened = top_layer::toggle(scope, state, node, force, source)?;
        Ok(if member == "togglePopover" {
            Some(v8::Boolean::new(scope, opened).into())
        } else {
            None
        })
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmldialogelement_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let receiver = wrap_node(scope, state, node)?;
        let receiver = v8::Global::new(scope, receiver);
        let value = if args.length() > 0 {
            Some(required_string(scope, args, 0)?)
        } else {
            None
        };
        let receiver = v8::Local::new(scope, receiver);
        let (owner, node) = node_identity::argument(scope, receiver.into())?
            .ok_or(V8BindingError::Allocation("dialog close owner"))?;
        dialog::invoke(
            scope,
            &owner,
            node,
            if member == "requestClose" {
                dialog::Action::RequestClose
            } else {
                dialog::Action::Close
            },
            value,
        )?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlelement_focus<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let target = if state
            .dom
            .borrow()
            .is_element(node, "http://www.w3.org/1999/xhtml", "label")
        {
            let own_focus = {
                let dom = state.dom.borrow();
                dom.parsed_tab_index(node)?.is_some() && !dom.is_inert(node)?
            } && !css_suppresses_focus_in_scope(scope, state, node)?;
            if own_focus {
                Some(node)
            } else {
                let dom = state.dom.borrow();
                dom.label_control(node)?.filter(|control| {
                    !dom.control_disabled(*control).unwrap_or(true)
                        && (matches!(
                            element_local_name(&dom, *control),
                            Some("input" | "button" | "select" | "textarea")
                        ) || dom.parsed_tab_index(*control).ok().flatten().is_some())
                })
            }
        } else {
            Some(node)
        };
        if let Some(target) = target {
            focus_html_element(scope, state, target, false)?;
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlinputelement_htmltextareaelement_97<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        if !state.dom.borrow().supports_select_all(node)? {
            return Ok(None);
        }
        focus_html_element(scope, state, node, false)?;
        // Focus handlers can replace the input state or value.
        if !state.dom.borrow().supports_select_all(node)? {
            return Ok(None);
        }
        if state.dom.borrow().focused() == Some(node)
            && let Some(control) = selection_text::prepare_control(state, node)?
        {
            *state.selected_text_control.borrow_mut() = Some(control);
        }
        let length = oryn_dom::dom_string::len(&state.dom.borrow().control_value(node)?) as u32;
        let next = (0, length, "none".into());
        let before = state
            .text_control_selections
            .borrow()
            .get(&node)
            .cloned()
            .unwrap_or((0, 0, "none".into()));
        if next != before {
            queue_control_select(scope, state, node)?;
            state
                .text_control_selections
                .borrow_mut()
                .insert(node, next);
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlinputelement_htmltextareaelement_98<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let original = wrap_node(scope, state, node)?;
        if args.length() < 2 {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: "TypeError".into(),
            });
        }
        let start = args
            .get(0)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let end = args
            .get(1)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let direction = if args.get(2).is_undefined() {
            "none".into()
        } else {
            args.get(2)
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope)
        };
        let direction = if matches!(direction.as_str(), "forward" | "backward") {
            direction
        } else {
            "none".into()
        };
        let (owner, node) =
            node_identity::argument(scope, original.into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        let dom = state.dom.borrow();
        if !dom.supports_selection_range(node)? {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: "InvalidStateError".into(),
            });
        }
        let length = oryn_dom::dom_string::len(&dom.control_value(node)?) as u32;
        drop(dom);
        let end = end.min(length);
        let next = (start.min(end), end, direction);
        let before = state
            .text_control_selections
            .borrow()
            .get(&node)
            .cloned()
            .unwrap_or((0, 0, "none".into()));
        if next != before {
            queue_control_select(scope, state, node)?;
            state
                .text_control_selections
                .borrow_mut()
                .insert(node, next);
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlelement_blur<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        if let Some(node) = focus_navigation::delegated_blur_target(state, node)? {
            state
                .focus_transition_revision
                .set(state.focus_transition_revision.get().wrapping_add(1));
            state.dom.borrow_mut().focus(None)?;
            commit_control_change(scope, state, node)?;
            let event = create_focus_event_object(scope, state, "blur", None)?;
            dispatch_event(scope, state, node, event)?;
            let event = create_focus_event_object(scope, state, "focusout", None)?;
            dispatch_event(scope, state, node, event)?;
        }
        interest::poll(scope, state)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_scroll<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let requested = scroll_coordinates(scope, args)?;
        perform_scroll_in_scope(scope, state, node, requested, member == "scrollBy")?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_element_scrollintoview<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let value = args.get(0);
        let (block, inline, nearest) = if value.is_object() || value.is_null() {
            let options = if value.is_null() {
                None
            } else {
                Some(
                    value
                        .to_object(scope)
                        .ok_or(V8BindingError::JavaScriptException)?,
                )
            };
            let read =
                |name: &str, default: &str, allowed: &[&str]| -> Result<String, V8BindingError> {
                    let Some(options) = options.as_ref() else {
                        return Ok(default.into());
                    };
                    let key = v8::String::new(scope, name)
                        .ok_or(V8BindingError::Allocation("scroll option"))?;
                    let value = options
                        .get(scope, key.into())
                        .ok_or(V8BindingError::JavaScriptException)?;
                    if value.is_undefined() {
                        return Ok(default.into());
                    }
                    let value = value
                        .to_string(scope)
                        .ok_or(V8BindingError::JavaScriptException)?
                        .to_rust_string_lossy(scope);
                    if !allowed.contains(&value.as_str()) {
                        return Err(V8BindingError::Unsupported {
                            interface: "Element".into(),
                            member: format!("TypeError: invalid scrollIntoView {name}"),
                        });
                    }
                    Ok(value)
                };
            let _behavior = read("behavior", "auto", &["auto", "instant", "smooth"])?;
            let block = read("block", "start", &["start", "center", "end", "nearest"])?;
            let container = read("container", "all", &["all", "nearest"])?;
            let inline = read("inline", "nearest", &["start", "center", "end", "nearest"])?;
            (block, inline, container == "nearest")
        } else {
            (
                (if value.is_undefined() || value.boolean_value(scope) {
                    "start"
                } else {
                    "end"
                })
                .into(),
                "nearest".into(),
                false,
            )
        };
        scroll_element_into_view_in_scope(scope, state, node, &block, &inline, nearest)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlcanvaselement_todataurl<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let callback = if member == "toBlob" {
            Some(
                v8::Local::<v8::Function>::try_from(args.get(0)).map_err(|_| {
                    V8BindingError::Unsupported {
                        interface: "HTMLCanvasElement".into(),
                        member: "TypeError: callback must be callable".into(),
                    }
                })?,
            )
        } else {
            None
        };
        // PNG is the mandatory fallback for unrecognized serialization types.
        let type_index = if callback.is_some() { 1 } else { 0 };
        if args.length() > type_index && !args.get(type_index).is_undefined() {
            let _ = required_string(scope, args, type_index)?;
        }
        let bytes = canvas_png(state, node)?;
        if let Some(callback) = callback {
            state.deferred_platform_tasks.borrow_mut().push_back(
                DeferredPlatformTask::CanvasBlob {
                    callback: v8::Global::new(scope, callback),
                    bytes,
                },
            );
            Ok(None)
        } else {
            let url = bytes.map_or_else(
                || "data:,".into(),
                |bytes| format!("data:image/png;base64,{}", BASE64.encode(bytes)),
            );
            Ok(Some(
                v8::String::new(scope, &url)
                    .ok_or(V8BindingError::Allocation("canvas data URL"))?
                    .into(),
            ))
        }
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlcanvaselement_getcontext<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let context_id = required_string(scope, args, 0)?;
        if context_id != "2d" {
            return Ok(Some(v8::null(scope).into()));
        }
        let owner = wrap_node(scope, state, node)?;
        let cache_name = v8::String::new(scope, "oryn.canvas.context2d")
            .ok_or(V8BindingError::Allocation("canvas context cache"))?;
        let cache_key = v8::Private::for_api(scope, Some(cache_name));
        if let Some(context) = owner
            .get_private(scope, cache_key)
            .filter(|value| value.is_object())
        {
            return Ok(Some(context));
        }
        let template = state
            .instance_templates
            .borrow()
            .get("CanvasRenderingContext2D")
            .and_then(|template| template.to_global(scope))
            .ok_or(V8BindingError::MissingInterface("CanvasRenderingContext2D"))?;
        let context = v8::Local::new(scope, &template)
            .new_instance(scope)
            .ok_or(V8BindingError::Allocation("canvas context"))?;
        let owner_name = v8::String::new(scope, "oryn.canvas.owner")
            .ok_or(V8BindingError::Allocation("canvas context owner"))?;
        let owner_key = v8::Private::for_api(scope, Some(owner_name));
        if context.set_private(scope, owner_key, owner.into()) != Some(true) {
            return Err(V8BindingError::Allocation("canvas context ownership"));
        }
        // A private edge in each direction preserves context identity while
        // either wrapper is reachable, without a native strong GC root.
        if owner.set_private(scope, cache_key, context.into()) != Some(true) {
            return Err(V8BindingError::Allocation("canvas context cache"));
        }
        Ok(Some(context.into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlelement_click<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        if state.dom.borrow().control_disabled(node)? {
            return Ok(None);
        }
        let event = create_synthetic_click_event(scope, state)?;
        if retained_activation::active(state, node)
            || !state.click_in_progress.borrow_mut().insert(node)
        {
            return Ok(None);
        }
        let result = dispatch_event(scope, state, node, event);
        let (owner, node) = node_identity::argument(scope, args.this().into())?
            .ok_or(V8BindingError::Allocation("click target owner"))?;
        let state = &owner;
        state.click_in_progress.borrow_mut().remove(&node);
        if result? {
            permission_elements::activate(state, node, false)?;
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlinputelement_stepup<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let original = wrap_node(scope, state, node)?;
        let count = if args.length() == 0 || args.get(0).is_undefined() {
            1
        } else {
            args.get(0)
                .int32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?
        };
        let (owner, node) =
            node_identity::argument(scope, original.into())?.ok_or(DomError::StaleNode(node))?;
        let state = &owner;
        if !state
            .dom
            .borrow_mut()
            .step_control(node, count, member == "stepDown")?
        {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: "InvalidStateError".into(),
            });
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_setcustomvalidity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let original = wrap_node(scope, state, node)?;
        let message = required_string(scope, args, 0)?;
        let (owner, node) =
            node_identity::argument(scope, original.into())?.ok_or(DomError::StaleNode(node))?;
        owner.dom.borrow_mut().set_custom_validity(node, message)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_checkvalidity_reportvalidity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let valid = validation::validate_control(scope, state, node, member == "reportValidity")?;
        Ok(Some(v8::Boolean::new(scope, valid).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlinputelement_showpicker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        temporal_picker::show(scope, state, node)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlselectelement_showpicker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        select_picker::show(scope, state, node)?;
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlselectelement_remove<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        if args.length() == 0 {
            state.dom.borrow_mut().remove(node)?;
        } else {
            let receiver = wrap_node(scope, state, node)?;
            let index = args
                .get(0)
                .int32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            let (state, node) = node_identity::argument(scope, receiver.into())?
                .ok_or(V8BindingError::Allocation("select remove owner"))?;
            state.dom.borrow_mut().remove_select_option(node, index)?;
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlselectelement_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        if args.length() == 0 {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: format!("{member} requires an argument"),
            });
        }
        let collection =
            node_lists::elements(scope, state, node, node_lists::ElementFilter::Options)?;
        let collection = v8::Local::<v8::Object>::try_from(collection)
            .map_err(|_| V8BindingError::Allocation("options collection"))?;
        if member == "namedItem" {
            node_lists::named_item(scope, state, collection, args.get(0)).map(Some)
        } else {
            node_lists::item(scope, state, collection, "HTMLCollection", args.get(0)).map(Some)
        }
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlselectelement_add<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    node_lists::select_add(scope, state, node, args)
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlformelement_checkvalidity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        let valid = if member == "reportValidity" {
            validation::validate_form(scope, state, node, true)?
        } else {
            validation::check_form(scope, state, node)?
        };
        Ok(Some(v8::Boolean::new(scope, valid).into()))
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlformelement_requestsubmit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        if member == "requestSubmit" {
            let submitter = if args.length() == 0 || args.get(0).is_null_or_undefined() {
                None
            } else {
                Some(argument_node(scope, state, args.get(0))?)
            };
            submissions::request_submit(scope, state, node, submitter)?;
        } else if state.dom.borrow().is_connected(node)? {
            submissions::queue(
                scope,
                state,
                node,
                None,
                submissions::SubmissionSource::Submit,
            )?;
        }
        Ok(None)
    }
}

#[inline(never)]
#[allow(unused_variables)]
pub(super) fn dispatch_htmlformelement_reset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    {
        reset_form(scope, state, node)?;
        Ok(None)
    }
}
