//! Parent-only physical device selection and AVFoundation custody.
use oryn_js::media_capture::{Kind, Track};
use serde_json::{Value, json};
use std::ffi::{CStr, CString, c_char, c_void};
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn oryn_capture_inventory(video: i32) -> *mut c_char;
    fn oryn_capture_free(value: *mut c_char);
    fn oryn_capture_open(
        video: i32,
        id: *const c_char,
        format: u32,
        rate: f64,
        status: *mut i32,
    ) -> *mut c_void;
    fn oryn_capture_sample(capture: *mut c_void) -> *mut c_char;
    fn oryn_capture_close(capture: *mut c_void);
}
#[derive(Debug)]
pub(super) struct Capture {
    #[cfg(target_os = "macos")]
    raw: *mut c_void,
    pub(super) capabilities: Value,
}
// Every adapter operation is serialized by the parent registry/source mutex;
// AVFoundation callbacks only access their own locked latest-sample storage.
unsafe impl Send for Capture {}
#[cfg(target_os = "macos")]
impl Drop for Capture {
    fn drop(&mut self) {
        unsafe { oryn_capture_close(self.raw) }
    }
}
#[cfg(target_os = "macos")]
fn json_result(raw: *mut c_char) -> Result<Value, String> {
    if raw.is_null() {
        return Err("NotReadableError: native capture response allocation".into());
    }
    let result = unsafe {
        let bytes = CStr::from_ptr(raw).to_bytes();
        if bytes.len() > 24 * 1024 * 1024 {
            Err("NotReadableError: capture response bound".into())
        } else {
            serde_json::from_slice(bytes)
                .map_err(|_| "NotReadableError: invalid native capture response".into())
        }
    };
    unsafe { oryn_capture_free(raw) };
    result
}
fn exact(value: &Value, actual: &Value, advanced: bool) -> bool {
    let required = value
        .get("exact")
        .or_else(|| (advanced && !value.is_object()).then_some(value));
    let matches = |value: &Value| {
        value == actual
            || value
                .as_f64()
                .zip(actual.as_f64())
                .is_some_and(|(a, b)| a == b)
    };
    if required.is_some_and(|value| {
        !matches(value)
            && !value
                .as_array()
                .is_some_and(|values| values.iter().any(matches))
    }) {
        return false;
    }
    if let Some(number) = actual.as_f64()
        && (value
            .get("min")
            .and_then(Value::as_f64)
            .is_some_and(|min| number < min)
            || value
                .get("max")
                .and_then(Value::as_f64)
                .is_some_and(|max| number > max))
    {
        return false;
    }
    true
}
fn supported(name: &str) -> bool {
    matches!(
        name,
        "deviceId"
            | "groupId"
            | "width"
            | "height"
            | "aspectRatio"
            | "frameRate"
            | "facingMode"
            | "resizeMode"
            | "sampleRate"
            | "sampleSize"
            | "channelCount"
            | "latency"
            | "echoCancellation"
            | "autoGainControl"
            | "noiseSuppression"
            | "backgroundBlur"
    )
}
fn absent(name: &str, value: &Value, advanced: bool) -> bool {
    !supported(name)
        || (!advanced
            && value.get("exact").is_none()
            && value.get("min").is_none()
            && value.get("max").is_none())
}
fn accepts(set: &Value, settings: &Value, advanced: bool) -> bool {
    set.as_object().is_none_or(|set| {
        set.iter().all(|(key, value)| {
            key == "advanced"
                || settings.get(key).map_or_else(
                    || absent(key, value, advanced),
                    |actual| exact(value, actual, advanced),
                )
        })
    })
}
fn score(base: &Value, settings: &Value) -> f64 {
    settings
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, actual)| {
            let Some(value) = base.get(key) else {
                return 0.;
            };
            let ideal = value
                .get("ideal")
                .or_else(|| (!value.is_object()).then_some(value));
            match ideal {
                Some(ideal) if ideal == actual => 0.,
                Some(ideal) => ideal
                    .as_f64()
                    .zip(actual.as_f64())
                    .map(|(a, b)| (a - b).abs() / a.abs().max(b.abs()).max(1.))
                    .unwrap_or(1.),
                None => 0.,
            }
        })
        .sum()
}
#[derive(Clone)]
struct Candidate {
    raw: String,
    label: String,
    format: u32,
    settings: Value,
}
fn candidates(
    inventory: &Value,
    kind: Kind,
    salt: &[u8; 24],
    constraints: &Value,
) -> Vec<Candidate> {
    use sha2::{Digest, Sha256};
    let mut candidates = Vec::new();
    for device in inventory["devices"].as_array().into_iter().flatten() {
        let Some(raw) = device["id"].as_str() else {
            continue;
        };
        let mut hash = Sha256::new();
        hash.update(salt);
        hash.update(raw.as_bytes());
        let opaque = format!("{:x}", hash.finalize());
        for format in device["formats"].as_array().into_iter().flatten() {
            let Some(index) = format["index"]
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
            else {
                continue;
            };
            let mut settings = json!({"deviceId":opaque,"groupId":""});
            if kind == Kind::Video {
                settings["width"] = format["width"].clone();
                settings["height"] = format["height"].clone();
                settings["aspectRatio"] = json!(
                    format["width"].as_f64().unwrap_or(0.)
                        / format["height"].as_f64().unwrap_or(1.)
                );
                settings["resizeMode"] = json!("none");
                settings["backgroundBlur"] = json!(false);
                for range in format["rates"].as_array().into_iter().flatten() {
                    let Some(min) = range[0].as_f64() else {
                        continue;
                    };
                    let Some(max) = range[1].as_f64() else {
                        continue;
                    };
                    if !min.is_finite() || !max.is_finite() || min <= 0. || max < min {
                        continue;
                    }
                    let value = &constraints["frameRate"];
                    let low = value["min"].as_f64().unwrap_or(min).max(min);
                    let high = value["max"].as_f64().unwrap_or(max).min(max);
                    if low > high {
                        continue;
                    }
                    let requested = value["exact"]
                        .as_f64()
                        .or(value["ideal"].as_f64())
                        .or(value.as_f64())
                        .unwrap_or(30.)
                        .clamp(low, high);
                    let mut choices = vec![requested, low, high];
                    for advanced in constraints["advanced"].as_array().into_iter().flatten() {
                        let value = &advanced["frameRate"];
                        for breakpoint in [
                            value.as_f64(),
                            value["exact"].as_f64(),
                            value["min"].as_f64(),
                            value["max"].as_f64(),
                            value["ideal"].as_f64(),
                        ]
                        .into_iter()
                        .flatten()
                        {
                            if breakpoint.is_finite() && breakpoint >= low && breakpoint <= high {
                                choices.push(breakpoint)
                            }
                        }
                    }
                    for rate in choices {
                        settings["frameRate"] = json!(rate);
                        candidates.push(Candidate {
                            raw: raw.into(),
                            label: device["label"].as_str().unwrap_or("").into(),
                            format: index,
                            settings: settings.clone(),
                        });
                    }
                }
            } else {
                settings["sampleRate"] = format["sampleRate"].clone();
                settings["channelCount"] = format["channelCount"].clone();
                settings["sampleSize"] = json!(32);
                candidates.push(Candidate {
                    raw: raw.into(),
                    label: device["label"].as_str().unwrap_or("").into(),
                    format: index,
                    settings,
                });
            }
        }
    }
    candidates
}
fn choose(mut candidates: Vec<Candidate>, constraints: &Value) -> Result<Candidate, String> {
    if candidates.is_empty() {
        return Err("NotFoundError: no supported physical capture format".into());
    }
    if let Some(values) = constraints.as_object() {
        for (name, value) in values {
            if name == "advanced" {
                continue;
            }
            candidates.retain(|candidate| {
                candidate.settings.get(name).map_or_else(
                    || absent(name, value, false),
                    |actual| exact(value, actual, false),
                )
            });
            if candidates.is_empty() {
                return Err(format!("OverconstrainedError: {name}"));
            }
        }
    }
    for advanced in constraints["advanced"].as_array().into_iter().flatten() {
        let filtered = candidates
            .iter()
            .filter(|candidate| accepts(advanced, &candidate.settings, true))
            .cloned()
            .collect::<Vec<_>>();
        if !filtered.is_empty() {
            candidates = filtered
        }
    }
    candidates
        .into_iter()
        .min_by(|a, b| score(constraints, &a.settings).total_cmp(&score(constraints, &b.settings)))
        .ok_or_else(|| "NotFoundError: no physical capture device".into())
}
pub(super) fn open(
    kind: Kind,
    constraints: Value,
    salt: &[u8; 24],
) -> Result<(Track, Capture), String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (kind, constraints, salt);
        Err("NotFoundError: physical capture provider unavailable on this platform".into())
    }
    #[cfg(target_os = "macos")]
    {
        let inventory =
            unsafe { json_result(oryn_capture_inventory(i32::from(kind == Kind::Video))) }?;
        let selected = choose(
            candidates(&inventory, kind, salt, &constraints),
            &constraints,
        )?;
        let raw = CString::new(selected.raw.clone())
            .map_err(|_| "NotReadableError: invalid native device identity")?;
        let mut status = 0;
        let capture = unsafe {
            oryn_capture_open(
                i32::from(kind == Kind::Video),
                raw.as_ptr(),
                selected.format,
                selected.settings["frameRate"].as_f64().unwrap_or(0.),
                &mut status,
            )
        };
        if capture.is_null() {
            return Err(match status {
                1 => "NotAllowedError: OS capture permission is not granted",
                2 => "NotFoundError: device disconnected",
                _ => "NotReadableError: AVFoundation capture could not start",
            }
            .into());
        }
        let all = candidates(&inventory, kind, salt, &json!({}));
        let mut capabilities = json!({});
        for (name, value) in selected.settings.as_object().unwrap() {
            let values = all
                .iter()
                .filter(|candidate| candidate.raw == selected.raw)
                .filter_map(|candidate| candidate.settings.get(name))
                .collect::<Vec<_>>();
            capabilities[name] = if value.is_number() {
                let numbers = values
                    .iter()
                    .filter_map(|value| value.as_f64())
                    .collect::<Vec<_>>();
                json!({"min":numbers.iter().copied().fold(f64::INFINITY,f64::min),"max":numbers.iter().copied().fold(f64::NEG_INFINITY,f64::max)})
            } else if name == "deviceId" || name == "groupId" {
                value.clone()
            } else {
                json!([value])
            };
        }
        let capture = Capture {
            raw: capture,
            capabilities,
        };
        Ok((
            Track {
                id: super::id()?,
                kind,
                label: selected.label,
                enabled: true,
                ended: false,
                settings: selected.settings,
                constraints,
            },
            capture,
        ))
    }
}
impl Capture {
    pub(super) fn sample(&mut self) -> Result<Value, String> {
        #[cfg(target_os = "macos")]
        {
            unsafe { json_result(oryn_capture_sample(self.raw)) }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err("NotReadableError: physical capture unavailable".into())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_selection_checks_all_devices_required_constraints_and_advanced_backoff() {
        let inventory = json!({"devices":[{"id":"host-camera-a","label":"A","formats":[{"index":0,"width":640,"height":480,"rates":[[15,30]]}]},{"id":"host-camera-b","label":"B","formats":[{"index":0,"width":1280,"height":720,"rates":[[24,60]]}]}]});
        let constraints =
            json!({"width":{"exact":1280},"frameRate":{"ideal":60},"advanced":[{"width":9999}]});
        let selected = choose(
            candidates(&inventory, Kind::Video, &[1; 24], &constraints),
            &constraints,
        )
        .unwrap();
        assert_eq!(selected.raw, "host-camera-b");
        assert_eq!(selected.settings["frameRate"], 60.);
        assert!(
            !selected.settings["deviceId"]
                .as_str()
                .unwrap()
                .contains("host-camera")
        );
        assert_ne!(
            candidates(&inventory, Kind::Video, &[1; 24], &json!({}))[0].settings["deviceId"],
            candidates(&inventory, Kind::Video, &[2; 24], &json!({}))[0].settings["deviceId"]
        );
        assert!(
            choose(
                candidates(&inventory, Kind::Video, &[1; 24], &json!({})),
                &json!({"width":{"exact":9999}})
            )
            .is_err()
        );
    }
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn oryn_audio_open(rate: f64, channels: u32) -> *mut c_void;
    fn oryn_audio_write(sink: *mut c_void, bytes: *const u8, size: usize, volume: f32) -> i32;
    fn oryn_audio_close(sink: *mut c_void);
    fn oryn_rate_sink_open(
        rate: f64,
        channels: u32,
        playback_rate: f64,
        preserves_pitch: i32,
    ) -> *mut c_void;
    fn oryn_rate_sink_write(
        sink: *mut c_void,
        bytes: *const u8,
        size: usize,
        volume: f32,
        reverse: i32,
    ) -> i32;
    fn oryn_rate_sink_audio(sink: *mut c_void, after: u64) -> *mut c_char;
    fn oryn_rate_sink_close(sink: *mut c_void);
}
#[derive(Debug)]
pub(super) struct AudioSink {
    #[cfg(target_os = "macos")]
    raw: *mut c_void,
    pub(super) rate: f64,
    pub(super) channels: u32,
    rate_processing: bool,
}
unsafe impl Send for AudioSink {}
impl AudioSink {
    pub(super) fn open(rate: f64, channels: u32) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            let raw = unsafe { oryn_audio_open(rate, channels) };
            if raw.is_null() {
                return Err("NotReadableError: native audio output unavailable".into());
            }
            Ok(Self {
                raw,
                rate,
                channels,
                rate_processing: false,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (rate, channels);
            Err("NotSupportedError: native audio output unavailable".into())
        }
    }
    pub(super) fn open_with_rate(
        rate: f64,
        channels: u32,
        playback_rate: f64,
        preserves_pitch: bool,
    ) -> Result<Self, String> {
        if !rate.is_finite()
            || !(8000.0..=192000.0).contains(&rate)
            || !(1..=8).contains(&channels)
            || !playback_rate.is_finite()
        {
            return Err("TypeError: invalid audio output configuration".into());
        }
        let magnitude = playback_rate.abs();
        let audible = if preserves_pitch {
            (1.0 / 32.0..=32.0).contains(&magnitude)
        } else {
            (0.25..=4.0).contains(&magnitude)
        };
        if !audible {
            return Err("NotSupportedError: silent rate has no physical sink".into());
        }
        #[cfg(target_os = "macos")]
        {
            let raw = unsafe {
                oryn_rate_sink_open(rate, channels, playback_rate, i32::from(preserves_pitch))
            };
            if raw.is_null() {
                return Err("NotReadableError: rate audio output unavailable".into());
            }
            Ok(Self {
                raw,
                rate,
                channels,
                rate_processing: true,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err("NotSupportedError: native rate audio output unavailable".into())
        }
    }
    pub(super) fn write(&mut self, bytes: &[u8], volume: f32) -> Result<bool, String> {
        self.write_directional(bytes, volume, false)
    }
    pub(super) fn write_directional(
        &mut self,
        bytes: &[u8],
        volume: f32,
        reverse: bool,
    ) -> Result<bool, String> {
        if bytes.is_empty()
            || bytes.len() > 1024 * 1024
            || !(1..=8).contains(&self.channels)
            || bytes.len() % (self.channels as usize * 4) != 0
            || !volume.is_finite()
            || !(0.0..=1.0).contains(&volume)
        {
            return Err("TypeError: invalid audio output window".into());
        }
        if reverse && !self.rate_processing {
            return Err("InvalidStateError: reverse window requires rate sink".into());
        }
        #[cfg(target_os = "macos")]
        {
            let result = unsafe {
                if self.rate_processing {
                    oryn_rate_sink_write(
                        self.raw,
                        bytes.as_ptr(),
                        bytes.len(),
                        volume,
                        i32::from(reverse),
                    )
                } else {
                    oryn_audio_write(self.raw, bytes.as_ptr(), bytes.len(), volume)
                }
            };
            match result {
                1 => Ok(true),
                2 => Ok(false),
                _ => Err("NotReadableError: native audio output unavailable".into()),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (bytes, volume, reverse);
            Err("NotSupportedError: native audio output unavailable".into())
        }
    }
    pub(super) fn processed_audio(&mut self, after: u64) -> Result<Value, String> {
        if !self.rate_processing {
            return Err("InvalidStateError: sink has no processed PCM observation".into());
        }
        #[cfg(target_os = "macos")]
        {
            json_result(unsafe { oryn_rate_sink_audio(self.raw, after) })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = after;
            Err("NotSupportedError: native PCM observation unavailable".into())
        }
    }
}
#[cfg(target_os = "macos")]
impl Drop for AudioSink {
    fn drop(&mut self) {
        unsafe {
            if self.rate_processing {
                oryn_rate_sink_close(self.raw)
            } else {
                oryn_audio_close(self.raw)
            }
        }
    }
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn oryn_capture_authorization(video: i32, request: i32) -> i32;
}
pub(super) fn authorized(request: bool, constraints: &serde_json::Value) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        let mut ready = true;
        for video in [0, 1] {
            if constraints.get(if video == 1 { "video" } else { "audio" })
                == Some(&serde_json::Value::Bool(false))
            {
                continue;
            }
            match unsafe { oryn_capture_authorization(video, i32::from(request)) } {
                1 => {}
                0 => ready = false,
                2 => return Err("NotAllowedError: OS capture permission denied".into()),
                _ => {
                    return Err(
                        "NotAllowedError: parent application lacks capture usage declaration"
                            .into(),
                    );
                }
            }
        }
        Ok(ready)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (request, constraints);
        Err("NotFoundError: physical capture provider unavailable".into())
    }
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn oryn_capture_alive(capture: *mut c_void) -> i32;
}
impl Capture {
    pub(super) fn alive(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            unsafe { oryn_capture_alive(self.raw) != 0 }
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}
