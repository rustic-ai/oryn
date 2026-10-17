// Native query controls, included by dom_queries.rs under cfg(test).
#[test]
fn native_has_slotted_grammar_assignment_fallback_and_original_siblings() {
    let mut dom = oryn_dom::html::parse("<x-host id=host><div id=a></div><div id=b></div></x-host>").into_runtime().unwrap();
    let host = dom.find_by_id("host").unwrap();
    let a = dom.find_by_id("a").unwrap();
    let b = dom.find_by_id("b").unwrap();
    let root = dom.create(NodeKind::ShadowRoot { mode: oryn_dom::ShadowMode::Open, clonable: false });
    dom.set_manual_slot_assignment(root).unwrap();
    dom.append_child(host, root).unwrap();
    let slot = dom.create_element("slot");
    dom.append_child(root, slot).unwrap();
    let states = BTreeMap::new();
    let matched = |dom: &DomRuntime, selector: &str| query(dom, slot, selector, SelectorOperation::Matches, &states).unwrap() == vec![slot];
    assert!(!matched(&dom, ":has-slotted"));
    dom.assign_slot_nodes(slot, &[b]).unwrap();
    // The sibling a is NOT assigned to slot, but original light topology counts.
    assert!(matched(&dom, ":has-slotted(div + div)"));
    assert!(!matched(&dom, ":has-slotted(.selected)"));
    let before = dom.mutation_cursor();
    dom.set_attribute(b, "class", "selected").unwrap();
    assert_ne!(before, dom.mutation_cursor());
    assert!(matched(&dom, ":has-slotted(.selected)"));
    dom.set_attribute(b, "class", "other").unwrap();
    assert!(!matched(&dom, ":has-slotted(.selected)"));
    let span = dom.create_element("span");
    dom.append_child(b, span).unwrap();
    assert!(matched(&dom, ":has-slotted(div:has(> span))"));
    dom.remove_child(b, span).unwrap();
    assert!(!matched(&dom, ":has-slotted(div:has(> span))"));
    dom.assign_slot_nodes(slot, &[]).unwrap();
    let fallback = dom.create_element("em");
    dom.append_child(slot, fallback).unwrap();
    assert!(matched(&dom, ":has-slotted(em)"));
    dom.remove_child(slot, fallback).unwrap();
    let text = dom.create(NodeKind::Text { data: " ".into() });
    dom.append_child(host, text).unwrap();
    let before = dom.mutation_cursor();
    dom.assign_slot_nodes(slot, &[text]).unwrap();
    assert_ne!(before, dom.mutation_cursor());
    assert!(matched(&dom, ":has-slotted"));
    assert!(!matched(&dom, ":has-slotted(*)"));
    dom.set_text(text, "").unwrap();
    assert!(matched(&dom, ":has-slotted"));
    dom.assign_slot_nodes(slot, &[a]).unwrap();
    assert!(matched(&dom, ":has-slotted(div)"));
    for invalid in ["::has-slotted(foo)", ":has-slotted()", ":has-slotted(0)", ":has-slotted(div > span)"] {
        assert!(query(&dom, slot, invalid, SelectorOperation::Matches, &states).is_err(), "{invalid}");
    }
}

#[test]
fn native_has_slotted_outside_shadow_and_typed_operational_error() {
    let mut dom = oryn_dom::html::parse("<slot id=slot><div></div></slot>").into_runtime().unwrap();
    let slot = dom.find_by_id("slot").unwrap();
    let states = BTreeMap::new();
    assert!(query_typed(&dom, slot, ":has-slotted", SelectorOperation::Matches, &states).unwrap().is_empty());
    assert!(matches!(query_typed(&dom, slot, ":has-slotted()", SelectorOperation::Matches, &states), Err(QueryError::Syntax(_))));
    let stale = NodeId { slot: slot.slot, generation: slot.generation.checked_add(1).unwrap() };
    assert!(matches!(query_typed(&dom, stale, ":has-slotted", SelectorOperation::Matches, &states), Err(QueryError::Operational(_))));
}

#[test]
fn native_has_slotted_snapshot_admission_is_operational_and_selective() {
    struct Refuse;
    impl crate::cssom::native_import_graph::ImportAdmission for Refuse {
        fn checkpoint(&self) -> Result<(), String> { Ok(()) }
        fn reserve(&self, _: usize) -> Result<std::sync::Arc<dyn Send + Sync>, String> { Err("snapshot refused".into()) }
    }
    let mut dom = oryn_dom::html::parse("<x-host id=host><div></div></x-host>").into_runtime().unwrap();
    let host=dom.find_by_id("host").unwrap();
    let root=dom.create(NodeKind::ShadowRoot { mode:oryn_dom::ShadowMode::Open, clonable:false });
    dom.append_child(host,root).unwrap();
    let slot=dom.create_element("slot"); dom.append_child(root,slot).unwrap();
    let states=BTreeMap::new();
    assert!(matches!(query_admitted(&dom,slot,":has-slotted(div)",SelectorOperation::Matches,&states,Some(&Refuse)),Err(QueryError::Operational(_))));
    // Ordinary queries never ask the slot snapshot allocator for admission.
    assert_eq!(query_admitted(&dom,slot,"slot",SelectorOperation::Matches,&states,Some(&Refuse)).unwrap(),vec![slot]);
    assert_eq!(query_typed(&dom,slot,":has-slotted(div)",SelectorOperation::Matches,&states).unwrap(),vec![slot]);
}

#[test]
fn native_slot_snapshot_agrees_with_original_assignment_algorithm() {
    let mut dom=oryn_dom::html::parse("<x-host id=host><div id=a></div></x-host>").into_runtime().unwrap();
    let host=dom.find_by_id("host").unwrap(); let a=dom.find_by_id("a").unwrap();
    let root=dom.create(NodeKind::ShadowRoot {mode:oryn_dom::ShadowMode::Open,clonable:false});dom.append_child(host,root).unwrap();
    let first=dom.create_element("slot");let duplicate=dom.create_element("slot");
    dom.append_child(root,first).unwrap();dom.append_child(root,duplicate).unwrap();
    let cdata=dom.create(NodeKind::CDataSection {data:"".into()});dom.append_child(host,cdata).unwrap();
    let comment=dom.create(NodeKind::Comment {data:"ignored".into()});dom.append_child(host,comment).unwrap();
    let nested=dom.create_element("slot");dom.append_child(duplicate,nested).unwrap();
    let fallback=dom.create_element("em");dom.append_child(nested,fallback).unwrap();
    let compare=|dom:&DomRuntime| {
        let projection=slot_query_projection::prepare(dom,root,None).unwrap();
        for (slot,candidates,_) in &projection.slots {
            assert_eq!(*candidates,dom.assigned_nodes(*slot,true).unwrap(),"slot {slot:?}");
        }
    };
    compare(&dom);
    dom.set_attribute(first,"name","later").unwrap();compare(&dom);
    dom.set_attribute(a,"slot","later").unwrap();compare(&dom);
    dom.set_manual_slot_assignment(root).unwrap();
    dom.assign_slot_nodes(first,&[cdata,a]).unwrap();compare(&dom);
    dom.assign_slot_nodes(first,&[]).unwrap();compare(&dom);
}
