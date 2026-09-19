//! Public textContent uses DOM node kinds and lossless descendant text.
use super::*;
#[test]
fn text_content_node_kinds_light_tree_and_utf16() {
    tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let dom = Rc::new(RefCell::new(
        oryn_dom::html::parse("<!doctype html><div id=host></div>")
            .into_runtime()
            .unwrap(),
    ));
    install_generated_bindings(scope, context, dom, RealmId(96), 1).unwrap();
    let script=v8::String::new(scope,r#"
        const check=(ok,message)=>{if(!ok)throw Error(message)};
        check(document.textContent===null&&document.doctype.textContent===null,'nullable node kinds');
        const host=document.getElementById('host');
        host.append(document.createTextNode('\ud83d'),document.createComment('ignored'),document.createTextNode('\ude00'));
        const shadow=host.attachShadow({mode:'open'});shadow.textContent='shadow';
        check(host.textContent==='😀'&&shadow.textContent==='shadow','light tree and lossless concatenation');
        const marker=String.fromCodePoint(0xf0000)+'oryn-wtf16:[56321]';
        host.appendChild(document.createTextNode(marker));
        check(host.textContent==='😀'+marker,'literal storage prefix');
        const xml=new DOMParser().parseFromString('<root>a<!--comment--><?pi ignored?><![CDATA[b]]></root>','application/xml');
        check(xml.textContent===null&&xml.documentElement.textContent==='ab','XML descendant Text and CDATA only');
        check(xml.documentElement.childNodes[1].textContent==='comment','Comment own data');
        check(xml.documentElement.childNodes[2].textContent==='ignored','PI own data');
        const script=document.createElement('script');
        script.append(document.createTextNode('\ud83d'),document.createComment('ignored'),document.createTextNode('\ude00'));
        const nested=document.createElement('span');nested.textContent='nested';script.appendChild(nested);
        check(script.text==='😀'&&script.textContent==='😀nested','script child text versus descendant text');
        const attr=document.createAttribute('x');attr.value=marker;check(attr.textContent===marker,'Attr value');
        const fragment=document.createDocumentFragment();fragment.append('one',document.createComment('ignored'),'two');
        check(fragment.textContent==='onetwo','fragment descendants');
    "#).unwrap();
    v8::Script::compile(scope, script, None)
        .unwrap()
        .run(scope)
        .unwrap();
}

#[test]
fn dynamic_script_preparation_uses_child_text_and_scalar_source() {
    tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let dom = Rc::new(RefCell::new(
        oryn_dom::html::parse("<body></body>")
            .into_runtime()
            .unwrap(),
    ));
    install_generated_bindings(scope, context, dom, RealmId(97), 1).unwrap();
    let source=v8::String::new(scope,r#"
        const script=document.createElement('script');
        script.appendChild(document.createTextNode("globalThis.prepared = '\ud83d"));
        const nested=document.createElement('span');nested.textContent="throw Error('nested source executed')";script.appendChild(nested);
        script.appendChild(document.createComment('ignored'));
        script.appendChild(document.createTextNode("\ude00';"));
        document.body.appendChild(script);
        if(globalThis.prepared!=='😀')throw Error('source code unit concatenation');
    "#).unwrap();
    v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap();
}
