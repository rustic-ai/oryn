//! WebDriver print option conversion and native paginated PDF capture.
use super::CommandError;
use oryn_common::v4::PrintOptions;
use serde_json::{Map, Value};

fn object<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>, CommandError> {
    value
        .as_object()
        .ok_or_else(|| CommandError::invalid(format!("{label} must be an object")))
}
fn number(
    fields: &Map<String, Value>,
    key: &str,
    default: f64,
    min: f64,
    max: f64,
) -> Result<f64, CommandError> {
    match fields.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_f64()
            .filter(|value| value.is_finite() && *value >= min && *value <= max)
            .ok_or_else(|| CommandError::invalid(format!("invalid print {key}"))),
    }
}
fn boolean(fields: &Map<String, Value>, key: &str, default: bool) -> Result<bool, CommandError> {
    fields.get(key).map_or(Ok(default), |value| {
        value
            .as_bool()
            .ok_or_else(|| CommandError::invalid(format!("invalid print {key}")))
    })
}
fn page_number(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok().filter(|value| *value > 0)
}
fn range(value: &Value) -> Result<String, CommandError> {
    if let Some(value) = value
        .as_f64()
        .filter(|value| value.fract() == 0. && *value >= 1. && *value <= 9_007_199_254_740_991.)
    {
        return Ok(format!("{value:.0}"));
    }
    let text = value
        .as_str()
        .ok_or_else(|| CommandError::invalid("invalid print page range"))?;
    let valid = if let Some((start, end)) = text.split_once('-') {
        let start = if start.is_empty() {
            Some(1)
        } else {
            page_number(start)
        };
        let end = if end.is_empty() {
            Some(u64::MAX)
        } else {
            page_number(end)
        };
        text != "-" && matches!((start,end),(Some(a),Some(b)) if a<=b)
    } else {
        page_number(text).is_some()
    };
    if !valid {
        return Err(CommandError::invalid("invalid print page range"));
    }
    Ok(text.to_string())
}

pub fn parse(value: &Value) -> Result<PrintOptions, CommandError> {
    let fields = object(value, "print parameters")?;
    let mut options = PrintOptions::default();
    options.landscape = match fields.get("orientation") {
        None => false,
        Some(Value::String(value)) if value == "portrait" => false,
        Some(Value::String(value)) if value == "landscape" => true,
        _ => return Err(CommandError::invalid("invalid print orientation")),
    };
    options.scale = number(fields, "scale", options.scale, 0.1, 2.)?;
    options.background = boolean(fields, "background", options.background)?;
    options.shrink_to_fit = boolean(fields, "shrinkToFit", options.shrink_to_fit)?;
    if let Some(page) = fields.get("page") {
        let page = object(page, "print page")?;
        options.page_width_cm = number(page, "width", options.page_width_cm, 0.0352, f64::MAX)?;
        options.page_height_cm = number(page, "height", options.page_height_cm, 0.0352, f64::MAX)?;
    }
    if let Some(margin) = fields.get("margin") {
        let margin = object(margin, "print margin")?;
        options.margin_top_cm = number(margin, "top", options.margin_top_cm, 0., f64::MAX)?;
        options.margin_right_cm = number(margin, "right", options.margin_right_cm, 0., f64::MAX)?;
        options.margin_bottom_cm =
            number(margin, "bottom", options.margin_bottom_cm, 0., f64::MAX)?;
        options.margin_left_cm = number(margin, "left", options.margin_left_cm, 0., f64::MAX)?;
    }
    if let Some(ranges) = fields.get("pageRanges") {
        let ranges = ranges
            .as_array()
            .ok_or_else(|| CommandError::invalid("pageRanges must be an array"))?;
        if ranges.len() > 4096 {
            return Err(CommandError::invalid("too many print ranges"));
        }
        options.page_ranges = ranges.iter().map(range).collect::<Result<Vec<_>, _>>()?;
    }
    Ok(options)
}

#[cfg(feature = "render")]
pub async fn capture(
    page: &oryn_native::PageHandle,
    parameters: &Value,
) -> Result<String, CommandError> {
    use base64::Engine as _;
    let options = parse(parameters)?;
    let snapshot = page
        .print_snapshot(options)
        .await
        .map_err(|error| CommandError {
            code: "unknown error",
            message: error.to_string(),
        })?;
    let artifact = tokio::task::spawn_blocking(move || {
        oryn_render::render_print_pdf(&snapshot, oryn_render::RenderOptions::default())
    })
    .await
    .map_err(|_| CommandError {
        code: "unknown error",
        message: "native print renderer task failed".into(),
    })?
    .map_err(|error| CommandError {
        code: "unknown error",
        message: error.to_string(),
    })?;
    if !artifact.diagnostics.is_empty() {
        return Err(CommandError {
            code: "unknown error",
            message: format!(
                "native print incomplete: {}",
                artifact.diagnostics.join("; ")
            ),
        });
    }
    if artifact.bytes.len() > 64 * 1024 * 1024 {
        return Err(CommandError {
            code: "unknown error",
            message: "encoded print exceeds output limit".into(),
        });
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(artifact.bytes))
}
#[cfg(not(feature = "render"))]
pub async fn capture(
    _page: &oryn_native::PageHandle,
    parameters: &Value,
) -> Result<String, CommandError> {
    parse(parameters)?;
    Err(CommandError {
        code: "unsupported operation",
        message: "native print requires render feature".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn upstream_print_reftest_options_preserve_physical_units() {
        let options=parse(&json!({"page":{"width":10,"height":20},"margin":{"top":1.27,"right":1.27,"bottom":1.27,"left":1.27},"background":true,"shrinkToFit":false})).unwrap();
        assert_eq!(options.page_width_cm, 10.);
        assert_eq!(options.margin_left_cm, 1.27);
        assert!(options.background);
        assert!(!options.shrink_to_fit);
        assert_eq!(parse(&json!({})).unwrap(), PrintOptions::default());
    }
    #[test]
    fn ranges_accept_open_and_overlapping_ranges_without_losing_order() {
        let options = parse(&json!({"pageRanges":["2-4","2-3","9-","-2",7]})).unwrap();
        assert_eq!(options.page_ranges, vec!["2-4", "2-3", "9-", "-2", "7"]);
    }
    #[test]
    fn invalid_print_values_are_rejected_before_capture() {
        for value in [
            json!({"orientation":0}),
            json!({"scale":"1"}),
            json!({"scale":3}),
            json!({"scale":0.01}),
            json!({"margin":{"bottom":-1}}),
            json!({"page":{"height":false}}),
            json!({"shrinkToFit":"false"}),
            json!({"pageRanges":["3-2"]}),
            json!({"pageRanges":["a-2"]}),
            json!({"pageRanges":["1-2-3"]}),
            json!({"pageRanges":[null]}),
            json!({"pageRanges":["-"]}),
            json!({"pageRanges":[0]}),
        ] {
            assert_eq!(
                parse(&value).unwrap_err().code,
                "invalid argument",
                "{value}"
            );
        }
    }
}
