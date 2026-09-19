//! Shadow-aware Selection ordering and frozen endpoint projection.
use super::*;
fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Selection".into(),
        member: name.into(),
    }
}
fn ancestors(dom: &DomRuntime, node: NodeId) -> Result<Vec<NodeId>, V8BindingError> {
    let mut nodes = Vec::new();
    let mut current = Some(node);
    let mut seen = BTreeSet::new();
    while let Some(node) = current {
        if !seen.insert(node) {
            return Err(DomError::Cycle.into());
        }
        nodes.push(node);
        if matches!(dom.arena().get(node)?.kind, NodeKind::Document) {
            break;
        }
        current = dom.parent(node)?;
    }
    nodes.reverse();
    Ok(nodes)
}
fn child_index(dom: &DomRuntime, parent: NodeId, child: NodeId) -> Result<u32, V8BindingError> {
    if matches!(dom.arena().get(child)?.kind, NodeKind::ShadowRoot { .. }) {
        return Ok(0);
    }
    light_children(dom, parent)?
        .iter()
        .position(|node| *node == child)
        .map(|index| index as u32)
        .ok_or_else(|| DomError::NotAChild(child, parent).into())
}
pub(super) fn compare(
    dom: &DomRuntime,
    left: BoundaryPoint,
    right: BoundaryPoint,
) -> Result<i32, V8BindingError> {
    if tree_root(dom, left.container)? == tree_root(dom, right.container)? {
        return compare_boundary_points(
            dom,
            (left.container, left.offset),
            (right.container, right.offset),
        );
    }
    let a = ancestors(dom, left.container)?;
    let b = ancestors(dom, right.container)?;
    if a.first() != b.first() {
        return Err(error("WrongDocumentError"));
    }
    let common = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    if common == a.len() {
        return Ok(
            if left.offset <= child_index(dom, left.container, b[common])? {
                -1
            } else {
                1
            },
        );
    }
    if common == b.len() {
        return Ok(
            if right.offset <= child_index(dom, right.container, a[common])? {
                1
            } else {
                -1
            },
        );
    }
    let parent = a[common - 1];
    let ai = a[common];
    let bi = b[common];
    let a_shadow = matches!(dom.arena().get(ai)?.kind, NodeKind::ShadowRoot { .. });
    let b_shadow = matches!(dom.arena().get(bi)?.kind, NodeKind::ShadowRoot { .. });
    Ok(if a_shadow != b_shadow {
        if a_shadow { -1 } else { 1 }
    } else if child_index(dom, parent, ai)? < child_index(dom, parent, bi)? {
        -1
    } else {
        1
    })
}
fn allowed(
    dom: &DomRuntime,
    root: NodeId,
    roots: &[(Rc<V8BindingState>, NodeId)],
    state: &V8BindingState,
) -> Result<bool, V8BindingError> {
    for (owner, node) in roots {
        if Rc::ptr_eq(&owner.dom, &state.dom) && ancestors(dom, *node)?.contains(&root) {
            return Ok(true);
        }
    }
    Ok(false)
}
fn rescope(
    dom: &DomRuntime,
    mut point: BoundaryPoint,
    end: bool,
    roots: &[(Rc<V8BindingState>, NodeId)],
    state: &V8BindingState,
) -> Result<BoundaryPoint, V8BindingError> {
    loop {
        let root = tree_root(dom, point.container)?;
        if !matches!(dom.arena().get(root)?.kind, NodeKind::ShadowRoot { .. })
            || allowed(dom, root, roots, state)?
        {
            return Ok(point);
        }
        let host = dom
            .parent(root)?
            .ok_or_else(|| error("InvalidStateError"))?;
        let (parent, index) = relative_boundary(dom, host, false)?;
        point = BoundaryPoint {
            container: parent,
            offset: index + u32::from(end),
        };
    }
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    options: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let mut wrappers = Vec::new();
    if !options.is_null_or_undefined() {
        let options = options
            .try_cast::<v8::Object>()
            .map_err(|_| error("TypeError"))?;
        let key = v8::String::new(scope, "shadowRoots")
            .ok_or(V8BindingError::Allocation("composed ranges options"))?;
        let sequence = options
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if !sequence.is_undefined() {
            let sequence = sequence
                .try_cast::<v8::Object>()
                .map_err(|_| error("TypeError"))?;
            let key = v8::Symbol::get_iterator(scope);
            let method = sequence
                .get(scope, key.into())
                .ok_or(V8BindingError::JavaScriptException)?
                .try_cast::<v8::Function>()
                .map_err(|_| error("TypeError"))?;
            iterable_conversion::consume(scope, sequence, method, |scope, value| {
                let (owner, node) =
                    node_identity::argument(scope, value)?.ok_or_else(|| error("TypeError"))?;
                if !matches!(
                    owner.dom.borrow().arena().get(node)?.kind,
                    NodeKind::ShadowRoot { .. }
                ) {
                    return Err(error("TypeError"));
                }
                wrappers
                    .try_reserve(1)
                    .map_err(|_| error("QuotaExceededError"))?;
                wrappers.push(v8::Global::new(scope, value));
                Ok(())
            })?;
        }
    }
    // Option getters and iterator callbacks may move roots or change Selection.
    let mut roots = Vec::new();
    for wrapper in wrappers {
        let value = v8::Local::new(scope, &wrapper);
        roots.push(node_identity::argument(scope, value)?.ok_or_else(|| error("TypeError"))?);
    }
    synchronize_selection_range(state)?;
    let Some(id) = state
        .selection_composed_range
        .get()
        .or(state.selection_range.get())
    else {
        return Ok(v8::Array::new(scope, 0).into());
    };
    let range = state.dom.borrow().live_range(LiveRangeId(id))?;
    let (start, end) = {
        let dom = state.dom.borrow();
        (
            rescope(&dom, range.start, false, &roots, state)?,
            rescope(&dom, range.end, true, &roots, state)?,
        )
    };
    let id = register_static_range(
        state,
        RangeRecord {
            start_container: start.container,
            start_offset: start.offset,
            end_container: end.container,
            end_offset: end.offset,
        },
    )?;
    let object = new_stateful_object(scope, state, "StaticRange", id)?;
    Ok(v8::Array::new_with_elements(scope, &[object.into()]).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_mutation_refreshes_primary_and_composed_records_together() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state =
            install_generated_bindings(scope, context, dom.clone(), RealmId(204), 1).unwrap();
        let source=v8::String::new(scope,"const text=document.body.firstChild.attachShadow({mode:'closed'}).appendChild(document.createTextNode('abcdef'));getSelection().setBaseAndExtent(text,1,text,4);").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let primary = state.selection_range.get().unwrap();
        let companion = state.selection_composed_range.get().unwrap();
        let text = state.selection_anchor.get().unwrap().container;
        // Bypass binding callbacks: native editors and host mutations must be
        // observed even when no global binding range refresh has run.
        dom.borrow_mut()
            .replace_character_data(text, 0, 0, "!")
            .unwrap();
        synchronize_selection_range(&state).unwrap();
        for id in [primary, companion] {
            let records = state.ranges.borrow();
            let range = &records[&id];
            assert_eq!(range.start_offset, 2);
            assert_eq!(range.end_offset, 5);
        }
        assert_eq!(state.selection_anchor.get().unwrap().offset, 2);
        assert_eq!(state.selection_focus.get().unwrap().offset, 5);
    }
    #[test]
    fn unwrapped_companion_is_rooted_only_while_associated() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=host></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state =
            install_generated_bindings(scope, context, dom.clone(), RealmId(203), 1).unwrap();
        let source=v8::String::new(scope,r#"globalThis.shadowText=document.getElementById('host').attachShadow({mode:'closed'}).appendChild(document.createTextNode('abcdef'));getSelection().setBaseAndExtent(shadowText,1,shadowText,4);globalThis.savedPrimary=getSelection().getRangeAt(0);"#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let primary = state.selection_range.get().unwrap();
        let companion = state.selection_composed_range.get().unwrap();
        assert_ne!(primary, companion);
        assert!(!state.range_objects.borrow().contains_key(&companion));
        collect_dead_ranges(&state, true).unwrap();
        assert!(dom.borrow().live_range(LiveRangeId(companion)).is_ok());
        // Replacing Selection must relinquish the old unexposed companion,
        // while the independently retained primary remains a valid Range.
        let source = v8::String::new(
            scope,
            "getSelection().setBaseAndExtent(shadowText,2,shadowText,5);",
        )
        .unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let replacement = state.selection_composed_range.get().unwrap();
        assert_ne!(companion, replacement);
        collect_dead_ranges(&state, true).unwrap();
        assert!(dom.borrow().live_range(LiveRangeId(companion)).is_err());
        assert!(dom.borrow().live_range(LiveRangeId(replacement)).is_ok());
        assert!(dom.borrow().live_range(LiveRangeId(primary)).is_ok());
        clear_selection(&state);
        collect_dead_ranges(&state, true).unwrap();
        assert!(dom.borrow().live_range(LiveRangeId(replacement)).is_err());
        let source=v8::String::new(scope,"if(savedPrimary.startContainer!==shadowText||savedPrimary.startOffset!==1||savedPrimary.endOffset!==4)throw Error('Selection cleanup changed retained Range');").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn revoked_primary_association_clears_cached_endpoints_once() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>selected text</body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(202), 1).unwrap();
        let source=v8::String::new(scope,"globalThis.retainedRange=document.createRange();retainedRange.selectNodeContents(document.body);getSelection().addRange(retainedRange);").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert!(state.selection_anchor.get().is_some());
        let revision = state.selection_revision.get();
        // Cross-arena migration revokes the shared association cells; the
        // per-binding endpoint cache is reconciled on its next observation.
        state.selection_range.set(None);
        state.selection_composed_range.set(None);
        synchronize_selection_range(&state).unwrap();
        assert!(state.selection_anchor.get().is_none());
        assert!(state.selection_focus.get().is_none());
        assert_eq!(state.selection_revision.get(), revision + 1);
        synchronize_selection_range(&state).unwrap();
        assert_eq!(state.selection_revision.get(), revision + 1);
        let source=v8::String::new(scope,r#"(()=>{const s=getSelection();if(s.rangeCount!==0||s.anchorNode!==null||s.focusNode!==null||s.type!=='None'||s.getComposedRanges().length!==0)throw Error('revoked Selection exposes cached endpoints');if(retainedRange.toString()!=='selected text')throw Error('revocation changed script-held Range');})()"#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn option_iteration_adopts_shadow_host_after_native_endpoint_capture() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let a = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><div id=container></div></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(200), 1).unwrap()
        };
        let b = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(201), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let document = wrap_node(scope, &b, b.document_root()).unwrap();
        let key = v8::String::new(scope, "destination").unwrap();
        first
            .global(scope)
            .set(scope, key.into(), document.into())
            .unwrap();
        let source=v8::String::new(scope,r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)},selection=getSelection(),container=document.getElementById('container');container.innerHTML='a<div></div>b';const host=container.children[0],root=host.attachShadow({mode:'closed'});root.textContent='abcdef';const text=root.firstChild;
          selection.setBaseAndExtent(text,2,container,2);const before=selection.getComposedRanges({shadowRoots:[root]})[0];let calls=0;
          const iterable={*[Symbol.iterator](){calls++;destination.body.append(host);yield root}};
          const after=selection.getComposedRanges({shadowRoots:iterable})[0];
          check(calls===1&&after.startContainer===container&&after.startOffset===1&&after.endOffset===1,'native composed endpoints settle before cross-arena host migration');
          check(before.startContainer===text&&text.ownerDocument===destination&&before.endContainer===container&&before.startOffset===2,'frozen mixed-arena endpoints retain originals');
          check(root.host===host&&host.ownerDocument===destination,'option roots refreshed after iterator adoption');
        })()"#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let _ = a;
    }
    #[test]
    fn native_composed_endpoints_rescope_freeze_and_follow_shadow_mutations() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=container></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(199), 1).unwrap();
        let source=v8::String::new(scope,r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)},selection=getSelection(),container=document.getElementById('container');
          selection.removeAllRanges();check(selection.getComposedRanges().length===0,'empty sequence');
          const setup=()=>{container.innerHTML='a<div></div>b';const host=container.children[0],root=host.attachShadow({mode:'closed'});root.textContent='hello, world';return{host,root,text:root.firstChild}};
          let {host,root,text}=setup();selection.setBaseAndExtent(text,7,container,2);
          let ranges=selection.getComposedRanges();check(ranges.length===1&&ranges[0] instanceof StaticRange&&ranges[0].startContainer===container&&ranges[0].startOffset===1&&ranges[0].endOffset===2,'hidden root projection');
          const retained=selection.getComposedRanges({shadowRoots:new Set([root])})[0];check(retained.startContainer===text&&retained.startOffset===7&&retained.endContainer===container,'explicit root visibility');
          check(selection.anchorNode===text&&selection.focusNode===container&&!selection.isCollapsed&&selection.direction==='forward','live composed selection endpoints');
          text.insertData(0,'!');check(selection.getComposedRanges({shadowRoots:[root]})[0].startOffset===8&&retained.startOffset===7,'native offset update and frozen static snapshot');
          host.remove();ranges=selection.getComposedRanges({shadowRoots:[root]});check(ranges[0].startContainer===container&&ranges[0].startOffset===1&&ranges[0].endOffset===1,'host removal adjusts composed endpoints');check(retained.startContainer===text&&retained.startOffset===7,'old snapshot remains rooted');
          ({host,root,text}=setup());selection.setBaseAndExtent(text,4,text,7);host.remove();ranges=selection.getComposedRanges({shadowRoots:[root]});check(ranges[0].startContainer===container&&ranges[0].collapsed,'noncollapsed shadow selection removal');
          ({host,root,text}=setup());selection.collapse(text,2);const original=selection.getRangeAt(0);host.replaceWith(host);check(selection.rangeCount===0&&original.startContainer===text&&original.startOffset===2,'collapsed selection detaches after remove/reinsert while original Range survives');
          ({host,root,text}=setup());root.innerHTML='<span></span>';const innerHost=root.firstChild,inner=innerHost.attachShadow({mode:'closed'});inner.textContent='nested';selection.setBaseAndExtent(inner.firstChild,1,inner.firstChild,5);
          ranges=selection.getComposedRanges({shadowRoots:[root]});check(ranges[0].startContainer===root&&ranges[0].startOffset===0&&ranges[0].endOffset===1,'outer-only capability rescopes inner');
          ranges=selection.getComposedRanges({shadowRoots:[inner]});check(ranges[0].startContainer===inner.firstChild&&ranges[0].endOffset===5,'inner capability includes its ancestor roots');
          selection.setBaseAndExtent(inner.firstChild,1,container,1);check(selection.direction==='backward','cross-shadow direction');
          selection.setBaseAndExtent(inner.firstChild,1,container,2);const projected=selection.getRangeAt(0);projected.selectNodeContents(container);check(selection.getComposedRanges({shadowRoots:[inner]})[0].startContainer===container,'explicit script Range mutation clears composed companion');
          let closed=0,caught=false;const invalid={ [Symbol.iterator](){return {next(){return {done:false,value:host}},return(){closed++;return {}}}}};
          try{selection.getComposedRanges({shadowRoots:invalid})}catch(error){caught=true;check(error instanceof TypeError,'ShadowRoot brand')};check(caught&&closed===1,'IteratorClose on item conversion failure');
          const marker={};caught=false;try{selection.getComposedRanges({get shadowRoots(){throw marker}})}catch(error){caught=true;check(error===marker,'dictionary exception identity')}check(caught,'dictionary getter executes');
          check(selection.getComposedRanges({get shadowRoots(){selection.removeAllRanges();return []}}).length===0,'options getter updates selection before capture');
          caught=false;try{selection.getComposedRanges({shadowRoots:[host]})}catch(error){caught=true;check(error instanceof TypeError,'empty Selection still converts sequence')}check(caught,'empty sequence validation');
        })()"#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
}
