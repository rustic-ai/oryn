//! ParentNode/ChildNode union conversion shared by native mutation callers.
use super::*;
pub(super) fn applies(interface: &str, member: &str) -> bool {
    (matches!(interface, "Document" | "DocumentFragment" | "Element")
        && matches!(
            member,
            "append" | "prepend" | "replaceChildren" | "moveBefore"
        ))
        || (matches!(interface, "Element" | "CharacterData" | "DocumentType")
            && matches!(member, "before" | "after" | "replaceWith" | "remove"))
}
enum Converted {
    Node(v8::Global<v8::Value>),
    Text(String),
}
fn arguments(
    scope: &mut v8::PinScope,
    args: &v8::FunctionCallbackArguments,
) -> Result<Vec<Converted>, V8BindingError> {
    let mut converted = Vec::new();
    for index in 0..args.length() {
        let value = args.get(index);
        if node_identity::canonical(scope, value)?.is_some() {
            converted.push(Converted::Node(v8::Global::new(scope, value)));
        } else {
            let value = value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            converted.push(Converted::Text(rust_string_preserving_utf16(scope, value)?));
        }
    }
    Ok(converted)
}
fn local_nodes(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    values: &[Converted],
) -> Result<Vec<NodeId>, V8BindingError> {
    let mut result = Vec::new();
    for value in values {
        if let Converted::Node(value) = value {
            let value = v8::Local::new(scope, value);
            if let Some((owner, node)) = node_identity::argument(scope, value)?
                && Rc::ptr_eq(&owner.dom, &state.dom)
            {
                result.push(node);
            }
        }
    }
    Ok(result)
}
fn stage(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    receiver: NodeId,
    values: Vec<Converted>,
) -> Result<(Rc<V8BindingState>, NodeId, oryn_dom::TrackedNodeLease), V8BindingError> {
    let owner = owner_document_for_node(state, receiver)?;
    if values.len() == 1 {
        return match values.into_iter().next().unwrap() {
            Converted::Node(value) => {
                let value = v8::Local::new(scope, &value);
                let (owner, node) =
                    node_identity::argument(scope, value)?.ok_or(V8BindingError::Unsupported {
                        interface: "Node".into(),
                        member: "TypeError".into(),
                    })?;
                let lease = oryn_dom::TrackedNodeLease::new(&owner.dom, node)?;
                Ok((owner, node, lease))
            }
            Converted::Text(value) => {
                let node = state.dom.borrow_mut().create_text(value);
                state.dom.borrow_mut().set_node_document(node, owner)?;
                let lease = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
                Ok((state.clone(), node, lease))
            }
        };
    }
    let fragment = state.dom.borrow_mut().create(NodeKind::DocumentFragment);
    state.dom.borrow_mut().set_node_document(fragment, owner)?;
    let lease = oryn_dom::TrackedNodeLease::new(&state.dom, fragment)?;
    let wrapper = wrap_node(scope, state, fragment)?;
    for value in values {
        let (current, fragment) =
            node_identity::argument(scope, wrapper.into())?.ok_or(DomError::StaleNode(fragment))?;
        match value {
            Converted::Node(value) => {
                let value = v8::Local::new(scope, &value);
                let insertion = node_migration::insertion_argument(
                    scope, &current, fragment, value, None, None,
                )?;
                insertion.owner.dom.borrow_mut().insert_dom_node(
                    insertion.parent,
                    insertion.node,
                    insertion.reference,
                )?;
            }
            Converted::Text(value) => {
                let document = owner_document_for_node(&current, fragment)?;
                let node = current.dom.borrow_mut().create_text(value);
                current.dom.borrow_mut().set_node_document(node, document)?;
                validate_pre_insertion(&current.dom.borrow(), fragment, node, None, None)?;
                current
                    .dom
                    .borrow_mut()
                    .insert_dom_node(fragment, node, None)?;
            }
        }
    }
    let (current, fragment) =
        node_identity::argument(scope, wrapper.into())?.ok_or(DomError::StaleNode(fragment))?;
    Ok((current, fragment, lease))
}
pub(super) fn operation(
    scope: &mut v8::PinScope,
    _state: &Rc<V8BindingState>,
    _receiver: NodeId,
    member: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    let values = arguments(scope, args)?;
    // Every DOMString conversion can invoke author code, including adoption
    // of the receiver into another arena. Resolve the original wrapper only
    // after conversion; the old numeric NodeId is not mutation authority.
    let (current_owner, receiver) =
        node_identity::argument(scope, args.this().into())?.ok_or(V8BindingError::Unsupported {
            interface: "Node".into(),
            member: "TypeError".into(),
        })?;
    let state = &current_owner;
    let nodes = local_nodes(scope, state, &values)?;
    let child_method = matches!(member, "before" | "after" | "replaceWith");
    let (parent, viable) = {
        let dom = state.dom.borrow();
        if child_method {
            let Some(parent) = dom.parent(receiver)? else {
                return Ok(());
            };
            let siblings = light_children(&dom, parent)?;
            let index = siblings
                .iter()
                .position(|node| *node == receiver)
                .ok_or(DomError::StaleNode(receiver))?;
            let viable = if member == "before" {
                siblings[..index]
                    .iter()
                    .rev()
                    .copied()
                    .find(|node| !nodes.contains(node))
            } else {
                siblings[index + 1..]
                    .iter()
                    .copied()
                    .find(|node| !nodes.contains(node))
            };
            (parent, viable)
        } else {
            (receiver, None)
        }
    };
    let parent_wrapper = wrap_node(scope, state, parent)?;
    let viable_wrapper = viable
        .map(|node| wrap_node(scope, state, node))
        .transpose()?;
    // The DOM conversion algorithm stages multiple values in a real fragment.
    // Its removals occur before destination validity checks and remain visible.
    let (insertion_owner, insertion, _staging_lease) = stage(scope, state, receiver, values)?;
    let (current_owner, parent) = node_identity::argument(scope, parent_wrapper.into())?
        .ok_or(DomError::StaleNode(parent))?;
    let state = &current_owner;
    let receiver_identity =
        node_identity::argument(scope, args.this().into())?.ok_or(DomError::StaleNode(receiver))?;
    let still_child = Rc::ptr_eq(&receiver_identity.0.dom, &state.dom)
        && state.dom.borrow().parent(receiver_identity.1)? == Some(parent);
    let receiver = receiver_identity.1;
    let reference = if member == "before" {
        match viable_wrapper {
            None => light_children(&state.dom.borrow(), parent)?
                .first()
                .copied(),
            Some(wrapper) => {
                let (owner, previous) = node_identity::argument(scope, wrapper.into())?
                    .ok_or(DomError::StaleNode(parent))?;
                // Staging can execute a source script which moves the saved
                // previous sibling. The algorithm reads that original node's
                // current nextSibling, not its position in the former parent.
                let next = {
                    let dom = owner.dom.borrow();
                    match dom.parent(previous)? {
                        None => None,
                        Some(current_parent) => {
                            let siblings = light_children(&dom, current_parent)?;
                            siblings
                                .iter()
                                .position(|node| *node == previous)
                                .and_then(|index| siblings.get(index + 1).copied())
                        }
                    }
                };
                if next.is_some() && !Rc::ptr_eq(&owner.dom, &state.dom) {
                    return Err(V8BindingError::Unsupported {
                        interface: "Node".into(),
                        member: "NotFoundError".into(),
                    });
                }
                next
            }
        }
    } else {
        let viable = match viable_wrapper {
            None => None,
            Some(wrapper) => {
                let (owner, node) = node_identity::argument(scope, wrapper.into())?
                    .ok_or(DomError::StaleNode(parent))?;
                if !Rc::ptr_eq(&owner.dom, &state.dom) {
                    return Err(V8BindingError::Unsupported {
                        interface: "Node".into(),
                        member: "NotFoundError".into(),
                    });
                }
                Some(node)
            }
        };
        match member {
            "prepend" => light_children(&state.dom.borrow(), parent)?
                .first()
                .copied(),
            "after" | "replaceWith" => viable,
            _ => None,
        }
    };
    let value = wrap_node(scope, &insertion_owner, insertion)?;
    let prepared = if member == "replaceChildren" {
        node_migration::replacement_argument(scope, state, parent, value.into())?
    } else {
        node_migration::insertion_argument(
            scope,
            state,
            parent,
            value.into(),
            reference,
            if member == "replaceWith" && still_child {
                Some(receiver)
            } else {
                None
            },
        )?
    };
    let state = &prepared.owner;
    let parent = prepared.parent;
    let insertion = prepared.node;
    let reference = prepared.reference;
    let receiver = prepared.replaced.unwrap_or(receiver);
    let preparation = post_connection_scripts::capture_insertion(scope, state, parent, insertion)?;
    let mutation = if member == "replaceChildren" {
        state
            .dom
            .borrow_mut()
            .replace_dom_children(parent, insertion)
    } else if member == "replaceWith" && still_child && !prepared.replacement_removed {
        state
            .dom
            .borrow_mut()
            .replace_dom_child(parent, insertion, receiver, reference)
    } else {
        state
            .dom
            .borrow_mut()
            .insert_dom_node(parent, insertion, reference)
    };
    preparation.finish_mutation(scope, mutation).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn before_reads_saved_previous_siblings_current_successor_after_script_staging() {
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
        install_generated_bindings(scope, context, dom, RealmId(319), 1).unwrap();
        let source=v8::String::new(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const source=()=>{const script=document.createElement('script');script.type='text/plain';const text=document.createTextNode('');script.append(document.createTextNode('globalThis.stagingCallback()'),text);document.body.append(script);script.type='text/javascript';return text};
            for(const mode of ['detached','other-last','other-with-next','foreign-detached','foreign-with-next']){
                const parent=document.createElement('div'),previous=document.createElement('i'),receiver=document.createElement('b');parent.append(previous,receiver);document.body.append(parent);
                const other=document.createElement('section'),tail=document.createElement('u');document.body.append(other);const foreign=document.implementation.createHTMLDocument('');const text=source();let calls=0;
                globalThis.stagingCallback=()=>{calls++;if(mode==='detached')previous.remove();else if(mode==='foreign-detached')foreign.adoptNode(previous);else if(mode==='foreign-with-next')foreign.body.append(previous,tail);else{other.append(previous);if(mode==='other-with-next')other.append(tail)}};
                let failure;try{receiver.before(text,'suffix')}catch(error){failure=error}
                check(calls===1,mode+' callback once');
                if(mode.endsWith('with-next')){check(failure&&failure.name==='NotFoundError',mode+' foreign successor rejects');check(parent.childNodes.length===1&&parent.firstChild===receiver,mode+' destination preserved');check(text.parentNode.nodeType===11,mode+' staging remains');}
                else{check(!failure&&parent.firstChild===receiver&&parent.childNodes[1]===text&&parent.lastChild.data==='suffix',mode+' null successor appends');}
                parent.remove();other.remove();
            }
            return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn complete_variadic_self_reference_document_validation_and_grouping_matrix() {
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
        install_generated_bindings(scope, context, dom, RealmId(320), 1).unwrap();
        let source=v8::String::new(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)},same=(actual,expected,m)=>check(actual.length===expected.length&&expected.every((node,i)=>actual[i]===node),m);
            for(const kind of ['Element','Text','Comment','CDATA','PI'])for(const method of ['before','after','replaceWith'])for(const includeSelf of [false,true]){
                const parent=document.createElement('div'),a=document.createElement('a'),c=document.createElement('c'),b=kind==='Element'?document.createElement('b'):kind==='Text'?document.createTextNode('b'):kind==='Comment'?document.createComment('b'):kind==='CDATA'?document.implementation.createDocument(null,null).createCDATASection('b'):document.createProcessingInstruction('b','');parent.append(a,b,c);
                const observer=new MutationObserver(()=>{});observer.observe(parent,{childList:true});
                const args=includeSelf?[c,a,b]:[a,c,a];b[method](...args);
                const expected=includeSelf?[c,a,b]:method==='before'?[c,a,b]:method==='after'?[b,c,a]:[c,a];same(parent.childNodes,expected,kind+method+' identity/order');
                const rows=observer.takeRecords(),last=rows[rows.length-1];same(last.addedNodes,includeSelf?[c,a,b]:[c,a],method+' grouped additions');same(last.removedNodes,!includeSelf&&method==='replaceWith'?[b]:[],method+' grouped replacement');
            }
            for(const method of ['append','prepend','replaceChildren'])for(const parentKind of ['element','fragment','shadow']){
                const parent=parentKind==='fragment'?document.createDocumentFragment():parentKind==='shadow'?document.createElement('div').attachShadow({mode:'open'}):document.createElement('div'),a=document.createElement('a'),b=document.createElement('b'),c=document.createElement('c');parent.append(a,b,c);parent[method](b,a,b);
                same(parent.childNodes,method==='append'?[c,a,b]:method==='prepend'?[a,b,c]:[a,b],method+' duplicate argument staging');
                for(const invalid of [parent,document,document.createAttribute('bad'),document.implementation.createDocumentType('html','','')]){let error;try{parent[method](invalid)}catch(e){error=e}check(error&&error.name==='HierarchyRequestError',method+' invalid kind/cycle');}
            }
            for(const method of ['append','prepend','replaceChildren']){
                const doc=document.implementation.createDocument(null,null),root=doc.createElement('root'),type=document.implementation.createDocumentType('root','','');doc.append(type);doc.append(root);
                for(const incoming of [doc.createTextNode('text'),doc.createCDATASection('text'),document,document.createAttribute('x')]){const before=Array.from(doc.childNodes);let error;try{doc[method](incoming)}catch(e){error=e}check(error&&error.name==='HierarchyRequestError',method+' Document rejects invalid child');same(doc.childNodes,before,method+' invalid single input preserves Document');}
                const fragment=doc.createDocumentFragment();fragment.append(doc.createElement('a'),doc.createElement('b'));let error;try{doc[method](fragment)}catch(e){error=e}check(error&&error.name==='HierarchyRequestError'&&fragment.childNodes.length===2,method+' invalid fragment remains intact');
                const replacement=doc.createElement('replacement');error=undefined;try{doc[method](replacement)}catch(e){error=e}
                if(method==='replaceChildren')same(doc.childNodes,[replacement],'Document replacement accepts one element');else check(error&&error.name==='HierarchyRequestError'&&replacement.parentNode===null,method+' rejects second element before adoption');
                const empty=document.implementation.createDocument(null,null),comment=empty.createComment('comment'),pi=empty.createProcessingInstruction('x','y');empty[method](comment,pi);same(empty.childNodes,[comment,pi],method+' comment/PI document sequence');
            }
            const doc=document.implementation.createDocument(null,'root'),type=document.implementation.createDocumentType('root','','');doc.prepend(type);check(doc.firstChild===type,'doctype precedes element');
            let error;try{doc.append(type)}catch(e){error=e}check(error&&error.name==='HierarchyRequestError'&&doc.firstChild===type,'doctype-after-element rejects before move');
            const root=doc.documentElement;type.after(root);same(doc.childNodes,[type,root],'DocumentType after caller');type.before(doc.createComment('lead'));check(doc.firstChild.nodeType===8,'DocumentType before caller');
            const spare=document.implementation.createDocumentType('new','','');type.replaceWith(spare);check(doc.doctype===spare,'DocumentType replacement caller');
            const holder=document.createElement('div'),a=document.createElement('a');holder.append(a);let converted=0;try{holder.append(a,{toString(){converted++;throw 7}})}catch(e){check(e===7,'original conversion exception')}check(converted===1&&a.parentNode===holder,'all conversions before staging');
            return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn variadic_capacity_failure_keeps_required_staging_but_preserves_destination() {
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
        let state = install_generated_bindings(scope, context, dom, RealmId(305), 1).unwrap();
        let source=v8::String::new(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            for(const method of ['append','prepend','replaceChildren','before','after','replaceWith']){
                const host=document.createElement('div'),old=document.createElement('b');host.appendChild(old);document.body.appendChild(host);
                const input=document.createDocumentFragment();for(let i=0;i<2049;i++)input.appendChild(document.createElement('script'));
                const first=input.firstChild,receiver=['before','after','replaceWith'].includes(method)?old:host;let calls=0,rejected=false;
                try{receiver[method](input,{toString(){calls++;return 'tail'}})}catch(error){rejected=true}
                check(rejected&&calls===1,method+' capacity and conversion');
                check(host.childNodes.length===1&&host.firstChild===old,method+' destination untouched');
                check(input.childNodes.length===0&&first.parentNode.nodeType===11&&first.parentNode.childNodes.length===2050,method+' required staging retained');
                check(first.ownerDocument===document,method+' owner identity');
                host.remove();
            }
            const host=document.createElement('div');document.body.appendChild(host);host.append('success');check(host.textContent==='success','later admission');
        })()"#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert!(post_connection_scripts::delay(&state).is_none());
    }
    #[test]
    fn argument_conversion_adopts_receiver_before_all_parent_and_child_mutations() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let a = v8::Context::new(scope, Default::default());
        let b = v8::Context::new(scope, Default::default());
        let first = {
            let scope = &mut v8::ContextScope::new(scope, a);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, a, dom, RealmId(214), 1).unwrap()
        };
        let second = {
            let scope = &mut v8::ContextScope::new(scope, b);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, b, dom, RealmId(215), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, a);
        let destination = wrap_node(scope, &second, second.document_root()).unwrap();
        let key = v8::String::new(scope, "destination").unwrap();
        a.global(scope)
            .set(scope, key.into(), destination.into())
            .unwrap();
        let source=v8::String::new(scope,r#"(()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            for(const method of ['append','prepend','replaceChildren','before','after','replaceWith']){
                const host=document.createElement('div'),child=document.createElement('i'),extra=document.createElement('b');host.append(child);document.body.append(host);extra.textContent='extra';const receiver=['before','after','replaceWith'].includes(method)?child:host,prototype=Object.getPrototypeOf(receiver);receiver.marker=method;
                const observer=new MutationObserver(()=>{});observer.observe(host,{childList:true});let conversions=0;
                receiver[method](extra,{toString(){conversions++;destination.body.append(host);return 'converted'}});
                check(conversions===1&&receiver.ownerDocument===destination&&host.ownerDocument===destination,method+' refreshed receiver');check(extra.ownerDocument===destination&&extra.parentNode===host&&host.textContent.includes('converted'),method+' argument and string insertion');check(receiver.marker===method&&Object.getPrototypeOf(receiver)===prototype,method+' original wrapper');
                const rows=observer.takeRecords();check(rows.length===1&&rows[0].addedNodes.length===2&&rows[0].addedNodes[0]===extra,method+' one destination mutation group');
            }
            const host=document.createElement('div'),extra=document.createElement('b'),sentinel={};document.body.append(host);let caught;try{host.append(extra,{toString(){destination.body.append(host);throw sentinel}})}catch(error){caught=error}check(caught===sentinel&&host.ownerDocument===destination&&host.childNodes.length===0&&extra.ownerDocument===document,'conversion exception preserves adoption side effect without staging');
        })()"#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let _ = first;
    }
    #[test]
    fn mixin_exposure_variadic_conversion_and_family_dom_updates() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<main></main>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(100), 1).unwrap();
        let source=v8::String::new(scope,r#"
            const check=(x,m)=>{if(!x)throw Error(m)};
            for(const proto of [Document.prototype,DocumentFragment.prototype,Element.prototype])for(const name of ['append','prepend','replaceChildren']){
                const d=Object.getOwnPropertyDescriptor(proto,name);check(d&&d.value.length===0&&d.writable&&d.enumerable&&d.configurable,'ParentNode descriptor');check(proto[Symbol.unscopables][name]===true,'ParentNode unscopable');
            }
            for(const proto of [Element.prototype,CharacterData.prototype,DocumentType.prototype])for(const name of ['before','after','replaceWith','remove']){
                const d=Object.getOwnPropertyDescriptor(proto,name);check(d&&d.value.length===0&&d.writable&&d.enumerable&&d.configurable,'ChildNode descriptor');check(proto[Symbol.unscopables][name]===true,'ChildNode unscopable');
            }
            for(const name of ['append','prepend','replaceChildren','before','after','replaceWith','remove','moveBefore'])check(!(name in Node.prototype),'Node exposure');
            const host=document.querySelector('main');host.append();host.prepend();check(host.childNodes.length===0,'empty call');
            const log=[];host.append({toString(){log.push('a');return 'one'}},{toString(){log.push('b');return 'two'}},null,undefined);
            check(host.textContent==='onetwonullundefined'&&log.join(',')==='a,b','union conversion');
            const prior=host.textContent,sentinel={};try{host.replaceChildren('prefix',{toString(){throw sentinel}})}catch(e){check(e===sentinel,'original exception')}check(host.textContent===prior,'no partial replacement');
            host.replaceChildren();check(host.childNodes.length===0,'empty replacement');
            const a=document.createElement('a'),b=document.createElement('b');host.append(a,b);b.before('x','y');a.after('p','q');
            check(host.textContent==='pqxy'&&host.lastChild===b,'variadic adjacency');
            let calls=0;document.createTextNode('').before({toString(){calls++;return 'x'}});check(calls===1,'detached conversion');
            const moved=document.createElement('section');host.append(moved);moved.before({toString(){moved.remove();return 'ignored'}});check(!moved.parentNode,'post-conversion parent');
            const text=document.createTextNode('old');host.append(text);text.replaceWith({toString(){return '\uD800'}});check(host.lastChild.data.charCodeAt(0)===0xD800,'DOMString units');
            let rejected=false;try{host.append(Symbol())}catch(e){rejected=e instanceof TypeError}check(rejected,'symbol conversion');
        "#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
}

#[cfg(test)]
mod fragment_tests {
    use super::*;
    #[test]
    fn staged_fragments_group_observers_and_preserve_failure_side_effects() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<main></main>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(101), 1).unwrap();
        let source=v8::String::new(scope,r#"
            const check=(x,m)=>{if(!x)throw Error(m)},host=document.querySelector('main'),fragment=document.createDocumentFragment();
            const a=document.createElement('a'),b=document.createElement('b');fragment.append(a,b);
            const sourceObserver=new MutationObserver(()=>{}),targetObserver=new MutationObserver(()=>{});sourceObserver.observe(fragment,{childList:true});targetObserver.observe(host,{childList:true});
            host.append(fragment);let sourceRows=sourceObserver.takeRecords(),rows=targetObserver.takeRecords();
            check(sourceRows.length===1&&sourceRows[0].removedNodes.length===2&&sourceRows[0].removedNodes[0]===a,'source fragment group');
            check(rows.length===1&&rows[0].addedNodes.length===2&&rows[0].addedNodes[1]===b&&rows[0].previousSibling===null&&rows[0].nextSibling===null,'target insertion group');
            host.append();check(targetObserver.takeRecords().length===0,'empty append no record');
            host.replaceChildren(a,'tail');rows=targetObserver.takeRecords();
            check(rows.length===2&&rows[0].removedNodes[0]===a&&rows[1].removedNodes.length===1&&rows[1].removedNodes[0]===b&&rows[1].addedNodes.length===2,'staging removal before replacement group');
            host.replaceChildren(a,b);targetObserver.takeRecords();a.after(a,b,a);rows=targetObserver.takeRecords();
            check(host.childNodes.length===2&&host.firstChild===b&&host.lastChild===a,'duplicate nodes staged to final position');
            check(rows[rows.length-1].addedNodes.length===2,'adjacent insertion group');
            const holder=document.createElement('div'),x=document.createElement('x'),y=document.createElement('y');holder.append(x,y);
            const empty=document.implementation.createDocument(null,null);let failed=false;try{empty.append(x,y)}catch(e){failed=e.name==='HierarchyRequestError'}
            check(failed&&holder.childNodes.length===0&&empty.childNodes.length===0,'fragment staging before destination validity');
            host.replaceChildren(a,b);targetObserver.takeRecords();a.replaceWith(a,b);rows=targetObserver.takeRecords();
            check(host.childNodes.length===2&&host.firstChild===a&&host.lastChild===b,'self replacement staged receiver');
            check(rows[rows.length-1].addedNodes.length===2&&rows[rows.length-1].removedNodes.length===0,'staged receiver already removed');
        "#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
}
