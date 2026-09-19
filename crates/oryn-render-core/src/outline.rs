//! CSS outlines occupy paint space without changing layout or scroll extents.
use crate::PaintCommand;
use oryn_common::v4::{CssRect, PaintBorderEdge, PaintNodeSnapshot};

pub(crate) fn commands(node: &PaintNodeSnapshot) -> Result<Vec<PaintCommand>, crate::RenderError> {
    let style = &node.style;
    if style.get("visibility").is_some_and(|v| v != "visible") {
        return Ok(Vec::new());
    }
    let kind = style
        .get("outline-style")
        .map(String::as_str)
        .unwrap_or("none");
    if matches!(kind, "none" | "hidden") {
        return Ok(Vec::new());
    }
    let length = |name: &str, default: f64| {
        style
            .get(name)
            .and_then(|v| v.trim().strip_suffix("px"))
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(default)
    };
    let width = length("outline-width", 3.);
    if width <= 0. {
        return Ok(Vec::new());
    }
    let offset = length("outline-offset", 0.);
    let color = style
        .get("outline-color")
        .filter(|v| !matches!(v.as_str(), "currentcolor" | "auto"))
        .or_else(|| style.get("color"))
        .cloned()
        .unwrap_or_else(|| "black".into());
    // The native theme uses a solid focus indicator for outline-style:auto.
    let edge = PaintBorderEdge {
        width,
        color,
        style: if kind == "auto" {
            "solid".into()
        } else {
            kind.into()
        },
    };
    let Some(model) = node
        .projection
        .as_ref()
        .map(|p| &p.box_model)
        .or(node.geometry.box_model.as_ref())
    else {
        return Ok(Vec::new());
    };
    let boxes = node
        .projection
        .as_ref()
        .filter(|p| !p.fragment_boxes.is_empty())
        .map(|p| p.fragment_boxes.as_slice())
        .unwrap_or_else(|| std::slice::from_ref(model));
    let mut commands = Vec::new();
    if boxes.len() > 1 {
        commands = crate::outline_contour::commands(
            &boxes
                .iter()
                .map(|model| model.border_box)
                .collect::<Vec<_>>(),
            &edge,
            offset,
        )?;
    } else {
        for model in boxes {
            let b = model.border_box;
            // Negative offsets stop collapsing at the centre of each physical axis.
            let dx = offset.max(-b.width / 2.) + width;
            let dy = offset.max(-b.height / 2.) + width;
            let rect = CssRect {
                x: b.x - dx,
                y: b.y - dy,
                width: b.width + 2. * dx,
                height: b.height + 2. * dy,
            };
            if ![rect.x, rect.y, rect.width, rect.height]
                .iter()
                .all(|v| v.is_finite())
            {
                continue;
            }
            let mut radii = crate::borders::radii(style, b);
            for (i, radius) in radii.iter_mut().enumerate() {
                if *radius > 0. {
                    *radius = (*radius + if i % 2 == 0 { dx } else { dy }).max(0.);
                }
            }
            commands.push(PaintCommand::BorderEdges {
                rect,
                edges: std::array::from_fn(|_| edge.clone()),
                radii,
            });
        }
    }
    Ok(if let Some(projection) = &node.projection {
        vec![PaintCommand::Projected {
            matrix: projection.matrix,
            clips: projection.clips.clone(),
            commands,
        }]
    } else {
        commands
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outline_expands_paint_only_and_retains_negative_offset() {
        let mut node = crate::tests::paint_node(
            1,
            None,
            "div",
            CssRect {
                x: 10.,
                y: 10.,
                width: 20.,
                height: 10.,
            },
        );
        node.projection = None;
        node.style.extend([
            ("outline-style".into(), "dashed".into()),
            ("outline-width".into(), "2px".into()),
            ("outline-offset".into(), "-1px".into()),
        ]);
        let before = node.geometry.clone();
        let output = commands(&node).unwrap();
        let PaintCommand::BorderEdges { rect, edges, .. } = &output[0] else {
            panic!("native outline edges")
        };
        assert_eq!(
            *rect,
            CssRect {
                x: 9.,
                y: 9.,
                width: 22.,
                height: 12.
            }
        );
        assert_eq!(edges[0].style, "dashed");
        assert_eq!(node.geometry, before);
    }

    #[test]
    fn context_outline_follows_descendants_inside_group_opacity() {
        let mut snapshot = crate::tests::empty_snapshot();
        let mut parent = crate::tests::paint_node(
            1,
            None,
            "div",
            CssRect {
                x: 0.,
                y: 0.,
                width: 20.,
                height: 20.,
            },
        );
        parent.style.insert("opacity".into(), "0.5".into());
        let child = crate::tests::paint_node(
            2,
            Some(1),
            "div",
            CssRect {
                x: 0.,
                y: 0.,
                width: 20.,
                height: 20.,
            },
        );
        snapshot.nodes = vec![parent, child];
        let mark = |red| PaintCommand::FillRect {
            rect: CssRect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
            color: [red, 0, 0, 255],
        };
        let mut output = Vec::new();
        crate::stacking::append(
            &snapshot,
            [(1, vec![mark(1)]), (2, vec![mark(2)])].into(),
            Default::default(),
            [(1, vec![mark(3)])].into(),
            Default::default(),
            &mut output,
        )
        .unwrap();
        let PaintCommand::Opacity { opacity, commands } = &output[0] else {
            panic!("context opacity")
        };
        assert_eq!(*opacity, 0.5);
        let colors = commands
            .iter()
            .map(|command| match command {
                PaintCommand::FillRect { color, .. } => color[0],
                _ => panic!("paint marker"),
            })
            .collect::<Vec<_>>();
        assert_eq!(colors, vec![1, 2, 3]);
    }
}
