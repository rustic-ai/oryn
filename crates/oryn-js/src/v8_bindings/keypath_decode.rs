//! Exact admitted decoder for the private compound-keypath JSON string array.
use crate::idb_owned_admission::{Buffer, Ledger, Scope, Text};
use std::sync::Arc;

fn whitespace(
    input: &[u8],
    position: &mut usize,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), &'static str> {
    while input
        .get(*position)
        .is_some_and(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t'))
    {
        if (*position).is_multiple_of(1024) && cancelled() {
            return Err("keypath cancelled");
        }
        *position += 1;
    }
    Ok(())
}
fn string(
    input: &[u8],
    position: &mut usize,
    mut emit: impl FnMut(u16) -> Result<(), &'static str>,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), &'static str> {
    if input.get(*position) != Some(&b'"') {
        return Err("keypath string");
    }
    *position += 1;
    loop {
        if cancelled() {
            return Err("keypath cancelled");
        }
        let byte = *input.get(*position).ok_or("keypath unterminated")?;
        *position += 1;
        if byte == b'"' {
            return Ok(());
        }
        if byte < 32 {
            return Err("keypath control");
        }
        if byte == b'\\' {
            let escaped = *input.get(*position).ok_or("keypath escape")?;
            *position += 1;
            let unit = match escaped {
                b'"' => 34,
                b'\\' => 92,
                b'/' => 47,
                b'b' => 8,
                b'f' => 12,
                b'n' => 10,
                b'r' => 13,
                b't' => 9,
                b'u' => {
                    let mut unit = 0u16;
                    for _ in 0..4 {
                        let digit = *input.get(*position).ok_or("keypath unicode escape")?;
                        *position += 1;
                        let digit = match digit {
                            b'0'..=b'9' => digit - b'0',
                            b'a'..=b'f' => digit - b'a' + 10,
                            b'A'..=b'F' => digit - b'A' + 10,
                            _ => return Err("keypath unicode escape"),
                        };
                        unit = (unit << 4) | u16::from(digit);
                    }
                    unit
                }
                _ => return Err("keypath escape"),
            };
            emit(unit)?;
        } else {
            let start = *position - 1;
            let width = match byte {
                0..=127 => 1,
                194..=223 => 2,
                224..=239 => 3,
                240..=244 => 4,
                _ => return Err("keypath utf8"),
            };
            let end = start.checked_add(width).ok_or("keypath overflow")?;
            let scalar = std::str::from_utf8(input.get(start..end).ok_or("keypath utf8")?)
                .map_err(|_| "keypath utf8")?
                .chars()
                .next()
                .ok_or("keypath utf8")?;
            *position = end;
            let mut units = [0u16; 2];
            for unit in scalar.encode_utf16(&mut units) {
                emit(*unit)?;
            }
        }
    }
}
fn walk<C: FnMut() -> bool>(
    input: &[u8],
    mut on_string: impl FnMut(usize, usize, &mut C) -> Result<(), &'static str>,
    cancelled: &mut C,
) -> Result<usize, &'static str> {
    let mut position = 0;
    whitespace(input, &mut position, cancelled)?;
    if input.get(position) != Some(&b'[') {
        return Err("keypath array");
    };
    position += 1;
    whitespace(input, &mut position, cancelled)?;
    let mut count = 0usize;
    if input.get(position) != Some(&b']') {
        loop {
            let start = position;
            let mut units = 0usize;
            string(
                input,
                &mut position,
                |_| {
                    units = units.checked_add(1).ok_or("keypath overflow")?;
                    Ok(())
                },
                cancelled,
            )?;
            on_string(start, units, cancelled)?;
            count = count.checked_add(1).ok_or("keypath overflow")?;
            whitespace(input, &mut position, cancelled)?;
            match input.get(position) {
                Some(b',') => {
                    position += 1;
                    whitespace(input, &mut position, cancelled)?;
                }
                Some(b']') => break,
                _ => return Err("keypath separator"),
            }
        }
    }
    position += 1;
    whitespace(input, &mut position, cancelled)?;
    if position != input.len() {
        return Err("keypath trailing");
    }
    if cancelled() {
        return Err("keypath cancelled");
    };
    Ok(count)
}
pub(crate) fn decode(
    json: &str,
    ledger: &Arc<Ledger>,
    limit: usize,
    used: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Scope, &'static str> {
    let input = json.as_bytes();
    let count = walk(input, |_, _, _| Ok(()), &mut cancelled)?;
    let mut scope = Scope::allocate(ledger, count, limit, used)?;
    walk(
        input,
        |start, count, cancelled| {
            let mut units = Buffer::<u16>::allocate(ledger, count, limit, used)?;
            let mut position = start;
            // Both count and fill passes poll the same native cancellation
            // checkpoint; neither invokes author callbacks.
            string(input, &mut position, |unit| units.push(unit), cancelled)?;
            let text = Text::from_units_cancelled(ledger, units.as_slice(), limit, used, cancelled)?;
            scope.push(text)
        },
        &mut cancelled,
    )?;
    Ok(scope)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_and_rejection_release() {
        let ledger = Arc::new(Ledger::default());
        for json in [
            "[]",
            "[\"same\",\"same\"]",
            "[\"\\uD800\",\"\\n\\\"\\\\\",\"非\"]",
        ] {
            let value = decode(json, &ledger, 1 << 20, 0, || false).unwrap();
            let (names, custody) = value.into_parts();
            if json == "[]" {
                assert!(names.is_empty());
            }
            if json == "[\"same\",\"same\"]" {
                assert_eq!(names, ["same", "same"]);
            }
            if json.contains("D800") {
                assert_eq!(oryn_dom::dom_string::units(&names[0]), [0xd800]);
            }
            drop(names);
            drop(custody);
            assert_eq!(ledger.live_bytes(), 0);
        }
        for json in ["[", "[\"a\",]", "[1]", "[\"\\x\"]", "[]x"] {
            assert!(decode(json, &ledger, 1 << 20, 0, || false).is_err());
            assert_eq!(ledger.live_bytes(), 0);
        }
        assert!(decode("[]", &ledger, 1 << 20, 0, || true).is_err());
        let input = " ".repeat(8192) + "[]";
        let mut calls = 0;
        assert!(
            decode(&input, &ledger, 1 << 20, 0, || {
                calls += 1;
                calls >= 3
            })
            .is_err()
        );
        assert!(calls >= 3);
        assert_eq!(ledger.live_bytes(), 0);
    }
}
