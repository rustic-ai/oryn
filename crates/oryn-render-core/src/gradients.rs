//! CSS gradient line geometry and premultiplied color interpolation. Stops are
//! resolved in CSS pixels before projected rasterization, including hard stops.
use super::*;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub position: f64,
    pub color: [u8; 4],
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinearGradient {
    pub start: [f64; 2],
    pub end: [f64; 2],
    pub stops: Vec<GradientStop>,
    pub repeating: bool,
}
pub(super) fn split(value: &str, separator: char) -> Option<Vec<&str>> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escape = false;
    let mut start = 0;
    let mut parts = Vec::new();
    for (index, ch) in value.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            continue;
        }
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
            }
            _ => {}
        }
        if ch == separator && depth == 0 {
            parts.push(value[start..index].trim());
            start = index + ch.len_utf8();
        }
    }
    if depth != 0 || quote.is_some() {
        return None;
    }
    parts.push(value[start..].trim());
    Some(parts)
}
fn direction(value: &str, rect: CssRect) -> Option<(f64, f64)> {
    if let Some(value) = value.strip_prefix("to ") {
        let mut x = 0.;
        let mut y = 0.;
        for side in value.split_whitespace() {
            match side {
                "left" => x = -1.,
                "right" => x = 1.,
                "top" => y = -1.,
                "bottom" => y = 1.,
                _ => return None,
            }
        }
        if x == 0. && y == 0. {
            return None;
        }
        if x != 0. && y != 0. {
            x *= rect.height;
            y *= rect.width;
        }
        let norm = f64::hypot(x, y);
        return (norm > 0.).then_some((x / norm, y / norm));
    }
    let angle = if let Some(v) = value.strip_suffix("deg") {
        v.parse::<f64>().ok()?.to_radians()
    } else if let Some(v) = value.strip_suffix("turn") {
        v.parse::<f64>().ok()? * std::f64::consts::TAU
    } else if let Some(v) = value.strip_suffix("grad") {
        v.parse::<f64>().ok()? * std::f64::consts::PI / 200.
    } else if let Some(v) = value.strip_suffix("rad") {
        v.parse::<f64>().ok()?
    } else {
        return None;
    };
    angle.is_finite().then_some((angle.sin(), -angle.cos()))
}
fn color_prefix<'a>(value: &'a str, current: &str) -> Option<([u8; 4], &'a str)> {
    let end = if let Some(open) = value.find('(') {
        if value[..open].contains(char::is_whitespace) {
            value.find(char::is_whitespace).unwrap_or(value.len())
        } else {
            let mut depth = 0;
            let mut end = None;
            for (index, ch) in value.char_indices() {
                if ch == '(' {
                    depth += 1;
                } else if ch == ')' {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(index + 1);
                        break;
                    }
                }
            }
            end?
        }
    } else {
        value.find(char::is_whitespace).unwrap_or(value.len())
    };
    let color = &value[..end];
    Some((
        parse_color(if color == "currentcolor" {
            current
        } else {
            color
        })?,
        value[end..].trim(),
    ))
}
fn position(value: &str, length: f64, font: f64) -> Option<f64> {
    let result = if let Some(v) = value.strip_suffix('%') {
        v.parse::<f64>().ok()? / 100.
    } else if let Some(v) = value.strip_suffix("px") {
        v.parse::<f64>().ok()? / length
    } else if let Some(v) = value.strip_suffix("em") {
        v.parse::<f64>().ok()? * font / length
    } else if value == "0" {
        0.
    } else {
        return None;
    };
    result.is_finite().then_some(result)
}
pub(super) fn parse(
    value: &str,
    rect: CssRect,
    current: &str,
    font: f64,
) -> Option<Vec<LinearGradient>> {
    let mut output = Vec::new();
    for layer in split(value, ',')? {
        let (repeating, body) = if let Some(body) = layer.strip_prefix("linear-gradient(") {
            (false, body.strip_suffix(')')?)
        } else if let Some(body) = layer.strip_prefix("repeating-linear-gradient(") {
            (true, body.strip_suffix(')')?)
        } else {
            return None;
        };
        let mut items = split(body, ',')?;
        let direction = direction(items.first().copied()?, rect);
        let (dx, dy) = if let Some(direction) = direction {
            items.remove(0);
            direction
        } else {
            (0., 1.)
        };
        let length = (rect.width * dx.abs() + rect.height * dy.abs()).max(0.000001);
        let center = [rect.x + rect.width / 2., rect.y + rect.height / 2.];
        let mut stops = Vec::<([u8; 4], Option<f64>)>::new();
        for item in items {
            let (color, positions) = color_prefix(item, current)?;
            let positions = positions.split_whitespace().collect::<Vec<_>>();
            if positions.len() > 2 {
                return None;
            }
            if positions.is_empty() {
                stops.push((color, None));
            } else {
                for value in positions {
                    stops.push((color, Some(position(value, length, font)?)));
                }
            }
        }
        if stops.len() < 2 || stops.len() > 4096 {
            return None;
        }
        if stops[0].1.is_none() {
            stops[0].1 = Some(0.);
        }
        let last = stops.len() - 1;
        if stops[last].1.is_none() {
            stops[last].1 = Some(1.);
        }
        let mut previous = f64::NEG_INFINITY;
        for (_, stop) in &mut stops {
            if let Some(value) = stop {
                *value = value.max(previous);
                previous = *value;
            }
        }
        let mut first = 0;
        while first < last {
            let end = (first + 1..=last).find(|index| stops[*index].1.is_some())?;
            let a = stops[first].1?;
            let b = stops[end].1?;
            for index in first + 1..end {
                stops[index].1 = Some(a + (b - a) * (index - first) as f64 / (end - first) as f64);
            }
            first = end;
        }
        output.push(LinearGradient {
            start: [center[0] - dx * length / 2., center[1] - dy * length / 2.],
            end: [center[0] + dx * length / 2., center[1] + dy * length / 2.],
            stops: stops
                .into_iter()
                .map(|(color, position)| GradientStop {
                    color,
                    position: position.unwrap(),
                })
                .collect(),
            repeating,
        });
    }
    Some(output)
}
impl LinearGradient {
    pub(super) fn sample(&self, x: f64, y: f64) -> [u8; 4] {
        let dx = self.end[0] - self.start[0];
        let dy = self.end[1] - self.start[1];
        let length = dx * dx + dy * dy;
        let t = if length > 0. {
            ((x - self.start[0]) * dx + (y - self.start[1]) * dy) / length
        } else {
            0.
        };
        sample_stops(&self.stops, self.repeating, t)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_table_alignment_hard_stops_keep_transparent_regions() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        let gradient = parse(
            "linear-gradient(to right, transparent 25%, red 25%, red 75%, transparent 75%)",
            rect,
            "black",
            16.,
        )
        .unwrap()
        .remove(0);
        assert_eq!(gradient.sample(10., 50.), [0, 0, 0, 0]);
        assert_eq!(gradient.sample(50., 50.), [255, 0, 0, 255]);
        assert_eq!(gradient.sample(90., 50.), [0, 0, 0, 0]);
    }
    #[test]
    fn repeating_stops_and_premultiplied_transparency_use_native_gradient_positions() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        let gradient = parse(
            "repeating-linear-gradient(90deg, red 0px 10px, blue 10px 20px)",
            rect,
            "black",
            16.,
        )
        .unwrap()
        .remove(0);
        assert_eq!(gradient.sample(5., 50.), gradient.sample(25., 50.));
        assert_eq!(gradient.sample(15., 50.), [0, 0, 255, 255]);
        let fade = parse(
            "linear-gradient(to right, red, transparent)",
            rect,
            "black",
            16.,
        )
        .unwrap()
        .remove(0);
        assert_eq!(fade.sample(50., 50.), [255, 0, 0, 128]);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BackgroundGradient {
    Linear(LinearGradient),
    Radial {
        center: [f64; 2],
        radii: [f64; 2],
        stops: Vec<GradientStop>,
        repeating: bool,
    },
    Conic {
        center: [f64; 2],
        start: f64,
        stops: Vec<GradientStop>,
        repeating: bool,
    },
}
fn angular(value: &str) -> Option<f64> {
    let value = value.trim();
    let result = if let Some(v) = value.strip_suffix("deg") {
        v.parse::<f64>().ok()? / 360.
    } else if let Some(v) = value.strip_suffix("grad") {
        v.parse::<f64>().ok()? / 400.
    } else if let Some(v) = value.strip_suffix("turn") {
        v.parse::<f64>().ok()?
    } else if let Some(v) = value.strip_suffix("rad") {
        v.parse::<f64>().ok()? / std::f64::consts::TAU
    } else if let Some(v) = value.strip_suffix('%') {
        v.parse::<f64>().ok()? / 100.
    } else if value == "0" {
        0.
    } else {
        return None;
    };
    result.is_finite().then_some(result)
}
fn center(value: &str, rect: CssRect, font: f64) -> Option<[f64; 2]> {
    let mut words = value.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return Some([rect.x + rect.width / 2., rect.y + rect.height / 2.]);
    }
    if words.len() > 2 {
        return None;
    }
    if words.len() == 1 {
        if matches!(words[0], "top" | "bottom") {
            words.insert(0, "center");
        } else {
            words.push("center");
        }
    }
    if matches!(words[0], "top" | "bottom") || matches!(words[1], "left" | "right") {
        words.swap(0, 1);
    }
    let coordinate = |value: &str, length: f64, low: &str, high: &str| -> Option<f64> {
        if value == "center" {
            Some(length / 2.)
        } else if value == low {
            Some(0.)
        } else if value == high {
            Some(length)
        } else {
            Some(position(value, length.max(1e-12), font)? * length)
        }
    };
    Some([
        rect.x + coordinate(words[0], rect.width, "left", "right")?,
        rect.y + coordinate(words[1], rect.height, "top", "bottom")?,
    ])
}
fn resolved_stops(
    items: &[&str],
    current: &str,
    position: impl Fn(&str) -> Option<f64>,
) -> Option<Vec<GradientStop>> {
    let mut stops = Vec::<([u8; 4], Option<f64>)>::new();
    for item in items {
        let (color, tail) = color_prefix(item, current)?;
        let values = tail.split_whitespace().collect::<Vec<_>>();
        if values.len() > 2 {
            return None;
        }
        if values.is_empty() {
            stops.push((color, None));
        } else {
            for value in values {
                stops.push((color, Some(position(value)?)));
            }
        }
    }
    if stops.len() < 2 || stops.len() > 4096 {
        return None;
    }
    if stops[0].1.is_none() {
        stops[0].1 = Some(0.);
    }
    let last = stops.len() - 1;
    if stops[last].1.is_none() {
        stops[last].1 = Some(1.);
    }
    let mut previous = f64::NEG_INFINITY;
    for (_, stop) in &mut stops {
        if let Some(value) = stop {
            *value = value.max(previous);
            previous = *value;
        }
    }
    let mut start = 0;
    while start < last {
        let end = (start + 1..=last).find(|i| stops[*i].1.is_some())?;
        let a = stops[start].1?;
        let b = stops[end].1?;
        for i in start + 1..end {
            stops[i].1 = Some(a + (b - a) * (i - start) as f64 / (end - start) as f64);
        }
        start = end;
    }
    Some(
        stops
            .into_iter()
            .map(|(color, position)| GradientStop {
                color,
                position: position.unwrap(),
            })
            .collect(),
    )
}
pub(super) fn backgrounds(
    value: &str,
    rect: CssRect,
    current: &str,
    font: f64,
) -> Option<Vec<BackgroundGradient>> {
    let mut result = Vec::new();
    for layer in split(value, ',')? {
        if layer.starts_with("linear-gradient(") || layer.starts_with("repeating-linear-gradient(")
        {
            result.push(BackgroundGradient::Linear(
                parse(layer, rect, current, font)?.remove(0),
            ));
            continue;
        }
        let (repeating, body) = if let Some(value) = layer.strip_prefix("repeating-") {
            (true, value)
        } else {
            (false, layer)
        };
        if let Some(body) = body
            .strip_prefix("radial-gradient(")
            .and_then(|v| v.strip_suffix(')'))
        {
            let mut items = split(body, ',')?;
            let header = if color_prefix(items[0], current).is_none() {
                items.remove(0)
            } else {
                ""
            };
            let (shape, at) = header
                .split_once(" at ")
                .or_else(|| header.strip_prefix("at ").map(|at| ("", at)))
                .unwrap_or((header, ""));
            let center = center(at, rect, font)?;
            let mut circle = false;
            let mut explicit_shape = false;
            let mut extent = None;
            let mut sizes = Vec::new();
            for token in shape.split_whitespace() {
                match token {
                    "circle" => {
                        circle = true;
                        explicit_shape = true;
                    }
                    "ellipse" => {
                        circle = false;
                        explicit_shape = true;
                    }
                    "closest-side" | "farthest-side" | "closest-corner" | "farthest-corner" => {
                        if extent.replace(token).is_some() {
                            return None;
                        }
                    }
                    _ => sizes.push(token),
                }
            }
            let radii = if !sizes.is_empty() {
                if extent.is_some() || sizes.len() > 2 {
                    return None;
                }
                if !explicit_shape {
                    circle = sizes.len() == 1;
                }
                if circle && sizes.len() != 1 || !circle && sizes.len() != 2 {
                    return None;
                }
                if circle && sizes[0].ends_with('%') {
                    return None;
                }
                let x = position(sizes[0], rect.width.max(1e-12), font)? * rect.width;
                let y = if circle {
                    x
                } else {
                    position(sizes[1], rect.height.max(1e-12), font)? * rect.height
                };
                if x < 0. || y < 0. {
                    return None;
                }
                [x, y]
            } else {
                let extent = extent.unwrap_or("farthest-corner");
                let near = extent.starts_with("closest");
                let sides = [
                    (center[0] - rect.x).abs(),
                    (rect.right() - center[0]).abs(),
                    (center[1] - rect.y).abs(),
                    (rect.bottom() - center[1]).abs(),
                ];
                let choose = |a: f64, b: f64| if near { a.min(b) } else { a.max(b) };
                if circle {
                    let mut radius = choose(choose(sides[0], sides[1]), choose(sides[2], sides[3]));
                    if extent.ends_with("corner") {
                        radius = if near { f64::INFINITY } else { 0. };
                        for x in [sides[0], sides[1]] {
                            for y in [sides[2], sides[3]] {
                                radius = choose(radius, x.hypot(y));
                            }
                        }
                    }
                    [radius, radius]
                } else {
                    let mut radii = [choose(sides[0], sides[1]), choose(sides[2], sides[3])];
                    if extent.ends_with("corner") {
                        let mut factor = if near { f64::INFINITY } else { 0. };
                        for x in [sides[0], sides[1]] {
                            for y in [sides[2], sides[3]] {
                                factor = choose(
                                    factor,
                                    (x / radii[0].max(1e-12)).hypot(y / radii[1].max(1e-12)),
                                );
                            }
                        }
                        radii = [radii[0] * factor, radii[1] * factor];
                    }
                    radii
                }
            };
            let stops = resolved_stops(&items, current, |value| {
                position(value, radii[0].max(1e-12), font)
            })?;
            result.push(BackgroundGradient::Radial {
                center,
                radii,
                stops,
                repeating,
            });
        } else if let Some(body) = body
            .strip_prefix("conic-gradient(")
            .and_then(|v| v.strip_suffix(')'))
        {
            let mut items = split(body, ',')?;
            let header = if color_prefix(items[0], current).is_none() {
                items.remove(0)
            } else {
                ""
            };
            let mut angle = 0.;
            let mut at = "";
            if let Some(value) = header.strip_prefix("from ") {
                let (a, p) = value.split_once(" at ").unwrap_or((value, ""));
                angle = angular(a)?;
                at = p;
            } else if let Some(value) = header.strip_prefix("at ") {
                at = value;
            } else if !header.is_empty() {
                return None;
            }
            result.push(BackgroundGradient::Conic {
                center: center(at, rect, font)?,
                start: angle,
                stops: resolved_stops(&items, current, angular)?,
                repeating,
            });
        } else {
            return None;
        }
    }
    Some(result)
}
impl BackgroundGradient {
    pub(super) fn sample(&self, x: f64, y: f64) -> [u8; 4] {
        match self {
            Self::Linear(value) => value.sample(x, y),
            Self::Radial {
                center,
                radii,
                stops,
                repeating,
            } => sample_stops(
                stops,
                *repeating,
                if radii.iter().any(|r| *r <= 0.) {
                    f64::INFINITY
                } else {
                    ((x - center[0]) / radii[0]).hypot((y - center[1]) / radii[1])
                },
            ),
            Self::Conic {
                center,
                start,
                stops,
                repeating,
            } => sample_stops(
                stops,
                *repeating,
                ((x - center[0]).atan2(-(y - center[1])) / std::f64::consts::TAU - start)
                    .rem_euclid(1.),
            ),
        }
    }
}
fn sample_stops(stops: &[GradientStop], repeating: bool, mut t: f64) -> [u8; 4] {
    let Some(first) = stops.first() else {
        return [0; 4];
    };
    let last = stops.last().unwrap();
    if !t.is_finite() {
        return last.color;
    }
    if repeating {
        let period = last.position - first.position;
        if period <= 0. {
            return last.color;
        }
        t = first.position + (t - first.position).rem_euclid(period);
    }
    if t < first.position {
        return first.color;
    }
    if t >= last.position {
        return last.color;
    }
    let right = stops.partition_point(|stop| stop.position <= t);
    let a = &stops[right - 1];
    let b = &stops[right];
    let f = ((t - a.position) / (b.position - a.position)).clamp(0., 1.);
    let alpha = f64::from(a.color[3]) * (1. - f) + f64::from(b.color[3]) * f;
    if alpha <= 0. {
        return [0; 4];
    }
    let mut color = [0; 4];
    color[3] = alpha.round() as u8;
    for channel in 0..3 {
        color[channel] = ((f64::from(a.color[channel]) * f64::from(a.color[3]) * (1. - f)
            + f64::from(b.color[channel]) * f64::from(b.color[3]) * f)
            / alpha)
            .round()
            .clamp(0., 255.) as u8;
    }
    color
}

#[cfg(test)]
mod radial_conic_tests {
    use super::*;
    #[test]
    fn radial_explicit_ellipse_and_extent_keywords_use_native_geometry() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 80.,
        };
        let gradient = backgrounds(
            "radial-gradient(ellipse 40px 20px at 50% 50%, red 50%, blue 50%)",
            rect,
            "black",
            16.,
        )
        .unwrap()
        .remove(0);
        assert_eq!(gradient.sample(50., 40.), [255, 0, 0, 255]);
        assert_eq!(gradient.sample(50., 55.), [0, 0, 255, 255]);
        assert_eq!(gradient.sample(75., 40.), [0, 0, 255, 255]);
        let gradient = backgrounds(
            "radial-gradient(circle closest-side at 20px 30px, red, blue)",
            rect,
            "black",
            16.,
        )
        .unwrap()
        .remove(0);
        assert_eq!(gradient.sample(40., 30.), [0, 0, 255, 255]);
    }
    #[test]
    fn conic_zero_is_up_and_increases_clockwise_with_repeating_angles() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        let gradient = backgrounds(
            "conic-gradient(from 0deg at center, red 0deg 90deg, blue 90deg 360deg)",
            rect,
            "black",
            16.,
        )
        .unwrap()
        .remove(0);
        assert_eq!(gradient.sample(60., 40.), [255, 0, 0, 255]);
        assert_eq!(gradient.sample(60., 60.), [0, 0, 255, 255]);
        let repeated = backgrounds(
            "repeating-conic-gradient(red 0deg 45deg, blue 45deg 90deg)",
            rect,
            "black",
            16.,
        )
        .unwrap()
        .remove(0);
        assert_eq!(repeated.sample(55., 40.), repeated.sample(60., 55.));
    }
    #[test]
    fn mixed_layers_and_degenerate_radial_sizes_are_explicit() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        let layers=backgrounds("radial-gradient(circle 0px, red, blue), conic-gradient(red, blue), linear-gradient(red, transparent)",rect,"black",16.).unwrap();
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0].sample(50., 50.), [0, 0, 255, 255]);
        assert!(
            backgrounds("radial-gradient(circle 30%, red, blue)", rect, "black", 16.).is_none()
        );
    }
}
