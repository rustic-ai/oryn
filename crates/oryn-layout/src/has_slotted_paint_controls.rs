// Included in shadow_tree_tests: real parsed Stylo cascade and continuity.
#[test]
fn has_slotted_paint_restyles_original_candidate_and_assignment_changes() {
    let mut input = fixture();
    input.stylesheets[1] = "slot { color: red } slot:has-slotted(.chosen) { color: green } slot:has-slotted(span:has(> em)) { width: 71px }".into();
    let mut engine = StyleEngine::default();
    let initial = engine.compute_at(&input, &LayoutLimits::default(), 0.).unwrap();
    assert_eq!(style_value(&initial.computed_styles[&6], "color", ""), "rgb(255, 0, 0)");
    input.nodes.iter_mut().find(|node| node.id == 4).unwrap().attributes.insert("class".into(), "chosen".into());
    let changed = engine.compute_at(&input, &LayoutLimits::default(), 0.).unwrap();
    assert_eq!(style_value(&changed.computed_styles[&6], "color", ""), "rgb(0, 128, 0)");
    input.nodes.iter_mut().find(|node| node.id == 4).unwrap().children.push(8);
    input.nodes.push(element(8, Some(4), "em", vec![]));
    let descendant = engine.compute_at(&input, &LayoutLimits::default(), 0.).unwrap();
    assert_eq!(style_value(&descendant.computed_styles[&6], "width", ""), "71px");
    input.shadow_trees.assigned_slots.clear();
    let removed = engine.compute_at(&input, &LayoutLimits::default(), 0.).unwrap();
    assert_eq!(style_value(&removed.computed_styles[&6], "color", ""), "rgb(255, 0, 0)");
    assert_ne!(style_value(&removed.computed_styles[&6], "width", ""), "71px");
}

#[test]
fn has_slotted_projection_admission_failure_is_explicit_and_retry_keeps_truth() {
    struct Admission(std::sync::atomic::AtomicBool);
    impl cssom::native_import_graph::ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> { Ok(()) }
        fn reserve(&self, _: usize) -> Result<Arc<dyn Send + Sync>, String> {
            if self.0.load(std::sync::atomic::Ordering::Acquire) { return Err("native control refusal".into()); }
            Ok(Arc::new(()))
        }
    }
    let mut input = fixture();
    input.stylesheets[1] = "slot { color:red } slot:has-slotted(span){color:green}".into();
    let admission = Arc::new(Admission(std::sync::atomic::AtomicBool::new(true)));
    let mut engine = StyleEngine::default();
    engine.set_slot_projection_admission(Some(admission.clone()));
    assert!(engine.compute_at(&input, &LayoutLimits::default(), 0.).is_err());
    admission.0.store(false, std::sync::atomic::Ordering::Release);
    let output = engine.compute_at(&input, &LayoutLimits::default(), 0.).unwrap();
    assert_eq!(style_value(&output.computed_styles[&6], "color", ""), "rgb(0, 128, 0)");
}
