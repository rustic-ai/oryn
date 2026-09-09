//! IndexedDB key transport and ordering shared by bindings and parent storage.
//! Values remain JSON at the host boundary, with tagged representations for
//! types JSON cannot represent. Tags in ordinary user objects are escaped by
//! the binding's value serializer.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Number(f64),
    Date(f64),
    String(Vec<u16>),
    Binary(Vec<u8>),
    Array(Vec<Key>),
}

impl Key {
    pub fn from_json(value: &Value) -> Option<Self> {
        match value {
            Value::Number(n) => n.as_f64().filter(|n| !n.is_nan()).map(Self::Number),
            Value::String(s) => Some(Self::String(s.encode_utf16().collect())),
            Value::Array(a) => a
                .iter()
                .map(Self::from_json)
                .collect::<Option<Vec<_>>>()
                .map(Self::Array),
            Value::Object(o) => match o.get("__oryn_idb_key_type")?.as_str()? {
                "date" => o
                    .get("value")?
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .map(Self::Date),
                "number" => match o.get("value")?.as_str()? {
                    "positive_infinity" => Some(Self::Number(f64::INFINITY)),
                    "negative_infinity" => Some(Self::Number(f64::NEG_INFINITY)),
                    _ => None,
                },
                "string" => o
                    .get("units")?
                    .as_array()?
                    .iter()
                    .map(|v| u16::try_from(v.as_u64()?).ok())
                    .collect::<Option<Vec<_>>>()
                    .map(Self::String),
                "binary" => o
                    .get("bytes")?
                    .as_array()?
                    .iter()
                    .map(|v| u8::try_from(v.as_u64()?).ok())
                    .collect::<Option<Vec<_>>>()
                    .map(Self::Binary),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Self::Number(n) if n.is_finite() => json!(if *n == 0.0 { 0.0 } else { *n }),
            Self::Number(n) => {
                json!({"__oryn_idb_key_type":"number","value":if n.is_sign_negative() {"negative_infinity"} else {"positive_infinity"}})
            }
            Self::Date(n) => json!({"__oryn_idb_key_type":"date","value":n}),
            Self::String(units) => String::from_utf16(units)
                .map(Value::String)
                .unwrap_or_else(|_| json!({"__oryn_idb_key_type":"string","units":units})),
            Self::Binary(bytes) => json!({"__oryn_idb_key_type":"binary","bytes":bytes}),
            Self::Array(keys) => Value::Array(keys.iter().map(Self::to_json).collect()),
        }
    }

    pub fn compare(&self, other: &Self) -> Ordering {
        let rank = |key: &Self| match key {
            Self::Number(_) => 0,
            Self::Date(_) => 1,
            Self::String(_) => 2,
            Self::Binary(_) => 3,
            Self::Array(_) => 4,
        };
        let order = rank(self).cmp(&rank(other));
        if !order.is_eq() {
            return order;
        }
        match (self, other) {
            (Self::Number(a), Self::Number(b)) | (Self::Date(a), Self::Date(b)) => {
                a.partial_cmp(b).expect("validated IndexedDB numeric key")
            }
            (Self::String(a), Self::String(b)) => a.cmp(b),
            (Self::Binary(a), Self::Binary(b)) => a.cmp(b),
            (Self::Array(a), Self::Array(b)) => a
                .iter()
                .zip(b)
                .map(|(a, b)| a.compare(b))
                .find(|order| !order.is_eq())
                .unwrap_or_else(|| a.len().cmp(&b.len())),
            _ => unreachable!("equal ranks have the same key type"),
        }
    }

    /// Stable identities retain the existing encodings for ordinary keys.
    /// Record traversal uses compare(), never lexical ordering of this ID.
    pub fn storage_id(&self) -> String {
        match self {
            Self::Number(n) => format!(
                "n:{:016x}",
                if *n == 0.0 {
                    0.0_f64.to_bits()
                } else {
                    n.to_bits()
                }
            ),
            Self::Date(n) => format!(
                "d:{:016x}",
                if *n == 0.0 {
                    0.0_f64.to_bits()
                } else {
                    n.to_bits()
                }
            ),
            Self::String(units) => String::from_utf16(units)
                .map(|s| format!("s:{}", json!(s)))
                .unwrap_or_else(|_| format!("u:{}", json!(units))),
            Self::Binary(bytes) => {
                let mut s = String::from("b:");
                for b in bytes {
                    use std::fmt::Write as _;
                    write!(&mut s, "{b:02x}").expect("write to string");
                }
                s
            }
            Self::Array(keys) => format!(
                "a:{}",
                json!(keys.iter().map(Self::storage_id).collect::<Vec<_>>())
            ),
        }
    }
}

pub fn compare_json(a: &Value, b: &Value) -> Option<Ordering> {
    // Cursor scans compare ordinary keys repeatedly. Their ordering can be
    // computed without allocating an owned key for each side of every comparison.
    match (a, b) {
        (Value::String(a), Value::String(b)) => Some(a.encode_utf16().cmp(b.encode_utf16())),
        (Value::Number(a), Value::Number(b)) => a.as_f64()?.partial_cmp(&b.as_f64()?),
        _ => Some(Key::from_json(a)?.compare(&Key::from_json(b)?)),
    }
}

/// Convert a stored clone value into a key, preserving its type.
pub fn key_from_stored(value: &Value) -> Option<Key> {
    match value.get("__oryn_clone_type").and_then(Value::as_str) {
        Some("date") => value
            .get("value")?
            .as_f64()
            .filter(|n| n.is_finite())
            .map(Key::Date),
        Some("number") => match value.get("value")?.as_str()? {
            "positive_infinity" => Some(Key::Number(f64::INFINITY)),
            "negative_infinity" => Some(Key::Number(f64::NEG_INFINITY)),
            _ => None,
        },
        Some("string") => value
            .get("units")?
            .as_array()?
            .iter()
            .map(|v| u16::try_from(v.as_u64()?).ok())
            .collect::<Option<Vec<_>>>()
            .map(Key::String),
        Some("array") => value
            .get("values")?
            .as_array()?
            .iter()
            .map(key_from_stored)
            .collect::<Option<Vec<_>>>()
            .map(Key::Array),
        Some("array_buffer" | "typed_array") => stored_binary_bytes(value).map(Key::Binary),
        Some(_) => None,
        None if value.is_array() => value
            .as_array()?
            .iter()
            .map(key_from_stored)
            .collect::<Option<Vec<_>>>()
            .map(Key::Array),
        None => Key::from_json(value),
    }
}

/// Decode both compact binary clones and legacy numeric-byte payloads.
pub fn stored_binary_bytes(value: &Value) -> Option<Vec<u8>> {
    if let Some(encoded) = value.get("bodyBase64").and_then(Value::as_str) {
        let bytes = STANDARD.decode(encoded).ok()?;
        if value.get("__oryn_clone_type").and_then(Value::as_str) == Some("typed_array") {
            let offset = usize::try_from(value.get("byteOffset")?.as_u64()?).ok()?;
            let length = usize::try_from(value.get("byteLength")?.as_u64()?).ok()?;
            return bytes
                .get(offset..offset.checked_add(length)?)
                .map(<[u8]>::to_vec);
        }
        return Some(bytes);
    }
    value
        .get("bytes")?
        .as_array()?
        .iter()
        .map(|v| u8::try_from(v.as_u64()?).ok())
        .collect()
}

/// Keep existing JSON number spellings while normalizing tagged clone types.
/// Numeric identity and order are determined by Key, not JSON number variants.
pub fn normalized_stored_key(value: &Value) -> Option<Value> {
    match value {
        Value::Number(_) | Value::String(_) => Key::from_json(value).map(|_| value.clone()),
        Value::Array(values) => values
            .iter()
            .map(normalized_stored_key)
            .collect::<Option<Vec<_>>>()
            .map(Value::Array),
        Value::Object(o) if o.get("__oryn_clone_type").and_then(Value::as_str) == Some("array") => {
            o.get("values")?
                .as_array()?
                .iter()
                .map(normalized_stored_key)
                .collect::<Option<Vec<_>>>()
                .map(Value::Array)
        }
        _ => key_from_stored(value).map(|key| key.to_json()),
    }
}

/// Extract index entries once for writes, backfills, and cursor updates.
/// multiEntry removes invalid members and duplicate keys within one record.
pub fn index_keys(value: &Value, path: &str, multi_entry: bool) -> Vec<Value> {
    let Some(value) = key_path(value, path) else {
        return Vec::new();
    };
    let elements = value.as_array().or_else(|| {
        (value.get("__oryn_clone_type").and_then(Value::as_str) == Some("array"))
            .then(|| value.get("values").and_then(Value::as_array))
            .flatten()
    });
    let candidates = if multi_entry {
        elements
            .map(Vec::as_slice)
            .unwrap_or_else(|| std::slice::from_ref(&value))
    } else {
        std::slice::from_ref(&value)
    };
    let mut seen = std::collections::BTreeSet::new();
    candidates
        .iter()
        .filter_map(normalized_stored_key)
        .filter(|value| Key::from_json(value).is_some_and(|key| seen.insert(key.storage_id())))
        .collect()
}

pub fn key_path(value: &Value, path: &str) -> Option<Value> {
    if let Some(paths) = path.strip_prefix("__oryn_compound_key_path__:") {
        return serde_json::from_str::<Vec<String>>(paths)
            .ok()?
            .iter()
            .map(|path| key_path(value, path))
            .collect::<Option<Vec<_>>>()
            .map(Value::Array);
    }
    if path.is_empty() {
        return Some(value.clone());
    }
    let mut parts = path.split('.');
    let mut current = property(value, parts.next()?)?;
    for part in parts {
        current = property(&current, part)?;
    }
    Some(current)
}

/// Check the cloned value before a generated key is submitted to storage.
/// Missing intermediate properties can be created, but primitives cannot own them.
pub fn can_inject_key_path(value: &Value, path: &str) -> bool {
    if path.is_empty() || path.starts_with("__oryn_compound_key_path__:") {
        return false;
    }
    let mut current = value.clone();
    let mut parts = path.split('.').peekable();
    while let Some(part) = parts.next() {
        if !current.is_object() && !current.is_array() {
            return false;
        }
        if matches!(
            current.get("__oryn_clone_type").and_then(Value::as_str),
            Some("undefined" | "number" | "string")
        ) {
            return false;
        }
        if parts.peek().is_none() {
            return true;
        }
        match property(&current, part) {
            Some(value) => current = value,
            None => return true,
        }
    }
    false
}

fn property(value: &Value, name: &str) -> Option<Value> {
    match value {
        Value::String(s) if name == "length" => Some(json!(s.encode_utf16().count())),
        Value::Array(a) if name == "length" => Some(json!(a.len())),
        Value::Object(o) => match o.get("__oryn_clone_type").and_then(Value::as_str) {
            Some("object") => o.get("properties")?.get(name).cloned(),
            Some("array") if name == "length" => Some(json!(o.get("values")?.as_array()?.len())),
            Some("array") => o.get("properties")?.get(name).cloned(),
            Some("string") if name == "length" => Some(json!(o.get("units")?.as_array()?.len())),
            Some("blob") => match name {
                "size" => Some(json!(
                    STANDARD.decode(o.get("bodyBase64")?.as_str()?).ok()?.len()
                )),
                "type" => o.get("type").cloned(),
                "name" | "lastModified" if o.get("name")?.is_string() => o.get(name).cloned(),
                _ => None,
            },
            Some(_) => None,
            None => o.get(name).cloned(),
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ordinary_json_key_comparisons_preserve_utf16_and_numeric_order() {
        assert_eq!(
            super::compare_json(&serde_json::json!("𐀀"), &serde_json::json!("\u{e000}")),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            super::compare_json(&serde_json::json!(1), &serde_json::json!(1.0)),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(
            super::compare_json(&serde_json::json!(-0.0), &serde_json::json!(0)),
            Some(std::cmp::Ordering::Equal)
        );
        let keys = [
            serde_json::json!(""),
            serde_json::json!("a"),
            serde_json::json!("aa"),
            serde_json::json!("💡"),
            serde_json::json!("\u{e000}"),
            serde_json::json!({"__oryn_idb_key_type":"string","units":[55296]}),
            serde_json::json!(-3),
            serde_json::json!(0),
            serde_json::json!(1.0),
        ];
        for left in &keys {
            for right in &keys {
                assert_eq!(
                    super::compare_json(left, right),
                    Some(
                        super::Key::from_json(left)
                            .unwrap()
                            .compare(&super::Key::from_json(right).unwrap())
                    )
                );
            }
        }
    }

    use super::*;
    #[test]
    fn generated_key_injection_rejects_existing_primitive_parents() {
        assert!(can_inject_key_path(&json!({}), "a.b.c"));
        assert!(can_inject_key_path(&json!({"a":{}}), "a.b.c"));
        assert!(!can_inject_key_path(&json!({"a":123}), "a.b"));
        assert!(!can_inject_key_path(&json!({"a":{"b":"foo"}}), "a.b.c"));
        assert!(!can_inject_key_path(&json!({"a":null}), "a.b"));
        assert!(!can_inject_key_path(&json!("text"), "key"));
        assert!(can_inject_key_path(
            &json!({"__oryn_clone_type":"object","properties":{"a":{}}}),
            "a.b"
        ));
    }

    #[test]
    fn transport_roundtrips_and_orders_all_key_types() {
        let keys = [
            Key::Number(f64::NEG_INFINITY),
            Key::Number(-1.0),
            Key::Number(0.0),
            Key::Number(f64::INFINITY),
            Key::Date(0.0),
            Key::String(vec![0xd800]),
            Key::String(vec![0xd83d, 0xdca1]),
            Key::String(vec![0xe000]),
            Key::Binary(vec![]),
            Key::Binary(vec![0, 255]),
            Key::Array(vec![]),
        ];
        for key in &keys {
            assert_eq!(Key::from_json(&key.to_json()).unwrap(), *key);
        }
        for pair in keys.windows(2) {
            assert!(pair[0].compare(&pair[1]).is_lt(), "{pair:?}");
        }
        assert_eq!(
            Key::Number(-0.0).storage_id(),
            Key::Number(0.0).storage_id()
        );
        assert_ne!(Key::Date(0.0).storage_id(), Key::Number(0.0).storage_id());
        assert!(Key::from_json(&json!({"__oryn_idb_key_type":"binary","bytes":[256]})).is_none());
    }
    #[test]
    fn index_entries_deduplicate_by_key_identity_and_preserve_date_type() {
        let value = json!({"tags":{"__oryn_clone_type":"array","values":[1,1.0,null,{"__oryn_clone_type":"date","value":1},-0.0,0],"properties":{}}});
        let keys = index_keys(&value, "tags", true);
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[0], json!(1));
        assert!(matches!(Key::from_json(&keys[1]), Some(Key::Date(1.0))));
        assert!(matches!(Key::from_json(&keys[2]),Some(Key::Number(n)) if n==0.0));
        assert!(index_keys(&value, "tags", false).is_empty());
    }

    #[test]
    fn key_paths_observe_clone_intrinsics_without_exposing_transport_fields() {
        assert_eq!(key_path(&json!({"s":"💡"}), "s.length"), Some(json!(2)));
        let array = json!({"__oryn_clone_type":"array","values":[1,2],"properties":{"custom":9}});
        assert_eq!(key_path(&array, "length"), Some(json!(2)));
        assert_eq!(key_path(&array, "custom"), Some(json!(9)));
        assert_eq!(key_path(&array, "values"), None);
        let file = json!({"__oryn_clone_type":"blob","bodyBase64":"AQID","type":"text/plain","name":"a.txt","lastModified":7});
        assert_eq!(key_path(&file, "size"), Some(json!(3)));
        assert_eq!(key_path(&file, "name.length"), Some(json!(5)));
        assert_eq!(key_path(&file, "bodyBase64"), None);
    }
}
