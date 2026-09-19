//! Document-scoped factories with WebIDL conversion and native adoption.
use super::*;
fn error(member: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "DOMImplementation".into(),
        member: member.into(),
    }
}
pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    let source_document = state
        .implementation_documents
        .borrow()
        .get(&id)
        .copied()
        .ok_or_else(|| error("TypeError"))?;
    match member {
        "hasFeature" => return Ok(Some(v8::Boolean::new(scope, true).into())),
        "createDocument" => {
            if args.length() < 2 {
                return Err(error("TypeError"));
            }
            let namespace = if args.get(0).is_null_or_undefined() {
                String::new()
            } else {
                required_string(scope, args, 0)?
            };
            let qualified_name = if args.get(1).is_null() {
                String::new()
            } else {
                required_string(scope, args, 1)?
            };
            // WebIDL DocumentType conversion precedes namespace validation.
            // Retain the actual foreign node identity; registry-local IDs
            // cannot identify a doctype from another native arena.
            let doctype = if args.get(2).is_null_or_undefined() {
                None
            } else {
                let (source, node) = node_identity::argument(scope, args.get(2))?
                    .ok_or_else(|| error("TypeError"))?;
                if !matches!(
                    source.dom.borrow().arena().get(node)?.kind,
                    NodeKind::DocumentType { .. }
                ) {
                    return Err(error("TypeError"));
                }
                Some((source, node))
            };
            if !qualified_name.is_empty() {
                validate_element_namespace(&namespace, &qualified_name)?;
            }
            let document = {
                let mut dom = state.dom.borrow_mut();
                let document = dom.create(NodeKind::Document);
                dom.arena_mut()
                    .get_mut(document)?
                    .document_metadata
                    .as_mut()
                    .unwrap()
                    .content_type = match namespace.as_str() {
                    "http://www.w3.org/1999/xhtml" => "application/xhtml+xml",
                    "http://www.w3.org/2000/svg" => "image/svg+xml",
                    _ => "application/xml",
                }
                .into();
                document
            };
            state.derive_document_provenance(source_document, document)?;
            let output = wrap_node(scope, state, document)?;
            let root = if qualified_name.is_empty() {
                None
            } else {
                Some(
                    state
                        .dom
                        .borrow_mut()
                        .create_element_ns(&namespace, &qualified_name),
                )
            };
            if let Some((source, node)) = doctype {
                let node = node_migration::adopt(scope, state, document, &source, node)?;
                state.dom.borrow_mut().append_child(document, node)?;
            }
            if let Some(root) = root {
                state.dom.borrow_mut().append_child(document, root)?;
            }
            return Ok(Some(output.into()));
        }
        "createHTMLDocument" => {
            let title = if args.get(0).is_undefined() {
                None
            } else {
                Some(required_string(scope, args, 0)?)
            };
            let mut dom = state.dom.borrow_mut();
            let document = dom.create(NodeKind::Document);
            dom.arena_mut().get_mut(document)?.document_metadata =
                Some(Box::new(oryn_dom::DocumentMetadata::html()));
            let doctype = dom.create(NodeKind::DocumentType {
                name: "html".into(),
                public_id: String::new(),
                system_id: String::new(),
            });
            let html = dom.create_element("html");
            let head = dom.create_element("head");
            let body = dom.create_element("body");
            dom.append_child(document, doctype)?;
            dom.append_child(document, html)?;
            dom.append_child(html, head)?;
            dom.append_child(html, body)?;
            if let Some(title) = title {
                let title_element = dom.create_element("title");
                let title_text = dom.create_text(title);
                dom.append_child(head, title_element)?;
                dom.append_child(title_element, title_text)?;
            }
            drop(dom);
            state.derive_document_provenance(source_document, document)?;
            return Ok(Some(wrap_node(scope, state, document)?.into()));
        }
        "createDocumentType" => {
            if args.length() < 3 {
                return Err(error("TypeError"));
            }
            let qualified_name = required_string(scope, args, 0)?;
            let public_id = required_string(scope, args, 1)?;
            let system_id = required_string(scope, args, 2)?;
            if qualified_name.chars().any(|character| {
                matches!(
                    character,
                    '\0' | '\t' | '\n' | '\u{000c}' | '\r' | ' ' | '>'
                )
            }) {
                return Err(V8BindingError::Unsupported {
                    interface: "DOMImplementation".into(),
                    member: "InvalidCharacterError".into(),
                });
            }
            let node = state.dom.borrow_mut().create(NodeKind::DocumentType {
                name: qualified_name,
                public_id,
                system_id,
            });
            state
                .dom
                .borrow_mut()
                .set_node_document(node, source_document)?;
            return Ok(Some(wrap_node(scope, state, node)?.into()));
        }
        _ => {}
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, code: &str) -> bool {
        super::super::test_eval::eval(scope, code).is_true()
    }
    #[test]
    fn factories_follow_pinned_conversion_and_document_metadata_contracts() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<!doctype html><body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(
            scope,
            r#"(()=>{const check=(ok,message)=>{if(!ok)throw Error(message)},throws=(name,fn)=>{try{fn()}catch(error){check(error.name===name,'exception '+error.name);return}throw Error('missing '+name)};
   const implementation=document.implementation;for(const args of [[],['html'],['html','']])throws('TypeError',()=>implementation.createDocumentType(...args));
   const conversions=[];const type=implementation.createDocumentType({toString(){conversions.push('name');return '1:<'}},{toString(){conversions.push('public');return '\ud800'}},{toString(){conversions.push('system');return '\udfff'}});check(conversions.join()==='name,public,system'&&type.name==='1:<'&&type.publicId.charCodeAt(0)===0xd800&&type.systemId.charCodeAt(0)===0xdfff,'doctype conversion');
   const undefinedIds=implementation.createDocumentType('',undefined,undefined);check(undefinedIds.publicId==='undefined'&&undefinedIds.systemId==='undefined','required DOMString conversion');
   for(const name of ['',':','1name','@name','edi:<','prefix::local'])check(implementation.createDocumentType(name,'','').name===name,'relaxed pinned name');for(const name of ['bad name','bad>','bad\0'])throws('InvalidCharacterError',()=>implementation.createDocumentType(name,'',''));
   throws('TypeError',()=>implementation.createDocument());throws('TypeError',()=>implementation.createDocument(null));throws('TypeError',()=>implementation.createDocument(null,'invalid name',document.createElement('span')));
   for(const [namespace,contentType]of [[null,'application/xml'],['http://www.w3.org/1999/xhtml','application/xhtml+xml'],['http://www.w3.org/2000/svg','image/svg+xml']]){const doc=implementation.createDocument(namespace,null,type);check(doc.doctype===type&&type.ownerDocument===doc&&doc.documentElement===null,'doctype adoption');check(doc.contentType===contentType&&doc.characterSet==='UTF-8'&&doc.URL==='about:blank'&&doc.location===null&&doc.compatMode==='CSS1Compat','created XML metadata');}
   check(implementation.createDocument(null,undefined).documentElement.localName==='undefined','undefined qualifiedName');
   check(implementation.createHTMLDocument().head.childNodes.length===0&&implementation.createHTMLDocument(undefined).head.childNodes.length===0,'absent title');check(implementation.createHTMLDocument(null).title==='null','null title');const titled=implementation.createHTMLDocument();titled.title='  a\n b  ';check(titled.title==='a b'&&titled.head.firstChild.textContent==='  a\n b  ','title whitespace preserves source');titled.title='\ud800';check(titled.title.charCodeAt(0)===0xd800,'title DOMString');const sentinel={};let caught;try{titled.title={toString(){throw sentinel}}}catch(e){caught=e}check(caught===sentinel&&titled.title.charCodeAt(0)===0xd800,'title conversion atomicity');const svg=implementation.createDocument('http://www.w3.org/2000/svg','svg');svg.title=' svg ';check(svg.title==='svg'&&svg.documentElement.firstChild.namespaceURI==='http://www.w3.org/2000/svg','SVG title');check(implementation.hasFeature({toString(){throw Error('must ignore')}}),'legacy ignored input');return true;
  })()"#
        ));
    }
    #[test]
    fn foreign_doctype_is_adopted_with_original_wrapper_and_no_source_replacement() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let source_context = v8::Context::new(scope, Default::default());
        let (source, doctype, source_document) = {
            let scope = &mut v8::ContextScope::new(scope, source_context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<!doctype html><body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state =
                install_generated_bindings(scope, source_context, dom, RealmId(0), 1).unwrap();
            let script = v8::String::new(scope, "document.doctype").unwrap();
            let doctype = v8::Script::compile(scope, script, None)
                .unwrap()
                .run(scope)
                .unwrap();
            let doctype = v8::Global::new(scope, doctype);
            let doc = wrap_node(scope, &state, state.document_root()).unwrap();
            (state, doctype, v8::Global::new(scope, doc))
        };
        let target_context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, target_context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _target =
            install_generated_bindings(scope, target_context, dom, RealmId(1), 1).unwrap();
        for (name, value) in [
            ("foreignType", v8::Local::new(scope, &doctype)),
            (
                "foreignDocument",
                v8::Local::new(scope, &source_document).into(),
            ),
        ] {
            let key = v8::String::new(scope, name).unwrap();
            target_context
                .global(scope)
                .set(scope, key.into(), value)
                .unwrap();
        }
        assert!(eval(
            scope,
            "(()=>{const old=foreignType.ownerDocument;const doc=document.implementation.createDocument('urn:test','p:root',foreignType);if(doc.doctype!==foreignType||foreignType.ownerDocument!==doc||old!==foreignDocument||old.doctype!==null||doc.documentElement.namespaceURI!=='urn:test'||doc.documentElement.prefix!=='p')throw Error('foreign doctype identity');let caught=false;try{document.implementation.createDocument(null,'x',foreignDocument.documentElement)}catch(error){caught=error.name==='TypeError'}return caught})()"
        ));
        assert!(
            source
                .dom
                .borrow()
                .arena()
                .get(source.document_root())
                .is_ok()
        );
    }
}
