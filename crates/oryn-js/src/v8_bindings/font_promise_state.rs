// Included in owning FontFaceSet module. V8 handles are weak caches; the ready
// promise and loaded promise are retained by private edges on their wrappers.
#[derive(Clone)]
struct PromiseSlot {
    resolver: Rc<v8::Weak<v8::PromiseResolver>>,
    settled: bool,
    _allocation:Option<std::sync::Arc<dyn Send+Sync>>,
}
fn promise_slot<'s>(
    scope:&mut v8::PinScope<'s,'_>,state:&Rc<V8BindingState>, wrapper:v8::Local<'s,v8::Object>, key:&str,
) -> Result<PromiseSlot,V8BindingError> {
    let allocation=reserve_font_container(scope,state,std::mem::size_of::<v8::Weak<v8::PromiseResolver>>().checked_add(2*std::mem::size_of::<usize>()).ok_or(V8BindingError::Allocation("font promise record bytes"))?)?;
    let resolver=v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("font promise"))?;
    let promise=resolver.get_promise(scope);
    let name=v8::String::new(scope,key).ok_or(V8BindingError::Allocation("font promise key"))?;
    let private=v8::Private::for_api(scope,Some(name));
    if wrapper.set_private(scope,private,promise.into())!=Some(true){return Err(V8BindingError::JavaScriptException)}
    let owner=v8::String::new(scope,"oryn.font.promise.owner").ok_or(V8BindingError::Allocation("font promise owner"))?;
    let owner=v8::Private::for_api(scope,Some(owner));
    if promise.set_private(scope,owner,wrapper.into())!=Some(true){return Err(V8BindingError::JavaScriptException)}
    // A pending resolver is reachable from its promise through a native private
    // edge, not a permanent Global root on the Rust cache.
    let key=v8::String::new(scope,"oryn.font.promise.resolver").ok_or(V8BindingError::Allocation("font resolver key"))?;
    let key=v8::Private::for_api(scope,Some(key));
    if promise.set_private(scope,key,resolver.into())!=Some(true){return Err(V8BindingError::JavaScriptException)}
    Ok(PromiseSlot{_allocation:allocation,resolver:Rc::new(v8::Weak::new(scope,resolver)),settled:false})
}
fn promise<'s>(scope:&mut v8::PinScope<'s,'_>, slot:&PromiseSlot)
    ->Result<v8::Local<'s,v8::Promise>,V8BindingError> {
    slot.resolver.to_local(scope).map(|resolver|resolver.get_promise(scope))
        .ok_or(V8BindingError::Allocation("font promise collected"))
}
struct FaceRecord {
    owner:font_loading_transport::FontLoadOwner,
    completed_cycle:Option<u64>,
    cycle_id:font_cycles::FaceId,
    native:Rc<dyn NativeFontFace>,
    loaded:PromiseSlot,
    connected:bool,
    observed_revision:u64,
}
struct SetRecord {
    pending_edges:Option<(Vec<u64>,Option<std::sync::Arc<dyn Send+Sync>>)>,
    retired_edges:Option<(Vec<u64>,Option<std::sync::Arc<dyn Send+Sync>>)>,
    faces:Vec<u64>,
    iteration_order:Vec<(u64,u64)>,
    next_serial:u64,
    membership_revision:u64,
    members_capacity:Option<std::sync::Arc<dyn Send+Sync>>,
    iteration_capacity:Option<std::sync::Arc<dyn Send+Sync>>,
    ready:PromiseSlot,
    ready_cycle:u64,
    ready_published:Option<u64>,
    loading_event_cycle:Option<u64>,
    done_event_cycle:Option<u64>,
    error_event_cycle:Option<u64>,
    document:Option<NodeId>,
}
pub(super) struct State {
    request_retry:Option<font_loading_transport::FontRequestQueue>,
    event_capacity: Option<std::sync::Arc<dyn Send+Sync>>,
    request_capacity: Option<std::sync::Arc<dyn Send+Sync>>,
    load_capacity: Option<std::sync::Arc<dyn Send+Sync>>,
    loads:Vec<LoadRecord>,
    provider:Option<Rc<dyn NativeFontLoading>>,
    cycles:font_cycles::Cycles,
    requests:VecDeque<font_loading_transport::FontLoadRequest>,
    face_objects:FontRegistry<FaceRecord>,
    sets:FontRegistry<SetRecord>,
    document_set:Option<u64>,
    layout_readiness:font_layout_readiness::LayoutReadiness,
    events:VecDeque<native_platform::publication::Notification>,
    event_retry:Option<native_platform::publication::Notification>,
    event_draining:bool,
}
pub trait NativeFontFace {
    fn decoded_backing(&self)->bool;
    fn identity(&self)->usize;
    fn same_face(&self,other:&dyn NativeFontFace)->bool;
    fn revision(&self)->u64;
    fn descriptor(&self,name:&str)->Result<font_loading_transport::FontDescriptorValue,String>;
    fn family(&self)->Result<font_loading_transport::FontDescriptorValue,String>;
    fn covers(&self,text:&str)->Result<bool,String>;
    fn axes(&self)->Result<font_loading_transport::FontAxes,String>;
    fn binary_source(&self)->bool;
    fn initial_error(&self)->bool;
    fn set_descriptor(&self,name:&str,value:&str)->Result<bool,String>;
    fn request(&self,token:font_loading_transport::FontLoadToken,text:&str)
        ->Result<font_loading_transport::FontLoadRequest,String>;
}
impl Default for State {
    fn default() -> Self {
        Self {
            request_retry:None,
            event_capacity:None,request_capacity:None,load_capacity:None,
            loads: Vec::new(), provider: None, cycles: font_cycles::Cycles::default(),
            requests: VecDeque::new(), face_objects: FontRegistry::default(),
            sets: FontRegistry::default(), document_set: None,
            layout_readiness:font_layout_readiness::LayoutReadiness::default(),
            events:VecDeque::new(),
            event_retry:None,event_draining:false,
        }
    }
}
