// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use fontique::Collection;

use fontique::SourceCache;

/// A font database/cache (wrapper around a Fontique [`Collection`] and [`SourceCache`]).
///
/// This type is designed to be a global resource with only one per-application (or per-thread).
#[derive(Default, Clone)]
pub struct FontContext {
    pub collection: Collection,
    pub source_cache: SourceCache,
    /// CSS descriptors attached to a registered face, applied only after fallback selects it.
    pub face_overrides: alloc::vec::Vec<(fontique::FamilyId,FontFaceOverrides)>,
}

impl FontContext {
    /// Create a new `FontContext`, discovering system fonts if available.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Per-face shaping descriptors. Percentages are represented as em fractions.
#[derive(Clone,Debug,Default)]
pub struct FontFaceOverrides {
    pub features:alloc::vec::Vec<crate::FontFeature>,
    pub variations:alloc::vec::Vec<crate::FontVariation>,
    pub axis_limits:alloc::vec::Vec<([u8;4],f32,f32)>,
    pub ascent:Option<f32>,
    pub descent:Option<f32>,
    pub line_gap:Option<f32>,
    pub size_adjust:Option<f32>,
    pub language:Option<[u8;4]>,
}
