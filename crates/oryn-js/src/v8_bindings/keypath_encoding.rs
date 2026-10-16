//! Staged native compound-keypath encoder. Not yet integrated.
//! Uses existing exact allocation admission; no serde staging allocation.
use crate::idb_owned_admission::{Buffer, Ledger, Text};
use std::sync::Arc;

pub(crate) fn encode(
    ledger: &Arc<Ledger>,
    names: &[String],
    compound: bool,
    limit: usize,
    used: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Text, &'static str> {
    if cancelled() { return Err("keypath cancelled"); }
    if !compound {
        let name=names.first().ok_or("keypath scalar")?;
        let mut bytes=Buffer::<u8>::allocate(ledger,name.len(),limit,used)?;
        for (offset,byte) in name.bytes().enumerate() {
            if offset % 1024 == 0 && cancelled() { return Err("keypath cancelled"); }
            bytes.push(byte)?;
        }
        if cancelled() { return Err("keypath cancelled"); }
        return Text::from_admitted_utf8(bytes);
    }
    const PREFIX: &str = "__oryn_compound_key_path__:";
    let mut length = PREFIX.len().checked_add(2).ok_or("keypath overflow")?;
    for (index, name) in names.iter().enumerate() {
        if cancelled() { return Err("keypath cancelled"); }
        length = length.checked_add(2).and_then(|value|value.checked_add(usize::from(index!=0))).ok_or("keypath overflow")?;
        for (offset,byte) in name.bytes().enumerate() {
            if offset % 1024 == 0 && cancelled() { return Err("keypath cancelled"); }
            let count = match byte { b'"' | b'\\' | b'\x08' | b'\x0c' | b'\n' | b'\r' | b'\t' => 2, 0..=31 => 6, _ => 1 };
            length = length.checked_add(count).ok_or("keypath overflow")?;
        }
    }
    let mut bytes = Buffer::<u8>::allocate(ledger, length, limit, used)?;
    for byte in PREFIX.bytes().chain(std::iter::once(b'[')) { bytes.push(byte)?; }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (index, name) in names.iter().enumerate() {
        if cancelled() { return Err("keypath cancelled"); }
        if index != 0 { bytes.push(b',')?; }
        bytes.push(b'"')?;
        for (offset, byte) in name.bytes().enumerate() {
            if offset % 1024 == 0 && cancelled() { return Err("keypath cancelled"); }
            let escaped = match byte {
                b'"' => Some(b'"'), b'\\' => Some(b'\\'), b'\x08' => Some(b'b'),
                b'\x0c' => Some(b'f'), b'\n' => Some(b'n'), b'\r' => Some(b'r'), b'\t' => Some(b't'), _ => None,
            };
            if let Some(escaped) = escaped { bytes.push(b'\\')?; bytes.push(escaped)?; }
            else if byte < 32 {
                for item in [b'\\', b'u', b'0', b'0', HEX[(byte >> 4) as usize], HEX[(byte & 15) as usize]] { bytes.push(item)?; }
            } else { bytes.push(byte)?; }
        }
        bytes.push(b'"')?;
    }
    if cancelled() { return Err("keypath cancelled"); }
    bytes.push(b']')?;
    Text::from_admitted_utf8(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compound_encoding_matches_persistent_format_and_releases() {
        for names in [vec![], vec!["one".into()], vec!["one".into(), "one".into()],
            vec!["quote\"\\\n\t\0".into(), "非ASCII".into()]] {
            let ledger = Arc::new(Ledger::default());
            let result = encode(&ledger, &names, true, 1 << 20, 0, || false).unwrap();
            assert_eq!(result.as_str(), format!("__oryn_compound_key_path__:{}", serde_json::to_string(&names).unwrap()));
            assert!(ledger.live_bytes() > 0);
            drop(result);
            assert_eq!(ledger.live_bytes(), 0);
        }
    }
    #[test]
    fn encoding_failure_and_cancel_release_exact_allocation() {
        let ledger = Arc::new(Ledger::default());
        assert!(encode(&ledger, &["long".into()], true, 1, 0, || false).is_err());
        assert_eq!(ledger.live_bytes(), 0);
        let mut calls = 0;
        assert!(encode(&ledger, &["long".into()], true, 1 << 20, 0, || { calls += 1; calls > 1 }).is_err());
        assert_eq!(ledger.live_bytes(), 0);
    }
}
