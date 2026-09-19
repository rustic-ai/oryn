//! Range fragment parsing uses the native start boundary and context namespace.
use super::*;
use oryn_dom::sanitizer::HTML_NAMESPACE;

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let markup = trusted_string(
        scope,
        state,
        value,
        TrustedKind::Html,
        "Range createContextualFragment",
    )?;
    // Trusted policy conversion may move the range: capture native context last.
    let (owner, id) = range_boundaries::owner(scope, object)?;
    let state = &owner;
    let range: RangeRecord = state.dom.borrow().live_range(LiveRangeId(id))?.into();
    let (document, context, html) = {
        let dom = state.dom.borrow();
        let start = range.start_container;
        let record = dom.arena().get(start)?;
        let document = dom
            .arena()
            .node_document(start)?
            .unwrap_or(state.document_root());
        let html = dom
            .arena()
            .get(document)?
            .document_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.is_html);
        let context = if matches!(record.kind, NodeKind::Element { .. }) {
            Some(start)
        } else {
            record.parent.filter(|parent| {
                dom.arena()
                    .get(*parent)
                    .is_ok_and(|record| matches!(record.kind, NodeKind::Element { .. }))
            })
        };
        (
            document,
            context.filter(|context| {
                !dom.is_element(*context, "http://www.w3.org/1999/xhtml", "html")
            }),
            html,
        )
    };
    let xml_error = |error| match error {
        oryn_dom::xml::FragmentError::Dom(error) => V8BindingError::from(error),
        oryn_dom::xml::FragmentError::Syntax(_) => V8BindingError::Unsupported {
            interface: "Range".into(),
            member: "SyntaxError".into(),
        },
    };
    let (parsed, wrapper) = if html {
        let context = match context {
            Some(context) => {
                oryn_dom::sanitizer::FragmentContext::capture(&state.dom.borrow(), context)?
            }
            None => oryn_dom::sanitizer::FragmentContext::html_body(),
        };
        oryn_dom::sanitizer::prepare_inert_fragment(&markup, context)?
    } else if let Some(context) = context {
        oryn_dom::xml::parse_fragment(&state.dom.borrow(), context, &markup).map_err(xml_error)?
    } else {
        // Range's missing/html-root context is a detached HTML body even for
        // XML Documents. It supplies namespace context, not HTML parsing.
        let fallback = oryn_dom::xml::parse(
            "<body xmlns='http://www.w3.org/1999/xhtml'/>",
            "application/xml",
        )
        .into_runtime()?;
        let context = fallback.arena().get(fallback.document())?.children[0];
        oryn_dom::xml::parse_fragment(&fallback, context, &markup).map_err(xml_error)?
    };
    // The XML parser preserves document containers. Range removes those only
    // at the fragment boundary, repeatedly promoting their children; foreign
    // namespaces and containers nested under ordinary elements remain intact.
    let mut pending = parsed
        .arena()
        .get(wrapper)?
        .children
        .iter()
        .rev()
        .copied()
        .collect::<Vec<_>>();
    let mut children = Vec::new();
    while let Some(child) = pending.pop() {
        let record = parsed.arena().get(child)?;
        if matches!(&record.kind,NodeKind::Element{name}if name.ns.as_ref()==HTML_NAMESPACE&&matches!(name.local.as_ref(),"html"|"head"|"body"))
        {
            pending.extend(record.children.iter().rev().copied());
        } else {
            children.push(child);
        }
    }
    let fragment = {
        let mut dom = state.dom.borrow_mut();
        let fragment = dom.create_fragment();
        dom.set_node_document(fragment, document)?;
        for child in children {
            let child = dom.import_subtree(parsed.arena(), child)?;
            dom.append_child(fragment, child)?;
        }
        fragment
    };
    {
        let mut dom = state.dom.borrow_mut();
        let mut pending = vec![fragment];
        while let Some(node) = pending.pop() {
            let record = dom.arena_mut().get_mut(node)?;
            if matches!(&record.kind,NodeKind::Element{name}if name.local.as_ref()=="script"&&matches!(name.ns.as_ref(),"http://www.w3.org/1999/xhtml"|"http://www.w3.org/2000/svg"))
            {
                record.script_already_started = false;
                record.script_parser_document = None;
            }
            pending.extend(record.children.iter().copied());
        }
    }
    custom_elements::initialize_fragment_registry(state, fragment, context.unwrap_or(document))?;
    custom_elements::upgrade_for_document(scope, state, fragment)?;
    wrap_node(scope, state, fragment)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xml_range_strips_document_containers_at_fragment_boundary_only() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source=v8::String::new(scope,r#"(()=>{const ns='http://www.w3.org/1999/xhtml',doc=new DOMParser().parseFromString('<html xmlns="'+ns+'"/>','application/xhtml+xml'),range=doc.createRange();range.setStart(doc.documentElement,0);
        const fragment=range.createContextualFragment('<html xmlns="'+ns+'"><head><title>hi</title></head><body><html><body><p>content</p></body></html></body></html>');
        if(fragment.childNodes.length!==2||fragment.firstChild.localName!=='title'||fragment.lastChild.localName!=='p'||fragment.lastChild.textContent!=='content'||fragment.ownerDocument!==doc)return false;
        const foreign=range.createContextualFragment('<html xmlns="urn:foreign"><head/></html>');if(foreign.firstChild.localName!=='html'||foreign.firstChild.namespaceURI!=='urn:foreign'||foreign.firstChild.firstChild.localName!=='head')return false;
        const nested=range.createContextualFragment('<section xmlns="'+ns+'"><body><p/></body></section>');return nested.firstChild.firstChild.localName==='body';})()"#).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn conversion_moves_original_range_before_context_parsing_and_result_wrapping() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let source = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(192), 1).unwrap()
        };
        let target = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><table><tbody id=rows></tbody></table></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(193), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let document = wrap_node(scope, &target, target.document_root()).unwrap();
        let key = v8::String::new(scope, "destination").unwrap();
        first
            .global(scope)
            .set(scope, key.into(), document.into())
            .unwrap();
        let code=v8::String::new(scope,r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)},range=document.createRange(),rows=destination.getElementById('rows');range.selectNodeContents(document.body);const prototype=Object.getPrototypeOf(range);
          let conversions=0;const fragment=range.createContextualFragment({toString(){conversions++;range.selectNodeContents(rows);return '<tr><td>moved</td></tr>'}});
          check(conversions===1&&fragment.ownerDocument===destination&&fragment.firstChild.localName==='tr'&&fragment.firstChild.firstChild.textContent==='moved','post-conversion table context and wrapper owner');
          check(Object.getPrototypeOf(range)===prototype&&range.startContainer===rows,'original Range preserved');
          const failure={};let caught=false;try{range.createContextualFragment({toString(){range.selectNodeContents(document.body);throw failure}})}catch(error){caught=true;check(error===failure,'original conversion exception')}
          check(caught&&range.startContainer===document.body,'conversion side effect retained without parsing');return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        let _ = source;
    }
    #[test]
    fn native_range_fragment_context_owner_registry_and_xml_namespaces() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><table><tbody id=rows></tbody></table></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source=v8::String::new(scope,r#"(()=>{
          const check=(value,label)=>{if(!value)throw Error(label)};const range=document.createRange();range.selectNodeContents(rows);
          const row=range.createContextualFragment('<tr><td>cell</td></tr>');check(row.childNodes.length===1&&row.firstChild.localName==='tr'&&row.ownerDocument===document,'table context and owner');
          const registry=new CustomElementRegistry();class Scoped extends HTMLElement{};registry.define('x-range-scoped',Scoped);
          const host=document.createElement('div',{customElementRegistry:registry});range.selectNodeContents(host);
          const custom=range.createContextualFragment('<x-range-scoped></x-range-scoped>');check(custom.firstChild instanceof Scoped&&custom.firstChild.customElementRegistry===registry,'scoped detached fragment');
          const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');range.selectNodeContents(svg);check(range.createContextualFragment('<circle/>').firstChild.namespaceURI===svg.namespaceURI,'SVG context');
          const xml=new DOMParser().parseFromString('<root xmlns:p="urn:example"/>','application/xml');const xr=xml.createRange();xr.selectNodeContents(xml.documentElement);
          const fragment=xr.createContextualFragment('<p:item/>text');check(fragment.firstChild.namespaceURI==='urn:example'&&fragment.childNodes.length===2&&fragment.ownerDocument===xml,'XML namespace inheritance');
          let rejected=false;try{xr.createContextualFragment('<bad>')}catch(error){rejected=error.name==='SyntaxError'}check(rejected,'malformed XML fragment');
          rejected=false;try{xr.createContextualFragment('\uD800')}catch(error){rejected=error.name==='SyntaxError'}check(rejected,'XML isolated surrogate');
          xr.selectNodeContents(xml);check(xr.createContextualFragment('<item/>').firstChild.namespaceURI==='http://www.w3.org/1999/xhtml','missing XML context body fallback');
          range.selectNodeContents(document.body);globalThis.rangeScriptRuns=0;const executable=range.createContextualFragment('<script>rangeScriptRuns++<'+ '/script>');document.body.appendChild(executable);check(rangeScriptRuns===1,'Range scripts execute on insertion');return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}
