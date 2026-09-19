//! Lossless JSON strings for WebDriver: UTF-16 code units, including lone surrogates.
use super::ScriptError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScriptValue {
    /// Trusted worker transport only; never accepted from or emitted as author JSON.
    WindowPage(oryn_common::v3::PageId),
    WindowRealm {
        page: oryn_common::v3::PageId,
        frame: String,
    },
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(Vec<u16>),
    Array(Vec<ScriptValue>),
    Object(Vec<(Vec<u16>, ScriptValue)>),
}
impl PartialEq for ScriptValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::WindowPage(a), Self::WindowPage(b)) => a == b,
            (Self::WindowRealm { page: a, frame: x }, Self::WindowRealm { page: b, frame: y }) => {
                a == b && x == y
            }
            (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Number(a), Self::Number(b)) => a == b || a.as_f64() == b.as_f64(),
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Object(a), Self::Object(b)) => {
                a.iter()
                    .map(|(k, v)| (k, v))
                    .collect::<std::collections::BTreeMap<_, _>>()
                    == b.iter()
                        .map(|(k, v)| (k, v))
                        .collect::<std::collections::BTreeMap<_, _>>()
            }
            _ => false,
        }
    }
}
impl From<serde_json::Value> for ScriptValue {
    fn from(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(v) => Self::Bool(v),
            serde_json::Value::Number(v) => Self::Number(v),
            serde_json::Value::String(v) => Self::String(v.encode_utf16().collect()),
            serde_json::Value::Array(v) => Self::Array(v.into_iter().map(Self::from).collect()),
            serde_json::Value::Object(v) => Self::Object(
                v.into_iter()
                    .map(|(k, v)| (k.encode_utf16().collect(), v.into()))
                    .collect(),
            ),
        }
    }
}
impl ScriptValue {
    pub fn get(&self, name: &str) -> Option<&Self> {
        let Self::Object(entries) = self else {
            return None;
        };
        entries
            .iter()
            .rev()
            .find(|(key, _)| key.iter().copied().eq(name.encode_utf16()))
            .map(|(_, value)| value)
    }
    pub fn as_string(&self) -> Result<String, ScriptError> {
        let Self::String(units) = self else {
            return Err(ScriptError::invalid("expected string"));
        };
        String::from_utf16(units)
            .map_err(|_| ScriptError::invalid("identifier contains an unpaired surrogate"))
    }
    pub fn from_json_text(text: &str) -> Result<Self, ScriptError> {
        if text.len() > super::MAX_VALUE_BYTES {
            return Err(ScriptError::invalid("JSON byte limit"));
        }
        let mut parser = Parser {
            input: text.as_bytes(),
            offset: 0,
            nodes: 0,
        };
        let value = parser.value(0)?;
        parser.space();
        if parser.offset != parser.input.len() {
            return Err(ScriptError::invalid("trailing JSON data"));
        }
        Ok(value)
    }
    pub fn to_json_text(&self) -> Result<String, ScriptError> {
        let mut output = String::new();
        let mut budget = super::ValueBudget::new();
        self.write_json(&mut output, &mut budget, 0)?;
        Ok(output)
    }
    fn write_json(
        &self,
        out: &mut String,
        budget: &mut super::ValueBudget,
        depth: usize,
    ) -> Result<(), ScriptError> {
        budget.consume(depth, 8)?;
        match self {
            Self::WindowPage(_) | Self::WindowRealm { .. } => {
                return Err(ScriptError::invalid(
                    "unresolved native window reference at HTTP boundary",
                ));
            }
            Self::Null => out.push_str("null"),
            Self::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
            Self::Number(v) => out.push_str(&v.to_string()),
            Self::String(v) => {
                budget.consume(depth, encoded_string_len(v))?;
                write_string(out, v)
            }
            Self::Array(values) => {
                out.push('[');
                for (i, v) in values.iter().enumerate() {
                    if i > 0 {
                        out.push(',')
                    }
                    v.write_json(out, budget, depth + 1)?
                }
                out.push(']')
            }
            Self::Object(values) => {
                out.push('{');
                for (i, (key, v)) in values.iter().enumerate() {
                    if i > 0 {
                        out.push(',')
                    }
                    budget.consume(depth, encoded_string_len(key))?;
                    write_string(out, key);
                    out.push(':');
                    v.write_json(out, budget, depth + 1)?
                }
                out.push('}')
            }
        }
        Ok(())
    }
}
fn encoded_string_len(units: &[u16]) -> usize {
    units.iter().fold(2usize, |len, unit| {
        len.saturating_add(if matches!(unit,0x20..=0x21|0x23..=0x5b|0x5d..=0x7e) {
            1
        } else {
            6
        })
    })
}
fn write_string(out: &mut String, units: &[u16]) {
    use std::fmt::Write;
    out.push('"');
    for &unit in units {
        match unit {
            0x20..=0x21 | 0x23..=0x5b | 0x5d..=0x7e => {
                out.push(char::from_u32(unit as u32).unwrap())
            }
            _ => {
                write!(out, "\\u{unit:04x}").unwrap();
            }
        }
    }
    out.push('"');
}
struct Parser<'a> {
    input: &'a [u8],
    offset: usize,
    nodes: usize,
}
impl Parser<'_> {
    fn error(&self) -> ScriptError {
        ScriptError::invalid(format!("invalid JSON at byte {}", self.offset))
    }
    fn space(&mut self) {
        while self
            .input
            .get(self.offset)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.offset += 1
        }
    }
    fn take(&mut self, b: u8) -> bool {
        self.space();
        if self.input.get(self.offset) == Some(&b) {
            self.offset += 1;
            true
        } else {
            false
        }
    }
    fn string(&mut self) -> Result<Vec<u16>, ScriptError> {
        if !self.take(b'"') {
            return Err(self.error());
        }
        let mut result = Vec::new();
        loop {
            let b = *self.input.get(self.offset).ok_or_else(|| self.error())?;
            self.offset += 1;
            match b {
                b'"' => return Ok(result),
                0..=31 => return Err(self.error()),
                b'\\' => {
                    let escape = *self.input.get(self.offset).ok_or_else(|| self.error())?;
                    self.offset += 1;
                    let unit = match escape {
                        b'"' => 34,
                        b'\\' => 92,
                        b'/' => 47,
                        b'b' => 8,
                        b'f' => 12,
                        b'n' => 10,
                        b'r' => 13,
                        b't' => 9,
                        b'u' => {
                            let mut value = 0u16;
                            for _ in 0..4 {
                                let digit =
                                    *self.input.get(self.offset).ok_or_else(|| self.error())?;
                                self.offset += 1;
                                value = value * 16
                                    + match digit {
                                        b'0'..=b'9' => (digit - b'0') as u16,
                                        b'a'..=b'f' => (digit - b'a' + 10) as u16,
                                        b'A'..=b'F' => (digit - b'A' + 10) as u16,
                                        _ => return Err(self.error()),
                                    }
                            }
                            value
                        }
                        _ => return Err(self.error()),
                    };
                    result.push(unit)
                }
                32..=127 => result.push(b as u16),
                _ => {
                    self.offset -= 1;
                    let width = match b {
                        0xc2..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf4 => 4,
                        _ => return Err(self.error()),
                    };
                    let bytes = self
                        .input
                        .get(self.offset..self.offset + width)
                        .ok_or_else(|| self.error())?;
                    let text = std::str::from_utf8(bytes).map_err(|_| self.error())?;
                    let ch = text.chars().next().ok_or_else(|| self.error())?;
                    self.offset += width;
                    let mut buf = [0; 2];
                    result.extend_from_slice(ch.encode_utf16(&mut buf));
                }
            }
        }
    }
    fn value(&mut self, depth: usize) -> Result<ScriptValue, ScriptError> {
        self.nodes += 1;
        if depth > super::MAX_VALUE_DEPTH || self.nodes > super::MAX_VALUE_NODES {
            return Err(self.error());
        }
        self.space();
        let b = *self.input.get(self.offset).ok_or_else(|| self.error())?;
        match b {
            b'"' => Ok(ScriptValue::String(self.string()?)),
            b'[' => {
                self.offset += 1;
                let mut values = Vec::new();
                if self.take(b']') {
                    return Ok(ScriptValue::Array(values));
                }
                loop {
                    values.push(self.value(depth + 1)?);
                    if self.take(b']') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err(self.error());
                    }
                }
                Ok(ScriptValue::Array(values))
            }
            b'{' => {
                self.offset += 1;
                let mut values: Vec<(Vec<u16>, ScriptValue)> = Vec::new();
                let mut keys = std::collections::BTreeMap::<Vec<u16>, usize>::new();
                if self.take(b'}') {
                    return Ok(ScriptValue::Object(values));
                }
                loop {
                    let key = self.string()?;
                    if !self.take(b':') {
                        return Err(self.error());
                    }
                    let value = self.value(depth + 1)?;
                    if let Some(index) = keys.get(&key) {
                        values[*index].1 = value
                    } else {
                        keys.insert(key.clone(), values.len());
                        values.push((key, value))
                    }
                    if self.take(b'}') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err(self.error());
                    }
                }
                Ok(ScriptValue::Object(values))
            }
            b'n' | b't' | b'f' => {
                let (literal, value) = match b {
                    b'n' => (b"null".as_slice(), ScriptValue::Null),
                    b't' => (b"true".as_slice(), ScriptValue::Bool(true)),
                    _ => (b"false".as_slice(), ScriptValue::Bool(false)),
                };
                if self.input.get(self.offset..self.offset + literal.len()) != Some(literal) {
                    return Err(self.error());
                }
                self.offset += literal.len();
                Ok(value)
            }
            b'-' | b'0'..=b'9' => {
                let start = self.offset;
                while self
                    .input
                    .get(self.offset)
                    .is_some_and(|b| matches!(b, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
                {
                    self.offset += 1
                }
                let text = std::str::from_utf8(&self.input[start..self.offset])
                    .map_err(|_| self.error())?;
                let number =
                    serde_json::from_str::<serde_json::Number>(text).map_err(|_| self.error())?;
                Ok(ScriptValue::Number(number))
            }
            _ => Err(self.error()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lone_surrogates_and_keys_roundtrip() {
        let text = r#"{"\ud800":["\udfff","😀","\u0000"]}"#;
        let value = ScriptValue::from_json_text(text).unwrap();
        assert_eq!(
            ScriptValue::from_json_text(&value.to_json_text().unwrap()).unwrap(),
            value
        );
        assert_eq!(value.get("missing"), None);
    }
    #[test]
    fn malformed_json_is_rejected() {
        for text in [
            "[1,]",
            "{\"x\":}",
            "01",
            "1e",
            "true false",
            "\"\\uXYZW\"",
            "\"\\x\"",
            "\"\n\"",
        ] {
            assert!(ScriptValue::from_json_text(text).is_err(), "{text}")
        }
    }
    #[test]
    fn literal_escape_and_surrogate_strings_are_distinct() {
        assert_ne!(
            ScriptValue::from_json_text(r#""\ud800""#).unwrap(),
            ScriptValue::from_json_text(r#""\\ud800""#).unwrap()
        );
    }
}

#[cfg(test)]
mod window_transport_tests {
    use super::*;
    #[test]
    fn native_page_identity_cannot_cross_json_boundary() {
        let value = ScriptValue::WindowPage(oryn_common::v3::PageId(42));
        assert!(value.to_json_text().is_err());
        let text = r#"{"WindowPage":42}"#;
        assert!(!matches!(
            ScriptValue::from_json_text(text).unwrap(),
            ScriptValue::WindowPage(_)
        ));
        let internal = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            serde_json::from_slice::<ScriptValue>(&internal).unwrap(),
            value
        );
    }
}
