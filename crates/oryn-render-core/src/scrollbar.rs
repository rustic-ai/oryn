//! Deterministic native classic scrollbar chrome. Overlay mode reserves no
//! gutters and therefore emits no persistent chrome.
use super::*;
pub(super) fn commands(node: &oryn_common::v4::PaintNodeSnapshot) -> Vec<PaintCommand> {
    let Some(projection) = &node.projection else {
        return Vec::new();
    };
    paint(
        oryn_common::scrollbar::geometry(projection, &node.style),
        &node.style,
    )
}
pub(super) fn viewport_commands(snapshot: &PaintSnapshot) -> Vec<PaintCommand> {
    let Some(viewport) = &snapshot.viewport_scroll else {
        return Vec::new();
    };
    paint(
        oryn_common::scrollbar::geometry_for_viewport_scroll(
            &viewport.scroll,
            CssRect {
                x: 0.,
                y: 0.,
                width: snapshot.viewport.width,
                height: snapshot.viewport.height,
            },
            &viewport.style,
            snapshot.visual_viewport,
        ),
        &viewport.style,
    )
}
fn paint(
    geometry: Vec<oryn_common::scrollbar::ScrollbarGeometry>,
    style: &std::collections::BTreeMap<String, String>,
) -> Vec<PaintCommand> {
    let colors = style
        .get("scrollbar-color")
        .and_then(|value| gradients::split(value, ' '))
        .and_then(|parts| {
            let parts = parts
                .into_iter()
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>();
            if parts.len() == 2 {
                Some((parse_color(parts[0])?, parse_color(parts[1])?))
            } else {
                None
            }
        })
        .unwrap_or(([128, 128, 128, 255], [240, 240, 240, 255]));
    let mut commands = Vec::new();
    for bar in geometry {
        commands.push(PaintCommand::FillRect {
            rect: bar.track,
            color: colors.1,
        });
        let radius = if bar.horizontal {
            bar.track.height
        } else {
            bar.track.width
        }
        .min(4.)
            / 2.;
        commands.push(PaintCommand::FillRoundedRect {
            rect: bar.thumb,
            color: colors.0,
            radii: [radius; 8],
        });
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zoom_changes_viewport_thumb_without_scaling_chrome_track() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.viewport_scroll = Some(oryn_common::v4::PaintViewportScroll {
            scroll: oryn_common::v4::PaintScrollGeometry {
                offset_x: 0.,
                offset_y: 12.,
                scrollport: CssRect {
                    x: 0.,
                    y: 0.,
                    width: 48.,
                    height: 32.,
                },
                scrollable: CssRect {
                    x: 0.,
                    y: 0.,
                    width: 48.,
                    height: 96.,
                },
            },
            style: std::collections::BTreeMap::from([("overflow-y".into(), "scroll".into())]),
        });
        let ordinary = viewport_commands(&snapshot);
        snapshot.visual_viewport = Some(oryn_common::visual_viewport::Geometry {
            scrollport: None,
            layout_width: snapshot.viewport.width,
            layout_height: snapshot.viewport.height,
            scale: 2.,
            offset_x: 0.,
            offset_y: 8.,
        });
        let zoomed = viewport_commands(&snapshot);
        let PaintCommand::FillRect { rect: before, .. } = &ordinary[0] else {
            panic!("track")
        };
        let PaintCommand::FillRect { rect: after, .. } = &zoomed[0] else {
            panic!("track")
        };
        assert_eq!(before, after);
        let expected = oryn_common::scrollbar::geometry_for_viewport_scroll(
            &snapshot.viewport_scroll.as_ref().unwrap().scroll,
            CssRect {
                x: 0.,
                y: 0.,
                width: snapshot.viewport.width,
                height: snapshot.viewport.height,
            },
            &snapshot.viewport_scroll.as_ref().unwrap().style,
            snapshot.visual_viewport,
        );
        let PaintCommand::FillRoundedRect { rect, .. } = &zoomed[1] else {
            panic!("thumb")
        };
        assert_eq!(*rect, expected[0].thumb);
        assert_eq!(expected[0].offset, 20.);
        assert_eq!(expected[0].page_step, 16.);
    }
    #[test]
    fn classic_track_and_thumb_follow_signed_scroll_range() {
        let mut node = crate::tests::paint_node(
            1,
            None,
            "div",
            CssRect {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 100.,
            },
        );
        node.style.insert("overflow-x".into(), "auto".into());
        node.projection.as_mut().unwrap().scroll = Some(oryn_common::v4::PaintScrollGeometry {
            offset_x: -100.,
            offset_y: 0.,
            scrollport: CssRect {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 84.,
            },
            scrollable: CssRect {
                x: -100.,
                y: 0.,
                width: 200.,
                height: 84.,
            },
        });
        let left = commands(&node);
        assert_eq!(left.len(), 2);
        let PaintCommand::FillRoundedRect { rect, .. } = left[1] else {
            panic!("thumb")
        };
        assert_eq!((rect.x, rect.width), (0., 50.));
        node.projection
            .as_mut()
            .unwrap()
            .scroll
            .as_mut()
            .unwrap()
            .offset_x = 0.;
        let right = commands(&node);
        let PaintCommand::FillRoundedRect { rect, .. } = right[1] else {
            panic!("thumb")
        };
        assert_eq!((rect.x, rect.width), (50., 50.));
        node.projection
            .as_mut()
            .unwrap()
            .scroll
            .as_mut()
            .unwrap()
            .scrollport
            .height = 100.;
        assert!(commands(&node).is_empty());
    }
    #[test]
    fn viewport_chrome_ignores_root_transform() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.viewport_scroll = Some(oryn_common::v4::PaintViewportScroll {
            scroll: oryn_common::v4::PaintScrollGeometry {
                offset_x: 0.,
                offset_y: 0.,
                scrollport: CssRect {
                    x: 0.,
                    y: 0.,
                    width: 48.,
                    height: 32.,
                },
                scrollable: CssRect {
                    x: 0.,
                    y: 0.,
                    width: 48.,
                    height: 96.,
                },
            },
            style: std::collections::BTreeMap::from([("overflow-y".into(), "auto".into())]),
        });
        let mut root = crate::tests::paint_node(
            1,
            None,
            "html",
            CssRect {
                x: 0.,
                y: 0.,
                width: 48.,
                height: 96.,
            },
        );
        root.projection.as_mut().unwrap().matrix[0] = 2.;
        snapshot.nodes.push(root);
        let scene = build_display_list(&snapshot, RenderOptions::default()).unwrap();
        assert!(
            matches!(scene.commands.last(),Some(PaintCommand::FillRoundedRect{rect,..})if rect.x>=48.&&rect.right()<=64.)
        );
    }
}
