//! Native Document members use the shared node migration/lifecycle machinery.
use super::*;

pub(super) fn last_modified<'s>(
    scope: &mut v8::PinScope<'s, '_>, state: &V8BindingState, document: NodeId,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let stored = state.dom.borrow().arena().get(document)?.document_metadata
        .as_ref().and_then(|metadata| metadata.last_modified);
    let seconds = stored.unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
            .unwrap_or(0)
    });
    v8_string(scope, &format_local_modified(seconds)?)
}
fn format_local_modified(seconds: i64) -> Result<String, V8BindingError> {
    let time: libc::time_t = seconds.try_into()
        .map_err(|_| V8BindingError::Allocation("document modification time range"))?;
    let mut fields = std::mem::MaybeUninit::<libc::tm>::uninit();
    #[cfg(unix)]
    let valid = unsafe { !libc::localtime_r(&time, fields.as_mut_ptr()).is_null() };
    #[cfg(windows)]
    let valid = unsafe { libc::localtime_s(fields.as_mut_ptr(), &time) == 0 };
    #[cfg(not(any(unix, windows)))]
    let valid = false;
    if !valid { return Err(V8BindingError::Allocation("document local modification time")); }
    // The thread-safe system conversion writes tm before success, including DST.
    let fields = unsafe { fields.assume_init() };
    Ok(format!("{:02}/{:02}/{:04} {:02}:{:02}:{:02}", fields.tm_mon + 1,
        fields.tm_mday, fields.tm_year + 1900, fields.tm_hour, fields.tm_min, fields.tm_sec))
}

pub type CookieAccess = dyn Fn(&V8BindingState, Option<String>) -> Result<String, String>;
impl V8BindingState {
    pub fn set_document_cookie_access(&self, access: Rc<CookieAccess>) {
        *self.document_cookie_access.borrow_mut() = Some(access);
    }
}
pub(super) fn cookie<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    document: NodeId,
    value: Option<v8::Local<'s, v8::Value>>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let wrapper = wrap_node(scope, state, document)?;
    let identity = (state.document_generation(), state.document_stream_epoch(), state.document_authority_epoch());
    let value = value.map(|value| {
        let string = value.to_string(scope).ok_or(V8BindingError::JavaScriptException)?;
        rust_string_preserving_utf16(scope, string)
    }).transpose()?;
    let (owner, current) = node_identity::argument(scope, wrapper.into())?
        .ok_or(DomError::StaleNode(document))?;
    if !Rc::ptr_eq(&owner, state) || current != document || state.execution_retired()
        || identity != (state.document_generation(), state.document_stream_epoch(), state.document_authority_epoch())
    {
        return Err(V8BindingError::Unsupported { interface: "Document".into(), member: "SecurityError".into() });
    }
    // Documents without a browsing context are cookie-averse. They must never
    // accidentally use the creator realm's cookie store or native capability.
    if document != state.document_root() { return v8_string(scope, ""); }
    let origin = state.document_provenance(document)?
        .map(|source| source.origin.clone()).unwrap_or_else(|| state.current_location_url());
    if crate::origin::Origin::from_url(&origin).is_opaque() {
        return Err(V8BindingError::Unsupported { interface: "Document".into(), member: "SecurityError".into() });
    }
    let access = state.document_cookie_access.borrow().clone();
    let result = match access {
        Some(access) => access(state, value).map_err(|error| {
            if error.contains("SecurityError:") {
                V8BindingError::Unsupported { interface: "Document".into(), member: "SecurityError".into() }
            } else { V8BindingError::Message(error) }
        })?,
        // Bare non-network binding embedders have no cookie store.
        None => String::new(),
    };
    v8_string(scope, &result)
}

pub(super) fn set_body(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    document: NodeId,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let hierarchy = || V8BindingError::Unsupported {
        interface: "Document".into(), member: "HierarchyRequestError".into(),
    };
    if value.is_null_or_undefined() { return Err(hierarchy()); }
    let (source, candidate) = node_identity::argument(scope, value)?
        .ok_or_else(|| V8BindingError::Unsupported {
            interface: "Document".into(), member: "TypeError".into(),
        })?;
    validate_interface_node(&source.dom.borrow(), candidate, "HTMLElement")?;
    if !source.dom.borrow().is_element(candidate, "http://www.w3.org/1999/xhtml", "body")
        && !source.dom.borrow().is_element(candidate, "http://www.w3.org/1999/xhtml", "frameset")
    { return Err(hierarchy()); }
    let (root, previous) = {
        let dom = state.dom.borrow();
        let previous = dom.document_body(document)?;
        if Rc::ptr_eq(&source.dom, &state.dom) && previous == Some(candidate) {
            return Ok(());
        }
        let root = dom.first_element_child(document)?.ok_or_else(hierarchy)?;
        if !dom.is_element(root, "http://www.w3.org/1999/xhtml", "html") {
            return Err(hierarchy());
        }
        (root, previous)
    };
    let insertion = node_migration::insertion_argument(
        scope, state, root, value, previous, previous,
    )?;
    let preparation = post_connection_scripts::capture_insertion(
        scope, &insertion.owner, insertion.parent, insertion.node,
    )?;
    let mutation = if let Some(child) = insertion.replaced.filter(|_| !insertion.replacement_removed) {
        let reference = {
            let dom = insertion.owner.dom.borrow();
            let children = light_children(&dom, insertion.parent)?;
            let index = children.iter().position(|node| *node == child)
                .ok_or(DomError::NotAChild(child, insertion.parent))?;
            children[index + 1..].iter().copied().find(|node| *node != insertion.node)
        };
        insertion.owner.dom.borrow_mut().replace_dom_child(
            insertion.parent, insertion.node, child, reference,
        ).map(|_| ())
    } else {
        insertion.owner.dom.borrow_mut().insert_dom_node(
            insertion.parent, insertion.node, insertion.reference,
        ).map(|_| ())
    };
    preparation.finish_mutation(scope, mutation)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_body_replacement_preserves_reactions_and_observer_records() {
        super::super::tests::check_global_event_prototype_script(r#"(()=>{
          const log=[];let reenter=false;
          customElements.define('body-reaction-probe',class extends HTMLElement {
            connectedCallback(){log.push('+'+this.id);if(reenter)this.appendChild(document.createElement('span'))}
            disconnectedCallback(){log.push('-'+this.id)}
          });
          const old=document.body,root=document.documentElement;
          const oldProbe=document.createElement('body-reaction-probe');oldProbe.id='old';old.appendChild(oldProbe);
          const next=document.createElement('body'),newProbe=document.createElement('body-reaction-probe');
          newProbe.id='new';next.appendChild(newProbe);log.length=0;reenter=true;
          const observer=new MutationObserver(()=>{});observer.observe(root,{childList:true});
          document.body=next;
          const records=observer.takeRecords();observer.disconnect();
          if(document.body!==next||old.parentNode!==null||newProbe.firstElementChild.localName!=='span')throw Error('reentrant connection');
          if(log.join('|')!=='-old|+new')throw Error('reaction ordering '+log.join('|'));
          return records.some(record=>record.target===root&&record.removedNodes[0]===old&&record.addedNodes[0]===next);
        })()"#);
    }
    #[test]
    fn document_last_modified_is_native_local_time_not_authored_date() {
        super::super::tests::initialize_v8();
        let mut isolate=v8::Isolate::new(Default::default());
        v8::scope!(let handles,&mut isolate);
        let context=v8::Context::new(handles,Default::default());
        let scope=&mut v8::ContextScope::new(handles,context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap()));
        let state=install_generated_bindings(scope,context,dom.clone(),RealmId(0),1).unwrap();
        dom.borrow_mut().arena_mut().get_mut(state.document_root()).unwrap()
            .document_metadata.as_mut().unwrap().last_modified=Some(784111777);
        let script=v8::String::new(scope,r#"(()=>{
          const pad=v=>String(v).padStart(2,'0'), date=new Date(784111777000);
          const expected=pad(date.getMonth()+1)+'/'+pad(date.getDate())+'/'+String(date.getFullYear()).padStart(4,'0')+
            ' '+pad(date.getHours())+':'+pad(date.getMinutes())+':'+pad(date.getSeconds());
          const descriptor=Object.getOwnPropertyDescriptor(Document.prototype,'lastModified');
          if(document.lastModified!==expected||descriptor.get.name!=='get lastModified'||descriptor.set!==undefined)throw Error('native local response timestamp');
          globalThis.Date=function(){throw Error('authored Date must not run')};
          if(document.lastModified!==expected)throw Error('authored date leak');
          const created=document.implementation.createHTMLDocument('');
          if(!/^\d{2}\/\d{2}\/\d{4} \d{2}:\d{2}:\d{2}$/.test(created.lastModified))throw Error('fallback format');
          let bad=false;try{descriptor.get.call({})}catch(e){bad=e instanceof TypeError}
          return bad;
        })()"#).unwrap();
        assert!(v8::Script::compile(scope,script,None).unwrap().run(scope).unwrap().is_true());
    }
    #[test]
    fn document_cookie_uses_native_receiver_and_rechecks_conversion_authority() {
        super::super::tests::initialize_v8();
        let mut isolate=v8::Isolate::new(Default::default());
        v8::scope!(let handles,&mut isolate);
        let context=v8::Context::new(handles,Default::default());
        let scope=&mut v8::ContextScope::new(handles,context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap()));
        let state=install_generated_bindings(scope,context,dom,RealmId(0),1).unwrap();
        state.commit_location("https://cookie.test/path").unwrap();
        let requests=Rc::new(RefCell::new(Vec::new()));
        let recorded=requests.clone();
        state.set_document_cookie_access(Rc::new(move |state,value| {
            assert_eq!(state.current_location_url(),"https://cookie.test/path");
            recorded.borrow_mut().push(value.clone());
            Ok(if value.is_none(){"a=b".into()}else{String::new()})
        }));
        fn retire(scope:&mut v8::PinScope,_args:v8::FunctionCallbackArguments,_rv:v8::ReturnValue) {
            scope.get_current_context().get_slot::<V8BindingState>().unwrap().retire_execution();
        }
        let function=v8::Function::new(scope,retire).unwrap();
        let key=v8::String::new(scope,"retireCookieOwner").unwrap();
        context.global(scope).create_data_property(scope,key.into(),function.into());
        let script=v8::String::new(scope,r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)};
          const d=Object.getOwnPropertyDescriptor(Document.prototype,'cookie');
          check(d.get.name==='get cookie'&&d.set.name==='set cookie','names');
          check(document.cookie==='a=b','native read');document.cookie='c=d';
          let converted=0,bad=false;
          try{d.set.call({}, {toString(){converted++;return 'bad'}})}catch(e){bad=e instanceof TypeError}
          check(bad&&converted===0,'receiver before conversion');
          const inert=document.implementation.createHTMLDocument('');
          check(inert.cookie==='','cookie-averse read');inert.cookie='ignored';
          bad=false;try{document.cookie={toString(){retireCookieOwner();return 'stale'}}}catch(e){bad=e.name==='SecurityError'}
          check(bad,'conversion authority');return true;
        })()"#).unwrap();
        assert!(v8::Script::compile(scope,script,None).unwrap().run(scope).unwrap().is_true());
        assert_eq!(&*requests.borrow(),&[None,Some("c=d".into())]);
    }
    #[test]
    fn document_location_is_own_unforgeable_even_without_browsing_context() {
        super::super::tests::check_global_event_prototype_script(r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)};
          check(!Object.hasOwn(Document.prototype,'location'),'prototype placement');
          for(const doc of [document,new Document(),document.implementation.createHTMLDocument('new')]) {
            const d=Object.getOwnPropertyDescriptor(doc,'location');
            check(d&&d.enumerable&&!d.configurable&&d.get.name==='get location'&&d.set.name==='set location','own unforgeable');
            check(d.get.length===0&&d.set.length===1,'arity');
            if(doc!==document)check(doc.location===null&&doc.defaultView===null,'no browsing context');
            let bad=false;try{d.get.call({})}catch(e){bad=e instanceof TypeError}check(bad,'receiver');
            bad=false;try{Object.defineProperty(doc,'location',{value:'forged'})}catch(e){bad=e instanceof TypeError}check(bad,'cannot replace');
          }
          return true;
        })()"#);
    }
    #[test]
    fn borrowed_document_default_view_uses_receiver_realm() {
        super::super::tests::initialize_v8();
        let mut isolate=v8::Isolate::new(Default::default());
        v8::scope!(let handles,&mut isolate);
        let token=v8::String::new(handles,"document-same-origin-fixture").unwrap();
        let mut owners=Vec::new();
        for index in 0..2 {
            let context=v8::Context::new(handles,Default::default());
            context.set_security_token(token.into());
            let scope=&mut v8::ContextScope::new(handles,context);
            let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap()));
            let document=dom.borrow().document();
            let state=install_generated_bindings(scope,context,dom,RealmId(index),1).unwrap();
            let wrapper=wrap_node(scope,&state,document).unwrap();
            let source=v8::String::new(scope,"Object.getOwnPropertyDescriptor(Document.prototype,'defaultView').get").unwrap();
            let getter=v8::Script::compile(scope,source,None).unwrap().run(scope).unwrap().try_cast::<v8::Function>().unwrap();
            owners.push((state,v8::Global::new(scope,context),v8::Global::new(scope,wrapper),v8::Global::new(scope,getter)));
        }
        for (caller,target) in [(0,1),(1,0)] {
            let context=v8::Local::new(handles,&owners[caller].1);
            let scope=&mut v8::ContextScope::new(handles,context);
            let getter=v8::Local::new(scope,&owners[caller].3);
            let receiver=v8::Local::new(scope,&owners[target].2);
            let expected=v8::Local::new(scope,&owners[target].1).global(scope);
            assert!(getter.call(scope,receiver.into(),&[]).unwrap().strict_equals(expected.into()));
        }
    }
    #[test]
    fn document_body_replacement_preserves_identity_and_rejects_invalid_values() {
        super::super::tests::check_global_event_prototype_script(r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)};
          const original=document.body;
          const descriptor=Object.getOwnPropertyDescriptor(Document.prototype,'body');
          check(descriptor.set.name==='set body'&&descriptor.set.length===1,'setter metadata');
          document.body=original;check(document.body===original,'same body');
          const frames=document.createElement('frameset');document.body=frames;
          check(document.body===frames&&original.parentNode===null,'frameset replacement');
          const body=document.createElement('body');document.body=body;
          check(document.body===body&&frames.parentNode===null,'body replacement');
          for(const value of [null,undefined,document.createElement('div')]) {
            let rejected=false;try{document.body=value}catch(e){rejected=e.name==='HierarchyRequestError'}
            check(rejected&&document.body===body,'invalid body cannot mutate');
          }
          const foreign=document.implementation.createHTMLDocument('foreign');
          const adopted=foreign.body;document.body=adopted;
          check(document.body===adopted&&adopted.ownerDocument===document&&foreign.body===null,'adoption');
          let rejected=false;try{descriptor.set.call({},body)}catch(e){rejected=e instanceof TypeError}
          check(rejected,'receiver');return true;
        })()"#);
    }
    #[test]
    fn document_specific_handlers_use_native_event_dispatch() {
        super::super::tests::check_global_event_prototype_script(r#"(()=>{
          let observed='';
          for(const type of ['readystatechange','visibilitychange']) {
            const name='on'+type,d=Object.getOwnPropertyDescriptor(Document.prototype,name);
            if(!d||!d.enumerable||!d.configurable||d.get.name!=='get '+name||d.set.name!=='set '+name)throw Error(name);
            const handler=function(e){if(this!==document||e.type!==type)throw Error('target');observed+=type+';'};
            document[name]=handler;if(document[name]!==handler)throw Error('identity');
            document.dispatchEvent(new Event(type));document[name]=null;
            if(document[name]!==null)throw Error('clear');
          }
          return observed==='readystatechange;visibilitychange;';
        })()"#);
    }
}
