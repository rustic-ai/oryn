//! Realm-qualified dependent signal construction. Sequence conversion completes
//! before native linking, so author iteration cannot observe a partial graph.
use super::*;
fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "AbortSignal".into(),
        member: "TypeError".into(),
    }
}
fn roots(
    owner: &Rc<V8BindingState>,
    id: u64,
) -> Result<Vec<(Rc<V8BindingState>, u64)>, V8BindingError> {
    let mut pending = VecDeque::from([(owner.clone(), id)]);
    let mut visited = BTreeSet::new();
    let mut result = Vec::new();
    while let Some((owner, id)) = pending.pop_front() {
        if !visited.insert((owner.instance_id, id)) {
            continue;
        }
        if visited.len() > 65536 {
            return Err(V8BindingError::Allocation("AbortSignal source graph limit"));
        }
        let mut parents = {
            let objects = owner.platform_objects.borrow();
            let Some(PlatformObjectRecord::AbortSignal(signal)) = objects.get(&id) else {
                return Err(invalid());
            };
            let mut parents = signal
                .foreign_sources
                .iter()
                .filter_map(|link| link.owner.upgrade().map(|owner| (owner, link.id)))
                .collect::<Vec<_>>();
            parents.extend(objects.iter().filter_map(|(source, record)| match record {
                PlatformObjectRecord::AbortSignal(signal) if signal.followers.contains(&id) => {
                    Some((owner.clone(), *source))
                }
                _ => None,
            }));
            parents
        };
        if parents.is_empty() {
            result.push((owner, id));
        } else {
            pending.extend(parents.drain(..));
        }
    }
    Ok(result)
}
pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let input = v8::Local::<v8::Object>::try_from(input).map_err(|_| invalid())?;
    let symbol = v8::Symbol::get_iterator(scope);
    let method = input
        .get(scope, symbol.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| invalid())?;
    let mut signals = Vec::new();
    iterable_conversion::consume(scope, input, method, |scope, value| {
        if signals.len() >= 65536 {
            return Err(V8BindingError::Allocation(
                "AbortSignal input sequence limit",
            ));
        }
        let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
        if object.is_proxy() {
            return Err(invalid());
        }
        let owner = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
        let id = stateful_id(scope, object)?;
        if !matches!(
            &*platform_object(&owner, id)?,
            PlatformObjectRecord::AbortSignal(_)
        ) {
            return Err(invalid());
        }
        signals.push((owner, id, v8::Global::new(scope, object)));
        Ok(())
    })?;
    let mut already_aborted = None;
    for (owner, id, _) in &signals {
        let record = platform_object(owner, *id)?;
        let PlatformObjectRecord::AbortSignal(signal) = &*record else {
            return Err(invalid());
        };
        if signal.aborted {
            already_aborted = Some(signal.reason.clone());
            break;
        }
    }
    let aborted = already_aborted.is_some();
    let reason = already_aborted.flatten();
    let mut sources = Vec::new();
    let mut identities = BTreeSet::new();
    if !aborted {
        for (owner, id, _) in &signals {
            for (owner, id) in roots(owner, *id)? {
                if identities.insert((owner.instance_id, id)) {
                    new_platform_object(scope, &owner, "AbortSignal", id)?;
                    sources.push((owner, id));
                }
            }
        }
    }
    let id = register_platform_object(
        state,
        PlatformObjectRecord::AbortSignal(AbortSignalRecord {
            aborted,
            reason,
            followers: Vec::new(),
            foreign_followers: Vec::new(),
            foreign_sources: Vec::new(),
        }),
    );
    let result = new_platform_object(scope, state, "AbortSignal", id)?;
    let linked = (|| -> Result<(), V8BindingError> {
        for (owner, source) in &sources {
            if let PlatformObjectRecord::AbortSignal(signal) =
                &mut *platform_object_mut(owner, *source)?
            {
                signal.foreign_followers.push(ForeignSignalLink {
                    owner: Rc::downgrade(state),
                    id,
                });
            }
            if let PlatformObjectRecord::AbortSignal(signal) = &mut *platform_object_mut(state, id)?
            {
                signal.foreign_sources.push(ForeignSignalLink {
                    owner: Rc::downgrade(owner),
                    id: *source,
                });
            }
        }
        platform_listener_graph::refresh_all(scope, state)?;
        for (owner, _) in &sources {
            platform_listener_graph::refresh_all(scope, owner)?;
        }
        Ok(())
    })();
    if let Err(error) = linked {
        for (owner, source) in &sources {
            if let Ok(mut record) = platform_object_mut(owner, *source) {
                if let PlatformObjectRecord::AbortSignal(signal) = &mut *record {
                    signal.foreign_followers.retain(|link| {
                        link.id != id
                            || link
                                .owner
                                .upgrade()
                                .is_none_or(|owner| owner.instance_id != state.instance_id)
                    });
                }
            }
        }
        if let Ok(mut record) = platform_object_mut(state, id) {
            if let PlatformObjectRecord::AbortSignal(signal) = &mut *record {
                signal.foreign_sources.clear();
            }
        }
        return Err(error);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn any_flattens_foreign_sources_preserves_reason_and_marks_before_callbacks() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let foreign = v8::Context::new(scope, Default::default());
        let (_foreign_state, values) = {
            let scope = &mut v8::ContextScope::new(scope, foreign);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, foreign, dom, RealmId(81), 1).unwrap();
            let values = eval(
                scope,
                "const controller=new AbortController();[controller,AbortSignal.any([controller.signal])]",
            );
            (state, v8::Global::new(scope, values))
        };
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(82), 1).unwrap();
        let key = v8::String::new(scope, "foreign").unwrap();
        let values = v8::Local::new(scope, &values);
        context
            .global(scope)
            .set(scope, key.into(), values)
            .unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)},local=new AbortController(),reason={native:true};
            const combined=AbortSignal.any(new Set([foreign[1],local.signal]));const nested=AbortSignal.any([combined,foreign[0].signal,combined]);let count=0,marked=false;
            foreign[0].signal.onabort=()=>{marked=combined.aborted&&nested.aborted};nested.onabort=()=>count++;
            foreign[0].abort(reason);check(combined.reason===reason&&nested.reason===reason&&count===1&&marked,'foreign reason identity and duplicate root');check(!local.signal.aborted,'numeric ID collision isolation');local.abort('later');check(nested.reason===reason&&count===1,'first reason wins');
            const first=AbortSignal.abort(reason),second=AbortSignal.abort('second');check(AbortSignal.any([first,second]).reason===reason,'first already aborted');
            let closed=0,error;try{AbortSignal.any({[Symbol.iterator](){let n=0;return {next(){return n++?{done:false,value:{}}:{done:false,value:local.signal}},return(){closed++;throw Error('ignored')}}}})}catch(e){error=e}check(error instanceof TypeError&&closed===1,'invalid signal closes iterator');
            const marker={};let caught;try{AbortSignal.any({get [Symbol.iterator](){throw marker}})}catch(e){caught=e}check(caught===marker,'original iterator exception');
            const reentrant=new AbortController();const ready=AbortSignal.any({*[Symbol.iterator](){yield reentrant.signal;reentrant.abort(reason)}});check(ready.aborted&&ready.reason===reason,'iteration completes before abort snapshot');
            return true;
        })()"#).is_true());
    }
}
