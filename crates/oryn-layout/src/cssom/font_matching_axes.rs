// Shared typed form of the existing web_font_faces::Face::rank algorithm.
#[derive(Clone, Copy, PartialEq)]
pub enum NativeFontStyle {
    Normal,
    Italic,
    Oblique(f32, f32),
}
#[derive(Clone, Copy)]
pub struct NativeFontAxes {
    pub weight: (f32, f32),
    pub width: (f32, f32),
    pub style: NativeFontStyle,
}
pub fn native_font_axes_rank(face: NativeFontAxes, query: NativeFontAxes) -> [f32; 7] {
    let width = query.width.0;
    let weight = query.weight.0;
    let width_value = width.clamp(face.width.0, face.width.1);
    let width_rank = if width_value == width {
        [0., 0.]
    } else if (width <= 100. && width_value < width) || (width > 100. && width_value > width) {
        [1., (width_value - width).abs()]
    } else {
        [2., (width_value - width).abs()]
    };
    let kind = |s: NativeFontStyle| match s {
        NativeFontStyle::Normal => 0,
        NativeFontStyle::Italic => 1,
        NativeFontStyle::Oblique(..) => 2,
    };
    let requested = kind(query.style);
    let actual = kind(face.style);
    let style_rank = if actual == requested {
        0.
    } else if (requested == 1 && actual == 2)
        || (requested == 2 && actual == 1)
        || (requested == 0 && actual == 2)
    {
        1.
    } else {
        2.
    };
    let angle_rank = match face.style {
        NativeFontStyle::Oblique(min, max) => native_font_angle_distance(
            match query.style {
                NativeFontStyle::Oblique(angle, _) => angle,
                NativeFontStyle::Italic => 14.,
                NativeFontStyle::Normal => 0.,
            },
            min,
            max,
        ),
        _ => [0., 0.],
    };
    let value = weight.clamp(face.weight.0, face.weight.1);
    let weight_rank = if value == weight {
        [0., 0.]
    } else if (400. ..=500.).contains(&weight) {
        if value >= weight && value <= 500. {
            [1., value - weight]
        } else if value < weight {
            [2., weight - value]
        } else {
            [3., value - 500.]
        }
    } else if (weight < 400. && value < weight) || (weight > 500. && value > weight) {
        [1., (value - weight).abs()]
    } else {
        [2., (value - weight).abs()]
    };
    [
        width_rank[0],
        width_rank[1],
        style_rank,
        angle_rank[0],
        angle_rank[1],
        weight_rank[0],
        weight_rank[1],
    ]
}
pub fn native_font_angle_distance(desired: f32, min: f32, max: f32) -> [f32; 2] {
    if desired < 0. {
        return native_font_angle_distance(-desired, -max, -min);
    }
    let actual = desired.clamp(min, max);
    if actual == desired {
        return [0., 0.];
    }
    if actual < 0. {
        return [3., -actual];
    }
    if (desired >= 11. && actual > desired) || (desired < 11. && actual < desired) {
        [1., (actual - desired).abs()]
    } else {
        [2., (actual - desired).abs()]
    }
}

#[cfg(test)]
mod native_axes_parity {
    use super::*;
    #[test]
    fn italic_oblique_fallback_keeps_renderers_fourteen_degree_target() {
        let axes = |style| NativeFontAxes {
            weight: (400., 400.),
            width: (100., 100.),
            style,
        };
        let query = axes(NativeFontStyle::Italic);
        assert!(
            native_font_axes_rank(axes(NativeFontStyle::Oblique(14., 14.)), query)
                < native_font_axes_rank(axes(NativeFontStyle::Oblique(0., 0.)), query)
        );
    }
}
