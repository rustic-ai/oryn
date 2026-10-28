//! Captured document fonts for Canvas, retaining admission through shaping.
use crate::{LayoutLimits, WebFont, cssom::native_import_graph::ImportAdmission};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub type FontInput = (String, Vec<u8>, String, bool, bool);
pub struct Prepared {
    pub fonts: Vec<FontInput>,
    // Drop decoded bytes and metadata before releasing their owning charges.
    _allocations: Vec<Arc<dyn Send + Sync>>,
}

pub fn prepare(
    captured: &[WebFont],
    limits: &LayoutLimits,
    admission: &dyn ImportAdmission,
) -> Result<Prepared, String> {
    admission.checkpoint()?;
    let metadata = captured
        .len()
        .checked_mul(std::mem::size_of::<FontInput>() + std::mem::size_of::<Arc<dyn Send + Sync>>())
        .and_then(|n| n.checked_add(std::mem::size_of::<Arc<dyn Send + Sync>>()))
        .ok_or("Canvas font metadata overflow")?;
    let metadata_charge = admission.reserve(metadata)?;
    let mut allocations = Vec::new();
    // Locals drop in reverse order on failure, unlike struct fields.
    let mut fonts = Vec::new();
    fonts
        .try_reserve_exact(captured.len())
        .map_err(|e| e.to_string())?;
    allocations
        .try_reserve_exact(
            captured
                .len()
                .checked_add(1)
                .ok_or("Canvas font lease overflow")?,
        )
        .map_err(|e| e.to_string())?;
    if fonts.capacity() > captured.len() || allocations.capacity() > captured.len() + 1 {
        return Err("Canvas font containers exceeded admission".into());
    }
    allocations.push(metadata_charge);
    let mut total = 0usize;
    for font in captured {
        admission.checkpoint()?;
        let bound = if font.pending {
            0
        } else {
            if font.bytes.len() > limits.max_font_bytes {
                return Err("Canvas font source limit exceeded".into());
            }
            if format!("{:x}", Sha256::digest(&font.bytes)) != font.sha256 {
                return Err("Canvas font source hash mismatch".into());
            }
            // The decoder verifies the compressed header, but may allocate
            // up to its effective limit; admit that limit before calling it.
            if matches!(font.bytes.get(..4), Some(b"wOFF") | Some(b"wOF2")) {
                limits.max_font_bytes
            } else {
                font.bytes.len()
            }
        };
        let bytes = bound
            .checked_add(font.family.len())
            .and_then(|n| n.checked_add(font.descriptors.len()))
            .ok_or("Canvas font allocation overflow")?;
        let charge = admission.reserve(bytes)?;
        admission.checkpoint()?;
        let copy = |text: &str| -> Result<String, String> {
            let mut result = String::new();
            result
                .try_reserve_exact(text.len())
                .map_err(|e| e.to_string())?;
            if result.capacity() > text.len() {
                return Err("Canvas font string exceeded admission".into());
            }
            result.push_str(text);
            Ok(result)
        };
        let family = copy(&font.family)?;
        let descriptors = copy(&font.descriptors)?;
        let decoded = if font.pending {
            Vec::new()
        } else {
            crate::decode_web_font(&font.bytes, limits.max_font_bytes)?
        };
        if decoded.capacity() > bound {
            return Err("Canvas decoded font exceeded admission".into());
        }
        total = total
            .checked_add(decoded.len())
            .ok_or("Canvas font page budget overflow")?;
        if total > limits.max_page_font_bytes {
            return Err("Canvas decoded font page limit exceeded".into());
        }
        // Replace a conservative compressed decode reservation only after the
        // exact exported storage has its own charge; never create a gap.
        let retained = if decoded.capacity() < bound {
            let exact = decoded
                .capacity()
                .checked_add(family.capacity())
                .and_then(|n| n.checked_add(descriptors.capacity()))
                .ok_or("Canvas font retained allocation overflow")?;
            let replacement = admission.reserve(exact)?;
            drop(charge);
            replacement
        } else {
            charge
        };
        fonts.push((family, decoded, descriptors, font.pending, font.blocked));
        allocations.push(retained);
        admission.checkpoint()?;
    }
    admission.checkpoint()?;
    Ok(Prepared {
        fonts,
        _allocations: allocations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Charge(Arc<AtomicUsize>);
    impl Drop for Charge {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    struct Admission {
        live: Arc<AtomicUsize>,
        checks: AtomicUsize,
        deny_at: usize,
        reserves: AtomicUsize,
        deny_reserve: usize,
    }
    impl ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            if self.checks.fetch_add(1, Ordering::SeqCst) >= self.deny_at {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        }
        fn reserve(&self, _: usize) -> Result<Arc<dyn Send + Sync>, String> {
            if self.reserves.fetch_add(1, Ordering::SeqCst) == self.deny_reserve {
                return Err("allocation denied".into());
            }
            self.live.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Charge(self.live.clone())))
        }
    }
    fn admission(deny_at: usize) -> Admission {
        Admission {
            live: Arc::new(AtomicUsize::new(0)),
            checks: AtomicUsize::new(0),
            deny_at,
            reserves: AtomicUsize::new(0),
            deny_reserve: usize::MAX,
        }
    }
    fn font() -> WebFont {
        WebFont {
            family: "Captured".into(),
            source_url: "https://example.test/font".into(),
            sha256: format!("{:x}", Sha256::digest(crate::DEFAULT_FONT_BYTES)),
            bytes: crate::DEFAULT_FONT_BYTES.to_vec(),
            descriptors: "font-weight:400;unicode-range:U+0-10FFFF".into(),
            pending: false,
            blocked: false,
            order: 0,
        }
    }
    // WOFF1 permits uncompressed tables. Wrap the bundled native SFNT without
    // adding an external fixture path or a second font source to the tests.
    fn woff() -> Vec<u8> {
        let sfnt = crate::DEFAULT_FONT_BYTES;
        let tables = u16::from_be_bytes(sfnt[4..6].try_into().unwrap()) as usize;
        let mut output = vec![0; 44 + tables * 20];
        output[..4].copy_from_slice(b"wOFF");
        output[4..8].copy_from_slice(&sfnt[..4]);
        output[12..14].copy_from_slice(&(tables as u16).to_be_bytes());
        let mut total_sfnt = 12 + tables * 16;
        for table in 0..tables {
            let row = &sfnt[12 + table * 16..28 + table * 16];
            let offset = u32::from_be_bytes(row[8..12].try_into().unwrap()) as usize;
            let length = u32::from_be_bytes(row[12..16].try_into().unwrap()) as usize;
            let destination = 44 + table * 20;
            let position = output.len() as u32;
            output[destination..destination + 4].copy_from_slice(&row[..4]);
            output[destination + 4..destination + 8].copy_from_slice(&position.to_be_bytes());
            output[destination + 8..destination + 12]
                .copy_from_slice(&(length as u32).to_be_bytes());
            output[destination + 12..destination + 16]
                .copy_from_slice(&(length as u32).to_be_bytes());
            output[destination + 16..destination + 20].copy_from_slice(&row[4..8]);
            output.extend_from_slice(&sfnt[offset..offset + length]);
            while output.len() % 4 != 0 {
                output.push(0);
            }
            total_sfnt += (length + 3) & !3;
        }
        let length = output.len() as u32;
        output[8..12].copy_from_slice(&length.to_be_bytes());
        output[16..20].copy_from_slice(&(total_sfnt as u32).to_be_bytes());
        output
    }
    #[test]
    fn canvas_font_inputs_replace_compressed_decode_charge_and_count_actual_page_bytes() {
        let mut compressed = font();
        compressed.bytes = woff();
        compressed.sha256 = format!("{:x}", Sha256::digest(&compressed.bytes));
        let decoded =
            crate::decode_web_font(&compressed.bytes, LayoutLimits::default().max_font_bytes)
                .unwrap();
        let mut limits = LayoutLimits::default();
        limits.max_page_font_bytes = decoded.len() * 5;
        let captured = vec![compressed; 5];
        let admitted = admission(usize::MAX);
        let prepared = prepare(&captured, &limits, &admitted).unwrap();
        assert_eq!(prepared.fonts.len(), 5);
        assert_eq!(prepared.fonts[0].1, decoded);
        assert_eq!(admitted.live.load(Ordering::SeqCst), 6);
        let stages = admitted.checks.load(Ordering::SeqCst);
        let reservations = admitted.reserves.load(Ordering::SeqCst);
        assert_eq!(
            reservations, 11,
            "metadata plus scratch and replacement for each compressed font"
        );
        drop(prepared);
        assert_eq!(admitted.live.load(Ordering::SeqCst), 0);
        for deny_at in 0..stages {
            let cancelled = admission(deny_at);
            assert!(prepare(&captured, &limits, &cancelled).is_err());
            assert_eq!(cancelled.live.load(Ordering::SeqCst), 0);
        }
        for deny_reserve in 0..reservations {
            let denied = Admission {
                deny_reserve,
                ..admission(usize::MAX)
            };
            assert!(
                prepare(&captured, &limits, &denied).is_err(),
                "reservation {deny_reserve}"
            );
            assert_eq!(
                denied.live.load(Ordering::SeqCst),
                0,
                "reservation {deny_reserve} leaked charge"
            );
        }
        limits.max_page_font_bytes -= 1;
        assert!(prepare(&captured, &limits, &admission(usize::MAX)).is_err());
    }
    #[test]
    fn canvas_font_inputs_keep_original_bytes_pending_state_and_cancelled_charge_custody() {
        let loaded = font();
        let pending = WebFont {
            pending: true,
            blocked: true,
            bytes: vec![],
            sha256: String::new(),
            ..loaded.clone()
        };
        let captured = [loaded.clone(), pending];
        let admitted = admission(usize::MAX);
        let prepared = prepare(&captured, &LayoutLimits::default(), &admitted).unwrap();
        assert_eq!(prepared.fonts[0].1, loaded.bytes);
        assert_eq!(prepared.fonts[0].2, loaded.descriptors);
        let style = std::collections::BTreeMap::from([
            ("font-family".into(), "Captured".into()),
            ("font-size".into(), "20px".into()),
        ]);
        let shaped =
            crate::canvas_text::shape("Ag", &style, Default::default(), &prepared.fonts[..1])
                .unwrap();
        assert!(shaped.width > 0.);
        assert!(
            shaped
                .fonts
                .values()
                .any(|resource| resource.bytes == loaded.bytes)
        );
        assert!(prepared.fonts[1].1.is_empty());
        assert_eq!((prepared.fonts[1].3, prepared.fonts[1].4), (true, true));
        assert_eq!(admitted.live.load(Ordering::SeqCst), 3);
        let stages = admitted.checks.load(Ordering::SeqCst);
        drop(prepared);
        assert_eq!(admitted.live.load(Ordering::SeqCst), 0);
        for deny_at in 0..stages {
            let denied = admission(deny_at);
            assert!(prepare(&captured, &LayoutLimits::default(), &denied).is_err());
            assert_eq!(denied.live.load(Ordering::SeqCst), 0);
        }
    }
    #[test]
    fn canvas_font_inputs_reject_invalid_sources_and_limits_without_fallback() {
        let mut source = font();
        source.sha256.clear();
        assert!(prepare(&[source], &LayoutLimits::default(), &admission(usize::MAX)).is_err());
        let mut source = font();
        source.bytes = b"invalid native font".to_vec();
        source.sha256 = format!("{:x}", Sha256::digest(&source.bytes));
        assert!(prepare(&[source], &LayoutLimits::default(), &admission(usize::MAX)).is_err());
        let mut limits = LayoutLimits::default();
        limits.max_font_bytes = 1;
        assert!(prepare(&[font()], &limits, &admission(usize::MAX)).is_err());
        limits = LayoutLimits::default();
        limits.max_page_font_bytes = 1;
        assert!(prepare(&[font()], &limits, &admission(usize::MAX)).is_err());
    }
}
