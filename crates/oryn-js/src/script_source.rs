//! Script text provenance is explicit at compilation. Network UTF-8 is never
//! decoded as the private DOM-string encoding, even if its bytes share a prefix.
use std::borrow::Cow;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptSource {
    PlainUtf8(String),
    DomUtf16(String),
}
#[derive(Clone, Copy, Debug)]
pub enum ScriptSourceRef<'a> {
    PlainUtf8(&'a str),
    DomUtf16(&'a str),
}
impl ScriptSource {
    pub fn plain(value: impl Into<String>) -> Self {
        Self::PlainUtf8(value.into())
    }
    pub fn dom(value: String) -> Self {
        Self::DomUtf16(value)
    }
    pub fn as_source(&self) -> ScriptSourceRef<'_> {
        match self {
            Self::PlainUtf8(text) => ScriptSourceRef::PlainUtf8(text),
            Self::DomUtf16(text) => ScriptSourceRef::DomUtf16(text),
        }
    }
    pub fn policy_text(&self) -> Cow<'_, str> {
        self.as_source().policy_text()
    }
    pub fn len(&self) -> usize {
        self.as_source().raw().len()
    }
    pub fn is_empty(&self) -> bool {
        self.as_source().raw().is_empty()
    }
}
impl From<String> for ScriptSource {
    fn from(text: String) -> Self {
        Self::PlainUtf8(text)
    }
}
impl From<&str> for ScriptSource {
    fn from(text: &str) -> Self {
        Self::plain(text)
    }
}
impl<'a> ScriptSourceRef<'a> {
    pub fn units(self) -> Vec<u16> {
        match self {
            Self::PlainUtf8(text) => text.encode_utf16().collect(),
            Self::DomUtf16(text) => oryn_dom::dom_string::units(text),
        }
    }
    pub fn policy_text(self) -> Cow<'a, str> {
        match self {
            Self::PlainUtf8(text) => Cow::Borrowed(text),
            Self::DomUtf16(text) => {
                Cow::Owned(String::from_utf16_lossy(&oryn_dom::dom_string::units(text)))
            }
        }
    }
    pub fn owned(self) -> ScriptSource {
        match self {
            Self::PlainUtf8(text) => ScriptSource::plain(text),
            Self::DomUtf16(text) => ScriptSource::dom(text.into()),
        }
    }
    pub fn raw(self) -> &'a str {
        match self {
            Self::PlainUtf8(text) | Self::DomUtf16(text) => text,
        }
    }
}
impl<'a> From<&'a ScriptSource> for ScriptSourceRef<'a> {
    fn from(source: &'a ScriptSource) -> Self {
        source.as_source()
    }
}
impl<'a> From<&'a str> for ScriptSourceRef<'a> {
    fn from(text: &'a str) -> Self {
        Self::PlainUtf8(text)
    }
}
impl<'a> From<&'a String> for ScriptSourceRef<'a> {
    fn from(text: &'a String) -> Self {
        Self::PlainUtf8(text)
    }
}

impl<'a> From<&ScriptSourceRef<'a>> for ScriptSourceRef<'a> {
    fn from(value: &ScriptSourceRef<'a>) -> Self {
        *value
    }
}
impl<'a> From<&'a &str> for ScriptSourceRef<'a> {
    fn from(value: &'a &str) -> Self {
        Self::PlainUtf8(value)
    }
}
impl<'a> From<&'a &String> for ScriptSourceRef<'a> {
    fn from(value: &'a &String) -> Self {
        Self::PlainUtf8(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fetched_reserved_prefix_and_dom_code_units_are_distinct() {
        let units = [0xd800, 0x61, 0xdc00];
        let encoded = oryn_dom::dom_string::from_units(&units);
        assert_eq!(
            ScriptSource::dom(encoded.clone()).as_source().units(),
            units
        );
        assert_eq!(
            ScriptSource::plain(encoded.clone()).as_source().units(),
            encoded.encode_utf16().collect::<Vec<_>>()
        );
        assert_ne!(
            ScriptSource::dom(encoded.clone()).as_source().units(),
            ScriptSource::plain(encoded).as_source().units()
        );
    }
}
