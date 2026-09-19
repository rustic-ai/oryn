//! Renderer-private semantic views; resource authority and URL bases are copied.
//! Synthetic paint IDs never leave PDF processing or become DOM identities.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Default)]
struct Budget {
    nodes: usize,
    bytes: usize,
    views: usize,
}
impl Budget {
    fn charge(&mut self, nodes: usize, bytes: usize) -> Result<(), RenderError> {
        self.nodes = self
            .nodes
            .checked_add(nodes)
            .filter(|n| *n <= 65536)
            .ok_or_else(|| RenderError::InvalidPrint("semantic SVG instance node budget".into()))?;
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .filter(|n| *n <= 64 * 1024 * 1024)
            .ok_or_else(|| {
                RenderError::InvalidPrint("semantic SVG instance resource budget".into())
            })?;
        self.views += 1;
        if self.views > 4096 {
            return Err(RenderError::InvalidPrint(
                "semantic SVG instance count".into(),
            ));
        }
        Ok(())
    }
}
pub(super) fn expand(source: &PaintSnapshot) -> Result<PaintSnapshot, RenderError> {
    crate::frames::validate(source)?;
    let mut snapshot = source.clone();
    let mut budget = Budget::default();
    document(&mut snapshot, &mut budget, 0)?;
    crate::frames::validate(&snapshot)?;
    Ok(snapshot)
}
fn document(
    snapshot: &mut PaintSnapshot,
    budget: &mut Budget,
    depth: usize,
) -> Result<(), RenderError> {
    if depth > 32 {
        return Err(RenderError::InvalidPrint(
            "semantic SVG instance depth".into(),
        ));
    }
    let mut used = snapshot
        .nodes
        .iter()
        .chain(
            snapshot
                .svg_foreign_objects
                .iter()
                .flat_map(|object| object.nodes.iter()),
        )
        .map(|node| node.node_id)
        .collect::<BTreeSet<_>>();
    let mut next = u64::MAX;
    let mut nodes = std::mem::take(&mut snapshot.nodes);
    views(
        snapshot,
        &mut nodes,
        &mut used,
        &mut next,
        budget,
        depth,
        &mut Vec::new(),
    )?;
    snapshot.nodes = nodes;
    // Child documents retain their own resources and native coordinates.
    for frame in &mut snapshot.frames {
        document(&mut frame.snapshot, budget, depth + 1)?;
    }
    Ok(())
}
fn views(
    snapshot: &mut PaintSnapshot,
    nodes: &mut Vec<oryn_common::v4::PaintNodeSnapshot>,
    used: &mut BTreeSet<u64>,
    next: &mut u64,
    budget: &mut Budget,
    depth: usize,
    stack: &mut Vec<usize>,
) -> Result<(), RenderError> {
    if depth > 32 {
        return Err(RenderError::InvalidPrint(
            "semantic SVG instance depth".into(),
        ));
    }
    for node in nodes {
        let Some(image) = node.image_resource else {
            continue;
        };
        let objects = snapshot
            .svg_foreign_objects
            .iter()
            .filter(|object| object.image_resource == image)
            .cloned()
            .collect::<Vec<_>>();
        if !snapshot
            .images
            .get(image)
            .is_some_and(|source| source.media_type == "image/svg+xml")
        {
            continue;
        }
        if stack.contains(&image) {
            return Err(RenderError::InvalidPrint(
                "cyclic captured SVG semantic instance".into(),
            ));
        }
        stack.push(image);
        let mut resource = snapshot
            .images
            .get(image)
            .ok_or_else(|| RenderError::InvalidPrint("missing semantic instance resource".into()))?
            .clone();
        resource.bytes =
            crate::svg::xml_bytes_with_type(&resource.bytes, resource.content_type.as_deref())
                .map_err(RenderError::InvalidPrint)?;
        resource.content_type = None;
        let index = snapshot.images.len();
        resource
            .bytes
            .extend_from_slice(format!("<!--oryn-private-semantic-instance:{index}-->").as_bytes());
        budget.charge(
            objects.iter().map(|object| object.nodes.len()).sum(),
            resource.bytes.len(),
        )?;
        snapshot.images.push(resource);
        node.image_resource = Some(index);
        for mut object in objects {
            object.image_resource = index;
            let mut remap = BTreeMap::new();
            for child in &object.nodes {
                let id = loop {
                    let id = *next;
                    *next = next.checked_sub(1).ok_or_else(|| {
                        RenderError::InvalidPrint("semantic paint identity exhausted".into())
                    })?;
                    if used.insert(id) {
                        break id;
                    }
                };
                remap.insert(child.node_id, id);
            }
            let frames = snapshot
                .frames
                .iter()
                .enumerate()
                .filter(|(_, frame)| remap.contains_key(&frame.owner_node))
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            for child in &mut object.nodes {
                child.node_id = remap[&child.node_id];
                child.parent_id = child
                    .parent_id
                    .and_then(|parent| remap.get(&parent).copied());
            }
            for index in frames {
                if snapshot.frames.len() >= 256 {
                    return Err(RenderError::InvalidPrint(
                        "semantic instance frame budget".into(),
                    ));
                }
                fn cost(snapshot: &PaintSnapshot) -> (usize, usize) {
                    let mut nodes = snapshot.nodes.len()
                        + snapshot
                            .svg_foreign_objects
                            .iter()
                            .map(|object| object.nodes.len())
                            .sum::<usize>();
                    let mut bytes = snapshot
                        .images
                        .iter()
                        .map(|image| image.bytes.len())
                        .sum::<usize>()
                        + snapshot
                            .fonts
                            .values()
                            .map(|font| font.bytes.len())
                            .sum::<usize>();
                    for frame in &snapshot.frames {
                        let (n, b) = cost(&frame.snapshot);
                        nodes = nodes.saturating_add(n);
                        bytes = bytes.saturating_add(b);
                    }
                    (nodes, bytes)
                }
                let (nodes, bytes) = cost(&snapshot.frames[index].snapshot);
                budget.charge(nodes, bytes)?;
                let mut frame = snapshot.frames[index].clone();
                frame.owner_node = remap[&frame.owner_node];
                snapshot.frames.push(frame);
            }
            views(
                snapshot,
                &mut object.nodes,
                used,
                next,
                budget,
                depth + 1,
                stack,
            )?;
            snapshot.svg_foreign_objects.push(object);
        }
        stack.pop();
    }
    Ok(())
}
