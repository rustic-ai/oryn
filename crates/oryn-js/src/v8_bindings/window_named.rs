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
        if dom
            .borrow()
            .embedded_content_kind(frame)
            .is_some_and(|kind| kind.selects_response_representation())
        {
            name.source = None;
        }
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
        self.value = dom
            .attribute_in_null_namespace(*frame, "name")
            .ok()
            .flatten()
            .unwrap_or("")
            .to_owned();
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

pub(super) fn children(
    state: &V8BindingState,
) -> Result<Vec<v8::Global<v8::Object>>, V8BindingError> {
    Ok(traversal_nodes(&state.dom.borrow(), state.document_root())?
        .into_iter()
        .skip(1)
        .filter_map(|node| state.frame_window(node))
        .collect())
}

/// Resolve only child navigable target names, never author properties or DOM
/// element IDs. Shared by same-origin named access and cross-origin projection.
pub(super) fn named_child(
    state: &V8BindingState,
    name: &str,
) -> Result<Option<v8::Global<v8::Object>>, V8BindingError> {
    if name.is_empty() {
        return Ok(None);
    }
    for node in traversal_nodes(&state.dom.borrow(), state.document_root())?
        .into_iter()
        .skip(1)
    {
        let matches = state
            .frame_target_names
            .borrow()
            .get(&node)
            .is_some_and(|target| target.borrow_mut().get() == name);
        if matches && let Some(window) = state.frame_window(node) {
            return Ok(Some(window));
        }
    }
    Ok(None)
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
    let name = rust_string_preserving_utf16(scope, name)?;
    let root = state.document_root();
    // A child navigable's target name takes precedence over element matches.
    if let Some(window) = named_child(&state, &name)? {
        return Ok(Some(v8::Local::new(scope, &window).into()));
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

/// Prepublish the native named object before Window's immutable prototype is
/// materialized. The carrier constructor remains private to this realm.
pub(super) fn carrier<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
    parent: v8::Local<'s, v8::FunctionTemplate>,
) -> Result<v8::Local<'s, v8::FunctionTemplate>, V8BindingError> {
    let prototype_key = v8::String::new(scope, "prototype").unwrap();
    let parent_constructor = parent.get_function(scope)
        .ok_or(V8BindingError::Allocation("Window parent constructor"))?;
    let event_prototype = parent_constructor
        .get(scope, prototype_key.into())
        .ok_or(V8BindingError::Allocation("Window parent prototype"))?;
    let template = v8::FunctionTemplate::new(
        scope,
        |_: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue| {},
    );
    template.inherit(parent);
    template.set_class_name(v8::String::new(scope, "WindowProperties").unwrap());
    let instance = template.instance_template(scope);
    instance.set_immutable_proto();
    instance.set_with_attr(
        v8::Symbol::get_to_string_tag(scope).into(),
        v8::String::new(scope, "WindowProperties")
            .ok_or(V8BindingError::Allocation("Window properties toStringTag"))?
            .into(),
        v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
    );
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
    if private_constructor.set(scope, prototype_key.into(), event_prototype) != Some(true) {
        return Err(V8BindingError::Allocation("Window named parent"));
    }
    let named = private_constructor
        .new_instance(scope, &[])
        .ok_or(V8BindingError::Allocation("Window named-properties object"))?;
    if private_constructor.set(scope, prototype_key.into(), named.into()) != Some(true) {
        return Err(V8BindingError::Allocation("Window named carrier"));
    }
    let key = v8::String::new(scope, "oryn.window.named.prepublished")
        .ok_or(V8BindingError::Allocation("Window named identity"))?;
    let key = v8::Private::for_api(scope, Some(key));
    if context.global(scope).set_private(scope, key, named.into()) != Some(true) {
        return Err(V8BindingError::Allocation("Window named identity"));
    }
    Ok(template)
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
    let constructor = global.get(scope, window_key.into())
        .and_then(|value| value.try_cast::<v8::Object>().ok())
        .ok_or(V8BindingError::Allocation("Window constructor"))?;
    let window_prototype = constructor.get(scope, prototype_key.into())
        .and_then(|value| value.try_cast::<v8::Object>().ok())
        .ok_or(V8BindingError::Allocation("Window prototype"))?;
    let key = v8::String::new(scope, "oryn.window.named.prepublished").unwrap();
    let key = v8::Private::for_api(scope, Some(key));
    let named = global.get_private(scope, key)
        .filter(|value| value.is_object())
        .ok_or(V8BindingError::Allocation("Window named identity"))?;
    // The immutable parent was established during template construction.
    if !window_prototype.get_prototype(scope).is_some_and(|value| value.strict_equals(named))
        || !crate::v8_context_bridge::set_global_object_prototype(
            scope,
            context,
            window_prototype.into(),
        )
    {
        return Err(V8BindingError::Allocation("Window prototype chain"));
    }
    global.set_private(scope, marker, v8::Boolean::new(scope, true).into());
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_prototype_is_immutable_extensible_and_preserves_named_chain() {
        super::super::tests::check_global_event_prototype_script(r#"(()=>{
            const check=(value,message)=>{if(!value)throw Error(message)};
            const prototype=Window.prototype, named=Object.getPrototypeOf(prototype);
            check(Object.getPrototypeOf(named)===EventTarget.prototype,'EventTarget parent');
            check(Object.getPrototypeOf(Window)===EventTarget,'constructor parent');
            check(prototype.constructor===Window&&named.constructor===EventTarget,'constructor identities');
            check(Object.isExtensible(prototype),'prototype remains extensible');
            check(Reflect.setPrototypeOf(prototype,named),'same parent accepted');
            check(!Reflect.setPrototypeOf(prototype,{}),'different parent rejected');
            for(const mutate of [()=>Object.setPrototypeOf(prototype,null),()=>{prototype.__proto__={}}]){
                try{mutate();throw Error('accepted mutation')}
                catch(error){check(error instanceof TypeError,'prototype mutation TypeError')}
            }
            prototype.authorExtension=123;
            check(window.authorExtension===123,'author extension remains allowed');
            delete prototype.authorExtension;
            const element=document.createElement('div');element.id='immutable_named_control';
            document.body.append(element);check(window.immutable_named_control===element,'live named lookup');
            element.remove();check(!('immutable_named_control' in window),'live named removal');
            check(Object.getPrototypeOf(prototype)===named,'parent retained');
            return true;
        })()"#);
    }
    #[test]
    fn window_named_properties_have_own_webidl_string_tag() {
        super::super::tests::check_global_event_prototype_script(
            r#"(()=>{
                const check=(value,message)=>{if(!value)throw Error(message)};
                const named=Object.getPrototypeOf(Window.prototype);
                const descriptor=Object.getOwnPropertyDescriptor(named,Symbol.toStringTag);
                check(descriptor&&descriptor.value==='WindowProperties','own tag');
                check(!descriptor.writable&&!descriptor.enumerable&&descriptor.configurable,'tag flags');
                check(Object.prototype.toString.call(named)==='[object WindowProperties]','named brand');
                check(Object.getPrototypeOf(named)===EventTarget.prototype,'parent retained');
                check(Reflect.setPrototypeOf(named,EventTarget.prototype),'same immutable parent accepted');
                check(!Reflect.setPrototypeOf(named,{}),'different immutable parent rejected');
                const element=document.createElement('div');element.id='named_tag_regression';
                document.body.append(element);
                check(window.named_tag_regression===element,'named lookup retained');element.remove();
                return true;
            })()"#,
        );
    }
    use super::*;

    #[test]
    fn window_names_and_named_element_keys_preserve_domstring_units() {
        super::super::tests::check_global_event_prototype_script(
            r#"(()=>{
            const check=(value,message)=>{if(!value)throw Error(message)};
            const prefix=String.fromCodePoint(0xf0000)+'oryn-wtf16:';
            for(const name of ['a\0b','a'+String.fromCharCode(0xd800),'a'+String.fromCharCode(0xdfff),prefix+'[55296]','a'+String.fromCodePoint(0x1f600)]){
                window.name=name;check(window.name===name,'Window.name exact units');
                const element=document.createElement('div');element.id=name;document.body.append(element);
                check(window[name]===element,'named element exact units');element.remove();
                const form=document.createElement('form'),button=document.createElement('button'),base=document.createElement('base'),anchor=document.createElement('a');
                for(const [element,property,attribute] of [[form,'target','target'],[button,'formTarget','formtarget'],[base,'target','target'],[anchor,'target','target']]){
                    element[property]=name;check(element[property]===name&&element.getAttribute(attribute)===name,'target reflection '+property);
                    const other=document.implementation.createHTMLDocument('');let conversions=0;
                    element[property]={toString(){conversions++;other.adoptNode(element);return name}};
                    check(conversions===1&&element.ownerDocument===other&&element[property]===name,'target conversion adoption '+property);
                }
            }
            window.name='before';let calls=0;window.name={toString(){calls++;return 'after'+String.fromCharCode(0xd800)}};
            check(calls===1&&window.name==='after'+String.fromCharCode(0xd800),'name coercion once');return true;
        })()"#,
        );
    }

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
        assert_eq!(target.get(), "plain");
    }
}
