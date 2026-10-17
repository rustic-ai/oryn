// Included only under cfg(test) in owning native FontFaceSet module.
mod settlement_controls {
    use super::*;
    thread_local! {static FAIL_AFTER:std::cell::Cell<Option<usize>>=const{std::cell::Cell::new(None)};}
    thread_local! {static SUSPEND_ON_SUCCESS:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};static PUBLICATIONS:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
    struct Reset;
    impl Drop for Reset{fn drop(&mut self){FAIL_AFTER.with(|v|v.set(None));SUSPEND_ON_SUCCESS.with(|v|v.set(false));PUBLICATIONS.with(|v|v.set(0));}}
    pub(super) fn after_publication(state:&Rc<V8BindingState>){if SUSPEND_ON_SUCCESS.with(|v|v.replace(false)){state.set_execution_suspended(true);}}
    pub(super) fn before_publication()->Result<(),V8BindingError>{
        PUBLICATIONS.with(|v|v.set(v.get()+1));
        FAIL_AFTER.with(|remaining|match remaining.get(){
            Some(0)=>{remaining.set(None);Err(V8BindingError::Allocation("injected native font publication failure"))},
            Some(n)=>{remaining.set(Some(n-1));Ok(())},None=>Ok(())
        })
    }
    #[test]
    fn native_three_ready_loads_preserve_failed_and_later_originals(){
        let _reset=Reset;
        super::super::tests::initialize_v8();
        let mut isolate=v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context=v8::Context::new(scope,Default::default());
        let scope=&mut v8::ContextScope::new(scope,context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<!doctype html><p>font settlement</p>").into_runtime().unwrap()));
        let state=install_generated_bindings(scope,context,dom,RealmId(0),1).unwrap();
        let mut promises=Vec::new();
        font_prepare_load_capacity(scope,&state).unwrap();
        for _ in 0..3{
            font_prepare_load_capacity(scope,&state).unwrap();
            let resolver=v8::PromiseResolver::new(scope).unwrap();let promise=resolver.get_promise(scope);
            promises.push(v8::Global::new(scope,promise));let array=v8::Array::new(scope,0);
            state.font_loading.borrow_mut().loads.push(LoadRecord{published:false,_faces:None,faces:Vec::new(),resolver:v8::Global::new(scope,resolver),array:v8::Global::new(scope,array)});
        }
        FAIL_AFTER.with(|remaining|remaining.set(Some(1)));
        assert!(matches!(settle_loads(scope,&state),Err(V8BindingError::Allocation("injected native font publication failure"))));
        assert_eq!(state.font_loading.borrow().loads.len(),2);
        assert_eq!(v8::Local::new(scope,&promises[0]).state(),v8::PromiseState::Fulfilled);
        assert_eq!(v8::Local::new(scope,&promises[1]).state(),v8::PromiseState::Pending);
        assert_eq!(v8::Local::new(scope,&promises[2]).state(),v8::PromiseState::Pending);
        settle_loads(scope,&state).unwrap();assert!(state.font_loading.borrow().loads.is_empty());
        for promise in promises{assert_eq!(v8::Local::new(scope,&promise).state(),v8::PromiseState::Fulfilled);}
        settle_loads(scope,&state).unwrap();assert!(state.font_loading.borrow().loads.is_empty());
    }
    #[test]
    fn native_successful_load_suspension_resumes_without_republication(){
        let _reset=Reset;
        super::super::tests::initialize_v8();
        let mut isolate=v8::Isolate::new(Default::default());isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);let context=v8::Context::new(scope,Default::default());let scope=&mut v8::ContextScope::new(scope,context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<!doctype html><p>font suspend</p>").into_runtime().unwrap()));
        let state=install_generated_bindings(scope,context,dom,RealmId(0),1).unwrap();
        let mut promises=Vec::new();
        for _ in 0..3{font_prepare_load_capacity(scope,&state).unwrap();let resolver=v8::PromiseResolver::new(scope).unwrap();promises.push(v8::Global::new(scope,resolver.get_promise(scope)));let array=v8::Array::new(scope,0);state.font_loading.borrow_mut().loads.push(LoadRecord{published:false,_faces:None,faces:Vec::new(),resolver:v8::Global::new(scope,resolver),array:v8::Global::new(scope,array)});}
        SUSPEND_ON_SUCCESS.with(|v|v.set(true));settle_loads(scope,&state).unwrap();
        assert!(state.execution_suspended());assert_eq!(state.font_loading.borrow().loads.len(),3);assert!(state.font_loading.borrow().loads[0].published);
        assert_eq!(PUBLICATIONS.with(|v|v.get()),1);assert_eq!(v8::Local::new(scope,&promises[0]).state(),v8::PromiseState::Fulfilled);
        assert_eq!(v8::Local::new(scope,&promises[1]).state(),v8::PromiseState::Pending);
        state.set_execution_suspended(false);settle_loads(scope,&state).unwrap();
        assert!(state.font_loading.borrow().loads.is_empty());assert_eq!(PUBLICATIONS.with(|v|v.get()),3);
        for promise in promises{assert_eq!(v8::Local::new(scope,&promise).state(),v8::PromiseState::Fulfilled);}
    }
}
