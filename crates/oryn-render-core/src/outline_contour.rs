//! Bounded rectangle-union contours for outlines around inline fragments.
//! The same disjoint paint primitives reach raster and vector PDF consumers.
type BoundarySegment = ([f64; 2], [f64; 2]);

use crate::{PaintCommand, RenderError};
use oryn_common::v4::{CssRect, PaintBorderEdge, PaintClip};
const LIMIT: usize = 65_536;
const ID: [f64; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];
type Span = (f64, f64);
fn failure() -> RenderError {
    RenderError::Encode("inline outline contour exceeds native geometry budget".into())
}
fn expanded(boxes: &[CssRect], offset: f64, stroke: f64) -> Vec<CssRect> {
    boxes
        .iter()
        .map(|b| {
            let x = offset.max(-b.width / 2.) + stroke;
            let y = offset.max(-b.height / 2.) + stroke;
            CssRect {
                x: b.x - x,
                y: b.y - y,
                width: b.width + 2. * x,
                height: b.height + 2. * y,
            }
        })
        .filter(|b| b.width > 0. && b.height > 0.)
        .collect()
}
fn intervals(boxes: &[CssRect], y: f64) -> Vec<Span> {
    let mut spans = boxes
        .iter()
        .filter(|b| b.y <= y && y < b.bottom())
        .map(|b| (b.x, b.right()))
        .collect::<Vec<_>>();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged = Vec::<Span>::new();
    for span in spans {
        if let Some(last) = merged.last_mut().filter(|last| last.1 >= span.0) {
            last.1 = last.1.max(span.1);
        } else {
            merged.push(span);
        }
    }
    merged
}
fn difference(outer: &[Span], inner: &[Span]) -> Vec<Span> {
    let mut result = Vec::new();
    let mut first = 0;
    for &(left, right) in outer {
        while first < inner.len() && inner[first].1 <= left {
            first += 1;
        }
        let mut cursor = left;
        for &(a, b) in &inner[first..] {
            if a >= right {
                break;
            }
            if a > cursor {
                result.push((cursor, a.min(right)));
            }
            cursor = cursor.max(b);
            if cursor >= right {
                break;
            }
        }
        if cursor < right {
            result.push((cursor, right));
        }
    }
    result
}
fn bands(outer: &[CssRect], inner: &[CssRect]) -> Result<Vec<CssRect>, RenderError> {
    if outer.iter().chain(inner).any(|b| {
        ![b.x, b.y, b.width, b.height, b.right(), b.bottom()]
            .iter()
            .all(|value| value.is_finite())
    }) {
        return Err(failure());
    }
    let mut ys = outer
        .iter()
        .chain(inner)
        .flat_map(|b| [b.y, b.bottom()])
        .collect::<Vec<_>>();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    if ys
        .len()
        .checked_mul(outer.len() + inner.len())
        .is_none_or(|work| work > 1_048_576)
    {
        return Err(failure());
    }
    let mut result = Vec::new();
    for pair in ys.windows(2) {
        let spans = difference(&intervals(outer, pair[0]), &intervals(inner, pair[0]));
        for (x, end) in spans {
            result.push(CssRect {
                x,
                y: pair[0],
                width: end - x,
                height: pair[1] - pair[0],
            });
            if result.len() > LIMIT {
                return Err(failure());
            }
        }
    }
    Ok(result)
}
fn ring(
    boxes: &[CssRect],
    offset: f64,
    outer: f64,
    inner: f64,
) -> Result<Vec<CssRect>, RenderError> {
    bands(
        &expanded(boxes, offset, outer),
        &expanded(boxes, offset, inner),
    )
}
fn boundary(boxes: &[CssRect]) -> Result<Vec<BoundarySegment>, RenderError> {
    let tiles = bands(boxes, &[])?;
    let mut ys = tiles
        .iter()
        .flat_map(|b| [b.y, b.bottom()])
        .collect::<Vec<_>>();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let mut edges = Vec::new();
    let mut previous = Vec::new();
    for (index, &y) in ys.iter().enumerate() {
        let current = intervals(boxes, y);
        edges.extend(
            difference(&current, &previous)
                .into_iter()
                .map(|(a, b)| ([a, y], [b, y])),
        );
        edges.extend(
            difference(&previous, &current)
                .into_iter()
                .map(|(a, b)| ([b, y], [a, y])),
        );
        if let Some(next) = ys.get(index + 1) {
            for &(a, b) in &current {
                edges.push(([a, *next], [a, y]));
                edges.push(([b, y], [b, *next]));
            }
        }
        previous = current;
        if edges.len() > LIMIT {
            return Err(failure());
        }
    }
    Ok(edges)
}
fn key(point: [f64; 2]) -> (u64, u64) {
    (
        if point[0] == 0. {
            0
        } else {
            point[0].to_bits()
        },
        if point[1] == 0. {
            0
        } else {
            point[1].to_bits()
        },
    )
}
fn intersect(a: CssRect, b: CssRect) -> Option<CssRect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let r = a.right().min(b.right());
    let d = a.bottom().min(b.bottom());
    (r > x && d > y).then_some(CssRect {
        x,
        y,
        width: r - x,
        height: d - y,
    })
}
fn opaque(mut color: [u8; 4]) -> [u8; 4] {
    color[3] = 255;
    color
}
pub(super) fn commands(
    boxes: &[CssRect],
    edge: &PaintBorderEdge,
    offset: f64,
) -> Result<Vec<PaintCommand>, RenderError> {
    if boxes.len() > LIMIT
        || boxes.iter().any(|b| {
            ![b.x, b.y, b.width, b.height, b.right(), b.bottom()]
                .iter()
                .all(|n| n.is_finite())
                || b.width < 0.
                || b.height < 0.
        })
    {
        return Err(failure());
    }
    let width = edge.width;
    if !width.is_finite() || width <= 0. || !offset.is_finite() {
        return Err(failure());
    }
    let color = crate::parse_color(&edge.color).unwrap_or([0, 0, 0, 255]);
    let fill = opaque(color);
    let mut output = Vec::new();
    let mut work = 0usize;
    match edge.style.as_str() {
        "double" => {
            for (outer, inner) in [(width, width * 2. / 3.), (width / 3., 0.)] {
                for rect in ring(boxes, offset, outer, inner)? {
                    output.push(PaintCommand::FillRect { rect, color: fill });
                }
            }
        }
        "inset" | "outset" | "groove" | "ridge" => {
            let region = ring(boxes, offset, width, 0.)?;
            if region
                .len()
                .checked_mul(boxes.len())
                .is_none_or(|work| work > 1_048_576)
            {
                return Err(failure());
            }
            let edge = PaintBorderEdge {
                color: format!("rgb({},{},{})", fill[0], fill[1], fill[2]),
                ..edge.clone()
            };
            // Reuse native directional bevel shading, clipped to the connected
            // ring so internal fragment sides cannot paint a seam.
            for rect in expanded(boxes, offset, width) {
                for tile in &region {
                    if let Some(clip) = intersect(rect, *tile) {
                        output.push(PaintCommand::Projected {
                            matrix: ID,
                            clips: vec![PaintClip {
                                shape: None,
                                owner: None,
                                scroll_x: false,
                                scroll_y: false,
                                matrix: ID,
                                rect: clip,
                                clip_x: true,
                                clip_y: true,
                            }],
                            commands: vec![PaintCommand::BorderEdges {
                                rect,
                                edges: std::array::from_fn(|_| edge.clone()),
                                radii: [0.; 8],
                            }],
                        });
                        if output.len() > LIMIT {
                            return Err(failure());
                        }
                    }
                }
            }
        }
        "dashed" | "dotted" => {
            let region = ring(boxes, offset, width, 0.)?;
            let edges = boundary(&expanded(boxes, offset, width / 2.))?;
            let mut pending = std::collections::BTreeMap::<(u64, u64), Vec<BoundarySegment>>::new();
            for edge in edges {
                pending.entry(key(edge.0)).or_default().push(edge);
            }
            while let Some(start) = pending.keys().next().copied() {
                let mut current = start;
                let mut distance = 0.;
                while let Some(edges) = pending.get_mut(&current) {
                    let Some((a, b)) = edges.pop() else {
                        pending.remove(&current);
                        break;
                    };
                    if edges.is_empty() {
                        pending.remove(&current);
                    }
                    let length = (b[0] - a[0]).abs() + (b[1] - a[1]).abs();
                    let step = if edge.style == "dotted" {
                        2. * width
                    } else {
                        6. * width
                    };
                    let mut position = (distance / step).floor() * step - distance;
                    while position < length {
                        work = work.saturating_add(region.len().max(1));
                        if work > 1_048_576 {
                            return Err(failure());
                        }
                        let extent = if edge.style == "dotted" {
                            width
                        } else {
                            3. * width
                        };
                        let from = position.max(0.);
                        let to = (position + extent).min(length);
                        if to > from {
                            let dx = (b[0] - a[0]).signum();
                            let dy = (b[1] - a[1]).signum();
                            let first = [a[0] + dx * from, a[1] + dy * from];
                            let last = [a[0] + dx * to, a[1] + dy * to];
                            let rect = if edge.style == "dotted" {
                                let center = position + width / 2.;
                                CssRect {
                                    x: a[0] + dx * center - width / 2.,
                                    y: a[1] + dy * center - width / 2.,
                                    width,
                                    height: width,
                                }
                            } else {
                                CssRect {
                                    x: first[0].min(last[0])
                                        - if dx == 0. { width / 2. } else { 0. },
                                    y: first[1].min(last[1])
                                        - if dy == 0. { width / 2. } else { 0. },
                                    width: if dx == 0. { width } else { to - from },
                                    height: if dy == 0. { width } else { to - from },
                                }
                            };
                            for tile in &region {
                                if let Some(clip) = intersect(rect, *tile) {
                                    if edge.style == "dotted" {
                                        output.push(PaintCommand::Projected {
                                            matrix: ID,
                                            clips: vec![PaintClip {
                                                shape: None,
                                                owner: None,
                                                scroll_x: false,
                                                scroll_y: false,
                                                matrix: ID,
                                                rect: clip,
                                                clip_x: true,
                                                clip_y: true,
                                            }],
                                            commands: vec![PaintCommand::FillRoundedRect {
                                                rect,
                                                color: fill,
                                                radii: [width / 2.; 8],
                                            }],
                                        });
                                    } else {
                                        output.push(PaintCommand::FillRect {
                                            rect: clip,
                                            color: fill,
                                        });
                                    }
                                    if output.len() > LIMIT {
                                        return Err(failure());
                                    }
                                }
                            }
                        }
                        let next = position + step;
                        if next <= position {
                            return Err(failure());
                        }
                        position = next;
                    }
                    distance += length;
                    current = key(b);
                    if current == start {
                        break;
                    }
                }
            }
        }
        _ => {
            for rect in ring(boxes, offset, width, 0.)? {
                output.push(PaintCommand::FillRect { rect, color: fill });
            }
        }
    }
    if color[3] < 255 {
        Ok(vec![PaintCommand::Opacity {
            opacity: f64::from(color[3]) / 255.,
            commands: output,
        }])
    } else {
        Ok(output)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrapped_union_has_no_internal_horizontal_outline() {
        let boxes = [
            CssRect {
                x: 0.,
                y: 0.,
                width: 40.,
                height: 10.,
            },
            CssRect {
                x: 0.,
                y: 10.,
                width: 20.,
                height: 10.,
            },
        ];
        let ring = ring(&boxes, 0., 2., 0.).unwrap();
        assert!(
            !ring
                .iter()
                .any(|b| b.x <= 10. && b.right() > 10. && b.y <= 10. && b.bottom() > 10.)
        );
        assert!(
            ring.iter()
                .any(|b| b.x <= 30. && b.right() > 30. && b.y <= 10. && b.bottom() > 10.)
        );
        for (i, a) in ring.iter().enumerate() {
            assert!(ring[i + 1..].iter().all(|b| intersect(*a, *b).is_none()));
        }
    }
    #[test]
    fn double_negative_offset_and_dashes_share_union_geometry() {
        let boxes = [
            CssRect {
                x: 3.,
                y: 4.,
                width: 40.,
                height: 10.,
            },
            CssRect {
                x: 3.,
                y: 14.,
                width: 20.,
                height: 10.,
            },
        ];
        for style in [
            "solid", "double", "dashed", "dotted", "inset", "outset", "groove", "ridge",
        ] {
            let edge = PaintBorderEdge {
                width: 3.,
                style: style.into(),
                color: "rgba(0,0,0,.5)".into(),
            };
            let commands = commands(&boxes, &edge, -1.).unwrap();
            assert!(matches!(&commands[..], [PaintCommand::Opacity { .. }]));
        }
    }
}
