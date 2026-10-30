//! Decoded URL media lives in the parent and is bounded independently of capture.
//! The decoder sees only bytes supplied by the policy-checked resource loader.
use super::{AgentOwner, system};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ffi::{CStr, c_char, c_void},
};
#[cfg(test)]
fn test_owner(generation: u64) -> AgentOwner {
    AgentOwner::Window {
        page: oryn_common::v3::PageId(1),
        target: oryn_js::message_wire::WindowTarget::Root,
        generation,
        epoch: 0,
    }
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn oryn_webm_open_diagnostic(
        bytes: *const u8,
        length: usize,
        manifest: *const u8,
        manifest_length: usize,
        diagnostic: *mut *mut c_char,
    ) -> *mut c_void;
    #[cfg(test)]
    fn oryn_webm_open_for_test(
        bytes: *const u8,
        length: usize,
        manifest: *const u8,
        manifest_length: usize,
        mode: u32,
        diagnostic: *mut *mut c_char,
    ) -> *mut c_void;
    #[cfg(test)]
    fn oryn_webm_decoder_unit_checks() -> i32;
    #[cfg(test)]
    fn oryn_webm_opus_unit_checks() -> i32;
    #[cfg(test)]
    fn oryn_webm_unindexed_bounds_check(asset: *mut c_void) -> i32;
    fn oryn_webm_metadata(asset: *mut c_void) -> *mut c_char;
    fn oryn_webm_sample(
        asset: *mut c_void,
        time: f64,
        audio_start: f64,
        audio_until: f64,
    ) -> *mut c_char;
    fn oryn_webm_close(asset: *mut c_void);
    fn oryn_webm_audio(asset: *mut c_void, start: f64, until: f64) -> *mut c_char;
    fn oryn_url_media_open(bytes: *const u8, length: usize) -> *mut c_void;
    fn oryn_url_media_metadata(asset: *mut c_void) -> *mut c_char;
    fn oryn_url_media_sample(
        asset: *mut c_void,
        time: f64,
        audio_start: f64,
        audio_until: f64,
    ) -> *mut c_char;
    fn oryn_url_media_close(asset: *mut c_void);
    fn oryn_url_media_audio(asset: *mut c_void, start: f64, until: f64) -> *mut c_char;
    fn oryn_capture_free(value: *mut c_char);
}
struct PendingAudio {
    plan: super::audio_windows::Plan,
    index: usize,
    bytes: Option<Vec<u8>>,
}
impl PendingAudio {
    fn acknowledge(&mut self, accepted: bool) -> Option<f64> {
        if !accepted {
            return None;
        }
        let segment = self.plan.segments().get(self.index)?;
        let cursor = if segment.reverse {
            segment.start
        } else {
            segment.end
        };
        self.bytes = None;
        self.index += 1;
        Some(cursor)
    }
}
struct Asset {
    audio: Option<super::decoded_audio::Audio>,
    duration: f64,
    audio_format: Option<(u32, u32)>,
    owner: AgentOwner,
    bytes: usize,
    #[cfg(target_os = "macos")]
    raw: *mut c_void,
    webm: bool,
    sink: Option<system::AudioSink>,
    last_audio: f64,
    playback_rate: f64,
    preserves_pitch: bool,
    looping: bool,
    paused: bool,
    volume: f32,
    processing_epoch: u64,
    processing_reset: bool,
    pending_audio: Option<PendingAudio>,
    pip: Option<String>,
}
unsafe impl Send for Asset {}
#[cfg(target_os = "macos")]
impl Drop for Asset {
    fn drop(&mut self) {
        unsafe {
            if let Some(id) = &self.pip {
                super::picture_in_picture::close(id);
            }
            if !self.raw.is_null() {
                if self.webm {
                    oryn_webm_close(self.raw)
                } else {
                    oryn_url_media_close(self.raw)
                }
            }
        }
    }
}
#[derive(Default)]
pub(super) struct Registry {
    assets: BTreeMap<String, Asset>,
}
#[cfg(target_os = "macos")]
fn result(raw: *mut c_char) -> Result<Value, String> {
    if raw.is_null() {
        return Err("NotSupportedError: media decode failed".into());
    }
    let value = unsafe {
        let bytes = CStr::from_ptr(raw).to_bytes();
        if bytes.len() > 24 * 1024 * 1024 {
            Err("QuotaExceededError: decoded media sample".into())
        } else {
            serde_json::from_slice(bytes)
                .map_err(|_| "NotReadableError: malformed decoded media sample".into())
        }
    };
    unsafe { oryn_capture_free(raw) };
    value
}
#[cfg(any(test, target_os = "macos"))]
fn decode_audio_window(
    sample: &Value,
    limit: usize,
    format: (u32, u32),
) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let Some(value) = sample.get("audio") else {
        return Ok(Vec::new());
    };
    let encoded = value
        .as_str()
        .ok_or("NotReadableError: malformed audio window")?;
    if !(8000..=192000).contains(&format.0)
        || !(1..=8).contains(&format.1)
        || sample["sampleRate"].as_f64() != Some(format.0 as f64)
        || sample["channels"].as_u64() != Some(format.1 as u64)
    {
        return Err("NotReadableError: changed audio window format".into());
    }
    if encoded.len() % 4 != 0 || encoded.len() > 1_398_104 {
        return Err("QuotaExceededError: decoded media audio transfer".into());
    }
    let padding = if encoded.ends_with("==") {
        2
    } else if encoded.ends_with('=') {
        1
    } else {
        0
    };
    let length = (encoded.len() / 4 * 3)
        .checked_sub(padding)
        .filter(|length| {
            *length <= limit && *length <= 1024 * 1024 && *length % (format.1 as usize * 4) == 0
        })
        .ok_or("QuotaExceededError: decoded media audio transfer")?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| "QuotaExceededError: decoded media audio transfer")?;
    bytes.resize(length, 0);
    let count = base64::engine::general_purpose::STANDARD
        .decode_slice(encoded, &mut bytes)
        .map_err(|_| "NotReadableError: malformed audio sample")?;
    bytes.truncate(count);
    Ok(bytes)
}
impl Registry {
    pub(super) fn open(&mut self, owner: &AgentOwner, bytes: &[u8]) -> Result<Value, String> {
        self.open_mode(owner, bytes, 0)
    }
    fn open_mode(&mut self, owner: &AgentOwner, bytes: &[u8], mode: u32) -> Result<Value, String> {
        #[cfg(any(not(test), not(target_os = "macos")))]
        let _ = mode;
        if bytes.is_empty()
            || bytes.len() > 16 * 1024 * 1024
            || self.assets.len() >= 64
            || self
                .assets
                .values()
                .map(|asset| asset.bytes)
                .sum::<usize>()
                .saturating_add(bytes.len())
                > 128 * 1024 * 1024
        {
            return Err("QuotaExceededError: decoded media asset inventory".into());
        }
        if let Ok(audio) = super::decoded_audio::decode(bytes) {
            let retained = audio.bytes();
            if self
                .assets
                .values()
                .map(|asset| asset.bytes)
                .sum::<usize>()
                .saturating_add(retained)
                > 128 * 1024 * 1024
            {
                return Err("QuotaExceededError: decoded media asset inventory".into());
            }
            let id = super::id()?;
            let metadata = serde_json::json!({"id":id,"duration":audio.duration(),"width":0,"height":0,"audio":true});
            self.assets.insert(
                id,
                Asset {
                    duration: audio.duration(),
                    audio_format: Some((audio.rate, audio.channels)),
                    audio: Some(audio),
                    owner: owner.clone(),
                    bytes: retained,
                    #[cfg(target_os = "macos")]
                    raw: std::ptr::null_mut(),
                    webm: false,
                    sink: None,
                    last_audio: 0.,
                    playback_rate: 1.,
                    preserves_pitch: true,
                    looping: false,
                    paused: true,
                    volume: 1.,
                    processing_epoch: 1,
                    processing_reset: false,
                    pending_audio: None,
                    pip: None,
                },
            );
            return Ok(metadata);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = owner;
            Err("NotSupportedError: URL media decoder unavailable".into())
        }
        #[cfg(target_os = "macos")]
        {
            let webm = bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]);
            let raw = if webm {
                let movie = super::webm::parse(bytes)?;
                let manifest =
                    serde_json::to_vec(&movie).map_err(|_| "NotSupportedError: WebM metadata")?;
                if manifest.len() > 32 * 1024 * 1024 {
                    return Err("QuotaExceededError: WebM packet inventory".into());
                }
                let mut diagnostic = std::ptr::null_mut();
                let raw = unsafe {
                    #[cfg(test)]
                    {
                        if mode != 0 {
                            oryn_webm_open_for_test(
                                bytes.as_ptr(),
                                bytes.len(),
                                manifest.as_ptr(),
                                manifest.len(),
                                mode,
                                &mut diagnostic,
                            )
                        } else {
                            oryn_webm_open_diagnostic(
                                bytes.as_ptr(),
                                bytes.len(),
                                manifest.as_ptr(),
                                manifest.len(),
                                &mut diagnostic,
                            )
                        }
                    }
                    #[cfg(not(test))]
                    {
                        oryn_webm_open_diagnostic(
                            bytes.as_ptr(),
                            bytes.len(),
                            manifest.as_ptr(),
                            manifest.len(),
                            &mut diagnostic,
                        )
                    }
                };
                if !diagnostic.is_null() {
                    let details = result(diagnostic)?;
                    if raw.is_null() {
                        return Err(format!(
                            "NotSupportedError: WebM decoder initialization: {details}"
                        ));
                    }
                }
                raw
            } else {
                unsafe { oryn_url_media_open(bytes.as_ptr(), bytes.len()) }
            };
            if raw.is_null() {
                return Err("NotSupportedError: unsupported media resource".into());
            }
            let mut asset = Asset {
                audio: None,
                duration: 0.,
                audio_format: None,
                owner: owner.clone(),
                bytes: bytes.len(),
                raw,
                webm,
                sink: None,
                last_audio: 0.,
                playback_rate: 1.,
                preserves_pitch: true,
                looping: false,
                paused: true,
                volume: 1.,
                processing_epoch: 1,
                processing_reset: false,
                pending_audio: None,
                pip: None,
            };
            let mut metadata = unsafe {
                result(if webm {
                    oryn_webm_metadata(raw)
                } else {
                    oryn_url_media_metadata(raw)
                })
            }?;
            let retained = metadata["retainedBytes"]
                .as_u64()
                .and_then(|x| usize::try_from(x).ok())
                .unwrap_or(bytes.len());
            if retained > 128 * 1024 * 1024
                || self
                    .assets
                    .values()
                    .map(|asset| asset.bytes)
                    .sum::<usize>()
                    .saturating_add(retained)
                    > 128 * 1024 * 1024
            {
                return Err("QuotaExceededError: decoded media asset inventory".into());
            }
            asset.duration = metadata["duration"]
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.)
                .ok_or("NotReadableError: invalid native media duration")?;
            if metadata["audio"].as_bool() == Some(true) {
                let rate = metadata["sampleRate"]
                    .as_f64()
                    .filter(|value| {
                        value.is_finite()
                            && value.fract() == 0.
                            && (8000. ..=192000.).contains(value)
                    })
                    .ok_or("NotReadableError: invalid native audio format")?;
                let channels = metadata["channels"]
                    .as_u64()
                    .filter(|value| (1..=8).contains(value))
                    .ok_or("NotReadableError: invalid native audio channels")?;
                asset.audio_format = Some((rate as u32, channels as u32));
            }
            asset.bytes = retained;
            metadata.as_object_mut().map(|m| m.remove("retainedBytes"));
            let id = super::id()?;
            metadata["id"] = Value::String(id.clone());
            self.assets.insert(id, asset);
            Ok(metadata)
        }
    }
    pub(super) fn sample(
        &mut self,
        owner: &AgentOwner,
        id: &str,
        time: f64,
        paused: bool,
        volume: f32,
        seek: bool,
    ) -> Result<Value, String> {
        self.sample_with_rate(owner, id, time, paused, volume, seek, 1., true)
    }
    pub(super) fn sample_with_rate(
        &mut self,
        owner: &AgentOwner,
        id: &str,
        time: f64,
        paused: bool,
        volume: f32,
        seek: bool,
        playback_rate: f64,
        preserves_pitch: bool,
    ) -> Result<Value, String> {
        self.sample_with_settings(
            owner,
            id,
            time,
            paused,
            volume,
            seek,
            playback_rate,
            preserves_pitch,
            false,
        )
    }
    pub(super) fn sample_with_settings(
        &mut self,
        owner: &AgentOwner,
        id: &str,
        time: f64,
        paused: bool,
        volume: f32,
        seek: bool,
        playback_rate: f64,
        preserves_pitch: bool,
        looping: bool,
    ) -> Result<Value, String> {
        if !time.is_finite()
            || time < 0.
            || !volume.is_finite()
            || !(0.0..=1.).contains(&volume)
            || !playback_rate.is_finite()
        {
            return Err("TypeError: invalid media sample position".into());
        }
        let asset = self
            .assets
            .get_mut(id)
            .filter(|asset| asset.owner == *owner)
            .ok_or("InvalidStateError: foreign or retired media asset")?;
        let changed = asset.playback_rate != playback_rate
            || asset.preserves_pitch != preserves_pitch
            || asset.looping != looping;
        let reset = asset.processing_reset
            || paused != asset.paused
            || volume != asset.volume
            || seek
            || changed;
        let magnitude = playback_rate.abs();
        let audible = !paused
            && if preserves_pitch {
                (1. / 32. ..=32.).contains(&magnitude)
            } else {
                (0.25..=4.).contains(&magnitude)
            };
        // Admission precedes epoch/settings mutation and sink retirement. A
        // blocked plan is reused exactly, regardless of later polling time.
        let prepared = if audible && (reset || asset.pending_audio.is_none()) {
            let format = asset
                .audio
                .as_ref()
                .map(|audio| (audio.duration(), audio.rate, audio.channels))
                .or_else(|| {
                    asset
                        .audio_format
                        .map(|(rate, channels)| (asset.duration, rate, channels))
                });
            if let Some((duration, rate, channels)) = format {
                let cursor = if reset { time } else { asset.last_audio }.clamp(0., duration);
                let wall = ((time - cursor) / playback_rate + 0.05).max(0.);
                Some(
                    super::audio_windows::plan(
                        cursor,
                        duration,
                        playback_rate,
                        wall,
                        rate,
                        channels,
                        looping,
                    )
                    .map_err(|_| "QuotaExceededError: decoded media window plan")?,
                )
            } else {
                None
            }
        } else {
            None
        };
        // Reset on transitions, not every paused poll. A gain/mute change must
        // also discard already queued output carrying the previous gain.
        if reset {
            asset.processing_epoch = asset
                .processing_epoch
                .checked_add(1)
                .ok_or("QuotaExceededError: audio processing epoch")?;
            asset.sink = None;
            asset.last_audio = time;
            asset.processing_reset = false;
            asset.pending_audio = None;
        }
        asset.playback_rate = playback_rate;
        asset.preserves_pitch = preserves_pitch;
        asset.looping = looping;
        asset.paused = paused;
        asset.volume = volume;
        // Audio-unit capabilities do not restrict the actual HTML clock. Zero
        // and extreme rates render video at the requested position but silence
        // audio rather than clamping rate or inventing pitched output.
        if let Some(plan) = prepared {
            asset.pending_audio = Some(PendingAudio {
                plan,
                index: 0,
                bytes: None,
            });
        }
        if let Some(audio) = &asset.audio {
            if audible && let Some(pending) = asset.pending_audio.as_mut() {
                while let Some(segment) = pending.plan.segments().get(pending.index).copied() {
                    if volume > 0. {
                        if pending.bytes.is_none() {
                            let first = ((segment.start * audio.rate as f64).floor() as usize)
                                .saturating_mul(audio.channels as usize)
                                .min(audio.samples.len());
                            let last = ((segment.end * audio.rate as f64).floor() as usize)
                                .saturating_mul(audio.channels as usize)
                                .min(audio.samples.len());
                            let count = (last - first)
                                .checked_mul(4)
                                .filter(|count| {
                                    *count <= segment.maximum_bytes && *count <= 1024 * 1024
                                })
                                .ok_or("QuotaExceededError: decoded media window")?;
                            let mut bytes = Vec::new();
                            bytes
                                .try_reserve_exact(count)
                                .map_err(|_| "QuotaExceededError: decoded media window")?;
                            for sample in &audio.samples[first..last] {
                                bytes.extend_from_slice(&sample.to_le_bytes());
                            }
                            pending.bytes = Some(bytes);
                        }
                        let bytes = pending.bytes.as_ref().unwrap();
                        if !bytes.is_empty() {
                            if asset.sink.is_none() {
                                asset.sink = Some(system::AudioSink::open_with_rate(
                                    audio.rate as f64,
                                    audio.channels,
                                    playback_rate,
                                    preserves_pitch,
                                )?);
                            }
                            if !asset.sink.as_mut().unwrap().write_directional(
                                bytes,
                                volume,
                                segment.reverse,
                            )? {
                                break;
                            }
                        }
                    }
                    // Advance only after accepted physical output (or explicit
                    // mute). Failed submission retains the exact bytes/index.
                    asset.last_audio = pending
                        .acknowledge(true)
                        .ok_or("InvalidStateError: audio segment custody")?;
                }
                if pending.index == pending.plan.segments().len() {
                    asset.pending_audio = None;
                }
            }
            return Ok(
                serde_json::json!({"timestamp":time,"ended":time>=audio.duration(),
                "audioPending":asset.pending_audio.is_some()}),
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = asset;
            Err("NotSupportedError: URL media decoder unavailable".into())
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(id) = &asset.pip {
                super::picture_in_picture::visible(id, true);
            }
            // Video is sampled once, with an empty audio interval. Directional
            // audio reads use the dedicated native APIs and cannot alter GOP state.
            let mut sample = unsafe {
                result(if asset.webm {
                    oryn_webm_sample(asset.raw, time, time, time)
                } else {
                    oryn_url_media_sample(asset.raw, time, time, time)
                })
            }?;
            if audible && let Some(pending) = asset.pending_audio.as_mut() {
                let format = asset
                    .audio_format
                    .ok_or("NotReadableError: missing audio format")?;
                while let Some(segment) = pending.plan.segments().get(pending.index).copied() {
                    if volume > 0. {
                        if pending.bytes.is_none() {
                            let row = unsafe {
                                result(if asset.webm {
                                    oryn_webm_audio(asset.raw, segment.start, segment.end)
                                } else {
                                    oryn_url_media_audio(asset.raw, segment.start, segment.end)
                                })
                            }?;
                            pending.bytes =
                                Some(decode_audio_window(&row, segment.maximum_bytes, format)?);
                        }
                        let bytes = pending.bytes.as_ref().unwrap();
                        if !bytes.is_empty() {
                            if asset.sink.is_none() {
                                asset.sink = Some(system::AudioSink::open_with_rate(
                                    format.0 as f64,
                                    format.1,
                                    playback_rate,
                                    preserves_pitch,
                                )?);
                            }
                            if !asset.sink.as_mut().unwrap().write_directional(
                                bytes,
                                volume,
                                segment.reverse,
                            )? {
                                break;
                            }
                        }
                    }
                    asset.last_audio = pending
                        .acknowledge(true)
                        .ok_or("InvalidStateError: audio segment custody")?;
                }
                if pending.index == pending.plan.segments().len() {
                    asset.pending_audio = None;
                }
            }
            if let Some(sample) = sample.as_object_mut() {
                sample.remove("audio");
                sample.insert(
                    "audioPending".into(),
                    Value::Bool(asset.pending_audio.is_some()),
                );
            }
            if let Some(id) = &asset.pip {
                super::picture_in_picture::publish(id, &sample)?;
            }
            Ok(sample)
        }
    }
    // Internal parent helper only. A public transport still needs native graph
    // consumer registration and origin-clean media source validation.
    pub(super) fn processed_audio(
        &mut self,
        owner: &AgentOwner,
        id: &str,
        after_epoch: u64,
        after_sequence: u64,
    ) -> Result<Value, String> {
        let asset = self
            .assets
            .get_mut(id)
            .filter(|asset| asset.owner == *owner)
            .ok_or("InvalidStateError: foreign or retired media asset")?;
        if after_epoch > asset.processing_epoch {
            return Err("InvalidStateError: future audio processing epoch".into());
        }
        let after = if after_epoch == asset.processing_epoch {
            after_sequence
        } else {
            0
        };
        let mut sample = if let Some(sink) = asset.sink.as_mut() {
            sink.processed_audio(after)?
        } else {
            serde_json::json!({"pending":true,"sequence":0,"data":""})
        };
        sample["processingEpoch"] = Value::from(asset.processing_epoch);
        Ok(sample)
    }
    pub(super) fn close_picture_in_picture(&mut self, owner: &AgentOwner) {
        for asset in self
            .assets
            .values_mut()
            .filter(|asset| asset.owner == *owner)
        {
            if let Some(id) = asset.pip.take() {
                super::picture_in_picture::close(&id);
            }
        }
    }
    pub(super) fn has_picture_in_picture(&self, owner: &AgentOwner) -> bool {
        self.assets.values().any(|asset| {
            asset.owner == *owner
                && asset
                    .pip
                    .as_ref()
                    .is_some_and(|id| super::picture_in_picture::state(id).is_some())
        })
    }
    pub(super) fn picture_in_picture(
        &mut self,
        owner: &AgentOwner,
        id: &str,
        enabled: bool,
    ) -> Result<Option<(u32, u32)>, String> {
        let asset = self
            .assets
            .get_mut(id)
            .filter(|asset| asset.owner == *owner)
            .ok_or("InvalidStateError: foreign media asset")?;
        if !enabled {
            if let Some(id) = asset.pip.take() {
                super::picture_in_picture::close(&id);
            }
            return Ok(None);
        }
        if let Some(id) = &asset.pip
            && let Some(size) = super::picture_in_picture::state(id)
        {
            return Ok(Some(size));
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err("NotSupportedError: video PiP decoder unavailable".into())
        }
        #[cfg(target_os = "macos")]
        {
            if asset.raw.is_null() {
                return Err("InvalidStateError: no video track".into());
            }
            let metadata = unsafe {
                result(if asset.webm {
                    oryn_webm_metadata(asset.raw)
                } else {
                    oryn_url_media_metadata(asset.raw)
                })
            }?;
            let width = metadata["width"].as_u64().unwrap_or(0) as u32;
            let height = metadata["height"].as_u64().unwrap_or(0) as u32;
            let key = super::id()?;
            super::picture_in_picture::open(&key, width, height)?;
            for old in self
                .assets
                .values_mut()
                .filter(|asset| asset.owner == *owner)
            {
                if let Some(old) = old.pip.take() {
                    super::picture_in_picture::close(&old);
                }
            }
            self.assets.get_mut(id).unwrap().pip = Some(key.clone());
            Ok(super::picture_in_picture::state(&key))
        }
    }
    pub(super) fn picture_in_picture_state(
        &self,
        owner: &AgentOwner,
        id: &str,
    ) -> Result<Option<(u32, u32)>, String> {
        let asset = self
            .assets
            .get(id)
            .filter(|asset| asset.owner == *owner)
            .ok_or("InvalidStateError: foreign media asset")?;
        Ok(asset.pip.as_ref().and_then(|id| {
            super::picture_in_picture::visible(id, true);
            super::picture_in_picture::state(id)
        }))
    }
    pub(super) fn release(&mut self, owner: &AgentOwner, id: &str) -> Result<(), String> {
        if self
            .assets
            .get(id)
            .is_some_and(|asset| asset.owner != *owner)
        {
            return Err("InvalidStateError: foreign media asset".into());
        }
        if let Some(asset) = self.assets.remove(id)
            && let Some(id) = &asset.pip
        {
            super::picture_in_picture::close(id);
        }
        Ok(())
    }
    pub(super) fn suspend_owner(&mut self, owner: &AgentOwner) {
        for asset in self
            .assets
            .values_mut()
            .filter(|asset| asset.owner == *owner)
        {
            asset.sink = None;
            // A resumed sink restarts sequence numbering. Advance its epoch on
            // next admission even if rate/gain/paused settings stayed identical.
            // Defer the fallible increment; exhaustion must prohibit reopening.
            asset.processing_reset = true;
            asset.pending_audio = None;
            if let Some(id) = &asset.pip {
                super::picture_in_picture::visible(id, false);
            }
        }
    }
    pub(super) fn retire_owner(&mut self, owner: &AgentOwner) {
        self.assets.retain(|_, asset| asset.owner != *owner)
    }
    pub(super) fn retire_page(&mut self, page: oryn_common::v3::PageId) {
        self.assets.retain(
            |_, asset| !matches!(&asset.owner,AgentOwner::Window{page:owned,..}if *owned==page),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_audio_transfer_checks_format_and_size_before_allocation() {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode([1u8; 16]);
        let row = serde_json::json!({"audio":encoded,"sampleRate":48000,"channels":2});
        assert_eq!(
            decode_audio_window(&row, 16, (48000, 2)).unwrap(),
            vec![1; 16]
        );
        assert!(decode_audio_window(&row, 15, (48000, 2)).is_err());
        assert!(decode_audio_window(&row, 16, (48000, 1)).is_err());
        assert!(decode_audio_window(&row, 16, (0, 2)).is_err());
        assert!(
            decode_audio_window(
                &serde_json::json!({"audio":"!!!!","sampleRate":48000,"channels":1}),
                4,
                (48000, 1)
            )
            .is_err()
        );
        assert!(decode_audio_window(&serde_json::json!({"audio":null}), 16, (48000, 2)).is_err());
        assert!(
            decode_audio_window(&serde_json::json!({}), 0, (48000, 2))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn rejected_audio_ack_retains_exact_bytes_index_and_direction() {
        for rate in [32., -32.] {
            let position = if rate < 0. { 2. } else { 0. };
            let plan = super::super::audio_windows::plan(position, 2., rate, 0.05, 48000, 2, false)
                .unwrap();
            let segment = plan.segments()[0];
            let mut pending = PendingAudio {
                plan,
                index: 0,
                bytes: Some(vec![1, 2, 3, 4]),
            };
            let address = pending.bytes.as_ref().unwrap().as_ptr();
            assert_eq!(pending.acknowledge(false), None);
            assert_eq!(pending.index, 0);
            assert_eq!(pending.bytes.as_ref().unwrap().as_ptr(), address);
            assert_eq!(pending.bytes.as_deref(), Some(&[1, 2, 3, 4][..]));
            assert_eq!(
                pending.acknowledge(true),
                Some(if rate < 0. {
                    segment.start
                } else {
                    segment.end
                })
            );
            assert_eq!(pending.index, 1);
            assert!(pending.bytes.is_none());
        }
    }
    #[test]
    fn audio_processing_epoch_tracks_transitions_and_rejects_exhaustion() {
        let owner = test_owner(1);
        let foreign = test_owner(2);
        let mut registry = Registry::default();
        registry.assets.insert(
            "pcm".into(),
            Asset {
                duration: 0.004,
                audio_format: Some((8000, 1)),
                audio: Some(super::super::decoded_audio::Audio {
                    rate: 8000,
                    channels: 1,
                    samples: vec![0.; 32],
                }),
                owner: owner.clone(),
                bytes: 128,
                #[cfg(target_os = "macos")]
                raw: std::ptr::null_mut(),
                webm: false,
                sink: None,
                last_audio: 0.,
                playback_rate: 1.,
                preserves_pitch: true,
                looping: false,
                paused: true,
                volume: 1.,
                processing_epoch: 1,
                processing_reset: false,
                pending_audio: None,
                pip: None,
            },
        );
        for (paused, volume, expected) in [
            (true, 1., 1),
            (true, 1., 1),
            (true, 0., 2),
            (true, 0., 2),
            (false, 0., 3),
            (false, 0., 3),
            (true, 0., 4),
        ] {
            registry
                .sample_with_rate(&owner, "pcm", 0., paused, volume, false, 1., true)
                .unwrap();
            assert_eq!(registry.assets["pcm"].processing_epoch, expected);
            assert!(
                registry.assets["pcm"].sink.is_none(),
                "silent fixture must not open hardware"
            );
        }
        assert!(
            registry
                .sample_with_rate(&foreign, "pcm", 0., false, 0., false, 1., true)
                .is_err()
        );
        assert_eq!(registry.assets["pcm"].processing_epoch, 4);
        registry.suspend_owner(&owner);
        assert!(registry.assets["pcm"].processing_reset);
        registry
            .sample_with_rate(&owner, "pcm", 0., true, 0., false, 1., true)
            .unwrap();
        assert_eq!(registry.assets["pcm"].processing_epoch, 5);
        assert!(!registry.assets["pcm"].processing_reset);
        registry.assets.get_mut("pcm").unwrap().processing_epoch = u64::MAX;
        registry.suspend_owner(&owner);
        assert!(
            registry
                .sample_with_rate(&owner, "pcm", 0., true, 0., false, 1., true)
                .is_err()
        );
        let asset = &registry.assets["pcm"];
        assert_eq!(asset.processing_epoch, u64::MAX);
        assert!(asset.processing_reset);
        assert!(asset.paused);
        assert_eq!(asset.volume, 0.);
        assert_eq!(asset.playback_rate, 1.);
        {
            let asset = registry.assets.get_mut("pcm").unwrap();
            asset.audio = Some(super::super::decoded_audio::Audio {
                rate: 192000,
                channels: 8,
                samples: vec![0.; 192000 * 8 * 2],
            });
            asset.processing_epoch = 7;
            asset.processing_reset = false;
        }
        // The complete source plan exceeds eight admitted buffers. Deny it
        // before altering gain/rate/paused history or opening physical audio.
        assert!(
            registry
                .sample_with_rate(&owner, "pcm", 0., false, 0., false, 32., true)
                .unwrap_err()
                .contains("QuotaExceededError")
        );
        let asset = &registry.assets["pcm"];
        assert_eq!(asset.processing_epoch, 7);
        assert_eq!(asset.playback_rate, 1.);
        assert!(asset.paused);
        assert!(asset.sink.is_none());
        assert!(asset.pending_audio.is_none());
        registry.assets.get_mut("pcm").unwrap().audio = Some(super::super::decoded_audio::Audio {
            rate: 8000,
            channels: 1,
            samples: vec![0.; 800],
        });
        registry
            .sample_with_settings(&owner, "pcm", 0.09, false, 0., false, 1., true, true)
            .unwrap();
        let asset = &registry.assets["pcm"];
        assert_eq!(asset.processing_epoch, 8);
        assert!(asset.looping);
        assert!((asset.last_audio - 0.04).abs() < 1e-12);
        assert!(asset.pending_audio.is_none());
        assert!(asset.sink.is_none(), "muted loop must not open hardware");
        registry
            .sample_with_settings(&owner, "pcm", 0.01, false, 0., false, -1., true, true)
            .unwrap();
        let asset = &registry.assets["pcm"];
        assert_eq!(asset.processing_epoch, 9);
        assert!((asset.last_audio - 0.06).abs() < 1e-12);
        assert!(asset.pending_audio.is_none());
    }
    #[test]
    fn foreign_asset_and_invalid_positions_are_rejected_before_decode() {
        let mut registry = Registry::default();
        let owner = test_owner(1);
        assert!(
            registry
                .sample(&owner, "missing", 0., true, 1., false)
                .is_err()
        );
        assert!(
            registry
                .sample(&owner, "missing", f64::NAN, false, 1., false)
                .is_err()
        );
        assert!(registry.open(&owner, &[]).is_err());
        assert!(registry.release(&owner, "already-released").is_ok());
    }
}

pub(super) fn can_play(mime: &str) -> bool {
    if mime.len() > 1024 || mime.contains('\0') {
        return false;
    }
    let kind = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if kind.contains("mpegurl") {
        return false;
    }
    if !mime.to_ascii_lowercase().contains("codecs=")
        && matches!(
            kind.as_str(),
            "audio/ogg"
                | "application/ogg"
                | "audio/flac"
                | "audio/mpeg"
                | "audio/mp3"
                | "audio/wav"
                | "audio/x-wav"
        )
    {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn oryn_url_media_can_play(mime: *const c_char) -> i32;
            fn oryn_webm_can_play(mime: *const c_char) -> i32;
        }
        let Ok(mime) = std::ffi::CString::new(mime) else {
            return false;
        };
        unsafe {
            if matches!(kind.as_str(), "video/webm" | "audio/webm") {
                oryn_webm_can_play(mime.as_ptr()) != 0
            } else {
                oryn_url_media_can_play(mime.as_ptr()) != 0
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(all(test, target_os = "macos"))]
mod decoder_tests {
    use super::*;
    #[test]
    fn native_opus_packet_boundaries_preserve_stream_and_tail_discard() {
        assert_eq!(unsafe { oryn_webm_opus_unit_checks() }, 1);
    }
    #[test]
    fn audio_only_native_wav_window_uses_admitted_range_and_format() {
        struct Native(*mut c_void);
        impl Drop for Native {
            fn drop(&mut self) {
                unsafe { oryn_url_media_close(self.0) }
            }
        }
        let bytes = wav();
        let native = Native(unsafe { oryn_url_media_open(bytes.as_ptr(), bytes.len()) });
        assert!(!native.0.is_null());
        let metadata = unsafe { result(oryn_url_media_metadata(native.0)) }.unwrap();
        assert_eq!(metadata["audio"].as_bool(), Some(true));
        assert_eq!(metadata["sampleRate"].as_f64(), Some(8000.));
        assert_eq!(metadata["channels"].as_u64(), Some(1));
        for (start, end) in [(0., 0.05), (0.05, 0.1)] {
            let row = unsafe { result(oryn_url_media_audio(native.0, start, end)) }.unwrap();
            assert!(
                row.get("data").is_none(),
                "audio-only API must not publish video"
            );
            let pcm = decode_audio_window(&row, 1608, (8000, 1)).unwrap();
            assert_eq!(pcm.len(), 1600);
            assert!(pcm.iter().all(|byte| *byte == 0));
        }
        assert!(unsafe { oryn_url_media_audio(native.0, 0., 0.31) }.is_null());
        assert!(unsafe { oryn_url_media_audio(native.0, -1., 0.) }.is_null());
        let frame = unsafe { result(oryn_url_media_sample(native.0, 0., 0., 0.)) }.unwrap();
        assert_eq!(frame["ended"].as_bool(), Some(false));
    }
    #[test]
    fn native_container_hint_does_not_admit_invalid_media() {
        for bytes in [
            b"RIFF\0\0\0\0WAVEinvalid".as_slice(),
            b"FORM\0\0\0\0AIFFinvalid".as_slice(),
            b"\0\0\0\x0cftypinvalid".as_slice(),
            b"caff\0\0\0\0invalid".as_slice(),
        ] {
            let native = unsafe { oryn_url_media_open(bytes.as_ptr(), bytes.len()) };
            if !native.is_null() {
                unsafe { oryn_url_media_close(native) };
                panic!("a container hint must not bypass decoder validation");
            }
        }
    }
    fn wav() -> Vec<u8> {
        let samples = 800u32;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(36 + samples * 2).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(samples * 2).to_le_bytes());
        bytes.resize(44 + samples as usize * 2, 0);
        bytes
    }
    #[test]
    fn actual_decoder_reads_admitted_pcm_and_enforces_asset_owner_retirement() {
        let owner = test_owner(1);
        let foreign = test_owner(2);
        let mut registry = Registry::default();
        let metadata = registry.open(&owner, &wav()).unwrap();
        assert!((metadata["duration"].as_f64().unwrap() - 0.1).abs() < 0.001);
        let id = metadata["id"].as_str().unwrap();
        assert!(registry.sample(&foreign, id, 0., false, 0., false).is_err());
        assert!(registry.sample(&owner, id, 0., false, 0., false).is_ok());
        assert!(registry.release(&foreign, id).is_err());
        registry.retire_owner(&owner);
        assert!(registry.sample(&owner, id, 0., true, 0., false).is_err());
        assert!(registry.open(&owner, b"not media").is_err());
    }
    #[test]
    fn webm_capability_does_not_advertise_unimplemented_codecs() {
        #[cfg(target_os = "macos")]
        assert!(can_play("video/webm; codecs=vp8"));
        assert!(!can_play("audio/webm; codecs=vorbis"));
        assert!(!can_play("video/webm; codecs=vp09.02.10.10"));
        assert!(!can_play("video/webm; codecs=\"vp9, unknown\""));
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn native_software_decoder_buffer_and_color_contracts() {
        assert_eq!(unsafe { oryn_webm_decoder_unit_checks() }, 1);
    }
    // External conformance assets are deliberately not copied into the source
    // tree. The qualification runner supplies the exact pinned checkout.
    #[test]
    #[ignore = "requires ORYN_WPT_ROOT pinned conformance assets and native media decoders"]
    fn pinned_vp9_opus_decodes_pixels_pcm_seek_and_pip_with_owner_cleanup() {
        println!(
            "\nORYN_MEDIA_BUILD {}",
            serde_json::json!({"media_source_sha256":env!("ORYN_NATIVE_MEDIA_SOURCE_SHA256"),"runtime":crate::runtime_build_info(),"compiled_protocols":{"worker_protocol_version":crate::worker::WORKER_PROTOCOL_VERSION,"context_service_protocol_version":crate::context_service::CONTEXT_SERVICE_PROTOCOL_VERSION,"agent_ipc_protocol_version":crate::agent_ipc::VERSION}})
        );
        for mode in [0, 1, 2, 4] {
            pinned_decoder_assertions(mode);
        }
    }
    fn pinned_decoder_assertions(mode: u32) {
        println!("ORYN_MEDIA_DECODER_MODE {mode}");
        use base64::Engine;
        let root = std::env::var_os("ORYN_WPT_ROOT").expect("ORYN_WPT_ROOT is required");
        let bytes =
            std::fs::read(std::path::Path::new(&root).join("media/movie_300.webm")).unwrap();
        let movie = super::super::webm::parse(&bytes).unwrap();
        assert!(movie.tracks.iter().any(|t| t.codec == "V_VP9"));
        assert!(movie.tracks.iter().any(|t| t.codec == "A_OPUS"));
        assert!(
            can_play("audio/webm; codecs=opus"),
            "native Opus converter unavailable"
        );
        assert!(
            can_play("video/webm; codecs=vp9"),
            "native/software VP9 decoder initialization unavailable"
        );
        assert!(can_play("video/webm; codecs=\"vp9, opus\""));
        let owner = test_owner(1);
        let foreign = test_owner(2);
        let mut registry = Registry::default();
        let metadata = registry.open_mode(&owner, &bytes, mode).unwrap();
        if mode == 1 || mode == 2 {
            assert_eq!(metadata["decoder"], "libvpx");
        }
        if mode == 2 {
            assert_eq!(metadata["injectedFailure"], true);
            assert_ne!(metadata["nativeStatus"], 0);
        }
        let id = metadata["id"].as_str().unwrap();
        assert!(metadata["width"].as_u64().unwrap() > 0);
        assert_eq!(metadata["audio"], true);
        let first = registry.sample(&owner, id, 0., true, 0., true).unwrap();
        let pixels = base64::engine::general_purpose::STANDARD
            .decode(first["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            pixels.len(),
            first["width"].as_u64().unwrap() as usize
                * first["height"].as_u64().unwrap() as usize
                * 4
        );
        let at = metadata["duration"].as_f64().unwrap().min(1.) / 2.;
        let forward = registry.sample(&owner, id, at, true, 0., true).unwrap();
        let backward = registry.sample(&owner, id, 0., true, 0., true).unwrap();
        assert_eq!(backward["data"], first["data"]);
        assert!(forward["data"].is_string());
        if mode == 4 {
            let detail =
                unsafe { result(oryn_webm_metadata(registry.assets.get(id).unwrap().raw)) }
                    .unwrap();
            assert_eq!(detail["packetFailureInjected"], true);
            assert_eq!(detail["decoder"], "libvpx");
            assert_ne!(detail["nativeStatus"], 0);
        }
        let asset = registry.assets.get(id).unwrap();
        assert!(asset.webm);
        assert_eq!(unsafe { oryn_webm_unindexed_bounds_check(asset.raw) }, 1);
        let decoded = unsafe { result(oryn_webm_sample(asset.raw, 0., 0., 0.05)) }.unwrap();
        let pcm = base64::engine::general_purpose::STANDARD
            .decode(decoded["audio"].as_str().unwrap())
            .unwrap();
        assert!(!pcm.is_empty());
        assert!(
            pcm.as_chunks::<4>()
                .0
                .iter()
                .all(|x| f32::from_le_bytes(*x).is_finite())
        );
        assert!(registry.sample(&foreign, id, 0., true, 0., false).is_err());
        assert!(
            registry
                .picture_in_picture(&owner, id, true)
                .unwrap()
                .is_some()
        );
        registry.sample(&owner, id, at, true, 0., false).unwrap();
        registry.suspend_owner(&owner);
        registry.sample(&owner, id, at, true, 0., true).unwrap();
        registry.retire_owner(&owner);
        assert!(registry.sample(&owner, id, 0., true, 0., true).is_err());
        assert!(registry.open(&owner, &bytes[..4096]).is_err());
        let constrained = registry.open_mode(&owner, &bytes, 3).unwrap();
        let constrained_id = constrained["id"].as_str().unwrap();
        assert!(
            registry
                .sample(&owner, constrained_id, 0., true, 0., true)
                .is_err()
        );
        registry.retire_owner(&owner);
        assert!(registry.assets.is_empty());
        let recovered = registry.open_mode(&owner, &bytes, 1).unwrap();
        assert!(
            registry
                .sample(
                    &owner,
                    recovered["id"].as_str().unwrap(),
                    0.,
                    true,
                    0.,
                    true
                )
                .is_ok()
        );
        registry.retire_owner(&owner);
        let vp8 = std::fs::read(std::path::Path::new(&root).join("media/rgb100.webm")).unwrap();
        let parsed = super::super::webm::parse(&vp8).unwrap();
        let track = parsed
            .tracks
            .iter()
            .find(|track| track.codec == "V_VP8")
            .unwrap();
        assert!(can_play("video/webm;codecs=vp8"));
        let metadata = registry.open(&owner, &vp8).unwrap();
        let id = metadata["id"].as_str().unwrap();
        let first = registry.sample(&owner, id, 0., true, 0., false).unwrap();
        let pixels = base64::engine::general_purpose::STANDARD
            .decode(first["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            pixels.len(),
            track.width as usize * track.height as usize * 4
        );
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 255)
        );
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[0] != pixel[1] || pixel[1] != pixel[2])
        );
        let later = registry
            .sample(
                &owner,
                id,
                metadata["duration"].as_f64().unwrap() / 2.,
                true,
                0.,
                false,
            )
            .unwrap();
        assert!(later["data"].is_string());
        assert_eq!(
            registry.sample(&owner, id, 0., true, 0., false).unwrap()["data"],
            first["data"]
        );
        registry.retire_owner(&owner);
        let mut invalid = vp8.clone();
        let offset = track.packets[0].offset;
        assert!(track.packets[0].length >= 10);
        invalid[offset + 6] = 255;
        invalid[offset + 7] = 63;
        let metadata = registry.open(&owner, &invalid).unwrap();
        let id = metadata["id"].as_str().unwrap();
        assert!(registry.sample(&owner, id, 0., true, 0., false).is_err());
        registry.retire_owner(&owner);
        // Direct Core media sniff callers require every container independently of
        // the HTTP MIME label; URL admission never passes that label to this decoder.
        for fixture in [
            "mp3-raw.mp3",
            "mp3-with-id3.mp3",
            "flac.flac",
            "ogg.ogg",
            "mp4.mp4",
            "wav.wav",
        ] {
            let bytes = std::fs::read(
                std::path::Path::new(&root)
                    .join("mimesniff/media/resources")
                    .join(fixture),
            )
            .unwrap();
            let metadata = registry
                .open(&owner, &bytes)
                .unwrap_or_else(|error| panic!("{fixture}: {error}"));
            let duration = metadata["duration"].as_f64().unwrap();
            assert!(duration.is_finite() && duration > 0., "{fixture} metadata");
            let id = metadata["id"].as_str().unwrap();
            assert!(
                registry.sample(&owner, id, 0., true, 0., false).is_ok(),
                "{fixture} sample"
            );
            registry.retire_owner(&owner);
            assert!(registry.assets.is_empty());
        }
        let sniff =
            std::fs::read(std::path::Path::new(&root).join("mimesniff/media/resources/webm.webm"))
                .unwrap();
        let metadata = registry.open(&owner, &sniff).unwrap();
        assert_eq!(metadata["audio"], true);
        assert_eq!(metadata["width"], 0);
        registry.retire_owner(&owner);
    }
}
