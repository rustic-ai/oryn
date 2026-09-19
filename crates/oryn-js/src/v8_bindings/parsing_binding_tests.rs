//! Selected DOM Parsing binding regression sources; parser algorithms themselves
//! remain shared with the production native HTML/XML parser and serializer.
use super::*;
fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, text: &str) -> v8::Local<'s, v8::Value> {
    let text = v8::String::new(scope, text).unwrap();
    v8::Script::compile(scope, text, None)
        .unwrap()
        .run(scope)
        .unwrap()
}
#[test]
fn parser_serializer_conversion_order_brands_and_descriptors() {
    tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let dom = Rc::new(RefCell::new(
        oryn_dom::html::parse("").into_runtime().unwrap(),
    ));
    install_generated_bindings(scope, context, dom, RealmId(90), 1).unwrap();
    eval(
        scope,
        r#"
        const check=(x,m)=>{if(!x)throw Error(m)},parser=new DOMParser(),serializer=new XMLSerializer();
        const order=[];
        const xml=parser.parseFromString({toString(){order.push('input');return '<root>value</root>'}},{toString(){order.push('type');return 'application/xml'}});
        check(order.join(',')==='input,type'&&xml.contentType==='application/xml','ordered conversions');
        check(serializer.serializeToString(xml)==='<root>value</root>','native roundtrip');
        let rejected=false;try{parser.parseFromString('', 'TEXT/HTML')}catch(e){rejected=e instanceof TypeError}check(rejected,'case-sensitive enum');
        for(const value of [null,{},new Proxy(xml,{})]){rejected=false;try{serializer.serializeToString(value)}catch(e){rejected=e instanceof TypeError}check(rejected,'native Node argument')}
        for(const [method,args] of [[DOMParser.prototype.parseFromString,['','text/html']],[XMLSerializer.prototype.serializeToString,[xml]]]){
            rejected=false;try{method.apply({},args)}catch(e){rejected=e instanceof TypeError}check(rejected,'receiver brand');
        }
        const sentinel={};order.length=0;try{parser.parseFromString({toString(){throw sentinel}},{toString(){order.push('wrong');return 'text/html'}})}catch(e){check(e===sentinel,'exact exception')}check(order.length===0,'short circuit');
        for(const [proto,name,length] of [[DOMParser.prototype,'parseFromString',2],[XMLSerializer.prototype,'serializeToString',1]]){const d=Object.getOwnPropertyDescriptor(proto,name);check(d.enumerable&&d.configurable&&d.writable&&d.value.length===length,'descriptor')}
        const inert=parser.parseFromString('<script>globalThis.unexpected=true<\/script><noscript><p>parsed</p></noscript>','text/html');
        check(!globalThis.unexpected&&inert.defaultView===null,'inert document');
        check(parser.parseFromString('<broken>','application/xml').getElementsByTagName('parsererror').length===1,'XML error document');
    "#,
    );
}
#[test]
fn serializer_uses_argument_arena_and_borrowed_receiver_identity() {
    tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let foreign = v8::Context::new(scope, Default::default());
    let (node, serializer) = {
        let scope = &mut v8::ContextScope::new(scope, foreign);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        install_generated_bindings(scope, foreign, dom, RealmId(91), 1).unwrap();
        let node = eval(
            scope,
            "new DOMParser().parseFromString('<foreign attr=\"yes\">original</foreign>','application/xml')",
        );
        let node = v8::Global::new(scope, node);
        let serializer = eval(scope, "new XMLSerializer()");
        (node, v8::Global::new(scope, serializer))
    };
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let dom = Rc::new(RefCell::new(
        oryn_dom::html::parse("").into_runtime().unwrap(),
    ));
    install_generated_bindings(scope, context, dom, RealmId(92), 1).unwrap();
    for (name, value) in [("foreignNode", &node), ("foreignSerializer", &serializer)] {
        let value = v8::Local::new(scope, value);
        let key = v8::String::new(scope, name).unwrap();
        context.global(scope).set(scope, key.into(), value);
    }
    eval(
        scope,
        r#"
        const local=new DOMParser().parseFromString('<local>decoy</local>','application/xml');
        const expected='<foreign attr="yes">original</foreign>';
        if(new XMLSerializer().serializeToString(foreignNode)!==expected)throw Error('foreign argument arena');
        if(XMLSerializer.prototype.serializeToString.call(foreignSerializer,local)!=='<local>decoy</local>')throw Error('borrowed native receiver');
        if(foreignNode.documentElement.localName!=='foreign'||local.documentElement.localName!=='local')throw Error('serialization mutated/imported source');
    "#,
    );
}
