//! Per-edge border styles share the same local box and project/clip operation
//! as backgrounds, including rounded outer and inner contours.
use super::*;
use oryn_common::v4::PaintBorderEdge;
pub(super) fn radii(style: &std::collections::BTreeMap<String, String>, rect: CssRect) -> [f64; 8] {
    let mut result = [0.; 8];
    for (i, corner) in ["top-left", "top-right", "bottom-right", "bottom-left"]
        .iter()
        .enumerate()
    {
        let value = style
            .get(&format!("border-{corner}-radius"))
            .or_else(|| style.get("border-radius"))
            .map(String::as_str)
            .unwrap_or("0px");
        let parts = value.split_whitespace().collect::<Vec<_>>();
        let length = |v: &str, basis: f64| {
            oryn_common::css_math::resolve_length(
                v,
                oryn_common::css_math::LengthContext {
                    percentage_basis: basis,
                    font_size: 16.,
                    root_font_size: 16.,
                    viewport_width: rect.width,
                    viewport_height: rect.height,
                },
            )
            .unwrap_or(0.)
            .max(0.)
        };
        result[i * 2] = length(parts.first().copied().unwrap_or("0px"), rect.width);
        result[i * 2 + 1] = length(
            parts.get(1).or(parts.first()).copied().unwrap_or("0px"),
            rect.height,
        );
    }
    let mut scale = 1_f64;
    for (available, used) in [
        (rect.width, result[0] + result[2]),
        (rect.width, result[6] + result[4]),
        (rect.height, result[1] + result[7]),
        (rect.height, result[3] + result[5]),
    ] {
        if used > 0. {
            scale = scale.min(available / used);
        }
    }
    result.map(|v| v * scale)
}
pub(super) fn inside(rect: CssRect, r: [f64; 8], x: f64, y: f64) -> bool {
    if rect.width <= 0.
        || rect.height <= 0.
        || x < rect.x
        || x > rect.right()
        || y < rect.y
        || y > rect.bottom()
    {
        return false;
    }
    for (i, cx, cy, left, top) in [
        (0, rect.x + r[0], rect.y + r[1], true, true),
        (1, rect.right() - r[2], rect.y + r[3], false, true),
        (2, rect.right() - r[4], rect.bottom() - r[5], false, false),
        (3, rect.x + r[6], rect.bottom() - r[7], true, false),
    ] {
        let (rx, ry) = (r[i * 2], r[i * 2 + 1]);
        if rx > 0.
            && ry > 0.
            && (if left { x < cx } else { x > cx })
            && (if top { y < cy } else { y > cy })
            && ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) > 1.
        {
            return false;
        }
    }
    true
}
pub(super) fn sample(
    rect: CssRect,
    edges: &[PaintBorderEdge; 4],
    r: [f64; 8],
    colors: [[u8; 4]; 4],
    x: f64,
    y: f64,
) -> [u8; 4] {
    if !inside(rect, r, x, y) {
        return [0; 4];
    }
    let widths = edges.each_ref().map(|e| {
        if matches!(e.style.as_str(), "none" | "hidden") {
            0.
        } else {
            e.width.max(0.)
        }
    });
    let inner = CssRect {
        x: rect.x + widths[3],
        y: rect.y + widths[0],
        width: rect.width - widths[3] - widths[1],
        height: rect.height - widths[0] - widths[2],
    };
    let ir = [
        (r[0] - widths[3]).max(0.),
        (r[1] - widths[0]).max(0.),
        (r[2] - widths[1]).max(0.),
        (r[3] - widths[0]).max(0.),
        (r[4] - widths[1]).max(0.),
        (r[5] - widths[2]).max(0.),
        (r[6] - widths[3]).max(0.),
        (r[7] - widths[2]).max(0.),
    ];
    if inside(inner, ir, x, y) {
        return [0; 4];
    }
    let distances = [y - rect.y, rect.right() - x, rect.bottom() - y, x - rect.x];
    let Some(side) = (0..4)
        .filter(|i| widths[*i] > 0.)
        .min_by(|a, b| (distances[*a] / widths[*a]).total_cmp(&(distances[*b] / widths[*b])))
    else {
        return [0; 4];
    };
    let width = widths[side];
    let depth = distances[side] / width;
    let along = if side % 2 == 0 {
        x - rect.x
    } else {
        y - rect.y
    };
    let mut color = colors[side];
    match edges[side].style.as_str() {
        "double" if depth > 1. / 3. && depth < 2. / 3. => return [0; 4],
        "dashed" if along.rem_euclid(width * 6.) >= width * 3. => return [0; 4],
        "dotted" => {
            let dx = along.rem_euclid(width * 2.) - width / 2.;
            let dy = distances[side] - width / 2.;
            if dx * dx + dy * dy > width * width / 4. {
                return [0; 4];
            }
        }
        "inset" | "outset" | "groove" | "ridge" => {
            let mut dark = side == 0 || side == 3;
            if matches!(edges[side].style.as_str(), "outset" | "ridge") {
                dark = !dark;
            }
            if matches!(edges[side].style.as_str(), "groove" | "ridge") && depth > 0.5 {
                dark = !dark;
            }
            for channel in &mut color[..3] {
                *channel = if dark {
                    (*channel as f64 * 0.6) as u8
                } else {
                    (*channel as f64 + (255. - *channel as f64) * 0.4) as u8
                };
            }
        }
        _ => {}
    }
    color
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edge_styles_colors_and_rounding_share_one_border_ring() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 40.,
            height: 40.,
        };
        let mut edges = std::array::from_fn(|_| PaintBorderEdge {
            width: 6.,
            style: "solid".into(),
            color: "red".into(),
        });
        let colors = [
            [255, 0, 0, 255],
            [0, 128, 0, 255],
            [0, 0, 255, 255],
            [0, 0, 0, 255],
        ];
        assert_eq!(sample(rect, &edges, [0.; 8], colors, 39., 20.), colors[1]);
        edges[0].style = "double".into();
        assert_eq!(sample(rect, &edges, [0.; 8], colors, 20., 3.), [0; 4]);
        assert_eq!(sample(rect, &edges, [10.; 8], colors, 0.1, 0.1), [0; 4]);
        edges[1].style = "hidden".into();
        assert_eq!(sample(rect, &edges, [0.; 8], colors, 39., 20.), [0; 4]);
    }
}
