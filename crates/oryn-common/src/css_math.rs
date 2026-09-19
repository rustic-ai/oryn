//! Context-independent CSS math used by geometry transform strings.
use cssparser::{ParseError, Parser, Token};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Unit([i16; 6]);
impl Unit {
    // CSS base types are independent: frequency is not inverse time.
    const NUMBER: Self = Self([0, 0, 0, 0, 0, 0]);
    const LENGTH: Self = Self([1, 0, 0, 0, 0, 0]);
    const ANGLE: Self = Self([0, 1, 0, 0, 0, 0]);
    const TIME: Self = Self([0, 0, 1, 0, 0, 0]);
    const FREQUENCY: Self = Self([0, 0, 0, 1, 0, 0]);
    const RESOLUTION: Self = Self([0, 0, 0, 0, 1, 0]);
    const FLEX: Self = Self([0, 0, 0, 0, 0, 1]);
    fn combine(self, other: Self, divide: bool) -> Option<Self> {
        let mut powers = self.0;
        for (power, rhs) in powers.iter_mut().zip(other.0) {
            *power = if divide {
                power.checked_sub(rhs)?
            } else {
                power.checked_add(rhs)?
            };
        }
        Some(Self(powers))
    }
}
#[derive(Clone, Copy)]
struct Value {
    number: f64,
    unit: Unit,
}
type Result<'i, T> = std::result::Result<T, ParseError<'i, ()>>;

pub fn evaluate<'i>(name: &str, parser: &mut Parser<'i, '_>, depth: usize) -> Result<'i, String> {
    let value = function(name, parser, depth)?;
    if !value.number.is_finite() {
        return Err(parser.new_custom_error(()));
    }
    let suffix = match value.unit {
        Unit::NUMBER => "",
        Unit::LENGTH => "px",
        Unit::ANGLE => "deg",
        _ => return Err(parser.new_custom_error(())),
    };
    Ok(format!("{}{suffix}", value.number))
}

fn complete_value(text: &str) -> Option<Value> {
    let mut input = cssparser::ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    let value = atom(&mut parser, 0).ok()?;
    parser.expect_exhausted().ok()?;
    value.number.is_finite().then_some(value)
}

/// Evaluate a dimensionless CSS value, including a math function.
pub fn resolve_number(text: &str) -> Option<f64> {
    let value = complete_value(text)?;
    (value.unit == Unit::NUMBER).then_some(value.number)
}

/// Evaluate an angle in degrees; unitless zero is accepted by transforms.
pub fn resolve_angle(text: &str) -> Option<f64> {
    let value = complete_value(text)?;
    (value.unit == Unit::ANGLE || value.unit == Unit::NUMBER && value.number == 0.)
        .then_some(value.number)
}

fn function<'i>(name: &str, parser: &mut Parser<'i, '_>, depth: usize) -> Result<'i, Value> {
    if depth > 32 {
        return Err(parser.new_custom_error(()));
    }
    if name.eq_ignore_ascii_case("calc") {
        let value = sum(parser, depth + 1)?;
        parser.expect_exhausted()?;
        return Ok(value);
    }
    let name = name.to_ascii_lowercase();
    if ![
        "min", "max", "clamp", "abs", "sign", "hypot", "sin", "cos", "tan", "asin", "acos", "atan",
        "atan2", "pow", "sqrt", "exp", "log", "round", "mod", "rem",
    ]
    .contains(&name.as_str())
    {
        return Err(parser.new_custom_error(()));
    }
    let strategy = if name == "round" {
        parser
            .try_parse(|p| {
                let strategy = p.expect_ident()?.to_ascii_lowercase();
                if !["nearest", "up", "down", "to-zero"].contains(&strategy.as_str()) {
                    return Err(p.new_custom_error::<(), ()>(()));
                }
                p.expect_comma()?;
                Ok(strategy)
            })
            .unwrap_or_else(|_| "nearest".into())
    } else {
        String::new()
    };
    let values = parser.parse_comma_separated(|p| sum(p, depth + 1))?;
    let Some(first) = values.first().copied() else {
        return Err(parser.new_custom_error(()));
    };
    let same_unit = values.iter().all(|v| v.unit == first.unit);
    let scalar = values.iter().all(|v| v.unit == Unit::NUMBER);
    let mut unit = first.unit;
    let number = match name.as_str() {
        "round"
            if same_unit
                && (values.len() == 2 || (values.len() == 1 && first.unit == Unit::NUMBER)) =>
        {
            let step = if values.len() == 2 {
                values[1].number.abs()
            } else {
                1.0
            };
            if step == 0.0 {
                return Err(parser.new_custom_error(()));
            }
            let quotient = first.number / step;
            let rounded = match strategy.as_str() {
                "up" => quotient.ceil(),
                "down" => quotient.floor(),
                "to-zero" => quotient.trunc(),
                _ => {
                    let lower = quotient.floor();
                    if quotient - lower < 0.5 {
                        lower
                    } else {
                        lower + 1.0
                    }
                }
            };
            if rounded == 0.0 {
                0.0_f64.copysign(first.number)
            } else {
                rounded * step
            }
        }
        "mod" | "rem" if same_unit && values.len() == 2 => {
            let divisor = values[1].number;
            if divisor == 0.0 {
                return Err(parser.new_custom_error(()));
            }
            let remainder = first.number % divisor;
            if name == "rem" {
                remainder
            } else if remainder == 0.0 {
                0.0_f64.copysign(divisor)
            } else if remainder.is_sign_negative() != divisor.is_sign_negative() {
                remainder + divisor
            } else {
                remainder
            }
        }
        "min" if same_unit => values.iter().fold(f64::INFINITY, |a, v| a.min(v.number)),
        "max" if same_unit => values
            .iter()
            .fold(f64::NEG_INFINITY, |a, v| a.max(v.number)),
        "clamp" if same_unit && values.len() == 3 => {
            values[0].number.max(values[1].number.min(values[2].number))
        }
        "hypot" if same_unit => values.iter().fold(0.0_f64, |a, v| a.hypot(v.number)),
        "abs" if values.len() == 1 => first.number.abs(),
        "sign" if values.len() == 1 => {
            unit = Unit::NUMBER;
            if first.number == 0.0 || first.number.is_nan() {
                first.number
            } else {
                first.number.signum()
            }
        }
        "sin" | "cos" | "tan"
            if values.len() == 1 && matches!(first.unit, Unit::NUMBER | Unit::ANGLE) =>
        {
            unit = Unit::NUMBER;
            let radians = if first.unit == Unit::ANGLE {
                first.number.to_radians()
            } else {
                first.number
            };
            match name.as_str() {
                "sin" => radians.sin(),
                "cos" => radians.cos(),
                _ => radians.tan(),
            }
        }
        "asin" | "acos" | "atan" if scalar && values.len() == 1 => {
            unit = Unit::ANGLE;
            match name.as_str() {
                "asin" => first.number.asin(),
                "acos" => first.number.acos(),
                _ => first.number.atan(),
            }
            .to_degrees()
        }
        "atan2" if same_unit && values.len() == 2 => {
            unit = Unit::ANGLE;
            first.number.atan2(values[1].number).to_degrees()
        }
        "pow" if scalar && values.len() == 2 => first.number.powf(values[1].number),
        "sqrt" if scalar && values.len() == 1 => first.number.sqrt(),
        "exp" if scalar && values.len() == 1 => first.number.exp(),
        "log" if scalar && values.len() == 1 => first.number.ln(),
        "log" if scalar && values.len() == 2 => first.number.log(values[1].number),
        _ => return Err(parser.new_custom_error(())),
    };
    Ok(Value { number, unit })
}

fn sum<'i>(parser: &mut Parser<'i, '_>, depth: usize) -> Result<'i, Value> {
    let mut value = product(parser, depth)?;
    loop {
        let op = parser.try_parse(|p| {
            let start = p.position();
            let token = p.next()?.clone();
            let op = match token {
                Token::Delim(op @ ('+' | '-')) => op,
                _ => return Err(p.new_custom_error::<(), ()>(())),
            };
            // CSS binary + and - require whitespace on both sides.
            let before = p.slice_from(start);
            if !before.starts_with(|c: char| c.is_ascii_whitespace()) {
                return Err(p.new_custom_error(()));
            }
            if !matches!(
                p.next_including_whitespace_and_comments()?,
                Token::WhiteSpace(_)
            ) {
                return Err(p.new_custom_error(()));
            }
            Ok(op)
        });
        let Ok(op) = op else {
            break;
        };
        let right = product(parser, depth)?;
        if value.unit != right.unit {
            return Err(parser.new_custom_error(()));
        }
        value.number = if op == '+' {
            value.number + right.number
        } else {
            value.number - right.number
        };
    }
    Ok(value)
}

fn product<'i>(parser: &mut Parser<'i, '_>, depth: usize) -> Result<'i, Value> {
    let mut value = atom(parser, depth)?;
    loop {
        let op = parser.try_parse(|p| match p.next()?.clone() {
            Token::Delim(op @ ('*' | '/')) => Ok(op),
            _ => Err(p.new_custom_error::<(), ()>(())),
        });
        let Ok(op) = op else {
            break;
        };
        let right = atom(parser, depth)?;
        value.unit = value
            .unit
            .combine(right.unit, op == '/')
            .ok_or_else(|| parser.new_custom_error(()))?;
        if op == '*' {
            value.number *= right.number;
        } else {
            if right.number == 0.0 {
                return Err(parser.new_custom_error(()));
            }
            value.number /= right.number;
        }
    }
    Ok(value)
}

fn atom<'i>(parser: &mut Parser<'i, '_>, depth: usize) -> Result<'i, Value> {
    if depth > 32 {
        return Err(parser.new_custom_error(()));
    }
    let start = parser.position();
    let token = parser.next()?.clone();
    let raw = parser.slice_from(start).trim();
    match token {
        Token::Ident(name) if name.eq_ignore_ascii_case("pi") => Ok(Value {
            number: std::f64::consts::PI,
            unit: Unit::NUMBER,
        }),
        Token::Ident(name) if name.eq_ignore_ascii_case("e") => Ok(Value {
            number: std::f64::consts::E,
            unit: Unit::NUMBER,
        }),
        Token::Number { value, .. } => Ok(Value {
            number: raw.parse().unwrap_or(value as f64),
            unit: Unit::NUMBER,
        }),
        Token::Dimension { value, unit, .. } => {
            let unit_name = unit.to_ascii_lowercase();
            let (kind, factor) = match unit_name.as_str() {
                "px" => (Unit::LENGTH, 1.0),
                "in" => (Unit::LENGTH, 96.0),
                "cm" => (Unit::LENGTH, 96.0 / 2.54),
                "mm" => (Unit::LENGTH, 96.0 / 25.4),
                "q" => (Unit::LENGTH, 96.0 / 101.6),
                "pt" => (Unit::LENGTH, 96.0 / 72.0),
                "pc" => (Unit::LENGTH, 16.0),
                "deg" => (Unit::ANGLE, 1.0),
                "rad" => (Unit::ANGLE, 180.0 / std::f64::consts::PI),
                "grad" => (Unit::ANGLE, 0.9),
                "turn" => (Unit::ANGLE, 360.0),
                "s" => (Unit::TIME, 1.0),
                "ms" => (Unit::TIME, 0.001),
                "hz" => (Unit::FREQUENCY, 1.0),
                "khz" => (Unit::FREQUENCY, 1000.0),
                "dppx" | "x" => (Unit::RESOLUTION, 1.0),
                "dpi" => (Unit::RESOLUTION, 1.0 / 96.0),
                "dpcm" => (Unit::RESOLUTION, 2.54 / 96.0),
                "fr" => (Unit::FLEX, 1.0),
                _ => return Err(parser.new_custom_error(())),
            };
            let number = raw
                .strip_suffix(unit.as_ref())
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(value as f64);
            Ok(Value {
                number: number * factor,
                unit: kind,
            })
        }
        Token::ParenthesisBlock => parser.parse_nested_block(|p| {
            let v = sum(p, depth + 1)?;
            p.expect_exhausted()?;
            Ok(v)
        }),
        Token::Function(name) => parser.parse_nested_block(|p| function(&name, p, depth + 1)),
        _ => Err(parser.new_custom_error(())),
    }
}

/// Inputs needed to resolve layout-relative units before typed math evaluation.
#[derive(Clone, Copy)]
pub struct LengthContext {
    pub percentage_basis: f64,
    pub font_size: f64,
    pub root_font_size: f64,
    pub viewport_width: f64,
    pub viewport_height: f64,
}

impl LengthContext {
    fn factor(self, unit: &str) -> Option<f64> {
        Some(match unit {
            "em" => self.font_size,
            "rem" => self.root_font_size,
            "vw" => self.viewport_width / 100.,
            "vh" => self.viewport_height / 100.,
            "vmin" => self.viewport_width.min(self.viewport_height) / 100.,
            "vmax" => self.viewport_width.max(self.viewport_height) / 100.,
            _ => return None,
        })
    }
}

fn resolve_relative_tokens<'i>(
    parser: &mut Parser<'i, '_>,
    context: Option<LengthContext>,
    depth: usize,
) -> Result<'i, String> {
    if depth > 32 {
        return Err(parser.new_custom_error(()));
    }
    let mut output = String::new();
    while !parser.is_exhausted() {
        let start = parser.position();
        let token = parser.next_including_whitespace_and_comments()?.clone();
        let raw = parser.slice_from(start);
        let value = match token {
            Token::Percentage { unit_value, .. } => {
                let percentage = raw
                    .strip_suffix('%')
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(unit_value as f64 * 100.);
                match context {
                    Some(context) => format!("{}px", percentage * context.percentage_basis / 100.),
                    None => format!("{}", percentage / 100.),
                }
            }
            Token::Dimension { value, unit, .. }
                if context
                    .and_then(|c| c.factor(&unit.to_ascii_lowercase()))
                    .is_some() =>
            {
                let number = raw
                    .strip_suffix(unit.as_ref())
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(value as f64);
                format!(
                    "{}px",
                    number
                        * context
                            .and_then(|c| c.factor(&unit.to_ascii_lowercase()))
                            .unwrap()
                )
            }
            Token::Function(name) => {
                let body = parser
                    .parse_nested_block(|p| resolve_relative_tokens(p, context, depth + 1))?;
                format!("{name}({body})")
            }
            Token::ParenthesisBlock => {
                let body = parser
                    .parse_nested_block(|p| resolve_relative_tokens(p, context, depth + 1))?;
                format!("({body})")
            }
            _ => raw.to_owned(),
        };
        output.push_str(&value);
    }
    Ok(output)
}

/// Returns a finite used length, rejecting expressions with another final type.
/// Font-metric units require additional metrics and are deliberately unresolved.
pub fn resolve_length(text: &str, context: LengthContext) -> Option<f64> {
    let mut input = cssparser::ParserInput::new(text);
    let normalized =
        resolve_relative_tokens(&mut Parser::new(&mut input), Some(context), 0).ok()?;
    let value = complete_value(&normalized)?;
    (value.number.is_finite()
        && (value.unit == Unit::LENGTH || value.unit == Unit::NUMBER && value.number == 0.))
        .then_some(value.number)
}

/// Resolve a CSS number-percentage, with 100% representing a factor of one.
/// This percentage hint is for scale factors, not matrix coefficients or axes.
pub fn resolve_scale(text: &str) -> Option<f64> {
    let mut input = cssparser::ParserInput::new(text);
    let normalized = resolve_relative_tokens(&mut Parser::new(&mut input), None, 0).ok()?;
    resolve_number(&normalized)
}

/// Resolve a CSS position against the free space on each axis (which may be
/// negative for a covered image). Nested math functions remain single values.
pub fn resolve_position(text: &str, x: LengthContext, y: LengthContext) -> Option<(f64, f64)> {
    let mut input = cssparser::ParserInput::new(text);
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
                    Ok::<(), ParseError<'_, ()>>(())
                })
                .ok()?;
        }
        values.push(parser.slice_from(start).trim().to_owned());
    }
    let resolve = |value: &str, context: LengthContext| match value {
        "left" | "top" => Some(0.),
        "center" => Some(context.percentage_basis / 2.),
        "right" | "bottom" => Some(context.percentage_basis),
        _ => resolve_length(value, context),
    };
    match values.len() {
        1 => {
            if matches!(values[0].as_str(), "top" | "bottom") {
                Some((x.percentage_basis / 2., resolve(&values[0], y)?))
            } else {
                Some((resolve(&values[0], x)?, y.percentage_basis / 2.))
            }
        }
        2 => {
            if matches!(values[0].as_str(), "top" | "bottom")
                || matches!(values[1].as_str(), "left" | "right")
            {
                values.swap(0, 1);
            }
            Some((resolve(&values[0], x)?, resolve(&values[1], y)?))
        }
        3 | 4 => {
            let (mut horizontal, mut vertical) = (None, None);
            let mut index = 0;
            while index < values.len() {
                let side = values[index].as_str();
                index += 1;
                if side == "center" {
                    let is_x = vertical.is_some()
                        || (horizontal.is_none()
                            && !values[index..]
                                .iter()
                                .any(|v| matches!(v.as_str(), "left" | "right")));
                    let (target, context) = if is_x {
                        (&mut horizontal, x)
                    } else {
                        (&mut vertical, y)
                    };
                    if target.replace(context.percentage_basis / 2.).is_some() {
                        return None;
                    }
                    continue;
                }
                let is_x = matches!(side, "left" | "right");
                if !is_x && !matches!(side, "top" | "bottom") {
                    return None;
                }
                let context = if is_x { x } else { y };
                let offset = if let Some(value) =
                    values.get(index).and_then(|v| resolve_length(v, context))
                {
                    index += 1;
                    value
                } else {
                    0.
                };
                let position = if matches!(side, "right" | "bottom") {
                    context.percentage_basis - offset
                } else {
                    offset
                };
                let target = if is_x { &mut horizontal } else { &mut vertical };
                if target.replace(position).is_some() {
                    return None;
                }
            }
            Some((horizontal?, vertical?))
        }
        _ => None,
    }
}

#[cfg(test)]
mod length_tests {
    use super::*;
    #[test]
    fn contextual_math_resolves_percentages_and_relative_units() {
        let context = LengthContext {
            percentage_basis: 200.,
            font_size: 20.,
            root_font_size: 16.,
            viewport_width: 1000.,
            viewport_height: 500.,
        };
        for (input, expected) in [
            ("calc(50% + 10px)", 110.),
            ("clamp(1em, 5vw, 4rem)", 50.),
            ("min(10vmin, 10vmax)", 50.),
            ("calc((100% - 2em) / 2)", 80.),
            ("calc(1in + 2.54cm)", 192.),
            ("calc(1px * (2s / 1s))", 2.),
        ] {
            assert_eq!(resolve_length(input, context), Some(expected), "{input}");
        }
        for input in [
            "1deg",
            "calc(1px + 2)",
            "calc(1px * 2px)",
            "1ex",
            "calc(1px / 0)",
            "calc(1px+ 2px)",
            "1px + 2px",
        ] {
            assert_eq!(resolve_length(input, context), None, "{input}");
        }
    }

    #[test]
    fn scalar_and_angle_math_keep_their_result_types() {
        assert_eq!(resolve_scale("150%"), Some(1.5));
        assert_eq!(resolve_scale("calc(50% + 1)"), Some(1.5));
        assert_eq!(resolve_scale("clamp(50%, 200%, 1.5)"), Some(1.5));
        assert_eq!(resolve_scale("calc(100% * 1px)"), None);
        assert_eq!(resolve_number("150%"), None);
        assert_eq!(resolve_angle("150%"), None);
        assert_eq!(resolve_angle("calc(0.5turn - 90deg)"), Some(90.));
        assert_eq!(resolve_angle("atan2(1, 1)"), Some(45.));
        assert_eq!(resolve_number("calc(2 * cos(0deg))"), Some(2.));
        assert_eq!(resolve_number("calc(4px / 2px)"), Some(2.));
        assert_eq!(resolve_number("1px"), None);
        assert_eq!(resolve_angle("calc(2 + 3)"), None);
        assert_eq!(resolve_number("2 + 3"), None);
    }
}
