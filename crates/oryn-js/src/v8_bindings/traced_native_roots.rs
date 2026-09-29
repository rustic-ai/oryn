//! Native leases owned by V8's traced object graph are not independent roots.
//! One refresh snapshot keeps multiplicity and exact arena/generation identity.
use super::*;
#[derive(Default)]
pub(super) struct Snapshot {
    owned: BTreeMap<oryn_dom::ArenaId, BTreeMap<NodeId, u32>>,
    live_documents: BTreeMap<oryn_dom::ArenaId, BTreeSet<NodeId>>,
    leases: BTreeSet<usize>,
}
impl Snapshot {
    pub(super) fn add_ref(&mut self, node: oryn_dom::NodeRef) -> Result<(), V8BindingError> {
        let count = self
            .owned
            .entry(node.arena)
            .or_default()
            .entry(node.node)
            .or_default();
        *count = count
            .checked_add(1)
            .ok_or(V8BindingError::Allocation("traced native lease overflow"))?;
        Ok(())
    }
    pub(super) fn add(&mut self, dom: &SharedDom, node: NodeId) -> Result<(), V8BindingError> {
        self.add_ref(oryn_dom::NodeRef {
            arena: dom.borrow().arena().id(),
            node,
        })
    }
    pub(super) fn add_lease(
        &mut self,
        lease: &Rc<oryn_dom::TrackedNodeLease>,
    ) -> Result<(), V8BindingError> {
        if self.leases.insert(Rc::as_ptr(lease) as usize) {
            let (dom, node) = lease.current();
            self.add(&dom, node)?;
        }
        Ok(())
    }
    pub(super) fn capture(states: &[Rc<V8BindingState>]) -> Result<Self, V8BindingError> {
        let mut roots = Self::default();
        let mut ranges = BTreeSet::new();
        let mut statics = BTreeSet::new();
        let mut iterators = BTreeSet::new();
        let mut owners = BTreeSet::new();
        for state in states {
            if !owners.insert(Rc::as_ptr(state) as usize) {
                continue;
            }
            let arena = state.dom.borrow().arena().id();
            if !state.execution_retired() {
                roots
                    .live_documents
                    .entry(arena)
                    .or_default()
                    .insert(state.document_root());
            }
            // Frame states can share the same range store. Native identities,
            // not the number of owners or live wrapper handles, define custody.
            for id in state.ranges.borrow().keys() {
                if state.static_ranges.borrow().contains(id) || !ranges.insert((arena, *id)) {
                    continue;
                }
                let range = state.dom.borrow().live_range(LiveRangeId(*id))?;
                roots.add_ref(oryn_dom::NodeRef {
                    arena,
                    node: range.start.container,
                })?;
                roots.add_ref(oryn_dom::NodeRef {
                    arena,
                    node: range.end.container,
                })?;
            }
            if statics.insert(Rc::as_ptr(&state.static_range_custody) as usize) {
                static_range_custody::owned_native_roots(state, &mut roots)?;
            }
            node_lists::owned_native_roots(state, &mut roots)?;
            node_iterators::owned_native_roots(state, &mut roots, &mut iterators)?;
            text_tracks::owned_native_roots(state, &mut roots)?;
            for record in state.platform_objects.borrow().values() {
                match record {
                    PlatformObjectRecord::SubmitButtonBehavior(record) => {
                        if let Some(lease) = submit_behavior::owned_native_root(&record.borrow()) {
                            roots.add_lease(&lease)?;
                        }
                    }
                    PlatformObjectRecord::MediaTrackList { _root, .. } => {
                        roots.add_lease(&_root.native_lease())?
                    }
                    _ => {}
                }
            }
            if let Some(lease) = state.selected_text_control.borrow().as_ref() {
                let (dom, node) = lease.current();
                roots.add(&dom, node)?;
            }
        }
        Ok(roots)
    }
    /// Validate the whole ledger once, including entries without a registry.
    /// Never hide an ownership bug by saturating subtraction.
    pub(super) fn validate(&self, dom: &DomRuntime) -> Result<(), V8BindingError> {
        if let Some(nodes) = self.owned.get(&dom.arena().id()) {
            for (node, owned) in nodes {
                if *owned > dom.native_root_count(*node)? {
                    return Err(V8BindingError::Allocation(
                        "traced native leases exceed native custody",
                    ));
                }
            }
        }
        Ok(())
    }
    pub(super) fn independent(
        &self,
        dom: &DomRuntime,
        node: NodeId,
    ) -> Result<bool, V8BindingError> {
        let arena = dom.arena().id();
        let actual = dom.native_root_count(node)?;
        let owned = self
            .owned
            .get(&arena)
            .and_then(|nodes| nodes.get(&node))
            .copied()
            .unwrap_or(0);
        if owned > actual {
            return Err(V8BindingError::Allocation(
                "traced native leases exceed native custody",
            ));
        }
        Ok(actual > owned
            || self
                .live_documents
                .get(&arena)
                .is_some_and(|nodes| nodes.contains(&node)))
    }
}

// A private holder makes owner replacement allocation-free after the native
// Range migration commits. The holder itself belongs to V8's traced graph.
fn owner_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, "oryn.range.owner")
        .ok_or(V8BindingError::Allocation("Range owner key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}
fn range_endpoints<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    range: &RangeRecord,
) -> Result<[v8::Local<'s, v8::Object>; 2], V8BindingError> {
    let owner = scope
        .get_slot::<BindingStateRegistry>()
        .and_then(|registry| {
            registry
                .instances
                .borrow()
                .get(&state.instance_id)
                .and_then(RcWeak::upgrade)
        })
        .ok_or(V8BindingError::Allocation("Range endpoint owner"))?;
    let start = wrap_node(scope, &owner, range.start_container)?;
    let end = if range.end_container == range.start_container {
        start
    } else {
        wrap_node(scope, &owner, range.end_container)?
    };
    Ok([start, end])
}
pub(super) fn initialize_range_owner(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    id: u64,
) -> Result<(), V8BindingError> {
    let owner = state
        .global_object
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("Range owner Window"))?;
    let range: RangeRecord = state.dom.borrow().live_range(LiveRangeId(id))?.into();
    let ends = range_endpoints(scope, state, &range)?;
    let template = v8::ObjectTemplate::new(scope);
    template.set_internal_field_count(3);
    let holder = template
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("Range owner holder"))?;
    holder.set_internal_field(0, owner.into());
    holder.set_internal_field(1, ends[0].into());
    holder.set_internal_field(2, ends[1].into());
    let key = owner_key(scope)?;
    if object.set_private(scope, key, holder.into()) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(())
}
pub(super) struct RangeOwner<'s> {
    holder: v8::Local<'s, v8::Object>,
    window: v8::Local<'s, v8::Object>,
    ends: [v8::Local<'s, v8::Object>; 2],
}
impl RangeOwner<'_> {
    pub(super) fn publish(self) {
        self.holder.set_internal_field(0, self.window.into());
        self.holder.set_internal_field(1, self.ends[0].into());
        self.holder.set_internal_field(2, self.ends[1].into());
    }
}
pub(super) fn prepare_range_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    range: &RangeRecord,
) -> Result<RangeOwner<'s>, V8BindingError> {
    let key = owner_key(scope)?;
    let holder = object
        .get_private(scope, key)
        .and_then(|value| value.try_cast::<v8::Object>().ok())
        .ok_or(V8BindingError::Allocation("Range owner holder"))?;
    if holder.internal_field_count() != 3 {
        return Err(V8BindingError::Allocation("Range owner holder shape"));
    }
    let window = state
        .global_object
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("Range owner Window"))?;
    let ends = range_endpoints(scope, state, range)?;
    Ok(RangeOwner {
        holder,
        window,
        ends,
    })
}
/// Native tree mutations can retarget live boundaries without a Range setter.
/// Publish all new endpoints before releasing any registry Global. Cache each
/// endpoint wrapper once for the entire pass across shared range stores.
pub(super) fn refresh_range_endpoints(
    scope: &mut v8::PinScope,
    states: &[Rc<V8BindingState>],
) -> Result<(), V8BindingError> {
    let mut wrappers = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for state in states {
        let revision = state.dom.borrow().range_endpoint_revision();
        if revision.is_some() && state.traced_range_revision.get() == revision {
            continue;
        }
        let arena = state.dom.borrow().arena().id();
        let mut objects = Vec::new();
        for (id, record) in state.range_objects.borrow().iter() {
            if state.static_ranges.borrow().contains(id) {
                continue;
            }
            let range = state.dom.borrow().live_range(LiveRangeId(*id))?;
            let endpoints = [range.start.container, range.end.container]
                .map(|node| oryn_dom::NodeRef { arena, node });
            // Offsets do not affect the traced graph. Native tree mutations
            // and cross-arena adoption still compare the actual endpoints.
            if record.traced_endpoints.get() == Some(endpoints) {
                continue;
            }
            if let Some(object) = record.handle.to_local(scope) {
                objects.push((*id, object, endpoints));
            }
        }
        for (id, object, endpoints) in objects {
            if !seen.insert((arena, id)) {
                continue;
            }
            let key = owner_key(scope)?;
            let holder = object
                .get_private(scope, key)
                .and_then(|value| value.try_cast::<v8::Object>().ok())
                .ok_or(V8BindingError::Allocation("Range owner holder"))?;
            if holder.internal_field_count() != 3 {
                return Err(V8BindingError::Allocation("Range owner holder shape"));
            }
            for (index, endpoint) in endpoints.into_iter().enumerate() {
                let node = endpoint.node;
                let wrapper = if let Some(wrapper) = wrappers.get(&(arena, node)) {
                    *wrapper
                } else {
                    let wrapper = wrap_node(scope, state, node)?;
                    wrappers.insert((arena, node), wrapper);
                    wrapper
                };
                holder.set_internal_field(index + 1, wrapper.into());
            }
            if let Some(record) = state.range_objects.borrow().get(&id) {
                record.traced_endpoints.set(Some(endpoints));
            }
        }
        state.traced_range_revision.set(revision);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
        let code = v8::String::new(scope, code).unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn exact_multiplicity_preserves_an_extra_native_lease_and_rejects_underflow() {
        let mut dom = oryn_dom::html::parse("<body><i></i></body>")
            .into_runtime()
            .unwrap();
        let body = dom.query_selector(dom.document(), "body").unwrap().unwrap();
        let point = BoundaryPoint {
            container: body,
            offset: 0,
        };
        let id = dom.create_live_range(point, point).unwrap();
        assert_eq!(dom.native_root_count(body).unwrap(), 2);
        let mut roots = Snapshot::default();
        for _ in 0..2 {
            roots
                .add_ref(oryn_dom::NodeRef {
                    arena: dom.arena().id(),
                    node: body,
                })
                .unwrap();
        }
        roots.validate(&dom).unwrap();
        assert!(!roots.independent(&dom, body).unwrap());
        dom.retain_native(body).unwrap();
        assert!(roots.independent(&dom, body).unwrap());
        dom.release_native(body).unwrap();
        assert!(!roots.independent(&dom, body).unwrap());
        roots
            .live_documents
            .entry(dom.arena().id())
            .or_default()
            .insert(dom.document());
        assert!(
            roots.independent(&dom, dom.document()).unwrap(),
            "live execution owns its Document without a native lease"
        );
        roots
            .add_ref(oryn_dom::NodeRef {
                arena: dom.arena().id(),
                node: body,
            })
            .unwrap();
        assert!(matches!(
            roots.validate(&dom),
            Err(V8BindingError::Allocation(
                "traced native leases exceed native custody"
            ))
        ));
        dom.drop_live_range(id).unwrap();
        assert_eq!(dom.native_root_count(body).unwrap(), 0);
    }
    #[test]
    fn retired_graph_preserves_original_constructor_and_extra_endpoint_custody_then_collects() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (context, state, document, lease) = {
            v8::scope!(let handles,&mut isolate);
            let context = v8::Context::new(handles, Default::default());
            let scope = &mut v8::ContextScope::new(handles, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse(
                    "<head><style>body{color:red}</style></head><body><x-kept></x-kept></body>",
                )
                .into_runtime()
                .unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom, RealmId(901), 1).unwrap();
            let document=eval(scope,r#"(()=>{
                const Original=HTMLSubmitButtonBehavior;globalThis.HTMLSubmitButtonBehavior=function(){throw Error('replacement')};
                document.makeBehavior=()=>new Original();document.rejectSubclass=()=>{try{new(class extends Original{})();return false}catch(e){return e instanceof TypeError}};
                if(!(document.makeBehavior() instanceof Original)||!document.rejectSubclass())throw Error('constructor identity');
                customElements.define('x-kept',class extends HTMLElement{});
                const range=document.createRange();range.selectNodeContents(document.body);range.collapse(true);
                document.held=[range,new StaticRange({startContainer:document.body,startOffset:0,endContainer:document.body,endOffset:0}),document.body.childNodes,document.createNodeIterator(document.body)];
                document.savedSheet=document.styleSheets[0];
                const media=document.createElement('video'),trackNode=document.createElement('track');media.append(trackNode);document.body.append(media);
                const textTrack=trackNode.track;document.mediaHeld=[textTrack,media.audioTracks,media.videoTracks];media.remove();
                return document;
            })()"#).try_cast::<v8::Object>().unwrap();
            let body = state
                .dom
                .borrow()
                .query_selector(state.document_root(), "body")
                .unwrap()
                .unwrap();
            // Complete the host checkpoint for the removed media subtree.
            // The explicit lease below is then the sole independent custody.
            {
                let mut dom = state.dom.borrow_mut();
                let mutations = dom.drain_mutations();
                dom.release_mutation_roots(&mutations).unwrap();
            }
            let lease = oryn_dom::TrackedNodeLease::new(&state.dom, body).unwrap();
            state.retire_execution();
            state.refresh_connected_wrapper_roots(scope);
            let roots = Snapshot::capture(&[state.clone(), state.clone()]).unwrap();
            roots.validate(&state.dom.borrow()).unwrap();
            assert!(roots.independent(&state.dom.borrow(), body).unwrap());
            (
                v8::Weak::new(scope, context),
                state,
                v8::Global::new(scope, document),
                lease,
            )
        };
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let handles,&mut isolate);
            let context = context.to_local(handles).unwrap();
            let scope = &mut v8::ContextScope::new(handles, context);
            assert!(eval(scope,"document.makeBehavior().disabled===false && document.rejectSubclass() && document.held[0].startContainer===document.body && document.held[1].endContainer===document.body && document.held[2][0]===document.body.firstChild && document.held[3].root===document.body && document.savedSheet.cssRules.length===1 && document.mediaHeld[0].kind==='subtitles' && document.mediaHeld[1].length===0 && document.mediaHeld[2].length===0").is_true());
        }
        drop(document);
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let handles,&mut isolate);
            assert!(
                context.to_local(handles).is_some(),
                "independent native endpoint owner preserves its registry and realm"
            );
        }
        drop(lease);
        {
            v8::scope!(let handles,&mut isolate);
            let context = context.to_local(handles).unwrap();
            let scope = &mut v8::ContextScope::new(handles, context);
            state.refresh_connected_wrapper_roots(scope);
            let body = state
                .dom
                .borrow()
                .query_selector(state.document_root(), "body")
                .unwrap()
                .unwrap();
            let roots = Snapshot::capture(std::slice::from_ref(&state)).unwrap();
            assert!(!roots.independent(&state.dom.borrow(), body).unwrap());
        }
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        v8::scope!(let handles,&mut isolate);
        assert!(
            context.to_local(handles).is_none(),
            "only collectable JS-owned native leases remain"
        );
    }
    #[test]
    fn cached_range_endpoints_follow_native_tree_removal_and_direct_setters() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=old>text</div><p id=next>next</p></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(906), 1).unwrap();
        eval(
            scope,
            "globalThis.r=document.createRange();r.selectNodeContents(document.body);r.collapse(true)",
        );
        refresh_range_endpoints(scope, std::slice::from_ref(&state)).unwrap();
        refresh_range_endpoints(scope, std::slice::from_ref(&state)).unwrap();
        // A setter publishes different endpoints, then native removal returns
        // to the previously cached body before the next explicit refresh.
        eval(
            scope,
            "r.selectNodeContents(document.getElementById('old'));r.collapse(true)",
        );
        let old = state
            .dom
            .borrow()
            .query_selector(state.document_root(), "#old")
            .unwrap()
            .unwrap();
        state.dom.borrow_mut().remove(old).unwrap();
        refresh_range_endpoints(scope, std::slice::from_ref(&state)).unwrap();
        assert!(
            eval(
                scope,
                "r.startContainer===document.body&&r.endContainer===document.body"
            )
            .is_true()
        );
        let object = eval(scope, "r").try_cast::<v8::Object>().unwrap();
        let key = owner_key(scope).unwrap();
        let holder = object
            .get_private(scope, key)
            .unwrap()
            .try_cast::<v8::Object>()
            .unwrap();
        let body = eval(scope, "document.body");
        for index in [1, 2] {
            assert!(
                holder
                    .get_internal_field(scope, index)
                    .unwrap()
                    .try_cast::<v8::Value>()
                    .unwrap()
                    .strict_equals(body)
            );
        }
        eval(
            scope,
            "r.selectNodeContents(document.getElementById('next'));r.collapse(false)",
        );
        refresh_range_endpoints(scope, std::slice::from_ref(&state)).unwrap();
        let range = eval(scope, "r").try_cast::<v8::Object>().unwrap();
        let key = owner_key(scope).unwrap();
        let holder = range
            .get_private(scope, key)
            .unwrap()
            .try_cast::<v8::Object>()
            .unwrap();
        let next = eval(scope, "document.getElementById('next')");
        assert!(
            holder
                .get_internal_field(scope, 1)
                .unwrap()
                .try_cast::<v8::Value>()
                .unwrap()
                .strict_equals(next)
        );
        assert!(
            holder
                .get_internal_field(scope, 2)
                .unwrap()
                .try_cast::<v8::Value>()
                .unwrap()
                .strict_equals(next)
        );
    }

    #[test]
    fn native_range_set_without_tree_mutation_refreshes_traced_endpoints() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><p>next</p>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(907), 1).unwrap();
        let object = eval(
            scope,
            "globalThis.r=document.createRange();r.selectNodeContents(document.body);r",
        )
        .try_cast::<v8::Object>()
        .unwrap();
        refresh_range_endpoints(scope, std::slice::from_ref(&state)).unwrap();
        let id = *state.range_objects.borrow().keys().next().unwrap();
        let node = state
            .dom
            .borrow()
            .query_selector(state.document_root(), "p")
            .unwrap()
            .unwrap();
        let cursor = state.dom.borrow().mutation_cursor();
        let point = BoundaryPoint {
            container: node,
            offset: 0,
        };
        state
            .dom
            .borrow_mut()
            .set_live_range(LiveRangeId(id), point, point)
            .unwrap();
        assert_eq!(state.dom.borrow().mutation_cursor(), cursor);
        refresh_range_endpoints(scope, std::slice::from_ref(&state)).unwrap();
        let key = owner_key(scope).unwrap();
        let holder = object
            .get_private(scope, key)
            .unwrap()
            .try_cast::<v8::Object>()
            .unwrap();
        let expected = eval(scope, "document.querySelector('p')");
        for index in [1, 2] {
            assert!(
                holder
                    .get_internal_field(scope, index)
                    .unwrap()
                    .try_cast::<v8::Value>()
                    .unwrap()
                    .strict_equals(expected)
            );
        }
    }
    #[test]
    fn detached_scoped_range_endpoint_keeps_identity_until_its_last_traced_owner_dies() {
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
        let state = install_generated_bindings(scope, context, dom, RealmId(905), 1).unwrap();
        let (endpoint, registry) = {
            v8::scope!(let temporary,scope);
            let values=eval(temporary,r#"(()=>{
            const registry=new CustomElementRegistry();registry.define('x-range-kept',class extends HTMLElement{});
            const endpoint=document.createElement('x-range-kept',{customElementRegistry:registry});endpoint.marker=137;
            globalThis.retainedRange=document.createRange();retainedRange.selectNodeContents(document.body);retainedRange.setStart(endpoint,0);retainedRange.collapse(true);
            return [endpoint,registry];
        })()"#).try_cast::<v8::Array>().unwrap();
            let endpoint = values
                .get_index(temporary, 0)
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap();
            let registry = values
                .get_index(temporary, 1)
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap();
            (
                v8::Weak::new(temporary, endpoint),
                v8::Weak::new(temporary, registry),
            )
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        {
            v8::scope!(let temporary,scope);
            let object = endpoint.to_local(temporary).unwrap();
            let range = eval(temporary, "retainedRange.startContainer");
            assert!(range.strict_equals(object.into()));
            assert!(eval(temporary,"retainedRange.startContainer.marker===137 && retainedRange.startContainer.customElementRegistry.get('x-range-kept')!==undefined").is_true());
            assert!(registry.to_local(temporary).is_some());
        }
        {
            v8::scope!(let temporary,scope);
            assert!(eval(temporary, "delete globalThis.retainedRange").is_true());
        }
        scope.low_memory_notification();
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        custom_elements::refresh_native_registry_roots(scope).unwrap();
        scope.low_memory_notification();
        assert!(endpoint.to_local(scope).is_none());
        assert!(registry.to_local(scope).is_none());
    }
    #[test]
    fn cross_realm_range_rebinding_traces_destination_until_the_range_is_released() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (first, second, a, b, range) = {
            v8::scope!(let handles,&mut isolate);
            let first = v8::Context::new(handles, Default::default());
            let second = v8::Context::new(handles, Default::default());
            let token = v8::String::new(handles, "range-retention").unwrap();
            first.set_security_token(token.into());
            second.set_security_token(token.into());
            let a = {
                let scope = &mut v8::ContextScope::new(handles, first);
                let dom = Rc::new(RefCell::new(
                    oryn_dom::html::parse("<body>source</body>")
                        .into_runtime()
                        .unwrap(),
                ));
                install_generated_bindings(scope, first, dom, RealmId(902), 1).unwrap()
            };
            let b = {
                let scope = &mut v8::ContextScope::new(handles, second);
                let dom = Rc::new(RefCell::new(
                    oryn_dom::html::parse("<body>destination</body>")
                        .into_runtime()
                        .unwrap(),
                ));
                install_generated_bindings(scope, second, dom, RealmId(903), 1).unwrap()
            };
            let scope = &mut v8::ContextScope::new(handles, first);
            let target = wrap_node(scope, &b, b.document_root()).unwrap();
            {
                let global = first.global(scope);
                set_object_property(scope, global, "destination", target.into())
            }
            .unwrap();
            let range=eval(scope,"(()=>{const range=document.createRange();range.selectNodeContents(document.body);range.selectNodeContents(destination.body);delete globalThis.destination;return range})()").try_cast::<v8::Object>().unwrap();
            let (owner, id) = range_boundaries::owner(scope, range).unwrap();
            assert!(Rc::ptr_eq(&owner, &b));
            assert_eq!(
                b.dom
                    .borrow()
                    .live_range(LiveRangeId(id))
                    .unwrap()
                    .start
                    .container,
                b.dom
                    .borrow()
                    .query_selector(b.document_root(), "body")
                    .unwrap()
                    .unwrap()
            );
            b.retire_execution();
            b.refresh_connected_wrapper_roots(scope);
            a.retire_execution();
            a.refresh_connected_wrapper_roots(scope);
            (
                v8::Weak::new(scope, first),
                v8::Weak::new(scope, second),
                a,
                b,
                v8::Global::new(scope, range),
            )
        };
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let handles,&mut isolate);
            let context = first.to_local(handles).unwrap();
            assert!(second.to_local(handles).is_some());
            let scope = &mut v8::ContextScope::new(handles, context);
            let range = v8::Local::new(scope, &range);
            let (owner, id) = range_boundaries::owner(scope, range).unwrap();
            assert!(Rc::ptr_eq(&owner, &b));
            assert_eq!(
                owner
                    .dom
                    .borrow()
                    .arena()
                    .text_content(
                        owner
                            .dom
                            .borrow()
                            .live_range(LiveRangeId(id))
                            .unwrap()
                            .start
                            .container
                    )
                    .unwrap(),
                "destination"
            );
        }
        drop(range);
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        v8::scope!(let handles,&mut isolate);
        assert!(first.to_local(handles).is_none());
        assert!(second.to_local(handles).is_none());
        drop((a, b));
    }
}
