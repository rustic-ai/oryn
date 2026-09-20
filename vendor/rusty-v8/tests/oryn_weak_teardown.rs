//! Regression target for the pinned Rust binding teardown repair.
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Once, RwLock};
// Match rusty-v8's test_api PROCESS_LOCK policy: ordinary isolate lifetimes
// may overlap, while snapshot creation owns the process through destruction.
// The extensible read-only snapshot can replace a group's read-only space.
static PROCESS_LOCK: RwLock<()> = RwLock::new(());
fn setup() {
    static START: Once = Once::new();
    START.call_once(|| {
        v8::V8::set_flags_from_string("--expose-gc");
        let platform=v8::new_default_platform(0,false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
        // Initialize the process-global sandbox allocator before concurrent tests.
        drop(v8::new_default_allocator());
    });
}
fn isolate()->v8::OwnedIsolate {
    setup();
    let heap=v8::cppgc::Heap::create(v8::V8::get_current_platform(),Default::default());
    v8::Isolate::new(v8::CreateParams::default().cpp_heap(heap))
}
#[test]
fn slot_owned_weaks_are_reset_before_cpp_heap_teardown() {
    let _process_guard = PROCESS_LOCK.read().unwrap();
    for _ in 0..8 {
        let mut isolate=isolate();
        {
            v8::scope!(let scope,&mut isolate);
            let context=v8::Context::new(scope,Default::default());
            let scope=&mut v8::ContextScope::new(scope,context);
            let mut values=Vec::new();
            for _ in 0..128 {
                let object=v8::Object::new(scope);
                values.push(v8::Weak::new(scope,object));
            }
            scope.set_slot(values);
        }
        drop(isolate);
    }
}
#[test]
fn guaranteed_finalizer_can_release_another_weak_after_revocation() {
    let _process_guard = PROCESS_LOCK.read().unwrap();
    let finalized=Rc::new(Cell::new(0));
    let (survivor,ordinary,global);
    {
        let mut isolate=isolate();
        v8::scope!(let scope,&mut isolate);
        let context=v8::Context::new(scope,Default::default());
        let scope=&mut v8::ContextScope::new(scope,context);
        let object=v8::Object::new(scope);
        global=v8::Global::new(scope,object);
        let nested=v8::Weak::new(scope,object);
        let count=finalized.clone();
        survivor=v8::Weak::with_guaranteed_finalizer(scope,object,Box::new(move||{
            assert!(nested.is_empty());
            drop(nested);
            count.set(count.get()+1);
        }));
        ordinary=v8::Weak::with_finalizer(scope,object,Box::new(|_|panic!("ordinary finalizer ran at isolate disposal")));
    }
    assert_eq!(finalized.get(),1);
    assert!(survivor.is_empty());assert!(ordinary.is_empty());
    drop(survivor);drop(ordinary);drop(global);
    assert_eq!(finalized.get(),1);
}
#[test]
fn weak_raw_roundtrip_and_drop_cancels_only_its_own_callback() {
    let _process_guard = PROCESS_LOCK.read().unwrap();
    let count=Rc::new(Cell::new(0));
    let late;
    {
        let mut isolate=isolate();
        v8::scope!(let scope,&mut isolate);
        let context=v8::Context::new(scope,Default::default());
        let scope=&mut v8::ContextScope::new(scope,context);
        let object=v8::Object::new(scope);
        let count2=count.clone();
        let weak=v8::Weak::with_guaranteed_finalizer(scope,object,Box::new(move||count2.set(count2.get()+1)));
        let raw=weak.into_raw();
        let restored=unsafe{v8::Weak::from_raw(scope,raw)};
        late=restored.clone();
        assert!(!restored.is_empty());
        drop(restored);
        assert!(!late.is_empty());
    }
    assert_eq!(count.get(),0);assert!(late.is_empty());drop(late);
}
#[test]
fn ordinary_gc_finalizer_can_drop_its_own_weak() {
    let _process_guard = PROCESS_LOCK.read().unwrap();
    let owner=Rc::new(RefCell::new(None::<v8::Weak<v8::Object>>));
    let count=Rc::new(Cell::new(0));
    let mut isolate=isolate();
    {
        v8::scope!(let scope,&mut isolate);
        let context=v8::Context::new(scope,Default::default());
        let scope=&mut v8::ContextScope::new(scope,context);
        let object=v8::Object::new(scope);
        let owner2=owner.clone();let count2=count.clone();
        *owner.borrow_mut()=Some(v8::Weak::with_finalizer(scope,object,Box::new(move|_|{
            owner2.borrow_mut().take();count2.set(count2.get()+1);
        })));
    }
    isolate.request_garbage_collection_for_testing(v8::GarbageCollectionType::Full);
    assert_eq!(count.get(),1);assert!(owner.borrow().is_none());
    drop(isolate);assert_eq!(count.get(),1);
}
#[test]
fn snapshot_teardown_keeps_weak_parameters_until_creator_is_destroyed() {
    let _process_guard = PROCESS_LOCK.write().unwrap();
    setup();
    let count=Rc::new(Cell::new(0));
    let late;
    let finalizer;
    let mut creator=v8::Isolate::snapshot_creator(None,None);
    {
        v8::scope!(let scope,&mut creator);
        let context=v8::Context::new(scope,Default::default());
        let scope=&mut v8::ContextScope::new(scope,context);
        let object=v8::Object::new(scope);
        late=v8::Weak::new(scope,object);
        let nested=v8::Weak::new(scope,object);
        let count2=count.clone();
        finalizer=v8::Weak::with_guaranteed_finalizer(scope,object,Box::new(move||{
            assert!(nested.is_empty());drop(nested);count2.set(count2.get()+1);
        }));
        // Preserve the finalizer owner outside slots so it is not canceled by
        // slot destruction before the guaranteed-finalizer teardown drain.
        let slot_weak=v8::Weak::new(scope,object);
        scope.set_slot(slot_weak);
        scope.set_default_context(context);
    }
    let blob=creator.create_blob(v8::FunctionCodeHandling::Clear).unwrap();
    assert!(!blob.is_empty());assert!(late.is_empty());drop(late);
    assert_eq!(count.get(),1);
    assert!(finalizer.is_empty());drop(finalizer);
    assert_eq!(count.get(),1);
}
#[test]
fn cpp_heap_owner_releases_weak_during_native_isolate_teardown() {
    let _process_guard = PROCESS_LOCK.read().unwrap();
    struct Holder { weak:v8::Weak<v8::Object>, dropped:Rc<Cell<usize>> }
    // Atomic sweeping below guarantees this thread-confined Weak is destroyed
    // on the isolate's thread; the weak edge must not keep the JS object alive.
    unsafe impl v8::cppgc::GarbageCollected for Holder {
        fn trace(&self,_:&mut v8::cppgc::Visitor) {}
        fn get_name(&self)->&'static std::ffi::CStr { c"OrynWeakTeardownHolder" }
    }
    impl Drop for Holder {
        fn drop(&mut self){assert!(self.weak.is_empty());self.dropped.set(self.dropped.get()+1);}
    }
    setup();let dropped=Rc::new(Cell::new(0));
    let heap=v8::cppgc::Heap::create(v8::V8::get_current_platform(),v8::cppgc::HeapCreateParams{
        marking_support:v8::cppgc::MarkingType::Atomic,
        sweeping_support:v8::cppgc::SweepingType::Atomic,
    });
    let mut isolate=v8::Isolate::new(v8::CreateParams::default().cpp_heap(heap));
    {
        v8::scope!(let scope,&mut isolate);
        let context=v8::Context::new(scope,Default::default());
        let scope=&mut v8::ContextScope::new(scope,context);
        let object=v8::Object::new(scope);
        let holder=Holder{weak:v8::Weak::new(scope,object),dropped:dropped.clone()};
        unsafe{v8::cppgc::make_garbage_collected(scope.get_cpp_heap().unwrap(),holder)};
    }
    drop(isolate);assert_eq!(dropped.get(),1);
}
