//! Ordered CSS transforms for layout rectangle projection.
use super::{CssRect, ViewportState, length};
use cssparser::{Parser, ParserInput, Token};

#[derive(Clone, Copy)]
struct Matrix([f64; 16]);
impl Matrix {
    const IDENTITY: Self = Self([
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ]);
    fn then(self, rhs: Self) -> Self {
        let mut result = [0.; 16];
        for column in 0..4 {
            for row in 0..4 {
                result[column * 4 + row] = (0..4)
                    .map(|k| self.0[k * 4 + row] * rhs.0[column * 4 + k])
                    .sum();
            }
        }
        Self(result)
    }
    fn rotation(x: f64, y: f64, z: f64, angle: f64) -> Self {
        // Normalize after scaling to avoid overflow for finite large axes.
        let scale = x.abs().max(y.abs()).max(z.abs());
        if scale == 0. {
            return Self::IDENTITY;
        }
        let (x, y, z) = (x / scale, y / scale, z / scale);
        let norm = (x * x + y * y + z * z).sqrt();
        let (x, y, z) = (x / norm, y / norm, z / norm);
        let (s, c) = angle.sin_cos();
        let t = 1. - c;
        Self([
            t * x * x + c,
            t * x * y + s * z,
            t * x * z - s * y,
            0.,
            t * x * y - s * z,
            t * y * y + c,
            t * y * z + s * x,
            0.,
            t * x * z + s * y,
            t * y * z - s * x,
            t * z * z + c,
            0.,
            0.,
            0.,
            0.,
            1.,
        ])
    }
}

fn angle(value: &str) -> Option<f64> {
    oryn_common::css_math::resolve_angle(value).map(f64::to_radians)
}

fn operation(
    name: &str,
    values: &[String],
    reference: CssRect,
    font: f64,
    viewport: ViewportState,
) -> Option<Matrix> {
    let number = |i: usize| oryn_common::css_math::resolve_number(values.get(i)?);
    let scale = |i: usize| oryn_common::css_math::resolve_scale(values.get(i)?);
    let distance = |i: usize, basis| values.get(i).map(|s| length(s, basis, font, viewport));
    let mut m = Matrix::IDENTITY;
    match (name, values.len()) {
        ("matrix", 6) => {
            for (i, index) in [0, 1, 4, 5, 12, 13].into_iter().enumerate() {
                m.0[index] = number(i)?;
            }
        }
        ("matrix3d", 16) => {
            for i in 0..16 {
                m.0[i] = number(i)?;
            }
        }
        ("translate", 1 | 2) => {
            m.0[12] = distance(0, reference.width)?;
            m.0[13] = if values.len() == 2 {
                distance(1, reference.height)?
            } else {
                0.
            };
        }
        ("translatex", 1) => m.0[12] = distance(0, reference.width)?,
        ("translatey", 1) => m.0[13] = distance(0, reference.height)?,
        ("translatez", 1) => m.0[14] = distance(0, 0.)?,
        ("translate3d", 3) => {
            m.0[12] = distance(0, reference.width)?;
            m.0[13] = distance(1, reference.height)?;
            m.0[14] = distance(2, 0.)?;
        }
        ("scale", 1 | 2) => {
            m.0[0] = scale(0)?;
            m.0[5] = if values.len() == 2 { scale(1)? } else { m.0[0] };
        }
        ("scalex", 1) => m.0[0] = scale(0)?,
        ("scaley", 1) => m.0[5] = scale(0)?,
        ("scalez", 1) => m.0[10] = scale(0)?,
        ("scale3d", 3) => {
            m.0[0] = scale(0)?;
            m.0[5] = scale(1)?;
            m.0[10] = scale(2)?;
        }
        ("rotate" | "rotatez", 1) => m = Matrix::rotation(0., 0., 1., angle(&values[0])?),
        ("rotatex", 1) => m = Matrix::rotation(1., 0., 0., angle(&values[0])?),
        ("rotatey", 1) => m = Matrix::rotation(0., 1., 0., angle(&values[0])?),
        ("rotate3d", 4) => {
            m = Matrix::rotation(number(0)?, number(1)?, number(2)?, angle(&values[3])?)
        }
        ("perspective", 1) => {
            if values[0] != "none" {
                let depth = distance(0, 0.)?;
                if depth < 0. {
                    return None;
                }
                m.0[11] = -1. / depth.max(1.);
            }
        }
        ("skew", 1 | 2) => {
            m.0[4] = angle(&values[0])?.tan();
            m.0[1] = if values.len() == 2 {
                angle(&values[1])?.tan()
            } else {
                0.
            };
        }
        ("skewx", 1) => m.0[4] = angle(&values[0])?.tan(),
        ("skewy", 1) => m.0[1] = angle(&values[0])?.tan(),
        _ => return None,
    }
    m.0.iter().all(|x| x.is_finite()).then_some(m)
}

fn parse(
    transform: &str,
    reference: CssRect,
    font: f64,
    viewport: ViewportState,
) -> Option<Matrix> {
    if transform.trim().eq_ignore_ascii_case("none") {
        return Some(Matrix::IDENTITY);
    }
    let mut input = ParserInput::new(transform);
    let mut parser = Parser::new(&mut input);
    let mut matrix = Matrix::IDENTITY;
    while !parser.is_exhausted() {
        let Token::Function(name) = parser.next().ok()?.clone() else {
            return None;
        };
        let values = parser
            .parse_nested_block(|nested| {
                let mut values = Vec::new();
                while !nested.is_exhausted() {
                    let start = nested.position();
                    let token = nested.next()?.clone();
                    if matches!(token, Token::Comma) {
                        continue;
                    }
                    if matches!(token, Token::Function(_)) {
                        nested.parse_nested_block(|inner| {
                            while !inner.is_exhausted() {
                                inner.next()?;
                            }
                            Ok::<(), cssparser::ParseError<'_, ()>>(())
                        })?;
                    }
                    values.push(nested.slice_from(start).trim().to_owned());
                }
                Ok::<_, cssparser::ParseError<'_, ()>>(values)
            })
            .ok()?;
        matrix = matrix.then(operation(
            &name.to_ascii_lowercase(),
            &values,
            reference,
            font,
            viewport,
        )?);
    }
    Some(matrix)
}

/// CSSOM resolves only the transform list, without origin or independent
/// translate/rotate/scale properties and without ancestor transformations.
pub(super) fn resolved_value(
    reference: CssRect,
    transform: &str,
    font: f64,
    viewport: ViewportState,
) -> Option<String> {
    if transform.trim().eq_ignore_ascii_case("none") {
        return Some("none".into());
    }
    let matrix = parse(transform, reference, font, viewport)?.0;
    let planar = [2, 3, 6, 7, 8, 9, 11, 14].iter().all(|i| matrix[*i] == 0.)
        && matrix[10] == 1.
        && matrix[15] == 1.;
    let values = if planar {
        [0, 1, 4, 5, 12, 13]
            .iter()
            .map(|i| matrix[*i])
            .collect::<Vec<_>>()
    } else {
        matrix.to_vec()
    };
    let values = values
        .iter()
        .map(|v| if *v == 0. { "0".into() } else { v.to_string() })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "{}({values})",
        if planar { "matrix" } else { "matrix3d" }
    ))
}

fn components(text: &str) -> Option<Vec<String>> {
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    let mut values = Vec::new();
    while !parser.is_exhausted() {
        let start = parser.position();
        let token = parser.next().ok()?.clone();
        if matches!(token, Token::Function(_)) {
            parser
                .parse_nested_block(|p| {
                    while !p.is_exhausted() {
                        p.next()?;
                    }
                    Ok::<(), cssparser::ParseError<'_, ()>>(())
                })
                .ok()?;
        }
        values.push(parser.slice_from(start).trim().to_owned());
    }
    Some(values)
}

/// Independent transform properties precede the transform list: T * R * S * M.
/// Preserve the functions until projection so no intermediate bounding box loses
/// orientation or depth information.
pub(super) fn compose_source(
    translate: &str,
    rotate: &str,
    scale: &str,
    transform: &str,
) -> Option<String> {
    let mut result = Vec::new();
    for (property, text) in [
        ("translate", translate),
        ("rotate", rotate),
        ("scale", scale),
    ] {
        if text.trim().eq_ignore_ascii_case("none") {
            continue;
        }
        let mut values = components(text)?;
        let function = match property {
            "translate" | "scale" => match values.len() {
                1 | 2 => property.to_owned(),
                3 => format!("{property}3d"),
                _ => return None,
            },
            "rotate" => {
                if values.len() > 1
                    && oryn_common::css_math::resolve_angle(&values[0]).is_some()
                    && oryn_common::css_math::resolve_number(&values[0]).is_none()
                {
                    let angle = values.remove(0);
                    values.push(angle);
                }
                match values.len() {
                    1 => "rotate".into(),
                    2 if matches!(values[0].as_str(), "x" | "y" | "z") => {
                        let axis = values.remove(0);
                        format!("rotate{axis}")
                    }
                    4 => "rotate3d".into(),
                    _ => return None,
                }
            }
            _ => unreachable!(),
        };
        result.push(format!("{function}({})", values.join(",")));
    }
    if !transform.trim().eq_ignore_ascii_case("none") {
        result.push(transform.into());
    }
    Some(if result.is_empty() {
        "none".into()
    } else {
        result.join(" ")
    })
}

/// Prepared once per element; every fragment shares its matrix and origin.
#[derive(Clone, Copy)]
pub(super) struct Projection {
    matrix: Matrix,
}

fn origin_offsets(
    reference: CssRect,
    origin: &str,
    font: f64,
    viewport: ViewportState,
) -> Option<[f64; 3]> {
    let mut components = components(origin)?;
    if components.is_empty() {
        components.extend(["50%".into(), "50%".into()]);
    }
    if components.len() == 1 {
        if matches!(components[0].as_str(), "top" | "bottom") {
            components.insert(0, "50%".into());
        } else {
            components.push("50%".into());
        }
    }
    if matches!(components[0].as_str(), "top" | "bottom")
        || matches!(components[1].as_str(), "left" | "right")
    {
        components.swap(0, 1);
    }
    let resolve = |value: &str, basis| match value {
        "left" | "top" => 0.,
        "center" => basis * 0.5,
        "right" | "bottom" => basis,
        _ => length(value, basis, font, viewport),
    };
    let ox = resolve(&components[0], reference.width);
    let oy = resolve(&components[1], reference.height);
    let oz = components
        .get(2)
        .map(|value| length(value, 0., font, viewport))
        .unwrap_or(0.);
    Some([ox, oy, oz])
}

pub(super) fn resolved_origin(
    reference: CssRect,
    origin: &str,
    font: f64,
    viewport: ViewportState,
) -> Option<String> {
    let values = origin_offsets(reference, origin, font, viewport)?;
    let count = if values[2] == 0. { 2 } else { 3 };
    Some(
        values[..count]
            .iter()
            .map(|value| format!("{}px", if *value == 0. { 0. } else { *value }))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

pub(super) fn prepare(
    reference: CssRect,
    transform: &str,
    origin: &str,
    font: f64,
    viewport: ViewportState,
) -> Option<Projection> {
    let matrix = parse(transform, reference, font, viewport)?;
    let [ox, oy, oz] = origin_offsets(reference, origin, font, viewport)?;
    let (ox, oy) = (reference.x + ox, reference.y + oy);
    let mut before = Matrix::IDENTITY;
    before.0[12] = ox;
    before.0[13] = oy;
    before.0[14] = oz;
    let mut after = Matrix::IDENTITY;
    after.0[12] = -ox;
    after.0[13] = -oy;
    after.0[14] = -oz;
    Some(Projection {
        matrix: before.then(matrix).then(after),
    })
}

impl Projection {
    pub(super) fn matrix(&self) -> [f64; 16] {
        self.matrix.0
    }
    pub(super) fn from_matrix(matrix: [f64; 16]) -> Self {
        Self {
            matrix: Matrix(matrix),
        }
    }
    /// Preserve homogeneous depth until the ancestor perspective/flattening boundary.
    pub(super) fn homogeneous(&self, point: [f64; 4]) -> [f64; 4] {
        let matrix = self.matrix.0;
        std::array::from_fn(|row| {
            (0..4)
                .map(|column| matrix[column * 4 + row] * point[column])
                .sum()
        })
    }

    pub(super) const IDENTITY: Self = Self {
        matrix: Matrix::IDENTITY,
    };
    pub(super) fn then(self, local: Self) -> Self {
        Self {
            matrix: self.matrix.then(local.matrix),
        }
    }
    pub(super) fn translation(x: f64, y: f64) -> Self {
        let mut matrix = Matrix::IDENTITY;
        matrix.0[12] = x;
        matrix.0[13] = y;
        Self { matrix }
    }
    pub(super) fn flatten_children(self) -> Self {
        let mut flatten = Matrix::IDENTITY;
        flatten.0[10] = 0.;
        Self {
            matrix: self.matrix.then(flatten),
        }
    }
    pub(super) fn project(&self, rect: CssRect) -> CssRect {
        oryn_common::layout_domain::projected_bounds(self.matrix.0, rect)
            .map(|bounds| bounds.rect)
            .unwrap_or(CssRect {
                x: 0.,
                y: 0.,
                width: 0.,
                height: 0.,
            })
    }
}

#[cfg(test)]
fn project(
    rect: CssRect,
    reference: CssRect,
    transform: &str,
    origin: &str,
    font: f64,
    viewport: ViewportState,
) -> CssRect {
    prepare(reference, transform, origin, font, viewport).map_or(rect, |p| p.project(rect))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cssom_matrix_resolves_percentages_without_reference_position() {
        let reference = CssRect {
            x: 50.,
            y: 70.,
            width: 100.,
            height: 40.,
        };
        assert_eq!(
            resolved_origin(
                reference,
                "calc(50% + 10px) bottom 3px",
                16.,
                ViewportState::default()
            )
            .as_deref(),
            Some("60px 40px 3px")
        );
        assert_eq!(
            resolved_origin(reference, "top right", 16., ViewportState::default()).as_deref(),
            Some("100px 0px")
        );
        assert_eq!(
            resolved_origin(reference, "center", 16., ViewportState::default()).as_deref(),
            Some("50px 20px")
        );
        assert_eq!(
            resolved_value(
                reference,
                "translate(50%,25%) scale(2)",
                16.,
                ViewportState::default()
            )
            .as_deref(),
            Some("matrix(2, 0, 0, 2, 50, 10)")
        );
        assert_eq!(
            resolved_value(reference, "translateZ(10px)", 16., ViewportState::default()).as_deref(),
            Some("matrix3d(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 10, 1)")
        );
        assert_eq!(
            resolved_value(reference, "none", 16., ViewportState::default()).as_deref(),
            Some("none")
        );
    }

    #[test]
    fn independent_properties_compose_before_the_transform_list() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 20.,
        };
        let source = compose_source("10px 20px", "z 90deg", "2 3", "translateX(5px)").unwrap();
        let actual = project(rect, rect, &source, "0 0", 16., ViewportState::default());
        assert!((actual.x + 50.).abs() < 1e-9 && (actual.y - 30.).abs() < 1e-9);
        assert!((actual.width - 60.).abs() < 1e-9 && (actual.height - 20.).abs() < 1e-9);
        assert_eq!(
            compose_source("none", "none", "none", "none").as_deref(),
            Some("none")
        );
        assert_eq!(
            compose_source("none", "90deg y", "none", "none").as_deref(),
            Some("rotatey(90deg)")
        );
        assert_eq!(
            compose_source("calc(50% + 1px) 0px 2px", "0 1 0 90deg", "200%", "none").as_deref(),
            Some("translate3d(calc(50% + 1px),0px,2px) rotate3d(0,1,0,90deg) scale(200%)")
        );
    }

    #[test]
    fn spatial_projection_preserves_order_axes_and_depth_origin() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 20.,
            height: 10.,
        };
        let p = |transform, origin| {
            project(rect, rect, transform, origin, 16., ViewportState::default())
        };
        let forward = p("perspective(100px) translateZ(50px)", "0 0");
        assert_eq!((forward.width, forward.height), (40., 20.));
        let reversed = p("translateZ(50px) perspective(100px)", "0 0");
        assert_eq!((reversed.width, reversed.height), (20., 10.));
        let rotated = p("rotate3d(0,0,10,90deg)", "0 0");
        assert!((rotated.x + 10.).abs() < 1e-9 && (rotated.width - 10.).abs() < 1e-9);
        assert!((rotated.height - 20.).abs() < 1e-9);
        let depth = p("rotateY(90deg)", "0 0 10px");
        assert!((depth.x + 10.).abs() < 1e-9 && depth.width.abs() < 1e-9);
        let identity = p("rotate3d(0,0,0,90deg)", "center");
        assert_eq!(identity, rect);
        let explicit = p("matrix3d(2,0,0,0,0,3,0,0,0,0,1,0,7,9,0,1)", "0 0");
        assert_eq!(
            (explicit.x, explicit.y, explicit.width, explicit.height),
            (7., 9., 40., 30.)
        );
        let composed = p("translate3d(7px,9px,0px) scale3d(2,3,1)", "0 0");
        assert_eq!(composed, explicit);
        let percentages = p("scale3d(200%, calc(100% + 2), 100%)", "0 0");
        assert_eq!((percentages.width, percentages.height), (40., 30.));
        let calculated = p(
            "rotate(calc(0.5turn - 90deg)) scale(calc(4px / 2px))",
            "0 0",
        );
        assert!((calculated.x + 20.).abs() < 1e-9);
        assert!((calculated.width - 20.).abs() < 1e-9 && (calculated.height - 40.).abs() < 1e-9);
    }

    #[test]
    fn transform_order_origin_and_fragment_reference() {
        let reference = CssRect {
            x: 10.,
            y: 20.,
            width: 100.,
            height: 40.,
        };
        let p = |rect, transform, origin| {
            project(
                rect,
                reference,
                transform,
                origin,
                16.,
                ViewportState::default(),
            )
        };
        assert_eq!(p(reference, "translate(10px) scale(2)", "0 0").x, 20.);
        assert_eq!(p(reference, "scale(2) translate(10px)", "0 0").x, 30.);
        assert_eq!(p(reference, "scale(-1, 2)", "center").x, 10.);
        assert_eq!(p(reference, "scale(-1, 2)", "center").y, 0.);
        let fragment = CssRect {
            x: 10.,
            y: 20.,
            width: 20.,
            height: 10.,
        };
        let actual = p(fragment, "scale(2)", "center");
        assert_eq!(
            (actual.x, actual.y, actual.width, actual.height),
            (-40., 0., 40., 20.)
        );
        let rotated = p(reference, "rotate(90deg)", "0 0");
        assert!((rotated.width - 40.).abs() < 1e-9 && (rotated.height - 100.).abs() < 1e-9);
        assert!((rotated.x + 30.).abs() < 1e-9);
        let translated = p(reference, "translate(50%,25%) translateX(10px)", "center");
        assert_eq!((translated.x, translated.y), (70., 30.));
        let calculated = p(reference, "scale(2)", "calc(50% + 10px) calc(50% - 5px)");
        assert_eq!((calculated.x, calculated.y), (-50., 5.));
        let moved = p(
            reference,
            "translate(calc(50% + 10px), min(25%, 20px))",
            "center",
        );
        assert_eq!((moved.x, moved.y), (70., 30.));
    }
}
