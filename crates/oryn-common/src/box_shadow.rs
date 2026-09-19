use crate::{color::parse_color, v4::CssRect};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoxShadow {
    pub offset: [f64; 2],
    pub blur: f64,
    pub spread: f64,
    pub color: [u8; 4],
    pub inset: bool,
}
fn words(value: &str) -> Option<Vec<&str>> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    for (i, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            _ => {}
        }
        if ch.is_whitespace() && depth == 0 {
            if i > start {
                result.push(&value[start..i]);
            }
            start = i + ch.len_utf8();
        }
    }
    if depth != 0 {
        return None;
    }
    if start < value.len() {
        result.push(&value[start..]);
    }
    Some(result)
}
pub fn parse(value: &str, current: &str, font: f64) -> Option<Vec<BoxShadow>> {
    if value == "none" {
        return Some(Vec::new());
    }
    let mut result = Vec::new();
    for layer in split_layers(value)? {
        let mut lengths = Vec::new();
        let mut color = None;
        let mut inset = false;
        for word in words(layer)? {
            if word == "inset" {
                if inset {
                    return None;
                }
                inset = true;
                continue;
            }
            if let Some(value) = parse_color(if word == "currentcolor" {
                current
            } else {
                word
            }) {
                if color.replace(value).is_some() {
                    return None;
                }
                continue;
            }
            let length = if let Some(value) = word.strip_suffix("px") {
                value.parse::<f64>().ok()?
            } else if let Some(value) = word.strip_suffix("em") {
                value.parse::<f64>().ok()? * font
            } else if word == "0" {
                0.
            } else {
                return None;
            };
            if !length.is_finite() {
                return None;
            }
            lengths.push(length);
        }
        if !(2..=4).contains(&lengths.len()) || lengths.get(2).is_some_and(|blur| *blur < 0.) {
            return None;
        }
        result.push(BoxShadow {
            offset: [lengths[0], lengths[1]],
            blur: lengths.get(2).copied().unwrap_or(0.),
            spread: lengths.get(3).copied().unwrap_or(0.),
            color: color
                .or_else(|| parse_color(current))
                .unwrap_or([0, 0, 0, 255]),
            inset,
        });
        if result.len() > 256 {
            return None;
        }
    }
    Some(result)
}
fn spread_rect(rect: CssRect, spread: f64, offset: [f64; 2]) -> CssRect {
    CssRect {
        x: rect.x - spread + offset[0],
        y: rect.y - spread + offset[1],
        width: (rect.width + 2. * spread).max(0.),
        height: (rect.height + 2. * spread).max(0.),
    }
}
pub fn bounds(rect: CssRect, shadow: &BoxShadow) -> CssRect {
    if shadow.inset {
        return rect;
    }
    let rect = spread_rect(rect, shadow.spread, shadow.offset);
    let extent = shadow.blur * 2.;
    CssRect {
        x: rect.x - extent,
        y: rect.y - extent,
        width: rect.width + extent * 2.,
        height: rect.height + extent * 2.,
    }
}
fn split_layers(value: &str) -> Option<Vec<&str>> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        match ch {
            '(' => depth = depth.checked_add(1)?,
            ')' => depth = depth.checked_sub(1)?,
            ',' if depth == 0 => {
                result.push(value[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    result.push(value[start..].trim());
    Some(result)
}
