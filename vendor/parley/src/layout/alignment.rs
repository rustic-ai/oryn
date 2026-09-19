// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{
    BreakReason,
    data::{ClusterData, LineItemData},
};
use crate::data::LayoutData;
use crate::style::Brush;

/// Alignment of a layout.
#[derive(Copy, Clone, Default, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Alignment {
    /// This is [`Alignment::Left`] for LTR text and [`Alignment::Right`] for RTL text.
    #[default]
    Start,
    /// This is [`Alignment::Right`] for LTR text and [`Alignment::Left`] for RTL text.
    End,
    /// Align content to the left edge.
    ///
    /// For alignment that should be aware of text direction, use [`Alignment::Start`] or
    /// [`Alignment::End`] instead.
    Left,
    /// Align each line centered within the container.
    Center,
    /// Align content to the right edge.
    ///
    /// For alignment that should be aware of text direction, use [`Alignment::Start`] or
    /// [`Alignment::End`] instead.
    Right,
    /// Justify each line by spacing out content, except for the last line.
    Justify,
}

/// Additional options to fine tune alignment
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignmentOptions {
    /// Alignment for final lines and lines before a forced break. None uses the
    /// paragraph alignment, except that ordinary justification uses start.
    pub last_line: Option<Alignment>,
    /// If set to `true`, "end" and "center" alignment will apply even if the line contents are
    /// wider than the alignment width. If it is set to `false`, all overflowing lines will be
    /// [`Alignment::Start`] aligned.
    pub align_when_overflowing: bool,
}

#[expect(
    clippy::derivable_impls,
    reason = "Make default values explicit rather than relying on the implicit default value of bool"
)]
impl Default for AlignmentOptions {
    fn default() -> Self {
        Self {
            align_when_overflowing: false,
            last_line: None,
        }
    }
}

/// Align the layout.
///
/// If [`Alignment::Justify`] is requested, clusters' [`ClusterData::advance`] will be adjusted.
/// Prior to re-line-breaking or re-aligning, [`unjustify`] has to be called.
pub(crate) fn align<B: Brush>(
    layout: &mut LayoutData<B>,
    alignment: Alignment,
    options: AlignmentOptions,
) {
    #[cfg(feature = "accesskit")]
    {
        layout.alignment = Some(alignment);
    }
    layout.is_aligned_justified = alignment == Alignment::Justify
        || options.last_line == Some(Alignment::Justify);

    align_impl::<_, false>(layout, alignment, options);
}

/// Removes previous justification applied to clusters.
///
/// This is part of resetting state in preparation for re-line-breaking or re-aligning the same
/// layout.
pub(crate) fn unjustify<B: Brush>(layout: &mut LayoutData<B>) {
    if layout.is_aligned_justified {
        align_impl::<_, true>(layout, Alignment::Justify, AlignmentOptions::default());
        layout.is_aligned_justified = false;
    }
}

/// The actual alignment implementation.
///
/// This is const-generic over `UNDO_JUSTIFICATION`: justified alignment adjusts clusters'
/// [`ClusterData::advance`], and this mutation has to be undone for re-line-breaking or
/// re-aligning. `UNDO_JUSTIFICATION` indicates whether the adjustment has to be applied, or
/// undone.
///
/// Undo uses the recorded adjustment and space count so changing last-line
/// alignment does not alter which earlier adjustments are restored.
fn align_impl<B: Brush, const UNDO_JUSTIFICATION: bool>(
    layout: &mut LayoutData<B>,
    alignment: Alignment,
    options: AlignmentOptions,
) {
    // Whether the text base direction is right-to-left.
    let is_rtl = layout.base_level & 1 == 1;

    // Apply alignment to line items
    for line in &mut layout.lines {
        if UNDO_JUSTIFICATION {
            if line.justified_spaces != 0 {
                adjust_spaces(&mut layout.clusters, &layout.line_items[line.item_range.clone()],
                    is_rtl, -line.justification, line.justified_spaces);
                line.metrics.advance -= line.justification * line.justified_spaces as f32;
                line.justification = 0.;
                line.justified_spaces = 0;
            }
            continue;
        }
        let last_line = matches!(line.break_reason, BreakReason::None | BreakReason::Explicit);
        let alignment = if last_line {
            options.last_line.unwrap_or(if alignment == Alignment::Justify { Alignment::Start } else { alignment })
        } else { alignment };
        let indent = line.indent;

        if is_rtl {
            // In RTL text, trailing whitespace is on the left. As we hang that whitespace, offset
            // the line to the left. Note: indent is not subtracted here because `free_space` below
            // already accounts for it.
            line.metrics.offset = -line.metrics.trailing_whitespace;
        } else {
            line.metrics.offset = indent;
        }

        // Compute free space.
        let line_width = line.metrics.inline_max_coord - line.metrics.inline_min_coord;
        let free_space =
            line_width - indent - line.metrics.advance + line.metrics.trailing_whitespace;

        if !options.align_when_overflowing && free_space <= 0.0 {
            if is_rtl {
                // In RTL text, right-align on overflow.
                line.metrics.offset += free_space;
            }
            continue;
        }

        match (alignment, is_rtl) {
            (Alignment::Left, _) | (Alignment::Start, false) | (Alignment::End, true) => {
                // Do nothing
            }
            (Alignment::Right, _) | (Alignment::Start, true) | (Alignment::End, false) => {
                line.metrics.offset += free_space;
            }
            (Alignment::Center, _) => {
                line.metrics.offset += free_space * 0.5;
            }
            (Alignment::Justify, _) => {
                // Justified alignment doesn't have any effect if free_space is negative or zero
                if free_space <= 0.0 {
                    continue;
                }

                // Lines without eligible spacing opportunities use start alignment.
                if line.num_spaces == 0
                {
                    if is_rtl {
                        line.metrics.offset += free_space;
                    }
                    continue;
                }

                let adjustment = free_space / line.num_spaces as f32;
                let applied = adjust_spaces(&mut layout.clusters,
                    &layout.line_items[line.item_range.clone()], is_rtl, adjustment, line.num_spaces);
                line.justification = adjustment;
                line.justified_spaces = applied;
                line.metrics.advance += adjustment * applied as f32;
            }
        }
    }
    // Hit testing skips whole runs using these widths before examining their
    // clusters. Keep them synchronized after both applying and undoing spacing.
    for item in &mut layout.line_items {
        if item.is_text_run() { item.advance = item.resolved_advance(&layout.clusters); }
    }
    layout.width = 0.;
    layout.full_width = 0.;
    for line in &layout.lines {
        let end = line.metrics.inline_min_coord + line.metrics.advance + line.indent.max(0.);
        layout.full_width = layout.full_width.max(end);
        layout.width = layout.width.max(end - line.metrics.trailing_whitespace);
    }

}


/// Adjust exactly the selected spaces in visual traversal order. Recording the
/// count and amount lets re-alignment restore advances without reconstructing
/// the old CSS last-line policy from newly changed alignment options.
fn adjust_spaces(clusters: &mut [ClusterData], items: &[LineItemData], rtl: bool,
    adjustment: f32, limit: usize) -> usize {
    let mut applied = 0;
    let items: &mut dyn Iterator<Item = &LineItemData> = if rtl {
        &mut items.iter().rev()
    } else { &mut items.iter() };
    for item in items.filter(|item| item.is_text_run()) {
        let range = &mut clusters[item.cluster_range.clone()];
        let range: &mut dyn Iterator<Item = (usize, &mut ClusterData)> = if item.bidi_level & 1 != 0 {
            &mut range.iter_mut().enumerate().rev()
        } else { &mut range.iter_mut().enumerate() };
        for (offset, cluster) in range {
            if applied == limit { return applied; }
            let index = item.cluster_range.start + offset;
            if cluster.info.whitespace().is_space_or_nbsp() && !item.cluster_advances.contains_key(&index) {
                cluster.advance += adjustment;
                applied += 1;
            }
        }
    }
    applied
}
