//! Native used CSS basic shapes shared by paint, hit testing and observation.
use crate::{
    css_math::{self, LengthContext},
    v4::CssRect,
};
use serde::{Deserialize, Serialize};
mod commands;
mod path;
mod region;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    RoundedRect {
        rect: CssRect,
        radii: [[f64; 2]; 4],
    },
    Ellipse {
        center: [f64; 2],
        radius: [f64; 2],
    },
    Polygon {
        points: Vec<[f64; 2]>,
        even_odd: bool,
    },
    Path {
        contours: Vec<Vec<[f64; 2]>>,
        even_odd: bool,
        #[serde(default)]
        source: Option<(String, [f64; 2])>,
    },
    Union(Vec<Shape>),
    Intersection(Vec<Shape>),
    Affine {
        matrix: [f64; 6],
        shape: Box<Shape>,
    },
}
impl Shape {
    pub fn bounds(&self) -> CssRect {
        match self {
            Self::Union(shapes) => bounds_union(shapes.iter().map(Shape::bounds)),
            Self::Intersection(shapes) => {
                let Some(first) = shapes.first() else {
                    return CssRect {
                        x: 0.,
                        y: 0.,
                        width: 0.,
                        height: 0.,
                    };
                };
                let mut rect = first.bounds();
                for shape in &shapes[1..] {
                    let next = shape.bounds();
                    let x = rect.x.max(next.x);
                    let y = rect.y.max(next.y);
                    rect = CssRect {
                        x,
                        y,
                        width: (rect.right().min(next.right()) - x).max(0.),
                        height: (rect.bottom().min(next.bottom()) - y).max(0.),
                    };
                }
                rect
            }
            Self::Affine { matrix, shape } => affine_rect(shape.bounds(), *matrix),
            Self::RoundedRect { rect, .. } => *rect,
            Self::Ellipse {
                center: [x, y],
                radius: [rx, ry],
            } => CssRect {
                x: x - rx,
                y: y - ry,
                width: 2. * rx,
                height: 2. * ry,
            },
            Self::Path { contours, .. } => {
                let points = contours.iter().flatten().copied().collect::<Vec<_>>();
                Self::Polygon {
                    points,
                    even_odd: false,
                }
                .bounds()
            }
            Self::Polygon { points, .. } => {
                let (mut x, mut y, mut right, mut bottom) = (
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                );
                for point in points {
                    x = x.min(point[0]);
                    y = y.min(point[1]);
                    right = right.max(point[0]);
                    bottom = bottom.max(point[1]);
                }
                if points.is_empty() {
                    CssRect {
                        x: 0.,
                        y: 0.,
                        width: 0.,
                        height: 0.,
                    }
                } else {
                    CssRect {
                        x,
                        y,
                        width: right - x,
                        height: bottom - y,
                    }
                }
            }
        }
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        if !x.is_finite() || !y.is_finite() {
            return false;
        }
        match self {
            Self::Union(shapes) => shapes.iter().any(|shape| shape.contains(x, y)),
            Self::Intersection(shapes) => {
                !shapes.is_empty() && shapes.iter().all(|shape| shape.contains(x, y))
            }
            Self::Affine { matrix, shape } => inverse_affine(*matrix).is_some_and(|matrix| {
                let point = affine_point([x, y], matrix);
                shape.contains(point[0], point[1])
            }),
            Self::Ellipse { center, radius } => {
                radius[0] > 0.
                    && radius[1] > 0.
                    && ((x - center[0]) / radius[0]).powi(2) + ((y - center[1]) / radius[1]).powi(2)
                        <= 1.
            }
            Self::RoundedRect { rect, radii } => {
                if rect.width <= 0.
                    || rect.height <= 0.
                    || x < rect.x
                    || x > rect.right()
                    || y < rect.y
                    || y > rect.bottom()
                {
                    return false;
                }
                for (index, [rx, ry]) in radii.iter().copied().enumerate() {
                    let right = matches!(index, 1 | 2);
                    let bottom = index >= 2;
                    let cx = if right {
                        rect.right() - rx
                    } else {
                        rect.x + rx
                    };
                    let cy = if bottom {
                        rect.bottom() - ry
                    } else {
                        rect.y + ry
                    };
                    if rx > 0.
                        && ry > 0.
                        && (if right { x > cx } else { x < cx })
                        && (if bottom { y > cy } else { y < cy })
                        && ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) > 1.
                    {
                        return false;
                    }
                }
                true
            }
            Self::Polygon { points, even_odd } => {
                contains_contours(std::slice::from_ref(points), *even_odd, x, y)
            }
            Self::Path {
                contours, even_odd, ..
            } => contains_contours(contours, *even_odd, x, y),
        }
    }
}
fn affine_point(point: [f64; 2], m: [f64; 6]) -> [f64; 2] {
    [
        m[0] * point[0] + m[2] * point[1] + m[4],
        m[1] * point[0] + m[3] * point[1] + m[5],
    ]
}
fn inverse_affine(m: [f64; 6]) -> Option<[f64; 6]> {
    let d = m[0] * m[3] - m[1] * m[2];
    if d == 0. || !d.is_finite() {
        return None;
    }
    Some([
        m[3] / d,
        -m[1] / d,
        -m[2] / d,
        m[0] / d,
        (m[2] * m[5] - m[3] * m[4]) / d,
        (m[1] * m[4] - m[0] * m[5]) / d,
    ])
}
fn affine_rect(rect: CssRect, m: [f64; 6]) -> CssRect {
    bounds_union(
        [
            [rect.x, rect.y],
            [rect.right(), rect.y],
            [rect.right(), rect.bottom()],
            [rect.x, rect.bottom()],
        ]
        .into_iter()
        .map(|point| {
            let point = affine_point(point, m);
            CssRect {
                x: point[0],
                y: point[1],
                width: 0.,
                height: 0.,
            }
        }),
    )
}
fn bounds_union(rects: impl Iterator<Item = CssRect>) -> CssRect {
    let (mut x, mut y, mut right, mut bottom) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for rect in rects {
        x = x.min(rect.x);
        y = y.min(rect.y);
        right = right.max(rect.right());
        bottom = bottom.max(rect.bottom());
    }
    if x.is_finite() && y.is_finite() && right.is_finite() && bottom.is_finite() {
        CssRect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    } else {
        CssRect {
            x: 0.,
            y: 0.,
            width: 0.,
            height: 0.,
        }
    }
}
fn contains_contours(contours: &[Vec<[f64; 2]>], even_odd: bool, x: f64, y: f64) -> bool {
    let mut winding = 0i32;
    for points in contours {
        for (a, b) in points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .take(points.len())
        {
            let cross = (b[0] - a[0]) * (y - a[1]) - (x - a[0]) * (b[1] - a[1]);
            if cross == 0.
                && x >= a[0].min(b[0])
                && x <= a[0].max(b[0])
                && y >= a[1].min(b[1])
                && y <= a[1].max(b[1])
            {
                return true;
            }
            if a[1] <= y && b[1] > y && cross > 0. {
                winding += 1
            } else if a[1] > y && b[1] <= y && cross < 0. {
                winding -= 1
            }
        }
    }
    if even_odd {
        winding % 2 != 0
    } else {
        winding != 0
    }
}
fn words(source: &str) -> Option<Vec<&str>> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (index, ch) in source.char_indices() {
        if ch.is_ascii_whitespace() && depth == 0 {
            if let Some(start) = start.take() {
                result.push(&source[start..index]);
            }
        } else {
            if start.is_none() {
                start = Some(index)
            }
            if ch == '(' {
                depth += 1
            } else if ch == ')' {
                depth = depth.checked_sub(1)?;
            }
        }
    }
    if depth != 0 {
        return None;
    }
    if let Some(start) = start {
        result.push(&source[start..]);
    }
    Some(result)
}
fn separated(source: &str, separator: char) -> Option<Vec<&str>> {
    let mut output = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, ch) in source.char_indices() {
        if ch == '(' {
            depth = depth.checked_add(1)?;
            if depth > 32 {
                return None;
            }
        } else if ch == ')' {
            depth = depth.checked_sub(1)?;
        } else if ch == separator && depth == 0 {
            output.push(source[start..index].trim());
            start = index + ch.len_utf8();
        }
    }
    if depth != 0 {
        return None;
    }
    output.push(source[start..].trim());
    Some(output)
}
fn four<T: Copy>(values: &[T]) -> Option<[T; 4]> {
    Some(match values {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return None,
    })
}
fn contexts(rect: CssRect, font: f64, viewport: (f64, f64)) -> (LengthContext, LengthContext) {
    let x = LengthContext {
        percentage_basis: rect.width,
        font_size: font,
        root_font_size: 16.,
        viewport_width: viewport.0,
        viewport_height: viewport.1,
    };
    (
        x,
        LengthContext {
            percentage_basis: rect.height,
            ..x
        },
    )
}
fn radii(
    source: Option<&str>,
    rect: CssRect,
    x: LengthContext,
    y: LengthContext,
) -> Option<[[f64; 2]; 4]> {
    let x = LengthContext {
        percentage_basis: rect.width,
        ..x
    };
    let y = LengthContext {
        percentage_basis: rect.height,
        ..y
    };
    let Some(source) = source else {
        return Some([[0.; 2]; 4]);
    };
    let sides = separated(source, '/')?;
    if sides.len() > 2 {
        return None;
    }
    let a = words(sides[0])?;
    let b = sides
        .get(1)
        .map(|source| words(source))
        .transpose_option()?;
    let a = four(&a)?;
    let b = match b {
        Some(b) => four(&b)?,
        None => a,
    };
    let mut result = [[0.; 2]; 4];
    for i in 0..4 {
        result[i] = [
            css_math::resolve_length(a[i], x)?,
            css_math::resolve_length(b[i], y)?,
        ];
        if result[i].iter().any(|n| *n < 0.) {
            return None;
        }
    }
    let mut factor = 1f64;
    for (total, size) in [
        (result[0][0] + result[1][0], rect.width),
        (result[3][0] + result[2][0], rect.width),
        (result[0][1] + result[3][1], rect.height),
        (result[1][1] + result[2][1], rect.height),
    ] {
        if total > 0. {
            factor = factor.min(size / total)
        }
    }
    for radius in &mut result {
        radius[0] *= factor.max(0.);
        radius[1] *= factor.max(0.);
    }
    Some(result)
}
trait TransposeOption<T> {
    fn transpose_option(self) -> Option<Option<T>>;
}
impl<T> TransposeOption<T> for Option<Option<T>> {
    fn transpose_option(self) -> Option<Option<T>> {
        match self {
            Some(Some(v)) => Some(Some(v)),
            Some(None) => None,
            None => Some(None),
        }
    }
}
/// Parse computed basic-shape values against their selected geometry reference
/// box. Unresolved URL/path functions are reported to the caller, never treated
/// as an absent clipping operation.
pub fn parse(
    source: &str,
    reference: CssRect,
    font: f64,
    viewport: (f64, f64),
) -> Result<Option<Shape>, String> {
    let source = source.trim();
    if source == "none" {
        return Ok(None);
    }
    if source.len() > 65536 {
        return Err("basic shape source limit".into());
    }
    let open = source.find('(').ok_or("basic shape function missing")?;
    let close = source.rfind(')').ok_or("basic shape function incomplete")?;
    if !source[close + 1..].trim().is_empty() {
        return Err("basic shape reference box must be selected by caller".into());
    }
    let kind = source[..open].trim();
    let body = &source[open + 1..close];
    let (x, y) = contexts(reference, font, viewport);
    let shape = (|| -> Option<Shape> {
        match kind {
            "inset" => {
                let parts = words(body)?;
                let round = parts.iter().position(|part| *part == "round");
                let inset = four(&parts[..round.unwrap_or(parts.len())])?;
                let corners = round.map(|round| parts[round + 1..].join(" "));
                let offsets = [
                    css_math::resolve_length(inset[0], y)?,
                    css_math::resolve_length(inset[1], x)?,
                    css_math::resolve_length(inset[2], y)?,
                    css_math::resolve_length(inset[3], x)?,
                ];
                let mut top = offsets[0];
                let mut right = offsets[1];
                let mut bottom = offsets[2];
                let mut left = offsets[3];
                if top + bottom > reference.height {
                    let scale = reference.height / (top + bottom);
                    top *= scale;
                    bottom *= scale;
                }
                if left + right > reference.width {
                    let scale = reference.width / (left + right);
                    left *= scale;
                    right *= scale;
                }
                let rect = CssRect {
                    x: reference.x + left,
                    y: reference.y + top,
                    width: (reference.width - left - right).max(0.),
                    height: (reference.height - top - bottom).max(0.),
                };
                Some(Shape::RoundedRect {
                    rect,
                    radii: radii(corners.as_deref(), rect, x, y)?,
                })
            }
            "xywh" | "rect" => {
                let tokens = words(body)?;
                let round = tokens.iter().position(|token| *token == "round");
                let edges = &tokens[..round.unwrap_or(tokens.len())];
                if edges.len() != 4 {
                    return None;
                }
                let corners = round.map(|index| tokens[index + 1..].join(" "));
                let rect = if kind == "xywh" {
                    let left = css_math::resolve_length(edges[0], x)?;
                    let top = css_math::resolve_length(edges[1], y)?;
                    let width = css_math::resolve_length(edges[2], x)?;
                    let height = css_math::resolve_length(edges[3], y)?;
                    if width < 0. || height < 0. {
                        return None;
                    }
                    CssRect {
                        x: reference.x + left,
                        y: reference.y + top,
                        width,
                        height,
                    }
                } else {
                    let edge = |value: &str, context: LengthContext, auto: f64| {
                        if value == "auto" {
                            Some(auto)
                        } else {
                            css_math::resolve_length(value, context)
                        }
                    };
                    let top = edge(edges[0], y, 0.)?;
                    let right = edge(edges[1], x, reference.width)?;
                    let bottom = edge(edges[2], y, reference.height)?;
                    let left = edge(edges[3], x, 0.)?;
                    CssRect {
                        x: reference.x + left,
                        y: reference.y + top,
                        width: (right - left).max(0.),
                        height: (bottom - top).max(0.),
                    }
                };
                Some(Shape::RoundedRect {
                    rect,
                    radii: radii(corners.as_deref(), rect, x, y)?,
                })
            }
            "circle" | "ellipse" => {
                let tokens = words(body)?;
                let at = tokens.iter().position(|token| *token == "at");
                let (radius, position) = match at {
                    Some(at) => (&tokens[..at], tokens[at + 1..].join(" ")),
                    None => (tokens.as_slice(), "center".into()),
                };
                let (cx, cy) = css_math::resolve_position(&position, x, y)?;
                let radius_value =
                    |token: &str, position: f64, size: f64, context: LengthContext| match token {
                        "closest-side" => Some(position.abs().min((size - position).abs())),
                        "farthest-side" => Some(position.abs().max((size - position).abs())),
                        _ => css_math::resolve_length(token, context),
                    };
                let radii = if kind == "circle" {
                    if radius.len() > 1 {
                        return None;
                    }
                    let token = radius.first().copied().unwrap_or("closest-side");
                    let value = match token {
                        "closest-side" => cx
                            .abs()
                            .min((reference.width - cx).abs())
                            .min(cy.abs())
                            .min((reference.height - cy).abs()),
                        "farthest-side" => cx
                            .abs()
                            .max((reference.width - cx).abs())
                            .max(cy.abs())
                            .max((reference.height - cy).abs()),
                        _ => css_math::resolve_length(
                            token,
                            LengthContext {
                                percentage_basis: (reference.width.powi(2)
                                    + reference.height.powi(2))
                                .sqrt()
                                    / 2f64.sqrt(),
                                ..x
                            },
                        )?,
                    };
                    [value; 2]
                } else {
                    let radius = if radius.is_empty() {
                        ["closest-side"; 2]
                    } else if radius.len() == 2 {
                        [radius[0], radius[1]]
                    } else {
                        return None;
                    };
                    [
                        radius_value(radius[0], cx, reference.width, x)?,
                        radius_value(radius[1], cy, reference.height, y)?,
                    ]
                };
                if radii.iter().any(|n| *n < 0.) {
                    return None;
                }
                Some(Shape::Ellipse {
                    center: [reference.x + cx, reference.y + cy],
                    radius: radii,
                })
            }
            "shape" => {
                let (mut contours, even_odd, source) = commands::parse(body, x, y)?;
                for point in contours.iter_mut().flatten() {
                    point[0] += reference.x;
                    point[1] += reference.y;
                }
                Some(Shape::Path {
                    contours,
                    even_odd,
                    source: Some((source, [reference.x, reference.y])),
                })
            }
            "path" => {
                let mut input = cssparser::ParserInput::new(body);
                let mut parser = cssparser::Parser::new(&mut input);
                let checkpoint = parser.state();
                let even_odd = if parser
                    .try_parse(|parser| parser.expect_ident_matching("evenodd"))
                    .is_ok()
                {
                    parser.expect_comma().ok()?;
                    true
                } else if parser
                    .try_parse(|parser| parser.expect_ident_matching("nonzero"))
                    .is_ok()
                {
                    parser.expect_comma().ok()?;
                    false
                } else {
                    parser.reset(&checkpoint);
                    false
                };
                let source = parser.expect_string().ok()?.to_string();
                parser.expect_exhausted().ok()?;
                let mut contours = path::parse(&source).ok()?;
                for point in contours.iter_mut().flatten() {
                    point[0] += reference.x;
                    point[1] += reference.y;
                }
                Some(Shape::Path {
                    contours,
                    even_odd,
                    source: Some((source, [reference.x, reference.y])),
                })
            }
            "polygon" => {
                let coordinates = separated(body, ',')?;
                let mut coordinates = coordinates.into_iter();
                let first = coordinates.next()?.trim();
                let even_odd = first == "evenodd";
                let mut points = Vec::new();
                for pair in (if matches!(first, "evenodd" | "nonzero") {
                    None
                } else {
                    Some(first)
                })
                .into_iter()
                .chain(coordinates)
                {
                    if points.len() >= 4096 {
                        return None;
                    }
                    let pair = words(pair)?;
                    if pair.len() != 2 {
                        return None;
                    }
                    points.push([
                        reference.x + css_math::resolve_length(pair[0], x)?,
                        reference.y + css_math::resolve_length(pair[1], y)?,
                    ]);
                }
                if points.len() < 3 {
                    return None;
                }
                Some(Shape::Polygon { points, even_odd })
            }
            _ => None,
        }
    })();
    shape
        .map(Some)
        .ok_or_else(|| format!("unresolved native basic shape: {kind}"))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rect() -> CssRect {
        CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        }
    }
    #[test]
    fn inset_collapses_and_rounded_corners_clip() {
        let shape = parse("inset(1000px)", rect(), 16., (100., 100.))
            .unwrap()
            .unwrap();
        assert_eq!(shape.bounds().width, 0.);
        assert!(!shape.contains(50., 50.));
        let shape = parse("inset(0 round 20px)", rect(), 16., (100., 100.))
            .unwrap()
            .unwrap();
        assert!(!shape.contains(0., 0.));
        assert!(shape.contains(20., 0.));
    }
    #[test]
    fn percentage_ellipse_and_nonconvex_polygon_use_native_geometry() {
        let shape = parse("ellipse(25% 50% at center)", rect(), 16., (100., 100.))
            .unwrap()
            .unwrap();
        assert!(shape.contains(50., 99.));
        assert!(!shape.contains(10., 50.));
        let shape = parse(
            "polygon(0 0, 100% 0, 100% 20%, 20% 20%, 20% 100%, 0 100%)",
            rect(),
            16.,
            (100., 100.),
        )
        .unwrap()
        .unwrap();
        assert!(shape.contains(10., 90.));
        assert!(!shape.contains(90., 90.));
    }
}

impl Shape {
    /// Exact axis-aligned bounds of the shape/rectangle intersection in this
    /// shape's coordinate plane. Curve/edge intersections retain fractional values;
    /// no sampling grid or polygonal approximation determines observation geometry.
    pub fn intersection_bounds(&self, rect: CssRect) -> Option<CssRect> {
        match self {
            Self::Intersection(shapes) => {
                let mut result = rect;
                if shapes.is_empty() {
                    return None;
                }
                for shape in shapes {
                    result = shape.intersection_bounds(result)?;
                }
                return Some(result);
            }
            Self::Union(shapes) => {
                let intersections = shapes
                    .iter()
                    .filter_map(|shape| shape.intersection_bounds(rect))
                    .collect::<Vec<_>>();
                return (!intersections.is_empty())
                    .then(|| bounds_union(intersections.into_iter()));
            }
            Self::Affine { matrix, shape } => {
                let inverse = inverse_affine(*matrix)?;
                let local = shape.intersection_bounds(affine_rect(rect, inverse))?;
                let projected = affine_rect(local, *matrix);
                let x = rect.x.max(projected.x);
                let y = rect.y.max(projected.y);
                let right = rect.right().min(projected.right());
                let bottom = rect.bottom().min(projected.bottom());
                return (right >= x && bottom >= y).then_some(CssRect {
                    x,
                    y,
                    width: right - x,
                    height: bottom - y,
                });
            }
            _ => {}
        }
        if rect.width < 0.
            || rect.height < 0.
            || self.bounds().width <= 0.
            || self.bounds().height <= 0.
        {
            return None;
        }
        let inside = |point: [f64; 2]| {
            point[0] >= rect.x
                && point[0] <= rect.right()
                && point[1] >= rect.y
                && point[1] <= rect.bottom()
        };
        let mut points = Vec::new();
        for point in [
            [rect.x, rect.y],
            [rect.right(), rect.y],
            [rect.right(), rect.bottom()],
            [rect.x, rect.bottom()],
        ] {
            if self.contains(point[0], point[1]) {
                points.push(point)
            }
        }
        let mut segment = |a: [f64; 2], b: [f64; 2]| {
            if inside(a) {
                points.push(a)
            }
            if inside(b) {
                points.push(b)
            }
            for (axis, edges) in [(0, [rect.x, rect.right()]), (1, [rect.y, rect.bottom()])] {
                let delta = b[axis] - a[axis];
                if delta == 0. {
                    continue;
                }
                for edge in edges {
                    let t = (edge - a[axis]) / delta;
                    if (0. ..=1.).contains(&t) {
                        let point = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                        if inside(point) {
                            points.push(point)
                        }
                    }
                }
            }
        };
        match self {
            Self::Path { contours, .. } => {
                for polygon in contours {
                    for (a, b) in polygon
                        .iter()
                        .zip(polygon.iter().cycle().skip(1))
                        .take(polygon.len())
                    {
                        segment(*a, *b)
                    }
                }
            }
            Self::Polygon {
                points: polygon, ..
            } => {
                for (a, b) in polygon
                    .iter()
                    .zip(polygon.iter().cycle().skip(1))
                    .take(polygon.len())
                {
                    segment(*a, *b)
                }
            }
            Self::RoundedRect { rect: shape, radii } => {
                segment(
                    [shape.x + radii[0][0], shape.y],
                    [shape.right() - radii[1][0], shape.y],
                );
                segment(
                    [shape.right(), shape.y + radii[1][1]],
                    [shape.right(), shape.bottom() - radii[2][1]],
                );
                segment(
                    [shape.right() - radii[2][0], shape.bottom()],
                    [shape.x + radii[3][0], shape.bottom()],
                );
                segment(
                    [shape.x, shape.bottom() - radii[3][1]],
                    [shape.x, shape.y + radii[0][1]],
                );
            }
            _ => {}
        }
        let mut ellipse = |center: [f64; 2], radius: [f64; 2], quadrant: Option<usize>| {
            if radius[0] <= 0. || radius[1] <= 0. {
                return;
            }
            let allowed = |point: [f64; 2]| {
                inside(point)
                    && quadrant.is_none_or(|index| {
                        let right = matches!(index, 1 | 2);
                        let bottom = index >= 2;
                        (if right {
                            point[0] >= center[0]
                        } else {
                            point[0] <= center[0]
                        }) && (if bottom {
                            point[1] >= center[1]
                        } else {
                            point[1] <= center[1]
                        })
                    })
            };
            for point in [
                [center[0] - radius[0], center[1]],
                [center[0] + radius[0], center[1]],
                [center[0], center[1] - radius[1]],
                [center[0], center[1] + radius[1]],
            ] {
                if allowed(point) {
                    points.push(point)
                }
            }
            for x in [rect.x, rect.right()] {
                let square = 1. - ((x - center[0]) / radius[0]).powi(2);
                if square >= 0. {
                    for y in [
                        center[1] - radius[1] * square.sqrt(),
                        center[1] + radius[1] * square.sqrt(),
                    ] {
                        let point = [x, y];
                        if allowed(point) {
                            points.push(point)
                        }
                    }
                }
            }
            for y in [rect.y, rect.bottom()] {
                let square = 1. - ((y - center[1]) / radius[1]).powi(2);
                if square >= 0. {
                    for x in [
                        center[0] - radius[0] * square.sqrt(),
                        center[0] + radius[0] * square.sqrt(),
                    ] {
                        let point = [x, y];
                        if allowed(point) {
                            points.push(point)
                        }
                    }
                }
            }
        };
        match self {
            Self::Ellipse { center, radius } => ellipse(*center, *radius, None),
            Self::RoundedRect { rect, radii } => {
                for (index, radius) in radii.iter().copied().enumerate() {
                    let center = [
                        if matches!(index, 1 | 2) {
                            rect.right() - radius[0]
                        } else {
                            rect.x + radius[0]
                        },
                        if index >= 2 {
                            rect.bottom() - radius[1]
                        } else {
                            rect.y + radius[1]
                        },
                    ];
                    ellipse(center, radius, Some(index))
                }
            }
            _ => {}
        }
        let first = *points.first()?;
        let (mut x, mut y, mut right, mut bottom) = (first[0], first[1], first[0], first[1]);
        for point in points {
            x = x.min(point[0]);
            y = y.min(point[1]);
            right = right.max(point[0]);
            bottom = bottom.max(point[1]);
        }
        Some(CssRect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;
    fn reference() -> CssRect {
        CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        }
    }
    #[test]
    fn compound_path_retains_holes_and_curves() {
        let shape = parse(
            "path(evenodd, 'M0 0H100V100H0Z M25 25H75V75H25Z')",
            reference(),
            16.,
            (100., 100.),
        )
        .unwrap()
        .unwrap();
        assert!(shape.contains(10., 10.));
        assert!(!shape.contains(50., 50.));
        assert!(
            shape
                .intersection_bounds(CssRect {
                    x: 40.,
                    y: 40.,
                    width: 10.,
                    height: 10.
                })
                .is_none()
        );
        let curve = parse(
            "path('M0 0 C0 100 100 100 100 0Z')",
            reference(),
            16.,
            (100., 100.),
        )
        .unwrap()
        .unwrap();
        assert!(curve.contains(50., 40.));
        assert!(!curve.contains(50., 80.));
        assert!((curve.bounds().height - 75.).abs() < 0.002);
    }
    #[test]
    fn arc_and_relative_smooth_commands_have_bounded_native_contours() {
        let circle = parse(
            "path('M0 50 A50 50 0 1 0 100 50 A50 50 0 1 0 0 50 Z')",
            reference(),
            16.,
            (100., 100.),
        )
        .unwrap()
        .unwrap();
        assert!(circle.contains(50., 50.));
        assert!(!circle.contains(0., 0.));
        assert!((circle.bounds().width - 100.).abs() < 0.002);
        let smooth = parse(
            "path('m0 0 q25 50 50 0 t50 0 l0 100 h-100z')",
            reference(),
            16.,
            (100., 100.),
        )
        .unwrap()
        .unwrap();
        assert!(smooth.contains(50., 50.));
        assert!(parse("path('M0 0 X10 10')", reference(), 16., (100., 100.)).is_err());
    }
}

/// SVG transform-list semantics, including the parser's normalization of
/// rotations about a specified center into elementary affine transforms.
pub fn svg_transform(source: &str) -> Result<[f64; 6], String> {
    let mut matrix = [1., 0., 0., 1., 0., 0.];
    let mut count = 0;
    for token in svgtypes::TransformListParser::from(source) {
        count += 1;
        if count > 1024 {
            return Err("SVG clip transform count".into());
        }
        let next = match token.map_err(|error| error.to_string())? {
            svgtypes::TransformListToken::Matrix { a, b, c, d, e, f } => [a, b, c, d, e, f],
            svgtypes::TransformListToken::Translate { tx, ty } => [1., 0., 0., 1., tx, ty],
            svgtypes::TransformListToken::Scale { sx, sy } => [sx, 0., 0., sy, 0., 0.],
            svgtypes::TransformListToken::Rotate { angle } => {
                let angle = angle.to_radians();
                [angle.cos(), angle.sin(), -angle.sin(), angle.cos(), 0., 0.]
            }
            svgtypes::TransformListToken::SkewX { angle } => {
                [1., 0., angle.to_radians().tan(), 1., 0., 0.]
            }
            svgtypes::TransformListToken::SkewY { angle } => {
                [1., angle.to_radians().tan(), 0., 1., 0., 0.]
            }
        };
        matrix = [
            matrix[0] * next[0] + matrix[2] * next[1],
            matrix[1] * next[0] + matrix[3] * next[1],
            matrix[0] * next[2] + matrix[2] * next[3],
            matrix[1] * next[2] + matrix[3] * next[3],
            matrix[0] * next[4] + matrix[2] * next[5] + matrix[4],
            matrix[1] * next[4] + matrix[3] * next[5] + matrix[5],
        ];
        if !matrix.iter().all(|value| value.is_finite()) {
            return Err("SVG clip transform is nonfinite".into());
        }
    }
    Ok(matrix)
}

pub fn svg_points(source: &str) -> Result<Vec<[f64; 2]>, String> {
    let mut points = Vec::new();
    for (x, y) in svgtypes::PointsParser::from(source) {
        if points.len() >= 65536 || !x.is_finite() || !y.is_finite() {
            return Err("SVG clip point budget".into());
        }
        points.push([x, y]);
    }
    Ok(points)
}

pub fn intersect_regions(shapes: &[(Shape, [f64; 16])]) -> Result<Option<CssRect>, String> {
    region::intersect(shapes)
}

impl Shape {
    /// Refine retained SVG commands to a precision measured after projection.
    /// A finite homography derivative bound converts output tolerance into a local
    /// curve tolerance; horizon crossings or excessive subdivision fail explicitly.
    pub fn refined_for_projection(
        &self,
        matrix: [f64; 16],
        tolerance: f64,
    ) -> Result<Shape, String> {
        let multiply = |a: [f64; 16], b: [f64; 16]| {
            std::array::from_fn(|i| {
                let row = i % 4;
                let col = i / 4;
                (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum()
            })
        };
        Ok(match self {
            Self::Union(shapes) => Self::Union(
                shapes
                    .iter()
                    .map(|shape| shape.refined_for_projection(matrix, tolerance))
                    .collect::<Result<_, _>>()?,
            ),
            Self::Intersection(shapes) => Self::Intersection(
                shapes
                    .iter()
                    .map(|shape| shape.refined_for_projection(matrix, tolerance))
                    .collect::<Result<_, _>>()?,
            ),
            Self::Affine { matrix: m, shape } => Self::Affine {
                matrix: *m,
                shape: Box::new(shape.refined_for_projection(
                    multiply(
                        matrix,
                        [
                            m[0], m[1], 0., 0., m[2], m[3], 0., 0., 0., 0., 1., 0., m[4], m[5], 0.,
                            1.,
                        ],
                    ),
                    tolerance,
                )?),
            },
            Self::Path {
                source: Some((source, origin)),
                even_odd,
                ..
            } => {
                if !tolerance.is_finite() || tolerance <= 0. {
                    return Err("projected path tolerance invalid".into());
                }
                let mut bounds = self.bounds();
                bounds.x -= 0.002;
                bounds.y -= 0.002;
                bounds.width += 0.004;
                bounds.height += 0.004;
                let [a, b, c, d, e, f, g, h, i] = [
                    matrix[0], matrix[4], matrix[12], matrix[1], matrix[5], matrix[13], matrix[3],
                    matrix[7], matrix[15],
                ];
                let corners = [
                    [bounds.x, bounds.y],
                    [bounds.right(), bounds.y],
                    [bounds.right(), bounds.bottom()],
                    [bounds.x, bounds.bottom()],
                ];
                let w = corners
                    .iter()
                    .map(|point| g * point[0] + h * point[1] + i)
                    .fold(f64::INFINITY, f64::min);
                if w <= 0. || !w.is_finite() {
                    return Err("path projection crosses horizon".into());
                }
                let derivative = corners
                    .iter()
                    .map(|point| {
                        let x = point[0];
                        let y = point[1];
                        [
                            ((a * h - b * g) * y + a * i - c * g) / w.powi(2),
                            ((b * g - a * h) * x + b * i - c * h) / w.powi(2),
                            ((d * h - e * g) * y + d * i - f * g) / w.powi(2),
                            ((e * g - d * h) * x + e * i - f * h) / w.powi(2),
                        ]
                        .iter()
                        .map(|v| v * v)
                        .sum::<f64>()
                        .sqrt()
                    })
                    .fold(0f64, f64::max);
                if !derivative.is_finite() {
                    return Err("projected path derivative overflow".into());
                }
                let mut contours =
                    path::parse_tolerance(source, (tolerance / derivative.max(1e-12)).min(0.001))?;
                for point in contours.iter_mut().flatten() {
                    point[0] += origin[0];
                    point[1] += origin[1];
                }
                Self::Path {
                    contours,
                    even_odd: *even_odd,
                    source: Some((source.clone(), *origin)),
                }
            }
            _ => self.clone(),
        })
    }
}
