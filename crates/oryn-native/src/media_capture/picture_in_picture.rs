//! Bounded decoded PiP surfaces. Actor threads publish pixels; AppKit consumes
//! snapshots on its existing main thread. No media actor calls window APIs.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};
#[derive(Clone)]
pub(crate) struct Surface {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub pixels: Arc<[u8]>,
    pub revision: u64,
    pub visible: bool,
}
#[derive(Default)]
struct Surfaces {
    entries: BTreeMap<String, Surface>,
}
fn surfaces() -> &'static Mutex<Surfaces> {
    static VALUE: OnceLock<Mutex<Surfaces>> = OnceLock::new();
    VALUE.get_or_init(Default::default)
}
pub(super) fn open(id: &str, width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("InvalidStateError: no video track".into());
    }
    let mut values = surfaces()
        .lock()
        .map_err(|_| "InvalidStateError: PiP surface lock")?;
    if values.entries.contains_key(id) {
        return Err("InvalidStateError: PiP surface identity already exists".into());
    }
    if values.entries.len() >= 16 {
        return Err("QuotaExceededError: PiP surfaces".into());
    }
    values.entries.insert(
        id.into(),
        Surface {
            id: id.into(),
            width: width.min(640),
            height: ((height as f64 * (width.min(640) as f64 / width as f64)).round() as u32)
                .max(1),
            pixel_width: width,
            pixel_height: height,
            pixels: Arc::from([]),
            revision: 0,
            visible: true,
        },
    );
    Ok(())
}
pub(super) fn publish(id: &str, sample: &serde_json::Value) -> Result<(), String> {
    use base64::Engine;
    if !surfaces()
        .lock()
        .map_err(|_| "InvalidStateError: PiP surface lock")?
        .entries
        .contains_key(id)
    {
        return Ok(());
    }
    let Some(encoded) = sample["data"].as_str() else {
        return Ok(());
    };
    let width = sample["width"]
        .as_u64()
        .ok_or("InvalidStateError: PiP width")?;
    let height = sample["height"]
        .as_u64()
        .ok_or("InvalidStateError: PiP height")?;
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || width * height * 4 > 16 * 1024 * 1024
    {
        return Err("QuotaExceededError: PiP pixels".into());
    }
    let expected = width as usize * height as usize * 4;
    if encoded.len() > expected.div_ceil(3) * 4 {
        return Err("QuotaExceededError: PiP encoded pixels".into());
    }
    let pixels = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "InvalidStateError: PiP pixels")?;
    if pixels.len() != width as usize * height as usize * 4 {
        return Err("InvalidStateError: PiP pixel length".into());
    }
    let mut values = surfaces()
        .lock()
        .map_err(|_| "InvalidStateError: PiP surface lock")?;
    if let Some(surface) = values.entries.get_mut(id) {
        surface.pixels = pixels.into();
        surface.revision = surface.revision.wrapping_add(1);
        surface.pixel_width = width as u32;
        surface.pixel_height = height as u32;
    }
    Ok(())
}
pub(super) fn close(id: &str) {
    surfaces()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .remove(id);
}
pub(super) fn visible(id: &str, value: bool) {
    if let Some(surface) = surfaces()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .get_mut(id)
    {
        surface.visible = value;
    }
}
pub(super) fn state(id: &str) -> Option<(u32, u32)> {
    surfaces()
        .lock()
        .ok()?
        .entries
        .get(id)
        .map(|surface| (surface.width, surface.height))
}
pub(crate) fn snapshots() -> Vec<Surface> {
    surfaces()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .values()
        .cloned()
        .collect()
}
pub(crate) fn user_closed(id: &str) {
    close(id)
}

pub(crate) fn resized(id: &str, width: u32, height: u32) {
    if let Some(surface) = surfaces()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .get_mut(id)
    {
        surface.width = width.max(1);
        surface.height = height.max(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoded_surface_rejects_malformed_pixels_and_preserves_close_identity() {
        use base64::Engine;
        let id = super::super::id().unwrap();
        assert!(open(&id, 0, 1).is_err());
        open(&id, 2, 1).unwrap();
        assert!(
            publish(
                &id,
                &serde_json::json!({"width":2,"height":1,"data":"AAAA"})
            )
            .is_err()
        );
        publish(&id,&serde_json::json!({"width":2,"height":1,"data":base64::engine::general_purpose::STANDARD.encode([0u8;8])})).unwrap();
        let surface = snapshots()
            .into_iter()
            .find(|surface| surface.id == id)
            .unwrap();
        assert_eq!(surface.pixels.len(), 8);
        visible(&id, false);
        assert!(
            !snapshots()
                .into_iter()
                .find(|surface| surface.id == id)
                .unwrap()
                .visible
        );
        resized(&id, 400, 200);
        assert_eq!(state(&id), Some((400, 200)));
        user_closed(&id);
        assert_eq!(state(&id), None);
        publish(&id, &serde_json::json!({})).unwrap();
        assert_eq!(state(&id), None);
    }
}

#[cfg(test)]
mod lifetime_tests {
    use super::*;
    #[test]
    fn resize_and_late_publication_cannot_replace_or_resurrect_native_surface() {
        let id = super::super::id().unwrap();
        open(&id, 1, 1).unwrap();
        assert!(open(&id, 2, 2).is_err());
        assert!(
            publish(
                &id,
                &serde_json::json!({"width":1,"height":1,"data":"A".repeat(128)})
            )
            .is_err()
        );
        assert_eq!(state(&id), Some((1, 1)));
        resized(&id, 500, 240);
        assert_eq!(state(&id), Some((500, 240)));
        visible(&id, false);
        user_closed(&id);
        resized(&id, 300, 200);
        visible(&id, true);
        publish(
            &id,
            &serde_json::json!({"width":u64::MAX,"height":u64::MAX,"data":"bad"}),
        )
        .unwrap();
        assert_eq!(state(&id), None);
        assert!(!snapshots().iter().any(|surface| surface.id == id));
    }
}
