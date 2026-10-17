// At the owning native checkpoint, graph snapshot/custody cloning is counted
// first; no admission callback executes under the publication registry borrow.
fn sweep_public_font_face_leases(
    registry:&Rc<RefCell<RetainedStylesheets>>,
    admission:&dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
)->Result<(),String>{
    let count=registry.borrow().records.len();
    let allocation=admission.reserve(count.checked_mul(std::mem::size_of::<Rc<RetainedStylesheetRecord>>())
        .ok_or("font lease sweep snapshot overflow")?)?;
    admission.checkpoint()?;
    let mut records=Vec::new();
    records.try_reserve_exact(count).map_err(|_|"font lease sweep snapshot allocation")?;
    {
        let registry=registry.borrow();
        if registry.records.len()!=count{return Err("font lease sweep registry changed".into());}
        records.extend(registry.records.iter().cloned());
    }
    for record in records {
        let context=record.graph.borrow().capture_font_lease_prune_context();
        let mut prepared=context.prepare(admission)?;
        let result={record.graph.borrow_mut().prune_font_leases(&mut prepared)};
        drop(prepared);
        result?;
    }
    drop(allocation);
    Ok(())
}
