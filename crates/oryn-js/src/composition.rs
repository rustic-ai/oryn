//! Private native text-system commands. Never a script or WebDriver JSON API.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRange {
    pub start: u32,
    pub length: u32,
}
impl TextRange {
    pub fn end(self) -> Option<u32> {
        self.start.checked_add(self.length)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Operation {
    Read,
    Command(TextCommand),
    SetMarked {
        text: Vec<u16>,
        selected: TextRange,
        replacement: Option<TextRange>,
    },
    Commit {
        text: Vec<u16>,
        replacement: Option<TextRange>,
    },
    Unmark,
    Cancel,
    Finish {
        keep_marked: bool,
    },
}
impl Operation {
    pub fn validate(&self) -> bool {
        match self {
            Self::SetMarked {
                text,
                selected,
                replacement,
            } => {
                text.len() <= 1024 * 1024
                    && selected.end().is_some_and(|end| end as usize <= text.len())
                    && replacement.is_none_or(|range| range.end().is_some())
            }
            Self::Commit { text, replacement } => {
                text.len() <= 1024 * 1024 && replacement.is_none_or(|range| range.end().is_some())
            }
            _ => true,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub mutation_revision: u64,
    pub aborted: bool,
    pub protected: bool,
    pub text: Vec<u16>,
    pub selection: TextRange,
    pub marked: Option<TextRange>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_ranges_are_utf16_and_preserve_lone_surrogates() {
        let value = Operation::SetMarked {
            text: vec![0xd800, 0xd83d, 0xde00],
            selected: TextRange {
                start: 1,
                length: 2,
            },
            replacement: Some(TextRange {
                start: 4,
                length: 0,
            }),
        };
        assert!(value.validate());
        let encoded = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<Operation>(&encoded).unwrap(), value);
    }
    #[test]
    fn marked_selection_and_replacement_overflow_are_rejected() {
        assert!(
            !Operation::SetMarked {
                text: vec![97],
                selected: TextRange {
                    start: 1,
                    length: 1
                },
                replacement: None
            }
            .validate()
        );
        assert!(
            !Operation::Commit {
                text: vec![],
                replacement: Some(TextRange {
                    start: u32::MAX,
                    length: 1
                })
            }
            .validate()
        );
        assert!(
            !Operation::Commit {
                text: vec![0; 1024 * 1024 + 1],
                replacement: None
            }
            .validate()
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextUnit {
    Character,
    Word,
    Line,
    LineBoundary,
    Paragraph,
    ParagraphBoundary,
    DocumentBoundary,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextDirection {
    Left,
    Right,
    Backward,
    Forward,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextCommand {
    Tab {
        backward: bool,
    },
    Move {
        direction: TextDirection,
        unit: TextUnit,
        extend: bool,
    },
    Delete {
        backward: bool,
        unit: TextUnit,
    },
    Newline {
        line_break: bool,
    },
}
