//! Context-owned capture custody. Synthetic fixture devices allocate real native
//! test samples; ordinary contexts truthfully have no installed capture device.
mod decoded_audio;
pub(crate) mod picture_in_picture;
mod system;
mod url_asset;
mod webm;
use crate::agent_registry::AgentOwner;
use base64::Engine;
use oryn_js::media_capture::{DeviceConfiguration, Kind, Request, Response, Stream, Track};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};
#[derive(Debug)]
enum Source {
    Video {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        started: Instant,
        frame_rate: f64,
    },
    Audio {
        sample_rate: u32,
        channels: u32,
        pcm: Vec<f32>,
        started: Instant,
    },
    Physical {
        capture: Mutex<system::Capture>,
    },
}
impl Source {
    fn bytes(&self) -> usize {
        match self {
            Self::Video { rgba, .. } => rgba.len(),
            Self::Audio { pcm, .. } => pcm.len() * 4,
            Self::Physical { .. } => 16 * 1024 * 1024,
        }
    }
}
struct OwnedTrack {
    pip: Option<String>,
    owner: AgentOwner,
    track: Track,
    capabilities: serde_json::Value,
    source: Option<Arc<Source>>,
}
impl Drop for OwnedTrack {
    fn drop(&mut self) {
        if let Some(id) = &self.pip {
            picture_in_picture::close(id);
        }
    }
}
struct PendingCapture {
    owner: AgentOwner,
    constraints: serde_json::Value,
    devices: DeviceConfiguration,
    started: Instant,
}
#[derive(Default)]
pub(crate) struct Registry {
    urls: url_asset::Registry,
    pending: BTreeMap<String, PendingCapture>,
    tracks: BTreeMap<String, OwnedTrack>,
    device_salt: Option<[u8; 24]>,
    outputs: BTreeMap<(String, u64), system::AudioSink>,
}
fn capabilities(source: &Source, kind: Kind) -> Result<serde_json::Value, String> {
    Ok(match source {
        Source::Physical { capture } => capture
            .lock()
            .map_err(|_| "NotReadableError: capture lock")?
            .capabilities
            .clone(),
        _ => crate::media_constraints::capabilities(kind),
    })
}
fn id() -> Result<String, String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|_| "NotReadableError: capture identity unavailable")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn synthetic(kind: Kind, constraints: serde_json::Value) -> Result<(Track, Arc<Source>), String> {
    let settings = crate::media_constraints::select(kind, &constraints)?;
    let id = id()?;
    let source = match kind {
        Kind::Video => {
            let width = settings["width"]
                .as_u64()
                .ok_or("NotReadableError: invalid width")? as u32;
            let height = settings["height"]
                .as_u64()
                .ok_or("NotReadableError: invalid height")? as u32;
            let mut rgba = vec![0u8; width as usize * height as usize * 4];
            for (pixel, bytes) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                bytes.copy_from_slice(&[
                    ((pixel % width as usize) * 255 / width as usize) as u8,
                    ((pixel / width as usize) * 255 / height as usize) as u8,
                    128,
                    255,
                ]);
            }
            Source::Video {
                width,
                height,
                rgba,
                started: Instant::now(),
                frame_rate: settings["frameRate"].as_f64().unwrap_or(30.),
            }
        }
        Kind::Audio => {
            let sample_rate = settings["sampleRate"]
                .as_f64()
                .ok_or("NotReadableError: invalid sample rate")?
                as u32;
            let channels = settings["channelCount"]
                .as_f64()
                .ok_or("NotReadableError: invalid channels")? as u32;
            Source::Audio {
                sample_rate,
                channels,
                pcm: vec![0f32; (sample_rate / 100 * channels) as usize],
                started: Instant::now(),
            }
        }
    };
    Ok((
        Track {
            id,
            kind,
            label: format!("Oryn conformance {} device", kind.as_str()),
            enabled: true,
            ended: false,
            settings,
            constraints,
        },
        Arc::new(source),
    ))
}
impl Registry {
    pub(crate) fn has_picture_in_picture(&self, owner: &AgentOwner) -> bool {
        self.urls.has_picture_in_picture(owner)
            || self.tracks.values().any(|track| {
                track.owner == *owner
                    && track
                        .pip
                        .as_ref()
                        .is_some_and(|id| picture_in_picture::state(id).is_some())
            })
    }
    pub(crate) fn is_capture_track(&self, id: &str) -> bool {
        self.tracks.contains_key(id)
    }
    fn picture_in_picture_state(
        &self,
        owner: &AgentOwner,
        id: &str,
    ) -> Result<Option<(u32, u32)>, String> {
        if let Some(track) = self.tracks.get(id) {
            if track.owner != *owner {
                return Err("InvalidStateError: foreign PiP track".into());
            }
            return Ok(track.pip.as_ref().and_then(|id| {
                picture_in_picture::visible(id, true);
                picture_in_picture::state(id)
            }));
        }
        self.urls.picture_in_picture_state(owner, id)
    }
    fn picture_in_picture(
        &mut self,
        owner: &AgentOwner,
        key: &str,
        enabled: bool,
        devices: DeviceConfiguration,
    ) -> Result<Option<(u32, u32)>, String> {
        if !self.tracks.contains_key(key) {
            let result = self.urls.picture_in_picture(owner, key, enabled)?;
            if enabled {
                for track in self
                    .tracks
                    .values_mut()
                    .filter(|track| track.owner == *owner)
                {
                    if let Some(id) = track.pip.take() {
                        picture_in_picture::close(&id);
                    }
                }
            }
            return Ok(result);
        }
        let track = self
            .tracks
            .get_mut(key)
            .filter(|track| track.owner == *owner)
            .ok_or("InvalidStateError: foreign PiP track")?;
        if !enabled {
            if let Some(id) = track.pip.take() {
                picture_in_picture::close(&id);
            }
            return Ok(None);
        }
        if track.track.ended || track.track.kind != Kind::Video {
            return Err("InvalidStateError: no active video track".into());
        }
        if let Some(size) = track
            .pip
            .as_ref()
            .and_then(|id| picture_in_picture::state(id))
        {
            return Ok(Some(size));
        }
        let Response::Sample(sample) = self.request(
            owner,
            devices,
            Request::Sample {
                id: key.into(),
                after: 0,
            },
        )?
        else {
            return Err("NotReadableError: PiP capture sample".into());
        };
        let width = sample["width"].as_u64().unwrap_or(0) as u32;
        let height = sample["height"].as_u64().unwrap_or(0) as u32;
        let surface = id()?;
        picture_in_picture::open(&surface, width, height)?;
        if let Err(error) = picture_in_picture::publish(&surface, &sample) {
            picture_in_picture::close(&surface);
            return Err(error);
        }
        self.urls.close_picture_in_picture(owner);
        for track in self
            .tracks
            .values_mut()
            .filter(|track| track.owner == *owner)
        {
            if let Some(id) = track.pip.take() {
                picture_in_picture::close(&id);
            }
        }
        self.tracks.get_mut(key).unwrap().pip = Some(surface.clone());
        Ok(picture_in_picture::state(&surface))
    }
    pub(crate) fn capture_features(
        &self,
        owner: &AgentOwner,
        request: &Request,
    ) -> Result<Vec<&'static str>, String> {
        let constraints = match request {
            Request::Capture { constraints } => Some(constraints),
            Request::PollCapture { id } => Some(
                &self
                    .pending
                    .get(id)
                    .filter(|pending| pending.owner == *owner)
                    .ok_or("InvalidStateError: foreign or retired capture request")?
                    .constraints,
            ),
            _ => None,
        };
        Ok([("video", "camera"), ("audio", "microphone")]
            .into_iter()
            .filter(|(key, _)| {
                constraints
                    .is_none_or(|value| value.get(*key) != Some(&serde_json::Value::Bool(false)))
            })
            .map(|(_, name)| name)
            .collect())
    }

    fn acquire(
        &mut self,
        devices: DeviceConfiguration,
        kind: Kind,
        constraints: serde_json::Value,
    ) -> Result<(Track, Arc<Source>), String> {
        if devices != DeviceConfiguration::System {
            return synthetic(kind, constraints);
        }
        if self.device_salt.is_none() {
            let mut salt = [0u8; 24];
            getrandom::fill(&mut salt).map_err(|_| "NotReadableError: capture privacy salt")?;
            self.device_salt = Some(salt);
        }
        let (track, capture) = system::open(kind, constraints, self.device_salt.as_ref().unwrap())?;
        Ok((
            track,
            Arc::new(Source::Physical {
                capture: Mutex::new(capture),
            }),
        ))
    }
    pub(crate) fn request(
        &mut self,
        owner: &AgentOwner,
        devices: DeviceConfiguration,
        request: Request,
    ) -> Result<Response, String> {
        match request {
            Request::UrlCanPlay { mime } => Ok(Response::UrlSupport(url_asset::can_play(&mime))),
            Request::UrlPictureInPicture { id, enabled } => Ok(Response::UrlPictureInPicture(
                self.picture_in_picture(owner, &id, enabled, devices)?,
            )),
            Request::UrlPictureInPictureState { id } => Ok(Response::UrlPictureInPicture(
                self.picture_in_picture_state(owner, &id)?,
            )),
            Request::UrlOpen { bytes_base64 } => {
                if bytes_base64.len() > 22_369_624 {
                    return Err("QuotaExceededError: URL media transfer bytes".into());
                }
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(bytes_base64)
                    .map_err(|_| "TypeError: malformed media bytes")?;
                self.urls.open(owner, &bytes).map(Response::UrlAsset)
            }
            Request::UrlSample {
                id,
                time,
                paused,
                volume,
                seek,
            } => self
                .urls
                .sample(owner, &id, time, paused, volume, seek)
                .map(Response::UrlFrame),
            Request::UrlRelease { id } => {
                self.urls.release(owner, &id)?;
                Ok(Response::Stopped)
            }
            Request::CancelCapture { id } => {
                if self
                    .pending
                    .get(&id)
                    .is_some_and(|pending| pending.owner != *owner)
                {
                    return Err("InvalidStateError: foreign capture request".into());
                }
                self.pending.remove(&id);
                Ok(Response::Stopped)
            }
            Request::PollCapture { id } => {
                let pending = self
                    .pending
                    .get(&id)
                    .filter(|pending| pending.owner == *owner)
                    .ok_or("InvalidStateError: foreign or retired capture request")?;
                if pending.started.elapsed() > std::time::Duration::from_secs(120) {
                    self.pending.remove(&id);
                    return Err("AbortError: OS capture permission timed out".into());
                }
                let authorized = if pending.devices == DeviceConfiguration::System {
                    system::authorized(false, &pending.constraints)
                } else {
                    Ok(true)
                };
                match authorized {
                    Ok(false) => Ok(Response::Pending { id }),
                    Err(error) => {
                        self.pending.remove(&id);
                        Err(error)
                    }
                    Ok(true) => {
                        let pending = self.pending.remove(&id).unwrap();
                        self.request(
                            owner,
                            pending.devices,
                            Request::Capture {
                                constraints: pending.constraints,
                            },
                        )
                    }
                }
            }
            Request::Capture { constraints } => {
                if devices == DeviceConfiguration::None {
                    return Err("NotFoundError: no native capture device is installed".into());
                }
                if !constraints.is_object()
                    || serde_json::to_vec(&constraints)
                        .map_err(|_| "TypeError: invalid constraints")?
                        .len()
                        > 65536
                {
                    return Err("TypeError: capture constraints exceed native bounds".into());
                }
                if ["audio", "video"].iter().any(|name| {
                    constraints.get(*name).is_some_and(|value| {
                        !value.is_object() && value != &serde_json::Value::Bool(false)
                    })
                }) {
                    return Err("TypeError: capture track constraints must be dictionaries".into());
                }
                self.pending.retain(|_, pending| {
                    pending.started.elapsed() <= std::time::Duration::from_secs(120)
                });
                if devices == DeviceConfiguration::System
                    && !system::authorized(true, &constraints)?
                {
                    if self.pending.len() >= 64 {
                        return Err("QuotaExceededError: pending OS capture requests".into());
                    }
                    let id = id()?;
                    self.pending.insert(
                        id.clone(),
                        PendingCapture {
                            owner: owner.clone(),
                            constraints,
                            devices,
                            started: Instant::now(),
                        },
                    );
                    return Ok(Response::Pending { id });
                }
                let kinds = [("audio", Kind::Audio), ("video", Kind::Video)]
                    .into_iter()
                    .filter(|(name, _)| {
                        constraints.get(*name) != Some(&serde_json::Value::Bool(false))
                    })
                    .collect::<Vec<_>>();
                if kinds.is_empty() {
                    return Err("TypeError: capture requires a track".into());
                }
                if self.tracks.len() + kinds.len() > 1024 {
                    return Err("NotReadableError: native capture track limit".into());
                }
                // Allocate every requested source before publishing any track.
                let mut sources = Vec::new();
                for (name, kind) in kinds {
                    sources.push(
                        self.acquire(
                            devices,
                            kind,
                            constraints
                                .get(name)
                                .cloned()
                                .unwrap_or_else(|| serde_json::json!({})),
                        )?,
                    );
                }
                let mut unique = BTreeMap::new();
                for track in self.tracks.values() {
                    if let Some(source) = &track.source {
                        unique.entry(Arc::as_ptr(source)).or_insert(source.bytes());
                    }
                }
                if unique.values().sum::<usize>()
                    + sources
                        .iter()
                        .map(|(_, source)| source.bytes())
                        .sum::<usize>()
                    > 64 * 1024 * 1024
                {
                    return Err("NotReadableError: native capture sample budget".into());
                }
                let stream_id = id()?;
                let tracks = sources.iter().map(|(track, _)| track.clone()).collect();
                for (track, source) in sources {
                    self.tracks.insert(
                        track.id.clone(),
                        OwnedTrack {
                            pip: None,
                            owner: owner.clone(),
                            capabilities: capabilities(&source, track.kind)?,
                            track,
                            source: Some(source),
                        },
                    );
                }
                Ok(Response::Captured(Stream {
                    id: stream_id,
                    tracks,
                }))
            }
            Request::AudioRender {
                id,
                consumer,
                after,
                paused,
                volume,
            } => {
                let owned = self
                    .tracks
                    .get(&id)
                    .filter(|track| track.owner == *owner && track.track.kind == Kind::Audio)
                    .ok_or("InvalidStateError: foreign audio output track")?;
                let key = (id.clone(), consumer);
                if paused || owned.track.ended {
                    self.outputs.remove(&key);
                    return Ok(Response::Stopped);
                }
                if !volume.is_finite() || !(0.0..=1.).contains(&volume) {
                    return Err("IndexSizeError: invalid output volume".into());
                }
                let Response::Sample(mut sample) =
                    self.request(owner, devices, Request::Sample { id, after })?
                else {
                    return Err("InvalidStateError: sample unavailable".into());
                };
                if volume == 0. {
                    self.outputs.remove(&key);
                    if sample.get("data").is_some() {
                        sample.as_object_mut().unwrap().remove("data");
                        sample["rendered"] = serde_json::json!(true);
                    }
                    return Ok(Response::Sample(sample));
                }
                if let Some(data) = sample["data"].as_str() {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .map_err(|_| "NotReadableError: malformed PCM")?;
                    let rate = sample["sampleRate"]
                        .as_f64()
                        .ok_or("NotReadableError: missing sample rate")?;
                    let channels = sample["channels"]
                        .as_u64()
                        .and_then(|value| u32::try_from(value).ok())
                        .ok_or("NotReadableError: missing channel count")?;
                    if self
                        .outputs
                        .get(&key)
                        .is_some_and(|output| output.rate != rate || output.channels != channels)
                    {
                        self.outputs.remove(&key);
                    }
                    if !self.outputs.contains_key(&key) {
                        if self.outputs.len() >= 32 {
                            return Err("QuotaExceededError: native audio output limit".into());
                        }
                        self.outputs
                            .insert(key.clone(), system::AudioSink::open(rate, channels)?);
                    }
                    let rendered = self.outputs.get_mut(&key).unwrap().write(&bytes, volume)?;
                    sample.as_object_mut().unwrap().remove("data");
                    sample["rendered"] = serde_json::json!(rendered);
                    sample["backpressure"] = serde_json::json!(!rendered);
                }
                Ok(Response::Sample(sample))
            }
            Request::Sample { id, after } => {
                let track = self
                    .tracks
                    .get_mut(&id)
                    .filter(|track| track.owner == *owner)
                    .ok_or("InvalidStateError: foreign or retired capture track")?;
                let Some(source) = &track.source else {
                    return Ok(Response::Sample(serde_json::json!({"ended":true})));
                };
                let mut sample = match &**source {
                    Source::Physical { capture } => capture
                        .lock()
                        .map_err(|_| "NotReadableError: capture lock")?
                        .sample()?,
                    Source::Video {
                        width,
                        height,
                        rgba,
                        started,
                        frame_rate,
                    } => {
                        let time = started.elapsed().as_secs_f64();
                        serde_json::json!({"kind":"video","sequence":(time*frame_rate)as u64+1,"timestamp":time,"width":width,"height":height,"data":base64::engine::general_purpose::STANDARD.encode(rgba),"ended":false,"dropped":0})
                    }
                    Source::Audio {
                        sample_rate,
                        channels,
                        pcm,
                        started,
                    } => {
                        let time = started.elapsed().as_secs_f64();
                        let bytes = pcm
                            .iter()
                            .flat_map(|value| value.to_le_bytes())
                            .collect::<Vec<_>>();
                        serde_json::json!({"kind":"audio","sequence":(time*100.)as u64+1,"timestamp":time,"sampleRate":sample_rate,"channels":channels,"data":base64::engine::general_purpose::STANDARD.encode(bytes),"ended":false,"dropped":0})
                    }
                };
                if matches!(&**source, Source::Physical { .. }) {
                    validate_physical_sample(&sample, &track.track)?;
                }
                normalize_sample_loss(
                    &mut sample,
                    after,
                    matches!(&**source, Source::Physical { .. }),
                );
                if sample["ended"].as_bool() == Some(true) {
                    track.track.ended = true;
                    track.source = None;
                    if let Some(id) = track.pip.take() {
                        picture_in_picture::close(&id);
                    }
                }
                if sample["sequence"].as_u64().unwrap_or(0) <= after {
                    sample.as_object_mut().unwrap().remove("data");
                } else if !track.track.enabled
                    && let Some(data) = sample["data"].as_str()
                {
                    let mut bytes = base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .map_err(|_| "NotReadableError: malformed native sample")?;
                    bytes.fill(0);
                    if track.track.kind == Kind::Video {
                        for pixel in bytes.as_chunks_mut::<4>().0 {
                            pixel[3] = 255
                        }
                    }
                    sample["data"] =
                        serde_json::json!(base64::engine::general_purpose::STANDARD.encode(bytes));
                }
                if let Some(id) = &track.pip {
                    if track.track.ended {
                        picture_in_picture::close(id);
                    } else {
                        picture_in_picture::publish(id, &sample)?;
                    }
                }
                Ok(Response::Sample(sample))
            }
            Request::Capabilities { id } => {
                let track = self
                    .tracks
                    .get(&id)
                    .filter(|track| track.owner == *owner)
                    .ok_or("InvalidStateError: foreign or retired capture track")?;
                Ok(Response::Capabilities(track.capabilities.clone()))
            }
            Request::ApplyConstraints { id, constraints } => {
                let old = self
                    .tracks
                    .get(&id)
                    .filter(|track| track.owner == *owner)
                    .ok_or("InvalidStateError: foreign or retired capture track")?;
                if old.track.ended {
                    return Ok(Response::Track(old.track.clone()));
                }
                if serde_json::to_vec(&constraints)
                    .map_err(|_| "TypeError: invalid constraints")?
                    .len()
                    > 65536
                {
                    return Err("TypeError: capture constraints exceed native bounds".into());
                }
                let kind = old.track.kind;
                let enabled = old.track.enabled;
                let physical = old
                    .source
                    .as_ref()
                    .is_some_and(|source| matches!(&**source, Source::Physical { .. }));
                let (mut track, source) = self.acquire(
                    if physical {
                        DeviceConfiguration::System
                    } else {
                        DeviceConfiguration::SyntheticFixtureV1
                    },
                    kind,
                    constraints,
                )?;
                track.id = id.clone();
                track.enabled = enabled;
                let mut unique = BTreeMap::new();
                for (other_id, other) in &self.tracks {
                    if other_id != &id
                        && let Some(source) = &other.source
                    {
                        unique.entry(Arc::as_ptr(source)).or_insert(source.bytes());
                    }
                }
                if unique.values().sum::<usize>() + source.bytes() > 64 * 1024 * 1024 {
                    return Err("NotReadableError: native capture sample budget".into());
                }
                let capabilities = capabilities(&source, track.kind)?;
                let pip = self.tracks.get_mut(&id).and_then(|old| old.pip.take());
                self.tracks.insert(
                    id,
                    OwnedTrack {
                        pip,
                        owner: owner.clone(),
                        capabilities,
                        track: track.clone(),
                        source: Some(source),
                    },
                );
                Ok(Response::Track(track))
            }
            Request::Release { id } => {
                if self
                    .tracks
                    .get(&id)
                    .is_some_and(|track| track.owner != *owner)
                {
                    return Err("InvalidStateError: foreign capture track".into());
                }
                self.outputs.retain(|(track, _), _| track != &id);
                self.tracks.remove(&id);
                Ok(Response::Stopped)
            }
            Request::Clone { id: old } => {
                if self.tracks.len() >= 1024 {
                    return Err("NotReadableError: native capture track limit".into());
                }
                let old = self
                    .tracks
                    .get(&old)
                    .filter(|track| track.owner == *owner)
                    .ok_or("InvalidStateError: foreign or retired capture track")?;
                let mut track = old.track.clone();
                track.id = id()?;
                let source = old.source.clone();
                let capabilities = old.capabilities.clone();
                self.tracks.insert(
                    track.id.clone(),
                    OwnedTrack {
                        pip: None,
                        owner: owner.clone(),
                        capabilities,
                        track: track.clone(),
                        source,
                    },
                );
                Ok(Response::Track(track))
            }
            request => {
                let id = match &request {
                    Request::Inspect { id } | Request::Stop { id } | Request::Enable { id, .. } => {
                        id
                    }
                    _ => unreachable!(),
                };
                let track = self
                    .tracks
                    .get_mut(id)
                    .filter(|track| track.owner == *owner)
                    .ok_or("InvalidStateError: foreign or retired capture track")?;
                match request {
                    Request::Inspect { .. } => {
                        let ended = match track.source.as_deref() {
                            Some(Source::Physical { capture }) => !capture
                                .lock()
                                .map_err(|_| "NotReadableError: capture lock")?
                                .alive(),
                            _ => false,
                        };
                        if ended {
                            self.outputs.retain(|(id, _), _| id != &track.track.id);
                            track.track.ended = true;
                            track.source = None;
                            if let Some(id) = track.pip.take() {
                                picture_in_picture::close(&id);
                            }
                        }
                        Ok(Response::Track(track.track.clone()))
                    }
                    Request::Stop { .. } => {
                        self.outputs.retain(|(id, _), _| id != &track.track.id);
                        track.track.ended = true;
                        track.source = None;
                        if let Some(id) = track.pip.take() {
                            picture_in_picture::close(&id);
                        }
                        Ok(Response::Stopped)
                    }
                    Request::Enable { enabled, .. } => {
                        track.track.enabled = enabled;
                        Ok(Response::Track(track.track.clone()))
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
    pub(crate) fn suspend_owner(&mut self, owner: &AgentOwner) {
        self.urls.suspend_owner(owner);
        for track in self.tracks.values().filter(|track| track.owner == *owner) {
            if let Some(id) = &track.pip {
                picture_in_picture::visible(id, false);
            }
        }
        self.outputs.retain(|(id, _), _| {
            self.tracks
                .get(id)
                .is_some_and(|track| track.owner != *owner)
        });
    }
    pub(crate) fn retire_owner(&mut self, owner: &AgentOwner) {
        self.urls.retire_owner(owner);
        self.pending.retain(|_, pending| &pending.owner != owner);
        self.outputs.retain(|(id, _), _| {
            self.tracks
                .get(id)
                .is_some_and(|track| &track.owner != owner)
        });
        self.tracks.retain(|_, track| &track.owner != owner);
    }
    pub(crate) fn retire_page(&mut self, page: oryn_common::v3::PageId) {
        self.urls.retire_page(page);
        self.pending.retain(
            |_, pending| !matches!(&pending.owner,AgentOwner::Window{page:owned,..}if *owned==page),
        );
        self.outputs.retain(|(id, _), _| {
            self.tracks.get(id).is_some_and(
                |track| !matches!(&track.owner,AgentOwner::Window{page:owned,..}if *owned==page),
            )
        });
        self.tracks.retain(
            |_, track| !matches!(&track.owner,AgentOwner::Window{page:owned,..}if *owned==page),
        );
    }
    pub(crate) fn end_kind(&mut self, owner: &AgentOwner, kind: Kind) {
        self.pending.retain(|_, pending| {
            &pending.owner != owner
                || pending.constraints.get(if kind == Kind::Video {
                    "video"
                } else {
                    "audio"
                }) == Some(&serde_json::Value::Bool(false))
        });
        for track in self
            .tracks
            .values_mut()
            .filter(|track| track.owner == *owner && track.track.kind == kind)
        {
            self.outputs.retain(|(id, _), _| id != &track.track.id);
            track.track.ended = true;
            track.source = None;
            if let Some(id) = track.pip.take() {
                picture_in_picture::close(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn owner(generation: u64) -> AgentOwner {
        AgentOwner::Window {
            page: oryn_common::v3::PageId(1),
            target: oryn_js::message_wire::WindowTarget::Root,
            generation,
            epoch: 0,
        }
    }
    fn capture(registry: &mut Registry, owner: &AgentOwner) -> Stream {
        let Response::Captured(stream)=registry.request(owner,DeviceConfiguration::SyntheticFixtureV1,Request::Capture{constraints:serde_json::json!({"audio":{},"video":{"width":1280,"height":720,"frameRate":{"exact":30}}})}).unwrap()else{panic!("stream response")};
        stream
    }
    #[test]
    fn pending_capture_is_owned_cancelable_expiring_and_resumes_once_without_new_activation() {
        let mut registry = Registry::default();
        let owner = owner(1);
        let pending = |owner: &AgentOwner| PendingCapture {
            owner: owner.clone(),
            constraints: serde_json::json!({}),
            devices: DeviceConfiguration::SyntheticFixtureV1,
            started: Instant::now(),
        };
        registry.pending.insert("pending".into(), pending(&owner));
        assert!(
            registry
                .request(
                    &super::tests::owner(2),
                    DeviceConfiguration::None,
                    Request::PollCapture {
                        id: "pending".into()
                    }
                )
                .is_err()
        );
        assert!(
            registry
                .request(
                    &super::tests::owner(2),
                    DeviceConfiguration::None,
                    Request::CancelCapture {
                        id: "pending".into()
                    }
                )
                .is_err()
        );
        let Response::Captured(stream) = registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::PollCapture {
                    id: "pending".into(),
                },
            )
            .unwrap()
        else {
            panic!("resumed capture")
        };
        assert_eq!(stream.tracks.len(), 2);
        assert!(
            registry
                .request(
                    &owner,
                    DeviceConfiguration::None,
                    Request::PollCapture {
                        id: "pending".into()
                    }
                )
                .is_err()
        );
        registry.pending.insert("canceled".into(), pending(&owner));
        registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::CancelCapture {
                    id: "canceled".into(),
                },
            )
            .unwrap();
        assert!(!registry.pending.contains_key("canceled"));
        let mut expired = pending(&owner);
        expired.started = Instant::now() - std::time::Duration::from_secs(121);
        registry.pending.insert("expired".into(), expired);
        assert!(
            registry
                .request(
                    &owner,
                    DeviceConfiguration::None,
                    Request::PollCapture {
                        id: "expired".into()
                    }
                )
                .unwrap_err()
                .starts_with("AbortError")
        );
        registry.pending.insert("retired".into(), pending(&owner));
        registry.retire_owner(&owner);
        assert!(registry.pending.is_empty());
        assert!(registry.tracks.is_empty());
    }
    #[test]
    fn camera_only_capture_never_allocates_audio_and_mic_revocation_preserves_pending_camera() {
        let mut registry = Registry::default();
        let owner = owner(1);
        let constraints = serde_json::json!({"audio":false,"video":{"width":640,"height":480}});
        let request = Request::Capture {
            constraints: constraints.clone(),
        };
        assert_eq!(
            registry.capture_features(&owner, &request).unwrap(),
            vec!["camera"]
        );
        let Response::Captured(stream) = registry
            .request(&owner, DeviceConfiguration::SyntheticFixtureV1, request)
            .unwrap()
        else {
            panic!("capture")
        };
        assert_eq!(stream.tracks.len(), 1);
        assert_eq!(stream.tracks[0].kind, Kind::Video);
        assert_eq!(registry.tracks.len(), 1);
        registry.pending.insert(
            "camera-pending".into(),
            PendingCapture {
                owner: owner.clone(),
                constraints,
                devices: DeviceConfiguration::SyntheticFixtureV1,
                started: Instant::now(),
            },
        );
        registry.end_kind(&owner, Kind::Audio);
        assert!(registry.pending.contains_key("camera-pending"));
        assert!(!registry.tracks[&stream.tracks[0].id].track.ended);
        assert!(
            registry
                .capture_features(
                    &super::tests::owner(2),
                    &Request::PollCapture {
                        id: "camera-pending".into()
                    }
                )
                .is_err()
        );
        registry.end_kind(&owner, Kind::Video);
        assert!(!registry.pending.contains_key("camera-pending"));
        assert!(registry.tracks[&stream.tracks[0].id].track.ended);
    }
    #[test]
    fn sample_consumers_use_native_custody_black_disabled_video_and_stop_on_retirement() {
        let mut registry = Registry::default();
        let owner = owner(1);
        let stream = capture(&mut registry, &owner);
        let video = stream
            .tracks
            .iter()
            .find(|track| track.kind == Kind::Video)
            .unwrap();
        let Response::Sample(first) = registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::Sample {
                    id: video.id.clone(),
                    after: 0,
                },
            )
            .unwrap()
        else {
            panic!("sample")
        };
        assert!(first["data"].as_str().is_some());
        assert!(
            registry
                .request(
                    &super::tests::owner(2),
                    DeviceConfiguration::None,
                    Request::Sample {
                        id: video.id.clone(),
                        after: 0
                    }
                )
                .is_err()
        );
        registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::Enable {
                    id: video.id.clone(),
                    enabled: false,
                },
            )
            .unwrap();
        let Response::Sample(disabled) = registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::Sample {
                    id: video.id.clone(),
                    after: 0,
                },
            )
            .unwrap()
        else {
            panic!("sample")
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(disabled["data"].as_str().unwrap())
            .unwrap();
        assert!(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == [0, 0, 0, 255])
        );
        registry.end_kind(&owner, Kind::Video);
        let Response::Sample(ended) = registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::Sample {
                    id: video.id.clone(),
                    after: 0,
                },
            )
            .unwrap()
        else {
            panic!("sample")
        };
        assert_eq!(ended["ended"], true);
        assert!(ended.get("data").is_none());
        registry.retire_owner(&owner);
        assert!(
            registry
                .request(
                    &owner,
                    DeviceConfiguration::None,
                    Request::Sample {
                        id: video.id.clone(),
                        after: 0
                    }
                )
                .is_err()
        );
    }
    #[test]
    fn grants_are_not_devices_and_failed_constraints_do_not_publish_partial_capture() {
        let mut registry = Registry::default();
        let owner = owner(1);
        assert!(
            registry
                .request(
                    &owner,
                    DeviceConfiguration::None,
                    Request::Capture {
                        constraints: serde_json::json!({})
                    }
                )
                .unwrap_err()
                .starts_with("NotFoundError")
        );
        assert!(registry.tracks.is_empty());
        assert!(registry.request(&owner,DeviceConfiguration::SyntheticFixtureV1,Request::Capture{constraints:serde_json::json!({"audio":{},"video":{"width":{"exact":9000}}})}).unwrap_err().starts_with("OverconstrainedError"));
        assert!(registry.tracks.is_empty());
    }
    #[test]
    fn apply_constraints_replaces_only_owned_track_after_success_and_preserves_state_on_failure() {
        let mut registry = Registry::default();
        let owner = owner(1);
        let stream = capture(&mut registry, &owner);
        let video = stream
            .tracks
            .iter()
            .find(|track| track.kind == Kind::Video)
            .unwrap();
        let Response::Track(clone) = registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::Clone {
                    id: video.id.clone(),
                },
            )
            .unwrap()
        else {
            panic!("clone")
        };
        let Response::Capabilities(capabilities) = registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::Capabilities {
                    id: video.id.clone(),
                },
            )
            .unwrap()
        else {
            panic!("capabilities")
        };
        assert_eq!(capabilities["width"]["max"], 1920);
        let Response::Track(updated) = registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::ApplyConstraints {
                    id: video.id.clone(),
                    constraints: serde_json::json!({"width":{"exact":320},"height":{"exact":240}}),
                },
            )
            .unwrap()
        else {
            panic!("apply")
        };
        assert_eq!(updated.id, video.id);
        assert_eq!(updated.settings["width"], 320);
        assert_eq!(registry.tracks[&clone.id].track.settings["width"], 1280);
        assert!(
            registry
                .request(
                    &owner,
                    DeviceConfiguration::None,
                    Request::ApplyConstraints {
                        id: video.id.clone(),
                        constraints: serde_json::json!({"width":{"exact":9000}})
                    }
                )
                .is_err()
        );
        assert_eq!(registry.tracks[&video.id].track, updated);
        registry.end_kind(&owner, Kind::Video);
        let Response::Track(ended) = registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::ApplyConstraints {
                    id: video.id.clone(),
                    constraints: serde_json::json!({"width":640}),
                },
            )
            .unwrap()
        else {
            panic!("ended")
        };
        assert!(ended.ended);
        assert!(registry.tracks[&video.id].source.is_none());
    }
    #[test]
    fn unsupported_exact_device_or_processing_constraints_never_publish_capture() {
        let mut registry = Registry::default();
        let owner = owner(1);
        for constraints in [
            serde_json::json!({"video":{"deviceId":{"exact":"not-installed"}}}),
            serde_json::json!({"audio":{"echoCancellation":{"exact":true}}}),
            serde_json::json!({"video":{"facingMode":{"exact":"environment"}}}),
        ] {
            assert!(
                registry
                    .request(
                        &owner,
                        DeviceConfiguration::SyntheticFixtureV1,
                        Request::Capture { constraints }
                    )
                    .unwrap_err()
                    .starts_with("OverconstrainedError")
            );
            assert!(registry.tracks.is_empty());
        }
        let Response::Captured(stream)=registry.request(&owner,DeviceConfiguration::SyntheticFixtureV1,Request::Capture{constraints:serde_json::json!({"audio":{"echoCancellation":{"ideal":true}},"video":{"deviceId":{"exact":"oryn-fixture-video-v1"}}})}).unwrap()else{panic!("capture")};
        assert_eq!(stream.tracks.len(), 2);
    }
    #[test]
    fn synthetic_tracks_own_samples_clone_independently_and_retire_by_native_principal() {
        let mut registry = Registry::default();
        let owner = owner(1);
        let stream = capture(&mut registry, &owner);
        assert_eq!(stream.tracks.len(), 2);
        let video = stream
            .tracks
            .iter()
            .find(|track| track.kind == Kind::Video)
            .unwrap();
        assert_eq!(video.settings["width"], 1280);
        let source = registry.tracks[&video.id].source.as_ref().unwrap().clone();
        match &*source {
            Source::Video {
                width,
                height,
                rgba,
                ..
            } => {
                assert_eq!((*width, *height), (1280, 720));
                assert_eq!(rgba.len(), 1280 * 720 * 4);
                assert_eq!(rgba[3], 255)
            }
            _ => panic!("video source"),
        }
        let audio = stream
            .tracks
            .iter()
            .find(|track| track.kind == Kind::Audio)
            .unwrap();
        match &**registry.tracks[&audio.id].source.as_ref().unwrap() {
            Source::Audio {
                sample_rate,
                channels,
                pcm,
                ..
            } => {
                assert_eq!((*sample_rate, *channels), (48000, 1));
                assert_eq!(pcm.len(), 480)
            }
            _ => panic!("audio source"),
        }
        let Response::Track(clone) = registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::Clone {
                    id: video.id.clone(),
                },
            )
            .unwrap()
        else {
            panic!("clone")
        };
        assert_ne!(clone.id, video.id);
        registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::Stop {
                    id: video.id.clone(),
                },
            )
            .unwrap();
        assert!(registry.tracks[&video.id].track.ended);
        assert!(registry.tracks[&video.id].source.is_none());
        assert!(!registry.tracks[&clone.id].track.ended);
        assert!(registry.tracks[&clone.id].source.is_some());
        assert!(
            registry
                .request(
                    &super::tests::owner(2),
                    DeviceConfiguration::None,
                    Request::Stop {
                        id: clone.id.clone()
                    }
                )
                .is_err()
        );
        registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::Release {
                    id: video.id.clone(),
                },
            )
            .unwrap();
        assert!(!registry.tracks.contains_key(&video.id));
        registry.retire_owner(&owner);
        assert!(registry.tracks.is_empty());
    }
}

// A retained latest frame is not necessarily lost: independent consumers may
// have already read it. Device-reported losses remain cumulative and separate
// from the exact delivered-sequence gap for this requesting consumer.
fn normalize_sample_loss(sample: &mut serde_json::Value, after: u64, physical: bool) {
    if physical {
        sample["source_dropped"] = sample["dropped"].clone();
    }
    sample["dropped"] = serde_json::json!(
        sample["sequence"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(after.saturating_add(1))
    );
}
#[cfg(test)]
mod sample_loss_regressions {
    use super::*;
    #[test]
    fn physical_samples_cannot_silently_change_advertised_capture_settings() {
        let track = oryn_js::media_capture::Track {
            id: "native".into(),
            kind: Kind::Audio,
            label: "device".into(),
            enabled: true,
            ended: false,
            settings: serde_json::json!({"sampleRate":48000,"channelCount":2}),
            constraints: serde_json::json!({}),
        };
        let sample =
            serde_json::json!({"data":"AAAAAA==","kind":"audio","sampleRate":48000,"channels":2});
        assert!(validate_physical_sample(&sample, &track).is_ok());
        let mut mismatch = sample.clone();
        mismatch["sampleRate"] = serde_json::json!(44100);
        assert!(
            validate_physical_sample(&mismatch, &track)
                .unwrap_err()
                .contains("sampleRate")
        );
        mismatch = sample.clone();
        mismatch["channels"] = serde_json::json!(1);
        assert!(
            validate_physical_sample(&mismatch, &track)
                .unwrap_err()
                .contains("channelCount")
        );
        mismatch = sample;
        mismatch["kind"] = serde_json::json!("video");
        assert!(validate_physical_sample(&mismatch, &track).is_err());
        assert!(validate_physical_sample(&serde_json::json!({"ended":false}), &track).is_ok());
    }
    #[test]
    fn sample_loss_is_consumer_scoped_and_preserves_independent_device_loss() {
        let native = serde_json::json!({"sequence":10,"dropped":3});
        let mut current = native.clone();
        normalize_sample_loss(&mut current, 9, true);
        assert_eq!(current["dropped"], 0);
        assert_eq!(current["source_dropped"], 3);
        let mut slow = native.clone();
        normalize_sample_loss(&mut slow, 6, true);
        assert_eq!(slow["dropped"], 3);
        assert_eq!(slow["source_dropped"], 3);
        normalize_sample_loss(&mut current, 10, true);
        assert_eq!(current["dropped"], 0);
        let mut no_sample = serde_json::json!({"ended":false});
        normalize_sample_loss(&mut no_sample, u64::MAX, false);
        assert_eq!(no_sample["dropped"], 0);
    }
}

fn validate_physical_sample(
    sample: &serde_json::Value,
    track: &oryn_js::media_capture::Track,
) -> Result<(), String> {
    if sample.get("data").is_none() {
        return Ok(());
    }
    let expected = if track.kind == Kind::Video {
        "video"
    } else {
        "audio"
    };
    if sample["kind"].as_str() != Some(expected) {
        return Err("NotReadableError: native capture sample kind changed".into());
    }
    let fields: &[(&str, &str)] = if track.kind == Kind::Video {
        &[("width", "width"), ("height", "height")]
    } else {
        &[("sampleRate", "sampleRate"), ("channels", "channelCount")]
    };
    for (sample_field, setting) in fields {
        if sample[*sample_field]
            .as_f64()
            .zip(track.settings[*setting].as_f64())
            .is_none_or(|(actual, advertised)| {
                !actual.is_finite() || actual <= 0. || actual != advertised
            })
        {
            return Err(format!(
                "NotReadableError: native capture {setting} differs from selected format"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod picture_in_picture_tests {
    use super::*;
    #[test]
    fn capture_pip_authenticates_tracks_and_closes_on_stop_and_retirement() {
        let mut registry = Registry::default();
        let owner = super::tests::owner(1);
        let foreign = super::tests::owner(2);
        // Native requests carry converted track dictionaries, matching the camera
        // element's producer. Reject an unconverted boolean without allocating a
        // source, then request only the video track used by picture-in-picture.
        assert!(
            registry
                .request(
                    &owner,
                    DeviceConfiguration::SyntheticFixtureV1,
                    Request::Capture {
                        constraints: serde_json::json!({"audio":false,"video":true})
                    }
                )
                .unwrap_err()
                .starts_with("TypeError: capture track constraints must be dictionaries")
        );
        assert!(registry.tracks.is_empty());
        assert!(registry.pending.is_empty());
        let Response::Captured(stream) = registry
            .request(
                &owner,
                DeviceConfiguration::SyntheticFixtureV1,
                Request::Capture {
                    constraints: serde_json::json!({"audio":false,"video":{}}),
                },
            )
            .unwrap()
        else {
            panic!("capture")
        };
        assert_eq!(stream.tracks.len(), 1);
        assert_eq!(stream.tracks[0].kind, Kind::Video);
        let track = stream.tracks[0].id.clone();
        assert!(
            registry
                .request(
                    &foreign,
                    DeviceConfiguration::None,
                    Request::UrlPictureInPicture {
                        id: track.clone(),
                        enabled: true
                    }
                )
                .is_err()
        );
        assert!(matches!(
            registry
                .request(
                    &owner,
                    DeviceConfiguration::None,
                    Request::UrlPictureInPicture {
                        id: track.clone(),
                        enabled: true
                    }
                )
                .unwrap(),
            Response::UrlPictureInPicture(Some(_))
        ));
        assert!(registry.has_picture_in_picture(&owner));
        let surface = registry.tracks[&track].pip.clone().unwrap();
        assert!(
            !picture_in_picture::snapshots()
                .into_iter()
                .find(|value| value.id == surface)
                .unwrap()
                .pixels
                .is_empty()
        );
        registry.suspend_owner(&owner);
        assert!(
            !picture_in_picture::snapshots()
                .into_iter()
                .find(|value| value.id == surface)
                .unwrap()
                .visible
        );
        registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::UrlPictureInPictureState { id: track.clone() },
            )
            .unwrap();
        assert!(
            picture_in_picture::snapshots()
                .into_iter()
                .find(|value| value.id == surface)
                .unwrap()
                .visible
        );
        registry
            .request(
                &owner,
                DeviceConfiguration::None,
                Request::Stop { id: track.clone() },
            )
            .unwrap();
        assert!(picture_in_picture::state(&surface).is_none());
        assert!(!registry.has_picture_in_picture(&owner));
        assert!(
            registry
                .request(
                    &owner,
                    DeviceConfiguration::None,
                    Request::UrlPictureInPicture {
                        id: track,
                        enabled: true
                    }
                )
                .is_err()
        );
        registry.retire_owner(&owner);
        assert!(registry.tracks.is_empty());
    }
}
