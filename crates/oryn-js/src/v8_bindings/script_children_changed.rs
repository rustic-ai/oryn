//! Direct script children-changed preparation shares the retained insertion queue.
use super::*;
pub(super) fn is_script_parent(
    state: &V8BindingState,
    node: NodeId,
) -> Result<bool, V8BindingError> {
    let dom = state.dom.borrow();
    Ok(is_script_element(&dom, node)
        && dom.arena().get(node)?.script_parser_document.is_none()
        && dom.is_connected(node)?
        && !dom.script_already_started(node)?)
}
pub(super) struct Prepared(Option<post_connection_scripts::PreparedInsertion>);
impl Prepared {
    pub(super) fn poll_retained(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<bool, V8BindingError> {
        match self.0.as_mut() {
            Some(prepared) => prepared.poll_retained(scope),
            None => Ok(true),
        }
    }
    pub(super) fn finish_result<T>(
        self,
        scope: &mut v8::PinScope,
        result: Result<T, V8BindingError>,
    ) -> Result<T, V8BindingError> {
        match self.0 {
            Some(prepared) => prepared.finish_result(scope, result),
            None => result,
        }
    }
    pub(super) fn finish_mutation<T>(
        self,
        scope: &mut v8::PinScope,
        result: Result<T, DomError>,
    ) -> Result<T, V8BindingError> {
        match self.0 {
            Some(prepared) => prepared.finish_mutation(scope, result),
            None => result.map_err(Into::into),
        }
    }
}
/// CharacterData invokes its parent's children-changed steps only for Text
/// (including CDATA), not Comment/PI data and not arbitrary text descendants.
pub(super) fn character_data(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<Prepared, V8BindingError> {
    let parent = {
        let dom = state.dom.borrow();
        let record = dom.arena().get(node)?;
        if matches!(
            record.kind,
            NodeKind::Text { .. } | NodeKind::CDataSection { .. }
        ) {
            record.parent
        } else {
            None
        }
    };
    parent_change(scope, state, parent)
}
pub(super) fn parent_change(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    parent: Option<NodeId>,
) -> Result<Prepared, V8BindingError> {
    Ok(Prepared(match parent {
        Some(parent) if is_script_parent(state, parent)? => Some(
            post_connection_scripts::capture_replacement(scope, state, parent, &[])?,
        ),
        _ => None,
    }))
}

/// Execute normalize's observable mutation phases without holding a DOM borrow
/// across a connected script's children-changed preparation.
pub(super) fn normalize(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    let root_wrapper = wrap_node(scope, state, root)?;
    let nodes = {
        let dom = state.dom.borrow();
        let mut pending = dom
            .arena()
            .get(root)?
            .children
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        let mut nodes = Vec::new();
        while let Some(node) = pending.pop() {
            let record = dom.arena().get(node)?;
            if matches!(record.kind, NodeKind::Text { .. }) {
                nodes.push(node);
            } else if !matches!(record.kind, NodeKind::ShadowRoot { .. }) {
                pending.extend(record.children.iter().rev().copied());
            }
        }
        nodes
    };
    let nodes = nodes
        .into_iter()
        .map(|node| wrap_node(scope, state, node).map(|node| v8::Global::new(scope, node)))
        .collect::<Result<Vec<_>, _>>()?;
    for original in nodes {
        let object = v8::Local::new(scope, original);
        let (owner, node) =
            node_identity::argument(scope, object.into())?.ok_or(DomError::StaleNode(root))?;
        let (root_owner, root) = node_identity::argument(scope, root_wrapper.into())?
            .ok_or(DomError::StaleNode(root))?;
        if !Rc::ptr_eq(&owner.dom, &root_owner.dom)
            || !owner.dom.borrow().arena().ancestors(node)?.contains(&root)
        {
            continue;
        }
        let (length, parent, data) = {
            let dom = owner.dom.borrow();
            let record = dom.arena().get(node)?;
            let NodeKind::Text { data } = &record.kind else {
                continue;
            };
            let length = oryn_dom::dom_string::len(data) as u32;
            let Some(parent) = record.parent else {
                continue;
            };
            let siblings = light_children(&dom, parent)?;
            let index = siblings
                .iter()
                .position(|child| *child == node)
                .ok_or(DomError::NotAChild(node, parent))?;
            let mut data = Vec::new();
            for sibling in &siblings[index + 1..] {
                let record = dom.arena().get(*sibling)?;
                let NodeKind::Text { data: text } = &record.kind else {
                    break;
                };
                data.extend(oryn_dom::dom_string::units(text));
            }
            (length, parent, oryn_dom::dom_string::from_units(&data))
        };
        let preparation = parent_change(scope, &owner, Some(parent))?;
        let result = if length == 0 {
            owner.dom.borrow_mut().remove(node)
        } else {
            owner
                .dom
                .borrow_mut()
                .replace_character_data(node, length, 0, data)
        };
        let result = result
            .map_err(V8BindingError::from)
            .and_then(|_| synchronize_live_ranges(&owner));
        preparation.finish_result(scope, result)?;
        if length == 0 {
            continue;
        }
        let (owner, node) =
            node_identity::argument(scope, object.into())?.ok_or(DomError::StaleNode(node))?;
        let siblings = owner
            .dom
            .borrow_mut()
            .normalize_text_boundaries(node, length)?;
        synchronize_live_ranges(&owner)?;
        let siblings = siblings
            .into_iter()
            .map(|node| wrap_node(scope, &owner, node).map(|node| v8::Global::new(scope, node)))
            .collect::<Result<Vec<_>, _>>()?;
        for sibling in siblings {
            let sibling = v8::Local::new(scope, sibling);
            let (owner, node) =
                node_identity::argument(scope, sibling.into())?.ok_or(DomError::StaleNode(node))?;
            let parent = owner.dom.borrow().parent(node)?;
            let preparation = parent_change(scope, &owner, parent)?;
            let removed = owner.dom.borrow_mut().remove(node);
            let result = removed
                .map_err(V8BindingError::from)
                .and_then(|_| synchronize_live_ranges(&owner));
            preparation.finish_result(scope, result)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_inserted_children_wait_for_parser_preparation() {
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
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source=v8::String::new(scope,"const parserCandidate=document.createElement('script');document.body.append(parserCandidate);parserCandidate").unwrap();
        let object = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let (_, node) = node_identity::argument(scope, object).unwrap().unwrap();
        state.dom.borrow_mut().mark_parser_script(node).unwrap();
        assert!(!is_script_parent(&state, node).unwrap());
        assert!(
            !state
                .dom
                .borrow_mut()
                .reject_moved_parser_script(node)
                .unwrap()
        );
        assert!(is_script_parent(&state, node).unwrap());
    }
    #[test]
    fn normalize_prepares_between_concatenation_and_sibling_removal() {
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
   const script=document.createElement('script');script.type='text/plain';const source='globalThis.normalizing();',first=document.createTextNode(source),second=document.createTextNode(source);script.append(first,second);document.body.append(script);const range=document.createRange();range.setStart(second,1);range.collapse(true);let calls=0;globalThis.normalizing=()=>{calls++;check(script.childNodes.length===2&&first.data===source+source,'callback before removals');check(range.startContainer===second,'range remapping after callback')};script.type='text/javascript';script.normalize();check(calls===3&&script.childNodes.length===1,'one script preparation on concatenated source');check(range.startContainer===first&&range.startOffset===source.length+1,'range remapping preserved');script.normalize();check(calls===3,'already started');
   const empty=document.createElement('script');empty.type='text/plain';const blank=document.createTextNode('');empty.append(blank,document.createTextNode('globalThis.emptyNormalized++'));document.body.append(empty);globalThis.emptyNormalized=0;empty.type='text/javascript';empty.normalize();check(emptyNormalized===1&&blank.parentNode===null&&empty.childNodes.length===1,'empty removal children change');
   return true;
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
    fn range_insert_prepares_direct_script_parent_and_retains_boundaries() {
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
        let source=v8::String::new(scope,r#"(()=>{globalThis.rangeRuns=0;
   for(const split of [false,true]){const script=document.createElement('script'),range=document.createRange();document.body.append(script);if(split){const text=document.createTextNode('');script.append(text);range.setStart(text,0);}else range.setStart(script,0);range.collapse(true);const before=rangeRuns;range.insertNode(document.createTextNode('rangeRuns++'));if(rangeRuns!==before+1||range.endContainer!==script||range.endOffset!==(split?2:1))return false;}
   const source=document.createElement('script');source.type='text/plain';const text=document.createTextNode('');source.append(document.createTextNode('globalThis.rangeRemoval()'),text);document.body.append(source);source.type='text/javascript';const target=document.createElement('div'),reference=document.createElement('i');target.append(reference);document.body.append(target);const range=document.createRange();range.setStart(target,0);globalThis.rangeRemoval=()=>reference.remove();let failed=false;try{range.insertNode(text)}catch(error){failed=error.name==='NotFoundError'}return failed&&text.parentNode===null&&target.childNodes.length===0;
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
    fn implicit_removal_runs_before_insert_and_revalidates_original_targets() {
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
   const source=()=>{const s=document.createElement('script');s.type='text/plain';const t=document.createTextNode('');s.append(document.createTextNode('globalThis.removing()'),t);document.body.append(s);s.type='text/javascript';return {s,t}};
   for(const method of ['appendChild','append']){const {s,t}=source(),dest=document.createElement('div'),other=document.createElement('div');document.body.append(dest,other);let ran=0;globalThis.removing=()=>{ran++;check(t.parentNode===null&&dest.childNodes.length===0,'removal before destination');other.append(t)};if(method==='append')dest.append(t,'suffix');else dest.appendChild(t);check(ran===1&&t.parentNode===dest,'reparented source inserted once');}
   {const {t}=source(),dest=document.createElement('div'),ref=document.createElement('i');dest.append(ref);document.body.append(dest);globalThis.removing=()=>ref.remove();let failed=false;try{dest.insertBefore(t,ref)}catch(error){failed=error.name==='NotFoundError'}check(failed&&t.parentNode===null&&dest.childNodes.length===0,'invalidated original reference');}
   {const {t}=source(),dest=document.createElement('div'),inert=document.implementation.createHTMLDocument('');document.body.append(dest);globalThis.removing=()=>inert.adoptNode(dest);dest.appendChild(t);check(dest.ownerDocument===inert&&t.ownerDocument===inert&&t.parentNode===dest,'adopted destination identity');}
   {const {s,t}=source();let ran=0;globalThis.removing=()=>ran++;s.insertBefore(t,t);check(ran===1&&t.parentNode===s,'self reference retains next sibling');}
   {const {s,t}=source(),other=document.createElement('div');document.body.append(other);let ran=0;globalThis.removing=()=>{ran++;check(t.parentNode===null,'self replacement removes first');other.append(t)};const returned=s.replaceChild(t,t);check(returned===t&&ran===1&&t.parentNode===s,'self replacement continuation');}
   return true;
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
    fn text_conversion_adoption_prepares_only_the_new_parent_realm() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let (source_state, text) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap();
            let script=v8::String::new(scope,"globalThis.adoptedRuns=0;globalThis.script=document.createElement('script');globalThis.text=document.createTextNode('');script.append(text);document.body.append(script);text").unwrap();
            let text = v8::Script::compile(scope, script, None)
                .unwrap()
                .run(scope)
                .unwrap();
            (state, v8::Global::new(scope, text))
        };
        {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let _destination =
                install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap();
            let key = v8::String::new(scope, "foreign").unwrap();
            let text = v8::Local::new(scope, &text);
            second.global(scope).set(scope, key.into(), text).unwrap();
            let script=v8::String::new(scope,"globalThis.adoptedRuns=0;const destination=document.createElement('script');document.body.append(destination);foreign.data={toString(){destination.appendChild(foreign);return 'globalThis.adoptedRuns++'}};adoptedRuns===1&&foreign.ownerDocument===document&&destination.firstChild===foreign").unwrap();
            assert!(
                v8::Script::compile(scope, script, None)
                    .unwrap()
                    .run(scope)
                    .unwrap()
                    .is_true()
            );
        }
        {
            let scope = &mut v8::ContextScope::new(scope, first);
            let script =
                v8::String::new(scope, "adoptedRuns===0&&script.childNodes.length===0").unwrap();
            assert!(
                v8::Script::compile(scope, script, None)
                    .unwrap()
                    .run(scope)
                    .unwrap()
                    .is_true()
            );
        }
        drop(source_state);
    }
    #[test]
    fn direct_text_mutations_prepare_once_and_ignore_descendant_payload() {
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
        let source=v8::String::new(scope,r#"(()=>{globalThis.runs=0;const code='runs++';
   for(const change of [s=>s.appendChild(document.createTextNode(code)),s=>s.append(code),s=>s.prepend(code),s=>s.insertAdjacentText('beforeend',code),s=>s.textContent=code,s=>s.innerText=code,s=>s.text=code]){const s=document.createElement('script');document.body.append(s);const before=runs;change(s);if(runs!==before+1)return false;s.appendChild(document.createTextNode(';runs++'));if(runs!==before+1)return false;}
   for(const change of [t=>t.data=code,t=>t.nodeValue=code,t=>t.textContent=code,t=>t.appendData(code),t=>t.insertData(0,code),t=>t.replaceData(0,0,code)]){const s=document.createElement('script'),t=document.createTextNode('');s.append(t);document.body.append(s);const before=runs;change(t);if(runs!==before+1)return false;}
   const s=document.createElement('script');document.body.append(s);const child=document.createElement('span');child.textContent='runs+=1000';const before=runs;s.append(child);s.append(document.createComment('runs+=1000'));if(runs!==before)return false;s.append(document.createTextNode(code));if(runs!==before+1)return false;
   const inert=document.createElement('div');inert.innerHTML='<script><'+ '/script>';document.body.append(inert);inert.firstChild.append(document.createTextNode('runs+=1000'));return runs===before+1;
  })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}
