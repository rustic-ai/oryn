//! Exact native frame ownership and shared renderer placement for OS AX trees.
use super::*;
use crate::host::webdriver_script::{AutomationRealmSnapshot, FrameCommand, FrameResponse};
#[derive(Clone, Debug)]
pub struct Placement {
    projection: Option<oryn_common::v4::PaintProjection>,
    content: CssRect,
    matrix: [f64; 16],
    camera: Option<oryn_common::visual_viewport::Geometry>,
    child_width: f64,
    child_height: f64,
}
const IDENTITY: [f64; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];
pub(super) fn placements(paint: &PaintSnapshot) -> BTreeMap<u64, Placement> {
    let mut result = BTreeMap::new();
    for frame in &paint.frames {
        if let Some(owner) = paint
            .nodes
            .iter()
            .find(|node| node.node_id == frame.owner_node)
        {
            let geometry = owner
                .projection
                .as_ref()
                .map(|p| (p.box_model.content_box, p.matrix))
                .or_else(|| {
                    owner
                        .geometry
                        .box_model
                        .as_ref()
                        .map(|model| (model.content_box, IDENTITY))
                });
            if let Some((content, matrix)) = geometry {
                result.insert(
                    frame.owner_node,
                    Placement {
                        projection: owner.projection.clone().map(|mut projection| {
                            projection.text.clear();
                            projection.text_clips.clear();
                            projection
                        }),
                        content,
                        matrix,
                        camera: paint.visual_viewport,
                        child_width: frame.snapshot.viewport.width,
                        child_height: frame.snapshot.viewport.height,
                    },
                );
            }
        }
    }
    result
}
impl Placement {
    pub(super) fn inverse(&self, mut x: f64, mut y: f64) -> Option<(f64, f64)> {
        if let Some(camera) = self.camera {
            (x, y) = camera.visual_to_layout(x, y)?;
        }
        if self
            .projection
            .as_ref()
            .is_some_and(|projection| !projection.contains_point(x, y))
        {
            return None;
        }
        let m = self.matrix;
        let a = m[0] - x * m[3];
        let b = m[4] - x * m[7];
        let c = x * m[15] - m[12];
        let d = m[1] - y * m[3];
        let e = m[5] - y * m[7];
        let f = y * m[15] - m[13];
        let determinant = a * e - b * d;
        if determinant == 0. || !determinant.is_finite() {
            return None;
        }
        let u = (c * e - b * f) / determinant;
        let v = (a * f - c * d) / determinant;
        if !u.is_finite()
            || !v.is_finite()
            || u < self.content.x
            || v < self.content.y
            || u > self.content.right()
            || v > self.content.bottom()
            || self.content.width <= 0.
            || self.content.height <= 0.
        {
            return None;
        }
        Some((
            (u - self.content.x) * self.child_width / self.content.width,
            (v - self.content.y) * self.child_height / self.content.height,
        ))
    }
    fn point(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        if self.child_width <= 0. || self.child_height <= 0. {
            return None;
        }
        let x = self.content.x + x * self.content.width / self.child_width;
        let y = self.content.y + y * self.content.height / self.child_height;
        let m = self.matrix;
        let w = m[3] * x + m[7] * y + m[15];
        if !w.is_finite() || w <= 0. {
            return None;
        }
        let point = (
            (m[0] * x + m[4] * y + m[12]) / w,
            (m[1] * x + m[5] * y + m[13]) / w,
        );
        if !point.0.is_finite() || !point.1.is_finite() {
            return None;
        }
        self.camera.map_or(Some(point), |camera| {
            camera.layout_to_visual(point.0, point.1)
        })
    }
}
fn project(rect: CssRect, chain: &[Placement]) -> Option<CssRect> {
    let mut points = [
        (rect.x, rect.y),
        (rect.right(), rect.y),
        (rect.right(), rect.bottom()),
        (rect.x, rect.bottom()),
    ];
    for placement in chain.iter().rev() {
        for point in &mut points {
            *point = placement.point(point.0, point.1)?;
        }
    }
    let left = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let top = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let right = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let bottom = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    Some(CssRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}
pub async fn capture(page: &PageHandle) -> Result<Snapshot, String> {
    let mut root = capture_document(page).await?;
    if page.frame_scope().is_some() {
        return Ok(root);
    }
    let FrameResponse::Inventory(before) = page
        .webdriver_frames(FrameCommand::Inventory)
        .await
        .map_err(|e| format!("{}: {}", e.code, e.message))?
    else {
        return Err("native accessibility frame inventory missing".into());
    };
    if before.len() > 257 {
        return Err("accessibility frame budget exceeded".into());
    }
    let Some(top) = before
        .iter()
        .find(|scope| scope.live && scope.parent_realm.is_none())
        .cloned()
    else {
        return Err("native accessibility root realm missing".into());
    };
    root.root_scope = Some(top.clone());
    let mut captured_children = Vec::new();
    let mut text_storage = root.text_storage_bytes()?;
    let mut parents: BTreeMap<_, (u64, Vec<Placement>, BTreeMap<u64, Placement>)> =
        BTreeMap::from([(
            top.realm,
            (root.generation, Vec::new(), root.placements.clone()),
        )]);
    let mut excluded = BTreeSet::new();
    let mut pending: Vec<AutomationRealmSnapshot> = before
        .iter()
        .filter(|scope| scope.live && scope.parent_realm.is_some())
        .cloned()
        .collect();
    while !pending.is_empty() {
        let Some(index) = pending.iter().position(|scope| {
            scope
                .parent_realm
                .is_some_and(|parent| parents.contains_key(&parent) || excluded.contains(&parent))
        }) else {
            return Err("cyclic or retired accessibility frame ancestry".into());
        };
        let scope = pending.remove(index);
        if scope
            .parent_realm
            .is_some_and(|parent| excluded.contains(&parent))
        {
            excluded.insert(scope.realm);
            continue;
        }
        let (generation, parent_chain, parent_placements) =
            parents.get(&scope.parent_realm.unwrap()).cloned().unwrap();
        let frame = scope.frame.ok_or("native frame owner missing")?;
        let packed = (u64::from(frame.generation) << 32) | u64::from(frame.slot);
        let owner = SemanticRef {
            page_id: root.page,
            document_generation: generation,
            node: packed,
        };
        if !root.nodes.contains_key(&owner) {
            excluded.insert(scope.realm);
            continue;
        }
        let placement = parent_placements
            .get(&packed)
            .cloned()
            .ok_or("native frame paint placement unavailable")?;
        let mut chain = parent_chain;
        chain.push(placement);
        if chain.len() > 32 {
            return Err("accessibility frame depth exceeded".into());
        }
        let child_page = page.with_frame_scope(Some(scope.clone()));
        let mut child = capture_document(&child_page).await?;
        if root.nodes.len().saturating_add(child.nodes.len()) > MAX_NODES {
            return Err("accessibility frame node budget exceeded".into());
        }
        text_storage = text_storage
            .checked_add(child.text_storage_bytes()?)
            .ok_or("accessibility frame text storage overflow")?;
        if text_storage > 256 * 1024 * 1024 {
            return Err("accessibility frame text and geometry storage budget exceeded".into());
        }
        captured_children.push((scope.clone(), child.generation, child.revision));
        parents.insert(
            scope.realm,
            (child.generation, chain.clone(), child.placements.clone()),
        );
        let owner_order = root.nodes[&owner].paint_order.clone();
        for node in child.nodes.values_mut() {
            let mut order = owner_order.clone();
            order.extend(node.paint_order.iter().copied());
            node.paint_order = order;
            if let Some(hit) = &mut node.hit {
                hit.frames = chain.clone();
            }
            node.scope = Some(scope.clone());
            node.rect = node.rect.and_then(|rect| project(rect, &chain));
            if let Some(text) = &mut node.text {
                text.protected_caret = text.protected_caret.take().and_then(|mut record| {
                    let caret = &mut record.caret;
                    let rect = project(
                        CssRect {
                            x: caret.x,
                            y: caret.y,
                            width: 0.,
                            height: 0.,
                        },
                        &chain,
                    )?;
                    caret.x = rect.x;
                    caret.y = rect.y;
                    caret.rect = caret.rect.and_then(|rect| project(rect, &chain));
                    caret.rect?;
                    Some(record)
                });
                text.carets.retain_mut(|caret| {
                    if let Some(rect) = project(
                        CssRect {
                            x: caret.x,
                            y: caret.y,
                            width: 0.,
                            height: 0.,
                        },
                        &chain,
                    ) {
                        caret.x = rect.x;
                        caret.y = rect.y;
                        caret.rect = caret.rect.and_then(|rect| project(rect, &chain));
                        true
                    } else {
                        false
                    }
                });
                text.boxes.retain_mut(|item| {
                    if let Some(rect) = project(item.rect, &chain) {
                        item.rect = rect;
                        true
                    } else {
                        false
                    }
                });
            }
            if node.parent.is_none() {
                node.parent = Some(owner);
            }
        }
        root.nodes
            .get_mut(&owner)
            .unwrap()
            .children
            .extend(child.roots.iter().copied());
        for (id, node) in child.nodes {
            if root.nodes.insert(id, node).is_some() {
                return Err("duplicate accessibility frame identity".into());
            }
        }
        root.live_regions.extend(child.live_regions);
        root.tables.extend(child.tables);
        for (id, picker) in child.file_pickers {
            if root.file_pickers.insert(id, picker).is_some() {
                return Err("duplicate native file picker frame identity".into());
            }
        }
        for (id, target) in child.picker_actions {
            if root.picker_actions.insert(id, target).is_some() {
                return Err("duplicate native picker frame identity".into());
            }
        }
    }
    // Frame inventory tracks lifetimes, not same-document semantic mutations.
    // Revalidate every captured child before publishing a composed observation.
    for (scope, generation, revision) in captured_children {
        let check = page
            .with_frame_scope(Some(scope))
            .observe_semantics(ProjectionProfile::Full)
            .await
            .map_err(|error| error.to_string())?;
        check_captured_revision(
            generation,
            revision,
            check.observation.page.document_generation,
            check.observation.revision,
        )?;
    }
    let FrameResponse::Inventory(after) = page
        .webdriver_frames(FrameCommand::Inventory)
        .await
        .map_err(|e| format!("{}: {}", e.code, e.message))?
    else {
        return Err("native accessibility frame inventory missing".into());
    };
    if before != after {
        return Err("accessibility frame lifetime changed during capture".into());
    }
    let check = page
        .observe_semantics(ProjectionProfile::Full)
        .await
        .map_err(|e| e.to_string())?;
    if check.observation.page.document_generation != root.generation
        || check.observation.revision != root.revision
    {
        return Err("root changed during accessibility frame assembly".into());
    }
    Ok(root)
}
fn check_captured_revision(
    expected_generation: u64,
    expected_revision: Revision,
    generation: u64,
    revision: Revision,
) -> Result<(), String> {
    if expected_generation != generation || expected_revision != revision {
        return Err("child changed during accessibility frame assembly".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_document_child_mutation_invalidates_composed_snapshot() {
        assert!(check_captured_revision(3, Revision(7), 3, Revision(8)).is_err());
        assert!(check_captured_revision(3, Revision(7), 4, Revision(7)).is_err());
        assert!(check_captured_revision(3, Revision(7), 3, Revision(7)).is_ok());
    }

    #[test]
    fn nested_frame_projection_uses_native_content_transform_and_camera_once() {
        let placement = Placement {
            projection: None,
            content: CssRect {
                x: 10.,
                y: 20.,
                width: 100.,
                height: 50.,
            },
            matrix: IDENTITY,
            camera: Some(oryn_common::visual_viewport::Geometry {
                layout_width: 400.,
                layout_height: 200.,
                scale: 2.,
                offset_x: 5.,
                offset_y: 0.,
                scrollport: None,
            }),
            child_width: 100.,
            child_height: 50.,
        };
        let result = project(
            CssRect {
                x: 3.,
                y: 4.,
                width: 2.,
                height: 3.,
            },
            &[placement],
        )
        .unwrap();
        assert_eq!(
            result,
            CssRect {
                x: 16.,
                y: 48.,
                width: 4.,
                height: 6.
            }
        );
    }
    #[test]
    fn invalid_projective_plane_does_not_invent_ax_position() {
        let mut matrix = IDENTITY;
        matrix[15] = 0.;
        assert!(
            project(
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: 1.,
                    height: 1.
                },
                &[Placement {
                    projection: None,
                    content: CssRect {
                        x: 0.,
                        y: 0.,
                        width: 1.,
                        height: 1.
                    },
                    matrix,
                    camera: None,
                    child_width: 1.,
                    child_height: 1.
                }]
            )
            .is_none()
        );
    }
}
