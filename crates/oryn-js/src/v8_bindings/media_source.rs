//! MediaSourceHandle owns an opaque native endpoint to its originating source.
//! Transfer moves that endpoint; it does not copy a JS marker or source realm.
use super::*;
pub(super) struct Handle {
    pub(super) detached: Cell<bool>,
    pub(super) port: u64,
    pub(super) endpoint: message_ports::Port,
    _wrapper: v8::Global<v8::Object>,
}
pub(super) type Backing = Rc<Handle>;
pub(super) struct SourceRecord {
    pub(super) managed: bool,
    handle: v8::Global<v8::Object>,
    _source_port: v8::Global<v8::Object>,
}
#[derive(Clone)]
pub(super) struct Transfer {
    pub(super) owner: Rc<V8BindingState>,
    pub(super) id: u64,
    pub(super) backing: Backing,
}
impl Transfer {
    pub(super) fn valid(&self) -> bool {
        !self.backing.detached.get()
            && !self.owner.execution_retired()
            && message_ports::transfer_live(&self.backing.endpoint)
    }
    pub(super) fn detach(&self) {
        self.backing.detached.set(true);
    }
    pub(super) fn append_port(
        &self,
        ports: &mut Vec<(Rc<V8BindingState>, u64, message_ports::Port)>,
    ) -> u32 {
        let index = ports.len() as u32;
        ports.push((
            self.owner.clone(),
            self.backing.port,
            self.backing.endpoint.clone(),
        ));
        index
    }
}
pub(super) fn identify(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Option<Transfer> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let owner = binding_state_for_receiver(scope, object)?;
    let id = stateful_id(scope, object).ok()?;
    if !owner
        .platform_wrappers
        .borrow()
        .get(&id)?
        .to_local(scope)?
        .strict_equals(value)
    {
        return None;
    }
    let backing = match owner.platform_objects.borrow().get(&id) {
        Some(PlatformObjectRecord::MediaSourceHandle(backing)) => backing.clone(),
        _ => return None,
    };
    Some(Transfer { owner, id, backing })
}
pub(super) fn receive<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    index: u32,
    ports: &[v8::Global<v8::Object>],
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let port = v8::Local::new(
        scope,
        ports
            .get(index as usize)
            .ok_or(V8BindingError::Allocation("media source endpoint"))?,
    );
    let (_, port_id, endpoint) = message_ports::transferable(scope, port.into())
        .ok_or(V8BindingError::Allocation("media source endpoint"))?;
    let backing = Rc::new(Handle {
        detached: Cell::new(false),
        port: port_id,
        endpoint,
        _wrapper: v8::Global::new(scope, port),
    });
    let id = register_platform_object(state, PlatformObjectRecord::MediaSourceHandle(backing));
    new_platform_object(scope, state, "MediaSourceHandle", id)
}
pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
    managed: bool,
) -> Result<(), V8BindingError> {
    let (source, handle) = message_ports::channel(state)?;
    let source_port = new_platform_object(scope, state, "MessagePort", source)?;
    let handle_port = new_platform_object(scope, state, "MessagePort", handle)?;
    let ports = [v8::Global::new(scope, handle_port)];
    let handle = receive(scope, state, 0, &ports)?;
    let record = SourceRecord {
        managed,
        handle: v8::Global::new(scope, handle),
        _source_port: v8::Global::new(scope, source_port),
    };
    let id = register_platform_object(state, PlatformObjectRecord::MediaSource(Rc::new(record)));
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}
fn getter(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| -> Result<v8::Local<v8::Value>, V8BindingError> {
        let state = binding_state_for_receiver(scope, args.this())
            .ok_or(V8BindingError::Allocation("MediaSource receiver"))?;
        let id = stateful_id(scope, args.this())?;
        let source = match state.platform_objects.borrow().get(&id) {
            Some(PlatformObjectRecord::MediaSource(source)) => source.clone(),
            _ => {
                throw_type_error(scope, "MediaSource receiver required");
                return Err(V8BindingError::JavaScriptException);
            }
        };
        match args.data().int32_value(scope).unwrap_or(-1) {
            0 => Ok(v8::Local::new(scope, &source.handle).into()),
            1 => Ok(v8::String::new(scope, "closed")
                .ok_or(V8BindingError::Allocation("media state"))?
                .into()),
            2 => Ok(v8::Number::new(scope, f64::NAN).into()),
            _ => Ok(v8::Boolean::new(scope, false).into()),
        }
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
pub(super) fn install_members(
    scope: &mut v8::PinScope,
    prototype: v8::Local<v8::Object>,
    worker: bool,
) -> Result<(), V8BindingError> {
    for (index, name) in ["handle", "readyState", "duration", "streaming"]
        .iter()
        .enumerate()
    {
        if index == 0 && !worker {
            continue;
        }
        let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("media member"))?;
        let data = v8::Integer::new(scope, index as i32);
        let getter = v8::Function::builder(getter)
            .data(data.into())
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("media getter"))?;
        let undefined = v8::undefined(scope);
        let mut descriptor =
            v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        prototype
            .define_property(scope, key.into(), &descriptor)
            .ok_or(V8BindingError::Allocation("media descriptor"))?;
    }
    Ok(())
}
