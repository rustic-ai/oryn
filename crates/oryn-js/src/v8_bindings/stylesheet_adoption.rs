//! Cross-document stylesheet authority is prepared anew after native adoption.
use super::*;
pub(super) fn publish(
    sources: &[Rc<V8BindingState>],
    destination: &Rc<V8BindingState>,
    nodes: &BTreeMap<NodeId, NodeId>,
) -> Result<(), V8BindingError> {
    // Complete preflight before mutating any publication state. The bridge advances
    // its retained stage only after success, and runs no author code in this stage.
    let mut sheets = Vec::new();
    let mut connected = Vec::new();
    {
        let dom = destination.dom.borrow();
        for node in nodes.values() {
            let record = dom.arena().get(*node)?;
            let sheet = matches!(&record.kind,NodeKind::Element{name} if matches!(name.local.as_ref(),"style"|"link"))
                || matches!(&record.kind,NodeKind::ProcessingInstruction{target,..} if target=="xml-stylesheet");
            if sheet {
                sheets.push(*node);
                if dom.is_connected(*node)? {
                    // reset_adopted_stylesheet advances this exact SSOT
                    // revision with saturating_add(1), with no author callback.
                    connected.push((*node, dom.arena().stylesheet_revision(*node)?.saturating_add(1)));
                }
            }
        }
    }
    if destination
        .pending_dynamic_stylesheets
        .borrow()
        .len()
        .saturating_add(connected.len())
        > 1024
    {
        return Err(V8BindingError::Allocation(
            "adopted stylesheet preparation capacity",
        ));
    }
    for source in sources {
        stylesheet_ownership::detach_adopted(source, nodes)?;
        source
            .inline_style_checks
            .borrow_mut()
            .retain(|(node, _), _| !nodes.contains_key(node));
        source
            .snapshot_inline_styles
            .borrow_mut()
            .retain(|node, _| !nodes.contains_key(node));
        source
            .pending_dynamic_stylesheets
            .borrow_mut()
            .retain(|node| !nodes.contains_key(node));
        source
            .prepared_stylesheets
            .borrow_mut()
            .retain(|node, _| !nodes.contains_key(node));
        source
            .stylesheet_event_tasks
            .borrow_mut()
            .retain(|task| !task.targets_adopted_nodes(&destination.dom, nodes));
    }
    {
        let mut dom = destination.dom.borrow_mut();
        for node in nodes.values() {
            // Author CSSOM wrappers migrate separately; no source document grant or
            // response source is permission to apply CSS in the new document.
            let record = dom
                .arena_mut()
                .get_mut(*node)
                .expect("preflighted adoption node");
            if record.attributes.contains_null("style") {
                record.style_attribute_policy_allowed = false;
            }
            record.style_attribute_cssom = false;
        }
        for node in sheets {
            dom.arena_mut()
                .reset_adopted_stylesheet(node)
                .expect("preflighted adoption stylesheet");
        }
    }
    for (node, revision) in connected {
        destination.prepared_stylesheets.borrow_mut().insert(node, revision);
        destination
            .pending_dynamic_stylesheets
            .borrow_mut()
            .push_back(node);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_sheet_does_not_follow_owner_into_denied_destination() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                html::parse("<body></body>").into_runtime().unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(114), 1).unwrap();
        }
        let second = v8::Context::new(scope, Default::default());
        let allowed = Rc::new(Cell::new(false));
        let target = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                html::parse("<body></body>").into_runtime().unwrap(),
            ));
            let state = install_generated_bindings(scope, second, dom, RealmId(115), 1).unwrap();
            let allowed = allowed.clone();
            state.set_inline_style_policy(Rc::new(move |_, _, _, _| Ok(allowed.get())));
            let document = wrap_node(scope, &state, state.document_root()).unwrap();
            v8::Global::new(scope, document)
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let target = v8::Local::new(scope, &target);
        let key = v8::String::new(scope, "targetDocument").unwrap();
        first.global(scope).set(scope, key.into(), target.into());
        let script=v8::String::new(scope,r#"
            const owner=document.createElement('style');owner.textContent='a{color:red}';document.head.append(owner);
            const old=owner.sheet, rules=old.cssRules;old.insertRule('b{color:blue}',1);
            targetDocument.adoptNode(owner);targetDocument.head.append(owner);
            if(old.ownerNode!==null||old.cssRules!==rules||rules.length!==2)throw Error('old sheet custody');
            if(owner.sheet!==null||targetDocument.styleSheets.length!==0)throw Error('destination policy');
            old.insertRule('c{color:green}',2);
            if(owner.sheet!==null)throw Error('old CSSOM cannot authorize destination');
            owner.remove();
        "#).unwrap();
        v8::Script::compile(scope, script, None)
            .unwrap()
            .run(scope)
            .unwrap();
        allowed.set(true);
        let script=v8::String::new(scope,r#"
            targetDocument.head.append(owner);const fresh=owner.sheet;
            if(fresh===null||fresh===old||fresh.ownerNode!==owner||fresh.cssRules.length!==1)throw Error('fresh destination sheet');
            if(old.ownerNode!==null||old.cssRules.length!==3)throw Error('retained old rules');
        "#).unwrap();
        v8::Script::compile(scope, script, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
}
