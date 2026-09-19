//! Native selected-file snapshots. No content attribute grants file selection.
use super::*;

fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "FileList".into(),
        member: "TypeError: incompatible receiver".into(),
    }
}
fn private<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = v8::String::new(scope, name)?;
    let key = v8::Private::for_api(scope, Some(key));
    object.get_private(scope, key)
}
fn set_private(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("FileList key"))?;
    let key = v8::Private::for_api(scope, Some(key));
    object
        .set_private(scope, key, value)
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(())
}
fn files(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<std::sync::Arc<Vec<oryn_dom::SelectedFile>>, V8BindingError> {
    if object.is_proxy()
        || object.internal_field_count() < 2
        || !object
            .get_internal_field(scope, 0)
            .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
            .is_some_and(|value| value.is_number())
    {
        return Err(invalid());
    }
    let state = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
    let id = stateful_id(scope, object)?;
    if state.execution_retired()
        || !state
            .platform_wrappers
            .borrow()
            .get(&id)
            .and_then(|wrapper| wrapper.to_local(scope))
            .is_some_and(|wrapper| wrapper.strict_equals(object.into()))
    {
        return Err(invalid());
    }
    let record = platform_object(&state, id)?;
    match &*record {
        PlatformObjectRecord::FileList(files) => Ok(files.clone()),
        PlatformObjectRecord::DataTransferFiles(view) => {
            Ok(std::sync::Arc::new(view.store.borrow().files()))
        }
        _ => Err(invalid()),
    }
}
pub(super) fn entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<Option<Vec<v8::Local<'s, v8::Value>>>, V8BindingError> {
    if files(scope, object).is_err() {
        return Ok(None);
    }
    let owner = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
    let id = stateful_id(scope, object)?;
    let transfer = match &*platform_object(&owner, id)? {
        PlatformObjectRecord::DataTransferFiles(store) => Some(store.clone()),
        _ => None,
    };
    if let Some(store) = transfer {
        return data_transfer::files_entries(scope, &owner, object, store).map(Some);
    }
    let array = private(scope, object, "oryn.files.entries")
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(invalid)?;
    let mut values = Vec::new();
    for index in 0..array.length() {
        values.push(
            array
                .get_index(scope, index)
                .ok_or(V8BindingError::JavaScriptException)?,
        );
    }
    Ok(Some(values))
}
pub(super) fn length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let count = files(scope, object)?.len();
    Ok(v8::Integer::new_from_unsigned(scope, count as u32).into())
}
pub(super) fn item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    index: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    files(scope, object)?;
    let index = index
        .uint32_value(scope)
        .ok_or(V8BindingError::JavaScriptException)? as usize;
    let values = entries(scope, object)?.ok_or_else(invalid)?;
    Ok(values
        .get(index)
        .copied()
        .unwrap_or_else(|| v8::null(scope).into()))
}
pub(super) fn input<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    value: Option<v8::Local<v8::Value>>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let node = resolve_receiver(scope, state, object, "HTMLInputElement")?;
    if let Some(value) = value {
        if value.is_null_or_undefined() {
            return Ok(v8::undefined(scope).into());
        }
        let list = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
        let selected = files(scope, list)?;
        if state.dom.borrow().is_file_input(node) {
            state.dom.borrow_mut().set_selected_files(node, selected)?;
            set_private(scope, object, "oryn.input.files", list.into())?;
        }
        return Ok(v8::undefined(scope).into());
    }
    if !state.dom.borrow().is_file_input(node) {
        return Ok(v8::null(scope).into());
    }
    let selected = state.dom.borrow().arena().get(node)?.selected_files.clone();
    if let Some(list) = private(scope, object, "oryn.input.files")
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
        && files(scope, list).is_ok_and(|old| std::sync::Arc::ptr_eq(&old, &selected))
    {
        return Ok(list.into());
    }
    let id = register_platform_object(state, PlatformObjectRecord::FileList(selected.clone()));
    let list = new_platform_object(scope, state, "FileList", id)?;
    let mut values = Vec::new();
    for file in selected.iter() {
        let id = register_platform_object(
            state,
            PlatformObjectRecord::Blob(BlobRecord {
                bytes: file.bytes.to_vec(),
                media_type: file.media_type.clone(),
                file: Some(FileRecord {
                    name: file.name.clone(),
                    last_modified: file.last_modified,
                }),
            }),
        );
        values.push(new_platform_object(scope, state, "File", id)?.into());
    }
    let array = v8::Array::new_with_elements(scope, &values);
    set_private(scope, list, "oryn.files.entries", array.into())?;
    set_private(scope, object, "oryn.input.files", list.into())?;
    Ok(list.into())
}

/// Automation can supply bytes already materialized by the parent, not paths or handles.
/// Author-created bytes have no more filesystem authority than a File constructor.
pub(super) fn select_bytes(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let result = (|| -> Result<(), V8BindingError> {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or_else(invalid)?;
        let node = argument_node(scope, &state, args.get(0))?;
        let encoded = args
            .get(1)
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope);
        if encoded.len() > 24 * 1024 * 1024 {
            return Err(invalid());
        }
        let uploads: Vec<SelectedUploadFile> =
            serde_json::from_str(&encoded).map_err(|_| invalid())?;
        if uploads.len() > 1024 {
            return Err(invalid());
        }
        let selected = uploads
            .into_iter()
            .map(|file| {
                Ok(oryn_dom::SelectedFile {
                    name: file.filename,
                    media_type: normalize_media_type(file.media_type),
                    bytes: BASE64
                        .decode(file.body_base64)
                        .map_err(|_| invalid())?
                        .into(),
                    last_modified: file.last_modified,
                })
            })
            .collect::<Result<Vec<_>, V8BindingError>>()?;
        state
            .dom
            .borrow_mut()
            .set_selected_files(node, std::sync::Arc::new(selected))?;
        Ok(())
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}
fn getter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match entries(scope, args.holder()) {
        Ok(Some(names)) if (index as usize) < names.len() => {
            rv.set(names[index as usize]);
            v8::Intercepted::kYes
        }
        Ok(_) => v8::Intercepted::kNo,
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
    match entries(scope, args.holder()) {
        Ok(Some(names)) if (index as usize) < names.len() => {
            rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
            v8::Intercepted::kYes
        }
        Ok(_) => v8::Intercepted::kNo,
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
    match entries(scope, args.holder()) {
        Ok(Some(names)) => {
            let indices = (0..names.len())
                .map(|i| v8::Integer::new_from_unsigned(scope, i as u32).into())
                .collect::<Vec<_>>();
            rv.set(v8::Array::new_with_elements(scope, &indices));
        }
        Ok(None) => {}
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn reject_write(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    match entries(scope, args.holder()) {
        Ok(None) => return v8::Intercepted::kNo,
        Err(error) => throw_binding_error(scope, &error),
        Ok(Some(_)) => {
            rv.set_bool(false);
            if args.should_throw_on_error() {
                throw_type_error(scope, "FileList indexed properties are read-only");
            }
        }
    }
    v8::Intercepted::kYes
}
fn setter(
    scope: &mut v8::PinScope,
    _: u32,
    _: v8::Local<v8::Value>,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    reject_write(scope, args, rv)
}
fn definer(
    scope: &mut v8::PinScope,
    _: u32,
    _: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    reject_write(scope, args, rv)
}
fn deleter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    match entries(scope, args.holder()) {
        Ok(None) => return v8::Intercepted::kNo,
        Err(error) => throw_binding_error(scope, &error),
        Ok(Some(names)) => {
            let allowed = index as usize >= names.len();
            rv.set_bool(allowed);
            if !allowed && args.should_throw_on_error() {
                throw_type_error(scope, "FileList indexed properties cannot be deleted");
            }
        }
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

/// Restore only deserialized native Files; retain their existing destination
/// wrappers so graph identity is preserved. No filesystem paths are accepted.
pub(super) fn restore_clone<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    entries: &[v8::Local<'s, v8::Value>],
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let mut files = Vec::new();
    for value in entries {
        if value.is_proxy() {
            return Err(invalid());
        }
        let object = v8::Local::<v8::Object>::try_from(*value).map_err(|_| invalid())?;
        if object.internal_field_count() < 2
            || !object
                .get_internal_field(scope, 0)
                .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
                .is_some_and(|value| value.is_number())
        {
            return Err(invalid());
        }
        let owner = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
        if owner.execution_retired() {
            return Err(invalid());
        }
        let id = stateful_id(scope, object)?;
        if !owner
            .platform_wrappers
            .borrow()
            .get(&id)
            .and_then(|wrapper| wrapper.to_local(scope))
            .is_some_and(|wrapper| wrapper.strict_equals(object.into()))
        {
            return Err(invalid());
        }
        let record = platform_object(&owner, id)?;
        let PlatformObjectRecord::Blob(blob) = &*record else {
            return Err(invalid());
        };
        let file = blob.file.as_ref().ok_or_else(invalid)?;
        files.push(oryn_dom::SelectedFile {
            name: file.name.clone(),
            media_type: blob.media_type.clone(),
            bytes: blob.bytes.clone().into(),
            last_modified: file.last_modified,
        });
    }
    let id = register_platform_object(
        state,
        PlatformObjectRecord::FileList(std::sync::Arc::new(files)),
    );
    let list = new_platform_object(scope, state, "FileList", id)?;
    let array = v8::Array::new_with_elements(scope, entries);
    set_private(scope, list, "oryn.files.entries", array.into())?;
    Ok(list)
}
