//! Select settings for the explicit, single synthetic device per kind. Required
//! constraints intersect; advanced sets are retained only when satisfiable.
use oryn_js::media_capture::Kind;
use serde_json::{Value, json};
fn required(value: &Value, advanced: bool) -> Option<&Value> {
    value
        .get("exact")
        .or_else(|| (advanced && !value.is_object()).then_some(value))
}
fn matches(value: &Value, actual: &Value) -> bool {
    value == actual
        || (value
            .as_f64()
            .zip(actual.as_f64())
            .is_some_and(|(a, b)| a == b))
        || value
            .as_array()
            .is_some_and(|values| values.iter().any(|value| matches(value, actual)))
}
fn accepts(sets: &[(&Value, bool)], name: &str, actual: &Value) -> bool {
    sets.iter().all(|(set, advanced)| {
        let Some(value) = set.get(name) else {
            return true;
        };
        if required(value, *advanced).is_some_and(|value| !matches(value, actual)) {
            return false;
        }
        if let Some(actual) = actual.as_f64()
            && (value
                .get("min")
                .and_then(Value::as_f64)
                .is_some_and(|min| actual < min)
                || value
                    .get("max")
                    .and_then(Value::as_f64)
                    .is_some_and(|max| actual > max))
        {
            return false;
        }
        true
    })
}
fn fitness(base: &Value, name: &str, actual: f64, _default: f64) -> f64 {
    let Some(ideal) = base
        .get(name)
        .and_then(|value| {
            if value.is_object() {
                value.get("ideal")
            } else {
                Some(value)
            }
        })
        .and_then(Value::as_f64)
    else {
        return 0.;
    };
    if ideal == actual {
        0.
    } else {
        (ideal - actual).abs() / ideal.abs().max(actual.abs()).max(1.)
    }
}
fn bounds(
    sets: &[(&Value, bool)],
    name: &str,
    min: f64,
    max: f64,
    integer: bool,
) -> Result<(f64, f64), String> {
    let mut low = min;
    let mut high = max;
    for (set, advanced) in sets {
        if let Some(value) = set.get(name) {
            if let Some(exact) = required(value, *advanced) {
                let exact = exact
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| name.to_owned())?;
                low = low.max(exact);
                high = high.min(exact)
            }
            if let Some(value) = value.get("min").and_then(Value::as_f64) {
                low = low.max(value)
            }
            if let Some(value) = value.get("max").and_then(Value::as_f64) {
                high = high.min(value)
            }
        }
    }
    if integer {
        low = low.ceil();
        high = high.floor()
    }
    if low > high {
        return Err(name.into());
    }
    Ok((low, high))
}
fn numeric(
    sets: &[(&Value, bool)],
    base: &Value,
    name: &str,
    default: f64,
    min: f64,
    max: f64,
    integer: bool,
) -> Result<f64, String> {
    let (low, high) = bounds(sets, name, min, max, integer)?;
    let ideal = base
        .get(name)
        .and_then(|value| {
            value
                .get("ideal")
                .or_else(|| (!value.is_object()).then_some(value))
        })
        .and_then(Value::as_f64)
        .unwrap_or(default);
    let chosen = ideal.clamp(low, high);
    Ok(if integer {
        chosen.round().clamp(low, high)
    } else {
        chosen
    })
}
fn solve(kind: Kind, base: &Value, sets: &[(&Value, bool)]) -> Result<Value, String> {
    let mut settings = match kind {
        Kind::Audio => {
            json!({"deviceId":"oryn-fixture-audio-v1","groupId":"oryn-fixture-group-v1","echoCancellation":false,"autoGainControl":false,"noiseSuppression":false,"latency":0.01})
        }
        Kind::Video => {
            json!({"deviceId":"oryn-fixture-video-v1","groupId":"oryn-fixture-group-v1","facingMode":"user","resizeMode":"none","backgroundBlur":false})
        }
    };
    for (name, value) in settings.as_object().unwrap() {
        if !accepts(sets, name, value) {
            return Err(name.clone());
        }
    }
    match kind {
        Kind::Audio => {
            for (name, default, min, max) in [
                ("sampleRate", 48000., 8000., 48000.),
                ("channelCount", 1., 1., 2.),
                ("sampleSize", 16., 16., 16.),
            ] {
                settings[name] = json!(numeric(sets, base, name, default, min, max, true)?)
            }
        }
        Kind::Video => {
            let frame_rate = numeric(sets, base, "frameRate", 30., 1., 60., false)?;
            // Integer dimensions and aspectRatio constrain each other. Evaluate
            // finite synthetic dimensions; select by basic-set fitness distance.
            let width = numeric(sets, base, "width", 640., 1., 1920., true)?;
            let height = numeric(sets, base, "height", 480., 1., 1080., true)?;
            let mut best = None;
            if accepts(sets, "aspectRatio", &json!(width / height)) {
                best = Some((
                    fitness(base, "width", width, 640.)
                        + fitness(base, "height", height, 480.)
                        + fitness(base, "aspectRatio", width / height, 4. / 3.),
                    width,
                    height,
                ));
            }
            let has_ratio = sets.iter().any(|(set, _)| set.get("aspectRatio").is_some());
            if has_ratio {
                let (wlo, whi) = bounds(sets, "width", 1., 1920., true)?;
                let (hlo, hhi) = bounds(sets, "height", 1., 1080., true)?;
                let (rlo, rhi) = bounds(sets, "aspectRatio", 1. / 1080., 1920., false)?;
                let ratio_ideal = base
                    .get("aspectRatio")
                    .and_then(|value| {
                        value
                            .get("ideal")
                            .or_else(|| (!value.is_object()).then_some(value))
                    })
                    .and_then(Value::as_f64)
                    .unwrap_or(4. / 3.);
                for h in hlo as u32..=hhi as u32 {
                    let low = wlo.max((h as f64 * rlo).ceil() - 1.);
                    let high = whi.min((h as f64 * rhi).floor() + 1.);
                    if low > high {
                        continue;
                    }
                    // Within each interval fitness is monotone or concave, so
                    // its minimum lies at a boundary or an ideal breakpoint.
                    let preferred = width.clamp(low, high);
                    let ratio = (ratio_ideal * h as f64).clamp(low, high);
                    for w in [
                        low,
                        low + 1.,
                        high - 1.,
                        high,
                        preferred.floor(),
                        preferred.ceil(),
                        ratio.floor(),
                        ratio.ceil(),
                    ] {
                        if w < low
                            || w > high
                            || !accepts(sets, "width", &json!(w))
                            || !accepts(sets, "aspectRatio", &json!(w / h as f64))
                        {
                            continue;
                        }
                        let score = fitness(base, "width", w, 640.)
                            + fitness(base, "height", h as f64, 480.)
                            + fitness(base, "aspectRatio", w / h as f64, 4. / 3.);
                        if best.as_ref().is_none_or(|(old, _, _)| score < *old) {
                            best = Some((score, w, h as f64));
                        }
                    }
                }
            }
            let (_, width, height) = best.ok_or_else(|| "aspectRatio".to_owned())?;
            settings["width"] = json!(width as u32);
            settings["height"] = json!(height as u32);
            settings["aspectRatio"] = json!(width / height);
            settings["frameRate"] = json!(frame_rate);
        }
    }
    Ok(settings)
}
pub(crate) fn select(kind: Kind, constraints: &Value) -> Result<Value, String> {
    if !constraints.is_object() {
        return Err("TypeError: track constraints must be a dictionary".into());
    }
    let mut sets = vec![(constraints, false)];
    let mut settings =
        solve(kind, constraints, &sets).map_err(|name| format!("OverconstrainedError: {name}"))?;
    if let Some(advanced) = constraints.get("advanced") {
        let advanced = advanced
            .as_array()
            .filter(|values| values.len() <= 128)
            .ok_or("TypeError: advanced constraints must be a bounded sequence")?;
        for set in advanced {
            if !set.is_object() {
                return Err("TypeError: advanced constraints must contain dictionaries".into());
            }
            sets.push((set, true));
            match solve(kind, constraints, &sets) {
                Ok(next) => settings = next,
                Err(_) => {
                    sets.pop();
                }
            }
        }
    }
    Ok(settings)
}
pub(crate) fn capabilities(kind: Kind) -> Value {
    match kind {
        Kind::Audio => {
            json!({"deviceId":"oryn-fixture-audio-v1","groupId":"oryn-fixture-group-v1","sampleRate":{"min":8000,"max":48000},"sampleSize":{"min":16,"max":16},"channelCount":{"min":1,"max":2},"latency":{"min":0.01,"max":0.01},"echoCancellation":[false],"autoGainControl":[false],"noiseSuppression":[false]})
        }
        Kind::Video => {
            json!({"deviceId":"oryn-fixture-video-v1","groupId":"oryn-fixture-group-v1","width":{"min":1,"max":1920},"height":{"min":1,"max":1080},"aspectRatio":{"min":1./1080.,"max":1920.},"frameRate":{"min":1,"max":60},"facingMode":["user"],"resizeMode":["none"],"backgroundBlur":[false]})
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_intersection_and_advanced_backoff_preserve_feasible_settings() {
        let settings=select(Kind::Video,&json!({"width":{"min":640,"ideal":1280},"height":{"exact":720},"advanced":[{"width":9000},{"width":1280},{"width":640}]})).unwrap();
        assert_eq!(settings["width"], 1280);
        assert_eq!(settings["height"], 720);
        assert!(
            select(Kind::Video, &json!({"width":{"min":700,"max":640}}))
                .unwrap_err()
                .contains("width")
        );
        assert!(select(Kind::Video, &json!({"width":{"exact":1.5}})).is_err());
    }
    #[test]
    fn device_identity_processing_and_coupled_dimensions_are_truthful() {
        assert!(select(Kind::Audio, &json!({"echoCancellation":{"exact":true}})).is_err());
        assert_eq!(
            select(Kind::Audio, &json!({"echoCancellation":{"ideal":true}})).unwrap()["echoCancellation"],
            false
        );
        assert!(
            select(
                Kind::Video,
                &json!({"deviceId":{"exact":["absent","oryn-fixture-video-v1"]}})
            )
            .is_ok()
        );
        let settings=select(Kind::Video,&json!({"width":{"exact":640},"height":{"min":300,"max":500},"aspectRatio":{"exact":4./3.}})).unwrap();
        assert_eq!(settings["height"], 480);
        assert!(
            select(
                Kind::Video,
                &json!({"width":{"exact":640},"height":{"exact":400},"aspectRatio":{"exact":4./3.}})
            )
            .is_err()
        );
    }
}
