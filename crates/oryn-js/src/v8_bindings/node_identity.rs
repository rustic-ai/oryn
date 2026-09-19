//! Node identity and root operations compare native arena ownership, and perform
//! option conversion before observing the potentially reentrantly changed tree.
use super::*;
fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Node".into(),
        member: "TypeError".into(),
    }
}
/// Prove the exact native Node wrapper, including a retained original owner.
/// Execution retirement is intentionally separate from DOM identity. This
/// function creates no persistent roots and never invokes author conversions.
pub(super) fn canonical(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Option<(Rc<V8BindingState>, NodeId)>, V8BindingError> {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok(None);
    };
    if object.is_proxy() || object.internal_field_count() < 5 {
        return Ok(None);
    }
    let Some(wrapper) = receiver_wrapper_id(scope, object) else {
        return Ok(None);
    };
    let invalid = || V8BindingError::Binding(BindingError::UnknownWrapper(wrapper));
    // Do not use ToNumber on embedder fields: malformed non-node native wrappers
    // must not trigger script while DOM argument conversion is inspecting identity.
    let mut fields = Vec::with_capacity(5);
    for index in 0..5 {
        let Some(field) = object
            .get_internal_field(scope, index)
            .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
        else {
            return Ok(None);
        };
        fields.push(field);
    }
    if !(fields[1].is_uint32()
        && fields[2].is_uint32()
        && fields[3].is_uint32()
        && fields[4].is_big_int())
    {
        return Ok(None);
    }
    let node = NodeId {
        slot: fields[1].uint32_value(scope).ok_or_else(invalid)?,
        generation: fields[2].uint32_value(scope).ok_or_else(invalid)?,
    };
    let realm = RealmId(fields[3].uint32_value(scope).ok_or_else(invalid)?);
    let generation = v8::Local::<v8::BigInt>::try_from(fields[4]).map_err(|_| invalid())?;
    let (generation, lossless) = generation.u64_value();
    if !lossless {
        return Err(invalid());
    }
    // Retained wrappers keep their original registry even after the Window's
    // execution services retire or its realm installs a replacement Document.
    // This lookup does not restore execution or grant host/automation authority.
    let owner = binding_state_for_receiver(scope, object)
        .filter(|owner| owner.realm == realm && owner.wrapper_generation == generation)
        .ok_or_else(invalid)?;
    let resolved = owner.wrappers.borrow().resolve(
        &owner.dom.borrow(),
        wrapper,
        realm,
        generation,
        InterfaceBrand::Node,
    )?;
    if resolved != node {
        return Err(invalid());
    }
    let canonical = owner
        .objects
        .borrow()
        .get(&wrapper)
        .and_then(|entry| entry.handle.to_local(scope))
        .ok_or_else(invalid)?;
    if !canonical.strict_equals(object.into()) {
        return Err(invalid());
    }
    Ok(Some((owner, node)))
}

pub(super) fn argument(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Option<(Rc<V8BindingState>, NodeId)>, V8BindingError> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
    if object.is_proxy() {
        return Err(invalid());
    }
    let owner = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
    let other = argument_node(scope, &owner, value)?;
    Ok(Some((owner, other)))
}
pub(super) fn same(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    value: v8::Local<v8::Value>,
) -> Result<bool, V8BindingError> {
    let Some((owner, other)) = argument(scope, value)? else {
        return Ok(false);
    };
    Ok(Rc::ptr_eq(&owner.dom, &state.dom) && node == other)
}
pub(super) fn root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let options = args.get(0);
    let composed = if options.is_null_or_undefined() {
        false
    } else {
        let options = v8::Local::<v8::Object>::try_from(options).map_err(|_| invalid())?;
        event_init_member(scope, Some(options), "composed")
            .ok_or(V8BindingError::JavaScriptException)?
            .boolean_value(scope)
    };
    // Conversion may move/adopt the receiver or replace its Document. Resolve
    // its native identity after the callback rather than using an earlier ID.
    let owner = binding_state_for_receiver(scope, args.this()).ok_or_else(invalid)?;
    let node = argument_node(scope, &owner, args.this().into())?;
    let root = if composed {
        shadow_including_tree_root(&owner.dom.borrow(), node)?
    } else {
        tree_root(&owner.dom.borrow(), node)?
    };
    Ok(wrap_node(scope, &owner, root)?.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_identity_and_composed_root_conversion() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<main></main>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source=v8::String::new(scope,r#"
            const check=(x,m)=>{if(!x)throw Error(m)};
            const nodes=[document,document.createDocumentFragment(),document.createTextNode('x'),document.createComment('x'),document.createProcessingInstruction('p','v'),document.createElement('p'),document.createAttribute('a'),document.implementation.createDocumentType('html','','')];
            for(const n of nodes){check(n.isSameNode(n),'self');check(!n.isSameNode(n.cloneNode())&&!n.isSameNode(null)&&!n.isSameNode(undefined),'distinct/null');}
            let bad=false;try{nodes[0].isSameNode({})}catch(e){bad=e instanceof TypeError}check(bad,'argument brand');
            bad=false;try{Node.prototype.isSameNode.call({},document)}catch(e){bad=e instanceof TypeError}check(bad,'receiver brand');
            const host=document.querySelector('main'),shadow=host.attachShadow({mode:'closed'}),inner=document.createElement('section');shadow.appendChild(inner);
            const nested=inner.attachShadow({mode:'open'}),text=document.createTextNode('x');nested.appendChild(text);
            check(text.getRootNode()===nested&&text.getRootNode({composed:false})===nested,'ordinary root');
            check(text.getRootNode({composed:true})===document&&nested.getRootNode({composed:1})===document,'nested composed root');
            check(text.getRootNode({get composed(){host.remove();return true}})===host,'callback tree update');
            check(text.getRootNode(null)===nested,'null dictionary');
            bad=false;try{text.getRootNode(1)}catch(e){bad=e instanceof TypeError}check(bad,'dictionary conversion');
            const sentinel={};try{text.getRootNode({get composed(){throw sentinel}});throw Error('missing throw')}catch(e){check(e===sentinel,'exception identity')}
            const d=Object.getOwnPropertyDescriptor(Node.prototype,'isSameNode');check(d.enumerable&&d.writable&&d.configurable&&d.value.length===1,'descriptor');
        "#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn independent_arenas_never_alias_numeric_node_ids() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let foreign = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("").into_runtime().unwrap(),
            ));
            let root = dom.borrow().document();
            let state = install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap();
            let node = wrap_node(scope, &state, root).unwrap();
            v8::Global::new(scope, node)
        };
        let second = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, second);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let root = dom.borrow().document();
        let state = install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap();
        let foreign = v8::Local::new(scope, &foreign);
        assert!(!same(scope, &state, root, foreign.into()).unwrap());
        let key = v8::String::new(scope, "foreign").unwrap();
        second.global(scope).set(scope, key.into(), foreign.into());
        let script=v8::String::new(scope,r#"
            const check=(x,m)=>{if(!x)throw Error(m)};
            check(!document.isSameNode(foreign)&&!foreign.isSameNode(document),'arena identity');
            check(!document.contains(foreign)&&!foreign.contains(document),'foreign containment');
            check(document.isEqualNode(foreign)&&foreign.isEqualNode(document),'cross arena structural equality');
            const a=document.compareDocumentPosition(foreign), b=foreign.compareDocumentPosition(document);
            check((a&33)===33&&(b&33)===33&&Boolean(a&2)!==Boolean(b&2),'stable disconnected order');
            check(a===document.compareDocumentPosition(foreign),'repeat ordering');
            foreign.documentElement.setAttribute('data-test','different');check(!document.isEqualNode(foreign),'different attributes');
        "#).unwrap();
        v8::Script::compile(scope, script, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }

    #[test]
    fn retained_nodes_use_dom_unions_without_restoring_host_authority() {
        crate::v8_bindings::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let parsed = oryn_dom::html::parse(
            "<section id=kept><span>text</span></section><select id=choices><option id=before>old</option></select>",
        );
        let child_document = dom
            .borrow_mut()
            .import_subtree(&parsed.dom, parsed.document)
            .unwrap();
        let kept_node = dom
            .borrow()
            .get_element_by_id(child_document, "kept")
            .unwrap()
            .unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let root_context = v8::Context::new(handles, Default::default());
        let child_context = v8::Context::new(handles, Default::default());
        let root_state = {
            let scope = &mut v8::ContextScope::new(handles, root_context);
            install_generated_bindings(scope, root_context, dom.clone(), RealmId(901), 1).unwrap()
        };
        let (child_state, document) = {
            let scope = &mut v8::ContextScope::new(handles, child_context);
            let state = install_generated_bindings_with_root(
                scope,
                child_context,
                dom,
                RealmId(902),
                2,
                child_document,
            )
            .unwrap();
            let document = wrap_node(scope, &state, child_document).unwrap();
            (state, v8::Global::new(scope, document))
        };
        let scope = &mut v8::ContextScope::new(handles, root_context);
        let document = v8::Local::new(scope, &document);
        let key = v8::String::new(scope, "sourceDocument").unwrap();
        root_context
            .global(scope)
            .set(scope, key.into(), document.into());
        let source = v8::String::new(
            scope,
            r#"
            globalThis.kept=sourceDocument.getElementById('kept');
            globalThis.retainedSelect=sourceDocument.getElementById('choices');
            globalThis.retainedBefore=sourceDocument.getElementById('before');
            globalThis.keptPrototype=Object.getPrototypeOf(kept);
            kept.marker=73;
            document.adoptNode(kept);
            globalThis.savedOwner=kept.ownerDocument;
        "#,
        )
        .unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        child_state.retire_execution();
        wrapper_graphs::refresh(scope, &child_state);
        let kept = wrap_node(scope, &child_state, kept_node).unwrap();
        let (owner, node) = canonical(scope, kept.into()).unwrap().unwrap();
        assert!(Rc::ptr_eq(&owner, &child_state));
        assert_eq!(node, kept_node);
        assert!(native_node_identity(scope, kept.into()).is_err());
        let source = v8::String::new(scope, r#"
            const check=(ok,message)=>{if(!ok)throw Error(message)};
            check(kept.marker===73&&kept.ownerDocument===document&&savedOwner===document&&kept.parentNode===null,'retained adopted identity');
            let conversions=0;
            kept.toString=()=>{conversions++;throw Error('Node string conversion')};
            retainedBefore.valueOf=()=>{conversions++;throw Error('HTMLElement numeric conversion')};
            sourceDocument.toString=()=>{conversions++;throw Error('Document body conversion')};
            document.body.append(kept);
            check(kept.parentNode===document.body&&Object.getPrototypeOf(kept)===keptPrototype,'append original wrapper');
            const iterator=document.createNodeIterator(kept,NodeFilter.SHOW_ELEMENT);
            check(iterator.root===kept&&iterator.nextNode()===kept&&iterator.nextNode()===kept.firstChild,'NodeIterator retained root');
            const walker=document.createTreeWalker(kept,NodeFilter.SHOW_ELEMENT);
            check(walker.root===kept&&walker.firstChild()===kept.firstChild,'TreeWalker retained root');
            const range=new StaticRange({startContainer:kept,startOffset:0,endContainer:kept,endOffset:1});
            check(range.startContainer===kept&&range.endContainer===kept&&range.startOffset===0&&range.endOffset===1,'StaticRange retained endpoints');
            check(document.evaluate('.',kept,null,XPathResult.FIRST_ORDERED_NODE_TYPE,null).singleNodeValue===kept,'XPath retained context');
            const option=document.createElement('option');option.textContent='new';
            retainedSelect.add(option,retainedBefore);
            check(retainedSelect.children.length===2&&retainedSelect.firstChild===option&&option.nextSibling===retainedBefore,'select add HTMLElement branch');
            let sendError='';
            try{new XMLHttpRequest().send(sourceDocument)}catch(error){sendError=error.name}
            check(sendError==='InvalidStateError','XHR Document branch reaches unopened-request state check');
            check(conversions===0,'native unions invoke no authored conversion');
        "#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert!(child_state.execution_retired());
        assert!(!root_state.execution_retired());
        assert!(native_node_identity(scope, kept.into()).is_err());
        assert!(
            child_state
                .objects
                .borrow()
                .values()
                .all(|entry| entry.connected_root.is_none())
        );
    }

    #[test]
    fn canonical_identity_rejects_proxy_and_copied_native_fields() {
        crate::v8_bindings::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p id=kept>text</p>")
                .into_runtime()
                .unwrap(),
        ));
        let node = dom.borrow().find_by_id("kept").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(903), 1).unwrap();
        let object = wrap_node(scope, &state, node).unwrap();
        let key = v8::String::new(scope, "kept").unwrap();
        context.global(scope).set(scope, key.into(), object.into());
        let source = v8::String::new(
            scope,
            r#"
            for(const name of ['nodeType','ownerDocument','isConnected','toJSON']){
                Object.defineProperty(kept,name,{get(){throw Error('identity getter')}});
            }
        "#,
        )
        .unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        state.retire_execution();
        let (owner, actual) = canonical(scope, object.into()).unwrap().unwrap();
        assert!(Rc::ptr_eq(&owner, &state));
        assert_eq!(actual, node);
        assert!(native_node_identity(scope, object.into()).is_err());
        for source in [
            "({get nodeType(){throw Error('getter')}})",
            "Object.create(kept)",
            "new Proxy(kept,{get(){throw Error('trap')}})",
            "(()=>{const p=Proxy.revocable(kept,{});p.revoke();return p.proxy})()",
        ] {
            let source = v8::String::new(scope, source).unwrap();
            let value = v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap();
            assert!(canonical(scope, value).unwrap().is_none());
        }
        let template = v8::ObjectTemplate::new(scope);
        template.set_internal_field_count(5);
        let forged = template.new_instance(scope).unwrap();
        for index in 0..5 {
            let field = object.get_internal_field(scope, index).unwrap();
            forged.set_internal_field(index, field);
        }
        assert!(
            canonical(scope, forged.into()).is_err(),
            "copied fields do not identify the canonical object"
        );
        let source =
            v8::String::new(scope, "({valueOf(){throw Error('native field coercion')}})").unwrap();
        let poison = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        forged.set_internal_field(1, poison.into());
        assert!(
            canonical(scope, forged.into()).unwrap().is_none(),
            "malformed fields are not converted"
        );
    }

    #[test]
    fn retained_document_generation_wins_over_numeric_wrapper_collision() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let first = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><p id=old>old</p></body>")
                .into_runtime()
                .unwrap(),
        ));
        let old_root = first.borrow().document();
        let previous = install_generated_bindings(scope, context, first, RealmId(904), 1).unwrap();
        let old_document = wrap_node(scope, &previous, old_root).unwrap();
        let old_wrapper = receiver_wrapper_id(scope, old_document).unwrap();
        let key = v8::String::new(scope, "retainedOldDocument").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), old_document.into());
        let source = v8::String::new(scope, "document.getElementById('old').expando=42").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let second = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><p id=new>new</p></body>")
                .into_runtime()
                .unwrap(),
        ));
        let new_root = second.borrow().document();
        let replacement =
            install_generated_bindings(scope, context, second, RealmId(904), 2).unwrap();
        let new_document = wrap_node(scope, &replacement, new_root).unwrap();
        assert_eq!(
            old_wrapper,
            receiver_wrapper_id(scope, new_document).unwrap()
        );
        let (owner, node) = canonical(scope, old_document.into()).unwrap().unwrap();
        assert!(Rc::ptr_eq(&owner, &previous));
        assert_eq!(node, old_root);
        assert!(!previous.execution_retired());
        assert!(
            native_node_identity(scope, old_document.into()).is_err(),
            "a replaced registry is not live host authority even before retirement"
        );
        previous.retire_execution();
        wrapper_graphs::refresh(scope, &previous);
        let (owner, _) = canonical(scope, old_document.into()).unwrap().unwrap();
        assert!(Rc::ptr_eq(&owner, &previous));
        let (owner, _) = native_node_identity(scope, new_document.into())
            .unwrap()
            .unwrap();
        assert!(Rc::ptr_eq(&owner, &replacement));
        let source = v8::String::new(scope, "document.getElementById('new').textContent+'|'+retainedOldDocument.getElementById('old').expando").unwrap();
        let value = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert_eq!(value.to_rust_string_lossy(scope), "new|42");
        assert!(
            previous
                .objects
                .borrow()
                .values()
                .all(|entry| entry.connected_root.is_none())
        );
        assert!(previous.execution_retired());
        assert!(!replacement.execution_retired());
    }
}
