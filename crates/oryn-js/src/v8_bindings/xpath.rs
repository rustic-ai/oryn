//! Native XPath API identity, result lifetime and mutation invalidation.
use super::*;
use oryn_dom::xpath::{Expression, Node as XPathNode, Value as XPathValue};
#[derive(Default)]
pub(super) struct Registry {
    next: u64,
    retained_bytes: usize,
    entries: BTreeMap<u64, Entry>,
}
struct Entry {
    bytes: usize,
    _wrapper: v8::Weak<v8::Object>,
    record: Record,
}
#[derive(Clone)]
enum Record {
    Expression(Rc<Expression>),
    Result(Rc<RefCell<ResultRecord>>),
    Namespace { prefix: String, uri: String },
}
struct ResultRecord {
    dom: SharedDom,
    document: NodeId,
    revision: Rc<()>,
    value: XPathValue,
    kind: u16,
    index: usize,
}
const MAX_RETAINED_BYTES: usize = 64 * 1024 * 1024;
fn record_bytes(record: &Record) -> usize {
    let base = std::mem::size_of::<Entry>();
    base.saturating_add(match record {
        Record::Expression(e) => e.retained_bytes(),
        Record::Namespace { prefix, uri, .. } => prefix.capacity().saturating_add(uri.capacity()),
        Record::Result(result) => {
            let result = result.borrow();
            std::mem::size_of::<ResultRecord>().saturating_add(match &result.value {
                XPathValue::String(s) => s.capacity(),
                XPathValue::Nodes(nodes) => nodes
                    .capacity()
                    .saturating_mul(
                        std::mem::size_of::<XPathNode>() + 2 * std::mem::size_of::<usize>(),
                    )
                    .saturating_add(
                        nodes
                            .iter()
                            .map(|node| match node {
                                XPathNode::Attribute {
                                    namespace, local, ..
                                } => namespace.capacity().saturating_add(local.capacity()),
                                XPathNode::Namespace { prefix, uri, .. } => {
                                    prefix.capacity().saturating_add(uri.capacity())
                                }
                                _ => 0,
                            })
                            .fold(0usize, usize::saturating_add),
                    ),
                _ => 0,
            })
        }
    })
}
fn error(e: oryn_dom::xpath::Error) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "XPathEvaluator".into(),
        member: match e {
            oryn_dom::xpath::Error::Namespace(_) => "NamespaceError".into(),
            oryn_dom::xpath::Error::Type => "TypeError".into(),
            oryn_dom::xpath::Error::Context => "NotSupportedError".into(),
            oryn_dom::xpath::Error::Limit => "QuotaExceededError".into(),
            oryn_dom::xpath::Error::Dom(_) => "InvalidStateError".into(),
            _ => "SyntaxError".into(),
        },
    }
}
fn invalid(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "XPathResult".into(),
        member: name.into(),
    }
}
fn brand(scope: &mut v8::PinScope, object: v8::Local<v8::Object>, expected: &str) -> bool {
    !object.is_proxy()
        && object.internal_field_count() >= 6
        && object
            .get_internal_field(scope, 5)
            .and_then(|v| v8::Local::<v8::String>::try_from(v).ok())
            .is_some_and(|s| s.to_rust_string_lossy(scope) == expected)
}
fn get(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    expected: &str,
) -> Result<Record, V8BindingError> {
    if !brand(scope, object, expected) {
        return Err(invalid("TypeError: incompatible receiver"));
    }
    let id = object
        .get_internal_field(scope, 2)
        .and_then(|v| v8::Local::<v8::Value>::try_from(v).ok())
        .and_then(exact_u64)
        .ok_or_else(|| invalid("TypeError"))?;
    state
        .xpath
        .borrow()
        .entries
        .get(&id)
        .map(|e| e.record.clone())
        .ok_or_else(|| invalid("InvalidStateError"))
}
fn wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    interface: &str,
    record: Record,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let template = state
        .instance_templates
        .borrow()
        .get(interface)
        .and_then(|template| template.to_global(scope))
        .ok_or(V8BindingError::MissingInterface("XPath interface"))?;
    let template = v8::Local::new(scope, &template);
    let object = template
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("XPath instance"))?;
    let bytes = record_bytes(&record);
    let id = {
        let mut registry = state.xpath.borrow_mut();
        if registry.entries.len() >= 65536
            || registry.retained_bytes.saturating_add(bytes) > MAX_RETAINED_BYTES
        {
            return Err(invalid("QuotaExceededError"));
        }
        registry.next = registry
            .next
            .checked_add(1)
            .ok_or_else(|| invalid("QuotaExceededError"))?;
        registry.next
    };
    object.set_internal_field(0, v8::Number::new(scope, 0.).into());
    object.set_internal_field(2, v8::BigInt::new_from_u64(scope, id).into());
    object.set_internal_field(1, v8::BigInt::new_from_u64(scope, state.instance_id).into());
    let tag = v8::String::new(scope, interface).ok_or(V8BindingError::Allocation("XPath brand"))?;
    object.set_internal_field(5, tag.into());
    let platform_id = if interface == "XPathNamespace" {
        let platform_id = register_platform_object(state, PlatformObjectRecord::XPathNamespace);
        bind_platform_object(scope, state, platform_id, object);
        Some(platform_id)
    } else {
        None
    };
    let weak = Rc::downgrade(state);
    let wrapper = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_| {
            if let Some(state) = weak.upgrade() {
                let mut registry = state.xpath.borrow_mut();
                if let Some(entry) = registry.entries.remove(&id) {
                    registry.retained_bytes = registry.retained_bytes.saturating_sub(entry.bytes);
                }
                drop(registry);
                if let Some(platform_id) = platform_id {
                    state.platform_objects.borrow_mut().remove(&platform_id);
                    state.platform_wrappers.borrow_mut().remove(&platform_id);
                    state
                        .platform_listeners
                        .borrow_mut()
                        .retain(|(id, _), _| *id != platform_id);
                }
            }
        }),
    );
    let mut registry = state.xpath.borrow_mut();
    registry.retained_bytes += bytes;
    registry.entries.insert(
        id,
        Entry {
            bytes,
            _wrapper: wrapper,
            record,
        },
    );
    Ok(object)
}
fn resolver_uri(
    scope: &mut v8::PinScope,
    resolver: v8::Local<v8::Value>,
    prefix: &str,
) -> Result<String, V8BindingError> {
    if resolver.is_null_or_undefined() {
        return Err(invalid("NamespaceError"));
    }
    let object = v8::Local::<v8::Object>::try_from(resolver).map_err(|_| invalid("TypeError"))?;
    let context = object
        .get_creation_context(scope)
        .ok_or(V8BindingError::Allocation("XPath resolver realm"))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let (result, exception, message) = {
        v8::tc_scope!(let tc,scope);
        let result = (|| {
            let (function, receiver) =
                if let Ok(function) = v8::Local::<v8::Function>::try_from(resolver) {
                    (function, v8::undefined(tc).into())
                } else {
                    let key = v8::String::new(tc, "lookupNamespaceURI")
                        .ok_or(V8BindingError::Allocation("XPath resolver key"))?;
                    let value = object
                        .get(tc, key.into())
                        .ok_or(V8BindingError::JavaScriptException)?;
                    let function = v8::Local::<v8::Function>::try_from(value).map_err(|_| {
                        throw_type_error(tc, "XPathNSResolver.lookupNamespaceURI must be callable");
                        V8BindingError::JavaScriptException
                    })?;
                    (function, object.into())
                };
            let prefix = v8_string(tc, prefix)?;
            let value = function
                .call(tc, receiver, &[prefix])
                .ok_or(V8BindingError::JavaScriptException)?;
            if value.is_null_or_undefined() {
                return Err(invalid("NamespaceError"));
            }
            let value = value
                .to_string(tc)
                .ok_or(V8BindingError::JavaScriptException)?;
            if value.length() > 1024 * 1024 {
                return Err(invalid("QuotaExceededError"));
            }
            rust_string_preserving_utf16(tc, value)
        })();
        let exception = tc.exception();
        let message = tc.message();
        if !tc.is_execution_terminating() {
            tc.reset();
        }
        (result, exception, message)
    };
    if scope.is_execution_terminating() {
        return Err(V8BindingError::JavaScriptException);
    }
    if let Some(exception) = exception {
        // DOM XPath reports callback exceptions in the resolver's realm and
        // then fails namespace resolution in the calling evaluator's realm.
        global_events::report_script_exception(scope, exception, message, false)?;
        return Err(invalid("NamespaceError"));
    }
    result
}
fn compile(
    scope: &mut v8::PinScope,
    source: &str,
    resolver: v8::Local<v8::Value>,
) -> Result<Expression, V8BindingError> {
    if !resolver.is_null_or_undefined() && !resolver.is_object() {
        return Err(invalid("TypeError"));
    }
    let mut callback_error = None;
    let expression = Expression::compile(source, &mut |prefix| {
        if prefix == "xml" {
            return Ok("http://www.w3.org/XML/1998/namespace".into());
        }
        match resolver_uri(scope, resolver, prefix) {
            Ok(uri) => Ok(uri),
            Err(e) => {
                callback_error = Some(e);
                Err(oryn_dom::xpath::Error::Namespace(prefix.into()))
            }
        }
    });
    if let Some(error) = callback_error {
        return Err(error);
    }
    expression.map_err(error)
}
fn result_kind(value: &XPathValue, requested: u16) -> Result<u16, V8BindingError> {
    if requested > 9 {
        return Err(invalid("NotSupportedError"));
    }
    if requested == 0 {
        return Ok(match value {
            XPathValue::Number(_) => 1,
            XPathValue::String(_) => 2,
            XPathValue::Boolean(_) => 3,
            XPathValue::Nodes(_) => 4,
        });
    }
    if requested >= 4 && !matches!(value, XPathValue::Nodes(_)) {
        return Err(invalid("TypeError"));
    }
    Ok(requested)
}
fn node_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    dom: &SharedDom,
    node: &XPathNode,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    // The anchor array stores the actual Attr at evaluation, not a later lookup.
    match node {
        XPathNode::Dom(id) => Ok(wrap_node(scope, state, *id)?.into()),
        XPathNode::Attribute {
            owner,
            namespace,
            local,
        } => {
            let id = dom
                .borrow_mut()
                .attribute_node_ns(
                    *owner,
                    (!namespace.is_empty()).then_some(namespace.as_str()),
                    local,
                )?
                .ok_or_else(|| invalid("InvalidStateError"))?;
            Ok(wrap_node(scope, state, id)?.into())
        }
        XPathNode::Namespace { owner, prefix, uri } => {
            let owner = wrap_node(scope, state, *owner)?;
            let key = anchors_key(scope)?;
            let states = scope
                .get_slot::<BindingStateRegistry>()
                .map(|registry| {
                    registry
                        .instances
                        .borrow()
                        .values()
                        .filter_map(RcWeak::upgrade)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec![state.clone()]);
            for candidate in states {
                let objects = candidate
                    .xpath
                    .borrow()
                    .entries
                    .values()
                    .filter_map(|entry| match &entry.record {
                        Record::Namespace { prefix: p, uri: u } if p == prefix && u == uri => {
                            entry._wrapper.to_local(scope)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                for object in objects {
                    if object
                        .get_private(scope, key)
                        .is_some_and(|anchor| anchor.strict_equals(owner.into()))
                    {
                        return Ok(object.into());
                    }
                }
            }
            let object = wrap(
                scope,
                state,
                "XPathNamespace",
                Record::Namespace {
                    prefix: prefix.clone(),
                    uri: uri.clone(),
                },
            )?;
            object
                .set_private(scope, key, owner.into())
                .ok_or(V8BindingError::JavaScriptException)?;
            Ok(object.into())
        }
    }
}
fn anchors_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let key = v8::String::new(scope, "oryn.xpath.anchors")
        .ok_or(V8BindingError::Allocation("XPath anchors"))?;
    Ok(v8::Private::for_api(scope, Some(key)))
}
fn result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    owner: &Rc<V8BindingState>,
    context: XPathNode,
    expression: &Expression,
    requested: u16,
    reuse: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let mut value = expression
        .evaluate_node(&owner.dom.borrow(), context.clone())
        .map_err(error)?;
    let kind = result_kind(&value, requested)?;
    value = match kind {
        1 => XPathValue::Number(value.number(&owner.dom.borrow()).map_err(error)?),
        2 => XPathValue::String(value.string(&owner.dom.borrow()).map_err(error)?),
        3 => XPathValue::Boolean(value.boolean()),
        _ => value,
    };
    let document = owner
        .dom
        .borrow()
        .arena()
        .get(context.owner())?
        .owner_document
        .unwrap_or(context.owner());
    let revision = owner.dom.borrow_mut().xpath_document_revision(document)?;
    let replacement = Rc::new(RefCell::new(ResultRecord {
        dom: owner.dom.clone(),
        document,
        revision,
        value,
        kind,
        index: 0,
    }));
    let native_record = Record::Result(replacement.clone());
    let bytes = record_bytes(&native_record);
    // Reject before allocating one V8 handle (or materializing an Attr) per node.
    let target = if reuse.is_null_or_undefined() {
        if state.xpath.borrow().retained_bytes.saturating_add(bytes) > MAX_RETAINED_BYTES {
            return Err(invalid("QuotaExceededError"));
        }
        None
    } else {
        let object = v8::Local::<v8::Object>::try_from(reuse).map_err(|_| invalid("TypeError"))?;
        let target_state =
            binding_state_for_receiver(scope, object).ok_or_else(|| invalid("TypeError"))?;
        let Record::Result(record) = get(scope, &target_state, object, "XPathResult")? else {
            return Err(invalid("TypeError"));
        };
        let id = object
            .get_internal_field(scope, 2)
            .and_then(|v| v8::Local::<v8::Value>::try_from(v).ok())
            .and_then(exact_u64)
            .ok_or_else(|| invalid("TypeError"))?;
        {
            let registry = target_state.xpath.borrow();
            let old = registry
                .entries
                .get(&id)
                .ok_or_else(|| invalid("InvalidStateError"))?;
            if registry
                .retained_bytes
                .saturating_sub(old.bytes)
                .saturating_add(bytes)
                > MAX_RETAINED_BYTES
            {
                return Err(invalid("QuotaExceededError"));
            }
        }
        Some((object, target_state, id, record))
    };
    let mut handles = Vec::new();
    {
        let mut record = replacement.borrow_mut();
        if let XPathValue::Nodes(nodes) = &mut record.value {
            handles.reserve(nodes.len());
            for node in nodes.iter_mut() {
                if let XPathNode::Attribute {
                    owner: element,
                    namespace,
                    local,
                } = node
                {
                    let id = owner
                        .dom
                        .borrow_mut()
                        .attribute_node_ns(
                            *element,
                            (!namespace.is_empty()).then_some(namespace.as_str()),
                            local,
                        )?
                        .ok_or_else(|| invalid("InvalidStateError"))?;
                    *node = XPathNode::Dom(id);
                }
                handles.push(node_value(scope, owner, &owner.dom, node)?);
            }
        }
    }
    let array = v8::Array::new_with_elements(scope, &handles);
    let key = anchors_key(scope)?;
    if let Some((object, target_state, id, target_record)) = target {
        // No externally visible record change until every fallible V8 allocation
        // and private-anchor installation has succeeded. GC can only release
        // other entries while these native records and local wrappers are live.
        let bytes = record_bytes(&native_record);
        {
            let registry = target_state.xpath.borrow();
            let old = registry
                .entries
                .get(&id)
                .ok_or_else(|| invalid("InvalidStateError"))?;
            if registry
                .retained_bytes
                .saturating_sub(old.bytes)
                .saturating_add(bytes)
                > MAX_RETAINED_BYTES
            {
                return Err(invalid("QuotaExceededError"));
            }
        }
        object
            .set_private(scope, key, array.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        let mut registry = target_state.xpath.borrow_mut();
        let old = registry
            .entries
            .get(&id)
            .expect("live XPath reuse wrapper")
            .bytes;
        registry.retained_bytes = registry.retained_bytes - old + bytes;
        std::mem::swap(
            &mut *target_record.borrow_mut(),
            &mut *replacement.borrow_mut(),
        );
        registry.entries.get_mut(&id).unwrap().bytes = bytes;
        Ok(object.into())
    } else {
        let object = wrap(scope, state, "XPathResult", native_record)?;
        object
            .set_private(scope, key, array.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        Ok(object.into())
    }
}
fn invalidated(record: &ResultRecord) -> bool {
    !record
        .dom
        .borrow()
        .xpath_document_revision_matches(record.document, &record.revision)
}

fn anchor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    index: usize,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = anchors_key(scope)?;
    let value = object
        .get_private(scope, key)
        .ok_or_else(|| invalid("InvalidStateError"))?;
    let array =
        v8::Local::<v8::Array>::try_from(value).map_err(|_| invalid("InvalidStateError"))?;
    if index >= array.length() as usize {
        Ok(v8::null(scope).into())
    } else {
        array
            .get_index(scope, index as u32)
            .ok_or(V8BindingError::JavaScriptException)
    }
}

fn namespace_identity(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<(Rc<V8BindingState>, XPathNode), V8BindingError> {
    let state = binding_state_for_receiver(scope, object).ok_or_else(|| invalid("TypeError"))?;
    let Record::Namespace { prefix, uri } = get(scope, &state, object, "XPathNamespace")? else {
        return Err(invalid("TypeError"));
    };
    let key = anchors_key(scope)?;
    let anchor = object
        .get_private(scope, key)
        .ok_or_else(|| invalid("InvalidStateError"))?;
    let (owner, node) =
        node_identity::argument(scope, anchor)?.ok_or_else(|| invalid("InvalidStateError"))?;
    Ok((
        owner,
        XPathNode::Namespace {
            owner: node,
            prefix,
            uri,
        },
    ))
}
fn context_identity(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<(Rc<V8BindingState>, XPathNode), V8BindingError> {
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && is_namespace(scope, object)
    {
        return namespace_identity(scope, object);
    }
    let (state, node) = node_identity::canonical(scope, value)?
        .ok_or_else(|| invalid("TypeError: context must be a Node"))?;
    Ok((state, XPathNode::Dom(node)))
}

pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let receiver_stamp = (
        state.document_generation(),
        state.document_stream_epoch(),
        state.execution_retired(),
    );
    if interface == "XPathResult" {
        let Record::Result(record) = get(scope, state, args.this(), interface)? else {
            return Err(invalid("TypeError"));
        };
        if member == "iterateNext" {
            let index = {
                let record = record.borrow();
                if !matches!(record.kind, 4 | 5) {
                    return Err(invalid("TypeError"));
                }
                if invalidated(&record) {
                    return Err(invalid("InvalidStateError"));
                }
                record.index
            };
            let value = anchor(scope, args.this(), index)?;
            if !value.is_null() {
                record.borrow_mut().index = index.saturating_add(1);
            }
            return Ok(value);
        }
        if member == "snapshotItem" {
            let index = args
                .get(0)
                .uint32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            if !matches!(record.borrow().kind, 6 | 7) {
                return Err(invalid("TypeError"));
            }
            return anchor(scope, args.this(), index as usize);
        }
        return Err(invalid("TypeError"));
    }
    if interface == "Document" {
        resolve_receiver(scope, state, args.this(), "Document")?;
    } else if interface == "XPathEvaluator" && !brand(scope, args.this(), interface) {
        return Err(invalid("TypeError: incompatible evaluator receiver"));
    }
    if member == "createNSResolver" {
        let _ = context_identity(scope, args.get(0))?;
        return Ok(args.get(0));
    }
    if interface == "XPathExpression" {
        get(scope, state, args.this(), interface)?;
    }
    let source = if interface == "XPathExpression" {
        None
    } else {
        let source = args
            .get(0)
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        if source.length() > 1024 * 1024 {
            return Err(invalid("QuotaExceededError"));
        }
        Some(rust_string_preserving_utf16(scope, source)?)
    };
    if member == "createExpression" {
        let expression = compile(scope, source.as_deref().unwrap(), args.get(1))?;
        if receiver_stamp
            != (
                state.document_generation(),
                state.document_stream_epoch(),
                state.execution_retired(),
            )
        {
            return Err(invalid("InvalidStateError"));
        }
        return Ok(wrap(
            scope,
            state,
            "XPathExpression",
            Record::Expression(Rc::new(expression)),
        )?
        .into());
    }
    let (context_index, kind_index, reuse_index) = if interface == "XPathExpression" {
        (0, 1, 2)
    } else {
        (1, 3, 4)
    };
    let context_value = args.get(context_index);
    let (owner, _context) = context_identity(scope, context_value)?;
    let context_stamp = (
        owner.document_generation(),
        owner.document_stream_epoch(),
        owner.execution_retired(),
    );
    if interface != "XPathExpression"
        && !args.get(2).is_null_or_undefined()
        && !args.get(2).is_object()
    {
        return Err(invalid("TypeError"));
    }
    let requested = if args.get(kind_index).is_undefined() {
        0
    } else {
        args.get(kind_index)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)? as u16
    };
    let reuse = args.get(reuse_index);
    if !reuse.is_null_or_undefined() {
        let object = v8::Local::<v8::Object>::try_from(reuse).map_err(|_| invalid("TypeError"))?;
        let result_state =
            binding_state_for_receiver(scope, object).ok_or_else(|| invalid("TypeError"))?;
        if !matches!(
            get(scope, &result_state, object, "XPathResult")?,
            Record::Result(_)
        ) {
            return Err(invalid("TypeError"));
        }
    }
    let expression = if interface == "XPathExpression" {
        let Record::Expression(expression) = get(scope, state, args.this(), interface)? else {
            return Err(invalid("TypeError"));
        };
        expression
    } else {
        Rc::new(compile(scope, source.as_deref().unwrap(), args.get(2))?)
    };
    // Retiring execution services does not revoke immutable expressions or
    // retained DOM/results. Reject a lifecycle transition during this operation,
    // while allowing a later pure query of an already-retained document. Resolver
    // callbacks and numeric conversion can replace/adopt the original context.
    if receiver_stamp
        != (
            state.document_generation(),
            state.document_stream_epoch(),
            state.execution_retired(),
        )
        || context_stamp
            != (
                owner.document_generation(),
                owner.document_stream_epoch(),
                owner.execution_retired(),
            )
    {
        return Err(invalid("InvalidStateError"));
    }
    let (owner, context) = context_identity(scope, context_value)?;
    result(scope, state, &owner, context, &expression, requested, reuse)
}
pub(super) fn getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let Record::Result(record) = get(scope, state, object, "XPathResult")? else {
        return Err(invalid("TypeError"));
    };
    let record = record.borrow();
    match member {
        "resultType" => Ok(v8::Integer::new_from_unsigned(scope, record.kind as u32).into()),
        "invalidIteratorState" => Ok(v8::Boolean::new(
            scope,
            matches!(record.kind, 4 | 5) && invalidated(&record),
        )
        .into()),
        "numberValue" if record.kind == 1 => {
            let XPathValue::Number(n) = record.value else {
                return Err(invalid("TypeError"));
            };
            Ok(v8::Number::new(scope, n).into())
        }
        "stringValue" if record.kind == 2 => {
            let XPathValue::String(ref s) = record.value else {
                return Err(invalid("TypeError"));
            };
            v8_string(scope, s)
        }
        "booleanValue" if record.kind == 3 => {
            Ok(v8::Boolean::new(scope, record.value.boolean()).into())
        }
        "singleNodeValue" if matches!(record.kind, 8 | 9) => {
            drop(record);
            anchor(scope, object, 0)
        }
        "snapshotLength" if matches!(record.kind, 6 | 7) => {
            let XPathValue::Nodes(ref nodes) = record.value else {
                return Err(invalid("TypeError"));
            };
            Ok(v8::Number::new(scope, nodes.len() as f64).into())
        }
        _ => Err(invalid("TypeError")),
    }
}
pub(super) fn is_non_event_target(scope: &mut v8::PinScope, object: v8::Local<v8::Object>) -> bool {
    ["XPathEvaluator", "XPathExpression", "XPathResult"]
        .iter()
        .any(|name| brand(scope, object, name))
}
pub(super) fn is_namespace(scope: &mut v8::PinScope, object: v8::Local<v8::Object>) -> bool {
    brand(scope, object, "XPathNamespace")
}
pub(super) fn namespace_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let (current, namespace) = namespace_identity(scope, object)?;
    let state = &current;
    let dom = &state.dom;
    let XPathNode::Namespace { owner, prefix, uri } = namespace else {
        return Err(invalid("TypeError"));
    };
    match member {
        "baseURI" => {
            let document = owner_document_for_node(state, owner)?;
            let base = if document == state.document_root() {
                document_base_url(scope, state, true)?
            } else {
                retained_document_base_url(state, document)?
            };
            v8_string(scope, &base)
        }
        "ownerElement" => Ok(wrap_node(scope, state, owner)?.into()),
        "nodeType" => Ok(v8::Integer::new(scope, 13).into()),
        "nodeName" => v8_string(scope, "#namespace"),
        "nodeValue" | "textContent" | "namespaceURI" => v8_string(scope, &uri),
        "prefix" | "localName" => v8_string(scope, &prefix),
        "ownerDocument" => {
            let document = dom.borrow().arena().get(owner)?.owner_document;
            optional_node(scope, state, document)
        }
        "parentNode" | "parentElement" | "firstChild" | "lastChild" | "previousSibling"
        | "nextSibling" => Ok(v8::null(scope).into()),
        "isConnected" => Ok(v8::Boolean::new(scope, dom.borrow().is_connected(owner)?).into()),
        "childNodes" => {
            let name = v8::String::new(scope, "oryn.xpath.empty.children")
                .ok_or(V8BindingError::Allocation("namespace children"))?;
            let key = v8::Private::for_api(scope, Some(name));
            if let Some(value) = object.get_private(scope, key)
                && !value.is_undefined()
            {
                return Ok(value);
            }
            let value = node_lists::snapshot(scope, state, &[])?;
            object
                .set_private(scope, key, value)
                .ok_or(V8BindingError::JavaScriptException)?;
            Ok(value)
        }
        _ => Err(invalid("NotSupportedError: XPathNamespace property")),
    }
}

pub(super) fn namespace_argument_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    if !matches!(
        member,
        "compareDocumentPosition"
            | "isSameNode"
            | "isEqualNode"
            | "contains"
            | "appendChild"
            | "insertBefore"
            | "replaceChild"
            | "removeChild"
            | "adoptNode"
            | "importNode"
    ) {
        return Ok(None);
    }
    let Ok(object) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return Ok(None);
    };
    if !is_namespace(scope, object) {
        return Ok(None);
    }
    let node = resolve_receiver(scope, state, args.this(), "Node")?;
    let (other_state, other) = context_identity(scope, args.get(0))?;
    match member {
        "appendChild" | "insertBefore" | "replaceChild" => {
            return Err(invalid("HierarchyRequestError"));
        }
        "removeChild" => return Err(invalid("NotFoundError")),
        "adoptNode" | "importNode" => return Err(invalid("NotSupportedError")),
        _ => {}
    }
    if member != "compareDocumentPosition" {
        return Ok(Some(v8::Boolean::new(scope, false).into()));
    }
    let this = XPathNode::Dom(node);
    let first = (Rc::as_ptr(&state.dom) as usize, &this);
    let second = (Rc::as_ptr(&other_state.dom) as usize, &other);
    Ok(Some(
        v8::Integer::new(scope, 1 | 32 | if first < second { 4 } else { 2 }).into(),
    ))
}

pub(super) fn namespace_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _state: &Rc<V8BindingState>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let (current, namespace) = namespace_identity(scope, args.this())?;
    let state = &current;
    let dom = &state.dom;
    let XPathNode::Namespace { owner, prefix, uri } = namespace else {
        return Err(invalid("TypeError"));
    };
    match member {
        "hasChildNodes" => Ok(v8::Boolean::new(scope, false).into()),
        "getRootNode" => Ok(args.this().into()),
        "isSameNode" | "isEqualNode" | "contains" => {
            let same = if let Ok(other) = v8::Local::<v8::Object>::try_from(args.get(0)) {
                if is_namespace(scope, other) {
                    let (other_state, other) = namespace_identity(scope, other)?;
                    match other {
                        XPathNode::Namespace {
                            owner: other_owner,
                            prefix: other_prefix,
                            uri: other_uri,
                        } => {
                            (member == "isEqualNode"
                                || (Rc::ptr_eq(dom, &other_state.dom) && owner == other_owner))
                                && prefix == other_prefix
                                && uri == other_uri
                        }
                        _ => false,
                    }
                } else {
                    false
                }
            } else {
                false
            };
            Ok(v8::Boolean::new(scope, same).into())
        }
        "compareDocumentPosition" => {
            let (other_state, other) = context_identity(scope, args.get(0))?;
            let this = XPathNode::Namespace {
                owner,
                prefix: prefix.clone(),
                uri: uri.clone(),
            };
            let same = Rc::ptr_eq(dom, &other_state.dom) && this == other;
            let position = if same {
                0
            } else {
                let first = (Rc::as_ptr(dom) as usize, &this);
                let second = (Rc::as_ptr(&other_state.dom) as usize, &other);
                1 | 32 | if first < second { 4 } else { 2 }
            };
            Ok(v8::Integer::new(scope, position).into())
        }
        "lookupPrefix" | "lookupNamespaceURI" | "isDefaultNamespace" => {
            let value = if args.get(0).is_null_or_undefined() {
                None
            } else {
                Some(required_string(scope, args, 0)?)
            };
            let (current, namespace) = namespace_identity(scope, args.this())?;
            let dom = &current.dom;
            let owner = namespace.owner();
            if member == "isDefaultNamespace" {
                return Ok(v8::Boolean::new(
                    scope,
                    dom.borrow().is_default_namespace(owner, value.as_deref())?,
                )
                .into());
            }
            let result = if member == "lookupPrefix" {
                dom.borrow().lookup_prefix(owner, value.as_deref())?
            } else {
                dom.borrow().lookup_namespace_uri(owner, value.as_deref())?
            };
            match result {
                Some(s) => v8_string(scope, &s),
                None => Ok(v8::null(scope).into()),
            }
        }
        "normalize" => Ok(v8::undefined(scope).into()),
        "appendChild" | "insertBefore" | "replaceChild" => Err(invalid("HierarchyRequestError")),
        "removeChild" => Err(invalid("NotFoundError")),
        "cloneNode" => Err(invalid("NotSupportedError")),
        _ => Err(invalid("NotSupportedError")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) -> String {
        v8::tc_scope!(let scope, scope);
        let source = v8::String::new(scope, source).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap_or_else(|| {
            let exception = scope
                .exception()
                .map(|value| value.to_rust_string_lossy(scope));
            panic!("XPath fixture compilation failed: {exception:?}");
        });
        let result = script.run(scope).unwrap_or_else(|| {
            let exception = scope
                .exception()
                .map(|value| value.to_rust_string_lossy(scope));
            let stack = scope
                .stack_trace()
                .map(|value| value.to_rust_string_lossy(scope));
            panic!("XPath fixture failed: {exception:?}; stack: {stack:?}");
        });
        result.to_rust_string_lossy(scope)
    }
    #[test]
    fn xpath_context_namespace_and_result_objects_follow_callback_adoption() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let source = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                html::parse("<body></body>").into_runtime().unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(121), 1).unwrap()
        };
        let target = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                html::parse("<body></body>").into_runtime().unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(122), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let foreign = wrap_node(scope, &target, target.document.get()).unwrap();
        let key = v8::String::new(scope, "foreignDocument").unwrap();
        first
            .global(scope)
            .set(scope, key.into(), foreign.into())
            .unwrap();
        assert_eq!(
            eval(
                scope,
                r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)};
          for(const phase of ['getter','callback','conversion','type']){
            const xml=new DOMParser().parseFromString('<r xmlns:p="urn:p"><p:item a="old"/></r>','text/xml'),root=xml.documentElement,item=root.firstChild;
            let moved=false;const move=()=>{if(!moved){moved=true;foreignDocument.adoptNode(root)}};
            const resolver={get lookupNamespaceURI(){if(phase==='getter')move();return ()=>{if(phase==='callback')move();return {toString(){if(phase==='conversion')move();return 'urn:p'}}}}};
            const kind={valueOf(){if(phase==='type')move();return 7}};
            const result=document.evaluate('descendant::p:item',root,resolver,kind);
            check(result.snapshotLength===1&&result.snapshotItem(0)===item,'post-callback context '+phase);
            const attr=item.getAttributeNode('a');const attrs=foreignDocument.evaluate('@a',item,null,7);
            const namespace=foreignDocument.evaluate('namespace::p',root,null,9).singleNodeValue,prototype=Object.getPrototypeOf(namespace);
            const iterator=foreignDocument.evaluate('descendant::p:item',root,()=> 'urn:p',5);
            document.adoptNode(root);check(iterator.invalidIteratorState,'detached adopted context invalidates source iterator');try{iterator.iterateNext();throw Error('moved iterator accepted')}catch(e){check(e.name==='InvalidStateError','moved iterator error')}
            check(result.snapshotItem(0)===item&&attrs.snapshotItem(0)===attr,'retained node/attr result');
            check(namespace.ownerElement===root&&namespace.ownerDocument===document,'namespace current owner');
            check(document.evaluate('.',namespace,null,9).singleNodeValue===namespace,'namespace cached object after adoption');
            check(Object.getPrototypeOf(namespace)===prototype&&namespace.lookupNamespaceURI('p')==='urn:p','namespace prototype/lookup');
            namespace.lookupNamespaceURI({toString(){foreignDocument.adoptNode(root);return 'p'}});
            check(namespace.ownerDocument===foreignDocument&&namespace.ownerElement===root,'namespace lookup conversion adoption');
            check(foreignDocument.evaluate('namespace::p',root,null,9).singleNodeValue===namespace,'namespace repeated migration');
          }
          return 'ok';
        })()"#
            ),
            "ok"
        );
        drop(source);
    }
    #[test]
    fn retired_document_retains_xpath_snapshots_expressions_and_namespace_identity() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let foreign = v8::Context::new(scope, Default::default());
        let (owner, values) = {
            let scope = &mut v8::ContextScope::new(scope, foreign);
            let dom = Rc::new(RefCell::new(
                html::parse("<body></body>").into_runtime().unwrap(),
            ));
            let owner = install_generated_bindings(scope, foreign, dom, RealmId(188), 1).unwrap();
            let source=v8::String::new(scope,r#"(()=>{
              const xml=new DOMParser().parseFromString('<r xmlns:p="urn:p"><p:item/></r>','text/xml');
              return [xml,xml.createExpression('descendant::p:item',()=> 'urn:p'),xml.evaluate('//p:item',xml,()=> 'urn:p',7),xml.evaluate('namespace::p',xml.documentElement,null,9).singleNodeValue,xml.evaluate('//p:item',xml,()=> 'urn:p',5)];
            })()"#).unwrap();
            let value = v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap();
            (owner, v8::Global::new(scope, value))
        };
        owner.retire_execution();
        let local = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, local);
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        install_generated_bindings(scope, local, dom, RealmId(189), 1).unwrap();
        let name = v8::String::new(scope, "retainedXPath").unwrap();
        let values = v8::Local::new(scope, &values);
        local.global(scope).set(scope, name.into(), values).unwrap();
        assert_eq!(
            eval(
                scope,
                r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)},[xml,expression,snapshot,namespace,iterator]=retainedXPath;
          const item=xml.documentElement.firstChild,prototype=Object.getPrototypeOf(snapshot);
          check(snapshot.resultType===7&&snapshot.snapshotItem(0)===item,'retained snapshot');
          check(namespace.ownerElement===xml.documentElement&&namespace.ownerDocument===xml&&namespace.namespaceURI==='urn:p','retained namespace slots');
          check(namespace.lookupNamespaceURI('p')==='urn:p','retained namespace lookup');
          check(expression.evaluate(xml,7).snapshotItem(0)===item,'immutable expression retained query');
          const live=document.createElementNS('urn:p','p:item');document.body.append(live);
          check(expression.evaluate(document,7).snapshotItem(0)===live,'retired expression active context');
          check(document.evaluate('.',namespace,null,9).singleNodeValue===namespace,'retained namespace identity');
          check(iterator.iterateNext()===item&&!iterator.invalidIteratorState,'retirement alone does not invalidate iterator');
          item.remove();check(iterator.invalidIteratorState,'retained document mutation still invalidates');
          check(snapshot.snapshotItem(0)===item&&Object.getPrototypeOf(snapshot)===prototype,'snapshot roots survive mutation');
          return 'ok';
        })()"#
            ),
            "ok"
        );
    }
    #[test]
    fn snapshot_index_conversion_observes_reused_result_without_stale_anchors() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            html::parse("<body><div id=a></div><span id=b></span></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(187), 1).unwrap();
        assert_eq!(
            eval(
                scope,
                r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)},a=document.getElementById('a'),b=document.getElementById('b');
          const result=document.evaluate('//div',document,null,7),prototype=Object.getPrototypeOf(result);result.marker=19;
          let conversions=0;
          check(result.snapshotItem({valueOf(){conversions++;check(document.evaluate('//span',document,null,7,result)===result,'reuse identity');return 0}})===b,'conversion uses replacement anchors');
          check(conversions===1&&result.marker===19&&Object.getPrototypeOf(result)===prototype,'snapshot wrapper preservation');
          let caught=false;try{result.snapshotItem({valueOf(){document.evaluate('true()',document,null,3,result);return 0}})}catch(error){caught=true;check(error instanceof TypeError,'replacement result kind')}
          check(caught&&result.booleanValue,'conversion observes latest kind');
          document.evaluate('//div',document,null,7,result);const marker={};caught=false;
          try{result.snapshotItem({valueOf(){throw marker}})}catch(error){caught=true;check(error===marker,'original index conversion exception')}
          check(caught&&result.snapshotItem(0)===a,'throw leaves reusable snapshot intact');
          check(result.snapshotItem(4294967296)===a&&result.snapshotItem(-1)===null,'unsigned long index conversion');
          let touched=false;try{XPathResult.prototype.snapshotItem.call({}, {valueOf(){touched=true;return 0}})}catch(error){check(error instanceof TypeError,'receiver brand')}
          check(!touched,'receiver brand before conversion');return 'ok';
        })()"#
            ),
            "ok"
        );
    }
    #[test]
    fn resolver_errors_report_in_resolver_realm_before_namespace_failure() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let foreign = v8::Context::new(scope, Default::default());
        let values = {
            let scope = &mut v8::ContextScope::new(scope, foreign);
            let dom = Rc::new(RefCell::new(
                html::parse("<body></body>").into_runtime().unwrap(),
            ));
            let _state = install_generated_bindings(scope, foreign, dom, RealmId(123), 1).unwrap();
            let code=v8::String::new(scope,r#"(()=>{const errors=[],marker={};addEventListener('error',e=>{errors.push(e.error);e.preventDefault()});return [{},{get lookupNamespaceURI(){throw marker}},()=>({toString(){throw marker}}),errors,marker,TypeError]})()"#).unwrap();
            let values = v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap();
            v8::Global::new(scope, values)
        };
        let local = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, local);
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let _state = install_generated_bindings(scope, local, dom, RealmId(124), 1).unwrap();
        let name = v8::String::new(scope, "foreignResolvers").unwrap();
        let values = v8::Local::new(scope, &values);
        local.global(scope).set(scope, name.into(), values).unwrap();
        assert_eq!(
            eval(
                scope,
                r#"(()=>{const [empty,getter,convert,errors,marker,ForeignTypeError]=foreignResolvers;let localErrors=0;addEventListener('error',()=>localErrors++);
          for(const resolver of [empty,getter,convert]){let caught=false;try{document.evaluate('//p:item',document,resolver)}catch(e){caught=true;if(e.name!=='NamespaceError'||!(e instanceof DOMException))throw Error('caller exception realm')}if(!caught)throw Error('missing namespace failure')}
          if(errors.length!==3||!(errors[0] instanceof ForeignTypeError)||errors[1]!==marker||errors[2]!==marker||localErrors)throw Error('resolver report realm/identity');
          for(const value of [undefined,null]){try{document.evaluate('//p:item',document,()=>value)}catch(e){if(e.name!=='NamespaceError')throw e}}
          if(errors.length!==3||localErrors)throw Error('null namespace reported as exception');return 'ok';})()"#
            ),
            "ok"
        );
    }
    #[test]
    fn native_xpath_result_conversion_identity_reuse_and_invalidation() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><ul><li id=a n=1>A</li><li id=b n=2>B</li></ul></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert_eq!(
            eval(
                scope,
                r#"(()=>{const check=(v,m)=>{if(!v)throw Error(m)};
            const a=document.getElementById('a'),b=document.getElementById('b');
            const iterator=document.evaluate('//li',document,null,XPathResult.ORDERED_NODE_ITERATOR_TYPE);
            check(iterator instanceof XPathResult&&iterator.iterateNext()===a&&iterator.iterateNext()===b&&iterator.iterateNext()===null,'native identity/order');
            a.focus();a.addEventListener('x',()=>{});check(!iterator.invalidIteratorState,'non-tree activity');
            const other=document.implementation.createHTMLDocument();other.body.appendChild(other.createElement('div'));check(!iterator.invalidIteratorState,'other document');
            a.setAttribute('n','3');check(iterator.invalidIteratorState,'native mutation');
            try{iterator.iterateNext();throw Error('missing invalidation')}catch(e){check(e.name==='InvalidStateError','invalidated iterator error')}
            const result=document.evaluate('//li',document,null,XPathResult.ORDERED_NODE_SNAPSHOT_TYPE);a.remove();check(result.snapshotLength===2&&result.snapshotItem(0)===a&&!result.invalidIteratorState,'snapshot retention');
            const reused=document.evaluate('count(//li)',document,null,XPathResult.NUMBER_TYPE,result);check(reused===result&&result.numberValue===1,'reuse');
            try{result.snapshotItem(0);throw Error('wrong type allowed')}catch(e){check(e instanceof TypeError,'result type')}
            return 'ok'})()"#
            ),
            "ok"
        );
    }
    #[test]
    fn xpath_resolver_callbacks_preserve_empty_namespaces_abrupt_completion_and_native_lookup() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=t></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert_eq!(
            eval(
                scope,
                r#"(()=>{const check=(v,m)=>{if(!v)throw Error(m)};let calls=0;const callable=()=>{calls++;return ''};Object.defineProperty(callable,'lookupNamespaceURI',{get(){throw Error('callable property read')}});
            document.evaluate('/p:empty',document,callable);document.evaluate('/p:empty',document,callable);check(calls===2,'per-evaluation resolution');
            const sentinel={};let reported;const report=e=>{reported=e.error;e.preventDefault()};window.addEventListener('error',report);try{document.evaluate('//p:item',document,()=>{throw sentinel});throw Error('missing exception')}catch(e){check(e.name==='NamespaceError','namespace failure')}window.removeEventListener('error',report);check(reported===sentinel,'reported exception identity')
            try{document.evaluate('//p:item',document,()=>null);throw Error('missing namespace error')}catch(e){check(e.name==='NamespaceError','null prefix')}
            try{document.evaluate('unknown()',document);throw Error('missing syntax error')}catch(e){check(e.name==='SyntaxError','syntax')}
            const xml=new DOMParser().parseFromString('<r xmlns:p="urn:p"><p:item id="one"/></r>','text/xml');
            const resolver=xml.createNSResolver(xml.documentElement);check(resolver===xml.documentElement&&resolver.lookupNamespaceURI('p')==='urn:p'&&resolver.lookupPrefix('urn:p')==='p','native resolver identity');
            check(document.evaluate('//p:item',xml,resolver,9).singleNodeValue===xml.documentElement.firstChild,'cross-document evaluation');return 'ok'})()"#
            ),
            "ok"
        );
    }
    #[test]
    fn xpath_roots_in_shadow_tree_and_never_crosses_to_host_light_tree() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=host><span id=light></span></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert_eq!(
            eval(
                scope,
                r#"(()=>{const root=host.attachShadow({mode:'closed'}),span=document.createElement('span');root.append(span);const evaluator=new XPathEvaluator();const result=evaluator.evaluate('//span',span,null,7);if(result.snapshotLength!==1||result.snapshotItem(0)!==span)throw Error('shadow root');const outer=document.evaluate('//span',document,null,7);if(outer.snapshotLength!==1||outer.snapshotItem(0)!==light)throw Error('shadow leaked');return 'ok'})()"#
            ),
            "ok"
        );
    }
    #[test]
    fn xpath_namespace_and_attribute_snapshots_retain_actual_native_owners() {
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
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert_eq!(
            eval(
                scope,
                r#"(()=>{const xml=new DOMParser().parseFromString('<r xmlns:p="urn:p" a="old"/>','text/xml');const owner=xml.documentElement;const result=xml.evaluate('@a',owner,null,7);const attr=owner.getAttributeNode('a');owner.removeAttribute('a');owner.setAttribute('a','new');if(result.snapshotItem(0)!==attr||attr.value!=='old'||attr.ownerElement!==null)throw Error('Attr identity');const n=xml.evaluate('namespace::p',owner,null,9).singleNodeValue;if(n.nodeType!==13||n.ownerElement!==owner||n.nodeValue!=='urn:p'||n.parentNode!==null)throw Error('namespace slots');if(xml.evaluate('.',n,null,9).singleNodeValue!==n)throw Error('namespace identity');return 'ok'})()"#
            ),
            "ok"
        );
    }
    #[test]
    fn detached_attributes_and_resolver_reentrancy_use_native_identity() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=t></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert_eq!(
            eval(
                scope,
                r#"(()=>{const check=(v,m)=>{if(!v)throw Error(m)};const a=document.createAttribute('foo');a.value='native';for(const query of ['..','ancestor::*','following::*','preceding::*'])check(document.evaluate(query,a,null,7).snapshotLength===0,query);check(document.evaluate('string(.)',a,null,2).stringValue==='native','orphan string');check(document.evaluate('name(.)',a,null,2).stringValue==='foo','orphan name');check(document.evaluate('.',a,null,9).singleNodeValue===a,'orphan identity');
          const root=t.attachShadow({mode:'closed'}),span=document.createElement('span');span.textContent='inside';root.append(span);check(document.evaluate('string(/)',span,null,2).stringValue==='inside','shadow root string');
          const xml=new DOMParser().parseFromString('<r xmlns:p="urn:p"><p:x/></r>','text/xml');const n=xml.evaluate('namespace::p',xml.documentElement,null,9).singleNodeValue;check(n.compareDocumentPosition(n)===0,'namespace identity');check((n.compareDocumentPosition(xml.documentElement)&33)===33,'namespace disconnected');check((xml.documentElement.compareDocumentPosition(n)&33)===33,'reverse namespace comparison');check(!xml.documentElement.contains(n)&&!xml.documentElement.isEqualNode(n),'reverse identity');let fired=0;n.addEventListener('x',e=>{check(e.target===n&&!e.isTrusted,'native namespace target');e.preventDefault();fired++},{once:true});check(!n.dispatchEvent(new Event('x',{cancelable:true}))&&n.dispatchEvent(new Event('x',{cancelable:true}))&&fired===1,'namespace event lifetime');const result=xml.evaluate('1',xml);try{EventTarget.prototype.addEventListener.call(result,'x',()=>{});throw Error('wrong brand accepted')}catch(e){check(e instanceof TypeError,'result EventTarget brand')}const run=xml.createElement('run'),empty=xml.createTextNode(''),first=xml.createTextNode('a'),last=xml.createTextNode('b');run.append(empty,first,last);xml.documentElement.append(run);check(xml.evaluate('text()',run,null,7).snapshotItem(0)===first&&xml.evaluate('string(.)',last,null,2).stringValue==='ab','logical text node');const resolver=()=>{xml.documentElement.appendChild(xml.createElementNS('urn:p','p:x'));return 'urn:p'};check(xml.evaluate('//p:x',xml,resolver,7).snapshotLength===2,'resolver mutation checkpoint');return 'ok'})()"#
            ),
            "ok"
        );
    }

    #[test]
    fn failed_evaluation_preserves_reused_result_and_retired_resolvers_cannot_publish() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=a></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert_eq!(
            eval(
                scope,
                r#"(()=>{const check=(v,m)=>{if(!v)throw Error(m)};const original=a,result=document.evaluate('//div',document,null,7);for(const query of ['unknown()','1']){try{document.evaluate(query,document,null,7,result);throw Error('expected rejection')}catch(e){check(e instanceof TypeError||e.name==='SyntaxError','rejection type')}check(result.resultType===7&&result.snapshotLength===1&&result.snapshotItem(0)===original,'reused snapshot unchanged')}
          const thrown={};let reported;const report=e=>{reported=e.error;e.preventDefault()};window.addEventListener('error',report);try{document.evaluate('//p:x',document,()=>{throw thrown},7,result)}catch(e){check(e.name==='NamespaceError','callback namespace failure')}window.removeEventListener('error',report);check(reported===thrown,'reported callback identity');check(result.snapshotItem(0)===original,'callback failure unchanged');
          const evaluator=new XPathEvaluator();try{evaluator.createExpression('//p:x',()=>{document.open();document.close();return 'urn:p'});throw Error('retired callback published')}catch(e){check(e.name==='InvalidStateError','resolver document lifetime')}return 'ok'})()"#
            ),
            "ok"
        );
    }

    #[test]
    fn cross_realm_evaluators_use_context_document_matching_and_native_wrapper_owner() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let foreign = v8::Context::new(scope, Default::default());
        let (evaluator, document) = {
            let scope = &mut v8::ContextScope::new(scope, foreign);
            let dom = Rc::new(RefCell::new(
                oryn_dom::xml::parse("<html xmlns='http://www.w3.org/1999/xhtml'/>", "text/xml")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, foreign, dom, RealmId(1), 1).unwrap();
            let source = v8::String::new(scope, "new XPathEvaluator()").unwrap();
            let evaluator = v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap();
            let evaluator = v8::Global::new(scope, evaluator);
            let key = v8::String::new(scope, "document").unwrap();
            let document = foreign.global(scope).get(scope, key.into()).unwrap();
            (evaluator, v8::Global::new(scope, document))
        };
        let local = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, local);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, local, dom, RealmId(0), 1).unwrap();
        for (name, value) in [
            ("foreignEvaluator", evaluator),
            ("foreignDocument", document),
        ] {
            let key = v8::String::new(scope, name).unwrap();
            let value = v8::Local::new(scope, &value);
            local.global(scope).set(scope, key.into(), value).unwrap();
        }
        assert_eq!(
            eval(
                scope,
                r#"(()=>{const localResult=foreignEvaluator.evaluate('//html',document,null,9);if(localResult.singleNodeValue!==document.documentElement)throw Error('foreign evaluator HTML matching');if(document.evaluate('//html',foreignDocument,null,7).snapshotLength!==0)throw Error('caller realm leaked into XML');const node=document.evaluate('/*',foreignDocument,null,9).singleNodeValue;if(node!==foreignDocument.documentElement)throw Error('foreign native wrapper identity');return 'ok'})()"#
            ),
            "ok"
        );
    }
}
