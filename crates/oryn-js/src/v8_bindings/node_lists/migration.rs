//! Collection identities are independent of the arena that currently owns
//! their members. Native migration already transfers their root counters.
type MovingList = (
    u64,
    v8::Global<v8::Object>,
    v8::Global<v8::Symbol>,
    v8::Global<v8::Array>,
);

use super::*;
#[cfg(test)]
thread_local! {static FAIL_PUBLICATION:Cell<bool>=const{Cell::new(false)};}
struct Entry {
    state: Rc<V8BindingState>,
    id: u64,
    updates: Vec<(NodeId, NodeId)>,
    _object: v8::Global<v8::Object>,
    moving: Option<MovingList>,
    published: bool,
}
pub(in crate::v8_bindings) struct Prepared {
    entries: Vec<Entry>,
    ownership_committed: bool,
}
fn root(contents: &Contents) -> Option<NodeId> {
    match contents {
        Contents::StyleSheets(node)
        | Contents::Attributes(node)
        | Contents::Fieldset(node)
        | Contents::Form(node)
        | Contents::All(node)
        | Contents::Children(node)
        | Contents::Labels(node) => Some(*node),
        Contents::Radios { form, .. } => Some(*form),
        Contents::Named { root, .. } | Contents::Elements { root, .. } => Some(*root),
        Contents::Snapshot(_) => None,
    }
}
fn cache<'a>(
    store: &'a Store,
    contents: &Contents,
) -> Option<(&'a RefCell<BTreeMap<NodeId, u64>>, NodeId)> {
    Some(match contents {
        Contents::StyleSheets(node) => (&store.stylesheets, *node),
        Contents::Attributes(node) => (&store.attributes, *node),
        Contents::Children(node) => (&store.children, *node),
        Contents::Labels(node) => (&store.labels, *node),
        Contents::Form(node) => (&store.forms, *node),
        Contents::Fieldset(node) => (&store.fieldsets, *node),
        Contents::All(node) => (&store.all, *node),
        Contents::Elements {
            root,
            filter: ElementFilter::Children,
        } => (&store.element_children, *root),
        Contents::Elements {
            root,
            filter: ElementFilter::TableRows,
        } => (&store.table_rows, *root),
        Contents::Elements {
            root,
            filter: ElementFilter::Options,
        } => (&store.options, *root),
        Contents::Elements {
            root,
            filter: ElementFilter::SelectedOptions,
        } => (&store.selected_options, *root),
        _ => return None,
    })
}
fn remap(contents: &mut Contents, nodes: &BTreeMap<NodeId, NodeId>) {
    match contents {
        Contents::StyleSheets(node)
        | Contents::Attributes(node)
        | Contents::Fieldset(node)
        | Contents::Form(node)
        | Contents::All(node)
        | Contents::Children(node)
        | Contents::Labels(node) => *node = nodes[node],
        Contents::Radios { form, .. } => *form = nodes[form],
        Contents::Named { root, .. } | Contents::Elements { root, .. } => *root = nodes[root],
        Contents::Snapshot(_) => {}
    }
}
/// Call after source-removal MutationRecords have been materialized, and before
/// native commit. Include all isolate binding states to find mixed snapshots.
pub(in crate::v8_bindings) fn capture_migration(
    scope: &mut v8::PinScope,
    states: &[Rc<V8BindingState>],
    source: &SharedDom,
    destination: &Rc<V8BindingState>,
    nodes: &BTreeSet<NodeId>,
) -> Result<Prepared, V8BindingError> {
    let mut entries = Vec::new();
    for state in states {
        // Release dead lists while their old native addresses still exist.
        collect(state)?;
        let lists = state.node_lists.lists.borrow();
        for (id, list) in lists.iter() {
            let moving = root(&list.contents)
                .is_some_and(|root| Rc::ptr_eq(&state.dom, source) && nodes.contains(&root));
            let updates = {
                list.roots
                    .iter()
                    .filter_map(|key| {
                        let (dom, node) = list
                            .foreign
                            .get(key)
                            .map(|reference| (&reference.dom, reference.node))
                            .unwrap_or((&state.dom, *key));
                        (Rc::ptr_eq(dom, source) && nodes.contains(&node)).then_some((*key, node))
                    })
                    .collect::<Vec<_>>()
            };
            if !moving && updates.is_empty() {
                continue;
            }
            let object = list
                .handle
                .to_local(scope)
                .ok_or(V8BindingError::Allocation("pending NodeList root release"))?;
            let keepalive = v8::Global::new(scope, object);
            let moving = if moving {
                let id = destination
                    .node_lists
                    .next
                    .get()
                    .checked_add(1)
                    .filter(|id| *id < (1_u64 << 53))
                    .ok_or(V8BindingError::Allocation("NodeList migration identity"))?;
                destination.node_lists.next.set(id);
                let mut slot = if list.interface == "HTMLAllCollection" {
                    destination.node_lists.all_brand.borrow_mut()
                } else if matches!(
                    list.interface,
                    "HTMLCollection" | "HTMLFormControlsCollection" | "HTMLOptionsCollection"
                ) {
                    destination.node_lists.collection_brand.borrow_mut()
                } else {
                    destination.node_lists.brand.borrow_mut()
                };
                if slot.is_none() {
                    let symbol = v8::Symbol::new(scope, None);
                    *slot = Some(v8::Global::new(scope, symbol));
                }
                let brand = slot.as_ref().unwrap().clone();
                drop(slot);
                // Only moved nodes remain valid cache keys after live-root
                // migration. The owner root keeps the complete new live tree.
                let wrappers = list
                    .wrappers
                    .iter()
                    .filter(|(node, _)| nodes.contains(node))
                    .filter_map(|(_, wrapper)| wrapper.to_local(scope))
                    .map(Into::into)
                    .collect::<Vec<v8::Local<v8::Value>>>();
                let roots = v8::Array::new_with_elements(scope, &wrappers);
                Some((
                    id,
                    v8::Global::new(scope, object),
                    brand,
                    v8::Global::new(scope, roots),
                ))
            } else {
                None
            };
            entries.push(Entry {
                state: state.clone(),
                id: *id,
                updates,
                _object: keepalive,
                moving,
                published: false,
            });
        }
    }
    Ok(Prepared {
        entries,
        ownership_committed: false,
    })
}
impl Prepared {
    /// Call immediately after native commit, before fallible V8 publication.
    /// GC and destructor custody now follow destination addresses even if a
    /// later wrapper publication fails and the transaction must be retried.
    pub(in crate::v8_bindings) fn commit_ownership(
        &mut self,
        destination: &Rc<V8BindingState>,
        nodes: &BTreeMap<NodeId, NodeId>,
    ) -> Result<(), V8BindingError> {
        if self.ownership_committed {
            return Ok(());
        }
        for entry in &self.entries {
            if !entry
                .state
                .node_lists
                .lists
                .borrow()
                .contains_key(&entry.id)
                || entry
                    .updates
                    .iter()
                    .any(|(_, node)| !nodes.contains_key(node))
            {
                return Err(V8BindingError::Allocation(
                    "incomplete collection ownership map",
                ));
            }
        }
        for entry in &self.entries {
            let mut lists = entry.state.node_lists.lists.borrow_mut();
            let list = lists
                .get_mut(&entry.id)
                .expect("prevalidated collection custody");
            for (key, node) in &entry.updates {
                if entry.moving.is_none() && !list.foreign.contains_key(key) {
                    let bytes = std::mem::size_of::<(NodeId, ForeignNode)>() as i64;
                    list.bytes += bytes;
                    entry
                        .state
                        .node_lists
                        .memory_delta
                        .set(entry.state.node_lists.memory_delta.get() + bytes);
                }
                list.foreign.insert(
                    *key,
                    ForeignNode {
                        owner: Rc::downgrade(destination),
                        dom: destination.dom.clone(),
                        node: nodes[node],
                    },
                );
            }
        }
        self.ownership_committed = true;
        Ok(())
    }
    /// Publishing never retains/releases native roots: the native move has
    /// transferred those exact counts. This retained plan supports retry.
    pub(in crate::v8_bindings) fn publish(
        &mut self,
        scope: &mut v8::PinScope,
        destination: &Rc<V8BindingState>,
        nodes: &BTreeMap<NodeId, NodeId>,
    ) -> Result<(), V8BindingError> {
        self.commit_ownership(destination, nodes)?;
        #[cfg(test)]
        if FAIL_PUBLICATION.with(|flag| flag.replace(false)) {
            return Err(V8BindingError::Allocation(
                "injected collection publication failure",
            ));
        }
        // Validate the complete remap before mutating any collection record.
        for entry in self.entries.iter().filter(|entry| !entry.published) {
            let lists = entry.state.node_lists.lists.borrow();
            let list = lists.get(&entry.id).ok_or_else(invalid_receiver)?;
            if entry.moving.is_some()
                && root(&list.contents).is_none_or(|root| !nodes.contains_key(&root))
            {
                return Err(V8BindingError::Allocation(
                    "incomplete live collection migration",
                ));
            }
            if entry
                .updates
                .iter()
                .any(|(_, node)| !nodes.contains_key(node))
            {
                return Err(V8BindingError::Allocation("incomplete snapshot migration"));
            }
        }
        for entry in self.entries.iter_mut().filter(|entry| !entry.published) {
            if let Some((id, object, brand, roots)) = &entry.moving {
                let mut list = entry
                    .state
                    .node_lists
                    .lists
                    .borrow_mut()
                    .remove(&entry.id)
                    .ok_or_else(invalid_receiver)?;
                if let Some((cache, root)) = cache(&entry.state.node_lists, &list.contents)
                    && cache.borrow().get(&root) == Some(&entry.id)
                {
                    cache.borrow_mut().remove(&root);
                }
                remap(&mut list.contents, nodes);
                list.roots = list.roots.iter().map(|node| nodes[node]).collect();
                list.foreign.clear();
                let previous = list.wrappers.len();
                list.wrappers = std::mem::take(&mut list.wrappers)
                    .into_iter()
                    .filter_map(|(node, wrapper)| nodes.get(&node).map(|node| (*node, wrapper)))
                    .collect();
                let released = ((previous - list.wrappers.len())
                    * std::mem::size_of::<v8::Weak<v8::Object>>())
                    as i64;
                let old_bytes = list.bytes;
                list.bytes -= released;
                list.roots_dirty = false;
                let object = v8::Local::new(scope, object);
                let number = v8::Number::new(scope, *id as f64);
                let owner = v8::BigInt::new_from_u64(scope, destination.instance_id);
                let brand = v8::Local::new(scope, brand);
                let roots = v8::Local::new(scope, roots);
                object.set_internal_field(0, number.into());
                object.set_internal_field(1, owner.into());
                object.set_internal_field(2, brand.into());
                object.set_internal_field(3, roots.into());
                if let Some((cache, root)) = cache(&destination.node_lists, &list.contents) {
                    cache.borrow_mut().insert(root, *id);
                }
                entry
                    .state
                    .node_lists
                    .memory_delta
                    .set(entry.state.node_lists.memory_delta.get() - old_bytes);
                destination
                    .node_lists
                    .memory_delta
                    .set(destination.node_lists.memory_delta.get() + list.bytes);
                destination.node_lists.lists.borrow_mut().insert(*id, list);
            }
            entry.published = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_indexed_properties_retain_member_identity_without_duplicate_array() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=member></div>")
                .into_runtime()
                .unwrap(),
        ));
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state =
            install_generated_bindings(scope, context, dom.clone(), RealmId(70), 1).unwrap();
        let node = dom
            .borrow()
            .get_element_by_id(state.document.get(), "member")
            .unwrap()
            .unwrap();
        let (held, weak) = {
            v8::scope!(let scope, scope);
            let member = wrap_node(scope, &state, node).unwrap();
            let key = v8::String::new(scope, "saved").unwrap();
            let value = v8::Integer::new(scope, 42);
            assert_eq!(member.set(scope, key.into(), value.into()), Some(true));
            let list = v8::Local::<v8::Object>::try_from(snapshot(scope, &state, &[node]).unwrap())
                .unwrap();
            (v8::Global::new(scope, list), v8::Weak::new(scope, member))
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        let list = v8::Local::new(scope, &held);
        assert!(
            v8::Local::<v8::Value>::try_from(list.get_internal_field(scope, 3).unwrap())
                .unwrap()
                .is_null()
        );
        let member = weak
            .to_local(scope)
            .expect("indexed property retains member");
        assert!(
            list.get_index(scope, 0)
                .unwrap()
                .strict_equals(member.into())
        );
        let key = v8::String::new(scope, "saved").unwrap();
        assert_eq!(
            member.get(scope, key.into()).unwrap().int32_value(scope),
            Some(42)
        );
    }

    #[test]
    fn live_identity_and_mixed_snapshot_follow_repeated_native_migration() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let source_dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=moved><span>child</span></div><p id=stays></p>")
                .into_runtime()
                .unwrap(),
        ));
        let target_dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=collision><span>unrelated</span></div><p></p>")
                .into_runtime()
                .unwrap(),
        ));
        let source_context = v8::Context::new(scope, Default::default());
        let target_context = v8::Context::new(scope, Default::default());
        let source = {
            let scope = &mut v8::ContextScope::new(scope, source_context);
            install_generated_bindings(scope, source_context, source_dom.clone(), RealmId(70), 1)
                .unwrap()
        };
        let target = {
            let scope = &mut v8::ContextScope::new(scope, target_context);
            install_generated_bindings(scope, target_context, target_dom.clone(), RealmId(71), 1)
                .unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, source_context);
        let root = source_dom
            .borrow()
            .get_element_by_id(source.document.get(), "moved")
            .unwrap()
            .unwrap();
        let stays = source_dom
            .borrow()
            .get_element_by_id(source.document.get(), "stays")
            .unwrap()
            .unwrap();
        let original = wrap_node(scope, &source, root).unwrap();
        let live =
            v8::Local::<v8::Object>::try_from(children(scope, &source, root).unwrap()).unwrap();
        let prototype = live.get_prototype(scope).unwrap();
        let snapshot =
            v8::Local::<v8::Object>::try_from(snapshot(scope, &source, &[root, stays]).unwrap())
                .unwrap();
        let static_id = identity(scope, &source, snapshot).unwrap();
        assert!(
            v8::Local::<v8::Value>::try_from(snapshot.get_internal_field(scope, 3).unwrap())
                .unwrap()
                .is_null()
        );
        assert!(
            snapshot
                .get_index(scope, 0)
                .unwrap()
                .strict_equals(original.into())
        );
        let mut native = DomRuntime::prepare_subtree_migration(
            &source_dom,
            &target_dom,
            root,
            target.document.get(),
        )
        .unwrap();
        let moved = native.plan.nodes.iter().copied().collect::<BTreeSet<_>>();
        let mut lists = capture_migration(
            scope,
            &[source.clone(), target.clone()],
            &source_dom,
            &target,
            &moved,
        )
        .unwrap();
        let mut wrappers = crate::v8_bindings::node_migration::Wrappers::capture(
            scope,
            std::slice::from_ref(&source),
            &target,
            &moved,
        )
        .unwrap();
        let map =
            DomRuntime::commit_subtree_migration(&source_dom, &target_dom, &mut native).unwrap();
        wrappers.publish(scope, &target, &map.nodes).unwrap();
        lists.publish(scope, &target, &map.nodes).unwrap();
        let after = children(scope, &target, map.root).unwrap();
        assert!(after.strict_equals(live.into()));
        assert!(live.get_prototype(scope).unwrap().strict_equals(prototype));
        assert!(
            node_at(scope, &source, static_id, 0)
                .unwrap()
                .unwrap()
                .strict_equals(original.into())
        );
        let members = native_snapshot(scope, &source, snapshot).unwrap().unwrap();
        assert!(
            v8::Local::<v8::Value>::try_from(snapshot.get_internal_field(scope, 3).unwrap())
                .unwrap()
                .is_null()
        );
        assert!(
            snapshot
                .get_index(scope, 0)
                .unwrap()
                .strict_equals(original.into())
        );
        assert!(Rc::ptr_eq(&members[0].0, &target));
        assert_eq!(members[0].1, map.root);
        assert!(Rc::ptr_eq(&members[1].0, &source));
        assert_eq!(members[1].1, stays);
        // A mixed snapshot resides in the original Store while one member moves
        // again. Enumeration must scan all stores and compare arena identity.
        let mut reverse = DomRuntime::prepare_subtree_migration(
            &target_dom,
            &source_dom,
            map.root,
            source.document.get(),
        )
        .unwrap();
        let moved = reverse.plan.nodes.iter().copied().collect::<BTreeSet<_>>();
        let mut lists = capture_migration(
            scope,
            &[source.clone(), target.clone()],
            &target_dom,
            &source,
            &moved,
        )
        .unwrap();
        let mut wrappers = crate::v8_bindings::node_migration::Wrappers::capture(
            scope,
            std::slice::from_ref(&target),
            &source,
            &moved,
        )
        .unwrap();
        let reversed =
            DomRuntime::commit_subtree_migration(&target_dom, &source_dom, &mut reverse).unwrap();
        wrappers.publish(scope, &source, &reversed.nodes).unwrap();
        lists.publish(scope, &source, &reversed.nodes).unwrap();
        assert!(
            children(scope, &source, reversed.root)
                .unwrap()
                .strict_equals(live.into())
        );
        let members = native_snapshot(scope, &source, snapshot).unwrap().unwrap();
        assert!(Rc::ptr_eq(&members[0].0, &source));
        assert_eq!(members[0].1, reversed.root);
        assert_eq!(members[1].1, stays);
        let lists = source.node_lists.lists.borrow();
        let retained = &lists[&static_id];
        assert!(Rc::ptr_eq(&retained.foreign[&root].dom, &source_dom));
        assert_eq!(retained.foreign[&root].node, reversed.root);
        assert!(!retained.foreign.contains_key(&stays));
    }
    #[test]
    fn foreign_root_release_waits_for_borrow_and_never_releases_equal_source_id() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let source_node = source.borrow_mut().create(NodeKind::Text {
            data: "source".into(),
        });
        let target = destination.borrow_mut().create(NodeKind::Text {
            data: "destination".into(),
        });
        assert_eq!(source_node, target);
        source.borrow_mut().retain_native(source_node).unwrap();
        destination.borrow_mut().retain_native(target).unwrap();
        let held = destination.borrow();
        release_root(&destination, target);
        drain_root_releases().unwrap();
        assert!(
            ROOT_RELEASES.with(|pending| pending.borrow().iter().any(|(dom, node)| dom
                .upgrade()
                .is_some_and(|dom| Rc::ptr_eq(&dom, &destination))
                && *node == target))
        );
        drop(held);
        drain_root_releases().unwrap();
        assert!(
            !ROOT_RELEASES.with(|pending| pending.borrow().iter().any(|(dom, _)| dom
                .upgrade()
                .is_some_and(|dom| Rc::ptr_eq(&dom, &destination))))
        );
        assert!(destination.borrow_mut().release_native(target).is_err());
        source.borrow_mut().release_native(source_node).unwrap();
    }

    #[test]
    fn publication_failure_releases_only_committed_destination_roots() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let source_dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=moved><span></span></div>")
                .into_runtime()
                .unwrap(),
        ));
        let destination_dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>unrelated</p>")
                .into_runtime()
                .unwrap(),
        ));
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let source = {
            let scope = &mut v8::ContextScope::new(scope, first);
            install_generated_bindings(scope, first, source_dom.clone(), RealmId(72), 1).unwrap()
        };
        let destination = {
            let scope = &mut v8::ContextScope::new(scope, second);
            install_generated_bindings(scope, second, destination_dom.clone(), RealmId(73), 1)
                .unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let root = source_dom
            .borrow()
            .get_element_by_id(source.document.get(), "moved")
            .unwrap()
            .unwrap();
        let list =
            v8::Local::<v8::Object>::try_from(children(scope, &source, root).unwrap()).unwrap();
        let id = identity(scope, &source, list).unwrap();
        let static_id = register_static_range(
            &source,
            RangeRecord {
                start_container: root,
                start_offset: 0,
                end_container: root,
                end_offset: 0,
            },
        )
        .unwrap();
        let _frozen_object = new_stateful_object(scope, &source, "StaticRange", static_id).unwrap();
        let mut native = DomRuntime::prepare_subtree_migration(
            &source_dom,
            &destination_dom,
            root,
            destination.document.get(),
        )
        .unwrap();
        let nodes = native.plan.nodes.iter().copied().collect::<BTreeSet<_>>();
        let mut prepared = capture_migration(
            scope,
            &[source.clone(), destination.clone()],
            &source_dom,
            &destination,
            &nodes,
        )
        .unwrap();
        let mut frozen = static_range_custody::capture(
            scope,
            &[source.clone(), destination.clone()],
            &source_dom,
            &nodes,
        )
        .unwrap();
        let mut wrappers = node_migration::Wrappers::capture(
            scope,
            std::slice::from_ref(&source),
            &destination,
            &nodes,
        )
        .unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source_dom, &destination_dom, &mut native)
                .unwrap();
        prepared
            .commit_ownership(&destination, &moved.nodes)
            .unwrap();
        frozen.commit_ownership(&destination, &moved.nodes).unwrap();
        wrappers.publish(scope, &destination, &moved.nodes).unwrap();
        FAIL_PUBLICATION.with(|flag| flag.set(true));
        assert!(prepared.publish(scope, &destination, &moved.nodes).is_err());
        assert!(source_dom.borrow().arena().get(root).is_err());
        let record = source.node_lists.lists.borrow_mut().remove(&id).unwrap();
        assert_eq!(record.foreign[&root].node, moved.root);
        assert!(Rc::ptr_eq(&record.foreign[&root].dom, &destination_dom));
        // Simulate terminal transaction disposal/store destruction after the
        // injected V8 publication failure. The production release path sees
        // destination custody even though the list never left its old Store.
        retire_list(record, &source_dom);
        assert!(
            static_range_custody::release_dead(&source, static_id, &mut source_dom.borrow_mut())
                .unwrap()
        );
        source.ranges.borrow_mut().remove(&static_id);
        source.static_ranges.borrow_mut().remove(&static_id);
        source.range_objects.borrow_mut().remove(&static_id);
        drain_root_releases().unwrap();
        assert!(
            !ROOT_RELEASES.with(
                |pending| pending.borrow().iter().any(|(dom, node)| *node == root
                    && dom
                        .upgrade()
                        .is_some_and(|dom| Rc::ptr_eq(&dom, &source_dom)))
            )
        );
    }
}
