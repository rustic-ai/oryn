#[derive(Clone, Copy)]
struct InlineTextFrame { node: NodeId, next: usize }
type CapturedInlineEffectiveText = (String, Arc<dyn Send + Sync>);

// Grow caller-owned cursor storage outside DOM borrows. The incumbent and
// replacement capacities are charged together until transfer has completed.
fn grow_inline_frames(
    frames: &mut Vec<InlineTextFrame>,
    charge: &mut Option<Arc<dyn Send + Sync>>,
    admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
) -> Result<(), String> {
    let capacity = frames.capacity().checked_mul(2).ok_or("inline cursor capacity overflow")?.max(1);
    if capacity <= frames.len() { return Err("inline cursor capacity overflow".into()); }
    let bytes = capacity.checked_add(frames.capacity())
        .and_then(|n| n.checked_mul(std::mem::size_of::<InlineTextFrame>()))
        .and_then(|n| n.checked_add(std::mem::size_of::<(Option<Arc<dyn Send + Sync>>,Arc<dyn Send + Sync>)>() + 2*std::mem::size_of::<usize>()))
        .ok_or("inline cursor allocation overflow")?;
    let lease = admission.reserve(bytes)?;
    let mut replacement = Vec::new();
    replacement.try_reserve_exact(capacity).map_err(|error| error.to_string())?;
    if replacement.capacity()>capacity {return Err("inline cursor allocation capacity exceeded admitted plan".into());}
    replacement.append(frames);
    *frames = replacement;
    *charge = Some(lease);
    Ok(())
}

fn capture_inline_effective_text(
    dom: &SharedDomRuntime,
    owner: NodeId,
    cursor: oryn_dom::MutationCursor,
    admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
) -> Result<Option<CapturedInlineEffectiveText>, String> {
    let mut frames = Vec::new();
    let mut frame_charge = None;
    grow_inline_frames(&mut frames, &mut frame_charge, admission)?;
    frames.push(InlineTextFrame { node: owner, next: 0 });
    let mut demand = 0usize;
    let mut maximum_depth = 1usize;
    let mut override_source = false;
    loop {
        admission.checkpoint()?;
        let mut need_capacity = false;
        {
            let borrowed = dom.try_borrow().map_err(|_| "inline cursor DOM conflict")?;
            if borrowed.mutation_cursor() != cursor { return Ok(None); }
            let arena = borrowed.arena();
            if let Some(source) = &arena.get(owner).map_err(|e|e.to_string())?.stylesheet_override {
                demand = source.as_ref().len().checked_mul(2).ok_or("inline override overflow")?;
                override_source = true;
                break;
            }
            let cursor_full = frames.len() == frames.capacity();
            let Some(frame) = frames.last_mut() else { break };
            let node = arena.get(frame.node).map_err(|e|e.to_string())?;
            if let Some(id) = node.children.get(frame.next).copied() {
                let child = arena.get(id).map_err(|e|e.to_string())?;
                match &child.kind {
                    NodeKind::Element { .. } if !child.children.is_empty() => {
                        if cursor_full { need_capacity = true; }
                        else {
                            frame.next = frame.next.checked_add(1).ok_or("inline cursor overflow")?;
                            frames.push(InlineTextFrame { node:id, next:0 });
                            maximum_depth = maximum_depth.max(frames.len());
                        }
                    }
                    NodeKind::Text { data } | NodeKind::CDataSection { data } => {
                        demand = demand.checked_add(data.len().checked_mul(8).ok_or("inline source overflow")?)
                            .ok_or("inline source overflow")?;
                        frame.next = frame.next.checked_add(1).ok_or("inline cursor overflow")?;
                    }
                    _ => frame.next = frame.next.checked_add(1).ok_or("inline cursor overflow")?,
                }
            } else { frames.pop(); }
        }
        if need_capacity { grow_inline_frames(&mut frames, &mut frame_charge, admission)?; }
    }
    let bytes = demand.checked_add(std::mem::size_of::<(Option<Arc<dyn Send+Sync>>,Arc<dyn Send+Sync>)>() + 2*std::mem::size_of::<usize>())
        .ok_or("inline source allocation overflow")?;
    let source_charge = admission.reserve(bytes)?;
    let mut units = Vec::new();
    if !override_source { units.try_reserve_exact(demand/2).map_err(|e|e.to_string())?; }
    if units.capacity()>demand/2 {return Err("inline source allocation capacity exceeded admitted plan".into());}
    admission.checkpoint()?;
    let source = {
        let borrowed = dom.try_borrow().map_err(|_| "inline source copy DOM conflict")?;
        if borrowed.mutation_cursor() != cursor { return Ok(None); }
        let arena = borrowed.arena();
        if override_source {
            arena.get(owner).map_err(|e|e.to_string())?.stylesheet_override.as_ref()
                .ok_or("inline override changed")?.as_ref().to_owned()
        } else {
            frames.clear();
            assert!(frames.capacity() >= maximum_depth);
            frames.push(InlineTextFrame {node:owner,next:0});
            while let Some(frame) = frames.last_mut() {
                let node = arena.get(frame.node).map_err(|e|e.to_string())?;
                if let Some(id) = node.children.get(frame.next).copied() {
                    frame.next = frame.next.checked_add(1).ok_or("inline cursor overflow")?;
                    let child = arena.get(id).map_err(|e|e.to_string())?;
                    match &child.kind {
                        NodeKind::Element { .. } if !child.children.is_empty() => frames.push(InlineTextFrame {node:id,next:0}),
                        NodeKind::Text {data} | NodeKind::CDataSection {data} => {
                            let piece = oryn_dom::dom_string::try_units(data,data.len()).map_err(str::to_owned)?;
                            if piece.len() > units.capacity()-units.len() {return Err("inline source demand changed".into());}
                            units.extend(piece);
                        }
                        _ => {}
                    }
                } else {frames.pop();}
            }
            oryn_dom::dom_string::from_units(&units)
        }
    };
    admission.checkpoint()?;
    drop(frames);
    drop(frame_charge);
    Ok(Some((source,source_charge)))
}
