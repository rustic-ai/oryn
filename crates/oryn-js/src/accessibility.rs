//! V8-free native assistive-technology text commands. These are never WebIDL or
//! author-script capabilities; native page ownership validates the target.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TextCommand {
    Read,
    Adjust {
        increase: bool,
    },
    Picker {
        expected: oryn_dom::TemporalPicker,
        action: PickerAction,
    },
    FilePicker {
        expected: oryn_dom::FilePicker,
        files: Option<Vec<PickerFile>>,
    },
    PermissionPrompt {
        expected: oryn_dom::PermissionPrompt,
        choice: crate::platform_control::PromptChoice,
    },
    Select {
        start: u32,
        end: u32,
        expected_value: Vec<u16>,
        expected_selection_revision: u64,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PickerAction {
    Choose { index: usize },
    Previous,
    Next,
    Clear,
    Accept,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextBox {
    pub start: u32,
    pub end: u32,
    pub line: usize,
    pub rect: oryn_common::v4::CssRect,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextCaret {
    pub offset: u32,
    pub x: f64,
    pub y: f64,
    pub line: usize,
    #[serde(default)]
    pub rect: Option<oryn_common::v4::CssRect>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextObject {
    pub offset: u32,
    pub node: oryn_dom::NodeId,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextSnapshot {
    #[serde(default)]
    pub objects: Vec<TextObject>,
    #[serde(default)]
    pub control: Option<oryn_dom::NodeId>,
    #[serde(default)]
    pub boundary_revision: u64,
    #[serde(default)]
    pub boundaries: Vec<SourceBoundary>,
    #[serde(default)]
    pub protected_caret: Option<ProtectedCaret>,
    pub value: Vec<u16>,
    pub start: u32,
    pub end: u32,
    pub selection_revision: u64,
    pub protected: bool,
    #[serde(default)]
    pub boxes: Vec<TextBox>,
    #[serde(default)]
    pub carets: Vec<TextCaret>,
}

impl TextSnapshot {
    /// Resident payload bound, including geometry and native source mappings.
    /// Capacity is counted because these snapshots are retained by the OS bridge.
    pub fn retained_bytes(&self) -> Option<usize> {
        let mut bytes = std::mem::size_of::<Self>();
        for (count, size) in [
            (self.value.capacity(), std::mem::size_of::<u16>()),
            (
                self.boundaries.capacity(),
                std::mem::size_of::<SourceBoundary>(),
            ),
            (self.objects.capacity(), std::mem::size_of::<TextObject>()),
            (self.boxes.capacity(), std::mem::size_of::<TextBox>()),
            (self.carets.capacity(), std::mem::size_of::<TextCaret>()),
        ] {
            bytes = bytes.checked_add(count.checked_mul(size)?)?;
        }
        Some(bytes)
    }
}
#[cfg(test)]
mod retained_snapshot_tests {
    use super::*;
    #[test]
    fn accounting_includes_unused_reserved_source_and_geometry_storage() {
        let mut snapshot = TextSnapshot {
            objects: Vec::new(),
            control: None,
            boundary_revision: 0,
            boundaries: Vec::new(),
            protected_caret: None,
            value: Vec::new(),
            start: 0,
            end: 0,
            selection_revision: 0,
            protected: false,
            boxes: Vec::new(),
            carets: Vec::new(),
        };
        let initial = snapshot.retained_bytes().unwrap();
        snapshot.boundaries.reserve(16);
        snapshot.carets.reserve(8);
        assert_eq!(
            snapshot.retained_bytes().unwrap() - initial,
            snapshot.boundaries.capacity() * std::mem::size_of::<SourceBoundary>()
                + snapshot.carets.capacity() * std::mem::size_of::<TextCaret>()
        );
    }
}

/// Move to an extended-grapheme boundary in the immutable native UTF-16 text.
/// Lossy decoding is used only for segmentation: each unmatched surrogate becomes
/// one U+FFFD code unit, so offsets still address the original, unmodified units.
/// A caller-provided index inside a surrogate pair or combining sequence moves
/// to the edge of that whole grapheme, never into its interior.
pub fn adjacent_grapheme(value: &[u16], offset: u32, forward: bool) -> Option<u32> {
    use unicode_segmentation::UnicodeSegmentation;
    let offset = usize::try_from(offset).ok()?;
    if offset > value.len() {
        return None;
    }
    let decoded = String::from_utf16_lossy(value);
    let mut position = 0usize;
    let mut previous = None;
    for grapheme in decoded.graphemes(true) {
        if !forward && position >= offset {
            return previous.and_then(|index| u32::try_from(index).ok());
        }
        if position < offset {
            previous = Some(position);
        }
        position = position.checked_add(grapheme.encode_utf16().count())?;
        if forward && position > offset {
            return u32::try_from(position).ok();
        }
    }
    if forward {
        None
    } else {
        previous.and_then(|index| u32::try_from(index).ok())
    }
}

#[cfg(test)]
mod marker_segmentation_tests {
    use super::adjacent_grapheme;
    #[test]
    fn movement_uses_extended_graphemes_and_utf16_offsets() {
        let text = "a\u{301}👩‍👩‍👧‍👦🇨🇦\r\nz".encode_utf16().collect::<Vec<_>>();
        let boundaries = [0, 2, 13, 17, 19, 20];
        for pair in boundaries.windows(2) {
            assert_eq!(adjacent_grapheme(&text, pair[0], true), Some(pair[1]));
            assert_eq!(adjacent_grapheme(&text, pair[1], false), Some(pair[0]));
            for inside in pair[0] + 1..pair[1] {
                assert_eq!(adjacent_grapheme(&text, inside, true), Some(pair[1]));
                assert_eq!(adjacent_grapheme(&text, inside, false), Some(pair[0]));
            }
        }
        assert_eq!(adjacent_grapheme(&text, 0, false), None);
        assert_eq!(adjacent_grapheme(&text, 20, true), None);
        assert_eq!(adjacent_grapheme(&text, 21, false), None);
    }
    #[test]
    fn unmatched_surrogates_do_not_shift_native_offsets() {
        let text = [0xd800, 97, 0xdc00, 0xd83d, 0xde00];
        for (start, end) in [(0, 1), (1, 2), (2, 3), (3, 5)] {
            assert_eq!(adjacent_grapheme(&text, start, true), Some(end));
            assert_eq!(adjacent_grapheme(&text, end, false), Some(start));
        }
        assert_eq!(adjacent_grapheme(&[], 0, true), None);
        assert_eq!(adjacent_grapheme(&[], 0, false), None);
    }
}

/// Native word navigation over Unicode word segments. This uses the pinned
/// Unicode segmenter; locale-specific dictionary segmentation is separate.
pub fn adjacent_word_boundary(value: &[u16], offset: u32, forward: bool) -> Option<u32> {
    use unicode_segmentation::UnicodeSegmentation;
    let offset = usize::try_from(offset).ok()?;
    if offset > value.len() {
        return None;
    }
    let decoded = String::from_utf16_lossy(value);
    let mut utf16 = 0usize;
    let mut previous = None;
    for segment in decoded.split_word_bounds() {
        let end = utf16.checked_add(segment.encode_utf16().count())?;
        if segment.unicode_words().next().is_some() {
            if forward && end > offset {
                return u32::try_from(end).ok();
            }
            if !forward && utf16 < offset {
                previous = u32::try_from(utf16).ok();
            }
        }
        utf16 = end;
        if !forward && utf16 >= offset {
            break;
        }
    }
    if forward { None } else { previous }
}

#[cfg(test)]
mod marker_word_tests {
    use super::adjacent_word_boundary;
    #[test]
    fn native_word_navigation_skips_punctuation_and_preserves_utf16() {
        let text = "😀 café, don't 42".encode_utf16().collect::<Vec<_>>();
        assert_eq!(adjacent_word_boundary(&text, 0, true), Some(7));
        assert_eq!(adjacent_word_boundary(&text, 7, true), Some(14));
        assert_eq!(adjacent_word_boundary(&text, 14, true), Some(17));
        assert_eq!(adjacent_word_boundary(&text, 17, false), Some(15));
        assert_eq!(adjacent_word_boundary(&text, 15, false), Some(9));
        assert_eq!(adjacent_word_boundary(&text, 9, false), Some(3));
        assert_eq!(adjacent_word_boundary(&text, 3, false), None);
        assert_eq!(adjacent_word_boundary(&text, 18, false), None);
        assert_eq!(adjacent_word_boundary(&text, 17, true), None);
    }
    #[test]
    fn malformed_utf16_and_combining_words_keep_original_indices() {
        let text = [0xd800, 0x61, 0x301, 0x20, 0x62, 0xdc00];
        assert_eq!(adjacent_word_boundary(&text, 0, true), Some(3));
        assert_eq!(adjacent_word_boundary(&text, 3, false), Some(1));
        assert_eq!(adjacent_word_boundary(&text, 3, true), Some(5));
        assert_eq!(adjacent_word_boundary(&text, 6, false), Some(4));
    }
}

/// The word touching an AX marker on its requested logical side. Whitespace
/// between the marker and the next/previous word yields an empty range at the
/// marker instead of unexpectedly jumping to a distant word.
pub fn touching_word_range(value: &[u16], offset: u32, right: bool) -> Option<(u32, u32)> {
    if usize::try_from(offset).ok()? > value.len() {
        return None;
    }
    let empty = (offset, offset);
    if right {
        let Some(end) = adjacent_word_boundary(value, offset, true) else {
            return Some(empty);
        };
        let Some(start) = adjacent_word_boundary(value, end, false) else {
            return Some(empty);
        };
        Some(if start <= offset { (start, end) } else { empty })
    } else {
        let Some(start) = adjacent_word_boundary(value, offset, false) else {
            return Some(empty);
        };
        let Some(end) = adjacent_word_boundary(value, start, true) else {
            return Some(empty);
        };
        Some(if end >= offset { (start, end) } else { empty })
    }
}
#[cfg(test)]
mod marker_word_range_tests {
    use super::touching_word_range;
    #[test]
    fn side_affinity_does_not_skip_whitespace() {
        let text = "one  two".encode_utf16().collect::<Vec<_>>();
        assert_eq!(touching_word_range(&text, 3, false), Some((0, 3)));
        assert_eq!(touching_word_range(&text, 3, true), Some((3, 3)));
        assert_eq!(touching_word_range(&text, 4, false), Some((4, 4)));
        assert_eq!(touching_word_range(&text, 4, true), Some((4, 4)));
        assert_eq!(touching_word_range(&text, 5, false), Some((5, 5)));
        assert_eq!(touching_word_range(&text, 5, true), Some((5, 8)));
        assert_eq!(touching_word_range(&text, 6, false), Some((5, 8)));
        assert_eq!(touching_word_range(&text, 6, true), Some((5, 8)));
        assert_eq!(touching_word_range(&text, 9, true), None);
        assert_eq!(touching_word_range(&[], 0, false), Some((0, 0)));
    }
}

/// Private OS text-input placement only. AX text/marker queries must not expose
/// this record, the protected selection, or source character counts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProtectedCaret {
    pub selection: crate::composition::TextRange,
    pub mutation_revision: u64,
    pub caret: TextCaret,
}
impl ProtectedCaret {
    pub fn matches(&self, state: &crate::composition::StateSnapshot) -> bool {
        state.protected
            && !state.aborted
            && state.text.is_empty()
            && state.mutation_revision == self.mutation_revision
            && state.selection == self.selection
    }
}

#[cfg(test)]
mod protected_placement_tests {
    use super::*;
    #[test]
    fn placement_requires_exact_native_revision_and_selection() {
        let selection = crate::composition::TextRange {
            start: 3,
            length: 0,
        };
        let record = ProtectedCaret {
            selection,
            mutation_revision: 9,
            caret: TextCaret {
                offset: 3,
                x: 12.,
                y: 4.,
                line: 0,
                rect: Some(oryn_common::v4::CssRect {
                    x: 12.,
                    y: 0.,
                    width: 0.,
                    height: 16.,
                }),
            },
        };
        let mut state = crate::composition::StateSnapshot {
            mutation_revision: 9,
            aborted: false,
            protected: true,
            text: Vec::new(),
            selection,
            marked: None,
        };
        assert!(record.matches(&state));
        state.mutation_revision = 10;
        assert!(!record.matches(&state));
        state.mutation_revision = 9;
        state.selection.start = 2;
        assert!(!record.matches(&state));
        state.selection = selection;
        state.aborted = true;
        assert!(!record.matches(&state));
        state.aborted = false;
        state.text = vec![97];
        assert!(!record.matches(&state));
    }
}

#[cfg(any(feature = "v8-host", test))]
#[path = "accessibility_resolved.rs"]
mod resolved;
#[cfg(feature = "v8-host")]
pub(crate) use resolved::resolved_nodes;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceBoundary {
    pub start: oryn_dom::BoundaryPoint,
    pub end: oryn_dom::BoundaryPoint,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickerFile {
    pub name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
    pub last_modified: i64,
}
