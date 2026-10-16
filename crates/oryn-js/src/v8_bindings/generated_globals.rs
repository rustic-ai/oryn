//! WebIDL [Global] placement and native global-owned IndexedDB factory.
use super::*;

pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    state: &V8BindingState,
    binding_global: BindingGlobal,
    constructors: &BTreeMap<&'static str, v8::Global<v8::Function>>,
) -> Result<(), V8BindingError> {
    let selected = GENERATED_INTERFACES.iter().find(|interface| {
        if interface.global_names.is_empty()
            || (interface.secure_context && !state.secure_context.get())
            || !exposed_on_global(interface.exposed, binding_global, context) { return false; }
        if binding_global == BindingGlobal::Worker {
            let name = match context.get_slot::<WorkerExposureKind>().map(|kind| kind.0) {
                Some(crate::worker_wire::AgentKind::Shared) => "SharedWorker",
                Some(crate::worker_wire::AgentKind::Service) => "ServiceWorker",
                Some(crate::worker_wire::AgentKind::AudioWorklet) => return false,
                _ => "DedicatedWorker",
            };
            interface.global_names.contains(&name)
        } else {
            interface.global_names.contains(&binding_global.webidl_name())
        }
    });
    let Some(selected) = selected else { return Ok(()); };
    let global = context.global(scope);
    let mut installed = BTreeSet::new();
    let mut declaring = Some(selected);
    while let Some(interface) = declaring {
        for member in interface.members {
            if member.kind != GeneratedMemberKind::Attribute || member.static_member
                || member.unforgeable
                || (member.secure_context && !state.secure_context.get())
                || (!member.exposed.is_empty()
                    && !exposed_on_global(member.exposed, binding_global, context)) {
                continue;
            }
            if !installed.insert(member.name) { continue; }
            // Unforgeable global accessors retain their existing native root /
            // reinstall path (Location). Ordinary attributes use the same
            // generated callbacks and member IDs as prototype attributes.
            let name = v8::String::new(scope, member.name)
                .ok_or(V8BindingError::Allocation("global attribute name"))?;
            let data = v8::Integer::new(scope, member.id.0 as i32);
            let getter = v8::FunctionTemplate::builder(generated_attribute_getter)
                .data(data.into()).length(0)
                .constructor_behavior(v8::ConstructorBehavior::Throw)
                .build(scope).get_function(scope)
                .ok_or(V8BindingError::Allocation("global attribute getter"))?;
            getter.set_name(v8::String::new(scope, &format!("get {}", member.name))
                .ok_or(V8BindingError::Allocation("global getter name"))?);
            let setter: v8::Local<v8::Value> = if !member.readonly || member.replaceable
                || member.put_forwards.is_some() {
                let setter = v8::FunctionTemplate::builder(generated_attribute_setter)
                    .data(data.into()).length(1)
                    .constructor_behavior(v8::ConstructorBehavior::Throw)
                    .build(scope).get_function(scope)
                    .ok_or(V8BindingError::Allocation("global attribute setter"))?;
                setter.set_name(v8::String::new(scope, &format!("set {}", member.name))
                    .ok_or(V8BindingError::Allocation("global setter name"))?);
                setter.into()
            } else { v8::undefined(scope).into() };
            let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter);
            descriptor.set_enumerable(true);
            descriptor.set_configurable(true);
            if global.define_property(scope, name.into(), &descriptor) != Some(true) {
                return Err(V8BindingError::JavaScriptException);
            }
        }
        declaring = interface.parent.and_then(|name|
            GENERATED_INTERFACES.iter().find(|interface| interface.name == name));
    }
    if binding_global == BindingGlobal::Worker {
        let constructor = constructors.get(selected.name)
            .ok_or(V8BindingError::MissingInterface(selected.name))?;
        let constructor = v8::Local::new(scope, constructor);
        let key = v8::String::new(scope, "prototype")
            .ok_or(V8BindingError::Allocation("worker prototype key"))?;
        let prototype = constructor.get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if !crate::v8_context_bridge::set_global_object_prototype(scope, context, prototype) {
            return Err(V8BindingError::Allocation("worker global prototype"));
        }
    }
    Ok(())
}

pub(super) fn indexed_db<'s>(
    scope: &mut v8::PinScope<'s, '_>, state: &V8BindingState,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let id = if let Some(id) = state.idb_factory_object.get() { id } else {
        let id = register_platform_object(state, PlatformObjectRecord::IdbFactory);
        state.idb_factory_object.set(Some(id));
        id
    };
    let context = state.realm_context.to_local(scope)
        .ok_or(V8BindingError::Allocation("IndexedDB factory owner realm"))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let factory = new_platform_object(scope, state, "IDBFactory", id)?;
    context_intrinsics::retain_object_for(scope, state, "indexed-db-factory", factory)?;
    Ok(factory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_global_attributes_keep_native_names_unforgeables_and_same_object() {
        super::super::tests::check_global_event_prototype_script(r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)};
          for(const name of ['indexedDB','name','length','outerWidth','outerHeight','history']) {
            const d=Object.getOwnPropertyDescriptor(globalThis,name);
            check(d&&typeof d.get==='function'&&d.enumerable&&d.configurable,name);
            check(!Object.hasOwn(Window.prototype,name),'prototype '+name);
          }
          const d=Object.getOwnPropertyDescriptor(globalThis,'indexedDB');
          check(d.set===undefined&&d.get.length===0&&d.get.name==='get indexedDB','descriptor');
          const f=indexedDB;check(f===indexedDB&&f instanceof IDBFactory,'identity');
          name='global-name';check(name==='global-name','name write');
          const loc=location;check(!Object.getOwnPropertyDescriptor(globalThis,'location').configurable,'location');
          check(loc===location&&history===history,'native roots');
          let bad=false;try{d.get.call({})}catch(e){bad=e instanceof TypeError}check(bad,'receiver');
          check(Object.getPrototypeOf(Window.prototype)!==EventTarget.prototype,'named chain');
          return true;
        })()"#);
    }

    #[test]
    fn worker_globals_preserve_inherited_accessor_and_exact_realm_factory() {
        super::super::tests::initialize_v8();
        let mut isolate=v8::Isolate::new(Default::default());
        v8::scope!(let handles,&mut isolate);
        let mut owners=Vec::new();
        for (index,kind) in [crate::worker_wire::AgentKind::Dedicated,
            crate::worker_wire::AgentKind::Shared,crate::worker_wire::AgentKind::Service]
            .into_iter().enumerate() {
            let context=v8::Context::new(handles,Default::default());
            context.set_slot(Rc::new(WorkerExposureKind(kind)));
            context.set_slot(Rc::new(crate::realm::SecureContextState(index == 1)));
            let scope=&mut v8::ContextScope::new(handles,context);
            let dom=Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
            let document=dom.borrow().document();
            let state=install_generated_bindings_with_root_for_global(scope,context,dom,
                RealmId(index as u64),1,document,BindingGlobal::Worker).unwrap();
            let caches_key=v8::String::new(scope,"caches").unwrap();
            let own=context.global(scope).get_own_property_descriptor(scope,caches_key.into()).unwrap();
            assert_eq!(!own.is_undefined(),index == 1);
            let script=v8::String::new(scope,r#"(()=>{
              const inherited=Object.getOwnPropertyDescriptor(WorkerGlobalScope.prototype,'indexedDB');
              const own=Object.getOwnPropertyDescriptor(globalThis,'indexedDB');
              if(!inherited||!own||own.set!==undefined||!own.enumerable||!own.configurable)throw Error('placement');
              if(inherited.get.call(globalThis)!==indexedDB||indexedDB!==indexedDB)throw Error('identity');
              let bad=false;try{inherited.get.call({})}catch(e){bad=e instanceof TypeError}
              if(!bad)throw Error('receiver');return indexedDB;
            })()"#).unwrap();
            let value=v8::Script::compile(scope,script,None).unwrap().run(scope).unwrap();
            let factory=value.try_cast::<v8::Object>().unwrap();
            assert_eq!(factory.get_creation_context(scope).unwrap(),context);
            owners.push((state,v8::Global::new(scope,context),v8::Global::new(scope,factory)));
        }
        for i in 0..owners.len() {
            for j in 0..i {
                let a=v8::Local::new(handles,&owners[i].2);
                let b=v8::Local::new(handles,&owners[j].2);
                assert!(!a.strict_equals(b.into()));
                let caller=v8::Local::new(handles,&owners[i].1);
                let scope=&mut v8::ContextScope::new(handles,caller);
                let target=v8::Local::new(scope,&owners[j].1).global(scope);
                let member=GENERATED_INTERFACES.iter().find(|i|i.name=="WorkerGlobalScope")
                    .unwrap().members.iter().find(|m|m.name=="indexedDB").unwrap();
                let data=v8::Integer::new(scope,member.id.0 as i32);
                let getter=v8::FunctionTemplate::builder(generated_attribute_getter)
                    .data(data.into()).constructor_behavior(v8::ConstructorBehavior::Throw)
                    .build(scope).get_function(scope).unwrap();
                let result=getter.call(scope,target.into(),&[]).unwrap();
                let expected=v8::Local::new(scope,&owners[j].2);
                assert!(expected.strict_equals(result));
            }
        }
    }
    #[test]
    fn foreign_window_getter_uses_receiver_owner_not_function_realm() {
        super::super::tests::initialize_v8();
        let mut isolate=v8::Isolate::new(Default::default());
        v8::scope!(let handles,&mut isolate);
        let token=v8::String::new(handles,"same-origin-native-fixture").unwrap();
        let mut owners=Vec::new();
        for index in 0..2 {
            let context=v8::Context::new(handles,Default::default());
            context.set_security_token(handles,token.into());
            let scope=&mut v8::ContextScope::new(handles,context);
            let dom=Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
            let state=install_generated_bindings(scope,context,dom,RealmId(index),1).unwrap();
            // Do not materialize either factory: the borrowed getter is first access.
            owners.push((state,v8::Global::new(scope,context)));
        }
        let context=v8::Local::new(handles,&owners[0].1);
        let scope=&mut v8::ContextScope::new(handles,context);
        let target=v8::Local::new(scope,&owners[1].1).global(scope);
        let member=GENERATED_INTERFACES.iter().find(|i|i.name=="Window").unwrap()
            .members.iter().find(|m|m.name=="indexedDB").unwrap();
        let data=v8::Integer::new(scope,member.id.0 as i32);
        let getter=v8::FunctionTemplate::builder(generated_attribute_getter).data(data.into())
            .constructor_behavior(v8::ConstructorBehavior::Throw).build(scope)
            .get_function(scope).unwrap();
        let result=getter.call(scope,target.into(),&[]).unwrap();
        let factory=result.try_cast::<v8::Object>().unwrap();
        let target_context=v8::Local::new(scope,&owners[1].1);
        assert_eq!(factory.get_creation_context(scope).unwrap(),target_context);
        let expected=indexed_db(scope,&owners[1].0).unwrap();
        let caller=indexed_db(scope,&owners[0].0).unwrap();
        assert!(expected.strict_equals(result));
        assert!(!caller.strict_equals(result));
    }

    #[test]
    fn indexed_db_declaring_global_brand_rejects_other_global_before_materialization() {
        super::super::tests::initialize_v8();
        let mut isolate=v8::Isolate::new(Default::default());
        v8::scope!(let handles,&mut isolate);
        let mut owners=Vec::new();
        for (index,global) in [BindingGlobal::Window,BindingGlobal::Worker].into_iter().enumerate() {
            let context=v8::Context::new(handles,Default::default());
            let scope=&mut v8::ContextScope::new(handles,context);
            let dom=Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
            let document=dom.borrow().document();
            let state=install_generated_bindings_with_root_for_global(scope,context,dom,
                RealmId(index as u64),1,document,global).unwrap();
            assert!(state.idb_factory_object.get().is_none());
            owners.push((state,v8::Global::new(scope,context)));
        }
        for (declaring,target) in [("Window",1usize),("WorkerGlobalScope",0usize)] {
            let caller=v8::Local::new(handles,&owners[1-target].1);
            let scope=&mut v8::ContextScope::new(handles,caller);
            let receiver=v8::Local::new(scope,&owners[target].1).global(scope);
            let member=GENERATED_INTERFACES.iter().find(|i|i.name==declaring).unwrap()
                .members.iter().find(|m|m.name=="indexedDB").unwrap();
            let data=v8::Integer::new(scope,member.id.0 as i32);
            let getter=v8::FunctionTemplate::builder(generated_attribute_getter).data(data.into())
                .constructor_behavior(v8::ConstructorBehavior::Throw).build(scope)
                .get_function(scope).unwrap();
            v8::tc_scope!(let scope,scope);
            assert!(getter.call(scope,receiver.into(),&[]).is_none());
            let exception=scope.exception().unwrap();
            let name=v8::String::new(scope,"name").unwrap();
            assert_eq!(exception.to_object(scope).unwrap().get(scope,name.into()).unwrap()
                .to_rust_string_lossy(scope),"TypeError");
            assert!(owners[target].0.idb_factory_object.get().is_none());
            assert!(owners[1-target].0.idb_factory_object.get().is_none());
        }
    }

}
