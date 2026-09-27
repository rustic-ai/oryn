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
    ) -> *mut c_void;
    #[cfg(test)]
    fn oryn_webm_decoder_unit_checks() -> i32;
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
    fn oryn_url_media_open(bytes: *const u8, length: usize) -> *mut c_void;
    fn oryn_url_media_metadata(asset: *mut c_void) -> *mut c_char;
    fn oryn_url_media_sample(
        asset: *mut c_void,
        time: f64,
        audio_start: f64,
        audio_until: f64,
    ) -> *mut c_char;
    fn oryn_url_media_close(asset: *mut c_void);
    fn oryn_capture_free(value: *mut c_char);
}
struct Asset {
    audio: Option<super::decoded_audio::Audio>,
    owner: AgentOwner,
    bytes: usize,
    #[cfg(target_os = "macos")]
    raw: *mut c_void,
    webm: bool,
    sink: Option<system::AudioSink>,
    last_audio: f64,
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
                    audio: Some(audio),
                    owner: owner.clone(),
                    bytes: retained,
                    #[cfg(target_os = "macos")]
                    raw: std::ptr::null_mut(),
                    webm: false,
                    sink: None,
                    last_audio: 0.,
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
                owner: owner.clone(),
                bytes: bytes.len(),
                raw,
                webm,
                sink: None,
                last_audio: 0.,
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
        if !time.is_finite() || time < 0. || !volume.is_finite() || !(0.0..=1.).contains(&volume) {
            return Err("TypeError: invalid media sample position".into());
        }
        let asset = self
            .assets
            .get_mut(id)
            .filter(|asset| asset.owner == *owner)
            .ok_or("InvalidStateError: foreign or retired media asset")?;
        if paused || seek {
            asset.sink = None;
            asset.last_audio = time;
        }
        if let Some(audio) = &asset.audio {
            let end = time + 0.05;
            let start = if seek || time > asset.last_audio + 0.25 || time + 0.25 < asset.last_audio
            {
                time
            } else {
                asset.last_audio
            };
            let scale = audio.rate as f64 * audio.channels as f64;
            let first = ((start * audio.rate as f64).floor() as usize)
                .saturating_mul(audio.channels as usize)
                .min(audio.samples.len());
            let last = ((end * audio.rate as f64).floor() as usize)
                .saturating_mul(audio.channels as usize)
                .min(audio.samples.len());
            if !paused && last > first {
                if volume > 0. {
                    if asset.sink.is_none() {
                        asset.sink =
                            Some(system::AudioSink::open(audio.rate as f64, audio.channels)?)
                    }
                    let bytes = audio.samples[first..last]
                        .iter()
                        .flat_map(|sample| sample.to_le_bytes())
                        .collect::<Vec<_>>();
                    if asset.sink.as_mut().unwrap().write(&bytes, volume)? {
                        asset.last_audio = last as f64 / scale;
                    }
                } else {
                    asset.last_audio = last as f64 / scale;
                }
            }
            return Ok(serde_json::json!({"timestamp":time,"ended":time>=audio.duration()}));
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
            let until = if paused { time } else { time + 0.05 };
            let start = if seek || time > asset.last_audio + 0.25 || time + 0.25 < asset.last_audio
            {
                time
            } else {
                asset.last_audio
            };
            let mut sample = unsafe {
                result(if asset.webm {
                    oryn_webm_sample(asset.raw, time, start, until.max(start))
                } else {
                    oryn_url_media_sample(asset.raw, time, start, until.max(start))
                })
            }
            .map_err(|error| {
                if asset.webm {
                    let details = unsafe { result(oryn_webm_metadata(asset.raw)) };
                    format!("{error}; decoder state: {details:?}")
                } else {
                    error
                }
            })?;
            if !paused && let Some(encoded) = sample["audio"].as_str() {
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|_| "NotReadableError: malformed audio sample")?;
                let rate = sample["sampleRate"]
                    .as_f64()
                    .ok_or("NotReadableError: audio rate")?;
                let channels = sample["channels"]
                    .as_u64()
                    .and_then(|x| u32::try_from(x).ok())
                    .ok_or("NotReadableError: audio channels")?;
                if bytes.len() > 1024 * 1024
                    || !rate.is_finite()
                    || !(8000.0..=192000.0).contains(&rate)
                    || channels == 0
                    || channels > 8
                    || bytes.len() % (channels as usize * 4) != 0
                {
                    return Err("NotReadableError: invalid decoded audio".into());
                }
                if volume > 0. {
                    if asset
                        .sink
                        .as_ref()
                        .is_none_or(|sink| sink.rate != rate || sink.channels != channels)
                    {
                        asset.sink = Some(system::AudioSink::open(rate, channels)?)
                    }
                    if asset.sink.as_mut().unwrap().write(&bytes, volume)? {
                        asset.last_audio = until;
                    }
                } else {
                    asset.last_audio = until;
                }
            }
            if let Some(sample) = sample.as_object_mut() {
                sample.remove("audio");
            }
            if let Some(id) = &asset.pip {
                super::picture_in_picture::publish(id, &sample)?;
            }
            Ok(sample)
        }
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
