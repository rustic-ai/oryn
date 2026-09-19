//! Immutable native hit geometry shared with the painter, including frame clips.
use super::*;
#[derive(Clone, Debug)]
pub struct Hit {
    pub projection: Option<oryn_common::v4::PaintProjection>,
    pub camera: Option<oryn_common::visual_viewport::Geometry>,
    pub frames: Vec<super::frames::Placement>,
}
impl Hit {
    pub fn contains(&self, mut x: f64, mut y: f64) -> bool {
        for frame in &self.frames {
            let Some(point) = frame.inverse(x, y) else {
                return false;
            };
            (x, y) = point;
        }
        if let Some(camera) = self.camera {
            let Some(point) = camera.visual_to_layout(x, y) else {
                return false;
            };
            (x, y) = point;
        }
        self.projection
            .as_ref()
            .is_some_and(|projection| projection.contains_point(x, y))
    }
}
pub(super) fn projection(
    node: &oryn_common::v4::PaintNodeSnapshot,
    camera: Option<oryn_common::visual_viewport::Geometry>,
) -> Option<Hit> {
    let mut projection = node.projection.clone()?;
    projection.text.clear();
    projection.text_clips.clear();
    projection.border_segments.clear();
    projection.border_edges = None;
    Some(Hit {
        projection: Some(projection),
        camera,
        frames: Vec::new(),
    })
}
