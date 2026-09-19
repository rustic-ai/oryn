//! Native drag feedback values. No V8 handles or author-provided capabilities.
use oryn_common::v4::CssRect;
use oryn_dom::NodeId;
use serde::{Deserialize, Serialize};
pub const MAX_PIXELS: usize = 16_000_000;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
    pub bounds: CssRect,
    pub scale: f64,
    pub hotspot: (f64, f64),
}
impl Snapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > 4096
            || self.height > 4096
            || (self.width as usize)
                .checked_mul(self.height as usize)
                .is_none_or(|pixels| pixels > MAX_PIXELS)
            || self.png.len() > MAX_BYTES
            || !self.png.starts_with(b"\x89PNG\r\n\x1a\n")
            || self.png.get(12..16) != Some(b"IHDR".as_slice())
            || self.png.get(16..20) != Some(self.width.to_be_bytes().as_slice())
            || self.png.get(20..24) != Some(self.height.to_be_bytes().as_slice())
            || ![
                self.scale,
                self.bounds.x,
                self.bounds.y,
                self.bounds.width,
                self.bounds.height,
                self.hotspot.0,
                self.hotspot.1,
            ]
            .iter()
            .all(|v| v.is_finite())
            || self.scale <= 0.
            || self.bounds.width <= 0.
            || self.bounds.height <= 0.
        {
            return Err("invalid native drag image snapshot".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct Resource {
    pub node: NodeId,
    pub url: String,
    pub url_aliases: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
    pub svg: Option<Vec<u8>>,
}
#[derive(Debug, Clone)]
pub struct Capture {
    pub css_resources: Vec<oryn_common::v4::PaintImageResource>,
    pub document: NodeId,
    pub source: NodeId,
    pub generation: u64,
    pub epoch: u64,
    pub hotspot: (i32, i32),
    pub polygons: Vec<[f64; 8]>,
    pub resources: Vec<Resource>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feedback {
    pub realm: u64,
    pub generation: u64,
    pub epoch: u64,
    pub session: u64,
    pub sequence: u64,
    pub image: Option<Snapshot>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeedbackBatch {
    pub page_generation: u64,
    pub updates: Vec<Feedback>,
}
/// Parent-side feedback is private to the native page. Stale sequence numbers
/// cannot revive a cleared drag. Navigation/worker loss clears the whole owner.
#[derive(Debug, Default)]
pub struct Controller(std::sync::Mutex<std::collections::BTreeMap<u64, Feedback>>);
impl Controller {
    pub fn apply(&self, batch: FeedbackBatch, current_generation: u64) -> Result<(), String> {
        if batch.page_generation != current_generation {
            return Err("retired drag feedback page".into());
        }
        if batch.updates.len() > 64 {
            return Err("drag feedback batch limit".into());
        }
        let mut incoming_bytes = 0usize;
        for update in &batch.updates {
            if let Some(image) = &update.image {
                image.validate()?;
                incoming_bytes = incoming_bytes
                    .checked_add(image.png.len())
                    .ok_or("drag feedback byte overflow")?;
            }
        }
        if incoming_bytes > MAX_BYTES {
            return Err("drag feedback batch byte limit".into());
        }
        let mut state = self.0.lock().map_err(|_| "drag feedback lock poisoned")?;
        // Validate the resulting inventory before committing anything. Keep clear
        // tombstones so reordered updates cannot revive a retired presentation.
        let mut next = state.clone();
        for update in batch.updates {
            if next.get(&update.realm).is_some_and(|previous| {
                (previous.generation, previous.epoch, previous.sequence)
                    >= (update.generation, update.epoch, update.sequence)
            }) {
                continue;
            }
            next.insert(update.realm, update);
        }
        if next.len() > 256 {
            return Err("drag feedback realm limit".into());
        }
        let retained = next
            .values()
            .filter_map(|value| value.image.as_ref())
            .try_fold(0usize, |sum, image| sum.checked_add(image.png.len()))
            .ok_or("drag feedback byte overflow")?;
        if retained > MAX_BYTES {
            return Err("drag feedback retained byte limit".into());
        }
        *state = next;
        Ok(())
    }
    pub fn current(&self) -> Vec<Feedback> {
        self.0
            .lock()
            .map(|state| {
                state
                    .values()
                    .filter(|update| update.image.is_some())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn clear(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image() -> Snapshot {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&1u32.to_be_bytes());
        png.extend_from_slice(&1u32.to_be_bytes());
        Snapshot {
            width: 1,
            height: 1,
            png,
            bounds: CssRect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
            scale: 1.,
            hotspot: (0., 0.),
        }
    }
    fn update(sequence: u64, image: Option<Snapshot>) -> Feedback {
        Feedback {
            realm: 3,
            generation: 1,
            epoch: 1,
            session: 1,
            sequence,
            image,
        }
    }
    fn batch(updates: Vec<Feedback>) -> FeedbackBatch {
        FeedbackBatch {
            page_generation: 9,
            updates,
        }
    }
    #[test]
    fn stale_image_cannot_revive_cleared_session() {
        let owner = Controller::default();
        owner
            .apply(batch(vec![update(1, Some(image())), update(2, None)]), 9)
            .unwrap();
        owner
            .apply(batch(vec![update(1, Some(image()))]), 9)
            .unwrap();
        assert!(owner.current().is_empty());
    }
    #[test]
    fn invalid_batch_is_atomic() {
        let owner = Controller::default();
        let mut invalid = image();
        invalid.bounds.width = -1.;
        assert!(
            owner
                .apply(
                    batch(vec![update(1, Some(image())), update(2, Some(invalid))]),
                    9
                )
                .is_err()
        );
        assert!(owner.current().is_empty());
    }
    #[test]
    fn root_generation_and_png_dimensions_are_checked() {
        let owner = Controller::default();
        assert!(
            owner
                .apply(batch(vec![update(1, Some(image()))]), 10)
                .is_err()
        );
        let mut invalid = image();
        invalid.width = 2;
        assert!(invalid.validate().is_err());
    }
    #[test]
    fn newer_document_epoch_accepts_restarted_sequence() {
        let owner = Controller::default();
        owner.apply(batch(vec![update(8, None)]), 9).unwrap();
        let mut next = update(1, Some(image()));
        next.epoch = 2;
        owner.apply(batch(vec![next]), 9).unwrap();
        assert_eq!(owner.current().len(), 1);
        owner.clear();
        assert!(owner.current().is_empty());
    }
}
