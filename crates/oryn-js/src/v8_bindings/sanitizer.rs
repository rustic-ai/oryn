//! Sanitizer Web IDL conversion. No native policy is borrowed or changed while
//! author getters, string coercions or iterators are running.
use super::*;
use oryn_dom::sanitizer::{Config, Element, HTML_NAMESPACE, Name};
type Result<T> = std::result::Result<T, V8BindingError>;
fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Sanitizer".into(),
        member: "TypeError".into(),
    }
}
fn string(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<String> {
    let value = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    rust_string_preserving_utf16(scope, value)
}
fn object<'s>(value: v8::Local<'s, v8::Value>) -> Result<Option<v8::Local<'s, v8::Object>>> {
    if value.is_null_or_undefined() {
        Ok(None)
    } else {
        v8::Local::<v8::Object>::try_from(value)
            .map(Some)
            .map_err(|_| invalid())
    }
}
fn field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: Option<v8::Local<v8::Object>>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>> {
    let Some(object) = object else {
        return Ok(v8::undefined(scope).into());
    };
    let key = v8_string(scope, name)?;
    object
        .get(scope, key)
        .ok_or(V8BindingError::JavaScriptException)
}
fn sequence<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    mut convert: impl FnMut(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Value>) -> Result<T>,
) -> Result<Option<Vec<T>>> {
    if value.is_undefined() {
        return Ok(None);
    }
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
    let key = v8::Symbol::get_iterator(scope);
    let method = object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| invalid())?;
    let mut result = Vec::new();
    iterable_conversion::consume(scope, object, method, |scope, value| {
        let value = convert(scope, value)?;
        result
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("sanitizer sequence"))?;
        result.push(value);
        Ok(())
    })?;
    Ok(Some(result))
}
fn qualified(
    scope: &mut v8::PinScope,
    object: Option<v8::Local<v8::Object>>,
    element: bool,
) -> Result<Name> {
    let value = field(scope, object, "name")?;
    if value.is_undefined() {
        return Err(invalid());
    }
    let name = string(scope, value)?;
    let value = field(scope, object, "namespace")?;
    let namespace = if value.is_undefined() {
        element.then(|| HTML_NAMESPACE.into())
    } else if value.is_null() {
        None
    } else {
        Some(string(scope, value)?)
    };
    Ok(Name::new(name, namespace))
}
pub(super) fn element(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<Name> {
    if value.is_object() || value.is_null_or_undefined() {
        qualified(scope, object(value)?, true)
    } else {
        Ok(Name::element(&string(scope, value)?))
    }
}
pub(super) fn attribute(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<Name> {
    if value.is_object() || value.is_null_or_undefined() {
        qualified(scope, object(value)?, false)
    } else {
        Ok(Name::attribute(&string(scope, value)?))
    }
}
pub(super) fn element_with_attributes(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Element> {
    if !value.is_object() && !value.is_null_or_undefined() {
        return Ok(Element {
            name: Name::element(&string(scope, value)?),
            attributes: None,
            remove_attributes: None,
        });
    }
    let object = object(value)?;
    // Inherited dictionary members precede this dictionary's own members.
    let name = qualified(scope, object, true)?;
    let value = field(scope, object, "attributes")?;
    let attributes = sequence(scope, value, attribute)?;
    let value = field(scope, object, "removeAttributes")?;
    let remove_attributes = sequence(scope, value, attribute)?;
    Ok(Element {
        name,
        attributes,
        remove_attributes,
    })
}
pub(super) fn processing_instruction(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<String> {
    if !value.is_object() && !value.is_null_or_undefined() {
        return string(scope, value);
    }
    let object = object(value)?;
    let value = field(scope, object, "target")?;
    if value.is_undefined() {
        return Err(invalid());
    }
    string(scope, value)
}
/// Full conversion precedes validity checking, including explicit conflicting
/// fields: exceptions and effects of later getters must still occur.
pub(super) fn configuration(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Config> {
    let object = object(value)?;
    let value = field(scope, object, "attributes")?;
    let attributes = sequence(scope, value, attribute)?;
    let value = field(scope, object, "comments")?;
    let comments = (!value.is_undefined()).then(|| value.boolean_value(scope));
    let value = field(scope, object, "dataAttributes")?;
    let data_attributes = (!value.is_undefined()).then(|| value.boolean_value(scope));
    let value = field(scope, object, "elements")?;
    let elements = sequence(scope, value, element_with_attributes)?;
    let value = field(scope, object, "processingInstructions")?;
    let processing_instructions = sequence(scope, value, processing_instruction)?;
    let value = field(scope, object, "removeAttributes")?;
    let remove_attributes = sequence(scope, value, attribute)?;
    let value = field(scope, object, "removeElements")?;
    let remove_elements = sequence(scope, value, element)?;
    let value = field(scope, object, "removeProcessingInstructions")?;
    let remove_processing_instructions = sequence(scope, value, processing_instruction)?;
    let value = field(scope, object, "replaceWithChildrenElements")?;
    let replace_with_children_elements = sequence(scope, value, element)?;
    Ok(Config {
        elements,
        remove_elements,
        replace_with_children_elements,
        processing_instructions,
        remove_processing_instructions,
        attributes,
        remove_attributes,
        comments,
        data_attributes,
    })
}
/// Constructor union conversion selects dictionaries for objects and nullish
/// values, and the sole enum value for other primitives. Undefined defaulting
/// belongs to the operation caller, before this conversion.
pub(super) fn constructor_configuration(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Config> {
    if value.is_object() || value.is_null_or_undefined() {
        configuration(scope, value)?
            .canonicalize(true)
            .map_err(|_| invalid())
    } else if string(scope, value)? == "default" {
        Ok(Config::default_preset())
    } else {
        Err(invalid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn html_sink_domstrings_preserve_surrogates_and_storage_prefix_text() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(value,label)=>{if(!value)throw Error(label)};
            const values=['x\uD800y','x\uDFFFy','\u{1F600}','\u{F0000}oryn-wtf16:[60,98,62]','\u{F0000}\uD800'];
            for(const value of values){
                const markup='<b title="'+value+'">'+value+'</b>';
                for(const parse of [Document.parseHTML,Document.parseHTMLUnsafe]){
                    const parsed=parse(markup),b=parsed.body.firstElementChild;
                    check(b&&b.localName==='b'&&b.textContent===value&&b.getAttribute('title')===value,'document markup code units');
                }
                const host=document.createElement('div'),shadow=host.attachShadow({mode:'open'});
                for(const target of [host,shadow]){
                    for(const method of ['setHTML','setHTMLUnsafe']){
                        target[method](markup);
                        const b=target.firstElementChild;
                        check(b&&b.localName==='b'&&b.textContent===value&&b.getAttribute('title')===value,method+' code units');
                    }
                    target.innerHTML=markup;
                    check(target.firstElementChild.textContent===value&&target.firstElementChild.getAttribute('title')===value,'legacy markup code units');
                }
            }
            const encoded='<b title="&#xF1000;&#xF2000;&#xF0000;">&#xF1000;&#xF2000;&#xF0000;</b>',expected='\u{F1000}\u{F2000}\u{F0000}';
            const parsed=Document.parseHTMLUnsafe(encoded),b=parsed.body.firstElementChild;
            check(b.textContent===expected&&b.getAttribute('title')===expected,'numeric character references cannot impersonate transport markers');
            return true;
        })()"#).is_true());
    }
    #[test]
    fn safe_sinks_mutate_native_instance_at_sanitization_only() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{const make=()=>new Sanitizer({}),blocked=s=>s.get().removeElements.some(x=>x.name==='script'&&x.namespace==='http://www.w3.org/1999/xhtml');
        const sanitizer=make(),host=document.createElement('div');host.setHTMLUnsafe('<p onclick="bad()">unsafe</p>',{sanitizer});if(blocked(sanitizer))return false;
        const marker={},untouched=make();try{host.setHTML({toString(){throw marker}},{sanitizer:untouched})}catch(error){if(error!==marker)return false}if(blocked(untouched))return false;
        document.createElement('script').setHTML('<b/>',{sanitizer:untouched});if(blocked(untouched))return false;
        host.setHTML('<p onclick="bad()">safe</p>',{sanitizer});if(!blocked(sanitizer)||host.firstChild.hasAttribute('onclick')||sanitizer.removeUnsafe())return false;
        const shadowSanitizer=make(),shadow=host.attachShadow({mode:'open'});shadow.setHTML('<i>shadow</i>',{sanitizer:shadowSanitizer});if(!blocked(shadowSanitizer))return false;
        const documentSanitizer=make();Document.parseHTML('<p>doc</p>',{sanitizer:documentSanitizer});if(!blocked(documentSanitizer))return false;
        const dictionary={removeElements:[]};host.setHTML('<b/>',{sanitizer:dictionary});return dictionary.removeElements.length===0;})()"#).is_true());
    }
    #[test]
    fn admitted_data_attribute_modifier_state_filters_without_dictionary_revalidation() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{const sanitizer=new Sanitizer({elements:['html','head','body',{name:'div',removeAttributes:['data-secret']},'span'],attributes:['data-secret'],dataAttributes:false});
        if(!sanitizer.setDataAttributes(true)||sanitizer.setDataAttributes(true))return false;
        const snapshot=sanitizer.get(),rule=snapshot.elements.find(rule=>rule.name==='div');if(snapshot.attributes.length||rule.removeAttributes[0].name!=='data-secret')return false;
        const markup='<div data-secret="blocked" data-other="kept"></div><span data-secret="kept"></span>',host=document.createElement('section');
        for(const safe of [false,true]){if(safe)host.setHTML(markup,{sanitizer});else host.setHTMLUnsafe(markup,{sanitizer});if(host.firstChild.hasAttribute('data-secret')||host.firstChild.getAttribute('data-other')!=='kept'||host.lastChild.getAttribute('data-secret')!=='kept')return false;}
        const parsed=Document.parseHTMLUnsafe(markup,{sanitizer});if(parsed.querySelector('div').hasAttribute('data-secret')||parsed.querySelector('span').getAttribute('data-secret')!=='kept')return false;
        let rejected=false;try{new Sanitizer(snapshot)}catch(error){rejected=error instanceof TypeError}if(!rejected)return false;
        const old=host.firstChild;rejected=false;try{host.setHTMLUnsafe(markup,{sanitizer:snapshot})}catch(error){rejected=error instanceof TypeError}if(!rejected||host.firstChild!==old)return false;
        sanitizer.setDataAttributes(false);host.setHTMLUnsafe(markup,{sanitizer});return !host.firstChild.hasAttribute('data-other')&&!host.lastChild.hasAttribute('data-secret');})()"#).is_true());
    }
    #[test]
    fn xml_markup_getters_use_strict_xml_serialization() {
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
        let _state = install_generated_bindings(scope, context, dom, RealmId(310), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)},doc=new DOMParser().parseFromString('<root/>','application/xml');
            const host=doc.documentElement,ns='http://www.w3.org/1999/xhtml';
            const xmp=doc.createElementNS(ns,'xmp'),span=doc.createElementNS(ns,'span');span.textContent='<';xmp.appendChild(span);host.appendChild(xmp);
            check(host.innerHTML==='<xmp xmlns="'+ns+'"><span>&lt;</span></xmp>','XML raw-text escaping');
            host.replaceChildren(doc.createElementNS(ns,'br'));check(host.innerHTML==='<br xmlns="'+ns+'" />','XML void namespace');
            host.replaceChildren(doc.createElement('test:test'));let rejected=false;try{host.innerHTML}catch(error){rejected=error.name==='InvalidStateError'}check(rejected,'colon local name rejected');
            for(const text of ['\f','\uD800']){host.replaceChildren(doc.createTextNode(text));rejected=false;try{host.innerHTML}catch(error){rejected=error.name==='InvalidStateError'}check(rejected,'invalid XML character rejected');}
            host.replaceChildren();const node=doc.createElementNS('urn:item','p:item');host.appendChild(node);check(node.outerHTML==='<p:item xmlns:p="urn:item"/>','outer XML namespace');
            const html=document.createElement('xmp');html.textContent='<';check(html.innerHTML==='<','HTML raw text remains HTML');
            return true;
        })()"#).boolean_value(scope));
    }
    #[test]
    fn legacy_xml_fragments_preserve_namespaces_and_reject_before_mutation() {
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
        let _state = install_generated_bindings(scope, context, dom, RealmId(309), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const doc=new DOMParser().parseFromString('<root xmlns="urn:default" xmlns:p="urn:outer"><context xmlns:p="urn:inner"/></root>','application/xml');const context=doc.documentElement.firstChild;
            context.innerHTML='<p:item p:key="value"/><plain/><![CDATA[text]]><?target data?><!--comment-->';
            check(context.childNodes.length===5&&context.firstChild.namespaceURI==='urn:inner'&&context.firstChild.getAttributeNS('urn:inner','key')==='value','prefix inheritance');
            check(context.childNodes[1].namespaceURI==='urn:default'&&context.childNodes[2].nodeType===4&&context.childNodes[3].nodeType===7&&context.childNodes[4].nodeType===8,'default namespace and character kinds');
            const old=context.firstChild;for(const markup of ['<unclosed>','<missing:item/>','<x a="1" a="2"/>','\uD800','<?xml version="1.0"?><x/>']){let rejected=false;try{context.innerHTML=markup}catch(error){rejected=error.name==='SyntaxError'}check(rejected&&context.firstChild===old&&context.childNodes.length===5,'XML failure before replacement');}
            old.outerHTML='<p:replacement/>';check(context.firstChild.namespaceURI==='urn:inner'&&context.firstChild.localName==='replacement','XML outer context');
            context.insertAdjacentHTML('beforeend','<p:adjacent/>');check(context.lastChild.namespaceURI==='urn:inner','XML adjacent context');
            context.innerHTML='<plain xmlns=""/>';check(context.firstChild.namespaceURI===null,'explicit default reset');
            const fragment=doc.createDocumentFragment(),child=doc.createElement('old');fragment.appendChild(child);child.outerHTML='<replacement/>';check(fragment.firstChild.namespaceURI==='http://www.w3.org/1999/xhtml'&&fragment.firstChild.ownerDocument===doc,'XML fragment parent uses synthetic HTML body');
            let malformed=false;try{fragment.firstChild.outerHTML='<broken>'}catch(error){malformed=error.name==='SyntaxError'}check(malformed&&fragment.firstChild.localName==='replacement','synthetic body retains XML parsing');
            return true;
        })()"#).boolean_value(scope));
    }
    #[test]
    fn legacy_html_setters_preserve_context_inertness_and_conversion_effects() {
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
        let _state = install_generated_bindings(scope, context, dom, RealmId(307), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');document.body.appendChild(svg);svg.innerHTML='<circle/>';check(svg.firstChild.namespaceURI===svg.namespaceURI,'inner SVG');svg.firstChild.outerHTML='<rect/>';check(svg.firstChild.localName==='rect'&&svg.firstChild.namespaceURI===svg.namespaceURI,'outer SVG');
            const host=document.createElement('div');document.body.appendChild(host);globalThis.legacyRan=0;host.innerHTML='<script>legacyRan++<'+ '/script><template shadowrootmode="open"><b>inert</b></template>';check(!host.shadowRoot&&host.lastChild.content.firstChild.localName==='b','legacy declarative disabled');const script=host.firstChild;script.remove();document.body.appendChild(script);check(legacyRan===0,'legacy script inert');
            const template=document.createElement('template'),content=template.content;template.innerHTML='<i>template</i>';check(template.content===content&&content.firstChild.localName==='i','stable template content');
            const shadow=host.attachShadow({mode:'open'});shadow.innerHTML='<b>shadow</b>';check(shadow.firstChild.textContent==='shadow','shadow context');shadow.innerHTML=null;check(shadow.childNodes.length===0,'null is empty');
            const detached=document.createElement('b');let conversions=0;detached.outerHTML={toString(){conversions++;return '<i></i>'}};check(conversions===1&&!detached.parentNode,'detached conversion/no-op');
            const left=document.createElement('div'),right=document.createElement('div'),node=document.createElement('i');left.appendChild(node);document.body.append(left,right);node.outerHTML={toString(){right.appendChild(node);return '<b>replacement</b>'}};check(left.childNodes.length===0&&right.firstChild.localName==='b'&&!node.parentNode,'outer conversion movement');
            let rejected=false;try{document.documentElement.outerHTML='<html></html>'}catch(error){rejected=error.name==='NoModificationAllowedError'}check(rejected,'Document parent');
            return true;
        })()"#).boolean_value(scope));
    }
    #[test]
    fn adjacent_html_uses_native_context_and_one_inert_fragment() {
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
        let _state = install_generated_bindings(scope, context, dom, RealmId(306), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const table=document.createElement('table');document.body.appendChild(table);table.insertAdjacentHTML('beforeend','<tr><td>cell</td></tr>');
            check(table.firstChild.localName==='tbody'&&table.firstChild.firstChild.firstChild.textContent==='cell','table context without wrapper');
            const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');document.body.appendChild(svg);svg.insertAdjacentHTML('afterbegin','<circle cx="3"/>');check(svg.firstChild.namespaceURI===svg.namespaceURI&&svg.firstChild.localName==='circle','SVG context');
            const host=document.createElement('div');document.body.appendChild(host);const observer=new MutationObserver(()=>{});observer.observe(host,{childList:true});globalThis.adjacentRan=0;
            host.insertAdjacentHTML('beforeend','<b>first</b><script>adjacentRan++<'+ '/script><template shadowrootmode="open"><i>hidden</i></template>');
            const records=observer.takeRecords();check(records.length===1&&records[0].addedNodes.length===3&&host.children.length===3,'one insertion mutation');check(host.firstChild.localName==='b'&&!host.shadowRoot&&host.lastChild.content.firstChild.localName==='i','no synthetic or declarative root');
            const script=host.children[1];script.remove();document.body.appendChild(script);check(adjacentRan===0,'fragment scripts stay inert after reconnect');
            const left=document.createElement('div'),right=document.createElement('div'),anchor=document.createElement('i');left.appendChild(anchor);document.body.append(left,right);let conversions=0;
            anchor.insertAdjacentHTML('beforebegin',{toString(){conversions++;right.appendChild(anchor);return '<b>moved</b>'}});check(conversions===1&&left.childNodes.length===0&&right.firstChild.localName==='b'&&right.lastChild===anchor,'resolve insertion point after conversion');
            let rejected=false;try{document.documentElement.insertAdjacentHTML('beforebegin','<p>invalid</p>')}catch(error){rejected=error.name==='NoModificationAllowedError'}check(rejected,'Document parent forbidden');
            return true;
        })()"#).boolean_value(scope));
    }
    #[test]
    fn sanitizer_get_sorts_all_lists_by_namespace_and_utf16() {
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
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const names=list=>list.map(x=>x.name).join(',');
            const policy=new Sanitizer({elements:[
                {name:'z',namespace:null},
                {name:'div',attributes:['c','b'],removeAttributes:['z','a']},
                {name:'a',namespace:null}
            ],attributes:['z','a'],dataAttributes:false,processingInstructions:['\uE000','\u{10000}','a']});
            let snapshot=policy.get();
            check(names(snapshot.elements)==='a,z,div','null namespace before HTML');
            check(names(snapshot.attributes)==='a,z','global allow list');
            check(names(snapshot.elements[2].attributes)==='b,c','local allow list');
            check(names(snapshot.elements[2].removeAttributes)==='a,z','local remove list');
            check(snapshot.processingInstructions.map(x=>x.target).join(',')==='a,\u{10000},\uE000','PI UTF16 ordering');
            snapshot.elements.reverse();snapshot.elements[0].attributes.reverse();
            check(names(policy.get().elements)==='a,z,div'&&names(policy.get().elements[2].attributes)==='b,c','fresh nested copies');
            const removed=new Sanitizer({removeElements:[
                {name:'a',namespace:'z'},{name:'z',namespace:''},{name:'a',namespace:null},{name:'b',namespace:'a'}
            ],replaceWithChildrenElements:['section','div'],removeAttributes:['\uE000','\u{10000}','\uD800'],removeProcessingInstructions:['z','a']}).get();
            check(names(removed.removeElements)==='a,z,b,a','namespace then local name');
            check(removed.removeElements[0].namespace===null&&removed.removeElements[1].namespace===null,'empty namespace canonicalized');
            check(names(removed.replaceWithChildrenElements)==='div,section','replacement list');
            check(names(removed.removeAttributes)==='\uD800,\u{10000},\uE000','isolated surrogate and astral ordering');
            check(removed.removeProcessingInstructions.map(x=>x.target).join(',')==='a,z','removed PI list');
            return true;
        })()"#).boolean_value(scope));
    }
    #[test]
    fn sanitizer_native_instances_copy_get_results_and_allow_reentrant_conversion() {
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
        assert!(eval(scope,r#"(()=>{
            const check=(value,message)=>{if(!value)throw Error(message)};
            const s=new Sanitizer({elements:['div'],attributes:[],comments:false});
            check(s instanceof Sanitizer,'native prototype');
            const config=s.get();config.elements[0].name='script';config.attributes.push('onclick');
            check(s.get().elements[0].name==='div'&&s.get().attributes.length===0,'fresh dictionaries');
            check(s.allowElement({get name(){s.allowElement('span');return 'section'}}),'reentrant modification');
            check(s.get().elements.map(x=>x.name).join(',')==='div,section,span','no lost reentrant edit');
            const marker={};let caught;
            try{s.allowElement({get name(){s.removeElement('span');throw marker}})}catch(error){caught=error}
            check(caught===marker&&s.get().elements.map(x=>x.name).join(',')==='div,section','exception preserves prior author edit');
            let touched=false;try{Sanitizer.prototype.allowElement.call({}, {get name(){touched=true;return 'x'}})}catch(error){check(error instanceof TypeError,'brand error')}
            check(!touched,'brand before conversion');
            check(s.setComments(true)&&!s.setComments(true),'boolean change result');
            check(s.allowProcessingInstruction('pi')===false,'permissive PI mode');
            check(s.removeProcessingInstruction({target:'pi'})&&!s.removeProcessingInstruction('pi'),'PI modifications');
            const unsafe=new Sanitizer({});check(unsafe.removeUnsafe()&&!unsafe.removeUnsafe(),'baseline changes');
            check(unsafe.get().removeElements.length===8,'qualified baseline');
            const preset=new Sanitizer();check(!preset.removeUnsafe(),'default unchanged');
            class Derived extends Sanitizer{};check(new Derived() instanceof Derived,'subclass');
            let writes=0;Object.defineProperty(Object.prototype,'name',{set(){writes++},configurable:true});
            try{check(s.get().elements[0].name==='div'&&writes===0,'get bypasses prototype setters')}finally{delete Object.prototype.name}
            return true;
        })()"#).boolean_value(scope));
    }
    #[test]
    fn sink_options_preserve_defaults_and_member_order_without_early_validation() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let input = eval(scope, "({})");
        let safe = options(scope, input, true, true).unwrap();
        assert_eq!(safe.configuration(true).unwrap(), Config::default_preset());
        assert!(!safe.run_scripts);
        let unsafe_options = options(scope, input, false, true).unwrap();
        let config = unsafe_options.configuration(false).unwrap();
        assert!(config.elements.is_none());
        assert_eq!(config.comments, Some(true));
        let input = eval(
            scope,
            r#"(()=>{globalThis.order=[];return {get runScripts(){order.push('runScripts');return true},get sanitizer(){order.push('sanitizer');return {elements:[],removeElements:[]}}}})()"#,
        );
        let pending = options(scope, input, false, true).unwrap();
        assert!(pending.run_scripts);
        assert!(pending.configuration(false).is_err());
        let order = eval(scope, "order.join(',')");
        assert_eq!(string(scope, order).unwrap(), "runScripts,sanitizer");
        let input = eval(
            scope,
            "({get runScripts(){throw Error('must not read')},sanitizer:{}})",
        );
        assert!(!options(scope, input, true, true).unwrap().run_scripts);
        assert!(!options(scope, input, false, false).unwrap().run_scripts);
    }
    #[test]
    fn foreign_sanitizer_options_retain_live_configuration_and_ignore_public_get() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let foreign = v8::Context::new(scope, Default::default());
        let (_owner, value) = {
            let scope = &mut v8::ContextScope::new(scope, foreign);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let owner = install_generated_bindings(scope, foreign, dom, RealmId(71), 1).unwrap();
            let value = eval(
                scope,
                "globalThis.s=new Sanitizer({elements:['div'],attributes:[]});s.get=()=>{throw Error('public get')};({sanitizer:s})",
            );
            (owner, v8::Global::new(scope, value))
        };
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(72), 1).unwrap();
        let value = v8::Local::new(scope, &value);
        let converted = options(scope, value, false, true).unwrap();
        {
            let scope = &mut v8::ContextScope::new(scope, foreign);
            eval(scope, "s.allowElement('span')");
        }
        let config = converted.configuration(false).unwrap();
        assert_eq!(
            config
                .elements
                .unwrap()
                .iter()
                .map(|rule| rule.name.name.as_str())
                .collect::<Vec<_>>(),
            vec!["div", "span"]
        );
    }
    #[test]
    fn sink_filtering_precedes_scoped_fragment_patches() {
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
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const markup='<div id="target"><?start name="p">old<?end></div><template for="p"><b>new</b></template>';
            for(const method of ['setHTMLUnsafe','appendHTMLUnsafe','prependHTMLUnsafe']){
                const host=document.createElement('section');document.body.append(host);
                host[method](markup,{sanitizer:{removeElements:['template']}});check(host.querySelector('#target').textContent==='old'&&!host.querySelector('template'),'drop template before patch');host.replaceChildren();
                host[method](markup,{sanitizer:{removeAttributes:['for']}});check(host.querySelector('#target').textContent==='old'&&host.querySelector('template'),'drop for before patch');host.replaceChildren();
                host[method](markup);check(host.querySelector('#target').textContent==='new'&&!host.querySelector('template'),'permitted fragment patch');host.remove();
            }
            const outside=document.createElement('section');outside.innerHTML='<?start name="outside">old<?end>';document.body.append(outside);
            const host=document.createElement('div');document.body.append(host);host.appendHTMLUnsafe('<template for="outside"><b>bad</b></template>');
            check(outside.textContent==='old'&&host.querySelector('template'),'fragment cannot patch outside target');
            const dropped=Document.parseHTMLUnsafe(markup,{sanitizer:{removeElements:['template']}});check(dropped.querySelector('#target').textContent==='old','Document filter before patch');
            const patched=Document.parseHTMLUnsafe(markup);check(patched.querySelector('#target').textContent==='new','Document permitted patch');
            return true;
        })()"#).is_true());
    }
    #[test]
    fn detached_html_documents_keep_encoding_base_style_and_script_inertness() {
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
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state
            .dom
            .borrow_mut()
            .arena_mut()
            .get_mut(state.document_root())
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .url = Some("https://origin.test/path/page".into());
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};globalThis.detachedRuns=0;
            for(const parse of [Document.parseHTML,Document.parseHTMLUnsafe]){
                const doc=parse('<meta charset="windows-1252"><div style="color: red">text</div><script>detachedRuns++<'+ '/script>');
                check(doc.URL==='about:blank'&&doc.documentURI==='about:blank'&&doc.baseURI==='about:blank'&&doc.defaultView===null,'inert document URL/base');
                check(doc.characterSet==='UTF-8'&&doc.charset==='UTF-8'&&doc.inputEncoding==='UTF-8','DOMString parser encoding');
                const div=doc.querySelector('div');
                if(parse===Document.parseHTMLUnsafe){
                    check(div.style.color==='red'&&div.style.cssText==='color: red;'&&div.getAttribute('style')==='color: red','parsed style declaration');
                    div.setAttribute('style','color:: invalid');check(div.style.color===''&&div.style.cssText==='','invalid style refresh');
                    div.setAttribute('style','color: green');div.style.backgroundColor='blue';check(div.style.color==='green'&&div.style.backgroundColor==='blue','style mutation preserves existing declarations');
                    document.body.append(doc.querySelector('script'));check(detachedRuns===0,'adopted script remains inert');
                }
                const based=parse('<base href="https://base.test/directory/"><a href="next">next</a>',{sanitizer:{}});
                if(parse===Document.parseHTMLUnsafe)check(based.URL==='about:blank'&&based.baseURI==='https://base.test/directory/','base changes fallback only');
            }return true;
        })()"#).is_true());
    }
    #[test]
    fn document_html_parsers_apply_native_policy_and_preserve_conversion_order() {
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
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const markup='<div onclick="bad()"><script>globalThis.executed=true<'+ '/script><a href="javascript:0">link</a></div>';
            const safe=Document.parseHTML(markup,{sanitizer:{}});
            check(!safe.querySelector('script')&&!safe.querySelector('div').hasAttribute('onclick')&&!safe.querySelector('a').hasAttribute('href'),'safe mandatory baseline');
            const unsafe=Document.parseHTMLUnsafe(markup);
            check(unsafe.querySelector('script')&&unsafe.querySelector('div').hasAttribute('onclick')&&unsafe.querySelector('a').getAttribute('href')==='javascript:0','unsafe permissive default');
            check(!globalThis.executed,'detached script inert');
            check(safe.URL==='about:blank'&&unsafe.contentType==='text/html'&&unsafe.defaultView===null,'detached metadata');
            const sanitizer=new Sanitizer({removeElements:['a']});const filtered=Document.parseHTMLUnsafe(markup,{sanitizer});
            check(!filtered.querySelector('a')&&filtered.querySelector('script'),'custom instance');
            check(sanitizer.get().removeElements.length===1,'instance not rewritten');
            const shadow=Document.parseHTMLUnsafe('<div><template shadowrootmode=open shadowrootserializable><a href="javascript:0">shadow</a></template></div>');
            check(shadow.querySelector('div').shadowRoot.querySelector('a'),'declarative parsing');
            const clean=Document.parseHTML('<div><template shadowrootmode=open><a href="javascript:0">shadow</a></template></div>',{sanitizer:{}});
            check(!clean.querySelector('div').shadowRoot.querySelector('a').hasAttribute('href'),'shadow filtering');
            const order=[];Document.parseHTMLUnsafe({toString(){order.push('html');return '<b>x</b>'}},{get sanitizer(){order.push('sanitizer');return {}},get runScripts(){throw Error('not a parse option')}});
            check(order.join(',')==='html,sanitizer','conversion order');
            const marker={};let caught;try{Document.parseHTMLUnsafe('<b>x</b>',{get sanitizer(){throw marker}})}catch(error){caught=error}
            check(caught===marker,'options exception identity');
            check(Document.parseHTML.length===1&&Document.parseHTMLUnsafe.length===1,'static arity');
            check(Document.parseHTML.name==='parseHTML'&&Document.parseHTMLUnsafe.name==='parseHTMLUnsafe','static names');
            for(const method of [Document.parseHTML,Document.parseHTMLUnsafe]){let converted=false,rejected=false;try{new method({toString(){converted=true;return ''}})}catch(error){rejected=error instanceof TypeError}check(rejected&&!converted,'static methods are not constructors')} 
            return true;
        })()"#).boolean_value(scope));
    }
    #[test]
    fn html_setters_filter_before_replacement_and_use_fragment_script_lifecycle() {
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
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)},host=document.createElement('div');document.body.append(host);
            const shadow=host.attachShadow({mode:'open'});
            for(const target of [host,shadow]){
                target.setHTML('<b onclick="bad()">safe</b><script>throw Error("unsafe")<'+ '/script>',{sanitizer:{}});
                check(target.querySelector('b')&&!target.querySelector('b').hasAttribute('onclick')&&!target.querySelector('script'),'safe setter');
                const previous=target.firstChild;let failed=false;
                try{target.setHTMLUnsafe('replacement',{sanitizer:{elements:[],removeElements:[]}})}catch(error){failed=error instanceof TypeError}
                check(failed&&target.firstChild===previous,'invalid policy before replacement');
                globalThis.setterRuns=0;target.setHTMLUnsafe('<script>setterRuns++<'+ '/script>');check(setterRuns===0,'default inert');
                const old=target.firstChild;old.remove();document.body.append(old);check(setterRuns===0,'moving inert scripts');old.remove();
                target.setHTMLUnsafe('<script>setterRuns++<'+ '/script>',{runScripts:true});check(setterRuns===1,'inline fragment execution');
                target.setHTMLUnsafe('<section><template shadowrootmode=open><script>setterRuns++<'+ '/script></template></section>',{runScripts:true});check(setterRuns===2,'declarative shadow script');
                target.setHTMLUnsafe('<template><script>setterRuns++<'+ '/script></template>',{runScripts:true});check(setterRuns===2,'template scripts inert');
                const templateScript=target.querySelector('template').content.firstChild;document.body.append(templateScript);check(setterRuns===2,'template parser state survives extraction');templateScript.remove();
            }
            check(host.shadowRoot===shadow,'existing shadow preserved');
            const script=document.createElement('script');script.textContent='old';script.setHTML('new',{sanitizer:{elements:[],removeElements:[]}});check(script.textContent==='old','safe script no-op precedes validation');
            const template=document.createElement('template');template.setHTMLUnsafe('<i>content</i>');check(template.childNodes.length===0&&template.content.firstChild.localName==='i','template target');
            const order=[];host.setHTMLUnsafe({toString(){order.push('html');return '<em>x</em>'}},{get runScripts(){order.push('runScripts');return false},get sanitizer(){order.push('sanitizer');return {}}});
            check(order.join(',')==='html,runScripts,sanitizer','setter conversion order');
            return true;
        })()"#).boolean_value(scope));
    }
    #[test]
    fn html_setter_re_resolves_adopted_receiver_and_its_template_contents() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let source = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(81), 1).unwrap()
        };
        let second = v8::Context::new(scope, Default::default());
        let destination = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(82), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let document = wrap_node(scope, &destination, destination.document_root()).unwrap();
        let key = v8::String::new(scope, "destination").unwrap();
        first
            .global(scope)
            .set(scope, key.into(), document.into())
            .unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const host=document.createElement('div');document.body.append(host);
            host.setHTMLUnsafe({toString(){destination.adoptNode(host);destination.body.append(host);return '<template shadowrootmode=open><b>shadow</b></template><i>light</i>'}});
            check(host.ownerDocument===destination&&host.shadowRoot.firstChild.textContent==='shadow'&&host.firstChild.localName==='i','receiver and top-level shadow follow adoption');
            const template=document.createElement('template'),contents=template.content;
            template.setHTMLUnsafe('<em>new</em>',{get sanitizer(){destination.adoptNode(template);return {}}});
            check(template.ownerDocument===destination&&template.content===contents&&contents.firstChild.localName==='em','template content identity follows conversion adoption');
            const shadow=host.shadowRoot;shadow.setHTML('<a href="javascript:0">safe</a>',{get sanitizer(){document.adoptNode(host);document.body.append(host);return {}}});
            check(shadow.firstChild.ownerDocument===document&&!shadow.firstChild.hasAttribute('href'),'shadow owner after options adoption');
            return true;
        })()"#).boolean_value(scope));
        let _ = source;
    }
    #[test]
    fn fragment_scripts_observe_complete_dom_and_skip_removed_following_scripts() {
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
        assert!(eval(scope,r##"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};globalThis.target=document.createElement('div');document.body.append(target);globalThis.order=[];
            target.setHTMLUnsafe('<script>order.push(target.querySelector("#after").textContent);target.querySelector("#next").remove()<'+ '/script><script id=next>order.push("removed")<'+ '/script><span id=after>complete</span>',{runScripts:true});
            check(order.join(',')==='complete','whole tree visible and removed suffix skipped');
            target.setHTMLUnsafe('<script>order.push("filtered")<'+ '/script>',{runScripts:true,sanitizer:{removeElements:['script']}});check(order.length===1,'filter precedes script admission');
            const prefix='x-sanitized-order';customElements.define(prefix,class extends HTMLElement{constructor(){super();order.push('constructor')}connectedCallback(){order.push('connected')}});
            target.setHTMLUnsafe('<'+prefix+'></'+prefix+'><script>order.push("script")<'+ '/script>',{runScripts:true});
            check(order.join(',')==='complete,script,constructor,connected','fragment scripts precede upgrade reactions');
            delete globalThis.target;return true;
        })()"##).boolean_value(scope));
    }
    #[test]
    fn fragment_external_and_module_scripts_use_captured_native_admission() {
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
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/page").unwrap();
        eval(
            scope,
            r#"globalThis.host=document.createElement('div');document.body.append(host);
            host.setHTMLUnsafe('<script src="/inert.js"><'+ '/script>');
            host.setHTMLUnsafe('<script src="/filtered.js"><'+ '/script>',{runScripts:true,sanitizer:{removeElements:['script']}});
        "#,
        );
        assert!(!state.has_pending_dynamic_scripts());
        eval(
            scope,
            r#"host.setHTMLUnsafe('<script src="/classic.js" integrity="sha256-before" crossorigin="anonymous"><'+ '/script><section><template shadowrootmode=open><script async src="/shadow.js"><'+ '/script></template></section><script type=module>export default 1<'+ '/script><template><script src="/template.js"><'+ '/script></template>',{runScripts:true});
            host.querySelector('script').setAttribute('integrity','sha256-after');
        "#,
        );
        let scripts = state.take_pending_dynamic_scripts();
        assert_eq!(scripts.len(), 3);
        assert_eq!(scripts[0].src.as_deref(), Some("/classic.js"));
        assert_eq!(scripts[0].integrity.as_deref(), Some("sha256-before"));
        assert_eq!(scripts[0].cross_origin.as_deref(), Some("anonymous"));
        assert_eq!(scripts[1].src.as_deref(), Some("/shadow.js"));
        assert!(scripts[1].asynchronous);
        assert!(scripts[2].module);
        assert_eq!(scripts[2].source.as_source().raw(), "export default 1");
        assert!(scripts[2].src.is_none());
        assert!(
            scripts
                .iter()
                .all(|script| script.base_url == "https://example.test/page"
                    && script.owner_is_current(&state))
        );
        eval(scope, "host.remove();document.body.append(host)");
        assert!(!state.has_pending_dynamic_scripts());
    }
    #[test]
    fn html_sink_trusted_policy_follows_options_and_precedes_instance_snapshot() {
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
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.configure_trusted_types(TrustedTypesConfig {
            require_script: true,
            allowed_policy_names: ["default".into(), "explicit".into()].into(),
            ..TrustedTypesConfig::default()
        });
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)},host=document.createElement('div'),order=[];document.body.append(host);
            let rejected=false;try{host.setHTMLUnsafe('<b>x</b>',{get sanitizer(){order.push('options');return {}}})}catch(error){rejected=error instanceof TypeError}
            check(rejected&&order.join(',')==='options'&&!host.firstChild,'options precede trusted rejection');
            host.setHTML('<b>safe</b>');check(host.textContent==='safe','safe sink does not require TrustedHTML');
            const sanitizer=new Sanitizer({});order.length=0;
            trustedTypes.createPolicy('default',{createHTML(value,type,sink){order.push(sink);sanitizer.removeElement('b');return value}});
            host.setHTMLUnsafe({toString(){order.push('html');return '<b>remove</b><i>keep</i>'}},{get runScripts(){order.push('runScripts');return false},get sanitizer(){order.push('options');return sanitizer}});
            check(order.join(',')==='html,runScripts,options,Element setHTMLUnsafe','policy timing');check(!host.querySelector('b')&&host.textContent==='keep','live config snapshot after policy');
            const explicit=trustedTypes.createPolicy('explicit',{createHTML:value=>value}).createHTML('<b>explicit</b>');order.length=0;
            host.setHTMLUnsafe(explicit);check(host.textContent==='explicit'&&order.length===0,'branded input bypasses default policy');
            return true;
        })()"#).boolean_value(scope));
    }
    #[test]
    fn top_level_shadow_scripts_precede_light_tree_scripts() {
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
        assert!(eval(scope,r#"(()=>{
            const host=document.createElement('div');document.body.append(host);globalThis.shadowOrder=[];
            host.setHTMLUnsafe('<script>shadowOrder.push("light")<'+ '/script><template shadowrootmode=open><script>shadowOrder.push("shadow")<'+ '/script></template>',{runScripts:true});
            if(shadowOrder.join(',')!=='shadow,light')throw Error('shadow-including insertion order');
            return true;
        })()"#).boolean_value(scope));
    }
    #[test]
    fn html_sinks_keep_html_parsing_rules_in_detached_and_xml_documents() {
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
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            const documentCopy=Document.parseHTMLUnsafe('<body><noscript><p id=one>one<p id=two>two</noscript><noembed>&lt;a&gt;</noembed>');
            check(documentCopy.querySelector('noscript').children.length===2,'scripting disabled for detached noscript');
            check(documentCopy.querySelector('noembed').textContent==='&lt;a&gt;','noembed raw text');
            check(documentCopy.compatMode==='BackCompat'&&Document.parseHTMLUnsafe('<!doctype html>').compatMode==='CSS1Compat','doctype mode');
            check(documentCopy.location===null&&documentCopy.defaultView===null,'detached browsing context');
            for(const namespace of [null,'http://www.w3.org/2000/svg']){
                const xml=document.implementation.createDocument(namespace,'root',null),root=xml.documentElement;
                root.setHTMLUnsafe('<p><foo><b><i>test</b></i>');
                check(root.firstChild.namespaceURI==='http://www.w3.org/1999/xhtml','HTML foreign-content breakout');
                check(root.firstChild.localName==='p'&&root.querySelector('i').textContent==='test','HTML error recovery in XML document');
                check(root.firstChild.ownerDocument===xml,'original XML document owner');
            }
            return true;
        })()"#).boolean_value(scope));
    }
    #[test]
    fn declarative_shadow_import_never_appears_in_host_child_list_records() {
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
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            for(const withLight of [false,true]){
                const host=document.createElement('div');document.body.append(host);
                const observer=new MutationObserver(()=>{});observer.observe(host,{childList:true,subtree:true});
                host.setHTMLUnsafe('<template shadowrootmode=open><b>shadow</b></template>'+(withLight?'<i>light</i>':''));
                const records=observer.takeRecords();check(host.shadowRoot.firstChild.localName==='b','shadow attached');
                check(records.length===(withLight?1:0),'only light child insertion is observable');
                if(withLight){check(records[0].target===host&&records[0].addedNodes.length===1&&records[0].addedNodes[0]===host.firstChild,'light addedNodes');check(records[0].removedNodes.length===0,'no removed shadow nodes')}
                observer.disconnect();host.remove();
            }
            return true;
        })()"#).boolean_value(scope));
    }
    #[test]
    fn dictionary_conversion_finishes_in_webidl_order_before_validation() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let input = eval(
            scope,
            r#"(()=>{
            globalThis.trace=[];
            const element=new Proxy({name:'DIV',namespace:'',attributes:['class'],removeAttributes:[]},{get(o,k){trace.push('element.'+String(k));return o[k]}});
            return new Proxy({attributes:[],comments:0,dataAttributes:1,elements:[element],processingInstructions:['pi'],removeAttributes:[],removeElements:[],removeProcessingInstructions:[],replaceWithChildrenElements:[]},{get(o,k){trace.push(String(k));return o[k]}});
        })()"#,
        );
        let config = configuration(scope, input).unwrap();
        assert_eq!(
            config.elements.as_ref().unwrap()[0].name,
            Name::new("DIV".into(), None)
        );
        assert_eq!(config.comments, Some(false));
        assert_eq!(config.data_attributes, Some(true));
        assert!(config.validate().is_err());
        let trace = eval(scope, "trace.join(',')");
        assert_eq!(
            string(scope, trace).unwrap(),
            "attributes,comments,dataAttributes,elements,element.name,element.namespace,element.attributes,element.removeAttributes,processingInstructions,removeAttributes,removeElements,removeProcessingInstructions,replaceWithChildrenElements"
        );
    }
    #[test]
    fn nested_sequence_conversion_closes_iterators_on_required_member_failure() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let input = eval(
            scope,
            r#"(()=>{
            globalThis.closed=[];
            const once=(value,label)=>({[Symbol.iterator](){return {next(){return {value,done:false}},return(){closed.push(label);throw Error('ignored close')}}}});
            return {elements:once({name:'div',attributes:once({namespace:null},'attribute')},'element')};
        })()"#,
        );
        assert!(configuration(scope, input).is_err());
        let closed = eval(scope, "closed.join(',')");
        assert_eq!(string(scope, closed).unwrap(), "attribute,element");
    }
    #[test]
    fn dictionary_getter_exception_survives_iterator_close_exception() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let input = eval(
            scope,
            r#"(()=>{globalThis.marker={original:true};globalThis.closed=0;return {elements:{[Symbol.iterator](){return {next(){return {done:false,value:{get name(){throw marker}}}},return(){closed++;throw Error('close')}}}}}})()"#,
        );
        {
            v8::tc_scope!(let tc,scope);
            assert!(matches!(
                configuration(tc, input),
                Err(V8BindingError::JavaScriptException)
            ));
            let exception = tc.exception().unwrap();
            let key = v8::String::new(tc, "marker").unwrap();
            let marker = context.global(tc).get(tc, key.into()).unwrap();
            assert!(exception.strict_equals(marker));
            tc.reset();
        }
        assert_eq!(eval(scope, "closed").int32_value(scope), Some(1));
    }
    #[test]
    fn sanitizer_union_preserves_presence_case_utf16_and_namespace_defaults() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let input = eval(
            scope,
            "({elements:['DIV',{name:'b'},{name:'i',namespace:null}],attributes:['TITLE',{name:'x',namespace:''}],comments:false})",
        );
        let config = constructor_configuration(scope, input).unwrap();
        let elements = config.elements.unwrap();
        assert_eq!(elements[0].name, Name::element("DIV"));
        assert_eq!(elements[1].name, Name::element("b"));
        assert_eq!(elements[2].name, Name::new("i".into(), None));
        assert_eq!(
            config.attributes,
            Some(vec![Name::attribute("TITLE"), Name::attribute("x")])
        );
        let input = eval(scope, "'default'");
        assert_eq!(
            constructor_configuration(scope, input).unwrap(),
            Config::default_preset()
        );
        for expression in [
            "'DEFAULT'",
            "42",
            "({attributes:null})",
            "({elements:[null]})",
            "({elements:[{}]})",
            "({elements:[{name:'x',attributes:'title'}]})",
        ] {
            let input = eval(scope, expression);
            assert!(
                constructor_configuration(scope, input).is_err(),
                "{expression}"
            );
        }
        let input = eval(scope, "({name:'x\\ud800',namespace:'urn:\\udfff'})");
        let name = element(scope, input).unwrap();
        let name_value = v8_string(scope, &name.name).unwrap();
        let namespace_value = v8_string(scope, name.namespace.as_deref().unwrap()).unwrap();
        let expected = eval(scope, "'x\\ud800'");
        assert!(name_value.strict_equals(expected));
        let expected = eval(scope, "'urn:\\udfff'");
        assert!(namespace_value.strict_equals(expected));
    }
}

fn put(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    value: v8::Local<v8::Value>,
) -> Result<()> {
    let key = v8::String::new(scope, name)
        .ok_or(V8BindingError::Allocation("Sanitizer dictionary key"))?;
    object
        .create_data_property(scope, key.into(), value)
        .filter(|ok| *ok)
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(())
}
fn name_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &Name,
) -> Result<v8::Local<'s, v8::Value>> {
    let object = v8::Object::new(scope);
    let value = v8_string(scope, &name.name)?;
    put(scope, object, "name", value)?;
    let namespace = match &name.namespace {
        Some(namespace) => v8_string(scope, namespace)?,
        None => v8::null(scope).into(),
    };
    put(scope, object, "namespace", namespace)?;
    Ok(object.into())
}
fn list_value<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[T],
    mut convert: impl FnMut(&mut v8::PinScope<'s, '_>, &T) -> Result<v8::Local<'s, v8::Value>>,
) -> Result<v8::Local<'s, v8::Value>> {
    let length = i32::try_from(values.len())
        .map_err(|_| V8BindingError::Allocation("Sanitizer get sequence"))?;
    let array = v8::Array::new(scope, length);
    for (index, value) in values.iter().enumerate() {
        let value = convert(scope, value)?;
        let key = v8::String::new(scope, &index.to_string())
            .ok_or(V8BindingError::Allocation("Sanitizer sequence index"))?;
        array
            .create_data_property(scope, key.into(), value)
            .filter(|ok| *ok)
            .ok_or(V8BindingError::JavaScriptException)?;
    }
    Ok(array.into())
}
fn element_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: &Element,
) -> Result<v8::Local<'s, v8::Value>> {
    let value = name_value(scope, &element.name)?;
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
    for (key, list) in [
        ("attributes", &element.attributes),
        ("removeAttributes", &element.remove_attributes),
    ] {
        if let Some(list) = list {
            let value = list_value(scope, list, name_value)?;
            put(scope, object, key, value)?;
        }
    }
    Ok(object.into())
}
fn pi_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: &str,
) -> Result<v8::Local<'s, v8::Value>> {
    let object = v8::Object::new(scope);
    let value = v8_string(scope, target)?;
    put(scope, object, "target", value)?;
    Ok(object.into())
}
pub(super) fn configuration_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    config: &Config,
) -> Result<v8::Local<'s, v8::Value>> {
    let config = config.sorted_snapshot();
    let object = v8::Object::new(scope);
    if let Some(elements) = &config.elements {
        let value = list_value(scope, elements, element_value)?;
        put(scope, object, "elements", value)?;
    }
    for (key, list) in [
        ("removeElements", &config.remove_elements),
        (
            "replaceWithChildrenElements",
            &config.replace_with_children_elements,
        ),
        ("attributes", &config.attributes),
        ("removeAttributes", &config.remove_attributes),
    ] {
        if let Some(list) = list {
            let value = list_value(scope, list, name_value)?;
            put(scope, object, key, value)?;
        }
    }
    for (key, list) in [
        ("processingInstructions", &config.processing_instructions),
        (
            "removeProcessingInstructions",
            &config.remove_processing_instructions,
        ),
    ] {
        if let Some(list) = list {
            let value = list_value(scope, list, |scope, target| pi_value(scope, target))?;
            put(scope, object, key, value)?;
        }
    }
    for (key, value) in [
        ("comments", config.comments),
        ("dataAttributes", config.data_attributes),
    ] {
        if let Some(value) = value {
            let value = v8::Boolean::new(scope, value);
            put(scope, object, key, value.into())?;
        }
    }
    Ok(object.into())
}
pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<()> {
    let config = if args.get(0).is_undefined() {
        Config::default_preset()
    } else {
        constructor_configuration(scope, args.get(0))?
    };
    let id = register_platform_object(
        state,
        PlatformObjectRecord::Sanitizer(Rc::new(RefCell::new(config))),
    );
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}
fn record(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
) -> Result<Rc<RefCell<Config>>> {
    let id = stateful_id(scope, object)?;
    match &*platform_object(state, id)? {
        PlatformObjectRecord::Sanitizer(config) => Ok(config.clone()),
        _ => Err(invalid()),
    }
}
pub(super) fn call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>> {
    // Brand checking precedes argument conversion, but the configuration is
    // not borrowed until conversion finishes. Reentrant edits remain visible.
    let config = record(scope, state, object)?;
    if member == "get" {
        let snapshot = config.borrow().clone();
        return configuration_value(scope, &snapshot).map(Some);
    }
    let changed = match member {
        "removeUnsafe" => config.borrow_mut().remove_unsafe(),
        "setComments" => {
            let value = args.get(0).boolean_value(scope);
            config.borrow_mut().set_comments(value)
        }
        "setDataAttributes" => {
            let value = args.get(0).boolean_value(scope);
            config.borrow_mut().set_data_attributes(value)
        }
        "allowElement" => {
            let value = element_with_attributes(scope, args.get(0))?;
            config.borrow_mut().allow_element(value)
        }
        "removeElement" => {
            let value = element(scope, args.get(0))?;
            config.borrow_mut().remove_element(value)
        }
        "replaceElementWithChildren" => {
            let value = element(scope, args.get(0))?;
            config.borrow_mut().replace_element_with_children(value)
        }
        "allowAttribute" => {
            let value = attribute(scope, args.get(0))?;
            config.borrow_mut().allow_attribute(value)
        }
        "removeAttribute" => {
            let value = attribute(scope, args.get(0))?;
            config.borrow_mut().remove_attribute(value)
        }
        "allowProcessingInstruction" => {
            let value = processing_instruction(scope, args.get(0))?;
            config.borrow_mut().allow_processing_instruction(value)
        }
        "removeProcessingInstruction" => {
            let value = processing_instruction(scope, args.get(0))?;
            config.borrow_mut().remove_processing_instruction(value)
        }
        _ => return Err(invalid()),
    };
    Ok(Some(v8::Boolean::new(scope, changed).into()))
}

#[derive(Clone)]
pub(super) enum PolicySource {
    Instance(Rc<RefCell<Config>>),
    Configuration(Config),
}
pub(super) struct PolicySnapshot {
    config: Config,
    instance: Option<Rc<RefCell<Config>>>,
}
#[derive(Clone)]
pub(super) struct SinkOptions {
    pub run_scripts: bool,
    source: PolicySource,
    source_object: Option<v8::Weak<v8::Object>>,
}
impl SinkOptions {
    /// Called at the sink's algorithmic sanitizer-acquisition point, after
    /// Web IDL conversion and Trusted Types callbacks. Instance mutations made
    /// by those callbacks are deliberately visible in this snapshot.
    pub(super) fn snapshot(&self, safe: bool) -> Result<PolicySnapshot> {
        Ok(PolicySnapshot {
            config: self.configuration(safe)?,
            instance: match &self.source {
                PolicySource::Instance(config) => Some(config.clone()),
                PolicySource::Configuration(_) => None,
            },
        })
    }
    pub(super) fn stream_configuration(&self, safe: bool) -> Result<Config> {
        if safe {
            if let PolicySource::Instance(config) = &self.source {
                config.borrow_mut().remove_unsafe();
            }
        }
        self.configuration(safe)
    }
    pub(super) fn stream_dictionary<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Result<v8::Local<'s, v8::Object>> {
        let object = v8::Object::new(scope);
        let run = v8::Boolean::new(scope, self.run_scripts);
        put(scope, object, "runScripts", run.into())?;
        let config = if let Some(value) = self
            .source_object
            .as_ref()
            .and_then(|value| value.to_local(scope))
        {
            value.into()
        } else {
            configuration_value(scope, &self.configuration(false)?)?
        };
        put(scope, object, "sanitizer", config)?;
        Ok(object)
    }
    pub(super) fn configuration(&self, safe: bool) -> Result<Config> {
        match &self.source {
            PolicySource::Instance(config) => Ok(config.borrow().clone()),
            PolicySource::Configuration(config) => {
                config.clone().canonicalize(!safe).map_err(|_| invalid())
            }
        }
    }
}
fn source(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<PolicySource> {
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
        // A proxy around a platform object does not carry its Web IDL brand.
        // It follows the dictionary branch, including ordinary proxy getters.
        if !object.is_proxy()
            && let Some(owner) = binding_state_for_receiver(scope, object)
            && let Ok(id) = stateful_id(scope, object)
            && let Some(PlatformObjectRecord::Sanitizer(config)) =
                owner.platform_objects.borrow().get(&id)
        {
            return Ok(PolicySource::Instance(config.clone()));
        }
        return configuration(scope, value).map(PolicySource::Configuration);
    }
    if value.is_null_or_undefined() {
        return configuration(scope, value).map(PolicySource::Configuration);
    }
    if string(scope, value)? == "default" {
        Ok(PolicySource::Configuration(Config::default_preset()))
    } else {
        Err(invalid())
    }
}
/// Safe sinks default to the preset; unsafe sinks default to an empty
/// dictionary. Only Element/ShadowRoot unsafe setters read runScripts.
pub(super) fn options(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    safe: bool,
    setter: bool,
) -> Result<SinkOptions> {
    let object = object(value)?;
    let run_scripts = if !safe && setter {
        field(scope, object, "runScripts")?.boolean_value(scope)
    } else {
        false
    };
    let value = field(scope, object, "sanitizer")?;
    let source = if value.is_undefined() {
        PolicySource::Configuration(if safe {
            Config::default_preset()
        } else {
            Config::default()
        })
    } else {
        source(scope, value)?
    };
    let source_object = if matches!(&source, PolicySource::Instance(_)) {
        value
            .try_cast::<v8::Object>()
            .ok()
            .map(|value| v8::Weak::new(scope, value))
    } else {
        None
    };
    Ok(SinkOptions {
        run_scripts,
        source,
        source_object,
    })
}

/// Web IDL conversion of the HTML argument precedes options. Trusted Types
/// policy evaluation is an algorithm step and follows all options conversion.
pub(super) fn html_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    safe: bool,
) -> Result<v8::Local<'s, v8::Value>> {
    if !safe && trusted_value_record(scope, value, TrustedKind::Html).is_some() {
        Ok(value)
    } else {
        value
            .to_string(scope)
            .map(Into::into)
            .ok_or(V8BindingError::JavaScriptException)
    }
}
pub(super) fn filter(
    snapshot: &PolicySnapshot,
    dom: &mut oryn_dom::DomRuntime,
    root: NodeId,
    safe: bool,
) -> Result<()> {
    // The safe sanitization algorithm mutates the actual Sanitizer after
    // parsing, not while converting arguments or acquiring the options.
    let instance = snapshot.instance.as_ref().map(|instance| {
        let mut config = instance.borrow_mut();
        if safe {
            config.remove_unsafe();
        }
        config.clone()
    });
    let config = instance.as_ref().unwrap_or(&snapshot.config);
    let result = if instance.is_some() {
        config.filter_instance_snapshot(dom, root, safe)
    } else if safe {
        config.filter_tree_safe(dom, root)
    } else {
        config.filter_tree(dom, root)
    };
    result.map_err(|error| match error {
        oryn_dom::sanitizer::SanitizeError::Configuration(_) => invalid(),
        oryn_dom::sanitizer::SanitizeError::Dom(error) => error.into(),
    })
}
pub(super) fn parse_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments<'s>,
    safe: bool,
) -> Result<v8::Local<'s, v8::Object>> {
    let html = html_argument(scope, args.get(0), safe)?;
    let options = options(scope, args.get(1), safe, false)?;
    let markup = if safe {
        let html = v8::Local::<v8::String>::try_from(html).map_err(|_| invalid())?;
        rust_string_preserving_utf16(scope, html)?
    } else {
        trusted_string(
            scope,
            state,
            html,
            TrustedKind::Html,
            "Document parseHTMLUnsafe",
        )?
    };
    let mut parsed =
        oryn_dom::html::parse_inert_dom_string_with_declarative_shadow_unpatched(&markup)
            .into_runtime()?;
    let document = parsed.document();
    if let Some(metadata) = parsed
        .arena_mut()
        .get_mut(document)?
        .document_metadata
        .as_mut()
    {
        metadata.url = Some("about:blank".into());
    }
    let config = options.snapshot(safe)?;
    filter(&config, &mut parsed, document, safe)?;
    oryn_dom::html::apply_fragment_patches(&mut parsed, document)?;
    let scripts = parsed
        .arena()
        .iter()
        .filter_map(|(node, record)| {
            matches!(&record.kind, NodeKind::Element { name }
            if matches!(name.ns.as_ref(), HTML_NAMESPACE | "http://www.w3.org/2000/svg")
                && name.local.as_ref() == "script")
            .then_some(node)
        })
        .collect::<Vec<_>>();
    for script in scripts {
        parsed.mark_script_started(script)?;
    }
    let document = state
        .dom
        .borrow_mut()
        .import_subtree(parsed.arena(), document)?;
    state.derive_document_provenance(state.document_root(), document)?;
    wrap_node(scope, state, document)
}

pub(super) fn set_html<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    args: &v8::FunctionCallbackArguments<'s>,
    safe: bool,
) -> Result<()> {
    let receiver = wrap_node(scope, state, node)?;
    let sink = if matches!(
        state.dom.borrow().arena().get(node)?.kind,
        NodeKind::ShadowRoot { .. }
    ) {
        "ShadowRoot setHTMLUnsafe"
    } else {
        "Element setHTMLUnsafe"
    };
    let html = html_argument(scope, args.get(0), safe)?;
    let options = options(scope, args.get(1), safe, true)?;
    let markup = if safe {
        let html = v8::Local::<v8::String>::try_from(html).map_err(|_| invalid())?;
        rust_string_preserving_utf16(scope, html)?
    } else {
        let (owner, _) = node_identity::argument(scope, receiver.into())?.ok_or_else(invalid)?;
        trusted_string(scope, &owner, html, TrustedKind::Html, sink)?
    };
    let (owner, node) = node_identity::argument(scope, receiver.into())?.ok_or_else(invalid)?;
    let (context_node, is_template) = {
        let dom = owner.dom.borrow();
        let record = dom.arena().get(node)?;
        match &record.kind {
            NodeKind::Element { .. } => (node, dom.is_element(node, HTML_NAMESPACE, "template")),
            NodeKind::ShadowRoot { .. } => (record.parent.ok_or_else(invalid)?, false),
            _ => return Err(invalid()),
        }
    };
    let context = oryn_dom::sanitizer::FragmentContext::capture(&owner.dom.borrow(), context_node)?;
    if safe && context.safe_sink_is_noop() {
        return Ok(());
    }
    let (fragment, root) = prepare_sink_fragment(&markup, &options, context, safe)?;
    let (target_owner, target) = if is_template {
        if let Some(content) = template_links::content(scope, &owner, node)? {
            node_identity::argument(scope, content)?.ok_or_else(invalid)?
        } else {
            let target = owner.dom.borrow_mut().template_content(node)?;
            (owner.clone(), target)
        }
    } else {
        (owner.clone(), node)
    };
    post_connection_scripts::check_fragment_capacity(
        scope,
        &target_owner,
        target,
        &fragment,
        root,
    )?;
    let children = target_owner
        .dom
        .borrow_mut()
        .import_fragment_children(target, &fragment, root)?;
    for child in &children {
        custom_elements::initialize_fragment_registry_from(&target_owner, *child, &owner, node)?;
        if !options.run_scripts {
            target_owner.mark_script_subtree_inert(*child)?;
        }
    }
    let preparation =
        post_connection_scripts::capture_replacement(scope, &target_owner, target, &children)?;
    let mutation = target_owner
        .dom
        .borrow_mut()
        .replace_imported_fragment_children(target, children);
    preparation.finish_mutation(scope, mutation)?;
    upgrade_custom_element_subtree(scope, &target_owner, target)?;
    Ok(())
}

/// Legacy setters share native fragment context and resource custody, while
/// retaining their inert-script and non-declarative parsing semantics.
pub(super) fn set_legacy_html<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    value: v8::Local<'s, v8::Value>,
    outer: bool,
    interface: &str,
) -> Result<()> {
    let receiver = wrap_node(scope, state, node)?;
    let value = if value.is_null() {
        v8::String::empty(scope).into()
    } else {
        value
    };
    let value = trusted_union_string_input(scope, value)?;
    let (owner, _) = node_identity::argument(scope, receiver.into())?.ok_or_else(invalid)?;
    let markup = trusted_string(
        scope,
        &owner,
        value,
        TrustedKind::Html,
        &format!(
            "{interface} {}",
            if outer { "outerHTML" } else { "innerHTML" }
        ),
    )?;
    let (owner, node) = node_identity::argument(scope, receiver.into())?.ok_or_else(invalid)?;
    let (context, target_owner, target) = if outer {
        let parent = owner.dom.borrow().parent(node)?;
        let Some(parent) = parent else { return Ok(()) };
        let context = {
            let dom = owner.dom.borrow();
            match &dom.arena().get(parent)?.kind {
                NodeKind::Document => {
                    return Err(V8BindingError::Unsupported {
                        interface: "Element".into(),
                        member: "NoModificationAllowedError".into(),
                    });
                }
                NodeKind::Element { .. } => {
                    oryn_dom::sanitizer::FragmentContext::capture(&dom, parent)?
                }
                _ => oryn_dom::sanitizer::FragmentContext::html_body(),
            }
        };
        (context, owner.clone(), parent)
    } else {
        let context_node = {
            let dom = owner.dom.borrow();
            match &dom.arena().get(node)?.kind {
                NodeKind::ShadowRoot { .. } => dom.parent(node)?.ok_or_else(invalid)?,
                _ => node,
            }
        };
        let context =
            oryn_dom::sanitizer::FragmentContext::capture(&owner.dom.borrow(), context_node)?;
        let (target_owner, target) =
            if owner
                .dom
                .borrow()
                .is_element(node, HTML_NAMESPACE, "template")
            {
                if let Some(content) = template_links::content(scope, &owner, node)? {
                    node_identity::argument(scope, content)?.ok_or_else(invalid)?
                } else {
                    let target = owner.dom.borrow_mut().template_content(node)?;
                    (owner.clone(), target)
                }
            } else {
                (owner.clone(), node)
            };
        (context, target_owner, target)
    };
    let (fragment, root) =
        prepare_legacy_fragment(&owner, if outer { target } else { node }, &markup, context)?;
    post_connection_scripts::check_fragment_capacity(
        scope,
        &target_owner,
        target,
        &fragment,
        root,
    )?;
    let children = target_owner
        .dom
        .borrow_mut()
        .import_fragment_children(target, &fragment, root)?;
    let context_node = if outer { target } else { node };
    for &child in &children {
        custom_elements::initialize_fragment_registry_from(
            &target_owner,
            child,
            &owner,
            context_node,
        )?;
        target_owner.mark_script_subtree_inert(child)?;
    }
    let preparation =
        post_connection_scripts::capture_replacement(scope, &target_owner, target, &children)?;
    let mutation = if outer {
        let mut dom = target_owner.dom.borrow_mut();
        let staging = dom.create(NodeKind::DocumentFragment);
        let document = dom
            .node_document(target)?
            .ok_or(DomError::StaleNode(target))?;
        dom.set_node_document(staging, document)?;
        for &child in &children {
            dom.append_child(staging, child)?;
        }
        let siblings = light_children(&dom, target)?;
        let index = siblings
            .iter()
            .position(|child| *child == node)
            .ok_or(DomError::NotAChild(node, target))?;
        let reference = siblings.get(index + 1).copied();
        dom.replace_dom_child(target, staging, node, reference)
            .map(|_| ())
    } else {
        target_owner
            .dom
            .borrow_mut()
            .replace_imported_fragment_children(target, children.clone())
    };
    preparation.finish_mutation(scope, mutation)?;
    for child in children {
        upgrade_custom_element_subtree(scope, &target_owner, child)?;
    }
    Ok(())
}

pub(super) fn prepare_legacy_fragment(
    owner: &V8BindingState,
    context_node: NodeId,
    markup: &str,
    html_context: oryn_dom::sanitizer::FragmentContext,
) -> Result<(DomRuntime, NodeId)> {
    let dom = owner.dom.borrow();
    let document = dom
        .node_document(context_node)?
        .ok_or(DomError::StaleNode(context_node))?;
    let html = dom
        .arena()
        .get(document)?
        .document_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.is_html);
    if html {
        drop(dom);
        Ok(oryn_dom::sanitizer::prepare_inert_fragment(
            markup,
            html_context,
        )?)
    } else {
        // outerHTML uses a synthetic HTML body when its parent is a
        // DocumentFragment, including in XML Documents. Keep XML parsing while
        // supplying that body's namespace instead of the bare fragment's scope.
        let parsed = if matches!(
            dom.arena().get(context_node)?.kind,
            NodeKind::DocumentFragment
        ) {
            let fallback = oryn_dom::xml::parse(
                "<body xmlns='http://www.w3.org/1999/xhtml'/>",
                "application/xml",
            )
            .into_runtime()?;
            let body = fallback.arena().get(fallback.document())?.children[0];
            oryn_dom::xml::parse_fragment(&fallback, body, markup)
        } else {
            oryn_dom::xml::parse_fragment(&dom, context_node, markup)
        };
        parsed.map_err(|error| match error {
            oryn_dom::xml::FragmentError::Dom(error) => error.into(),
            oryn_dom::xml::FragmentError::Syntax(_) => V8BindingError::Unsupported {
                interface: "Element".into(),
                member: "SyntaxError".into(),
            },
        })
    }
}

/// All safe/unsafe insertion paths finish filtering before fragment patching,
/// custom-element publication or executable script admission.
pub(super) fn prepare_sink_fragment(
    markup: &str,
    options: &SinkOptions,
    context: oryn_dom::sanitizer::FragmentContext,
    safe: bool,
) -> Result<(DomRuntime, NodeId)> {
    let snapshot = options.snapshot(safe)?;
    let (mut fragment, root) = oryn_dom::sanitizer::prepare_unfiltered_fragment(markup, context)?;
    filter(&snapshot, &mut fragment, root, safe)?;
    oryn_dom::html::apply_fragment_patches(&mut fragment, root)?;
    if options.run_scripts {
        let mut pending = vec![root];
        while let Some(node) = pending.pop() {
            let record = fragment.arena_mut().get_mut(node)?;
            if matches!(&record.kind,NodeKind::Element{name}if name.local.as_ref()=="script"&&matches!(name.ns.as_ref(),HTML_NAMESPACE|"http://www.w3.org/2000/svg"))
            {
                record.script_already_started = false;
                record.script_parser_document = None;
            }
            pending.extend(record.children.iter().rev().copied());
        }
    }
    Ok((fragment, root))
}
#[path = "sanitizer_insert.rs"]
mod insertion;
pub(super) fn insert_html<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    args: &v8::FunctionCallbackArguments<'s>,
    member: &str,
) -> Result<()> {
    insertion::run(scope, state, node, args, member)
}

#[cfg(test)]
mod parsed_script_adoption_tests {
    use super::*;
    #[test]
    fn parsed_scripts_stay_inert_through_template_shadow_clone_and_adoption() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let _state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            )),
            RealmId(952),
            1,
        )
        .unwrap();
        assert!(super::super::test_eval::eval(scope, r#"(()=>{
            globalThis.adoptedParseRuns=0;
            const parsed=Document.parseHTMLUnsafe('<script>adoptedParseRuns++<'+ '/script><template><script>adoptedParseRuns++<'+ '/script></template><div><template shadowrootmode=open><script>adoptedParseRuns++<'+ '/script></template></div>');
            const scripts=[parsed.querySelector('script'),parsed.querySelector('template').content.querySelector('script'),parsed.querySelector('div').shadowRoot.querySelector('script')];
            for(const script of scripts){document.body.append(script.cloneNode(true),script)}
            if(adoptedParseRuns!==0)throw Error('parsed scripts executed after adoption');
            const live=document.createElement('script');live.textContent='adoptedParseRuns++';document.body.append(live);
            if(adoptedParseRuns!==1)throw Error('ordinary new script lost execution');return true;
        })()"#).is_true());
    }
}
