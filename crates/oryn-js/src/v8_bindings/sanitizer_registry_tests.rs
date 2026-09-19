//! Pinned scoped-registry fragment context regressions; source generated only.
use super::*;

#[test]
fn shadow_markup_sinks_use_target_registry_not_parsing_host_registry() {
    tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let dom = Rc::new(RefCell::new(
        oryn_dom::html::parse("<body></body>")
            .into_runtime()
            .unwrap(),
    ));
    let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
    let source = v8::String::new(scope, r#"(()=>{
      const log=[],registry=new CustomElementRegistry();
      customElements.define('x-sink-context',class extends HTMLElement{constructor(){super();log.push('global')}});
      registry.define('x-sink-context',class extends HTMLElement{constructor(){super();log.push('scoped')}});
      for(const method of ['innerHTML','setHTML','setHTMLUnsafe']){
        const host=document.createElement('div');document.body.append(host);
        const shadow=host.attachShadow({mode:'open',customElementRegistry:registry});
        if(method==='innerHTML')shadow.innerHTML='<x-sink-context/>';
        else shadow[method]('<x-sink-context/>',{sanitizer:{elements:['x-sink-context']}});
        if(shadow.firstChild.customElementRegistry!==registry||log.pop()!=='scoped'||log.length)return false;
      }
      const host=document.createElement('div');document.body.append(host);
      const shadow=host.attachShadow({mode:'open',customElementRegistry:null});
      shadow.setHTMLUnsafe('<x-sink-context/>');
      if(shadow.firstChild.customElementRegistry!==null||log.length)return false;
      const delayed=new CustomElementRegistry(),parent=document.createElement('div');document.body.append(parent);
      const target=parent.attachShadow({mode:'open',customElementRegistry:delayed});
      target.innerHTML='<x-delayed-sink id="first"></x-delayed-sink><x-delayed-sink id="second"></x-delayed-sink>';
      delayed.define('x-delayed-sink',class extends HTMLElement{constructor(){super();log.push(this.id)}});
      return log.join()==='first,second';
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
fn sanitized_customizations_and_decoded_attributes_follow_target_registry() {
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
    let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
    let source=v8::String::new(scope,r#"(()=>{
        const check=(v,m)=>{if(!v)throw Error(m)},log=[],units='\uD800x\uDC00'+String.fromCodePoint(0xF1000,0xF2000,0xF0000);
        const div=document.createElement('div'),span=document.createElement('span');
        check(div instanceof HTMLDivElement&&div instanceof HTMLElement&&!(span instanceof HTMLDivElement),'native div interface');
        let direct=false;try{new HTMLDivElement()}catch(error){direct=error instanceof TypeError}check(direct,'unregistered div construction');
        div.align='LEFT';check(div.align==='LEFT'&&div.getAttribute('align')==='LEFT','legacy div reflection');
        for(const scoped of [false,true]){
            const registry=scoped?new CustomElementRegistry():customElements;
            const suffix=scoped?'scoped':'global',built='x-sanitized-div-'+suffix,auto='x-sanitized-auto-'+suffix;
            customElements.define(built,class extends HTMLDivElement{constructor(){super();log.push(['built',this.getAttribute('title')])}},{extends:'div'});
            registry.define(auto,class extends HTMLElement{constructor(){super();log.push(['auto',this.getAttribute('title')])}});
            for(const method of ['setHTML','setHTMLUnsafe']){
                const host=document.createElement('div');document.body.append(host);
                const target=scoped?host.attachShadow({mode:'open',customElementRegistry:registry}):host;
                for(const sanitizer of [{removeAttributes:['is']},{elements:['div',auto],attributes:['title']},'default']){
                    log.length=0;target[method]('<div is="'+built+'" title="'+units+'"></div>',{sanitizer});
                    check(log.length===0&&!target.firstChild.hasAttribute('is'),'removed customization must not construct');
                }
                log.length=0;target[method]('<div is="'+built+'" title="'+units+'"></div>',{sanitizer:{elements:['div'],attributes:['is','title']}});
                check(scoped?log.length===0:log.length===1&&log[0][0]==='built'&&log[0][1]===units,'allowed customization uses target registry and decoded units');
                log.length=0;target[method]('<'+auto+' is="'+built+'" title="'+units+'"></'+auto+'>',{sanitizer:{elements:[auto],removeAttributes:['is']}});
                check(log.length===1&&log[0][0]==='auto'&&log[0][1]===units,'autonomous construction independent of removed is');
                log.length=0;target[method]('<div is="'+built+'"></div>',{sanitizer:{replaceWithChildrenElements:['div']}});
                check(!target.firstChild&&log.length===0,'unwrapped customization never constructed');host.remove();
            }
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
fn declarative_fragment_registry_defaults_use_document_and_keep_explicit_null() {
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
    let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
    let code=v8::String::new(scope,r#"(()=>{
        const check=(v,m)=>{if(!v)throw Error(m)},scoped=new CustomElementRegistry(),log=[];
        customElements.define('x-declarative-context',class extends HTMLElement{constructor(){super();log.push('global')}});
        scoped.define('x-declarative-context',class extends HTMLElement{constructor(){super();log.push('scoped')}});
        for(const method of ['setHTMLUnsafe','appendHTMLUnsafe','prependHTMLUnsafe']){
            const host=document.createElement('section',{customElementRegistry:scoped});document.body.append(host);
            host[method]('<div><template shadowrootmode="open"><x-declarative-context></x-declarative-context></template></div>');
            check(host.firstChild.customElementRegistry===scoped,'light descendants inherit fragment context');
            check(host.firstChild.shadowRoot.customElementRegistry===customElements&&log.pop()==='global'&&!log.length,'nested root defaults to document registry');host.remove();
            const top=document.createElement('section',{customElementRegistry:scoped});document.body.append(top);
            top[method]('<template shadowrootmode="open"><x-declarative-context></x-declarative-context></template>');
            check(top.shadowRoot.customElementRegistry===customElements&&log.pop()==='global'&&!log.length,'top root defaults to document registry');top.remove();
            const nullHost=document.createElement('section');document.body.append(nullHost);
            nullHost[method]('<div><template shadowrootmode="open" shadowrootcustomelementregistry><x-declarative-context></x-declarative-context></template></div>');
            check(nullHost.firstChild.shadowRoot.customElementRegistry===null&&!log.length,'explicit null preserved');nullHost.remove();
        }
        const template=document.createElement('template');template.setHTMLUnsafe('<div><template shadowrootmode="open"><x-declarative-context></x-declarative-context></template></div>');
        check(!log.length,'template owner has no registry construction');document.body.append(template.content);
        check(log.join()==='global','template extraction adopts declarative root registry');return true;
    })()"#).unwrap();
    assert!(
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .is_true()
    );
}
