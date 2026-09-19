//! Live native iterator positions; V8 holds filter and Node identities as traced edges.
use super::*;
use oryn_dom::NodeIteratorId;
#[derive(Clone)]
struct Location {
    dom: SharedDom,
    arena: oryn_dom::ArenaId,
    id: NodeIteratorId,
    owner: std::rc::Weak<V8BindingState>,
}
pub(super) struct Record {
    location: Rc<RefCell<Location>>,
    candidate: RefCell<Option<Rc<RefCell<Location>>>>,
    what: u32,
    active: Cell<bool>,
    wrapper: v8::Weak<v8::Object>,
}
thread_local! {static RELEASES:RefCell<Vec<Location>>=const{RefCell::new(Vec::new())};}
fn release_location(place: &Location) -> (bool, Option<oryn_dom::DomError>) {
    let Ok(mut dom) = place.dom.try_borrow_mut() else {
        return (false, None);
    };
    if dom.arena().id() != place.arena || dom.node_iterator(place.id).is_err() {
        return (true, None);
    }
    let result = dom.release_node_iterator(place.id);
    (
        result.is_ok() || dom.node_iterator(place.id).is_err(),
        result.err(),
    )
}
fn release_or_queue(place: &Location) {
    if !release_location(place).0 {
        RELEASES.with(|queue| queue.borrow_mut().push(place.clone()));
    }
}
impl Drop for Record {
    fn drop(&mut self) {
        release_or_queue(&self.location.borrow());
    }
}
pub(super) fn drain() -> Result<(), BindingError> {
    oryn_dom::drain_node_iterator_releases()?;
    let pending = RELEASES.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
    let mut retained = Vec::new();
    let mut failure = None;
    for place in pending {
        let (done, error) = release_location(&place);
        if !done {
            retained.push(place)
        }
        if let Some(error) = error {
            failure.get_or_insert(error);
        }
    }
    RELEASES.with(|queue| queue.borrow_mut().extend(retained));
    if let Err(error) = oryn_dom::drain_node_iterator_releases() {
        failure.get_or_insert(error);
    }
    if let Some(error) = failure {
        return Err(error.into());
    }
    Ok(())
}
fn key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name =
        v8::String::new(scope, name).ok_or(V8BindingError::Allocation("NodeIterator key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}
fn record(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> Result<Rc<Record>, V8BindingError> {
    drain()?;
    state
        .node_iterators
        .borrow()
        .get(&stateful_id(scope, object)?)
        .cloned()
        .ok_or(V8BindingError::Allocation("NodeIterator record"))
}
pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    root: v8::Local<v8::Value>,
    what: u32,
    filter: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if !filter.is_null_or_undefined() && !filter.is_object() {
        return Err(V8BindingError::Message(
            "TypeError: NodeFilter requires an object".into(),
        ));
    }
    let (owner, node) = node_identity::canonical(scope, root)?.ok_or(V8BindingError::Message(
        "TypeError: NodeIterator root requires a Node".into(),
    ))?;
    drain()?;
    let id = state.next_traversal_id.get();
    let next = id
        .checked_add(1)
        .filter(|id| *id <= 1 << 53)
        .ok_or(V8BindingError::Allocation("NodeIterator identifiers"))?;
    let object = new_stateful_object(scope, state, "NodeIterator", id)?;
    for (name, value) in [
        ("oryn.iterator.root", root),
        ("oryn.iterator.reference", root),
        (
            "oryn.iterator.filter",
            if filter.is_null_or_undefined() {
                v8::null(scope).into()
            } else {
                filter
            },
        ),
    ] {
        let key = key(scope, name)?;
        if object.set_private(scope, key, value) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
    }
    let native = owner.dom.borrow_mut().create_node_iterator(node)?;
    let weak = Rc::downgrade(state);
    let wrapper = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_| {
            if let Some(state) = weak.upgrade() {
                state.node_iterators.borrow_mut().remove(&id);
            }
        }),
    );
    state.node_iterators.borrow_mut().insert(
        id,
        Rc::new(Record {
            location: Rc::new(RefCell::new(Location {
                dom: owner.dom.clone(),
                arena: owner.dom.borrow().arena().id(),
                id: native,
                owner: Rc::downgrade(&owner),
            })),
            candidate: RefCell::new(None),
            what,
            active: Cell::new(false),
            wrapper,
        }),
    );
    state.next_traversal_id.set(next);
    Ok(object)
}
pub(super) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let record = record(scope, state, object)?;
    if member == "filter" {
        let key = key(scope, "oryn.iterator.filter")?;
        return object
            .get_private(scope, key)
            .ok_or(V8BindingError::JavaScriptException);
    }
    if member == "whatToShow" {
        return Ok(v8::Integer::new_from_unsigned(scope, record.what).into());
    }
    let place = record.location.borrow().clone();
    let native = place.dom.borrow().node_iterator(place.id)?;
    if member == "pointerBeforeReferenceNode" {
        return Ok(v8::Boolean::new(scope, native.before).into());
    }
    let owner = if member == "referenceNode" {
        endpoint_owner(scope, &place)?
    } else {
        place
            .owner
            .upgrade()
            .ok_or(V8BindingError::Allocation("NodeIterator owner"))?
    };
    let node = match member {
        "root" => native.root,
        "referenceNode" => native.reference,
        _ => return Err(V8BindingError::Allocation("NodeIterator attribute")),
    };
    Ok(wrap_node(scope, &owner, node)?.into())
}
fn accept(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    record: &Record,
    owner: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<u16, V8BindingError> {
    if record.active.get() {
        return Err(V8BindingError::Unsupported {
            interface: "NodeIterator".into(),
            member: "InvalidStateError".into(),
        });
    }
    let kind = traversal_node_type(&owner.dom.borrow(), node)?;
    if record.what & (1 << (kind - 1)) == 0 {
        return Ok(3);
    }
    let key = key(scope, "oryn.iterator.filter")?;
    let callback = object
        .get_private(scope, key)
        .ok_or(V8BindingError::JavaScriptException)?;
    if callback.is_null() {
        return Ok(1);
    }
    let argument = wrap_node(scope, owner, node)?;
    record.active.set(true);
    let result = (|| {
        let callback_object = v8::Local::<v8::Object>::try_from(callback)
            .map_err(|_| V8BindingError::Allocation("NodeFilter"))?;
        let context = callback_object
            .get_creation_context(scope)
            .ok_or(V8BindingError::Allocation("NodeFilter realm"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let (function, receiver) =
            if let Ok(function) = v8::Local::<v8::Function>::try_from(callback) {
                (function, v8::undefined(scope).into())
            } else {
                let name = v8::String::new(scope, "acceptNode")
                    .ok_or(V8BindingError::Allocation("NodeFilter method"))?;
                let value = callback_object
                    .get(scope, name.into())
                    .ok_or(V8BindingError::JavaScriptException)?;
                let function = v8::Local::<v8::Function>::try_from(value).map_err(|_| {
                    throw_type_error(scope, "NodeFilter.acceptNode must be callable");
                    V8BindingError::JavaScriptException
                })?;
                (function, callback_object.into())
            };
        let value = function
            .call(scope, receiver, &[argument.into()])
            .ok_or(V8BindingError::JavaScriptException)?;
        Ok(value
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)? as u16)
    })();
    record.active.set(false);
    result
}
fn endpoint_owner(
    scope: &mut v8::PinScope,
    place: &Location,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    let arena = place.dom.borrow().node_iterator_reference_arena(place.id)?;
    let registry = scope
        .get_slot::<BindingStateRegistry>()
        .ok_or(V8BindingError::Allocation("NodeIterator agent"))?;
    let owner = registry
        .instances
        .borrow()
        .values()
        .filter_map(RcWeak::upgrade)
        .find(|state| state.dom.borrow().arena().id() == arena)
        .ok_or(V8BindingError::Allocation("NodeIterator endpoint owner"))?;
    Ok(owner)
}
struct CandidateGuard {
    record: Rc<Record>,
}
impl Drop for CandidateGuard {
    fn drop(&mut self) {
        if let Some(place) = self.record.candidate.borrow_mut().take() {
            release_or_queue(&place.borrow());
        }
    }
}
pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let record = record(scope, state, object)?;
    if member == "detach" {
        return Ok(None);
    }
    if record.active.get() {
        return Err(V8BindingError::Unsupported {
            interface: "NodeIterator".into(),
            member: "InvalidStateError".into(),
        });
    }
    let forward = match member {
        "nextNode" => true,
        "previousNode" => false,
        _ => return Err(V8BindingError::Allocation("NodeIterator method")),
    };
    let place = record.location.borrow().clone();
    let native = place.dom.borrow().node_iterator(place.id)?;
    let owner = endpoint_owner(scope, &place)?;
    let candidate_id = place.dom.borrow_mut().create_node_iterator(native.root)?;
    let candidate_place = Rc::new(RefCell::new(Location {
        id: candidate_id,
        ..place.clone()
    }));
    *record.candidate.borrow_mut() = Some(candidate_place.clone());
    let _guard = CandidateGuard {
        record: record.clone(),
    };
    DomRuntime::set_node_iterator_reference_from(
        &place.dom,
        candidate_id,
        &owner.dom,
        native.reference,
        native.before,
    )?;
    loop {
        let cursor = candidate_place.borrow().clone();
        let native = cursor.dom.borrow().node_iterator(cursor.id)?;
        let owner = endpoint_owner(scope, &cursor)?;
        let root = oryn_dom::NodeRef {
            arena: cursor.dom.borrow().arena().id(),
            node: native.root,
        };
        let before = !forward;
        let next = if forward {
            if native.before {
                Some(native.reference)
            } else {
                owner
                    .dom
                    .borrow()
                    .iterator_following_from(root, native.reference, false)?
            }
        } else if !native.before {
            Some(native.reference)
        } else {
            owner
                .dom
                .borrow()
                .iterator_preceding_from(root, native.reference)?
        };
        let Some(next) = next else {
            return Ok(Some(v8::null(scope).into()));
        };
        let candidate = wrap_node(scope, &owner, next)?;
        DomRuntime::set_node_iterator_reference_from(
            &cursor.dom,
            cursor.id,
            &owner.dom,
            next,
            before,
        )?;
        if accept(scope, object, &record, &owner, next)? == 1 {
            // Removal during the callback adjusts the private traversal cursor,
            // while the public reference continues to denote the old position.
            let cursor = candidate_place.borrow().clone();
            let accepted = cursor.dom.borrow().node_iterator(cursor.id)?;
            let owner = endpoint_owner(scope, &cursor)?;
            let place = record.location.borrow().clone();
            let reference = wrap_node(scope, &owner, accepted.reference)?;
            let key = key(scope, "oryn.iterator.reference")?;
            if object.set_private(scope, key, reference.into()) != Some(true) {
                return Err(V8BindingError::JavaScriptException);
            }
            DomRuntime::set_node_iterator_reference_from(
                &place.dom,
                place.id,
                &owner.dom,
                accepted.reference,
                accepted.before,
            )?;
            return Ok(Some(candidate.into()));
        }
    }
}
pub(super) struct Prepared {
    records: Vec<(
        Rc<RefCell<Location>>,
        NodeIteratorId,
        v8::Global<v8::Object>,
    )>,
}
pub(super) fn capture(
    scope: &mut v8::PinScope,
    states: &[Rc<V8BindingState>],
    source: &SharedDom,
    nodes: &BTreeSet<NodeId>,
) -> Result<Prepared, V8BindingError> {
    drain()?;
    if RELEASES.with(|queue| {
        queue
            .borrow()
            .iter()
            .any(|place| Rc::ptr_eq(&place.dom, source))
    }) {
        return Err(V8BindingError::Allocation(
            "NodeIterator release blocks migration",
        ));
    }
    let mut records = Vec::new();
    for state in states {
        for record in state.node_iterators.borrow().values() {
            let mut locations = vec![record.location.clone()];
            if let Some(candidate) = record.candidate.borrow().clone() {
                locations.push(candidate)
            }
            for location in locations {
                let place = location.borrow();
                if Rc::ptr_eq(&place.dom, source)
                    && nodes.contains(&source.borrow().node_iterator(place.id)?.root)
                {
                    let object =
                        record
                            .wrapper
                            .to_local(scope)
                            .ok_or(V8BindingError::Allocation(
                                "NodeIterator pending collection",
                            ))?;
                    records.push((location.clone(), place.id, v8::Global::new(scope, object)));
                }
            }
        }
    }
    Ok(Prepared { records })
}
impl Prepared {
    pub(super) fn commit_ownership(
        &mut self,
        destination: &Rc<V8BindingState>,
        ids: &BTreeMap<NodeIteratorId, NodeIteratorId>,
    ) -> Result<(), V8BindingError> {
        for (_, id, _) in &self.records {
            if !ids.contains_key(id) {
                return Err(V8BindingError::Allocation("NodeIterator migration map"));
            }
        }
        for (record, id, _) in self.records.drain(..) {
            *record.borrow_mut() = Location {
                dom: destination.dom.clone(),
                arena: destination.dom.borrow().arena().id(),
                id: ids[&id],
                owner: Rc::downgrade(destination),
            };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_interface_errors_and_conversion_reentrancy_use_filter_realm() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let foreign = v8::Context::new(scope, Default::default());
        let values = {
            let scope = &mut v8::ContextScope::new(scope, foreign);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, foreign, dom, RealmId(185), 1).unwrap();
            let source=v8::String::new(scope,r#"(()=>{
              const revokedObject=Proxy.revocable({},{}),revokedFunction=Proxy.revocable(()=>1,{}),revokedMethod=Proxy.revocable(()=>1,{});
              revokedObject.revoke();revokedFunction.revoke();revokedMethod.revoke();
              return [[{},{acceptNode:{}},revokedObject.proxy,revokedFunction.proxy,{acceptNode:revokedMethod.proxy}],TypeError];
            })()"#).unwrap();
            let value = v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap();
            v8::Global::new(scope, value)
        };
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=root><span></span></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(186), 1).unwrap();
        let name = v8::String::new(scope, "foreignFilters").unwrap();
        let values = v8::Local::new(scope, &values);
        context
            .global(scope)
            .set(scope, name.into(), values)
            .unwrap();
        let source=v8::String::new(scope,r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)},[filters,ForeignTypeError]=foreignFilters,root=document.getElementById('root');
          for(const factory of ['createNodeIterator','createTreeWalker']) {
            const method=factory==='createNodeIterator'?'nextNode':'firstChild';
            for(const filter of filters){const cursor=document[factory](root,1,filter);let caught=false;
              try{cursor[method]()}catch(error){caught=true;check(error instanceof ForeignTypeError&&!(error instanceof TypeError),'filter error realm '+factory)}check(caught,'missing filter error');
            }
            for(const phase of ['getter','conversion']){
              let cursor,fail=true;const marker={};const reenter=()=>{let caught=false;try{cursor[method]()}catch(error){caught=true;check(error.name==='InvalidStateError','active callback flag');check(error instanceof DOMException&&error.code===11,'native reentry exception brand')}check(caught,'recursive filter accepted');throw marker};
              const filter={get acceptNode(){if(fail&&phase==='getter')reenter();return()=>({valueOf(){if(fail&&phase==='conversion')reenter();return 65537}})}};
              cursor=document[factory](root,1,filter);try{cursor[method]();throw Error('exception swallowed')}catch(error){check(error===marker,'exact callback exception')}
              check(factory==='createNodeIterator'?cursor.referenceNode===root&&cursor.pointerBeforeReferenceNode:cursor.currentNode===root,'failure preserves cursor');
              fail=false;check(cursor[method]() === (factory==='createNodeIterator'?root:root.firstChild),'active flag reset and unsigned short conversion');
            }
          }return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn borrowed_iterator_cleanup_retains_runtime_and_ignores_replaced_arena() {
        let owner = Rc::new(RefCell::new(DomRuntime::new()));
        let root = owner.borrow_mut().create_element("div");
        let id = owner.borrow_mut().create_node_iterator(root).unwrap();
        let place = Location {
            dom: owner.clone(),
            arena: owner.borrow().arena().id(),
            id,
            owner: RcWeak::new(),
        };
        let held = owner.borrow_mut();
        release_or_queue(&place);
        drop(held);
        assert!(RELEASES.with(|queue| {
            queue
                .borrow()
                .iter()
                .any(|item| item.id == id && item.arena == place.arena)
        }));
        let replacement = DomRuntime::new();
        let (arena, document) = replacement.snapshot();
        owner
            .borrow_mut()
            .replace_parsed_document(arena, document)
            .unwrap();
        let document = owner.borrow().document();
        let live = owner.borrow_mut().create_node_iterator(document).unwrap();
        drain().unwrap();
        assert!(owner.borrow().node_iterator(live).is_ok());
        assert!(
            !RELEASES.with(|queue| queue.borrow().iter().any(|item| item.arena == place.arena))
        );
        owner.borrow_mut().release_node_iterator(live).unwrap();
        // Already-consumed release handles are terminal, not endlessly retried.
        let consumed = Location {
            dom: owner.clone(),
            arena: owner.borrow().arena().id(),
            id: live,
            owner: RcWeak::new(),
        };
        RELEASES.with(|queue| queue.borrow_mut().push(consumed));
        drain().unwrap();
        assert!(!RELEASES.with(|queue| {
            queue
                .borrow()
                .iter()
                .any(|item| item.id == live && Rc::ptr_eq(&item.dom, &owner))
        }));
    }
    #[test]
    fn native_ownership_settlement_precedes_failed_wrapper_publication_cleanup() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let source_dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id='r'></div>")
                .into_runtime()
                .unwrap(),
        ));
        let target_dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let source = {
            let scope = &mut v8::ContextScope::new(scope, first);
            install_generated_bindings(scope, first, source_dom.clone(), RealmId(83), 1).unwrap()
        };
        let target = {
            let scope = &mut v8::ContextScope::new(scope, second);
            install_generated_bindings(scope, second, target_dom.clone(), RealmId(84), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let root = source_dom
            .borrow()
            .get_element_by_id(source.document.get(), "r")
            .unwrap()
            .unwrap();
        let wrapper = wrap_node(scope, &source, root).unwrap();
        let filter = v8::null(scope);
        let iterator = create(scope, &source, wrapper.into(), 1, filter.into()).unwrap();
        let id = stateful_id(scope, iterator).unwrap();
        let old = source.node_iterators.borrow()[&id].location.borrow().id;
        let mut native = DomRuntime::prepare_subtree_migration(
            &source_dom,
            &target_dom,
            root,
            target.document.get(),
        )
        .unwrap();
        let nodes = native.plan.nodes.iter().copied().collect();
        let mut prepared = capture(
            scope,
            &[source.clone(), target.clone()],
            &source_dom,
            &nodes,
        )
        .unwrap();
        let migrated =
            DomRuntime::commit_subtree_migration(&source_dom, &target_dom, &mut native).unwrap();
        prepared
            .commit_ownership(&target, &migrated.iterators)
            .unwrap();
        // Stop before wrapper publication, as with a retained transaction's
        // failed V8 stage, and retire the original Store under a target borrow.
        let target_id = migrated.iterators[&old];
        let held = target_dom.borrow();
        source.node_iterators.borrow_mut().remove(&id);
        assert!(RELEASES.with(|queue| {
            queue
                .borrow()
                .iter()
                .any(|place| place.id == target_id && Rc::ptr_eq(&place.dom, &target_dom))
        }));
        drop(held);
        drain().unwrap();
        assert!(target_dom.borrow().node_iterator(target_id).is_err());
        assert!(source_dom.borrow().node_iterator(old).is_err());
    }
    #[test]
    fn root_adoption_inside_filter_keeps_iterator_filter_and_original_nodes() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let source_dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id='r'><span id='a'></span></div>")
                .into_runtime()
                .unwrap(),
        ));
        let target_dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let source = {
            let scope = &mut v8::ContextScope::new(scope, first);
            install_generated_bindings(scope, first, source_dom, RealmId(81), 1).unwrap()
        };
        let target = {
            let scope = &mut v8::ContextScope::new(scope, second);
            install_generated_bindings(scope, second, target_dom, RealmId(82), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let foreign = wrap_node(scope, &target, target.document.get()).unwrap();
        let key = v8::String::new(scope, "foreignDocument").unwrap();
        first
            .global(scope)
            .set(scope, key.into(), foreign.into())
            .unwrap();
        let script=v8::String::new(scope,r#"
          const root=document.getElementById('r'),child=document.getElementById('a');let once=false;
          const filter=node=>{if(!once){once=true;foreignDocument.adoptNode(root)}return 1};
          const iterator=document.createNodeIterator(root,1,filter),prototype=Object.getPrototypeOf(iterator);
          if(iterator.nextNode()!==root||iterator.root!==root||iterator.referenceNode!==root||iterator.filter!==filter)throw Error('adopted iterator identity');
          if(iterator.nextNode()!==child||Object.getPrototypeOf(iterator)!==prototype)throw Error('adopted descendants');
          document.adoptNode(root);
          if(iterator.previousNode()!==child||iterator.root!==root||root.ownerDocument!==document)throw Error('repeated migration');
          iterator.detach();if(iterator.previousNode()!==root)throw Error('detach');
        "#).unwrap();
        v8::Script::compile(scope, script, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert_eq!(source.node_iterators.borrow().len(), 1);
        assert!(target.node_iterators.borrow().is_empty());
    }
    #[test]
    fn live_filtering_removal_direction_and_reentry_preserve_reference() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><div id='r'><span id='a'></span><span id='b'></span></div></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source=v8::String::new(scope,r#"
            const root=document.getElementById('r'),a=document.getElementById('a');let calls=0;
            const filter={acceptNode(node){calls++;return node===root?NodeFilter.FILTER_REJECT:NodeFilter.FILTER_ACCEPT}};
            const it=document.createNodeIterator(root,NodeFilter.SHOW_ELEMENT,filter);
            if(calls||it.filter!==filter||it.referenceNode!==root||!it.pointerBeforeReferenceNode)throw Error('eager filtering');
            if(it.nextNode()!==a||calls!==2)throw Error('lazy reject descendants');
            if(it.previousNode()!==a||!it.pointerBeforeReferenceNode)throw Error('direction toggle');
            a.remove();if(it.referenceNode.id!=='b'||!it.pointerBeforeReferenceNode)throw Error('native removal adjustment');
            const c=document.createElement('span');c.id='c';root.append(c);
            if(it.nextNode().id!=='b'||it.nextNode()!==c||it.nextNode()!==null)throw Error('live insertion');
            let iterator;const sentinel={};iterator=document.createNodeIterator(root,NodeFilter.SHOW_ELEMENT,()=>{
                try{iterator.nextNode();throw Error('reentrant accepted')}catch(e){if(e.name!=='InvalidStateError')throw e}
                throw sentinel;
            });
            try{iterator.nextNode();throw Error('swallowed')}catch(e){if(e!==sentinel)throw e}
            if(iterator.referenceNode!==root||!iterator.pointerBeforeReferenceNode)throw Error('failed filter changed reference');
            let removeIt;removeIt=document.createNodeIterator(root,1,node=>{if(node===c)c.remove();return 1});
            removeIt.nextNode();removeIt.nextNode();
            if(removeIt.nextNode()!==c||removeIt.referenceNode.id!=='b'||removeIt.pointerBeforeReferenceNode)throw Error('filtered removal retarget');
            let order=[];document.createNodeIterator(root,{valueOf(){order.push('mask');return 1}},{get acceptNode(){throw Error('eager method lookup')}});
            if(order.join()!=='mask')throw Error('conversion');
        "#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
}

pub(super) fn owned_native_roots(
    state: &V8BindingState,
    roots: &mut traced_native_roots::Snapshot,
    seen: &mut BTreeSet<(oryn_dom::ArenaId, NodeIteratorId)>,
) -> Result<(), V8BindingError> {
    for record in state.node_iterators.borrow().values() {
        let mut places = vec![record.location.borrow().clone()];
        if let Some(candidate) = record.candidate.borrow().as_ref() {
            places.push(candidate.borrow().clone());
        }
        for place in places {
            if !seen.insert((place.arena, place.id)) {
                continue;
            }
            let dom = place.dom.borrow();
            if dom.arena().id() != place.arena {
                return Err(V8BindingError::Allocation("NodeIterator custody arena"));
            }
            match dom.node_iterator_native_roots(place.id) {
                Ok(nodes) => {
                    for node in nodes {
                        roots.add_ref(node)?;
                    }
                }
                Err(DomError::UnknownIterator(_)) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod deferred_release_checkpoint_tests {
    use super::*;
    #[test]
    fn ordinary_custody_checkpoint_drains_iterator_finalizer_after_native_borrow() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles,&mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(904), 1).unwrap();
        {
            v8::scope!(let temporary,scope);
            let code=v8::String::new(temporary,"(()=>{document.createNodeIterator(document.createElement('aside'));return true})()").unwrap();
            v8::Script::compile(temporary, code, None)
                .unwrap()
                .run(temporary)
                .unwrap();
        }
        let place = state
            .node_iterators
            .borrow()
            .values()
            .next()
            .unwrap()
            .location
            .borrow()
            .clone();
        let root = state.dom.borrow().node_iterator(place.id).unwrap().root;
        {
            let held = state.dom.borrow_mut();
            scope.low_memory_notification();
            scope.low_memory_notification();
            assert!(state.node_iterators.borrow().is_empty());
            assert_eq!(held.native_root_count(root).unwrap(), 2);
        }
        state.process_weak_releases().unwrap();
        assert!(state.dom.borrow().node_iterator(place.id).is_err());
        assert!(state.dom.borrow().arena().get(root).is_err());
        custom_elements::refresh_native_registry_roots(scope).unwrap();
    }
}
