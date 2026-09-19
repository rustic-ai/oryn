//! DOM listener registration resolves its native receiver after WebIDL
//! conversion and keeps AbortSignal authority in the signal's own realm.
use super::*;
fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "EventTarget".into(),
        member: "TypeError".into(),
    }
}
fn member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("listener option"))?;
    object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)
}
fn signal(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<ForeignSignalLink, V8BindingError> {
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
    if object.is_proxy() {
        return Err(invalid());
    }
    let owner = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
    if object.internal_field_count() < 2
        || !object
            .get_internal_field(scope, 0)
            .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
            .is_some_and(|value| value.is_number())
    {
        return Err(invalid());
    }
    let id = stateful_id(scope, object)?;
    if !matches!(
        &*platform_object(&owner, id)?,
        PlatformObjectRecord::AbortSignal(_)
    ) || !owner
        .platform_wrappers
        .borrow()
        .get(&id)
        .and_then(|weak| weak.to_local(scope))
        .is_some_and(|actual| actual.strict_equals(object.into()))
    {
        return Err(invalid());
    }
    Ok(ForeignSignalLink {
        owner: Rc::downgrade(&owner),
        id,
    })
}
pub(super) fn aborted(signal: &ForeignSignalLink) -> bool {
    signal.owner.upgrade().is_some_and(|owner| {
        owner.platform_objects.borrow().get(&signal.id).is_some_and(
            |record| matches!(record,PlatformObjectRecord::AbortSignal(signal)if signal.aborted),
        )
    })
}
pub(super) fn options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    add: bool,
) -> Result<(bool, bool, Option<bool>, Option<ForeignSignalLink>), V8BindingError> {
    if value.is_null_or_undefined() {
        return Ok((false, false, None, None));
    }
    if !value.is_object() {
        return Ok((value.boolean_value(scope), false, None, None));
    }
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
    let capture = member(scope, object, "capture")?.boolean_value(scope);
    if !add {
        return Ok((capture, false, None, None));
    }
    let once = member(scope, object, "once")?.boolean_value(scope);
    let value = member(scope, object, "passive")?;
    let passive = (!value.is_undefined()).then(|| value.boolean_value(scope));
    let value = member(scope, object, "signal")?;
    let signal = if value.is_undefined() {
        None
    } else {
        Some(signal(scope, value)?)
    };
    Ok((capture, once, passive, signal))
}
pub(super) fn change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    add: bool,
) -> Result<(), V8BindingError> {
    let event_type = required_string(scope, args, 0)?;
    let callback = args.get(1);
    if !callback.is_null_or_undefined() && !callback.is_object() {
        return Err(invalid());
    }
    let (capture, once, passive, signal) = options(scope, args.get(2), add)?;
    let (state, node) = node_identity::argument(scope, args.this().into())?.ok_or_else(invalid)?;
    if callback.is_null_or_undefined() || signal.as_ref().is_some_and(aborted) {
        return Ok(());
    }
    let passive = passive.unwrap_or_else(|| {
        if !matches!(
            event_type.as_str(),
            "touchstart" | "touchmove" | "wheel" | "mousewheel"
        ) {
            return false;
        }
        let dom = state.dom.borrow();
        let document = dom.arena().node_document(node).ok().flatten();
        matches!(
            dom.arena().get(node).map(|record| &record.kind),
            Ok(NodeKind::Document)
        ) || document.is_some_and(|document| {
            dom.first_element_child(document).ok().flatten() == Some(node)
                || dom.is_element(node, "http://www.w3.org/1999/xhtml", "body")
                    && dom.parent(node).ok().flatten().is_some_and(|parent| {
                        dom.first_element_child(document).ok().flatten() == Some(parent)
                    })
        })
    });
    let mut map = state.listeners.borrow_mut();
    if add {
        let entries = map.entry((node, event_type.clone())).or_default();
        if entries.iter().any(|entry| {
            entry.capture == capture
                && v8::Local::new(scope, &entry.callback).strict_equals(callback)
        }) {
            return Ok(());
        }
        let id = state.next_event_listener_id.get();
        state.next_event_listener_id.set(
            id.checked_add(1)
                .ok_or(V8BindingError::Allocation("listener identity"))?,
        );
        entries.push(EventListener {
            registration_id: id,
            callback: v8::Global::new(scope, callback),
            capture,
            once,
            passive,
            signal,
        });
    } else if let Some(entries) = map.get_mut(&(node, event_type.clone())) {
        entries.retain(|entry| {
            entry.capture != capture
                || !v8::Local::new(scope, &entry.callback).strict_equals(callback)
        });
    }
    let present = map
        .get(&(node, event_type.clone()))
        .is_some_and(|entries| !entries.is_empty());
    drop(map);
    state
        .dom
        .borrow_mut()
        .set_event_listener_presence(node, &event_type, present)?;
    Ok(())
}
pub(super) fn abort(
    scope: &mut v8::PinScope,
    signal_owner: &V8BindingState,
    id: u64,
) -> Result<(), V8BindingError> {
    let states = scope
        .get_slot::<BindingStateRegistry>()
        .map(|registry| {
            registry
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut failure = None;
    for state in states {
        let changed = {
            let mut map = state.listeners.borrow_mut();
            let mut changed = Vec::new();
            for ((node, name), entries) in map.iter_mut() {
                let before = entries.len();
                entries.retain(|entry| {
                    !entry.signal.as_ref().is_some_and(|signal| {
                        signal.id == id
                            && signal
                                .owner
                                .upgrade()
                                .is_some_and(|owner| owner.instance_id == signal_owner.instance_id)
                    })
                });
                if entries.len() != before {
                    changed.push((*node, name.clone(), !entries.is_empty()));
                }
            }
            changed
        };
        let platform_changed = {
            let mut map = state.platform_listeners.borrow_mut();
            let mut changed = false;
            for entries in map.values_mut() {
                let before = entries.len();
                entries.retain(|entry| {
                    !entry.signal.as_ref().is_some_and(|signal| {
                        signal.id == id
                            && signal
                                .owner
                                .upgrade()
                                .is_some_and(|owner| owner.instance_id == signal_owner.instance_id)
                    })
                });
                changed |= entries.len() != before;
            }
            changed
        };
        if platform_changed {
            if let Err(error) = platform_listener_graph::refresh_all(scope, &state) {
                failure.get_or_insert(error);
            }
        }
        for (node, name, present) in changed {
            if let Err(error) = state
                .dom
                .borrow_mut()
                .set_event_listener_presence(node, &name, present)
            {
                failure.get_or_insert(V8BindingError::from(error));
            }
        }
    }
    failure.map_or(Ok(()), Err)
}
pub(super) fn active(
    scope: &mut v8::PinScope,
    receiver: v8::Local<v8::Object>,
    name: &str,
    listener: &EventListener,
) -> Result<bool, V8BindingError> {
    let (state, node) = node_identity::argument(scope, receiver.into())?.ok_or_else(invalid)?;
    if listener.signal.as_ref().is_some_and(aborted) {
        return Ok(false);
    }
    let present = state
        .listeners
        .borrow()
        .get(&(node, name.into()))
        .is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.registration_id == listener.registration_id)
        });
    if present && listener.once {
        let remains = {
            let mut map = state.listeners.borrow_mut();
            let entries = map.get_mut(&(node, name.into())).unwrap();
            entries.retain(|entry| entry.registration_id != listener.registration_id);
            !entries.is_empty()
        };
        state
            .dom
            .borrow_mut()
            .set_event_listener_presence(node, name, remains)?;
    }
    Ok(present)
}
pub(super) fn invoke(
    scope: &mut v8::PinScope,
    receiver: v8::Local<v8::Value>,
    event: v8::Local<v8::Object>,
    id: u64,
    phase: u16,
    capture: bool,
    property: bool,
) -> Result<(), V8BindingError> {
    let (owner, node) = node_identity::argument(scope, receiver)?.ok_or_else(invalid)?;
    invoke_node_event_listeners(
        scope,
        &owner,
        node,
        event,
        id,
        phase,
        Some(capture),
        property,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn options_abort_and_dispatch_follow_native_targets_across_adoption() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let _first_state = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<main></main>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(130), 1).unwrap()
        };
        let second = v8::Context::new(scope, Default::default());
        let (_second_state, document, controller) = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<main></main>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, second, dom, RealmId(131), 1).unwrap();
            let document = wrap_node(scope, &state, state.document_root()).unwrap();
            let document = v8::Global::new(scope, document);
            let code = v8::String::new(scope, "new AbortController()").unwrap();
            let controller = v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap();
            (state, document, v8::Global::new(scope, controller))
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        for (name, value) in [
            ("foreignDocument", v8::Local::new(scope, &document).into()),
            ("foreignController", v8::Local::new(scope, &controller)),
        ] {
            let key = v8::String::new(scope, name).unwrap();
            first.global(scope).set(scope, key.into(), value);
        }
        let code=v8::String::new(scope,r#"
            const check=(value,message)=>{if(!value)throw Error(message)};
            const target=document.createElement('button');document.body.append(target);let calls=0;
            const callback=()=>calls++,controller=new AbortController();
            const order=[];target.addEventListener({toString(){order.push('type');return 'probe'}},callback,{get capture(){order.push('capture');return false},get once(){order.push('once');return false},get passive(){order.push('passive');return false},get signal(){order.push('signal');return controller.signal}});
            check(order.join(',')==='type,capture,once,passive,signal','dictionary conversion order');target.dispatchEvent(new Event('probe'));check(calls===1,'initial registration');
            controller.abort();target.dispatchEvent(new Event('probe'));check(calls===1,'abort removes before dispatch');
            let nullOptions=0;target.addEventListener('none',null,{get capture(){nullOptions++;return false},get signal(){nullOptions++;return controller.signal}});check(nullOptions===2,'null callback still converts options');
            let invalid=false;try{target.addEventListener('none',null,{signal:null})}catch(e){invalid=e instanceof TypeError}check(invalid,'nonnullable signal brand');
            const sentinel={};try{target.addEventListener('bad',callback,{get once(){throw sentinel}})}catch(e){check(e===sentinel,'original options exception')}target.dispatchEvent(new Event('bad'));check(calls===1,'no partial registration');
            const a=new AbortController(),b=new AbortController();target.addEventListener('duplicate',callback,{signal:a.signal});target.addEventListener('duplicate',callback,{once:true,signal:b.signal});b.abort();target.dispatchEvent(new Event('duplicate'));target.dispatchEvent(new Event('duplicate'));check(calls===3,'duplicate keeps first options');a.abort();
            const platform=new EventTarget();let platformCalls=0;
            platform.addEventListener('foreign',()=>platformCalls++,{signal:foreignController.signal});platform.dispatchEvent(new Event('foreign'));check(platformCalls===1,'foreign platform signal registration');
            const stopPlatform=()=>platform.dispatchEvent(new Event('foreign'));foreignController.signal.addEventListener('abort',stopPlatform,{once:true});
            let platformInvalid=false;try{platform.addEventListener('none',null,{signal:null})}catch(e){platformInvalid=e instanceof TypeError}check(platformInvalid,'platform null callback converts nonnullable signal');
            const platformAbort=new AbortController();platform.addEventListener('nested',()=>platformAbort.abort(),{once:true});platform.addEventListener('nested',()=>platformCalls++,{signal:platformAbort.signal});platform.dispatchEvent(new Event('nested'));check(platformCalls===1,'platform abort skips snapshot callback');
            target.addEventListener('foreign',callback,{signal:foreignController.signal});foreignDocument.adoptNode(target);foreignDocument.body.append(target);foreignController.abort();target.dispatchEvent(new Event('foreign'));check(calls===3,'foreign signal follows migrated target');check(platformCalls===1,'platform foreign abort removes before public abort event');
            const aborted=AbortSignal.abort();target.addEventListener('aborted',callback,{signal:aborted});target.dispatchEvent(new Event('aborted'));check(calls===3,'already aborted skipped');
            const reentrant=document.createElement('div');document.body.append(reentrant);const events=[];
            reentrant.addEventListener('move',()=>{events.push('first');foreignDocument.adoptNode(reentrant);foreignDocument.body.append(reentrant);});
            reentrant.addEventListener('move',()=>events.push('second'),{once:true});
            document.body.addEventListener('move',()=>events.push('original-parent'));
            reentrant.dispatchEvent(new Event('move',{bubbles:true}));check(events.join(',')==='first,second,original-parent','captured event path with moved listener map');
            events.length=0;reentrant.dispatchEvent(new Event('move'));check(events.join(',')==='first','once removed from current owner');
            const during=new AbortController();target.addEventListener('during',()=>{during.abort();events.push('abort')});target.addEventListener('during',()=>events.push('forbidden'),{signal:during.signal});events.length=0;target.dispatchEvent(new Event('during'));check(events.join(',')==='abort','abort skips later snapshot listener');
            const converted=document.createElement('i');const handler=()=>calls++;converted.addEventListener('converted',handler,{get capture(){foreignDocument.adoptNode(converted);return false}});converted.dispatchEvent(new Event('converted'));check(calls===4,'receiver resolved after conversion');converted.removeEventListener('converted',handler,{get capture(){document.adoptNode(converted);return false}});converted.dispatchEvent(new Event('converted'));check(calls===4,'remove follows conversion adoption');
            const box=document.createElement('input');box.type='checkbox';document.body.append(box);box.addEventListener('click',event=>{foreignDocument.adoptNode(box);foreignDocument.body.append(box);box.click();event.preventDefault()},{once:true});box.click();check(!box.checked&&box.ownerDocument===foreignDocument,'cancelled activation restores migrated checkbox');
        "#).unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
}
