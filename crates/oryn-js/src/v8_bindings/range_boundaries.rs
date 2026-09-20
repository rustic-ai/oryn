//! Range boundary conversion and identity routing across native arenas.
use super::*;
fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Range".into(),
        member: name.into(),
    }
}
pub(super) fn handles(member: &str) -> bool {
    matches!(
        member,
        "setStart"
            | "setEnd"
            | "setStartBefore"
            | "setStartAfter"
            | "setEndBefore"
            | "setEndAfter"
            | "selectNode"
            | "selectNodeContents"
            | "compareBoundaryPoints"
            | "comparePoint"
            | "isPointInRange"
            | "intersectsNode"
    )
}
pub(super) fn owner(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<(Rc<V8BindingState>, u64), V8BindingError> {
    if object.is_proxy() {
        return Err(error("TypeError"));
    }
    let state = binding_state_for_receiver(scope, object).ok_or_else(|| error("TypeError"))?;
    let id = stateful_id(scope, object)?;
    let actual = state
        .range_objects
        .borrow()
        .get(&id)
        .and_then(|record| record.handle.to_local(scope));
    if state.static_ranges.borrow().contains(&id)
        || !actual.is_some_and(|actual| actual.strict_equals(object.into()))
    {
        return Err(error("TypeError"));
    }
    state.dom.borrow().live_range(LiveRangeId(id))?;
    Ok((state, id))
}
fn read(state: &V8BindingState, id: u64) -> Result<RangeRecord, V8BindingError> {
    Ok(state.dom.borrow().live_range(LiveRangeId(id))?.into())
}
fn validate(dom: &DomRuntime, node: NodeId, offset: u32) -> Result<(), V8BindingError> {
    if matches!(dom.arena().get(node)?.kind, NodeKind::DocumentType { .. }) {
        return Err(error("InvalidNodeTypeError"));
    }
    validate_boundary_offset(dom, node, offset)
}
fn clear_outside_selections(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    range: &RangeRecord,
) -> Result<(), V8BindingError> {
    clear_composed_selection_for_range(state, id);
    let root = shadow_including_tree_root(&state.dom.borrow(), range.start_container)?;
    let states = scope
        .get_slot::<BindingStateRegistry>()
        .map(|states| {
            states
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for candidate in states {
        if Rc::ptr_eq(&candidate.dom, &state.dom) && candidate.selection_range.get() == Some(id) {
            if candidate.document_root() != root {
                clear_selection(&candidate);
            } else {
                synchronize_selection_range(&candidate)?;
            }
        }
    }
    Ok(())
}
fn publish(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    source: &Rc<V8BindingState>,
    id: u64,
    destination: &Rc<V8BindingState>,
    range: RangeRecord,
) -> Result<(), V8BindingError> {
    let owner = traced_native_roots::prepare_range_owner(scope, destination, object, &range)?;
    if Rc::ptr_eq(&source.dom, &destination.dom) {
        let native = LiveRange::from(&range);
        source
            .dom
            .borrow_mut()
            .set_live_range(LiveRangeId(id), native.start, native.end)?;
        source.ranges.borrow_mut().insert(id, range.clone());
        owner.publish();
        return clear_outside_selections(scope, source, id, &range);
    }
    // Keep old endpoints alive through native accounting commit. Their final
    // reclamation belongs to the existing deferred lease-release mechanism.
    let previous = read(source, id)?;
    let _start = oryn_dom::TrackedNodeLease::new(&source.dom, previous.start_container)?;
    let _end = oryn_dom::TrackedNodeLease::new(&source.dom, previous.end_container)?;
    let new = register_live_range(destination, range.clone())?;
    source.dom.borrow_mut().drop_live_range(LiveRangeId(id))?;
    source.ranges.borrow_mut().remove(&id);
    let mut wrapper = source
        .range_objects
        .borrow_mut()
        .remove(&id)
        .expect("preflighted original Range wrapper");
    wrapper.selection_root.take();
    object.set_internal_field(0, v8::Number::new(scope, new as f64).into());
    object.set_internal_field(
        1,
        v8::BigInt::new_from_u64(scope, destination.instance_id).into(),
    );
    destination.range_objects.borrow_mut().insert(new, wrapper);
    owner.publish();
    let states = scope
        .get_slot::<BindingStateRegistry>()
        .map(|states| {
            states
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for candidate in states {
        if Rc::ptr_eq(&candidate.dom, &source.dom) && candidate.selection_range.get() == Some(id) {
            clear_selection(&candidate);
        }
    }
    source
        .range_memory_delta
        .set(source.range_memory_delta.get() - range_payload_bytes(false));
    source.rooted_range_selection_ids.borrow_mut().take();
    destination.rooted_range_selection_ids.borrow_mut().take();
    apply_range_memory_accounting(scope, source);
    apply_range_memory_accounting(scope, destination);
    Ok(())
}
/// Select the original wrapper after a post-connection script may have adopted
/// both the Range and its new parent into another native arena.
pub(super) fn select_node(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let (state, id) = owner(scope, object)?;
    let (target, node) =
        node_identity::argument(scope, value)?.ok_or_else(|| error("TypeError"))?;
    let (container, start) = relative_boundary(&target.dom.borrow(), node, false)?;
    publish(
        scope,
        object,
        &state,
        id,
        &target,
        RangeRecord {
            start_container: container,
            start_offset: start,
            end_container: container,
            end_offset: start + 1,
        },
    )
}
pub(super) fn collapse_native(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    target: &Rc<V8BindingState>,
    container: NodeId,
    offset: u32,
) -> Result<(), V8BindingError> {
    validate(&target.dom.borrow(), container, offset)?;
    let (state, id) = owner(scope, object)?;
    publish(
        scope,
        object,
        &state,
        id,
        target,
        RangeRecord {
            start_container: container,
            start_offset: offset,
            end_container: container,
            end_offset: offset,
        },
    )
}
pub(super) fn set_end_native(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    target: &Rc<V8BindingState>,
    container: NodeId,
    offset: u32,
) -> Result<(), V8BindingError> {
    validate(&target.dom.borrow(), container, offset)?;
    let (state, id) = owner(scope, object)?;
    let mut range = read(&state, id)?;
    if !Rc::ptr_eq(&state.dom, &target.dom) {
        range = RangeRecord {
            start_container: container,
            start_offset: offset,
            end_container: container,
            end_offset: offset,
        };
    } else {
        range.end_container = container;
        range.end_offset = offset;
        if tree_root(&state.dom.borrow(), container)?
            != tree_root(&state.dom.borrow(), range.start_container)?
            || compare_boundary_points(
                &state.dom.borrow(),
                (range.start_container, range.start_offset),
                (container, offset),
            )? > 0
        {
            range.start_container = container;
            range.start_offset = offset;
        }
    }
    publish(scope, object, &state, id, target, range)
}
pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    owner(scope, object)?; // receiver brand always precedes argument conversion
    if member == "compareBoundaryPoints" {
        let how = (args
            .get(0)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)? as u16) as u32;
        let other = args
            .get(1)
            .try_cast::<v8::Object>()
            .map_err(|_| error("TypeError"))?;
        let (other_owner, other_id) = owner(scope, other)?;
        let (state, id) = owner(scope, object)?;
        if how > 3 {
            return Err(error("NotSupportedError"));
        }
        if !Rc::ptr_eq(&state.dom, &other_owner.dom) {
            return Err(error("WrongDocumentError"));
        }
        let range = read(&state, id)?;
        let other = read(&other_owner, other_id)?;
        let dom = state.dom.borrow();
        if tree_root(&dom, range.start_container)? != tree_root(&dom, other.start_container)? {
            return Err(error("WrongDocumentError"));
        }
        let (left, right) = match how {
            0 => (
                (range.start_container, range.start_offset),
                (other.start_container, other.start_offset),
            ),
            1 => (
                (range.end_container, range.end_offset),
                (other.start_container, other.start_offset),
            ),
            2 => (
                (range.end_container, range.end_offset),
                (other.end_container, other.end_offset),
            ),
            3 => (
                (range.start_container, range.start_offset),
                (other.end_container, other.end_offset),
            ),
            _ => unreachable!(),
        };
        return Ok(Some(
            v8::Integer::new(scope, compare_boundary_points(&dom, left, right)?).into(),
        ));
    }
    node_identity::argument(scope, args.get(0))?.ok_or_else(|| error("TypeError"))?;
    let offset = if matches!(
        member,
        "setStart" | "setEnd" | "comparePoint" | "isPointInRange"
    ) {
        args.get(1)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?
    } else {
        0
    };
    let (target, node) =
        node_identity::argument(scope, args.get(0))?.ok_or_else(|| error("TypeError"))?;
    let (state, id) = owner(scope, object)?;
    let mut range = read(&state, id)?;
    if matches!(member, "comparePoint" | "isPointInRange" | "intersectsNode") {
        if !Rc::ptr_eq(&target.dom, &state.dom)
            || tree_root(&target.dom.borrow(), node)?
                != tree_root(&state.dom.borrow(), range.start_container)?
        {
            return if member == "comparePoint" {
                Err(error("WrongDocumentError"))
            } else {
                Ok(Some(v8::Boolean::new(scope, false).into()))
            };
        }
        let dom = state.dom.borrow();
        if member == "intersectsNode" {
            let intersects = if let Some(parent) = dom.parent(node)? {
                let children = light_children(&dom, parent)?;
                let index = children
                    .iter()
                    .position(|child| *child == node)
                    .ok_or(DomError::NotAChild(node, parent))? as u32;
                compare_boundary_points(
                    &dom,
                    (parent, index),
                    (range.end_container, range.end_offset),
                )? < 0
                    && compare_boundary_points(
                        &dom,
                        (parent, index + 1),
                        (range.start_container, range.start_offset),
                    )? > 0
            } else {
                true
            };
            return Ok(Some(v8::Boolean::new(scope, intersects).into()));
        }
        validate(&dom, node, offset)?;
        let comparison = if compare_boundary_points(
            &dom,
            (node, offset),
            (range.start_container, range.start_offset),
        )? < 0
        {
            -1
        } else if compare_boundary_points(
            &dom,
            (node, offset),
            (range.end_container, range.end_offset),
        )? > 0
        {
            1
        } else {
            0
        };
        return Ok(Some(if member == "comparePoint" {
            v8::Integer::new(scope, comparison).into()
        } else {
            v8::Boolean::new(scope, comparison == 0).into()
        }));
    }
    let (container, start, end) = {
        let dom = target.dom.borrow();
        if member == "selectNode" {
            let (parent, offset) = relative_boundary(&dom, node, false)?;
            (parent, offset, offset + 1)
        } else if member == "selectNodeContents" {
            validate(&dom, node, 0)?;
            (node, 0, boundary_length(&dom, node)?)
        } else if matches!(member, "setStart" | "setEnd") {
            validate(&dom, node, offset)?;
            (node, offset, offset)
        } else {
            let (parent, offset) = relative_boundary(&dom, node, member.ends_with("After"))?;
            (parent, offset, offset)
        }
    };
    if member.starts_with("selectNode") || !Rc::ptr_eq(&state.dom, &target.dom) {
        range = RangeRecord {
            start_container: container,
            start_offset: start,
            end_container: container,
            end_offset: end,
        };
    } else if member.starts_with("setStart") {
        range.start_container = container;
        range.start_offset = start;
        if tree_root(&state.dom.borrow(), container)?
            != tree_root(&state.dom.borrow(), range.end_container)?
            || compare_boundary_points(
                &state.dom.borrow(),
                (container, start),
                (range.end_container, range.end_offset),
            )? > 0
        {
            range.end_container = container;
            range.end_offset = start;
        }
    } else {
        range.end_container = container;
        range.end_offset = end;
        if tree_root(&state.dom.borrow(), container)?
            != tree_root(&state.dom.borrow(), range.start_container)?
            || compare_boundary_points(
                &state.dom.borrow(),
                (range.start_container, range.start_offset),
                (container, end),
            )? > 0
        {
            range.start_container = container;
            range.start_offset = end;
        }
    }
    publish(scope, object, &state, id, &target, range)?;
    Ok(None)
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
    fn boundary_conversion_is_atomic_and_all_methods_follow_foreign_native_nodes() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "range-boundary").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let a = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><div id=source>abcd</div></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(180), 1).unwrap()
        };
        let b = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><div id=target>wxyz</div></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(181), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let document = wrap_node(scope, &b, b.document_root()).unwrap();
        let key = v8::String::new(scope, "destination").unwrap();
        first.global(scope).set(scope, key.into(), document.into());
        assert!(eval(scope,r#"(()=>{
          const check=(v,label)=>{if(!v)throw Error(label)},source=document.getElementById('source'),target=destination.getElementById('target'),range=document.createRange();range.selectNodeContents(source.firstChild);const prototype=Object.getPrototypeOf(range);range.marker=17;
          for(const method of ['setStart','setEnd','comparePoint','isPointInRange']){const failure={method};try{range[method](source.firstChild,{valueOf(){throw failure}});throw Error('conversion accepted')}catch(error){check(error===failure,'exact numeric exception')}check(range.startOffset===0&&range.endOffset===4,'no conversion-error mutation')}
          let calls=0;range.setStart(source.firstChild,{valueOf(){calls++;destination.body.append(source);return 2}});check(calls===1&&range.startContainer===source.firstChild&&range.startOffset===2,'adopt argument during conversion');
          check(range.endContainer===source.firstChild&&range.endOffset===2&&range.marker===17&&Object.getPrototypeOf(range)===prototype,'foreign store transfer preserves object');
          range.setEnd(source.firstChild,4);check(range.toString()==='cd','native live range after transfer');source.firstChild.insertData(0,'!');check(range.startOffset===3&&range.endOffset===5,'destination mutation updates transferred range');
          for(const method of ['setStart','setEnd','setStartBefore','setStartAfter','setEndBefore','setEndAfter','selectNode','selectNodeContents']){
            range.selectNodeContents(document.body);range[method](target,0);check(range.startContainer.ownerDocument===destination&&range.endContainer.ownerDocument===destination,method+' foreign boundaries');check(target.ownerDocument===destination,method+' does not adopt target');
          }
          range.selectNodeContents(target.firstChild);const before=range.startContainer;
          try{range.setStart(document.body,0xffffffff);throw Error('invalid boundary accepted')}catch(error){check(error.name==='IndexSizeError','invalid destination offset')}
          check(range.startContainer===before&&range.toString()==='wxyz','failed destination leaves original range');
          check(!range.isPointInRange(document.body,0)&&!range.intersectsNode(document.body),'foreign tests return false');
          try{range.comparePoint(document.body,0);throw Error('foreign compare accepted')}catch(error){check(error.name==='WrongDocumentError','foreign compare error')}
          const selection=getSelection();
          for(const method of ['setStart','setEnd','setStartBefore','setStartAfter','setEndBefore','setEndAfter','selectNode','selectNodeContents']){
            const selected=document.createRange();selected.selectNodeContents(document.body);selection.removeAllRanges();selection.addRange(selected);selected[method](target,0);
            check(selection.rangeCount===0&&selected.startContainer.ownerDocument===destination,method+' foreign selection detachment');
          }
          const other=destination.createRange();other.selectNodeContents(target.firstChild);check(range.compareBoundaryPoints(65536,other)===0,'unsigned short modulo');
          const failure={};try{range.compareBoundaryPoints({valueOf(){throw failure}},other)}catch(error){check(error===failure,'compare conversion identity')}
          try{range.compareBoundaryPoints(99,{});throw Error('invalid range accepted')}catch(error){check(error instanceof TypeError,'argument brand before algorithm how check')}
          check(range.compareBoundaryPoints({valueOf(){document.body.append(target);return 0}},other)===0,'both original ranges follow conversion adoption');
          return true;
        })()"#).is_true());
        let _ = a;
    }
    #[test]
    fn selected_range_leaves_selection_on_fragment_boundary() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=selected><br></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(182), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const selected=document.getElementById('selected'),selection=getSelection();
          for(const method of ['setStart','setEnd','setStartBefore','setStartAfter','setEndBefore','setEndAfter','selectNode','selectNodeContents']){
            const range=document.createRange();range.selectNodeContents(selected);selection.removeAllRanges();selection.addRange(range);
            const fragment=document.createDocumentFragment(),target=document.createElement('div');fragment.append(target);range[method](target,0);
            if(selection.rangeCount!==0||range.startContainer.getRootNode()!==fragment)throw Error(method+' retained outside selection');
          }return true;
        })()"#).is_true());
    }
}
