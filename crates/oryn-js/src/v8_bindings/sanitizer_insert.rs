//! Nonstreaming HTML insertions share conversion, filtering and retained resource custody.
use super::*;

pub(super) fn run<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    args: &v8::FunctionCallbackArguments<'s>,
    member: &str,
) -> Result<()> {
    let receiver = wrap_node(scope, state, node)?;
    let safe = !member.ends_with("Unsafe");
    let operation = member.trim_end_matches("Unsafe").trim_end_matches("HTML");
    let html = html_argument(scope, args.get(0), safe)?;
    let options = options(scope, args.get(1), safe, true)?;
    let markup = if safe {
        let html = html.try_cast::<v8::String>().map_err(|_| invalid())?;
        rust_string_preserving_utf16(scope, html)?
    } else {
        let (owner, node) = node_identity::argument(scope, receiver.into())?.ok_or_else(invalid)?;
        let interface = if matches!(
            owner.dom.borrow().arena().get(node)?.kind,
            NodeKind::ShadowRoot { .. }
        ) {
            "ShadowRoot"
        } else {
            "Element"
        };
        trusted_string(
            scope,
            &owner,
            html,
            TrustedKind::Html,
            &format!("{interface} {member}"),
        )?
    };
    let (owner, node) = node_identity::argument(scope, receiver.into())?.ok_or_else(invalid)?;
    let sibling = matches!(operation, "before" | "after" | "replaceWith");
    let parent = if sibling {
        let Some(parent) = owner.dom.borrow().parent(node)? else {
            return Ok(());
        };
        parent
    } else {
        node
    };
    let context_node = {
        let dom = owner.dom.borrow();
        match &dom.arena().get(parent)?.kind {
            NodeKind::Element { .. } => Some(parent),
            NodeKind::ShadowRoot { .. } => dom.parent(parent)?,
            NodeKind::DocumentFragment => None,
            _ => {
                return Err(V8BindingError::Unsupported {
                    interface: "Element".into(),
                    member: "NoModificationAllowedError".into(),
                });
            }
        }
    };
    let context = if let Some(context) = context_node {
        oryn_dom::sanitizer::FragmentContext::capture(&owner.dom.borrow(), context)?
    } else {
        let mut context = oryn_dom::sanitizer::FragmentContext::html_body();
        context.disable_context_shadow = true;
        context
    };
    if safe && context.safe_sink_is_noop() {
        return Ok(());
    }
    let (fragment, root) = prepare_sink_fragment(&markup, &options, context, safe)?;
    let (target_owner, target) = if !sibling
        && owner
            .dom
            .borrow()
            .is_element(parent, HTML_NAMESPACE, "template")
    {
        if let Some(content) = template_links::content(scope, &owner, parent)? {
            node_identity::argument(scope, content)?.ok_or_else(invalid)?
        } else {
            let content = owner.dom.borrow_mut().template_content(parent)?;
            (owner.clone(), content)
        }
    } else {
        (owner.clone(), parent)
    };
    let children = light_children(&target_owner.dom.borrow(), target)?;
    let reference = match operation {
        "append" => None,
        "prepend" => children.first().copied(),
        "before" => Some(node),
        "after" | "replaceWith" => children
            .iter()
            .position(|child| *child == node)
            .and_then(|index| children.get(index + 1).copied()),
        _ => return Err(invalid()),
    };
    post_connection_scripts::check_fragment_capacity(
        scope,
        &target_owner,
        target,
        &fragment,
        root,
    )?;
    let staging = target_owner
        .dom
        .borrow_mut()
        .create(NodeKind::DocumentFragment);
    let _staging = oryn_dom::TrackedNodeLease::new(&target_owner.dom, staging)?;
    let document = target_owner
        .dom
        .borrow()
        .node_document(target)?
        .ok_or(DomError::StaleNode(target))?;
    target_owner
        .dom
        .borrow_mut()
        .set_node_document(staging, document)?;
    let inserted = target_owner
        .dom
        .borrow_mut()
        .import_fragment_children(target, &fragment, root)?;
    for &child in &inserted {
        target_owner.dom.borrow_mut().append_child(staging, child)?;
        custom_elements::initialize_fragment_registry_from(&target_owner, child, &owner, parent)?;
        if !options.run_scripts {
            target_owner.mark_script_subtree_inert(child)?;
        }
    }
    let preparation =
        post_connection_scripts::capture_replacement(scope, &target_owner, target, &inserted)?;
    let mutation = (|| -> std::result::Result<(), DomError> {
        let mut dom = target_owner.dom.borrow_mut();
        // Shadow roots are native attachment edges, never inserted light nodes.
        dom.attach_imported_fragment_shadows(target, &inserted)?;
        if operation == "replaceWith" {
            dom.replace_dom_child(target, staging, node, reference)?;
        } else {
            dom.insert_dom_node(target, staging, reference)?;
        }
        Ok(())
    })();
    preparation.finish_mutation(scope, mutation)?;
    for child in inserted {
        upgrade_custom_element_subtree(scope, &target_owner, child)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_nonstreaming_html_positions_preserve_identity_and_group_mutations() {
        super::super::super::tests::initialize_v8();
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
        let code=v8::String::new(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            for(const unsafe of [false,true])for(const operation of ['append','prepend','before','after','replaceWith']){
                const host=document.createElement('div');host.innerHTML='<i>L</i><i>A</i><i>R</i>';document.body.append(host);
                const [left,anchor,right]=host.children,method=operation+'HTML'+(unsafe?'Unsafe':'');
                const receiver=['append','prepend'].includes(operation)?host:anchor;
                const observer=new MutationObserver(()=>{});observer.observe(host,{childList:true});
                receiver[method]('<b>X</b><u>Y</u>',{sanitizer:{}});
                check(host.textContent===({append:'LARXY',prepend:'XYLAR',before:'LXYAR',after:'LAXYR',replaceWith:'LXYR'})[operation],method+' position');
                check(left.parentNode===host&&right.parentNode===host,'existing identities');
                const records=observer.takeRecords();check(records.length===1&&records[0].addedNodes.length===2&&records[0].removedNodes.length===(operation==='replaceWith'?1:0),'one grouped mutation '+method);
                observer.disconnect();host.remove();
            }
            for(const unsafe of [false,true])for(const operation of ['append','prepend']){
                const method=operation+'HTML'+(unsafe?'Unsafe':'');
                for(const kind of ['shadow','template']){
                    const host=document.createElement(kind==='template'?'template':'div');document.body.append(host);
                    const receiver=kind==='shadow'?host.attachShadow({mode:'open'}):host,target=kind==='template'?host.content:receiver;
                    receiver[method]('<b>A</b>',{sanitizer:{}});receiver[method]('<i>B</i>',{sanitizer:{}});
                    check(target.textContent===(operation==='append'?'AB':'BA'),kind+' target');host.remove();
                }
            }
            const template=document.createElement('template');
            check(template.buffer===false,'buffer default');template.buffer=true;
            check(template.buffer===true&&template.getAttribute('buffer')==='','buffer true reflects');
            template.buffer=false;check(!template.buffer&&!template.hasAttribute('buffer'),'buffer false removes');
            template.setAttribute('buffer','false');check(template.buffer===true,'buffer presence ignores lexical false');
            template.removeAttribute('buffer');check(template.buffer===false,'buffer removal reflected');
            template.buffer={valueOf(){throw Error('boolean valueOf')},toString(){throw Error('boolean toString')}};
            check(template.buffer===true,'buffer uses boolean conversion');
            for(const value of [null,undefined,0,'',false]){template.buffer=value;check(template.buffer===false,'buffer false values');}
            check(template.shadowRootAdoptedStyleSheets==='','stylesheet descriptor default');
            const descriptor=' red '+String.fromCharCode(0xd800)+'  green ';
            template.shadowRootAdoptedStyleSheets=descriptor;
            check(template.shadowRootAdoptedStyleSheets===descriptor&&template.getAttribute('shadowrootadoptedstylesheets')===descriptor,'stylesheet descriptor exact DOMString');
            const inert=document.implementation.createHTMLDocument('');let descriptorConversions=0;
            template.shadowRootAdoptedStyleSheets={toString(){descriptorConversions++;inert.adoptNode(template);return descriptor+' moved'}};
            check(descriptorConversions===1&&template.ownerDocument===inert&&template.shadowRootAdoptedStyleSheets===descriptor+' moved','stylesheet descriptor adoption during conversion');
            template.removeAttribute('shadowrootadoptedstylesheets');check(template.shadowRootAdoptedStyleSheets==='','stylesheet descriptor removal');
            const parsed=document.createElement('div');parsed.setHTMLUnsafe('<template buffer><i>inert</i></template>');
            check(parsed.firstChild.buffer===true&&parsed.firstChild.content.textContent==='inert','parsed native buffer reflection');
            const host=document.createElement('div');document.body.append(host);host.innerHTML='<b>old</b>';const old=host.firstChild;
            let failed=false;try{host.appendHTML('<i>bad</i>',{sanitizer:{elements:[],removeElements:[]}})}catch{failed=true}
            check(failed&&host.firstChild===old&&host.childNodes.length===1,'invalid policy before mutation');
            const other=document.createElement('div');document.body.append(other);let conversions=0;
            old.beforeHTMLUnsafe({toString(){conversions++;return '<i>moved</i>'}},{get sanitizer(){other.append(old);return {}}});
            check(conversions===1&&other.firstChild.localName==='i'&&other.lastChild===old&&!host.firstChild,'resolve after options movement');
            const detached=document.createElement('i');let options=0;detached.afterHTMLUnsafe('ignored',{get sanitizer(){options++;return {}}});check(options===1&&!detached.parentNode,'detached conversion then no-op');
            globalThis.insertRuns=0;host.appendHTMLUnsafe('<script>insertRuns++<'+ '/script>',{runScripts:true});check(insertRuns===1,'unsafe scripts execute');
            host.prependHTML('<script>insertRuns++<'+ '/script>',{get runScripts(){throw Error('safe read runScripts')},sanitizer:{}});check(insertRuns===1,'safe scripts removed');
            return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn inserted_declarative_roots_never_publish_host_children() {
        super::super::super::tests::initialize_v8();
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
        let code=v8::String::new(scope,r#"(()=>{
            for(const method of ['appendHTMLUnsafe','prependHTMLUnsafe']){
                const host=document.createElement('div');document.body.append(host);host.innerHTML='<b>old</b>';const old=host.firstChild;
                const observer=new MutationObserver(()=>{});observer.observe(host,{childList:true});
                host[method]('<template shadowrootmode="open"><i>shadow</i></template>');
                if(!host.shadowRoot||host.shadowRoot.textContent!=='shadow'||host.firstChild!==old||observer.takeRecords().length)throw Error('shadow attachment appeared as light insertion');
                host[method]('<template shadowrootmode="open"><i>fallback</i></template>');
                if(host.shadowRoot.textContent!=='shadow'||!host.querySelector('template'))throw Error('existing shadow root overwritten');observer.disconnect();host.remove();
            }
            for(const method of ['beforeHTMLUnsafe','afterHTMLUnsafe','replaceWithHTMLUnsafe']){
                const fragment=document.createDocumentFragment(),anchor=document.createElement('i');fragment.append(anchor);
                anchor[method]('<template shadowrootmode="open"><b>inert</b></template>');
                const template=fragment.querySelector('template');
                if(!template||template.content.textContent!=='inert')throw Error('synthetic fragment context consumed DSD '+method);
            }
            return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}
