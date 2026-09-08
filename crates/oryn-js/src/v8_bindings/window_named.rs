//! Live named access on the Window named-properties object.
use super::*;

#[derive(Default)]
pub(super) struct TargetName {
    value: String,
    source: Option<(std::rc::Weak<RefCell<DomRuntime>>, NodeId)>,
    revision: u64,
}

impl TargetName {
    pub(super) fn for_frame(dom: &Rc<RefCell<DomRuntime>>, frame: NodeId) -> Self {
        let mut name = Self {
            source: Some((Rc::downgrade(dom), frame)),
            revision: u64::MAX,
            ..Self::default()
        };
        name.refresh();
        name
    }
    fn refresh(&mut self) {
        let Some((source, frame)) = &self.source else {
            return;
        };
        let Some(dom) = source.upgrade() else { return };
        let dom = dom.borrow();
        let Ok(revision) = dom.name_attribute_revision(*frame) else {
            return;
        };
        if revision == self.revision {
            return;
        }
        self.revision = revision;
        let unqualified = dom.arena().existing_attribute_node(*frame, "name").is_none_or(|attribute| {
            dom.arena().get(attribute).is_ok_and(|record| matches!(&record.kind, NodeKind::Attribute { namespace, .. } if namespace.is_none()))
        });
        self.value = if unqualified {
            dom.attribute(*frame, "name")
                .ok()
                .flatten()
                .unwrap_or("")
                .to_owned()
        } else {
            String::new()
        };
    }
    pub(super) fn get(&mut self) -> String {
        self.refresh();
        self.value.clone()
    }
    pub(super) fn set(&mut self, value: String) {
        self.refresh();
        self.value = value;
    }
}

fn resolve<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<v8::Object>,
    key: v8::Local<v8::Name>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let Ok(name) = v8::Local::<v8::String>::try_from(key) else {
        return Ok(None);
    };
    let mut ancestor = holder.get_prototype(scope);
    while let Some(value) = ancestor {
        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
            break;
        };
        if object.has_real_named_property(scope, key) == Some(true) {
            return Ok(None);
        }
        ancestor = object.get_prototype(scope);
    }
    let Some(context) = holder.get_creation_context(scope) else {
        return Ok(None);
    };
    let Some(state) = context.get_slot::<V8BindingState>() else {
        return Ok(None);
    };
    let name = name.to_rust_string_lossy(scope);
    let root = state.document_root();
    // A child navigable's target name takes precedence over element matches.
    for node in traversal_nodes(&state.dom.borrow(), root)?
        .into_iter()
        .skip(1)
    {
        let matches = state
            .frame_target_names
            .borrow()
            .get(&node)
            .is_some_and(|target| !name.is_empty() && target.borrow_mut().get() == name);
        if matches && let Some(window) = state.frame_window(node) {
            return Ok(Some(v8::Local::new(scope, &window).into()));
        }
    }
    let nodes = node_lists::window_named_nodes(&state.dom.borrow(), root, &name)?;
    // Returned platform objects belong to the Window's realm even when its
    // named-properties object is accessed by another same-origin realm.
    let scope = &mut v8::ContextScope::new(scope, context);
    match nodes.as_slice() {
        [] => Ok(None),
        [node] => Ok(Some(wrap_node(scope, &state, *node)?.into())),
        _ => node_lists::elements(
            scope,
            &state,
            root,
            node_lists::ElementFilter::WindowName { name },
        )
        .map(Some),
    }
}

fn getter(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match resolve(scope, args.holder(), key) {
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
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    match resolve(scope, args.holder(), key) {
        Ok(Some(_)) => {
            rv.set_int32(v8::PropertyAttribute::DONT_ENUM.as_u32() as i32);
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}
fn descriptor(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match resolve(scope, args.holder(), key) {
        Ok(Some(value)) => {
            let descriptor = v8::Object::new(scope);
            for (name, value) in [
                ("value", value),
                ("writable", v8::Boolean::new(scope, true).into()),
                ("enumerable", v8::Boolean::new(scope, false).into()),
                ("configurable", v8::Boolean::new(scope, true).into()),
            ] {
                let key = v8::String::new(scope, name).unwrap();
                descriptor.create_data_property(scope, key.into(), value);
            }
            rv.set(descriptor.into());
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}
fn definer(
    scope: &mut v8::PinScope,
    _: v8::Local<v8::Name>,
    _: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    rv.set_bool(false);
    if args.should_throw_on_error() {
        throw_type_error(
            scope,
            "Window named-properties object rejects property definitions",
        );
    }
    v8::Intercepted::kYes
}
fn deleter(
    scope: &mut v8::PinScope,
    _: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(false);
    if args.should_throw_on_error() {
        throw_type_error(scope, "Window named-properties object rejects deletion");
    }
    v8::Intercepted::kYes
}

pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let global = context.global(scope);
    let marker_name = v8::String::new(scope, "oryn.window.named.installed").unwrap();
    let marker = v8::Private::for_api(scope, Some(marker_name));
    if global.has_private(scope, marker) == Some(true) {
        return Ok(());
    }
    let prototype_key = v8::String::new(scope, "prototype").unwrap();
    let window_key = v8::String::new(scope, "Window").unwrap();
    let constructor = global
        .get(scope, window_key.into())
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
        .ok_or(V8BindingError::Allocation("Window constructor"))?;
    let window_prototype = constructor
        .get(scope, prototype_key.into())
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
        .ok_or(V8BindingError::Allocation("Window prototype"))?;
    // Global-interface attributes are own properties of the global object.
    let name_key = v8::String::new(scope, "name").unwrap();
    let attribute = window_prototype
        .get_own_property_descriptor(scope, name_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("Window name descriptor"))?;
    let get_key = v8::String::new(scope, "get").unwrap();
    let set_key = v8::String::new(scope, "set").unwrap();
    let name_getter = attribute
        .get(scope, get_key.into())
        .ok_or(V8BindingError::Allocation("Window name getter"))?;
    let name_setter = attribute
        .get(scope, set_key.into())
        .ok_or(V8BindingError::Allocation("Window name setter"))?;
    let mut name_descriptor = v8::PropertyDescriptor::new_from_get_set(name_getter, name_setter);
    name_descriptor.set_enumerable(true);
    name_descriptor.set_configurable(true);
    if global.define_property(scope, name_key.into(), &name_descriptor) != Some(true)
        || window_prototype.delete(scope, name_key.into()) != Some(true)
    {
        return Err(V8BindingError::Allocation("Window global name attribute"));
    }
    let event_prototype = window_prototype
        .get_prototype(scope)
        .ok_or(V8BindingError::Allocation("Window parent prototype"))?;
    let template = v8::FunctionTemplate::new(
        scope,
        |_: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue| {},
    );
    template.set_class_name(v8::String::new(scope, "WindowProperties").unwrap());
    let instance = template.instance_template(scope);
    instance.set_immutable_proto();
    instance.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(getter)
            .query(query)
            .descriptor(descriptor)
            .definer(definer)
            .deleter(deleter),
    );
    let private_constructor = template
        .get_function(scope)
        .ok_or(V8BindingError::Allocation("Window properties constructor"))?;
    private_constructor.set(scope, prototype_key.into(), event_prototype);
    let named = private_constructor
        .new_instance(scope, &[])
        .ok_or(V8BindingError::Allocation("Window named-properties object"))?;
    if window_prototype.set_prototype(scope, named.into()) != Some(true)
        || global.set_prototype(scope, window_prototype.into()) != Some(true)
    {
        return Err(V8BindingError::Allocation("Window prototype chain"));
    }
    global.set_private(scope, marker, v8::Boolean::new(scope, true).into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_named_target_tracks_attribute_writes_after_child_renaming() {
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let frame = dom
            .borrow_mut()
            .create_element_with_local_name("http://www.w3.org/1999/xhtml", "iframe");
        dom.borrow_mut()
            .set_attribute(frame, "name", "authored")
            .unwrap();
        let mut target = TargetName::for_frame(&dom, frame);
        assert_eq!(target.get(), "authored");
        target.set("child".into());
        assert_eq!(target.get(), "child");
        dom.borrow_mut()
            .set_attribute(frame, "name", "authored")
            .unwrap();
        assert_eq!(target.get(), "authored");
        target.set("child".into());
        dom.borrow_mut().remove_attribute(frame, "name").unwrap();
        assert_eq!(target.get(), "");
        target.set("child".into());
        dom.borrow_mut()
            .set_attribute_ns(frame, Some("urn:test".into()), "name", "foreign")
            .unwrap();
        assert_eq!(target.get(), "child");
        dom.borrow_mut()
            .set_attribute_ns(frame, None, "name", "plain")
            .unwrap();
        assert_eq!(target.get(), "plain");
        dom.borrow_mut()
            .set_attribute_ns(frame, Some("urn:test".into()), "name", "foreign")
            .unwrap();
        assert_eq!(target.get(), "");
    }
}
