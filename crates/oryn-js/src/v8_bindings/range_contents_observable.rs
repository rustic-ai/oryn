//! Range contents operations release native borrows at each observable mutation.
use super::*;
struct Step {
    node: v8::Global<v8::Object>,
    fully: bool,
    depth: usize,
}
fn owner(
    scope: &mut v8::PinScope,
    node: v8::Local<v8::Object>,
) -> Result<(Rc<V8BindingState>, NodeId), V8BindingError> {
    node_identity::argument(scope, node.into())?
        .ok_or(V8BindingError::Allocation("Range contents node identity"))
}
fn append(
    scope: &mut v8::PinScope,
    output: v8::Local<v8::Object>,
    node: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    let (state, parent) = owner(scope, output)?;
    let insertion =
        node_migration::insertion_argument(scope, &state, parent, node.into(), None, None)?;
    let state = &insertion.owner;
    let preparation =
        post_connection_scripts::capture_insertion(scope, state, insertion.parent, insertion.node)?;
    // Release the mutation borrow before synchronizing ranges or publishing
    // connection effects; both may read this same native DOM.
    let inserted = state
        .dom
        .borrow_mut()
        .insert_dom_node(insertion.parent, insertion.node, None)
        .map_err(V8BindingError::from);
    let result = inserted.and_then(|_| synchronize_live_ranges(state));
    preparation.finish_result(scope, result)
}
fn clone_shell<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let clone = state.dom.borrow_mut().clone_node(node, false)?;
    let (source, target) = {
        let mut dom = state.dom.borrow_mut();
        (
            oryn_dom::TrackedNodeLease::new_in(&state.dom, &mut dom, node)?,
            oryn_dom::TrackedNodeLease::new_in(&state.dom, &mut dom, clone)?,
        )
    };
    state
        .range_clone_completions
        .borrow_mut()
        .push(Rc::new(RangeCloneCompletion {
            source,
            target,
            deep: false,
            background: Cell::new(true),
            stage: Cell::new(0),
        }));
    wrap_node(scope, state, clone)
}
pub(super) fn collect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    range: &RangeRecord,
    mode: RangeContentsMode,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if mode == RangeContentsMode::Clone {
        let prior = state.range_clone_completions.borrow().clone();
        let fragment = range_contents_for_edit(&mut state.dom.borrow_mut(), state, range, mode)?;
        for job in state.range_clone_completions.borrow().iter() {
            if !prior.iter().any(|old| Rc::ptr_eq(old, job)) {
                job.background.set(true);
            }
        }
        let output = wrap_node(scope, state, fragment)?;
        complete_editing_range_clones(scope, state)?;
        return Ok(output);
    }
    let (steps, fragment) = {
        let mut dom = state.dom.borrow_mut();
        validate_boundary_offset(&dom, range.start_container, range.start_offset)?;
        validate_boundary_offset(&dom, range.end_container, range.end_offset)?;
        let common = common_ancestor(&dom, range.start_container, range.end_container)?;
        let collapsed =
            range.start_container == range.end_container && range.start_offset == range.end_offset;
        let same_data = range.start_container == range.end_container
            && matches!(
                dom.arena().get(range.start_container)?.kind,
                NodeKind::Text { .. }
                    | NodeKind::CDataSection { .. }
                    | NodeKind::ProcessingInstruction { .. }
                    | NodeKind::Comment { .. }
            );
        let mut steps = Vec::new();
        if !collapsed && same_data {
            steps.push((range.start_container, false, 0));
        } else if !collapsed {
            let mut pending = light_children(&dom, common)?
                .into_iter()
                .rev()
                .map(|node| (node, 0))
                .collect::<Vec<_>>();
            let mut seen = BTreeSet::new();
            while let Some((node, depth)) = pending.pop() {
                if !seen.insert(node) {
                    return Err(DomError::Cycle.into());
                }
                let before = relative_boundary(&dom, node, false)?;
                let after = relative_boundary(&dom, node, true)?;
                let start = (range.start_container, range.start_offset);
                let end = (range.end_container, range.end_offset);
                if compare_boundary_points(&dom, end, before)? <= 0
                    || compare_boundary_points(&dom, start, after)? >= 0
                {
                    continue;
                }
                let fully = compare_boundary_points(&dom, start, before)? <= 0
                    && compare_boundary_points(&dom, after, end)? <= 0;
                if fully
                    && mode != RangeContentsMode::Delete
                    && matches!(dom.arena().get(node)?.kind, NodeKind::DocumentType { .. })
                {
                    return Err(hierarchy_request_error());
                }
                steps.push((node, fully, depth));
                if !fully {
                    pending.extend(
                        light_children(&dom, node)?
                            .into_iter()
                            .rev()
                            .map(|child| (child, depth + 1)),
                    );
                }
            }
        }
        let document = dom
            .arena()
            .node_document(range.start_container)?
            .ok_or(V8BindingError::Allocation("Range owner document"))?;
        let fragment = dom.create_fragment();
        dom.set_node_document(fragment, document)?;
        (steps, fragment)
    };
    let output = wrap_node(scope, state, fragment)?;
    let steps = steps
        .into_iter()
        .map(|(node, fully, depth)| {
            let object = wrap_node(scope, state, node)?;
            Ok(Step {
                node: v8::Global::new(scope, object),
                fully,
                depth,
            })
        })
        .collect::<Result<Vec<_>, V8BindingError>>()?;
    let start_object = wrap_node(scope, state, range.start_container)?;
    let end_object = wrap_node(scope, state, range.end_container)?;
    let mut outputs = vec![v8::Global::new(scope, output)];
    for step in steps {
        outputs.truncate(step.depth + 1);
        let destination = v8::Local::new(
            scope,
            outputs
                .last()
                .ok_or(V8BindingError::Allocation("Range partial output"))?,
        );
        let object = v8::Local::new(scope, step.node);
        let (current, node) = owner(scope, object)?;
        if step.fully {
            if mode == RangeContentsMode::Extract {
                append(scope, destination, object)?;
            } else {
                let parent = current.dom.borrow().parent(node)?;
                let preparation = script_children_changed::parent_change(scope, &current, parent)?;
                let removed = current
                    .dom
                    .borrow_mut()
                    .remove(node)
                    .map_err(V8BindingError::from);
                let result = removed.and_then(|_| synchronize_live_ranges(&current));
                preparation.finish_result(scope, result)?;
            }
            continue;
        }
        let kind = current.dom.borrow().arena().get(node)?.kind.clone();
        if let NodeKind::Text { data }
        | NodeKind::CDataSection { data }
        | NodeKind::ProcessingInstruction { data, .. }
        | NodeKind::Comment { data } = &kind
        {
            let start = if object.strict_equals(start_object.into()) {
                range.start_offset
            } else {
                0
            };
            let end = if object.strict_equals(end_object.into()) {
                range.end_offset
            } else {
                oryn_dom::dom_string::len(data) as u32
            };
            if mode == RangeContentsMode::Extract {
                let (target, _) = owner(scope, destination)?;
                let selected = create_character_data_like(
                    &mut target.dom.borrow_mut(),
                    &kind,
                    utf16_slice(data, start, end),
                )?;
                let selected = wrap_node(scope, &target, selected)?;
                append(scope, destination, selected)?;
            }
            let preparation = script_children_changed::character_data(scope, &current, node)?;
            let replaced = current
                .dom
                .borrow_mut()
                .replace_character_data(node, start, end.saturating_sub(start), "")
                .map_err(V8BindingError::from);
            let result = replaced.and_then(|_| synchronize_live_ranges(&current));
            preparation.finish_result(scope, result)?;
        } else {
            let shell = if mode == RangeContentsMode::Delete {
                destination
            } else {
                let shell = clone_shell(scope, &current, node)?;
                append(scope, destination, shell)?;
                complete_editing_range_clones(scope, &current)?;
                shell
            };
            outputs.push(v8::Global::new(scope, shell));
        }
    }
    Ok(output)
}

/// Only already-created clone completions survive a failed synchronous Range
/// call. The unperformed mutation suffix is deliberately not queued.
pub(super) fn completion_delay(state: &V8BindingState) -> Option<std::time::Duration> {
    if state.execution_retired()
        || state.execution_tasks_stopped()
        || state.execution_suspended()
        || !state
            .range_clone_completions
            .borrow()
            .iter()
            .any(|job| job.background.get())
    {
        return None;
    }
    Some(
        state
            .range_clone_retry
            .get()
            .map_or(std::time::Duration::ZERO, |deadline| {
                deadline.saturating_duration_since(std::time::Instant::now())
            }),
    )
}
pub(super) fn poll_completions(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if completion_delay(state).is_none_or(|delay| !delay.is_zero())
        || scope.is_execution_terminating()
    {
        return Ok(0);
    }
    let jobs = state
        .range_clone_completions
        .borrow()
        .iter()
        .filter(|job| job.background.get())
        .cloned()
        .collect::<Vec<_>>();
    let mut completed = 0;
    for job in jobs {
        if state.execution_suspended()
            || state.execution_retired()
            || scope.is_execution_terminating()
        {
            break;
        }
        if let Err(error) = complete_range_clone_job(scope, state, &job) {
            state.range_clone_retry.set(Some(
                std::time::Instant::now() + std::time::Duration::from_millis(10),
            ));
            return Err(error);
        }
        completed += 1;
    }
    state.range_clone_retry.set(None);
    Ok(completed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contents_publish_children_changes_before_next_mutation() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let code=v8::String::new(scope,r#"(()=>{const check=(x,m)=>{if(!x)throw Error(m)};
   for(const method of ['deleteContents','extractContents']){
    const script=document.createElement('script');script.type='text/plain';const text=document.createTextNode('globalThis.rangeStep()'),empty=document.createTextNode('');script.append(text,empty);document.body.append(script);let calls=0;globalThis.rangeStep=()=>{calls++;check(empty.parentNode===null,'removal callback precedes fragment append')};script.type='text/javascript';const range=document.createRange();range.selectNode(empty);const fragment=range[method]();check(calls===1&&range.collapsed,'contained removal prepares once');if(fragment)check(fragment.firstChild===empty,'extract preserves identity');
   }
   for(const method of ['deleteContents','extractContents']){
    const script=document.createElement('script');script.type='text/plain';const text=document.createTextNode('xglobalThis.rangeStep()');script.append(text);document.body.append(script);let calls=0;globalThis.rangeStep=()=>{calls++;check(text.data==='globalThis.rangeStep()','partial data already replaced')};script.type='text/javascript';const range=document.createRange();range.setStart(text,0);range.setEnd(text,1);const fragment=range[method]();check(calls===1&&range.collapsed,'partial mutation prepares');if(fragment)check(fragment.textContent==='x','selected data retained');
   }
   {const script=document.createElement('script');script.type='text/plain';const first=document.createTextNode(''),code=document.createTextNode('globalThis.rangeAdopt()'),last=document.createTextNode('');script.append(first,code,last);document.body.append(script);const inert=document.implementation.createHTMLDocument('');globalThis.rangeAdopt=()=>inert.adoptNode(last);script.type='text/javascript';const range=document.createRange();range.selectNodeContents(script);const result=range.extractContents();check(result.childNodes.length===3&&result.lastChild===last&&last.ownerDocument===document,'selected sibling identity after callback adoption');}
   const host=document.createElement('script');host.type='text/plain';host.textContent='globalThis.surrounded++';document.body.append(host);globalThis.surrounded=0;const range=document.createRange();range.selectNodeContents(host);const wrapper=document.createElement('script');range.surroundContents(wrapper);check(surrounded===1&&wrapper.parentNode===host&&range.startContainer===host,'surround insertion children change');return true;
  })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn mutations_release_dom_borrows_before_live_range_refresh() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(super::super::test_eval::eval(scope,r#"(()=>{
   const check=(ok,message)=>{if(!ok)throw Error(message)};
   for(const method of ['deleteContents','extractContents']){
    const host=document.createElement('div');host.innerHTML='<b>ab</b><i>cd</i>';document.body.append(host);
    const first=host.firstChild.firstChild,last=host.lastChild.firstChild,r=document.createRange(),watch=document.createRange();
    watch.selectNodeContents(host);r.setStart(first,1);r.setEnd(last,1);const output=r[method]();
    check(host.textContent==='ad'&&r.collapsed&&r.startContainer===host&&r.startOffset===1,'partial mutation and collapse '+method);
    check(watch.toString()==='ad','independent live range '+method);
    if(output)check(output.textContent==='bc','extracted text');
    r.selectNode(host.firstChild);r.deleteContents();check(host.textContent==='d'&&watch.toString()==='d','full-node deletion');
   }
   const host=document.createElement('div');host.textContent='abcd';document.body.append(host);
   const r=document.createRange(),watch=document.createRange();watch.selectNodeContents(host);r.setStart(host.firstChild,1);r.setEnd(host.firstChild,3);
   const wrapper=document.createElement('b');wrapper.textContent='discard';r.surroundContents(wrapper);
   check(host.textContent==='abcd'&&wrapper.textContent==='bc'&&r.toString()==='bc'&&watch.toString()==='abcd','surround replacement and insertion');return true;
  })()"#).is_true());
    }
    #[test]
    fn failed_direct_clone_drains_completion_only_and_respects_suspension() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=host><span>original</span></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        FAIL_RANGE_CLONE_COMPLETION.with(|flag| flag.set(true));
        let code=v8::String::new(scope,"(()=>{const r=document.createRange();r.selectNodeContents(document.getElementById('host'));try{r.cloneContents();return false}catch(error){return true}})()").unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        assert_eq!(state.range_clone_completions.borrow().len(), 1);
        let retained = state.range_clone_completions.borrow()[0].clone();
        assert!(retained.background.get());
        assert_eq!(retained.stage.get(), 3);
        state.set_execution_suspended(true);
        assert!(completion_delay(&state).is_none());
        assert_eq!(poll_completions(scope, &state).unwrap(), 0);
        assert_eq!(state.range_clone_completions.borrow().len(), 1);
        state.set_execution_suspended(false);
        assert_eq!(poll_completions(scope, &state).unwrap(), 1);
        assert!(state.range_clone_completions.borrow().is_empty());
        assert_eq!(poll_completions(scope, &state).unwrap(), 0);
        let code = v8::String::new(
            scope,
            "document.getElementById('host').textContent==='original'",
        )
        .unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}
