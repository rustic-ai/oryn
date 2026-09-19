//! Backdrops are independently laid-out stacking contexts immediately
//! below their originating top-layer element, outside that element's opacity.
use crate::{PaintCommand, RenderError, RenderOptions};
use oryn_common::v4::*;
use std::collections::BTreeMap;
pub(crate) fn commands(
    snapshot: &PaintSnapshot,
    mut options: RenderOptions,
    diagnostics: &mut Vec<String>,
) -> Result<BTreeMap<u64, Vec<PaintCommand>>, RenderError> {
    let mut output = BTreeMap::new();
    for node in snapshot.nodes.iter().filter(|node| {
        node.top_layer.is_some() && node.geometry.availability == GeometryAvailability::Available
    }) {
        let Some(style) = node
            .backdrop
            .as_ref()
            .map(|backdrop| &backdrop.style)
            .or(node.backdrop_style.as_ref())
        else {
            continue;
        };
        if style.get("display").is_some_and(|value| value == "none") {
            continue;
        }
        if output.len() >= 1024 {
            return Err(RenderError::Encode(
                "native backdrop count exceeds 1024".into(),
            ));
        }
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: snapshot.viewport.width,
            height: snapshot.viewport.height,
        };
        let model = BoxModel {
            border_box: rect,
            padding_box: rect,
            content_box: rect,
            margin: Default::default(),
            padding: Default::default(),
            border: Default::default(),
        };
        let mut backdrop = if let Some(backdrop) = &node.backdrop {
            if backdrop.backdrop.is_some()
                || backdrop.backdrop_style.is_some()
                || backdrop.top_layer.is_some()
                || backdrop.parent_id != Some(node.node_id)
                || backdrop.node_id == node.node_id
                || snapshot
                    .nodes
                    .iter()
                    .any(|node| node.node_id == backdrop.node_id)
            {
                return Err(RenderError::Encode(
                    "invalid native backdrop owner or nested pseudo snapshot".into(),
                ));
            }
            (**backdrop).clone()
        } else {
            // Legacy snapshots predate independent pseudo geometry. Current
            // native producers always attach an actual layout snapshot.
            let mut backdrop = node.clone();
            backdrop.backdrop = None;
            backdrop.top_layer = None;
            backdrop.backdrop_style = None;
            backdrop.control = None;
            backdrop.tag = "oryn-native-backdrop".into();
            backdrop.style = style.clone();
            backdrop.image_resource = None;
            backdrop.text.clear();
            backdrop.projection = None;
            backdrop.geometry = NodeGeometry {
                availability: GeometryAvailability::Available,
                box_model: Some(model),
                client_rects: vec![rect],
                in_viewport: true,
                interactable: false,
                diagnostic: None,
            };
            backdrop
        };
        backdrop.parent_id = None;
        let mut local = snapshot.clone();
        local.nodes = vec![backdrop];
        local.frames.clear();
        local.visual_viewport = None;
        local.viewport_scroll = None;
        options.background = [0, 0, 0, 0];
        let mut scene = crate::build_display_list(&local, options)?;
        diagnostics.append(&mut scene.diagnostics);
        if !scene.commands.is_empty() {
            scene.commands.remove(0);
        }
        output.insert(node.node_id, scene.commands);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backdrop_is_independent_of_dialog_opacity_and_ancestor_clip() {
        let mut snapshot = crate::tests::empty_snapshot();
        let mut dialog = crate::tests::paint_node(
            9,
            None,
            "dialog",
            CssRect {
                x: 10.,
                y: 10.,
                width: 5.,
                height: 5.,
            },
        );
        dialog.top_layer = Some(0);
        dialog.style.insert("opacity".into(), "0".into());
        dialog.backdrop_style = Some([("background-color".into(), "rgb(255, 0, 0)".into())].into());
        snapshot.nodes.push(dialog);
        let mut diagnostics = Vec::new();
        let result = commands(&snapshot, RenderOptions::default(), &mut diagnostics).unwrap();
        assert!(diagnostics.is_empty());
        assert!(result[&9].iter().any(|command|matches!(command,PaintCommand::FillRect{rect,color} if rect.width==snapshot.viewport.width&&rect.height==snapshot.viewport.height&&*color==[255,0,0,255])));
    }
    #[test]
    fn native_backdrop_geometry_is_used_and_nested_snapshot_is_rejected() {
        let mut snapshot = crate::tests::empty_snapshot();
        let mut owner = crate::tests::paint_node(
            2,
            None,
            "dialog",
            CssRect {
                x: 0.,
                y: 0.,
                width: 2.,
                height: 2.,
            },
        );
        owner.top_layer = Some(0);
        let mut backdrop = crate::tests::paint_node(
            99,
            Some(2),
            "oryn-native-backdrop",
            CssRect {
                x: 7.,
                y: 11.,
                width: 20.,
                height: 25.,
            },
        );
        backdrop
            .style
            .insert("background-color".into(), "rgb(255, 0, 0)".into());
        owner.backdrop = Some(Box::new(backdrop.clone()));
        snapshot.nodes.push(owner);
        let mut diagnostics = Vec::new();
        let result = commands(&snapshot, RenderOptions::default(), &mut diagnostics).unwrap();
        assert!(!result[&2].is_empty());
        backdrop.backdrop = Some(Box::new(crate::tests::paint_node(
            100,
            Some(99),
            "nested",
            CssRect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
        )));
        snapshot.nodes[0].backdrop = Some(Box::new(backdrop));
        assert!(commands(&snapshot, RenderOptions::default(), &mut diagnostics).is_err());
    }
}
