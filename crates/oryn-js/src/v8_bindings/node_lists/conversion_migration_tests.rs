//! Source regressions; execution is deferred until the complete generation gate.
use super::*;

fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, source).unwrap();
    v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap()
}

#[test]
fn live_collection_conversions_resolve_migrated_original_receiver() {
    super::super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let first = v8::Context::new(scope, Default::default());
    let second = v8::Context::new(scope, Default::default());
    let token = v8::String::new(scope, "collection-conversion").unwrap();
    first.set_security_token(token.into());
    second.set_security_token(token.into());
    let first_state = {
        let scope = &mut v8::ContextScope::new(scope, first);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, first, dom, RealmId(170), 1).unwrap()
    };
    let second_state = {
        let scope = &mut v8::ContextScope::new(scope, second);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, second, dom, RealmId(171), 1).unwrap()
    };
    let scope = &mut v8::ContextScope::new(scope, first);
    let key = v8::String::new(scope, "destination").unwrap();
    let document = wrap_node(scope, &second_state, second_state.document_root()).unwrap();
    first.global(scope).set(scope, key.into(), document.into());
    assert!(eval(scope,r#"(()=>{
      const check=(v,label)=>{if(!v)throw Error(label)};
      for(const kind of ['childNodes','children']){
        const host=document.createElement('div');host.innerHTML='<span id=member></span>';document.body.append(host);const collection=host[kind],member=host.firstChild;let calls=0;
        const result=collection.item({valueOf(){calls++;destination.body.append(host);return 0}});
        check(result===member&&collection===host[kind]&&member.ownerDocument===destination&&calls===1,kind+' migrated item');
      }
      const host=document.createElement('div');host.innerHTML='<input id=member>';const members=host.children,member=host.firstChild;
      check(members.namedItem({toString(){destination.body.append(host);return 'member'}})===member,'named member');
      const element=document.createElement('div');element.setAttribute('data-value','kept');const attributes=element.attributes,attribute=attributes[0];
      check(attributes.getNamedItem({toString(){destination.body.append(element);return 'data-value'}})===attribute,'NamedNodeMap get migration');
      element.setAttributeNS('urn:collection','c:value','namespace');const namespaceAttribute=element.getAttributeNodeNS('urn:collection','value');const order=[];
      check(attributes.removeNamedItemNS({toString(){order.push('namespace');document.body.append(element);return 'urn:collection'}},{toString(){order.push('local');destination.body.append(element);return 'value'}})===namespaceAttribute,'NamedNodeMap two conversions');
      check(order.join()==='namespace,local'&&element.getAttributeNS('urn:collection','value')===null,'conversion order and current destination');
      const form=document.createElement('form');form.innerHTML='<input name=group type=radio value=a><input name=group type=radio value=b>';document.body.append(form);const controls=form.elements;
      const radios=controls.namedItem({toString(){destination.body.append(form);return 'group'}});check(radios.length===2,'form namedItem migration');
      radios.value={toString(){document.body.append(form);return 'b'}};check(radios.value==='b'&&form.lastChild.checked&&radios[1]===controls.namedItem('group')[1],'radio value migration and member identity');
      const indexed=Object.getOwnPropertyDescriptor(members,'0'),named=Object.getOwnPropertyDescriptor(members,'member');
      check(indexed.value===member&&indexed.enumerable&&!indexed.writable&&indexed.configurable,'migrated indexed descriptor');
      check(named.value===member&&!named.enumerable&&!named.writable&&named.configurable,'migrated named descriptor');
      const iterable=document.createElement('div');iterable.innerHTML='<b></b><i></i>';const iterator=iterable.children[Symbol.iterator]();const first=iterable.firstChild;check(iterator.next().value===first,'iterator prefix');destination.body.append(iterable);iterable.removeChild(first);const tail=destination.createElement('u');iterable.append(tail);check(iterator.next().value===tail&&iterator.next().done,'iterator uses current migrated indices');
      const unusual=document.createElement('div');unusual.id='\uD800';document.body.append(unusual);check(document.all.namedItem('\uD800')===unusual,'HTMLAllCollection lossless lookup');
      check(unusual.id==='\uD800'&&unusual.getAttribute('id')==='\uD800','lossless reflected id');
      const replacement=document.createElement('div');replacement.id='\uFFFD';document.body.append(replacement);check(document.all.namedItem('\uFFFD')===replacement,'replacement character remains distinct');
      unusual.id={toString(){destination.body.append(unusual);return '\uDC00'}};
      check(unusual.ownerDocument===destination&&unusual.id==='\uDC00'&&destination.all.namedItem('\uDC00')===unusual,'id conversion resolves migrated receiver');
      unusual.className='\uD800';check(unusual.className==='\uD800'&&unusual.getAttribute('class')==='\uD800','lossless reflected class');
      const sentinel={};try{members.item({valueOf(){throw sentinel}});throw Error('expected conversion failure')}catch(error){check(error===sentinel,'exact conversion exception')}
      let converted=false;try{HTMLCollection.prototype.namedItem.call({}, {toString(){converted=true;return 'x'}})}catch(error){check(error instanceof TypeError,'brand error')}check(!converted,'brand precedes conversion');
      return true;
    })()"#).is_true());
    let _ = first_state;
}

#[test]
fn select_and_options_share_foreign_insertion_and_post_conversion_ownership() {
    super::super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let first = v8::Context::new(scope, Default::default());
    let second = v8::Context::new(scope, Default::default());
    let token = v8::String::new(scope, "select-conversion").unwrap();
    first.set_security_token(token.into());
    second.set_security_token(token.into());
    let first_state = {
        let scope = &mut v8::ContextScope::new(scope, first);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, first, dom, RealmId(172), 1).unwrap()
    };
    let second_state = {
        let scope = &mut v8::ContextScope::new(scope, second);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, second, dom, RealmId(173), 1).unwrap()
    };
    let scope = &mut v8::ContextScope::new(scope, first);
    let key = v8::String::new(scope, "destination").unwrap();
    let document = wrap_node(scope, &second_state, second_state.document_root()).unwrap();
    first.global(scope).set(scope, key.into(), document.into());
    assert!(eval(scope,r#"(()=>{
      const check=(v,label)=>{if(!v)throw Error(label)},select=document.createElement('select');select.innerHTML='<option>a</option><option>b</option>';document.body.append(select);const options=select.options;
      options.selectedIndex={valueOf(){destination.body.append(select);return 1}};check(select.selectedIndex===1&&options===select.options,'selectedIndex owner');
      options.length={valueOf(){document.body.append(select);return 1}};check(select.length===1,'length owner');
      const foreign=destination.createElement('option');foreign.textContent='foreign';select.add(foreign);check(select.lastChild===foreign&&foreign.ownerDocument===document,'foreign add identity');
      const added=document.createElement('option');options.add(added,{valueOf(){destination.body.append(select);return 0}});check(select.firstChild===added&&added.ownerDocument===destination,'add numeric conversion migration');
      const replacement=document.createElement('option');options[1]=replacement;check(options[1]===replacement&&replacement.ownerDocument===destination,'indexed foreign replacement');
      options.remove({valueOf(){document.body.append(select);return 0}});check(select.options[0]===replacement,'options remove owner');
      select.remove({valueOf(){destination.body.append(select);return 0}});check(!replacement.parentNode,'select remove owner');
      document.body.append(select);const reactions=[];customElements.define('x-option-reaction',class extends HTMLOptionElement{connectedCallback(){reactions.push('connected')}disconnectedCallback(){reactions.push('disconnected')}},{extends:'option'});
      const observed=document.createElement('option',{is:'x-option-reaction'});options[0]=observed;check(reactions.join()==='connected','indexed setter CE boundary');Object.defineProperty(options,'0',{value:null});check(reactions.join()==='connected,disconnected','indexed definer CE boundary');
      const detached=document.createElement('option'),wrong=document.createElement('option');try{select.add(detached,wrong);throw Error('foreign reference accepted')}catch(error){check(error.name==='NotFoundError','reference rejection')}check(detached.ownerDocument===document&&!detached.parentNode,'rejected reference does not adopt');
      return true;
    })()"#).is_true());
    let _ = first_state;
}
