//! Platform-provided submit behavior. Public properties are projections of native
//! records; DOM algorithms never consult replaceable behavior/element properties.
use super::*;
#[derive(Default)]
pub(super) struct Registry {
    constructor: RefCell<Option<v8::Weak<v8::Function>>>,
}
#[derive(Default)]
pub(super) struct Record {
    data: oryn_dom::SubmitButtonBehavior,
    target: Option<Rc<oryn_dom::TrackedNodeLease>>,
}
pub(super) fn release_collected(state: &V8BindingState, id: u64) {
    if matches!(
        state.platform_objects.borrow().get(&id),
        Some(PlatformObjectRecord::SubmitButtonBehavior(_))
    ) {
        state.platform_objects.borrow_mut().remove(&id);
        state.platform_wrappers.borrow_mut().remove(&id);
    }
}
fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "HTMLSubmitButtonBehavior".into(),
        member: "TypeError".into(),
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    constructors: &BTreeMap<&'static str, v8::Global<v8::Function>>,
) -> Result<(), V8BindingError> {
    // Window-only interfaces are intentionally absent from worker globals.
    let Some(constructor) = constructors.get("HTMLSubmitButtonBehavior") else {
        state.submit_behaviors.constructor.borrow_mut().take();
        return Ok(());
    };
    let constructor = v8::Local::new(scope, constructor);
    *state.submit_behaviors.constructor.borrow_mut() = Some(context_intrinsics::retain_function(
        scope,
        "submit-behavior-constructor",
        constructor,
    )?);
    Ok(())
}
pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    let constructor = state
        .submit_behaviors
        .constructor
        .borrow()
        .as_ref()
        .and_then(|constructor| constructor.to_local(scope))
        .ok_or_else(invalid)?;
    if !constructor.strict_equals(args.new_target()) {
        return Err(invalid());
    }
    let id = register_platform_object(
        state,
        PlatformObjectRecord::SubmitButtonBehavior(Rc::new(RefCell::new(Record::default()))),
    );
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}
fn record(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<(Rc<V8BindingState>, Rc<RefCell<Record>>), V8BindingError> {
    if object.is_proxy() {
        return Err(invalid());
    }
    let state = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
    let id = stateful_id(scope, object).map_err(|_| invalid())?;
    let entry = platform_object(&state, id)?;
    let PlatformObjectRecord::SubmitButtonBehavior(data) = &*entry else {
        return Err(invalid());
    };
    let data = data.clone();
    drop(entry);
    Ok((state, data))
}
fn key<'s>(scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, "oryn.internals.behaviors")
        .ok_or(V8BindingError::Allocation("behaviors key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}
fn target_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, "oryn.behavior.target")
        .ok_or(V8BindingError::Allocation("behavior target key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}
pub(super) struct Attachment {
    record: Option<Rc<RefCell<Record>>>,
    values: v8::Global<v8::Array>,
}
pub(super) fn prepare(
    scope: &mut v8::PinScope,
    options: v8::Local<v8::Value>,
) -> Result<Attachment, V8BindingError> {
    let mut values = Vec::new();
    let mut records = Vec::new();
    if !options.is_null_or_undefined() {
        let object = options.try_cast::<v8::Object>().map_err(|_| invalid())?;
        let name = v8_string(scope, "behaviors")?;
        let sequence = object
            .get(scope, name)
            .ok_or(V8BindingError::JavaScriptException)?;
        if !sequence.is_undefined() {
            let object = sequence.try_cast::<v8::Object>().map_err(|_| invalid())?;
            let symbol = v8::Symbol::get_iterator(scope);
            let method = object
                .get(scope, symbol.into())
                .ok_or(V8BindingError::JavaScriptException)?
                .try_cast::<v8::Function>()
                .map_err(|_| invalid())?;
            iterable_conversion::consume(scope, object, method, |scope, value| {
                if values.len() >= 4096 {
                    return Err(V8BindingError::Allocation("behavior sequence limit"));
                }
                let object = value.try_cast::<v8::Object>().map_err(|_| invalid())?;
                let (_, data) = record(scope, object)?;
                records.push(data);
                values.push(v8::Global::new(scope, value));
                Ok(())
            })?;
        }
    }
    if records.len() > 1
        || records
            .iter()
            .any(|record| record.borrow().target.is_some())
    {
        return Err(invalid());
    }
    let values = values
        .iter()
        .map(|value| v8::Local::new(scope, value))
        .collect::<Vec<_>>();
    let array = v8::Array::new_with_elements(scope, &values);
    if array.set_integrity_level(scope, v8::IntegrityLevel::Frozen) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(Attachment {
        record: records.pop(),
        values: v8::Global::new(scope, array),
    })
}
impl Attachment {
    pub(super) fn commit(
        self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        node: NodeId,
        internals: v8::Local<v8::Object>,
    ) -> Result<(), V8BindingError> {
        let target = if self.record.is_some() {
            Some(Rc::new(oryn_dom::TrackedNodeLease::new(&state.dom, node)?))
        } else {
            None
        };
        let key = key(scope)?;
        let values = v8::Local::new(scope, self.values);
        let host = wrap_node(scope, state, node)?;
        let name = v8::String::new(scope, "oryn.host.internals")
            .ok_or(V8BindingError::Allocation("internals owner edge"))?;
        let host_key = v8::Private::for_api(scope, Some(name));
        // The host owns its internals and behavior list even if script keeps
        // neither return value. Both directions are ordinary traced JS edges,
        // so an unreachable host/internals/behavior cycle remains collectible.
        if internals.set_private(scope, key, values.into()) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
        if let Some(record) = self.record {
            if record.borrow().target.is_some() {
                return Err(invalid());
            }
            let behavior = values
                .get_index(scope, 0)
                .ok_or(V8BindingError::JavaScriptException)?
                .try_cast::<v8::Object>()
                .map_err(|_| invalid())?;
            let target_key = target_key(scope)?;
            if behavior.set_private(scope, target_key, host.into()) != Some(true) {
                return Err(V8BindingError::JavaScriptException);
            }
            if host.set_private(scope, host_key, internals.into()) != Some(true) {
                behavior.delete_private(scope, target_key);
                return Err(V8BindingError::JavaScriptException);
            }
            if let Err(error) = state
                .dom
                .borrow_mut()
                .set_submit_behavior(node, record.borrow().data.clone())
            {
                behavior.delete_private(scope, target_key);
                host.delete_private(scope, host_key);
                return Err(error.into());
            }
            record.borrow_mut().target = target;
        } else if host.set_private(scope, host_key, internals.into()) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
        Ok(())
    }
}

pub(super) fn behaviors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let object = associated_wrapper(scope, state, node, "ElementInternals")?
        .try_cast::<v8::Object>()
        .map_err(|_| invalid())?;
    let key = key(scope)?;
    object
        .get_private(scope, key)
        .ok_or(V8BindingError::JavaScriptException)
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let (state, record) = record(scope, object)?;
    let data = record.borrow().data.clone();
    match member {
        "disabled" => Ok(v8::Boolean::new(scope, data.disabled).into()),
        "formNoValidate" => Ok(v8::Boolean::new(scope, data.form_no_validate).into()),
        "form" | "labels" => {
            let target_key = target_key(scope)?;
            let target = object
                .get_private(scope, target_key)
                .ok_or(V8BindingError::JavaScriptException)?;
            let target = if target.is_undefined() {
                None
            } else {
                node_identity::argument(scope, target)?
            };
            if let Some((owner, node)) = target {
                if member == "labels" {
                    node_lists::labels(scope, &owner, node)
                } else {
                    let form = {
                        let dom = owner.dom.borrow();
                        if dom.is_form_associated_custom_element(node) {
                            dom.form_owner(node)?
                        } else {
                            None
                        }
                    };
                    optional_node(scope, &owner, form)
                }
            } else if member == "form" {
                Ok(v8::null(scope).into())
            } else {
                node_lists::snapshot(scope, &state, &[])
            }
        }
        _ => v8_string(
            scope,
            match member {
                "formAction" => &data.form_action,
                "formMethod" => &data.form_method,
                "formEnctype" => &data.form_enctype,
                "formTarget" => &data.form_target,
                "name" => &data.name,
                "value" => &data.value,
                _ => return Err(invalid()),
            },
        ),
    }
}
pub(super) fn set(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    member: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let (_, record) = record(scope, object)?;
    let text = if matches!(member, "disabled" | "formNoValidate") {
        None
    } else {
        let value = value
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        Some(rust_string_preserving_utf16(scope, value)?)
    };
    let mut data = record.borrow().data.clone();
    match member {
        "disabled" => data.disabled = value.boolean_value(scope),
        "formNoValidate" => data.form_no_validate = value.boolean_value(scope),
        "formAction" => data.form_action = text.unwrap(),
        "formMethod" => data.form_method = text.unwrap(),
        "formEnctype" => data.form_enctype = text.unwrap(),
        "formTarget" => data.form_target = text.unwrap(),
        "name" => data.name = text.unwrap(),
        "value" => data.value = text.unwrap(),
        _ => return Err(invalid()),
    }
    let changed = record.borrow().data != data;
    let target = record.borrow().target.clone();
    if let Some(target) = target {
        let (dom, node) = target.current();
        dom.borrow_mut().set_submit_behavior(node, data.clone())?;
        if changed {
            custom_elements::mark_internals_changed_in_dom(scope, &dom, node);
        }
    }
    record.borrow_mut().data = data;
    Ok(())
}

#[cfg(test)]
mod regressions {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn window_only_constructor_cache_does_not_require_worker_exposure() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles,&mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let document = dom.borrow().document();
        let state = install_generated_bindings_with_root_for_global(
            scope,
            context,
            dom,
            RealmId(906),
            1,
            document,
            BindingGlobal::Worker,
        )
        .unwrap();
        assert!(state.submit_behaviors.constructor.borrow().is_none());
        assert!(
            eval(
                scope,
                "typeof HTMLSubmitButtonBehavior==='undefined' && typeof Blob==='function'"
            )
            .is_true()
        );
    }
    #[test]
    fn native_attachment_brand_identity_and_form_entry_authority() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<form id=f><x-submit id=x name=ignored></x-submit><x-submit id=y></x-submit><button id=b>native</button></form><label id=l for=x>Submit</label><form id=g></form><x-plain id=p></x-plain>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)},throws=(fn)=>{let threw=false;try{fn()}catch(e){threw=e instanceof TypeError}check(threw,'native TypeError')};
          throws(()=>new(class extends HTMLSubmitButtonBehavior {})());
          const fresh=new HTMLSubmitButtonBehavior();check(fresh.form===null&&fresh.labels.length===0&&!fresh.disabled&&fresh.name==='','defaults');
          customElements.define('x-submit',class extends HTMLElement{static formAssociated=true;constructor(){super();this.behavior=new HTMLSubmitButtonBehavior();this.i=this.attachInternals({behaviors:[this.behavior]});this.i.setFormValue('must-not-submit');}});
          customElements.define('x-plain',class extends HTMLElement{constructor(){super();this.behavior=new HTMLSubmitButtonBehavior();this.i=this.attachInternals({behaviors:[this.behavior]});}});
          check(x.i.behaviors[0]===x.behavior&&Object.isFrozen(x.i.behaviors),'frozen identity');
          check(x.behavior.form===f&&x.behavior.labels[0]===l,'native owner and labels');
          x.behavior.name='action';x.behavior.value='save';y.behavior.name='other';y.behavior.value='ignored';
          check(new FormData(f).get('ignored')===null&&new FormData(f).get('action')===null,'setFormValue ignored for submit behavior');
          check([...new FormData(f,x)].map(([k,v])=>k+':'+v).join()==='action:save','only selected behavior entry');
          x.setAttribute('name','authored');x.setAttribute('value','authored');check(new FormData(f,x).get('action')==='save','attributes cannot replace native slots');
          check(x.matches(':default')&&!y.matches(':default')&&!b.matches(':default'),'tree ordered default');
          x.behavior.disabled=true;check(x.matches(':default')&&!new FormData(f,x).has('action'),'disabled excludes entries but stays default');x.behavior.disabled=false;
          x.setAttribute('form','g');check(x.behavior.form===g&&y.matches(':default'),'external association');x.removeAttribute('form');
          f.append(p);check(p.behavior.form===null&&!p.matches(':default'),'non-FACE cannot submit');p.focus();check(document.activeElement===p&&p.tabIndex===0,'non-FACE implicit focus');
          p.behavior.disabled=true;p.blur();p.focus();check(document.activeElement!==p,'disabled native focus');
          const detached=document.createElement('div');detached.append(x);check(x.behavior.form===null&&x.i.behaviors[0]===x.behavior,'detach preserves attached behavior');f.prepend(x);
          let cloneRejected=false;try{structuredClone(x.behavior)}catch(e){cloneRejected=e.name==='DataCloneError'}check(cloneRejected,'native behavior cannot be cloned');
          customElements.define('x-options',class extends HTMLElement{});
          for(const list of [[x.behavior],[fresh,fresh],[new HTMLSubmitButtonBehavior(),new HTMLSubmitButtonBehavior()],[{}],[new Proxy(fresh,{})]]){const host=document.createElement('x-options');throws(()=>host.attachInternals({behaviors:list}));check(host.attachInternals({behaviors:[]}).behaviors.length===0,'failed attachment leaves host reusable');}
          const reentrant=document.createElement('x-options'),outerBehavior=new HTMLSubmitButtonBehavior();let nestedInternals;
          const reentrantOptions={get behaviors(){nestedInternals=reentrant.attachInternals();return [outerBehavior]}};
          let outerError;try{reentrant.attachInternals(reentrantOptions)}catch(e){outerError=e}
          check(outerError?.name==='NotSupportedError'&&nestedInternals.behaviors.length===0,'conversion reentrancy cannot replace existing internals');
          const available=document.createElement('x-options');check(available.attachInternals({behaviors:[outerBehavior]}).behaviors[0]===outerBehavior,'failed outer attachment does not consume behavior');
          const claimed=new HTMLSubmitButtonBehavior(),claimant=document.createElement('x-options'),loser=document.createElement('x-options');let steps=0;
          const racing={[Symbol.iterator](){return {next(){steps++;if(steps===1)return {done:false,value:claimed};claimant.attachInternals({behaviors:[claimed]});return {done:true}}}}};
          throws(()=>loser.attachInternals({behaviors:racing}));check(loser.attachInternals().behaviors.length===0,'iteration-time claim leaves losing host reusable');
          const saved=x,behavior=saved.behavior,other=document.implementation.createHTMLDocument('foreign'),foreignForm=other.createElement('form');other.body.append(foreignForm);
          behavior.value={toString(){other.adoptNode(saved);foreignForm.append(saved);return 'after-adoption'}};check(behavior.form===foreignForm&&new FormData(foreignForm,saved).get('action')==='after-adoption','conversion adopts before native slot mutation');f.prepend(saved);
          const wrong=Object.getOwnPropertyDescriptor(HTMLSubmitButtonBehavior.prototype,'value').get;throws(()=>wrong.call({}));throws(()=>wrong.call(new Proxy(x.behavior,{})));
          return true;
        })()"#).is_true());
        let dom = dom.borrow();
        let x = dom.find_by_id("x").unwrap();
        let record = dom.arena().get(x).unwrap();
        assert_eq!(
            record.accessibility_attribute("role").map(String::as_str),
            Some("button")
        );
        assert!(record.attributes.get_null("role").is_none());
    }
    #[test]
    fn host_traces_behavior_without_author_fields_and_unreachable_cycle_releases_lease() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "customElements.define('x-hidden-behavior',class extends HTMLElement{static formAssociated=true;constructor(){super();this.attachInternals({behaviors:[new HTMLSubmitButtonBehavior()]})});globalThis.host=document.createElement('x-hidden-behavior');true",
        );
        let id = state
            .platform_objects
            .borrow()
            .iter()
            .find_map(|(id, record)| {
                matches!(record, PlatformObjectRecord::SubmitButtonBehavior(_)).then_some(*id)
            })
            .unwrap();
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert!(state.platform_objects.borrow().contains_key(&id));
        eval(scope, "host=null;true");
        scope.low_memory_notification();
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert!(!state.platform_objects.borrow().contains_key(&id));
    }
    #[test]
    fn native_enter_and_space_dispatch_trusted_behavior_click_once() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<form id=f><x-submit id=x></x-submit></form>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.clicks=[];globalThis.submits=0;customElements.define('x-submit',class extends HTMLElement{static formAssociated=true;constructor(){super();this.behavior=new HTMLSubmitButtonBehavior();this.attachInternals({behaviors:[this.behavior]})}});x.addEventListener('click',e=>clicks.push(e.isTrusted));f.addEventListener('submit',e=>{submits++;e.preventDefault()});x.focus()",
        );
        for (key, down) in [("Enter", true), ("Enter", false), (" ", true)] {
            native_keyboard::dispatch(
                scope,
                &state,
                &native_keyboard::NativeKeyboardInput {
                    key: key.encode_utf16().collect(),
                    down,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        assert!(eval(scope, "submits===1&&clicks.join()==='true'").is_true());
        native_keyboard::dispatch(
            scope,
            &state,
            &native_keyboard::NativeKeyboardInput {
                key: vec![32],
                down: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(eval(scope, "submits===2&&clicks.join()==='true,true'").is_true());
        eval(scope, "x.behavior.disabled=true");
        native_keyboard::dispatch(
            scope,
            &state,
            &native_keyboard::NativeKeyboardInput {
                key: "Enter".encode_utf16().collect(),
                down: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(eval(scope, "submits===2").is_true());
    }
    #[test]
    fn behavior_activation_validation_and_dialog_use_native_submitter() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<form id=f><input id=required required><x-submit id=x></x-submit></form><dialog id=d><form id=df method=dialog><x-submit id=finish></x-submit></form></dialog>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)};let submits=0;
          customElements.define('x-submit',class extends HTMLElement{static formAssociated=true;constructor(){super();this.behavior=new HTMLSubmitButtonBehavior();this.i=this.attachInternals({behaviors:[this.behavior]});}});
          f.addEventListener('submit',e=>{check(e.submitter===x,'native submitter');submits++;e.preventDefault()});required.addEventListener('invalid',e=>e.preventDefault());
          x.click();check(submits===0,'validation blocks');x.behavior.formNoValidate=true;x.click();check(submits===1,'behavior override');x.behavior.disabled=true;x.click();check(submits===1,'disabled prevents activation');
          x.behavior.disabled=false;x.addEventListener('click',e=>e.preventDefault(),{once:true});x.click();check(submits===1,'cancelled click');
          f.requestSubmit(x);check(submits===2,'explicit requestSubmit');
          d.showModal();finish.behavior.value='native-result';finish.setAttribute('value','spoof');finish.click();check(!d.open&&d.returnValue==='native-result','dialog return value');
          return true;
        })()"#).is_true());
    }
}

pub(super) fn owned_native_root(record: &Record) -> Option<Rc<oryn_dom::TrackedNodeLease>> {
    record.target.clone()
}
