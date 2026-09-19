//! Shared native classic scrollbar geometry for paint and pointer interaction.
use crate::v4::{CssRect, PaintProjection};
use std::collections::BTreeMap;
#[derive(Debug, Clone, PartialEq)]
pub struct ScrollbarGeometry {
    pub horizontal: bool,
    pub track: CssRect,
    pub thumb: CssRect,
    pub min_offset: f64,
    pub max_offset: f64,
    pub offset: f64,
    pub page_step: f64,
}
impl ScrollbarGeometry {
    /// Map a dragged thumb's leading edge (not pointer center) to native offset.
    pub fn offset_at_thumb_origin(&self, origin: f64) -> Option<f64> {
        if !origin.is_finite() {
            return None;
        }
        let (start, length, thumb) = if self.horizontal {
            (self.track.x, self.track.width, self.thumb.width)
        } else {
            (self.track.y, self.track.height, self.thumb.height)
        };
        let travel = length - thumb;
        Some(if travel <= 0. {
            self.min_offset
        } else {
            self.min_offset
                + ((origin - start) / travel).clamp(0., 1.) * (self.max_offset - self.min_offset)
        })
    }
}
fn finite(rect: CssRect) -> bool {
    rect.width >= 0.
        && rect.height >= 0.
        && [
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            rect.right(),
            rect.bottom(),
        ]
        .iter()
        .all(|v| v.is_finite())
}
pub fn geometry(
    projection: &PaintProjection,
    style: &BTreeMap<String, String>,
) -> Vec<ScrollbarGeometry> {
    let Some(scroll) = &projection.scroll else {
        return Vec::new();
    };
    geometry_for_scroll(scroll, projection.box_model.padding_box, style)
}
/// Fixed physical chrome with offsets in layout CSS pixels. The camera scales
/// document content, never the track thickness or thumb hit target.
pub fn geometry_for_viewport_scroll(
    scroll: &crate::v4::PaintScrollGeometry,
    padding: CssRect,
    style: &BTreeMap<String, String>,
    visual: Option<crate::visual_viewport::Geometry>,
) -> Vec<ScrollbarGeometry> {
    let Some(visual) = visual else {
        return geometry_for_scroll(scroll, padding, style);
    };
    if !visual.is_valid()
        || (padding.width - visual.layout_width).abs() > 0.001
        || (padding.height - visual.layout_height).abs() > 0.001
    {
        return Vec::new();
    }
    let port = scroll.scrollport;
    let area = scroll.scrollable;
    let scale = visual.scale;
    let transformed = crate::v4::PaintScrollGeometry {
        offset_x: (scroll.offset_x + visual.offset_x) * scale,
        offset_y: (scroll.offset_y + visual.offset_y) * scale,
        scrollport: port,
        scrollable: CssRect {
            x: port.x + (area.x - port.x) * scale,
            y: port.y + (area.y - port.y) * scale,
            width: area.width * scale,
            height: area.height * scale,
        },
    };
    let mut bars = geometry_for_scroll(&transformed, padding, style);
    for bar in &mut bars {
        bar.min_offset /= scale;
        bar.max_offset /= scale;
        bar.offset /= scale;
        bar.page_step /= scale;
    }
    bars
}

/// Allocate a combined viewport offset to the existing layout scroll and then
/// native visual pan. Preserve layout scroll while the camera can pan; carry
/// only the residual into layout scrolling. Returned values use layout units.
pub fn split_viewport_offset(
    scroll: &crate::v4::PaintScrollGeometry,
    visual: crate::visual_viewport::Geometry,
    horizontal: bool,
    total: f64,
) -> Option<(f64, f64)> {
    if !visual.is_valid()
        || !total.is_finite()
        || !finite(scroll.scrollport)
        || !finite(scroll.scrollable)
    {
        return None;
    }
    let (port_start, port_size, start, end, current, visual_limit) = if horizontal {
        (
            scroll.scrollport.x,
            scroll.scrollport.width,
            scroll.scrollable.x,
            scroll.scrollable.right(),
            scroll.offset_x,
            (visual.content_rect().width - visual.width()).max(0.),
        )
    } else {
        (
            scroll.scrollport.y,
            scroll.scrollport.height,
            scroll.scrollable.y,
            scroll.scrollable.bottom(),
            scroll.offset_y,
            (visual.content_rect().height - visual.height()).max(0.),
        )
    };
    if !current.is_finite() {
        return None;
    }
    let min = (start - port_start).min(0.);
    let max = (end - port_start - port_size).max(0.);
    let camera_max = (port_size - port_size / visual.scale)
        .max(0.)
        .min(visual_limit);
    let combined_max = (end - port_start - port_size / visual.scale).max(0.);
    let total = total.clamp(min, combined_max);
    let camera = (total - current).clamp(0., camera_max);
    let layout = (total - camera).clamp(min, max);
    let camera = (total - layout).clamp(0., camera_max);
    Some((layout, camera))
}
pub fn geometry_for_scroll(
    scroll: &crate::v4::PaintScrollGeometry,
    padding: CssRect,
    style: &BTreeMap<String, String>,
) -> Vec<ScrollbarGeometry> {
    let port = scroll.scrollport;
    let area = scroll.scrollable;
    if !finite(padding)
        || !finite(port)
        || !finite(area)
        || ![scroll.offset_x, scroll.offset_y]
            .iter()
            .all(|v| v.is_finite())
        || style.get("scrollbar-width").is_some_and(|v| v == "none")
    {
        return Vec::new();
    }
    let visible = |axis, extent, size| {
        style
            .get(axis)
            .is_some_and(|value| value == "scroll" || (value == "auto" && extent > size + 0.01))
    };
    let mut result = Vec::new();
    let mut add = |track: CssRect, horizontal: bool| {
        if !finite(track) || track.width <= 0. || track.height <= 0. {
            return;
        }
        let (start, length, thickness, port_start, port_size, content_start, content_end, offset) =
            if horizontal {
                (
                    track.x,
                    track.width,
                    track.height,
                    port.x,
                    port.width,
                    area.x,
                    area.right(),
                    scroll.offset_x,
                )
            } else {
                (
                    track.y,
                    track.height,
                    track.width,
                    port.y,
                    port.height,
                    area.y,
                    area.bottom(),
                    scroll.offset_y,
                )
            };
        let min_offset = (content_start - port_start).min(0.);
        let max_offset = (content_end - port_start - port_size).max(0.);
        let range = max_offset - min_offset;
        if !range.is_finite() {
            return;
        }
        let content_size = port_size + range;
        let thumb_length = (length * port_size
            / content_size.max(port_size).max(f64::MIN_POSITIVE))
        .max((thickness * 1.5).min(length))
        .min(length);
        let offset = offset.clamp(min_offset, max_offset);
        let fraction = if range > 0. {
            (offset - min_offset) / range
        } else {
            0.
        };
        let leading = start + (length - thumb_length) * fraction;
        let inset = thickness.min(4.) / 2.;
        let thumb = if horizontal {
            CssRect {
                x: leading,
                y: track.y + inset,
                width: thumb_length,
                height: (track.height - 2. * inset).max(0.),
            }
        } else {
            CssRect {
                x: track.x + inset,
                y: leading,
                width: (track.width - 2. * inset).max(0.),
                height: thumb_length,
            }
        };
        result.push(ScrollbarGeometry {
            horizontal,
            track,
            thumb,
            min_offset,
            max_offset,
            offset,
            page_step: port_size,
        });
    };
    if visible("overflow-y", area.height, port.height) {
        let left = style.get("direction").is_some_and(|v| v == "rtl")
            && style
                .get("writing-mode")
                .map(String::as_str)
                .unwrap_or("horizontal-tb")
                == "horizontal-tb";
        add(
            if left {
                CssRect {
                    x: padding.x,
                    y: port.y,
                    width: port.x - padding.x,
                    height: port.height,
                }
            } else {
                CssRect {
                    x: port.right(),
                    y: port.y,
                    width: padding.right() - port.right(),
                    height: port.height,
                }
            },
            false,
        );
    }
    if visible("overflow-x", area.width, port.width) {
        add(
            CssRect {
                x: port.x,
                y: port.bottom(),
                width: port.width,
                height: padding.bottom() - port.bottom(),
            },
            true,
        );
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visual_viewport_chrome_uses_physical_track_and_layout_offsets() {
        let scroll = crate::v4::PaintScrollGeometry {
            offset_x: -20.,
            offset_y: 60.,
            scrollport: CssRect {
                x: 0.,
                y: 0.,
                width: 184.,
                height: 84.,
            },
            scrollable: CssRect {
                x: -100.,
                y: 0.,
                width: 500.,
                height: 300.,
            },
        };
        let visual = crate::visual_viewport::Geometry {
            scrollport: None,
            layout_width: 200.,
            layout_height: 100.,
            scale: 2.,
            offset_x: 30.,
            offset_y: 20.,
        };
        let padding = CssRect {
            x: 0.,
            y: 0.,
            width: 200.,
            height: 100.,
        };
        let style = BTreeMap::from([
            ("overflow-x".into(), "auto".into()),
            ("overflow-y".into(), "auto".into()),
        ]);
        let ordinary = geometry_for_scroll(&scroll, padding, &style);
        let zoomed = geometry_for_viewport_scroll(&scroll, padding, &style, Some(visual));
        assert_eq!(zoomed.len(), 2);
        for bar in &zoomed {
            let original = ordinary
                .iter()
                .find(|old| old.horizontal == bar.horizontal)
                .unwrap();
            assert_eq!(bar.track, original.track);
            assert_eq!(bar.page_step, original.page_step / 2.);
            let origin = if bar.horizontal {
                bar.thumb.x
            } else {
                bar.thumb.y
            };
            assert!((bar.offset_at_thumb_origin(origin).unwrap() - bar.offset).abs() < 1e-9);
        }
        let horizontal = zoomed.iter().find(|bar| bar.horizontal).unwrap();
        assert_eq!(
            (
                horizontal.min_offset,
                horizontal.max_offset,
                horizontal.offset
            ),
            (-100., 308., 10.)
        );
        assert_eq!(
            split_viewport_offset(&scroll, visual, true, 0.),
            Some((-20., 20.))
        );
        assert_eq!(
            split_viewport_offset(&scroll, visual, true, 308.),
            Some((216., 92.))
        );
        assert_eq!(
            split_viewport_offset(&scroll, visual, true, -1000.),
            Some((-100., 0.))
        );
        assert!(
            geometry_for_viewport_scroll(
                &scroll,
                padding,
                &style,
                Some(crate::visual_viewport::Geometry {
                    scale: f64::NAN,
                    ..visual
                })
            )
            .is_empty()
        );
    }
    #[test]
    fn negative_scroll_thumb_drag_inverts_paint_position() {
        let bar = ScrollbarGeometry {
            horizontal: true,
            track: CssRect {
                x: 0.,
                y: 84.,
                width: 100.,
                height: 16.,
            },
            thumb: CssRect {
                x: 0.,
                y: 86.,
                width: 50.,
                height: 12.,
            },
            min_offset: -100.,
            max_offset: 0.,
            offset: -100.,
            page_step: 100.,
        };
        assert_eq!(bar.offset_at_thumb_origin(0.), Some(-100.));
        assert_eq!(bar.offset_at_thumb_origin(25.), Some(-50.));
        assert_eq!(bar.offset_at_thumb_origin(100.), Some(0.));
        assert_eq!(bar.offset_at_thumb_origin(f64::NAN), None);
    }
}
