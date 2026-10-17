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
    SetMarkedWithFormats {
        text: Vec<u16>,
        selected: TextRange,
        replacement: Option<TextRange>,
        formats: Vec<NativeTextFormat>,
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
    pub fn into_text_operation(self) -> (Self, Option<Vec<NativeTextFormat>>) {
        match self {
            Self::SetMarkedWithFormats { text, selected, replacement, formats } =>
                (Self::SetMarked { text, selected, replacement }, Some(formats)),
            operation => (operation, None),
        }
    }
    pub fn copy_bytes(&self)->Option<usize>{
        let (text,formats)=match self {
            Self::SetMarked{text,..}|Self::Commit{text,..}=>(text.len(),0),
            Self::SetMarkedWithFormats{text,formats,..}=>(text.len(),formats.len()),
            _=>(0,0),
        };
        text.checked_mul(std::mem::size_of::<u16>())?
            .checked_add(formats.checked_mul(std::mem::size_of::<NativeTextFormat>())?)?
            .checked_add(std::mem::size_of::<Self>())
    }
    pub fn validate(&self) -> bool {
        match self {
            Self::SetMarkedWithFormats { text, selected, replacement, formats } => {
                text.len() <= 1024 * 1024
                    && selected.end().is_some_and(|end| end as usize <= text.len())
                    && replacement.is_none_or(|range| range.end().is_some())
                    && formats.len() <= text.len()
                    && formats.iter().all(|format| format.valid_for(text.len()))
                    && formats.windows(2).all(|pair| pair[0].range.end().is_some_and(|end| end <= pair[1].range.start))
            }
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

// Native marked-text metadata. Not an author dictionary or inferred underline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NativeUnderlineStyle { None, Solid, Dotted, Dashed }
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NativeUnderlineThickness { None, Thin, Thick }
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NativeTextFormat {
    pub range: TextRange,
    pub style: NativeUnderlineStyle,
    pub thickness: NativeUnderlineThickness,
}
impl NativeTextFormat {
    pub fn from_cocoa_underline(range:TextRange, bits:u32)->Option<Self>{
        // AppKit's NSUnderlineStyle uses low-nibble thickness and pattern bits.
        // Double/by-word/dash-dot have no lossless EditContext representation.
        let thickness=match bits&0xff {0=>NativeUnderlineThickness::None,1=>NativeUnderlineThickness::Thin,2=>NativeUnderlineThickness::Thick,_=>return None};
        if bits&!0x703 !=0{return None}
        let style=if thickness==NativeUnderlineThickness::None {NativeUnderlineStyle::None} else {
            match bits&0x700 {0=>NativeUnderlineStyle::Solid,0x100=>NativeUnderlineStyle::Dotted,0x200=>NativeUnderlineStyle::Dashed,_=>return None}
        };
        Some(Self{range,style,thickness})
    }
    pub fn valid_for(self,text_len:usize)->bool {
        self.range.length!=0&&self.range.end().is_some_and(|end|end as usize<=text_len)
    }
}
#[cfg(test)]
mod native_format_controls {
    use super::*;
    #[test]
    fn cocoa_only_maps_representable_explicit_underline_metadata(){
        let range=TextRange{start:1,length:2};
        let thin=NativeTextFormat::from_cocoa_underline(range,0x101).unwrap();
        assert_eq!(thin.style,NativeUnderlineStyle::Dotted);
        assert_eq!(thin.thickness,NativeUnderlineThickness::Thin);
        assert!(thin.valid_for(3));assert!(!thin.valid_for(2));
        assert!(NativeTextFormat::from_cocoa_underline(range,9).is_none());
        assert!(NativeTextFormat::from_cocoa_underline(range,0x301).is_none());
        assert!(NativeTextFormat::from_cocoa_underline(range,0x8001).is_none());
        assert_eq!(NativeTextFormat::from_cocoa_underline(range,0).unwrap().style,NativeUnderlineStyle::None);
    }
    #[test]
    fn formatted_operation_normalization_preserves_original_wire_and_utf16_ranges(){
        let format=NativeTextFormat::from_cocoa_underline(TextRange{start:1,length:2},1).unwrap();
        let operation=Operation::SetMarkedWithFormats{text:vec![0xd800,0xd83d,0xde00],selected:TextRange{start:3,length:0},replacement:Some(TextRange{start:7,length:1}),formats:vec![format]};
        assert!(operation.validate());let bytes=operation.copy_bytes().unwrap();
        let encoded=serde_json::to_string(&operation).unwrap();let decoded:Operation=serde_json::from_str(&encoded).unwrap();assert_eq!(decoded,operation);
        let (plain,formats)=decoded.into_text_operation();assert_eq!(formats.unwrap(),vec![format]);
        assert_eq!(plain,Operation::SetMarked{text:vec![0xd800,0xd83d,0xde00],selected:TextRange{start:3,length:0},replacement:Some(TextRange{start:7,length:1})});
        assert!(bytes>=3*2+std::mem::size_of::<NativeTextFormat>());
        let bad=Operation::SetMarkedWithFormats{text:vec![97],selected:TextRange{start:0,length:0},replacement:None,formats:vec![format]};assert!(!bad.validate());
    }
}
