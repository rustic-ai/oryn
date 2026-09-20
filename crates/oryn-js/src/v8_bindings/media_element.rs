//! Native MediaStream-backed media elements; no author track properties are used
//! for parent capability lookup or decoded frame publication.
use super::*;
#[derive(Default)]
pub(super) struct Playback {
    asset: Option<String>,
    duration: f64,
    anchor: Option<std::time::Instant>,
    last_timeupdate: Option<std::time::Instant>,
    origin_clean: bool,
    has_audio: bool,
    seeking: bool,
    seek_sampled: bool,
    url_events: VecDeque<&'static str>,
    aborted: Vec<v8::Global<v8::PromiseResolver>>,
    stream: Option<v8::Global<v8::Object>>,
    error: Option<v8::Global<v8::Object>>,
    session: u64,
    seek_serial: u64,
    paused: bool,
    ended: bool,
    volume: f32,
    muted: bool,
    ready: u16,
    time: f64,
    width: u32,
    height: u32,
    sequences: BTreeMap<String, u64>,
    pending: Vec<v8::Global<v8::PromiseResolver>>,
}
fn fail(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "HTMLMediaElement".into(),
        member: name.into(),
    }
}
pub(super) fn url_reset(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    url_reset_for_selection(state, node, false)
}
pub(super) fn url_reset_for_selection(
    state: &V8BindingState,
    node: NodeId,
    preserve_play: bool,
) -> Result<(), V8BindingError> {
    ensure(state, node)?;
    state
        .dom
        .borrow_mut()
        .set_showing_poster(node, !preserve_play)?;
    state.dom.borrow_mut().set_media_frame(node, None)?;
    picture_in_picture::invalidate(state, node);
    let mut media = state.native_media.borrow_mut();
    let value = media.playback.get_mut(&node).unwrap();
    let asset = value.asset.take();
    value.ready = 0;
    value.error = None;
    value.ended = false;
    value.anchor = None;
    value.time = 0.;
    value.seeking = false;
    value.seek_sampled = false;
    if !preserve_play {
        value.url_events.clear();
    }
    value.session = value
        .session
        .checked_add(1)
        .ok_or(fail("QuotaExceededError"))?;
    if !preserve_play {
        value.paused = true;
        value.aborted.append(&mut value.pending);
    }
    drop(media);
    if let Some(id) = asset {
        media_resource::release_asset(state, id);
    }
    publish_state(state, node)?;
    Ok(())
}
pub(super) fn url_error(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    ensure(state, node)?;
    let id = register_platform_object(
        state,
        PlatformObjectRecord::MediaError {
            code: 4,
            message: "No supported media resource".into(),
        },
    );
    let object = new_platform_object(scope, state, "MediaError", id)?;
    let mut media = state.native_media.borrow_mut();
    let value = media.playback.get_mut(&node).unwrap();
    value.ready = 0;
    value.error = Some(v8::Global::new(scope, object));
    value.paused = true;
    let pending = std::mem::take(&mut value.pending);
    drop(media);
    for resolver in pending {
        let resolver = v8::Local::new(scope, resolver);
        let reason = dom_exception_value(scope, "NotSupportedError")?;
        resolver.reject(scope, reason);
    }
    Ok(())
}
pub(super) fn url_metadata(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    ensure(state, node)?;
    let mut media = state.native_media.borrow_mut();
    let value = media.playback.get_mut(&node).unwrap();
    value.ready = value.ready.max(1);
    Ok(())
}
/// Native cue sampling shares playback time and the seek/session discontinuity.
pub(super) fn text_track_clock(
    state: &V8BindingState,
    node: NodeId,
) -> Result<(u64, u64, f64), V8BindingError> {
    ensure(state, node)?;
    let media = state.native_media.borrow();
    let value = &media.playback[&node];
    let time = if value.asset.is_some() {
        (value.time
            + value
                .anchor
                .map_or(0., |anchor| anchor.elapsed().as_secs_f64()))
        .min(value.duration)
    } else {
        value.time
    };
    Ok((value.session, value.seek_serial, time))
}
pub(super) fn track_descriptors(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(u64, Vec<av_tracks::Descriptor>), V8BindingError> {
    ensure(state, node)?;
    let (session, stream, asset, audio, width, height) = {
        let media = state.native_media.borrow();
        let value = &media.playback[&node];
        (
            value.session,
            value.stream.clone(),
            value.asset.clone(),
            value.has_audio,
            value.width,
            value.height,
        )
    };
    if let Some(stream) = stream {
        let stream = v8::Local::new(scope, stream);
        let tracks = native_media::stream_tracks(scope, state, stream)?;
        return Ok((
            session,
            tracks
                .into_iter()
                .filter(|track| !track.ended)
                .map(|track| av_tracks::Descriptor {
                    key: track.id,
                    video: track.kind == crate::media_capture::Kind::Video,
                    label: track.label,
                    language: String::new(),
                })
                .collect(),
        ));
    }
    let mut tracks = Vec::new();
    if let Some(asset) = asset {
        if audio {
            tracks.push(av_tracks::Descriptor {
                key: format!("{asset}:audio"),
                video: false,
                label: String::new(),
                language: String::new(),
            });
        }
        if width > 0 && height > 0 {
            tracks.push(av_tracks::Descriptor {
                key: format!("{asset}:video"),
                video: true,
                label: String::new(),
                language: String::new(),
            });
        }
    }
    Ok((session, tracks))
}
pub(super) fn track_selection_changed(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    clear_video: bool,
) -> Result<(), V8BindingError> {
    stop_outputs(scope, state, node)?;
    let request = {
        let media = state.native_media.borrow();
        media.playback.get(&node).and_then(|value| {
            value
                .asset
                .as_ref()
                .map(|id| crate::media_capture::Request::UrlSample {
                    id: id.clone(),
                    time: (value.time
                        + value
                            .anchor
                            .map_or(0., |anchor| anchor.elapsed().as_secs_f64()))
                    .min(value.duration),
                    paused: true,
                    volume: 0.,
                    seek: true,
                })
        })
    };
    if let Some(request) = request {
        native_platform::media_request(state, request)?;
    }
    if let Some(value) = state.native_media.borrow_mut().playback.get_mut(&node) {
        value.seek_sampled = false;
    }
    if clear_video {
        state.decoded_images.borrow_mut().remove(&node);
        state.dom.borrow_mut().set_media_frame(node, None)?;
    }
    Ok(())
}
pub(super) fn has_stream(value: &Playback) -> bool {
    value.stream.is_some()
}
fn ensure(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    if !["video", "audio"].iter().any(|tag| {
        state
            .dom
            .borrow()
            .is_element(node, "http://www.w3.org/1999/xhtml", tag)
    }) {
        return Err(fail("TypeError"));
    }
    let muted = state
        .dom
        .borrow()
        .attribute_in_null_namespace(node, "muted")?
        .is_some();
    let mut media = state.native_media.borrow_mut();
    if !media.playback.contains_key(&node) && media.playback.len() >= 256 {
        return Err(fail("QuotaExceededError"));
    }
    media.playback.entry(node).or_insert_with(|| Playback {
        paused: true,
        volume: 1.,
        muted,
        ..Default::default()
    });
    Ok(())
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    ensure(state, node)?;
    let media = state.native_media.borrow();
    let value = &media.playback[&node];
    Ok(match member {
        "error" => value
            .error
            .as_ref()
            .map(|error| v8::Local::new(scope, error).into())
            .unwrap_or_else(|| v8::null(scope).into()),
        "srcObject" => value
            .stream
            .as_ref()
            .map(|stream| v8::Local::new(scope, stream).into())
            .unwrap_or_else(|| v8::null(scope).into()),
        "ended" => v8::Boolean::new(scope, value.ended).into(),
        "seeking" => v8::Boolean::new(scope, value.seeking).into(),
        "muted" => v8::Boolean::new(scope, value.muted).into(),
        "volume" => v8::Number::new(scope, f64::from(value.volume)).into(),
        "paused" => v8::Boolean::new(scope, value.paused).into(),
        "readyState" => v8::Integer::new(scope, i32::from(value.ready)).into(),
        "currentTime" => v8::Number::new(
            scope,
            if value.asset.is_some() {
                (value.time
                    + value
                        .anchor
                        .map_or(0., |anchor| anchor.elapsed().as_secs_f64()))
                .min(value.duration)
            } else {
                value.time
            },
        )
        .into(),
        "videoWidth" => v8::Number::new(scope, f64::from(value.width)).into(),
        "videoHeight" => v8::Number::new(scope, f64::from(value.height)).into(),
        _ => return Err(fail("unsupported native media attribute")),
    })
}
pub(super) fn set(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    ensure(state, node)?;
    media_resource::cancel(state, node)?;
    url_reset(state, node)?;
    let stream = if value.is_null() {
        None
    } else {
        let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| fail("TypeError"))?;
        native_media::stream_tracks(scope, state, object)?;
        Some(v8::Global::new(scope, object))
    };
    stop_outputs(scope, state, node)?;
    let (session, volume, muted) = {
        let media = state.native_media.borrow();
        let old = &media.playback[&node];
        (
            old.session
                .checked_add(1)
                .ok_or(fail("QuotaExceededError"))?,
            old.volume,
            old.muted,
        )
    };
    let old = state
        .native_media
        .borrow_mut()
        .playback
        .insert(
            node,
            Playback {
                stream,
                session,
                paused: true,
                volume,
                muted,
                ..Default::default()
            },
        )
        .unwrap();
    for pending in old.pending.into_iter().chain(old.aborted) {
        let pending = v8::Local::new(scope, pending);
        let reason = dom_exception_value(scope, "AbortError")?;
        pending.reject(scope, reason);
    }
    state.decoded_images.borrow_mut().remove(&node);
    state.dom.borrow_mut().set_media_frame(node, None)?;
    Ok(())
}
pub(super) fn call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    member: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    ensure(state, node)?;
    if member == "pause" {
        stop_outputs(scope, state, node)?;
        url_pause(state, node)?;
        let pending = {
            let mut media = state.native_media.borrow_mut();
            let playback = media.playback.get_mut(&node).unwrap();
            if !playback.paused && playback.stream.is_some() {
                playback.url_events.push_back("timeupdate");
                playback.url_events.push_back("pause");
            }
            playback.paused = true;
            std::mem::take(&mut playback.pending)
        };
        for pending in pending {
            let pending = v8::Local::new(scope, pending);
            let reason = dom_exception_value(scope, "AbortError")?;
            pending.reject(scope, reason);
        }
        publish_state(state, node)?;
        return Ok(None);
    }
    let resolver = v8::PromiseResolver::new(scope).ok_or(fail("allocation"))?;
    let promise = resolver.get_promise(scope);
    state.dom.borrow_mut().set_showing_poster(node, false)?;
    let mut media = state.native_media.borrow_mut();
    let playback = media.playback.get_mut(&node).unwrap();
    if playback.pending.len() >= 64 {
        return Err(fail("QuotaExceededError"));
    }
    if playback.stream.is_none() {
        let first = playback.paused;
        playback.paused = false;
        if first {
            playback.url_events.push_back("play");
            if playback.ready < 3 {
                playback.url_events.push_back("waiting");
            }
        }
        playback.pending.push(v8::Global::new(scope, resolver));
        let loaded = playback.asset.is_some();
        if loaded {
            if first {
                if playback.ended {
                    playback.time = 0.;
                    playback.ended = false;
                }
                playback.anchor = Some(std::time::Instant::now());
            }
            playback
                .url_events
                .push_back(if first { "playing" } else { "__resolveplay" });
        }
        drop(media);
        if !loaded {
            media_resource::request_play(state, node)?;
        }
    } else {
        let first = playback.paused;
        playback.paused = false;
        if first {
            playback.url_events.push_back("play");
            if playback.ready < 3 {
                playback.url_events.push_back("waiting");
            }
        }
        playback.pending.push(v8::Global::new(scope, resolver));
        if playback.ready >= 3 {
            playback
                .url_events
                .push_back(if first { "playing" } else { "__resolveplay" });
        }
        drop(media);
    }
    publish_state(state, node)?;
    Ok(Some(promise.into()))
}
/// A collected or adopted node cannot be passed through `ensure` or have its
/// new owner's DOM output cleared. Keep the old playback until all fallible
/// promise publication has succeeded; native release failures retain ACK custody.
fn release_stale_owner(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    media_resource::cancel(state, node)?;
    stop_outputs(scope, state, node)?;
    let asset = {
        let mut media = state.native_media.borrow_mut();
        media.playback.get_mut(&node).and_then(|value| {
            value.paused = true;
            value.anchor = None;
            value.url_events.clear();
            value.asset.take()
        })
    };
    if let Some(asset) = asset {
        media_resource::release_asset(state, asset);
    }
    loop {
        let pending = {
            let media = state.native_media.borrow();
            media.playback.get(&node).and_then(|value| {
                value
                    .pending
                    .first()
                    .map(|pending| (false, pending.clone()))
                    .or_else(|| value.aborted.first().map(|pending| (true, pending.clone())))
            })
        };
        let Some((aborted, pending)) = pending else {
            break;
        };
        let reason = dom_exception_value(scope, "AbortError")?;
        let pending = v8::Local::new(scope, pending);
        pending
            .reject(scope, reason)
            .ok_or(V8BindingError::JavaScriptException)?;
        if let Some(value) = state.native_media.borrow_mut().playback.get_mut(&node) {
            if aborted {
                value.aborted.remove(0);
            } else {
                value.pending.remove(0);
            }
        }
    }
    state.native_media.borrow_mut().playback.remove(&node);
    state.decoded_images.borrow_mut().remove(&node);
    picture_in_picture::invalidate(state, node);
    Ok(())
}
fn poll_one(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<usize, V8BindingError> {
    let mut changed = 0;
    if state.dom.borrow().node_document(node).ok().flatten() != Some(state.document_root()) {
        release_stale_owner(scope, state, node)?;
        return Ok(0);
    }
    av_tracks::refresh(scope, state, node)?;
    let (stream, sequences, paused, volume, session) = {
        let media = state.native_media.borrow();
        let value = &media.playback[&node];
        (
            value.stream.clone(),
            value.sequences.clone(),
            value.paused,
            if value.muted { 0. } else { value.volume },
            value.session,
        )
    };
    if paused && stream.is_some() {
        return Ok(0);
    }
    let Some(stream) = stream else {
        return poll_url(scope, state, node);
    };
    let stream = v8::Local::new(scope, stream);
    let tracks = native_media::stream_tracks(scope, state, stream)?;
    if tracks.iter().all(|track| track.ended) {
        let pending = {
            let mut media = state.native_media.borrow_mut();
            let value = media.playback.get_mut(&node).unwrap();
            value.ended = true;
            value.paused = true;
            std::mem::take(&mut value.pending)
        };
        for resolver in pending {
            let resolver = v8::Local::new(scope, resolver);
            let reason = dom_exception_value(scope, "AbortError")?;
            resolver.reject(scope, reason);
        }
        event(scope, state, node, "ended")?;
        return Ok(1);
    }
    for track in tracks {
        if track.ended
            || !av_tracks::enabled(
                state,
                node,
                &track.id,
                track.kind == crate::media_capture::Kind::Video,
            )
        {
            continue;
        }
        let after = sequences.get(&track.id).copied().unwrap_or(0);
        let crate::media_capture::Response::Sample(sample) = native_platform::media_request(
            state,
            if track.kind == crate::media_capture::Kind::Audio {
                crate::media_capture::Request::AudioRender {
                    id: track.id.clone(),
                    consumer: consumer(node),
                    after,
                    paused: false,
                    volume,
                }
            } else {
                crate::media_capture::Request::Sample {
                    id: track.id.clone(),
                    after,
                }
            },
        )?
        else {
            return Err(fail("InvalidStateError"));
        };
        let data = sample["data"].as_str().unwrap_or("");
        if data.is_empty() && sample["rendered"].as_bool() != Some(true) {
            continue;
        }
        if data.len() > 24 * 1024 * 1024 {
            return Err(fail("QuotaExceededError"));
        }
        let bytes = BASE64.decode(data).map_err(|_| fail("NotReadableError"))?;
        let sequence = sample["sequence"]
            .as_u64()
            .ok_or(fail("NotReadableError"))?;
        let timestamp = sample["timestamp"]
            .as_f64()
            .filter(|time| time.is_finite() && *time >= 0.)
            .ok_or(fail("NotReadableError"))?;
        if track.kind == crate::media_capture::Kind::Video {
            let width = sample["width"]
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or(fail("NotReadableError"))?;
            let height = sample["height"]
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or(fail("NotReadableError"))?;
            if width == 0
                || height == 0
                || bytes.len() > 16 * 1024 * 1024
                || u64::from(width) * u64::from(height) * 4 != bytes.len() as u64
            {
                return Err(fail("NotReadableError"));
            }
            let mut png = Vec::new();
            use image::ImageEncoder;
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(&bytes, width, height, image::ExtendedColorType::Rgba8)
                .map_err(|_| fail("NotReadableError"))?;
            state.decoded_images.borrow_mut().insert(
                node,
                DecodedImage {
                    origin_clean: true,
                    width,
                    height,
                    rgba: bytes,
                },
            );
            state
                .dom
                .borrow_mut()
                .set_image_dimensions(node, Some((width, height)))?;
            state.dom.borrow_mut().set_media_frame(node, Some(png))?;
            let mut media = state.native_media.borrow_mut();
            let playback = media.playback.get_mut(&node).unwrap();
            playback.width = width;
            playback.height = height;
        }
        if track.kind == crate::media_capture::Kind::Audio
            && state
                .dom
                .borrow()
                .is_element(node, "http://www.w3.org/1999/xhtml", "video")
        {
            state
                .native_media
                .borrow_mut()
                .playback
                .get_mut(&node)
                .unwrap()
                .sequences
                .insert(track.id, sequence);
            continue;
        }
        let first = {
            let mut media = state.native_media.borrow_mut();
            let playback = media.playback.get_mut(&node).unwrap();
            let first = playback.ready == 0;
            playback.ready = 4;
            playback.time = timestamp;
            playback.sequences.insert(track.id, sequence);
            first
        };
        if first {
            for name in ["loadedmetadata", "loadeddata", "canplay", "playing"] {
                event(scope, state, node, name)?;
            }
        }
        changed += 1;
        if !current(state, node, session) {
            break;
        }
    }
    Ok(changed)
}

pub(super) fn is_playing(value: &Playback) -> bool {
    !value.paused || !value.url_events.is_empty() || !value.aborted.is_empty() || value.seeking
}

fn consumer(node: NodeId) -> u64 {
    ((node.generation as u64) << 32) | node.slot as u64
}
fn stop_outputs(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    let stream = state
        .native_media
        .borrow()
        .playback
        .get(&node)
        .and_then(|value| value.stream.clone());
    if let Some(stream) = stream {
        let stream = v8::Local::new(scope, stream);
        for track in native_media::stream_tracks(scope, state, stream)? {
            if track.kind == crate::media_capture::Kind::Audio {
                let key = (track.id, consumer(node));
                let response = native_platform::media_request(
                    state,
                    crate::media_capture::Request::AudioRender {
                        id: key.0.clone(),
                        consumer: key.1,
                        after: 0,
                        paused: true,
                        volume: 0.,
                    },
                );
                if response.is_err() {
                    state.native_media.borrow_mut().audio_cleanup.insert(key);
                }
            }
        }
    }
    Ok(())
}
pub(super) fn volume(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    member: &str,
    input: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    ensure(state, node)?;
    let volume = if member == "muted" {
        None
    } else {
        let value = input
            .number_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        if !value.is_finite() || !(0.0..=1.).contains(&value) {
            return Err(fail("IndexSizeError"));
        }
        Some(value as f32)
    };
    let mut media = state.native_media.borrow_mut();
    let playback = media.playback.get_mut(&node).unwrap();
    let changed = if let Some(volume) = volume {
        let changed = playback.volume != volume;
        playback.volume = volume;
        changed
    } else {
        let value = input.boolean_value(scope);
        let changed = playback.muted != value;
        playback.muted = value;
        changed
    };
    if changed {
        playback.url_events.push_back("volumechange");
    }
    Ok(())
}

fn current(state: &V8BindingState, node: NodeId, session: u64) -> bool {
    !state.execution_retired()
        && state
            .native_media
            .borrow()
            .playback
            .get(&node)
            .is_some_and(|value| value.session == session && !value.paused)
        && state.dom.borrow().node_document(node).ok().flatten() == Some(state.document_root())
}
fn event(
    _scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    name: &'static str,
) -> Result<(), V8BindingError> {
    let mut media = state.native_media.borrow_mut();
    let value = media
        .playback
        .get_mut(&node)
        .ok_or(fail("InvalidStateError"))?;
    if value.url_events.len() >= 512 {
        return Err(V8BindingError::Allocation("media event bound"));
    }
    value.url_events.push_back(name);
    Ok(())
}

fn fail_playback(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    error: &V8BindingError,
) -> Result<(), V8BindingError> {
    stop_outputs(scope, state, node)?;
    let (name, message) = match error {
        V8BindingError::Unsupported { member, .. } => (
            member.split(':').next().unwrap_or("NotReadableError"),
            member.clone(),
        ),
        _ => ("NotReadableError", "Native media sample unavailable".into()),
    };
    let code = if name == "NotSupportedError" {
        4
    } else if name == "AbortError" {
        1
    } else {
        3
    };
    let id = register_platform_object(state, PlatformObjectRecord::MediaError { code, message });
    let object = new_platform_object(scope, state, "MediaError", id)?;
    let (asset, pending) = {
        let mut media = state.native_media.borrow_mut();
        let Some(value) = media.playback.get_mut(&node) else {
            return Ok(());
        };
        value.paused = true;
        value.anchor = None;
        value.url_events.clear();
        value.error = Some(v8::Global::new(scope, object));
        (value.asset.take(), std::mem::take(&mut value.pending))
    };
    if let Some(id) = asset {
        media_resource::release_asset(state, id);
    }
    publish_state(state, node)?;
    for pending in pending {
        let pending = v8::Local::new(scope, pending);
        let reason = dom_exception_value(scope, name)?;
        pending.reject(scope, reason);
    }
    if !state.execution_retired() {
        event(scope, state, node, "error")?;
    }
    Ok(())
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.execution_retired()
        || state.execution_suspended()
        || state.native_media.borrow().event_polling
    {
        return Ok(0);
    }
    state.native_media.borrow_mut().event_polling = true;
    struct Guard<'a>(&'a V8BindingState);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.native_media.borrow_mut().event_polling = false;
        }
    }
    let _guard = Guard(state);
    flush_events(state)?;
    let (before, seeking) = poll_events(scope, state)?;
    if seeking || !state.native_media.borrow().events.is_empty() || state.execution_suspended() {
        return Ok(before);
    }
    let cleanup = state
        .native_media
        .borrow()
        .audio_cleanup
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    for (id, consumer) in cleanup {
        let response = native_platform::media_request(
            state,
            crate::media_capture::Request::AudioRender {
                id: id.clone(),
                consumer,
                after: 0,
                paused: true,
                volume: 0.,
            },
        );
        let retired = matches!(&response,Err(V8BindingError::Unsupported{member,..})if member.starts_with("InvalidStateError"));
        if response.is_ok() || retired {
            state
                .native_media
                .borrow_mut()
                .audio_cleanup
                .remove(&(id, consumer));
        }
    }
    let nodes = state
        .native_media
        .borrow()
        .playback
        .keys()
        .copied()
        .collect::<Vec<_>>();
    let mut changed = before;
    for node in nodes {
        if state.execution_retired() {
            break;
        }
        match poll_one(scope, state, node) {
            Ok(count) => changed += count,
            Err(error @ (V8BindingError::Allocation(_) | V8BindingError::JavaScriptException)) => {
                return Err(error);
            }
            Err(error) => {
                if state.execution_retired() {
                    break;
                }
                fail_playback(scope, state, node, &error)?;
                changed += 1;
            }
        }
    }
    flush_events(state)?;
    let (after, _) = poll_events(scope, state)?;
    Ok(changed + after)
}

pub(super) fn url_wants_data(state: &V8BindingState, node: NodeId) -> bool {
    state
        .native_media
        .borrow()
        .playback
        .get(&node)
        .is_some_and(|value| !value.paused)
}
pub(super) fn url_decoded(
    state: &V8BindingState,
    node: NodeId,
    metadata: &serde_json::Value,
    origin_clean: bool,
) -> Result<(), V8BindingError> {
    ensure(state, node)?;
    let id = metadata["id"]
        .as_str()
        .ok_or(fail("NotReadableError"))?
        .to_owned();
    let duration = metadata["duration"]
        .as_f64()
        .filter(|x| x.is_finite() && *x >= 0.)
        .ok_or(fail("NotReadableError"))?;
    let width = metadata["width"]
        .as_u64()
        .and_then(|x| u32::try_from(x).ok())
        .ok_or(fail("NotReadableError"))?;
    let height = metadata["height"]
        .as_u64()
        .and_then(|x| u32::try_from(x).ok())
        .ok_or(fail("NotReadableError"))?;
    let first = native_platform::media_request(
        state,
        crate::media_capture::Request::UrlSample {
            id: id.clone(),
            time: 0.,
            paused: true,
            volume: 0.,
            seek: true,
        },
    );
    if !matches!(first, Ok(crate::media_capture::Response::UrlFrame(_))) {
        media_resource::release_asset(state, id);
        return Err(fail("NotSupportedError"));
    }
    let autoplay = state
        .dom
        .borrow()
        .attribute_in_null_namespace(node, "autoplay")?
        .is_some()
        && state.refresh_native_container_policy().is_ok()
        && state.native_permissions_policy_allows("autoplay");
    let old = {
        let mut media = state.native_media.borrow_mut();
        let value = media.playback.get_mut(&node).unwrap();
        if autoplay && (value.muted || state.has_sticky_user_activation()) {
            if value.paused {
                value.url_events.push_back("play");
            }
            value.paused = false;
        }
        value.origin_clean = origin_clean;
        value.has_audio = metadata["audio"].as_bool().unwrap_or(false);
        value.duration = duration;
        value.width = width;
        value.height = height;
        value.ready = 4;
        value.seek_sampled = false;
        value.time = value.time.min(duration);
        if !value.paused {
            value.anchor = Some(std::time::Instant::now());
            value.url_events.push_back("playing");
        }
        value.asset.replace(id)
    };
    if let Some(id) = old {
        media_resource::release_asset(state, id);
    }
    if !state.native_media.borrow().playback[&node].paused {
        state.dom.borrow_mut().set_showing_poster(node, false)?;
    }
    Ok(())
}
fn url_pause(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    let request = {
        let mut media = state.native_media.borrow_mut();
        let value = media.playback.get_mut(&node).unwrap();
        if value.stream.is_some() {
            return Ok(());
        }
        if !value.paused {
            if let Some(anchor) = value.anchor.take() {
                value.time = (value.time + anchor.elapsed().as_secs_f64()).min(value.duration);
            }
            value.url_events.push_back("timeupdate");
            value.url_events.push_back("pause");
        }
        value
            .asset
            .clone()
            .map(|id| crate::media_capture::Request::UrlSample {
                id,
                time: value.time,
                paused: true,
                volume: 0.,
                seek: true,
            })
    };
    if let Some(request) = request {
        native_platform::media_request(state, request)?;
    }
    Ok(())
}
pub(super) fn seek(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    input: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    ensure(state, node)?;
    let time = input
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    if !time.is_finite() {
        return Err(fail("TypeError"));
    }
    let mut media = state.native_media.borrow_mut();
    let value = media.playback.get_mut(&node).unwrap();
    value.seek_serial = value
        .seek_serial
        .checked_add(1)
        .ok_or(fail("QuotaExceededError"))?;
    value.time = if value.asset.is_some() {
        time.max(0.).min(value.duration)
    } else {
        time.max(0.)
    };
    value.anchor = (!value.paused).then(std::time::Instant::now);
    value.ended = false;
    let hide_poster = value.ready > 0;
    if hide_poster {
        value.seeking = true;
        value.seek_sampled = false;
        value.url_events.push_back("seeking");
    }
    drop(media);
    if hide_poster {
        state.dom.borrow_mut().set_showing_poster(node, false)?;
    }
    publish_state(state, node)?;
    Ok(())
}
fn poll_url(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<usize, V8BindingError> {
    let aborted = std::mem::take(
        &mut state
            .native_media
            .borrow_mut()
            .playback
            .get_mut(&node)
            .unwrap()
            .aborted,
    );
    for resolver in aborted {
        let resolver = v8::Local::new(scope, resolver);
        let reason = dom_exception_value(scope, "AbortError")?;
        resolver.reject(scope, reason);
    }
    let changed = 0;
    // Each checkpoint publishes a stable event phase. A seeking handler and its
    // microtasks observe seeking=true before the following seeked task.
    let delivered_seeking = state.native_media.borrow().playback[&node]
        .url_events
        .contains(&"seeking");
    if delivered_seeking || state.execution_retired() || state.execution_suspended() {
        return Ok(changed);
    }
    let (id, time, paused, volume, seek, already, duration) = {
        let media = state.native_media.borrow();
        let value = &media.playback[&node];
        (
            value.asset.clone(),
            value.time
                + value
                    .anchor
                    .map_or(0., |anchor| anchor.elapsed().as_secs_f64()),
            value.paused,
            if value.muted { 0. } else { value.volume },
            value.seeking,
            value.seek_sampled,
            value.duration,
        )
    };
    let Some(id) = id else { return Ok(changed) };
    let volume = if av_tracks::enabled(state, node, &format!("{id}:audio"), false) {
        volume
    } else {
        0.
    };
    let video_selected = av_tracks::enabled(state, node, &format!("{id}:video"), true);
    if paused && already && !seek {
        return Ok(changed);
    }
    let looping = state
        .dom
        .borrow()
        .attribute_in_null_namespace(node, "loop")?
        .is_some();
    let position = if looping && duration > 0. {
        time % duration
    } else {
        time.min(duration)
    };
    let crate::media_capture::Response::UrlFrame(frame) = native_platform::media_request(
        state,
        crate::media_capture::Request::UrlSample {
            id,
            time: position,
            paused,
            volume,
            seek: seek || position < time,
        },
    )?
    else {
        return Err(fail("NotReadableError"));
    };
    if let Some(data) = frame["data"].as_str().filter(|_| video_selected) {
        if data.len() > 24 * 1024 * 1024 {
            return Err(fail("QuotaExceededError"));
        }
        let bytes = BASE64.decode(data).map_err(|_| fail("NotReadableError"))?;
        let width = frame["width"]
            .as_u64()
            .and_then(|x| u32::try_from(x).ok())
            .ok_or(fail("NotReadableError"))?;
        let height = frame["height"]
            .as_u64()
            .and_then(|x| u32::try_from(x).ok())
            .ok_or(fail("NotReadableError"))?;
        if width == 0
            || height == 0
            || bytes.len() > 16 * 1024 * 1024
            || u64::from(width) * u64::from(height) * 4 != bytes.len() as u64
        {
            return Err(fail("NotReadableError"));
        }
        let mut png = Vec::new();
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&bytes, width, height, image::ExtendedColorType::Rgba8)
            .map_err(|_| fail("NotReadableError"))?;
        state.decoded_images.borrow_mut().insert(
            node,
            DecodedImage {
                origin_clean: state.native_media.borrow().playback[&node].origin_clean,
                width,
                height,
                rgba: bytes,
            },
        );
        state
            .dom
            .borrow_mut()
            .set_image_dimensions(node, Some((width, height)))?;
        state.dom.borrow_mut().set_media_frame(node, Some(png))?;
    }
    let mut media = state.native_media.borrow_mut();
    let value = media.playback.get_mut(&node).unwrap();
    value.seek_sampled = true;
    if seek {
        value.seeking = false;
        value.url_events.push_back("timeupdate");
        value.url_events.push_back("seeked");
    }
    if !paused {
        if value
            .last_timeupdate
            .is_none_or(|last| last.elapsed() >= std::time::Duration::from_millis(250))
        {
            value.last_timeupdate = Some(std::time::Instant::now());
            value.url_events.push_back("timeupdate");
        }
        value.time = position;
        value.anchor = Some(std::time::Instant::now());
        if position >= duration && !looping {
            value.ended = true;
            value.paused = true;
            value.anchor = None;
            value.url_events.push_back("timeupdate");
            value.url_events.push_back("pause");
            value.url_events.push_back("ended");
        }
    }
    drop(media);
    publish_state(state, node)?;
    Ok(changed + 1)
}

pub(super) fn publish_state(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    let flags = {
        let media = state.native_media.borrow();
        let value = media.playback.get(&node);
        value.map_or(2, |value| {
            (if value.paused { 2 } else { 4 })
                | (if value.seeking { 8 } else { 0 })
                | (if !value.paused && value.ready < 3 {
                    16
                } else {
                    0
                })
        })
    };
    let flags = flags
        | if media_resource::stalled(state, node) {
            32
        } else {
            0
        };
    state
        .dom
        .borrow_mut()
        .set_media_selector_state(node, flags)?;
    Ok(())
}

/// Invoker commands use the same native playback algorithms as authored calls.
pub(super) fn command(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    command: &str,
) -> Result<bool, V8BindingError> {
    if !state
        .dom
        .borrow()
        .is_element(node, "http://www.w3.org/1999/xhtml", "audio")
        && !state
            .dom
            .borrow()
            .is_element(node, "http://www.w3.org/1999/xhtml", "video")
    {
        return Ok(false);
    }
    ensure(state, node)?;
    match command {
        "play" | "pause" | "play-pause" => {
            let paused = state.native_media.borrow().playback[&node].paused;
            let operation = if command == "play-pause" {
                if paused { "play" } else { "pause" }
            } else {
                command
            };
            let _ = call(scope, state, node, operation)?;
        }
        "mute" | "unmute" | "toggle-muted" => {
            let old = state.native_media.borrow().playback[&node].muted;
            let muted = if command == "toggle-muted" {
                !old
            } else {
                command == "mute"
            };
            let value = v8::Boolean::new(scope, muted);
            volume(scope, state, node, "muted", value.into())?;
        }
        "seek-backward" | "seek-forward" => {
            let time = state.native_media.borrow().playback[&node].time
                + if command == "seek-forward" { 10. } else { -10. };
            let value = v8::Number::new(scope, time);
            seek(scope, state, node, value.into())?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub(super) fn suspend(state: &V8BindingState, suspended: bool) {
    let now = std::time::Instant::now();
    for value in state.native_media.borrow_mut().playback.values_mut() {
        if value.stream.is_some() {
            continue;
        }
        if suspended {
            if let Some(anchor) = value.anchor.take() {
                value.time = (value.time + now.saturating_duration_since(anchor).as_secs_f64())
                    .min(value.duration);
            }
        } else if value.asset.is_some() {
            value.seek_sampled = false;
            if !value.paused {
                value.anchor = Some(now);
            }
        }
    }
}

pub(super) fn picture_in_picture_asset(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<String, V8BindingError> {
    if !state
        .dom
        .borrow()
        .is_element(node, "http://www.w3.org/1999/xhtml", "video")
        || state.dom.borrow().node_document(node)? != Some(state.document_root())
    {
        return Err(fail("InvalidStateError"));
    }
    let media = state.native_media.borrow();
    let value = media.playback.get(&node).ok_or(fail("InvalidStateError"))?;
    if value.ready == 0 || value.width == 0 || value.height == 0 {
        return Err(fail("InvalidStateError"));
    }
    if let Some(asset) = &value.asset {
        return Ok(asset.clone());
    }
    let stream = value.stream.clone().ok_or(fail("InvalidStateError"))?;
    drop(media);
    let stream = v8::Local::new(scope, stream);
    native_media::stream_tracks(scope, state, stream)?
        .into_iter()
        .find(|track| track.kind == crate::media_capture::Kind::Video && !track.ended)
        .map(|track| track.id)
        .ok_or(fail("InvalidStateError"))
}
pub(super) fn picture_in_picture_refresh(state: &V8BindingState, node: NodeId) {
    if let Some(value) = state.native_media.borrow_mut().playback.get_mut(&node)
        && value.asset.is_some()
    {
        value.seek_sampled = false;
        value.url_events.push_back("__resolveplay");
    }
}

pub(super) struct EventTask {
    node: NodeId,
    session: u64,
    epoch: u64,
    name: &'static str,
    event: Option<retained_dom_event::Event>,
}
fn flush_events(state: &V8BindingState) -> Result<(), V8BindingError> {
    let mut media = state.native_media.borrow_mut();
    let nodes = media.playback.keys().copied().collect::<Vec<_>>();
    for node in nodes {
        while let Some(name) = media.playback[&node].url_events.front().copied() {
            if media.events.len() >= 512 {
                return Err(V8BindingError::Allocation("media event bound"));
            }
            let value = media.playback.get_mut(&node).unwrap();
            value.url_events.pop_front();
            let session = value.session;
            media.events.push_back(EventTask {
                node,
                session,
                epoch: state.document_stream_epoch(),
                name,
                event: None,
            });
        }
    }
    Ok(())
}
fn poll_events(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(usize, bool), V8BindingError> {
    let mut count = 0;
    for _ in 0..128 {
        if state.execution_suspended() || state.execution_retired() {
            break;
        }
        let Some(mut task) = state.native_media.borrow_mut().events.pop_front() else {
            break;
        };
        if task.epoch != state.document_stream_epoch() {
            continue;
        }
        let same = state
            .native_media
            .borrow()
            .playback
            .get(&task.node)
            .is_some_and(|value| value.session == task.session);
        if task.event.is_none() && !same {
            continue;
        }
        let result = (|| -> Result<bool, V8BindingError> {
            if task.name != "__resolveplay" {
                if task.event.is_none() {
                    let event = create_event_object(scope, state, task.name, false, false)?;
                    let id = event_id(scope, event)?;
                    if let Some(record) = state.events.borrow_mut().get_mut(&id) {
                        record.is_trusted = true;
                    }
                    task.event = Some(retained_dom_event::Event::new(
                        scope, state, task.node, task.name, event,
                    )?);
                }
                if task.event.as_mut().unwrap().poll(scope, state)?.is_none() {
                    return Ok(false);
                }
            }
            if matches!(task.name, "playing" | "__resolveplay") {
                loop {
                    let pending = {
                        let media = state.native_media.borrow();
                        media
                            .playback
                            .get(&task.node)
                            .filter(|value| {
                                value.session == task.session && !value.paused && value.ready >= 3
                            })
                            .and_then(|value| value.pending.first().cloned())
                    };
                    let Some(pending) = pending else { break };
                    let resolver = v8::Local::new(scope, pending);
                    let value = v8::undefined(scope);
                    resolver
                        .resolve(scope, value.into())
                        .ok_or(V8BindingError::JavaScriptException)?;
                    if let Some(value) = state
                        .native_media
                        .borrow_mut()
                        .playback
                        .get_mut(&task.node)
                        .filter(|value| value.session == task.session)
                        && !value.pending.is_empty()
                    {
                        value.pending.remove(0);
                    }
                }
            }
            Ok(true)
        })();
        match result {
            Ok(true) => {
                count += 1;
                if task.name == "seeking" {
                    return Ok((count, true));
                }
            }
            result => {
                state.native_media.borrow_mut().events.push_front(task);
                return result.map(|_| (count, false));
            }
        }
    }
    Ok((count, false))
}

#[cfg(test)]
mod url_tests {
    use super::super::native_user_activation::test_transport::Messages;
    use super::*;
    use std::sync::{Arc, Mutex};
    #[derive(Default)]
    struct Decoder {
        releases: Mutex<Vec<String>>,
        fail_release: Mutex<bool>,
        samples: Mutex<Vec<(f64, bool)>>,
        volumes: Mutex<Vec<f32>>,
    }
    impl crate::platform_control::Backend for Decoder {
        fn request(
            &self,
            _: crate::worker_wire::Creator,
            request: crate::platform_control::Request,
        ) -> Result<crate::platform_control::Response, String> {
            let crate::platform_control::Request::Media { request } = request else {
                return Err("unexpected request".into());
            };
            let response = match request {
                crate::media_capture::Request::UrlSample {
                    time,
                    paused,
                    volume,
                    ..
                } => {
                    self.samples.lock().unwrap().push((time, paused));
                    self.volumes.lock().unwrap().push(volume);
                    crate::media_capture::Response::UrlFrame(
                        serde_json::json!({"timestamp":time,"ended":false,"width":1,"height":1,"data":BASE64.encode([10,20,30,255])}),
                    )
                }
                crate::media_capture::Request::UrlRelease { id } => {
                    self.releases.lock().unwrap().push(id);
                    if *self.fail_release.lock().unwrap() {
                        return Err("transient release failure".into());
                    }
                    crate::media_capture::Response::Stopped
                }
                crate::media_capture::Request::UrlCanPlay { mime } => {
                    crate::media_capture::Response::UrlSupport(mime == "video/mp4")
                }
                _ => return Err("unexpected decoder request".into()),
            };
            Ok(crate::platform_control::Response::Media { response })
        }
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, text: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, text).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn adopted_media_cleanup_rejects_play_and_retains_failed_asset_release() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<video id=v></video>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let decoder = Arc::new(Decoder::default());
        let messages = Arc::new(Messages::new(
            state.document_generation(),
            state.document_authority_epoch(),
        ));
        state.set_message_transport(messages.clone());
        state.set_native_platform_backend(decoder.clone());
        let node = dom.borrow().find_by_id("v").unwrap();
        ensure(&state, node).unwrap();
        let resolver = v8::PromiseResolver::new(scope).unwrap();
        let promise = resolver.get_promise(scope);
        {
            let mut media = state.native_media.borrow_mut();
            let playback = media.playback.get_mut(&node).unwrap();
            playback.asset = Some("old-owner-asset".into());
            playback.pending.push(v8::Global::new(scope, resolver));
        }
        *decoder.fail_release.lock().unwrap() = true;
        eval(
            scope,
            "globalThis.oldVideo=document.getElementById('v');document.implementation.createHTMLDocument('').adoptNode(oldVideo)",
        );
        poll_one(scope, &state, node).unwrap();
        assert!(!state.native_media.borrow().playback.contains_key(&node));
        assert_eq!(promise.state(), v8::PromiseState::Rejected);
        assert!(state.native_media.borrow().resources.has_work());
        assert_eq!(
            decoder.releases.lock().unwrap().as_slice(),
            ["old-owner-asset"]
        );
        release_stale_owner(scope, &state, node).unwrap();
        assert_eq!(decoder.releases.lock().unwrap().len(), 1);
        *decoder.fail_release.lock().unwrap() = false;
        media_resource::poll(scope, &state).unwrap();
        assert_eq!(
            decoder.releases.lock().unwrap().as_slice(),
            ["old-owner-asset", "old-owner-asset"]
        );
        media_resource::poll(scope, &state).unwrap();
        assert_eq!(decoder.releases.lock().unwrap().len(), 2);
        messages.assert_lifecycle(0);
        state.retire_execution();
        messages.assert_lifecycle(1);
    }
    #[test]
    fn url_play_pause_seek_flags_promises_and_native_pixels_share_one_state() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<video id=v></video>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let decoder = Arc::new(Decoder::default());
        let messages = Arc::new(Messages::new(
            state.document_generation(),
            state.document_authority_epoch(),
        ));
        state.set_message_transport(messages.clone());
        state.set_native_platform_backend(decoder.clone());
        let node = dom.borrow().find_by_id("v").unwrap();
        assert!(eval(scope,"globalThis.v=document.getElementById('v');v.paused && v.matches(':paused') && !v.matches(':playing')").is_true());
        url_decoded(
            &state,
            node,
            &serde_json::json!({"id":"asset-1","duration":20.,"width":1,"height":1}),
            true,
        )
        .unwrap();
        assert!(eval(scope,"globalThis.settled=false;v.play().then(()=>settled=true);!v.paused && v.matches(':playing')").is_true());
        poll(scope, &state).unwrap();
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "settled").is_true());
        assert!(state.decoded_images.borrow()[&node].origin_clean);
        state.set_execution_suspended(true);
        let frozen = state.native_media.borrow().playback[&node].time;
        assert!(state.native_media.borrow().playback[&node].anchor.is_none());
        assert_eq!(state.native_media.borrow().playback[&node].time, frozen);
        state.set_execution_suspended(false);
        assert!(state.native_media.borrow().playback[&node].anchor.is_some());
        assert!(eval(scope,"globalThis.events=[];v.addEventListener('seeking',()=>events.push(v.matches(':seeking')));v.addEventListener('seeked',()=>events.push(!v.seeking));v.currentTime=5;v.seeking && v.matches(':seeking')").is_true());
        poll(scope, &state).unwrap();
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "v.seeking && events.join()==='true'").is_true());
        poll(scope, &state).unwrap();
        poll(scope, &state).unwrap();
        assert!(eval(scope, "!v.seeking && events.join()==='true,true'").is_true());
        assert!(eval(scope, "v.pause();v.paused && v.matches(':paused')").is_true());
        assert!(
            decoder
                .samples
                .lock()
                .unwrap()
                .iter()
                .any(|(_, paused)| *paused)
        );
        assert!(eval(scope, "v.load();v.paused && !v.seeking").is_true());
        assert_eq!(*decoder.releases.lock().unwrap(), vec!["asset-1"]);
        messages.assert_lifecycle(0);
        state.retire_execution();
        messages.assert_lifecycle(1);
    }
    #[test]
    fn pending_play_aborts_on_load_and_failed_decoder_release_is_retained() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<audio id=a></audio>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let decoder = Arc::new(Decoder::default());
        let messages = Arc::new(Messages::new(
            state.document_generation(),
            state.document_authority_epoch(),
        ));
        state.set_message_transport(messages.clone());
        state.set_native_platform_backend(decoder.clone());
        let node = dom.borrow().find_by_id("a").unwrap();
        eval(
            scope,
            "globalThis.a=document.getElementById('a');globalThis.failure='';a.play().catch(e=>failure=e.name);a.load()",
        );
        poll(scope, &state).unwrap();
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "failure==='AbortError' && a.paused").is_true());
        url_decoded(
            &state,
            node,
            &serde_json::json!({"id":"retained","duration":1.,"width":0,"height":0}),
            false,
        )
        .unwrap();
        *decoder.fail_release.lock().unwrap() = true;
        url_reset(&state, node).unwrap();
        assert!(state.native_media.borrow().resources.has_work());
        *decoder.fail_release.lock().unwrap() = false;
        media_resource::poll(scope, &state).unwrap();
        assert_eq!(
            *decoder.releases.lock().unwrap(),
            vec!["retained", "retained"]
        );
        let div = dom.borrow_mut().create_element("div");
        assert!(!command(scope, &state, div, "play").unwrap());
        assert!(command(scope, &state, node, "toggle-muted").unwrap());
        assert!(get(scope, &state, node, "muted").unwrap().is_true());
        messages.assert_lifecycle(0);
        state.retire_execution();
        messages.assert_lifecycle(1);
    }
    #[test]
    fn playing_listener_failure_retains_event_and_defers_play_promise() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<video id=v></video>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let messages = Arc::new(Messages::new(
            state.document_generation(),
            state.document_authority_epoch(),
        ));
        state.set_message_transport(messages.clone());
        state.set_native_platform_backend(Arc::new(Decoder::default()));
        let node = dom.borrow().find_by_id("v").unwrap();
        url_decoded(
            &state,
            node,
            &serde_json::json!({"id":"asset","duration":10.,"width":1,"height":1}),
            true,
        )
        .unwrap();
        eval(
            scope,
            "globalThis.log=[];globalThis.done=false;globalThis.v=document.getElementById('v');v.addEventListener('playing',e=>{log.push('first');globalThis.original=e},{once:true});v.addEventListener('playing',e=>log.push(e===original?'second':'wrong'));v.play().then(()=>{done=true;log.push('resolved')})",
        );
        native_platform::publication::fail_after_next_callback();
        assert!(poll(scope, &state).is_err());
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "log.join()==='first' && !done").is_true());
        assert!(state.native_media.borrow().playback[&node].error.is_none());
        state.set_execution_suspended(true);
        assert_eq!(poll(scope, &state).unwrap(), 0);
        state.set_execution_suspended(false);
        poll(scope, &state).unwrap();
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "log.join()==='first,second,resolved'").is_true());
        messages.assert_lifecycle(0);
        state.retire_execution();
        messages.assert_lifecycle(1);
    }

    #[test]
    fn native_audio_video_lists_select_real_output_and_retain_event_tracks() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<video id=v></video>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let decoder = Arc::new(Decoder::default());
        let messages = Arc::new(Messages::new(
            state.document_generation(),
            state.document_authority_epoch(),
        ));
        state.set_message_transport(messages.clone());
        state.set_native_platform_backend(decoder.clone());
        let node = dom.borrow().find_by_id("v").unwrap();
        assert!(eval(scope,"globalThis.v=document.getElementById('v');globalThis.audio=v.audioTracks;globalThis.video=v.videoTracks;globalThis.added=[];audio.onaddtrack=e=>added.push(e.track);audio.length===0&&video.selectedIndex===-1").is_true());
        url_decoded(
            &state,
            node,
            &serde_json::json!({"id":"av","duration":10.,"width":1,"height":1,"audio":true}),
            true,
        )
        .unwrap();
        av_tracks::refresh(scope, &state, node).unwrap();
        text_tracks::poll(scope, &state).unwrap();
        assert!(eval(scope,"audio===v.audioTracks&&video===v.videoTracks&&audio.length===1&&video.length===1&&audio[0] instanceof AudioTrack&&video[0] instanceof VideoTrack&&added[0]===audio[0]&&audio.getTrackById(audio[0].id)===audio[0]&&video.selectedIndex===0&&new TrackEvent('addtrack',{track:audio[0]}).track===audio[0]").is_true());
        eval(
            scope,
            "audio[0].enabled=false;video[0].selected=false;v.play()",
        );
        poll(scope, &state).unwrap();
        assert!(!state.decoded_images.borrow().contains_key(&node));
        assert_eq!(decoder.volumes.lock().unwrap().last(), Some(&0.));
        assert!(eval(scope, "video.selectedIndex===-1&&!audio[0].enabled").is_true());
        eval(scope, "video[0].selected=true");
        poll(scope, &state).unwrap();
        assert!(state.decoded_images.borrow().contains_key(&node));
        url_reset(&state, node).unwrap();
        av_tracks::refresh(scope, &state, node).unwrap();
        assert!(
            eval(
                scope,
                "audio.length===0&&video.length===0&&video.selectedIndex===-1"
            )
            .is_true()
        );
        messages.assert_lifecycle(0);
        state.retire_execution();
        messages.assert_lifecycle(1);
    }
}
