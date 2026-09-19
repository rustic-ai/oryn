//! Used CSS shape() command resolution before the shared SVG contour parser.
use super::*;

type ParsedCommands = (Vec<Vec<[f64; 2]>>, bool, String);
fn pair(tokens: &[&str], x: LengthContext, y: LengthContext) -> Option<[f64; 2]> {
    if tokens.len() != 2 {
        return None;
    }
    Some([
        css_math::resolve_length(tokens[0], x)?,
        css_math::resolve_length(tokens[1], y)?,
    ])
}
fn control(
    source: &str,
    start: [f64; 2],
    end: [f64; 2],
    relative: bool,
    x: LengthContext,
    y: LengthContext,
) -> Option<[f64; 2]> {
    let tokens = words(source)?;
    let from = tokens.iter().position(|token| *token == "from");
    let value = pair(&tokens[..from.unwrap_or(tokens.len())], x, y)?;
    let origin = if let Some(index) = from {
        if tokens.len() != index + 2 {
            return None;
        }
        match tokens[index + 1] {
            "origin" => [0.; 2],
            "start" => start,
            "end" => end,
            _ => return None,
        }
    } else if relative {
        start
    } else {
        [0.; 2]
    };
    Some([value[0] + origin[0], value[1] + origin[1]])
}
pub(super) fn parse(body: &str, x: LengthContext, y: LengthContext) -> Option<ParsedCommands> {
    let commands = separated(body, ',')?;
    let header = words(commands.first()?)?;
    let (mut index, mut even_odd) = (0, false);
    if header.first() == Some(&"evenodd") {
        index += 1;
        even_odd = true
    } else if header.first() == Some(&"nonzero") {
        index += 1
    }
    if header.get(index) != Some(&"from") {
        return None;
    }
    let position = header[index + 1..].join(" ");
    let (px, py) = css_math::resolve_position(&position, x, y)?;
    let mut current = [px, py];
    let mut initial = current;
    let mut svg = format!("M{px} {py}");
    for command in commands.iter().skip(1) {
        let tokens = words(command)?;
        let name = *tokens.first()?;
        if name == "close" {
            if tokens.len() != 1 {
                return None;
            }
            svg.push_str(" Z");
            current = initial;
            continue;
        }
        let relative = match *tokens.get(1)? {
            "by" => true,
            "to" => false,
            _ => return None,
        };
        let endpoint = |pair: [f64; 2]| {
            if relative {
                [current[0] + pair[0], current[1] + pair[1]]
            } else {
                pair
            }
        };
        match name {
            "move" | "line" => {
                let point = endpoint(pair(&tokens[2..], x, y)?);
                svg.push_str(&format!(
                    " {}{} {}",
                    if name == "move" { 'M' } else { 'L' },
                    point[0],
                    point[1]
                ));
                current = point;
                if name == "move" {
                    initial = current;
                }
            }
            "hline" | "vline" => {
                if tokens.len() != 3 {
                    return None;
                }
                let axis = usize::from(name == "vline");
                let mut point = current;
                let value = css_math::resolve_length(tokens[2], if axis == 0 { x } else { y })?;
                point[axis] = if relative { point[axis] + value } else { value };
                svg.push_str(&format!(" L{} {}", point[0], point[1]));
                current = point;
            }
            "curve" | "smooth" => {
                let with = tokens.iter().position(|token| *token == "with");
                let point = endpoint(pair(&tokens[2..with.unwrap_or(tokens.len())], x, y)?);
                let controls = with.map(|index| tokens[index + 1..].join(" "));
                let controls = controls
                    .as_deref()
                    .map(|source| separated(source, '/'))
                    .transpose_option()?;
                match (name, controls) {
                    ("curve", Some(controls)) if controls.len() == 1 => {
                        let c = control(controls[0], current, point, relative, x, y)?;
                        svg.push_str(&format!(" Q{} {} {} {}", c[0], c[1], point[0], point[1]));
                    }
                    ("curve", Some(controls)) if controls.len() == 2 => {
                        let a = control(controls[0], current, point, relative, x, y)?;
                        let b = control(controls[1], current, point, relative, x, y)?;
                        svg.push_str(&format!(
                            " C{} {} {} {} {} {}",
                            a[0], a[1], b[0], b[1], point[0], point[1]
                        ));
                    }
                    ("smooth", None) => svg.push_str(&format!(" T{} {}", point[0], point[1])),
                    ("smooth", Some(controls)) if controls.len() == 1 => {
                        let c = control(controls[0], current, point, relative, x, y)?;
                        svg.push_str(&format!(" S{} {} {} {}", c[0], c[1], point[0], point[1]));
                    }
                    _ => return None,
                }
                current = point;
            }
            "arc" => {
                let of = tokens.iter().position(|token| *token == "of")?;
                let point = endpoint(pair(&tokens[2..of], x, y)?);
                let mut index = of + 1;
                let rx = css_math::resolve_length(tokens.get(index)?, x)?;
                index += 1;
                let ry = if tokens.get(index).is_some_and(|token| {
                    !matches!(*token, "cw" | "ccw" | "large" | "small" | "rotate")
                }) {
                    let value = css_math::resolve_length(tokens[index], y)?;
                    index += 1;
                    value
                } else {
                    css_math::resolve_length(tokens[of + 1], y)?
                };
                if rx < 0. || ry < 0. {
                    return None;
                }
                let (mut sweep, mut large, mut rotation) = (false, false, 0.);
                let (mut direction_seen, mut size_seen, mut rotation_seen) = (false, false, false);
                while index < tokens.len() {
                    match tokens[index] {
                        "cw" | "ccw" => {
                            if direction_seen {
                                return None;
                            }
                            direction_seen = true;
                            sweep = tokens[index] == "cw";
                            index += 1
                        }
                        "large" | "small" => {
                            if size_seen {
                                return None;
                            }
                            size_seen = true;
                            large = tokens[index] == "large";
                            index += 1
                        }
                        "rotate" => {
                            if rotation_seen {
                                return None;
                            }
                            rotation_seen = true;
                            rotation = css_math::resolve_angle(tokens.get(index + 1)?)?;
                            index += 2
                        }
                        _ => return None,
                    }
                }
                svg.push_str(&format!(
                    " A{rx} {ry} {rotation} {} {} {} {}",
                    u8::from(large),
                    u8::from(sweep),
                    point[0],
                    point[1]
                ));
                current = point;
            }
            _ => return None,
        }
        if svg.len() > 1024 * 1024 {
            return None;
        }
    }
    Some((path::parse(&svg).ok()?, even_odd, svg))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_control_origins_and_percentage_arcs_resolve_in_native_space() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 200.,
        };
        let (x, y) = contexts(rect, 16., (100., 200.));
        let(contours,_,_)=parse("from 10px 20px, line by 20% 10%, curve by 10px 20px with 5px 10px from start / 0px 0px from end, close",x,y).unwrap();
        assert_eq!(contours[0][0], [10., 20.]);
        assert!(contours[0].contains(&[40., 60.]));
        assert!(
            parse(
                "from center, arc to 10px 20px of 10% cw large rotate 30deg",
                x,
                y
            )
            .is_some()
        );
    }
}
