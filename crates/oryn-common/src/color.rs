//! Shared sRGB color parsing for native raster consumers.
//! Context-dependent colors and wide-gamut conversion belong to the style engine.

pub fn parse_color(value: &str) -> Option<[u8; 4]> {
    let mut input = cssparser::ParserInput::new(value);
    let mut parser = cssparser::Parser::new(&mut input);
    let token = parser.next().ok()?.clone();
    let value = match token {
        cssparser::Token::Ident(name) => {
            parser.expect_exhausted().ok()?;
            if name.eq_ignore_ascii_case("transparent") {
                return Some([0, 0, 0, 0]);
            }
            let (r, g, b) = cssparser::color::parse_named_color(&name.to_ascii_lowercase()).ok()?;
            return Some([r, g, b, 255]);
        }
        cssparser::Token::Hash(hex) | cssparser::Token::IDHash(hex) => {
            parser.expect_exhausted().ok()?;
            format!("#{hex}")
        }
        cssparser::Token::Function(name) => {
            let body = parser
                .parse_nested_block::<_, _, ()>(|nested| {
                    let mut parts = Vec::new();
                    while !nested.is_exhausted() {
                        let start = nested.position();
                        let token = nested.next_including_whitespace_and_comments()?.clone();
                        let raw = nested.slice_from(start);
                        let part = match token {
                            cssparser::Token::WhiteSpace(_) | cssparser::Token::Comment(_) => {
                                continue;
                            }
                            cssparser::Token::Number { .. }
                            | cssparser::Token::Percentage { .. } => raw.to_owned(),
                            cssparser::Token::Dimension { value, unit, .. } => {
                                let number = raw
                                    .strip_suffix(unit.as_ref())
                                    .and_then(|n| n.parse::<f64>().ok())
                                    .unwrap_or(value as f64);
                                format!("{number}{}", unit.to_ascii_lowercase())
                            }
                            cssparser::Token::Ident(ident)
                                if ident.eq_ignore_ascii_case("none") =>
                            {
                                "none".into()
                            }
                            cssparser::Token::Comma => ",".into(),
                            cssparser::Token::Delim('/') => "/".into(),
                            _ => return Err(nested.new_custom_error(())),
                        };
                        parts.push(part);
                    }
                    Ok(parts.join(" "))
                })
                .ok()?;
            parser.expect_exhausted().ok()?;
            format!("{}({body})", name.to_ascii_lowercase())
        }
        _ => return None,
    };
    if let Some(hex) = value.strip_prefix('#') {
        // Validate bytes before slicing: arbitrary UTF-8 is valid input here.
        if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let mut rgba = [0, 0, 0, 255];
        match hex.len() {
            3 | 4 => {
                for (index, byte) in hex.bytes().enumerate() {
                    rgba[index] = (byte as char).to_digit(16)? as u8 * 17;
                }
            }
            6 | 8 => {
                for index in 0..hex.len() / 2 {
                    rgba[index] = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
                }
            }
            _ => return None,
        }
        return Some(rgba);
    }
    let (name, body) = value.split_once('(')?;
    if !matches!(name, "rgb" | "rgba" | "hsl" | "hsla" | "hwb") {
        return None;
    }
    let body = body.strip_suffix(')')?;
    let legacy = body.contains(',');
    let (channels, alpha) = if body.contains(',') {
        if body.contains('/') || name == "hwb" {
            return None;
        }
        let parts: Vec<_> = body.split(',').map(str::trim).collect();
        if !(3..=4).contains(&parts.len()) {
            return None;
        }
        if matches!(name, "rgb" | "rgba") {
            let percentage = parts[0].ends_with('%');
            if parts[..3]
                .iter()
                .any(|part| part.ends_with('%') != percentage)
            {
                return None;
            }
        }
        (parts[..3].to_vec(), parts.get(3).copied())
    } else {
        let (channels, alpha) = body
            .split_once('/')
            .map_or((body, None), |(c, a)| (c, Some(a.trim())));
        (channels.split_ascii_whitespace().collect::<Vec<_>>(), alpha)
    };
    if channels.len() != 3 {
        return None;
    }
    let component = |text: &str, maximum: f64| -> Option<u8> {
        if !legacy && text == "none" {
            return Some(0);
        }
        let (number, scale) = text
            .strip_suffix('%')
            .map_or((text, maximum), |number| (number, 100.0));
        let number = number.parse::<f64>().ok()?;
        if !number.is_finite() {
            return None;
        }
        Some((number.clamp(0.0, scale) / scale * 255.0).round() as u8)
    };
    let alpha = match alpha {
        Some(alpha) => component(alpha, 1.0)?,
        None => 255,
    };
    if matches!(name, "rgb" | "rgba") {
        return Some([
            component(channels[0], 255.0)?,
            component(channels[1], 255.0)?,
            component(channels[2], 255.0)?,
            alpha,
        ]);
    }
    let hue = if !legacy && channels[0] == "none" {
        0.0
    } else {
        let text = channels[0];
        let (number, factor) = if let Some(n) = text.strip_suffix("deg") {
            (n, 1.0)
        } else if let Some(n) = text.strip_suffix("grad") {
            (n, 0.9)
        } else if let Some(n) = text.strip_suffix("rad") {
            (n, 180.0 / std::f64::consts::PI)
        } else if let Some(n) = text.strip_suffix("turn") {
            (n, 360.0)
        } else {
            (text, 1.0)
        };
        let hue = number.parse::<f64>().ok()? * factor;
        if !hue.is_finite() {
            return None;
        }
        hue.rem_euclid(360.0) / 60.0
    };
    let percentage = |text: &str| -> Option<f64> {
        if !legacy && text == "none" {
            return Some(0.0);
        }
        let n = text.strip_suffix('%')?.parse::<f64>().ok()?;
        n.is_finite().then(|| n.clamp(0.0, 100.0) / 100.0)
    };
    let second = percentage(channels[1])?;
    let third = percentage(channels[2])?;
    let (chroma, offset) = if name == "hwb" {
        if second + third >= 1.0 {
            let gray = (255.0 * second / (second + third)).round() as u8;
            return Some([gray, gray, gray, alpha]);
        }
        (1.0 - second - third, second)
    } else {
        let chroma = (1.0 - (2.0 * third - 1.0).abs()) * second;
        (chroma, third - chroma / 2.0)
    };
    let x = chroma * (1.0 - (hue % 2.0 - 1.0).abs());
    let rgb = match hue as u8 {
        0 => [chroma, x, 0.0],
        1 => [x, chroma, 0.0],
        2 => [0.0, chroma, x],
        3 => [0.0, x, chroma],
        4 => [x, 0.0, chroma],
        _ => [chroma, 0.0, x],
    };
    let byte = |c: f64| ((c + offset).clamp(0.0, 1.0) * 255.0).round() as u8;
    Some([byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), alpha])
}

#[cfg(test)]
mod tests {
    use super::parse_color;

    #[test]
    fn parses_alpha_hex_and_rgb_syntax() {
        for (text, expected) in [
            ("#1234", [17, 34, 51, 68]),
            ("#12345678", [18, 52, 86, 120]),
            ("RGB(100%, 0%, 50%)", [255, 0, 128, 255]),
            ("rgba(255, 0, 0, .5)", [255, 0, 0, 128]),
            ("rgb(255 0 50% / 25%)", [255, 0, 128, 64]),
            ("rgb(999 -5 20)", [255, 0, 20, 255]),
        ] {
            assert_eq!(parse_color(text), Some(expected), "{text}");
        }
    }

    #[test]
    fn functional_colors_use_css_tokens_and_reject_trailing_input() {
        for (text, expected) in [
            (r"\72 gb(255 0 0)", [255, 0, 0, 255]),
            ("/**/ rgb(255/**/ 0 0 / .5) /**/", [255, 0, 0, 128]),
            ("hsl(120/**/,100%/**/,50%)", [0, 255, 0, 255]),
            (r"hsl(.5t\75 rn 100% 50%)", [0, 255, 255, 255]),
            (r"rgb(n\6f ne 0 255)", [0, 0, 255, 255]),
            ("rgb(2.55e2 0 0)", [255, 0, 0, 255]),
        ] {
            assert_eq!(parse_color(text), Some(expected), "{text}");
        }
        for text in [
            "rgb (255 0 0)",
            "rgb(255 0 0) red",
            "rgb(255 0 0)!important",
            "rgb(255 0 0))",
            "rgb('255' 0 0)",
            "rgb(255px 0 0)",
            "rgb(255 0 0 / 50 %)",
            "hsl(.5 turn 100% 50%)",
            "rgb(0xFF 0 0)",
        ] {
            assert_eq!(parse_color(text), None, "{text}");
        }
    }

    #[test]
    fn parses_standard_named_colors_and_css_identifier_escapes() {
        for (text, expected) in [
            ("aliceblue", [240, 248, 255, 255]),
            ("DarkSlateGrey", [47, 79, 79, 255]),
            ("papayawhip", [255, 239, 213, 255]),
            ("lightgoldenrodyellow", [250, 250, 210, 255]),
            ("rebeccapurple", [102, 51, 153, 255]),
            (r"\72 ed", [255, 0, 0, 255]),
            ("/**/ blue /**/", [0, 0, 255, 255]),
            ("/**/#1234/**/", [17, 34, 51, 68]),
            (r"#\66 f0000", [255, 0, 0, 255]),
        ] {
            assert_eq!(parse_color(text), Some(expected), "{text}");
        }
        for text in [
            "red blue",
            "red!important",
            "light/**/blue",
            "#123 #456",
            "currentcolor",
            "inherit",
            "canvastext",
        ] {
            assert_eq!(parse_color(text), None, "{text}");
        }
    }

    #[test]
    fn parses_hue_units_hsl_and_hwb() {
        for (text, expected) in [
            ("hsl(120, 100%, 50%)", [0, 255, 0, 255]),
            ("hsla(-240deg, 100%, 50%, .5)", [0, 255, 0, 128]),
            ("hsl(.5turn 100% 50% / 25%)", [0, 255, 255, 64]),
            ("hsl(200grad 100% 50%)", [0, 255, 255, 255]),
            ("hsl(3.141592653589793rad 100% 50%)", [0, 255, 255, 255]),
            ("hsl(720 100% 50%)", [255, 0, 0, 255]),
            ("hwb(0 20% 40%)", [153, 51, 51, 255]),
            ("hwb(120 80% 80% / .5)", [128, 128, 128, 128]),
            ("hsl(none 100% 50%)", [255, 0, 0, 255]),
            ("hwb(0 none none)", [255, 0, 0, 255]),
            ("rgb(none 128 255 / none)", [0, 128, 255, 0]),
        ] {
            assert_eq!(parse_color(text), Some(expected), "{text}");
        }
        for text in [
            "hsl(0, 1, .5)",
            "hsl(0 100% 50% 1)",
            "hwb(0, 20%, 40%)",
            "hsl(1px 100% 50%)",
            "hsl(none, 100%, 50%)",
            "hwb(0 20 40)",
            "hsl(0 100% 50% / NaN)",
            "hsl(0,100%,50% / .5)",
        ] {
            assert_eq!(parse_color(text), None, "{text}");
        }
    }

    #[test]
    fn rejects_malformed_and_unicode_hex_without_slicing_panics() {
        for text in [
            "#éa",
            "#💚aa",
            "#12345",
            "rgb(1,2,3 / .5)",
            "rgb(1%,2,3)",
            "rgb(1 2)",
            "rgb(1 2 3 / .5 / .2)",
            "rgb(NaN 2 3)",
            "rgba(1,2,3,)",
        ] {
            assert_eq!(parse_color(text), None, "{text}");
        }
    }
}
