//! Native data-transfer wrappers. Store modes are changed only by the host.
use super::*;
use crate::data_transfer::{Data, Mode, Store};
pub(super) type SharedStore = Rc<RefCell<Store>>;
pub(super) struct View {
    pub store: SharedStore,
    wrappers: RefCell<BTreeMap<u64, v8::Weak<v8::Object>>>,
}
impl View {
    fn new(store: SharedStore) -> Rc<Self> {
        Rc::new(Self {
            store,
            wrappers: RefCell::new(BTreeMap::new()),
        })
    }
}
fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "DataTransfer".into(),
        member: name.into(),
    }
}
fn private_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("transfer key"))?;
    Ok(v8::Private::for_api(scope, Some(key)))
}
fn cached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = private_key(scope, name).ok()?;
    object
        .get_private(scope, key)
        .filter(|value| !value.is_undefined())
}
fn cache(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let key = private_key(scope, name)?;
    object
        .set_private(scope, key, value)
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(())
}
fn text(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<Vec<u16>, V8BindingError> {
    let value = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let mut units = vec![0; value.length()];
    value.write_v2(scope, 0, &mut units, v8::WriteFlags::default());
    Ok(units)
}
fn dom_string(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<String, V8BindingError> {
    let value = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    rust_string_preserving_utf16(scope, value)
}
pub(super) fn store(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    interface: &str,
) -> Result<(Rc<V8BindingState>, SharedStore, Option<u64>), V8BindingError> {
    if object.is_proxy() {
        return Err(error("TypeError: incompatible receiver"));
    }
    let owner = binding_state_for_receiver(scope, object)
        .ok_or_else(|| error("TypeError: incompatible receiver"))?;
    let id = stateful_id(scope, object)?;
    let (store, item) = match &*platform_object(&owner, id)? {
        PlatformObjectRecord::DataTransfer(store) if interface == "DataTransfer" => {
            (store.clone(), None)
        }
        PlatformObjectRecord::DataTransferItems(view) if interface == "DataTransferItemList" => {
            (view.store.clone(), None)
        }
        PlatformObjectRecord::DataTransferItem(store, id) if interface == "DataTransferItem" => {
            (store.clone(), Some(*id))
        }
        _ => return Err(error("TypeError: incompatible receiver")),
    };
    Ok((owner, store, item))
}
pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    let mut store = Store::default();
    store.effect_allowed = "none".into();
    let id = register_platform_object(
        state,
        PlatformObjectRecord::DataTransfer(Rc::new(RefCell::new(store))),
    );
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}
pub(super) fn wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    store: SharedStore,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let id = register_platform_object(state, PlatformObjectRecord::DataTransfer(store));
    new_platform_object(scope, state, "DataTransfer", id)
}
fn item_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    list: v8::Local<v8::Object>,
    store: SharedStore,
    item: u64,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = stateful_id(scope, list)?;
    let view = match &*platform_object(state, id)? {
        PlatformObjectRecord::DataTransferItems(view) => view.clone(),
        _ => return Err(error("TypeError: incompatible item list")),
    };
    view.wrappers
        .borrow_mut()
        .retain(|_, weak| weak.to_local(scope).is_some());
    if let Some(value) = view
        .wrappers
        .borrow()
        .get(&item)
        .and_then(|weak| weak.to_local(scope))
    {
        return Ok(value.into());
    }
    let id = register_platform_object(state, PlatformObjectRecord::DataTransferItem(store, item));
    let value = new_platform_object(scope, state, "DataTransferItem", id)?;
    view.wrappers
        .borrow_mut()
        .insert(item, v8::Weak::new(scope, value));
    Ok(value.into())
}

pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let (owner, shared, id) = store(scope, object, interface)?;
    match (interface, member) {
        ("DataTransfer", "items") => {
            if let Some(value) = cached(scope, object, "oryn.transfer.items") {
                return Ok(value);
            }
            let id = register_platform_object(
                &owner,
                PlatformObjectRecord::DataTransferItems(View::new(shared)),
            );
            let value = new_platform_object(scope, &owner, "DataTransferItemList", id)?.into();
            cache(scope, object, "oryn.transfer.items", value)?;
            Ok(value)
        }
        ("DataTransfer", "types") => {
            let revision = shared.borrow().revision.to_string();
            if let Some(old) = cached(scope, object, "oryn.transfer.types.revision")
                && old
                    .to_string(scope)
                    .is_some_and(|old| old.to_rust_string_lossy(scope) == revision)
                && let Some(value) = cached(scope, object, "oryn.transfer.types")
            {
                return Ok(value);
            }
            let types = shared.borrow().types();
            let values = types
                .iter()
                .map(|value| v8_string(scope, value))
                .collect::<Result<Vec<_>, _>>()?;
            let array = v8::Array::new_with_elements(scope, &values);
            array
                .set_integrity_level(scope, v8::IntegrityLevel::Frozen)
                .ok_or(V8BindingError::JavaScriptException)?;
            cache(scope, object, "oryn.transfer.types", array.into())?;
            let revision = v8_string(scope, &revision)?;
            cache(scope, object, "oryn.transfer.types.revision", revision)?;
            Ok(array.into())
        }
        ("DataTransfer", "files") => {
            if let Some(value) = cached(scope, object, "oryn.transfer.files") {
                return Ok(value);
            }
            let id = register_platform_object(
                &owner,
                PlatformObjectRecord::DataTransferFiles(View::new(shared)),
            );
            let value = new_platform_object(scope, &owner, "FileList", id)?.into();
            cache(scope, object, "oryn.transfer.files", value)?;
            Ok(value)
        }
        ("DataTransfer", "dropEffect") => v8_string(scope, &shared.borrow().drop_effect),
        ("DataTransfer", "effectAllowed") => {
            let store = shared.borrow();
            v8_string(
                scope,
                if store.mode == Mode::Detached {
                    "uninitialized"
                } else {
                    &store.effect_allowed
                },
            )
        }
        ("DataTransferItemList", "length") => {
            Ok(v8::Integer::new_from_unsigned(scope, shared.borrow().items().len() as u32).into())
        }
        ("DataTransferItem", "kind") => {
            let value = shared.borrow();
            let kind = value
                .item(id.unwrap())
                .map(|item| match item.kind {
                    Data::String(_) => "string",
                    Data::File(_) => "file",
                })
                .unwrap_or("");
            v8_string(scope, kind)
        }
        ("DataTransferItem", "type") => {
            let value = shared.borrow();
            v8_string(
                scope,
                value
                    .item(id.unwrap())
                    .map(|item| item.media_type.as_str())
                    .unwrap_or(""),
            )
        }
        _ => Err(error("TypeError: unknown attribute")),
    }
}
pub(super) fn set(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    member: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let (_, shared, _) = store(scope, object, "DataTransfer")?;
    let value = dom_string(scope, value)?;
    match member {
        "dropEffect" => shared.borrow_mut().set_drop_effect(&value),
        "effectAllowed" => shared.borrow_mut().set_effect_allowed(&value),
        _ => return Err(error("TypeError: unknown attribute")),
    }
    Ok(())
}
pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let (owner, shared, id) = store(scope, object, interface)?;
    match (interface, member) {
        ("DataTransfer", "setData") => {
            let format = dom_string(scope, args.get(0))?;
            let value = text(scope, args.get(1))?;
            shared.borrow_mut().set_data(&format, value);
        }
        ("DataTransfer", "getData") => {
            let format = dom_string(scope, args.get(0))?;
            let value = shared.borrow().get_data(&format);
            return Ok(Some(
                v8::String::new_from_two_byte(scope, &value, v8::NewStringType::Normal)
                    .ok_or(V8BindingError::Allocation("transfer data"))?
                    .into(),
            ));
        }
        ("DataTransfer", "clearData") => {
            let format = if args.length() == 0 {
                None
            } else {
                Some(dom_string(scope, args.get(0))?)
            };
            shared.borrow_mut().clear_data(format.as_deref());
        }
        ("DataTransfer", "setDragImage") => {
            let target = v8::Local::<v8::Object>::try_from(args.get(0))
                .map_err(|_| error("TypeError: drag image must be an Element"))?;
            let node = resolve_receiver(scope, &owner, target, "Element")?;
            let x = args
                .get(1)
                .int32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            let y = args
                .get(2)
                .int32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            if shared.borrow().mode == Mode::ReadWrite {
                let snapshot = native_drag_image::capture(&owner, node, (x, y), Vec::new())?;
                let mut store = shared.borrow_mut();
                if store.mode == Mode::ReadWrite {
                    store.drag_image = Some((node, x, y));
                    store.drag_image_snapshot = Some(std::sync::Arc::new(snapshot));
                }
            }
        }
        ("DataTransferItemList", "clear") => shared.borrow_mut().clear(),
        ("DataTransferItemList", "remove") => {
            let index = args
                .get(0)
                .uint32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            shared.borrow_mut().remove(index as usize).map_err(error)?;
        }
        ("DataTransferItemList", "add") => {
            let file = if args.length() == 1 && args.get(0).is_object() {
                let object = v8::Local::<v8::Object>::try_from(args.get(0)).ok();
                object.and_then(|object| {
                    let state = binding_state_for_receiver(scope, object)?;
                    let id = stateful_id(scope, object).ok()?;
                    let record = platform_object(&state, id).ok()?;
                    if let PlatformObjectRecord::Blob(blob) = &*record {
                        blob.file.as_ref().map(|file| oryn_dom::SelectedFile {
                            name: file.name.clone(),
                            last_modified: file.last_modified,
                            media_type: blob.media_type.clone(),
                            bytes: blob.bytes.clone().into(),
                        })
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            let id = if let Some(file) = file {
                shared.borrow_mut().add_file(file)
            } else {
                if args.length() < 2 {
                    return Err(error("TypeError: string add requires type"));
                }
                let value = text(scope, args.get(0))?;
                let kind = dom_string(scope, args.get(1))?;
                shared.borrow_mut().add_string(value, kind).map_err(error)?
            };
            return Ok(Some(if let Some(id) = id {
                item_object(scope, &owner, object, shared, id)?
            } else {
                v8::null(scope).into()
            }));
        }
        ("DataTransferItem", "getAsFile") => {
            let file = {
                let value = shared.borrow();
                if !value.readable() {
                    None
                } else {
                    value.item(id.unwrap()).and_then(|item| {
                        if let Data::File(file) = &item.kind {
                            Some(file.clone())
                        } else {
                            None
                        }
                    })
                }
            };
            if let Some(file) = file {
                let id = register_platform_object(
                    &owner,
                    PlatformObjectRecord::Blob(BlobRecord {
                        bytes: file.bytes.to_vec(),
                        media_type: file.media_type,
                        file: Some(FileRecord {
                            name: file.name,
                            last_modified: file.last_modified,
                        }),
                    }),
                );
                return Ok(Some(new_platform_object(scope, &owner, "File", id)?.into()));
            }
            return Ok(Some(v8::null(scope).into()));
        }
        ("DataTransferItem", "getAsString") => {
            if args.get(0).is_null_or_undefined() {
                return Ok(None);
            }
            let callback = v8::Local::<v8::Function>::try_from(args.get(0))
                .map_err(|_| error("TypeError: callback must be callable"))?;
            let value = {
                let store = shared.borrow();
                if !store.readable() {
                    None
                } else {
                    store.item(id.unwrap()).and_then(|item| {
                        if let Data::String(value) = &item.kind {
                            Some(value.clone())
                        } else {
                            None
                        }
                    })
                }
            };
            if let Some(value) = value {
                owner.deferred_platform_tasks.borrow_mut().push_back(
                    DeferredPlatformTask::DataTransferString {
                        callback: v8::Global::new(scope, callback),
                        value,
                    },
                );
            }
        }
        _ => return Err(error("TypeError: unknown operation")),
    }
    Ok(None)
}

pub(super) fn files_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    _object: v8::Local<v8::Object>,
    view: Rc<View>,
) -> Result<Vec<v8::Local<'s, v8::Value>>, V8BindingError> {
    let entries = {
        let store = view.store.borrow();
        if !store.readable() {
            return Ok(Vec::new());
        }
        store
            .items()
            .iter()
            .filter_map(|item| {
                if let Data::File(file) = &item.kind {
                    Some((item.id, file.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    view.wrappers
        .borrow_mut()
        .retain(|_, weak| weak.to_local(scope).is_some());
    let mut values = Vec::new();
    for (item, file) in entries {
        let old = view
            .wrappers
            .borrow()
            .get(&item)
            .and_then(|weak| weak.to_local(scope));
        let value = if let Some(value) = old {
            value
        } else {
            let id = register_platform_object(
                state,
                PlatformObjectRecord::Blob(BlobRecord {
                    bytes: file.bytes.to_vec(),
                    media_type: file.media_type,
                    file: Some(FileRecord {
                        name: file.name,
                        last_modified: file.last_modified,
                    }),
                }),
            );
            let value = new_platform_object(scope, state, "File", id)?;
            view.wrappers
                .borrow_mut()
                .insert(item, v8::Weak::new(scope, value));
            value
        };
        values.push(value.into());
    }
    Ok(values)
}

fn indexed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    index: u32,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let (owner, shared, _) = store(scope, object, "DataTransferItemList")?;
    let id = shared
        .borrow()
        .items()
        .get(index as usize)
        .map(|item| item.id);
    id.map(|id| item_object(scope, &owner, object, shared, id))
        .transpose()
}
fn getter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match indexed(scope, args.holder(), index) {
        Ok(Some(value)) => {
            rv.set(value);
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}
fn query(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    match indexed(scope, args.holder(), index) {
        Ok(Some(_)) => {
            rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}
fn enumerator(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Array>,
) {
    match store(scope, args.holder(), "DataTransferItemList") {
        Ok((_, store, _)) => {
            let values = (0..store.borrow().items().len())
                .map(|index| v8::Integer::new_from_unsigned(scope, index as u32).into())
                .collect::<Vec<_>>();
            rv.set(v8::Array::new_with_elements(scope, &values));
        }
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn setter(
    scope: &mut v8::PinScope,
    _index: u32,
    _value: v8::Local<v8::Value>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    rv.set_bool(false);
    if args.should_throw_on_error() {
        throw_type_error(scope, "DataTransferItemList indices are read-only");
    }
    v8::Intercepted::kYes
}
fn definer(
    scope: &mut v8::PinScope,
    _index: u32,
    _descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    rv.set_bool(false);
    if args.should_throw_on_error() {
        throw_type_error(scope, "DataTransferItemList indices cannot be defined");
    }
    v8::Intercepted::kYes
}
fn deleter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    match indexed(scope, args.holder(), index) {
        Ok(value) => {
            let allowed = value.is_none();
            rv.set_bool(allowed);
            if !allowed && args.should_throw_on_error() {
                throw_type_error(scope, "DataTransferItemList indices cannot be deleted");
            }
        }
        Err(error) => throw_binding_error(scope, &error),
    }
    v8::Intercepted::kYes
}
pub(super) fn configure(instance: v8::Local<v8::ObjectTemplate>) {
    instance.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(getter)
            .query(query)
            .enumerator(enumerator)
            .setter(setter)
            .definer(definer)
            .deleter(deleter),
    );
}

pub(super) fn attach_event(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    interface: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    if !value.is_null() {
        let data = v8::Local::<v8::Object>::try_from(value)
            .map_err(|_| error("TypeError: transfer must be a DataTransfer"))?;
        store(scope, data, "DataTransfer")?;
    }
    cache(
        scope,
        object,
        &format!("oryn.transfer.event.{interface}"),
        value,
    )
}
pub(super) fn event_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    interface: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    event_id(scope, object)?;
    cached(scope, object, &format!("oryn.transfer.event.{interface}"))
        .ok_or_else(|| error("TypeError: incompatible event receiver"))
}

pub(super) struct TrustedTransfer {
    owner: Rc<V8BindingState>,
    identity: (NodeId, u64, u64, u64),
    source: SharedStore,
    event_store: SharedStore,
    event: retained_dom_event::Event,
    result: Option<bool>,
}
impl TrustedTransfer {
    pub(super) fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        target: NodeId,
        event_type: &str,
        shared: SharedStore,
        init: &crate::data_transfer::EventInit,
    ) -> Result<Self, V8BindingError> {
        let clipboard = matches!(event_type, "copy" | "cut" | "paste");
        if !clipboard
            && !matches!(
                event_type,
                "dragstart" | "drag" | "dragenter" | "dragleave" | "dragover" | "drop" | "dragend"
            )
        {
            return Err(error("TypeError: unsupported native transfer event"));
        }
        if state.execution_retired()
            || state.dom.borrow().arena().node_document(target)? != Some(state.document_root())
        {
            return Err(error("InvalidStateError"));
        }
        let interface = if clipboard {
            "ClipboardEvent"
        } else {
            "DragEvent"
        };
        let cancelable = !matches!(event_type, "dragleave" | "dragend");
        let event = if clipboard {
            create_event_object_with_interface(
                scope, state, interface, event_type, true, cancelable,
            )?
        } else {
            use NativeInputValue::{Bool, Number};
            create_native_input_event(
                scope,
                state,
                NativeInputEventInputs {
                    interface: "DragEvent",
                    event_type,
                    bubbles: true,
                    cancelable,
                    composed: true,
                    detail: 0,
                    related: init.related_target,
                    fields: &[
                        ("clientX", Number(f64::from(init.client_x))),
                        ("clientY", Number(f64::from(init.client_y))),
                        ("screenX", Number(f64::from(init.screen_x))),
                        ("screenY", Number(f64::from(init.screen_y))),
                        ("button", Number(f64::from(init.button))),
                        ("buttons", Number(f64::from(init.buttons))),
                        ("movementX", Number(0.)),
                        ("movementY", Number(0.)),
                        ("altKey", Bool(init.alt_key)),
                        ("ctrlKey", Bool(init.ctrl_key)),
                        ("metaKey", Bool(init.meta_key)),
                        ("shiftKey", Bool(init.shift_key)),
                        ("modifierAltGraph", Bool(false)),
                        ("modifierCapsLock", Bool(false)),
                    ],
                },
            )?
        };
        // Each native event has its own association. Prior event wrappers must not
        // regain read access when a later drop temporarily opens the shared store.
        let event_store = Rc::new(RefCell::new(shared.borrow().clone()));
        let transfer = wrap(scope, state, event_store.clone())?;
        attach_event(scope, event, interface, transfer.into())?;
        let id = event_id(scope, event)?;
        if let Some(record) = state.events.borrow_mut().get_mut(&id) {
            record.is_trusted = true;
            record.composed = true;
        }
        event_store.borrow_mut().set_mode(match event_type {
            "dragstart" | "copy" | "cut" => Mode::ReadWrite,
            "drop" | "paste" => Mode::ReadOnly,
            _ => Mode::Protected,
        });
        let event = retained_dom_event::Event::new(scope, state, target, event_type, event)?;
        Ok(Self {
            owner: state.clone(),
            identity: (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            ),
            source: shared,
            event_store,
            event,
            result: None,
        })
    }
    fn current(&self) -> bool {
        let state = &self.owner;
        !state.execution_retired()
            && !state.execution_tasks_stopped()
            && self.identity
                == (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.document_authority_epoch(),
                )
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<bool>, V8BindingError> {
        if let Some(result) = self.result {
            return Ok(Some(result));
        }
        if !self.current() {
            self.cancel();
            return Ok(Some(false));
        }
        let Some(accepted) = self.event.poll(scope, &self.owner)? else {
            return Ok(None);
        };
        if !self.current() {
            self.cancel();
            return Ok(Some(false));
        }
        // Publish negotiation/payload changes only after the full event finishes.
        // Saved wrappers remain detached when a later event opens its own store.
        let mut updated = self.event_store.borrow().clone();
        updated.set_mode(Mode::Protected);
        *self.source.borrow_mut() = updated;
        self.event_store.borrow_mut().set_mode(Mode::Detached);
        self.result = Some(accepted);
        Ok(Some(accepted))
    }
    pub(super) fn cancel(&mut self) {
        if self.result.is_none() {
            self.event.cancel(&self.owner);
            self.result = Some(false);
        }
        self.event_store.borrow_mut().set_mode(Mode::Detached);
    }
}
impl Drop for TrustedTransfer {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(super) fn trusted_event(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    event_type: &str,
    shared: SharedStore,
    init: &crate::data_transfer::EventInit,
) -> Result<bool, V8BindingError> {
    let mut event = TrustedTransfer::new(scope, state, target, event_type, shared, init)?;
    event.poll(scope)?.ok_or(V8BindingError::Allocation(
        "native transfer event suspended",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_dom::html;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, code)
    }
    #[test]
    fn retained_drag_event_keeps_suffix_payload_and_per_event_access() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            html::parse("<div id=t draggable=true></div>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(222), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        eval(
            scope,
            "globalThis.t=document.getElementById('t');globalThis.trace=[];t.addEventListener('dragstart',e=>{trace.push('first');globalThis.saved=e.dataTransfer;e.dataTransfer.setData('text/plain','a')},{once:true});t.addEventListener('dragstart',e=>{trace.push('second');e.dataTransfer.setData('text/plain',e.dataTransfer.getData('text/plain')+'b')});",
        );
        let source = Rc::new(RefCell::new(Store::default()));
        source.borrow_mut().set_mode(Mode::Protected);
        let mut event = TrustedTransfer::new(
            scope,
            &state,
            target,
            "dragstart",
            source.clone(),
            &crate::data_transfer::EventInit::default(),
        )
        .unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(event.poll(scope).is_err());
        assert!(source.borrow().items().is_empty());
        assert_eq!(event.event_store.borrow().mode, Mode::ReadWrite);
        state.set_execution_suspended(true);
        assert!(event.poll(scope).unwrap().is_none());
        state.set_execution_suspended(false);
        assert_eq!(event.poll(scope).unwrap(), Some(true));
        assert_eq!(event.poll(scope).unwrap(), Some(true));
        let mut data = source.borrow().clone();
        data.set_mode(Mode::ReadOnly);
        assert_eq!(data.get_data("text/plain"), vec![97, 98]);
        assert!(
            eval(
                scope,
                "trace.join(',')==='first,second'&&saved.getData('text/plain')===''"
            )
            .is_true()
        );
        eval(
            scope,
            "t.addEventListener('drop',e=>{if(saved.getData('text/plain')!==''||e.dataTransfer.getData('text/plain')!=='ab')throw Error('event access leaked');trace.push('drop')});",
        );
        let mut drop = TrustedTransfer::new(
            scope,
            &state,
            target,
            "drop",
            source.clone(),
            &crate::data_transfer::EventInit::default(),
        )
        .unwrap();
        assert_eq!(drop.poll(scope).unwrap(), Some(true));
        assert!(eval(scope, "trace.join(',')==='first,second,drop'").is_true());
        let mut abandoned = TrustedTransfer::new(
            scope,
            &state,
            target,
            "dragstart",
            source,
            &crate::data_transfer::EventInit::default(),
        )
        .unwrap();
        abandoned.cancel();
        assert_eq!(abandoned.poll(scope).unwrap(), Some(false));
        assert_eq!(abandoned.event_store.borrow().mode, Mode::Detached);
    }
    #[test]
    fn native_transfer_live_items_files_and_string_semantics() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<input type=file id=file>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let result = eval(
            scope,
            r#"(()=>{
            const dt=new DataTransfer(),items=dt.items,files=dt.files;
            if(items!==dt.items||files!==dt.files)throw Error('stable lists');
            let old=dt.types;if(old!==dt.types||!Object.isFrozen(old))throw Error('stable types');items.clear();dt.clearData();items.remove(42);if(old!==dt.types)throw Error('no-op identity');dt.setData('text','a');if(old===dt.types)throw Error('changed identity');old=dt.types;dt.setData('text','a');if(old===dt.types)throw Error('replacement identity');items.clear();
            dt.setData('TEXT','first');const item=items[0];dt.setData('text/html','html');dt.setData('text/plain','second');
            if(items[0]!==item||dt.getData('Text')!=='second'||dt.types.join('|')!=='text/plain|text/html'||!Object.isFrozen(dt.types))throw Error('ordered replacement');
            let duplicate=false;try{items.add('x','TEXT/PLAIN')}catch(e){duplicate=e.name==='NotSupportedError'}if(!duplicate)throw Error('duplicate type');
            items.add(new File(['abc'],'actual.txt',{type:'text/plain',lastModified:7}));
            if(files.length!==1||files[0].name!=='actual.txt'||items[2].kind!=='file')throw Error('live files');
            document.getElementById('file').files=files;if(document.getElementById('file').files[0].size!==3)throw Error('selected bytes');
            dt.clearData();if(items.length!==1||files.length!==1||dt.types[0]!=='Files')throw Error('clearData file preservation');
            items.clear();if(files.length||item.kind!==''||item.type!==''||item.getAsFile()!==null)throw Error('removed identity');
            dt.setData('url','# comment\r\n https://example.test/ \r\nhttps://other.test/');if(dt.getData('URL')!=='https://example.test/')throw Error('URI list alias');
            dt.setData('text','\ud800x');if(dt.getData('text').charCodeAt(0)!==0xd800)throw Error('UTF16 fidelity');return true;
        })()"#,
        );
        assert!(result.is_true());
    }
    #[test]
    fn author_transfer_constructor_has_none_effect_without_changing_native_seed() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<div></div>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,"(()=>{const d=new DataTransfer();return d.effectAllowed==='none'&&d.dropEffect==='none'&&d.items.length===0&&d.types.length===0})()").is_true());
        assert_eq!(Store::default().effect_allowed, "uninitialized");
    }
    #[test]
    fn synthetic_events_are_untrusted_and_callbacks_are_queued() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<div id=t></div>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"globalThis.values=[];globalThis.dt=new DataTransfer();dt.setData('text','value');dt.items[0].getAsString(v=>values.push(v));const drag=new DragEvent('drop',{dataTransfer:dt,clientX:12});const clip=new ClipboardEvent('paste',{clipboardData:dt});values.length===0 && drag instanceof MouseEvent && drag.dataTransfer===dt && drag.clientX===12 && !drag.isTrusted && clip.clipboardData===dt && !clip.isTrusted && new DragEvent('drag').dataTransfer===null"#).is_true());
        state.drain_platform_completions(scope, 8).unwrap();
        assert!(eval(scope, "values.join('|')==='value'").is_true());
    }
    #[test]
    fn native_drag_protects_store_after_callbacks_and_rejects_retired_realm() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<div id=t></div>").into_runtime().unwrap(),
        ));
        let target = {
            let dom = dom.borrow();
            dom.get_element_by_id(dom.document(), "t").unwrap().unwrap()
        };
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            r#"globalThis.seen=[];t.addEventListener('dragstart',e=>{seen.push(e.isTrusted);e.dataTransfer.setData('text','payload');globalThis.saved=e.dataTransfer;e.preventDefault()});t.addEventListener('drop',e=>{if(saved.getData('text')!==''||saved.types.length)throw Error('old event reopened');seen.push(e.dataTransfer.getData('text'));e.dataTransfer.setData('text','forbidden')});"#,
        );
        let shared = Rc::new(RefCell::new(Store::new(Mode::Protected)));
        assert!(
            !trusted_event(
                scope,
                &state,
                target,
                "dragstart",
                shared.clone(),
                &Default::default()
            )
            .unwrap()
        );
        assert_eq!(shared.borrow().mode, Mode::Protected);
        assert!(
            eval(
                scope,
                "saved.getData('text')==='' && saved.types.length===0"
            )
            .is_true()
        );
        trusted_event(
            scope,
            &state,
            target,
            "drop",
            shared.clone(),
            &Default::default(),
        )
        .unwrap();
        assert!(eval(scope, "seen.join('|')==='true|payload'").is_true());
        shared.borrow_mut().set_mode(Mode::ReadOnly);
        assert_eq!(
            shared.borrow().get_data("text"),
            "payload".encode_utf16().collect::<Vec<_>>()
        );
        state.retire_execution();
        assert!(trusted_event(scope, &state, target, "drop", shared, &Default::default()).is_err());
    }
}

#[cfg(test)]
mod overload_arity_tests {
    use super::*;
    #[test]
    fn file_overload_is_admitted_but_missing_string_type_and_empty_calls_still_reject() {
        super::super::tests::initialize_v8();
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
        assert!(super::super::test_eval::eval(scope,r#"(()=>{
            const items=new DataTransfer().items,file=new File(['bytes'],'f.txt');
            const admitted=items.add(file).getAsFile();if(items.add.length!==1||admitted.name!==file.name||admitted.size!==file.size)throw Error('one-argument File overload');
            for(const args of [[],['text']]){
                let rejected=false;try{items.add(...args)}catch(e){rejected=e instanceof TypeError}
                if(!rejected)throw Error('invalid overload accepted');
            }
            const item=items.add('text','text/plain');
            return item.kind==='string'&&items.length===2;
        })()"#).is_true());
    }
}
