//! Frozen endpoints retain their own arena identity. Adopting only one endpoint
//! never turns an unrelated equal NodeId into the other endpoint.
use super::*;
#[derive(Clone)]
pub(super) struct Endpoint {
    dom: SharedDom,
    node: NodeId,
    owner: Option<std::rc::Weak<V8BindingState>>,
}
pub(super) struct Record {
    ends: [Endpoint; 2],
    released: [bool; 2],
}
pub(super) fn register(
    state: &V8BindingState,
    range: RangeRecord,
    owners: Option<[Rc<V8BindingState>; 2]>,
) -> Result<u64, V8BindingError> {
    let count = state.ranges.borrow().len();
    if count >= 64 && count.is_multiple_of(64) {
        collect_dead_ranges(state, false)?;
    }
    let ends = [range.start_container, range.end_container]
        .into_iter()
        .enumerate()
        .map(|(index, node)| {
            let owner = owners.as_ref().map(|owners| owners[index].clone());
            Endpoint {
                dom: owner
                    .as_ref()
                    .map_or_else(|| state.dom.clone(), |owner| owner.dom.clone()),
                node,
                owner: owner.map(|owner| Rc::downgrade(&owner)),
            }
        })
        .collect::<Vec<_>>();
    for endpoint in &ends {
        if matches!(
            &endpoint.dom.borrow().arena().get(endpoint.node)?.kind,
            NodeKind::Attribute { .. } | NodeKind::DocumentType { .. }
        ) {
            return Err(V8BindingError::Unsupported {
                interface: "StaticRange".into(),
                member: "InvalidNodeTypeError".into(),
            });
        }
    }
    // Allocate identity independently of frozen offsets or document equality.
    // StaticRange does not validate offsets against current node lengths.
    let root = state.document.get();
    let point = BoundaryPoint {
        container: root,
        offset: 0,
    };
    let id = state.dom.borrow_mut().create_live_range(point, point)?.0;
    state.dom.borrow_mut().drop_live_range(LiveRangeId(id))?;
    ends[0].dom.borrow_mut().retain_native(ends[0].node)?;
    if let Err(error) = ends[1].dom.borrow_mut().retain_native(ends[1].node) {
        node_lists::release_root(&ends[0].dom, ends[0].node);
        return Err(error.into());
    }
    let mut ends = ends.into_iter();
    state.static_range_custody.borrow_mut().insert(
        id,
        Record {
            ends: [ends.next().unwrap(), ends.next().unwrap()],
            released: [false, false],
        },
    );
    state.ranges.borrow_mut().insert(id, range);
    state.static_ranges.borrow_mut().insert(id);
    state
        .range_memory_delta
        .set(state.range_memory_delta.get() + range_payload_bytes(true));
    Ok(id)
}
fn owner(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    endpoint: &Endpoint,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    endpoint
        .owner
        .as_ref()
        .and_then(std::rc::Weak::upgrade)
        .or_else(|| {
            if endpoint.owner.is_some() {
                None
            } else {
                scope
                    .get_slot::<BindingStateRegistry>()
                    .and_then(|registry| {
                        registry
                            .realms
                            .borrow()
                            .get(&state.realm)
                            .and_then(std::rc::Weak::upgrade)
                    })
            }
        })
        .ok_or(V8BindingError::Allocation("StaticRange endpoint owner"))
}
pub(super) fn trace(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    id: u64,
) -> Result<(), V8BindingError> {
    let ends = state
        .static_range_custody
        .borrow()
        .get(&id)
        .ok_or(V8BindingError::Allocation("StaticRange custody"))?
        .ends
        .clone();
    let mut wrappers = Vec::new();
    for endpoint in &ends {
        let owner = owner(scope, state, endpoint)?;
        wrappers.push(wrap_node(scope, &owner, endpoint.node)?.into());
    }
    let values = v8::Array::new_with_elements(scope, &wrappers);
    let name = v8::String::new(scope, "oryn.static-range.endpoints")
        .ok_or(V8BindingError::Allocation("StaticRange endpoint roots"))?;
    let key = v8::Private::for_api(scope, Some(name));
    if object.set_private(scope, key, values.into()) != Some(true) {
        return Err(V8BindingError::Allocation("StaticRange endpoint roots"));
    }
    Ok(())
}
pub(super) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    _object: v8::Local<v8::Object>,
    id: u64,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("StaticRange record"))?;
    let ends = state
        .static_range_custody
        .borrow()
        .get(&id)
        .ok_or(V8BindingError::Allocation("StaticRange custody"))?
        .ends
        .clone();
    match name {
        "startContainer" | "endContainer" => {
            let endpoint = &ends[usize::from(name == "endContainer")];
            let owner = owner(scope, state, endpoint)?;
            Ok(wrap_node(scope, &owner, endpoint.node)?.into())
        }
        "startOffset" => Ok(v8::Integer::new_from_unsigned(scope, range.start_offset).into()),
        "endOffset" => Ok(v8::Integer::new_from_unsigned(scope, range.end_offset).into()),
        "collapsed" => Ok(v8::Boolean::new(
            scope,
            Rc::ptr_eq(&ends[0].dom, &ends[1].dom)
                && ends[0].node == ends[1].node
                && range.start_offset == range.end_offset,
        )
        .into()),
        _ => Err(V8BindingError::Unsupported {
            interface: "StaticRange".into(),
            member: name.into(),
        }),
    }
}
pub(super) fn release_dead(
    state: &V8BindingState,
    id: u64,
    local: &mut DomRuntime,
) -> Result<bool, BindingError> {
    let mut records = state.static_range_custody.borrow_mut();
    let record = records
        .get_mut(&id)
        .ok_or(DomError::UnknownRange(LiveRangeId(id)))?;
    for index in 0..2 {
        if record.released[index] {
            continue;
        }
        let endpoint = &record.ends[index];
        if Rc::ptr_eq(&endpoint.dom, &state.dom) {
            local.release_native(endpoint.node)?;
        } else {
            let Ok(mut dom) = endpoint.dom.try_borrow_mut() else {
                return Ok(false);
            };
            dom.release_native(endpoint.node)?;
        }
        record.released[index] = true;
    }
    records.remove(&id);
    Ok(true)
}
pub(super) fn retire_store(state: &mut V8BindingState) {
    // Several frame realms can share one range registry. Only its last owner
    // drops native static endpoints; an earlier realm teardown cannot revoke it.
    if Rc::strong_count(&state.static_range_custody) != 1 {
        return;
    }
    let records = std::mem::take(&mut *state.static_range_custody.borrow_mut());
    for (_, record) in records {
        for (index, endpoint) in record.ends.into_iter().enumerate() {
            if !record.released[index] {
                node_lists::release_root(&endpoint.dom, endpoint.node);
            }
        }
    }
}
struct Change {
    state: Rc<V8BindingState>,
    id: u64,
    updates: Vec<(usize, NodeId)>,
    _object: Option<v8::Global<v8::Object>>,
}
pub(super) struct Prepared {
    changes: Vec<Change>,
}
pub(super) fn capture(
    scope: &mut v8::PinScope,
    states: &[Rc<V8BindingState>],
    source: &SharedDom,
    nodes: &BTreeSet<NodeId>,
) -> Result<Prepared, V8BindingError> {
    let mut seen = BTreeSet::new();
    let mut changes = Vec::new();
    for state in states {
        if !seen.insert(Rc::as_ptr(&state.static_range_custody) as usize) {
            continue;
        }
        collect_dead_ranges(state, false)?;
        for (id, record) in state.static_range_custody.borrow().iter() {
            let updates = record
                .ends
                .iter()
                .enumerate()
                .filter_map(|(index, endpoint)| {
                    (Rc::ptr_eq(&endpoint.dom, source) && nodes.contains(&endpoint.node))
                        .then_some((index, endpoint.node))
                })
                .collect::<Vec<_>>();
            if updates.is_empty() {
                continue;
            }
            if record.released.iter().any(|released| *released) {
                return Err(V8BindingError::Allocation(
                    "pending static endpoint release",
                ));
            }
            let object = state
                .range_objects
                .borrow()
                .get(id)
                .and_then(|object| object.handle.to_local(scope));
            changes.push(Change {
                state: state.clone(),
                id: *id,
                updates,
                _object: object.map(|object| v8::Global::new(scope, object)),
            });
        }
    }
    Ok(Prepared { changes })
}
impl Prepared {
    pub(super) fn commit_ownership(
        &mut self,
        destination: &Rc<V8BindingState>,
        nodes: &BTreeMap<NodeId, NodeId>,
    ) -> Result<(), V8BindingError> {
        self.publish(destination, nodes)
    }
    pub(super) fn publish(
        &mut self,
        destination: &Rc<V8BindingState>,
        nodes: &BTreeMap<NodeId, NodeId>,
    ) -> Result<(), V8BindingError> {
        if self.changes.iter().any(|change| {
            change
                .updates
                .iter()
                .any(|(_, node)| !nodes.contains_key(node))
        }) {
            return Err(V8BindingError::Allocation(
                "incomplete StaticRange migration",
            ));
        }
        for change in &self.changes {
            let mut records = change.state.static_range_custody.borrow_mut();
            let record = records
                .get_mut(&change.id)
                .ok_or(V8BindingError::Allocation("StaticRange migration custody"))?;
            for (index, node) in &change.updates {
                record.ends[*index] = Endpoint {
                    dom: destination.dom.clone(),
                    node: nodes[node],
                    owner: Some(Rc::downgrade(destination)),
                };
            }
        }
        self.changes.clear();
        Ok(())
    }
}

pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    let invalid = || V8BindingError::Unsupported {
        interface: "StaticRange".into(),
        member: "TypeError".into(),
    };
    let init = v8::Local::<v8::Object>::try_from(args.get(0)).map_err(|_| invalid())?;
    fn required<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        init: v8::Local<v8::Object>,
        name: &str,
    ) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("StaticRange init key"))?;
        let value = init
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if value.is_undefined() {
            return Err(V8BindingError::Unsupported {
                interface: "StaticRange".into(),
                member: "TypeError".into(),
            });
        }
        Ok(value)
    }
    // Dictionary members are read in Web IDL name order. Every conversion can
    // run script; re-read native node ownership only after all conversions.
    let end = required(scope, init, "endContainer")?;
    node_identity::canonical(scope, end)?.ok_or_else(invalid)?;
    let end_offset = required(scope, init, "endOffset")?
        .uint32_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let start = required(scope, init, "startContainer")?;
    node_identity::canonical(scope, start)?.ok_or_else(invalid)?;
    let start_offset = required(scope, init, "startOffset")?
        .uint32_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let (start_owner, start_container) =
        node_identity::canonical(scope, start)?.ok_or_else(invalid)?;
    let (end_owner, end_container) = node_identity::canonical(scope, end)?.ok_or_else(invalid)?;
    let id = register(
        state,
        RangeRecord {
            start_container,
            start_offset,
            end_container,
            end_offset,
        },
        Some([start_owner, end_owner]),
    )?;
    args.this()
        .set_internal_field(0, v8::Number::new(scope, id as f64).into());
    track_range_wrapper(scope, state, args.this(), id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constructor_keeps_foreign_endpoint_identity_unbounded_offsets_and_conversion_order() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>text</p>").into_runtime().unwrap(),
        ));
        let foreign = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>text</p>").into_runtime().unwrap(),
        ));
        let context = v8::Context::new(scope, Default::default());
        let other = v8::Context::new(scope, Default::default());
        let state = {
            let scope = &mut v8::ContextScope::new(scope, context);
            install_generated_bindings(scope, context, dom.clone(), RealmId(80), 1).unwrap()
        };
        let other_state = {
            let scope = &mut v8::ContextScope::new(scope, other);
            install_generated_bindings(scope, other, foreign.clone(), RealmId(81), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let p = foreign
            .borrow()
            .query_selector(other_state.document.get(), "p")
            .unwrap()
            .unwrap();
        let node = foreign.borrow().arena().get(p).unwrap().children[0];
        let wrapper = wrap_node(scope, &other_state, node).unwrap();
        let key = v8::String::new(scope, "foreignText").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), wrapper.into())
            .unwrap();
        let source=v8::String::new(scope,r#"(()=>{const local=document.querySelector('p').firstChild;const order=[];const r=new StaticRange({get endContainer(){order.push('endContainer');return foreignText},get endOffset(){order.push('endOffset');return 999},get startContainer(){order.push('startContainer');return local},get startOffset(){order.push('startOffset');return 999}});if(order.join(',')!=='endContainer,endOffset,startContainer,startOffset'||r.collapsed||r.startContainer!==local||r.endContainer!==foreignText||r.endOffset!==999)throw Error('mixed static endpoints');const marker=new Error('offset');try{new StaticRange({endContainer:local,get endOffset(){throw marker},startContainer:local,startOffset:0});throw Error('accepted')}catch(e){if(e!==marker)throw e}try{new StaticRange({startContainer:document.createAttribute('x'),startOffset:0,endContainer:local,endOffset:0});throw Error('accepted Attr')}catch(e){if(e.name!=='InvalidNodeTypeError')throw e}return true})()"#).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        let _ = state;
    }
    #[test]
    fn mixed_migration_preserves_frozen_object_and_partial_gc_release_custody() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let source_dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p id=stays>a</p><div id=moved>b</div>")
                .into_runtime()
                .unwrap(),
        ));
        let destination_dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>x</p><div>y</div>")
                .into_runtime()
                .unwrap(),
        ));
        let context = v8::Context::new(scope, Default::default());
        let other = v8::Context::new(scope, Default::default());
        let state = {
            let scope = &mut v8::ContextScope::new(scope, context);
            install_generated_bindings(scope, context, source_dom.clone(), RealmId(82), 1).unwrap()
        };
        let destination = {
            let scope = &mut v8::ContextScope::new(scope, other);
            install_generated_bindings(scope, other, destination_dom.clone(), RealmId(83), 1)
                .unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let stays = source_dom
            .borrow()
            .get_element_by_id(state.document.get(), "stays")
            .unwrap()
            .unwrap();
        let moved = source_dom
            .borrow()
            .get_element_by_id(state.document.get(), "moved")
            .unwrap()
            .unwrap();
        let id = register(
            &state,
            RangeRecord {
                start_container: stays,
                start_offset: 99,
                end_container: moved,
                end_offset: 77,
            },
            None,
        )
        .unwrap();
        let object = new_stateful_object(scope, &state, "StaticRange", id).unwrap();
        let prototype = object.get_prototype(scope).unwrap();
        let old = wrap_node(scope, &state, moved).unwrap();
        let mut native = DomRuntime::prepare_subtree_migration(
            &source_dom,
            &destination_dom,
            moved,
            destination.document.get(),
        )
        .unwrap();
        let nodes = native.plan.nodes.iter().copied().collect::<BTreeSet<_>>();
        let mut frozen = capture(
            scope,
            &[state.clone(), destination.clone()],
            &source_dom,
            &nodes,
        )
        .unwrap();
        let mut wrappers =
            node_migration::Wrappers::capture(scope, &[state.clone()], &destination, &nodes)
                .unwrap();
        let map = DomRuntime::commit_subtree_migration(&source_dom, &destination_dom, &mut native)
            .unwrap();
        wrappers.publish(scope, &destination, &map.nodes).unwrap();
        frozen.publish(&destination, &map.nodes).unwrap();
        assert!(
            attribute(scope, &state, object, id, "endContainer")
                .unwrap()
                .strict_equals(old.into())
        );
        assert!(
            object
                .get_prototype(scope)
                .unwrap()
                .strict_equals(prototype)
        );
        assert!(
            attribute(scope, &state, object, id, "collapsed")
                .unwrap()
                .is_false()
        );
        assert_eq!(
            attribute(scope, &state, object, id, "endOffset")
                .unwrap()
                .uint32_value(scope),
            Some(77)
        );
        let held = destination_dom.borrow();
        assert!(!release_dead(&state, id, &mut source_dom.borrow_mut()).unwrap());
        assert_eq!(
            state.static_range_custody.borrow()[&id].released,
            [true, false]
        );
        drop(held);
        assert!(release_dead(&state, id, &mut source_dom.borrow_mut()).unwrap());
        state.ranges.borrow_mut().remove(&id);
        state.static_ranges.borrow_mut().remove(&id);
        state.range_objects.borrow_mut().remove(&id);
    }
}

/// Shared stores are deduplicated by the caller; endpoint multiplicity remains.
pub(super) fn owned_native_roots(
    state: &V8BindingState,
    roots: &mut traced_native_roots::Snapshot,
) -> Result<(), V8BindingError> {
    for record in state.static_range_custody.borrow().values() {
        for (index, end) in record.ends.iter().enumerate() {
            if !record.released[index] {
                roots.add(&end.dom, end.node)?;
            }
        }
    }
    Ok(())
}
