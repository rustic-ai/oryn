//! Lossless UTF-16 adapter for the existing String-backed DOM storage.
//! Ordinary Unicode stays unchanged. The reserved prefix is escaped too, so
//! author strings resembling an encoded value round-trip without collisions.
pub const PREFIX: &str = "\u{f0000}oryn-wtf16:";

pub fn units(value: &str) -> Vec<u16> {
    units_with_allocation(
        value,
        &mut crate::projection_allocation::OrdinaryProjectionAllocation,
        &mut || Ok(()),
    )
    .expect("ordinary DOMString allocation failed")
}

/// Decode canonical DOM storage with checked caller-owned scratch demand.
/// No callback is run and no partially decoded value escapes on failure.
pub fn try_units(value: &str, max_units: usize) -> Result<Vec<u16>, &'static str> {
    let body = value
        .strip_prefix(PREFIX)
        .and_then(|v| v.strip_prefix('['))
        .and_then(|v| v.strip_suffix(']'));
    if let Some(body) = body {
        let count = if body.is_empty() {
            0
        } else {
            body.split(',').count()
        };
        if count > max_units {
            return Err("DOMString unit limit");
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(count)
            .map_err(|_| "DOMString unit allocation")?;
        let mut valid = true;
        if !body.is_empty() {
            for part in body.split(',') {
                match part.trim().parse::<u16>() {
                    Ok(unit) => output.push(unit),
                    Err(_) => {
                        valid = false;
                        break;
                    }
                }
            }
        }
        if valid {
            return Ok(output);
        }
        // Malformed reserved-prefix text is ordinary Unicode, as in units().
    }
    let count = value.encode_utf16().count();
    if count > max_units {
        return Err("DOMString unit limit");
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(count)
        .map_err(|_| "DOMString unit allocation")?;
    output.extend(value.encode_utf16());
    Ok(output)
}

pub fn from_units(units: &[u16]) -> String {
    from_units_with_allocation(
        units,
        &mut crate::projection_allocation::OrdinaryProjectionAllocation,
        &mut || Ok(()),
    )
    .expect("ordinary DOMString allocation failed")
}

/// ASCII case mapping over DOMString code units, preserving lone surrogates
/// and escaping the private representation rather than transforming its bytes.
pub fn ascii_lowercase(value: &str) -> String {
    from_units(
        &units(value)
            .into_iter()
            .map(|unit| {
                if (65..=90).contains(&unit) {
                    unit + 32
                } else {
                    unit
                }
            })
            .collect::<Vec<_>>(),
    )
}
pub fn ascii_uppercase(value: &str) -> String {
    from_units(
        &units(value)
            .into_iter()
            .map(|unit| {
                if (97..=122).contains(&unit) {
                    unit - 32
                } else {
                    unit
                }
            })
            .collect::<Vec<_>>(),
    )
}

pub fn len(value: &str) -> usize {
    units(value).len()
}

/// USVString conversion, retaining collision-safe internal storage.
pub fn usv(value: &str) -> String {
    from_units(
        &String::from_utf16_lossy(&units(value))
            .encode_utf16()
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descendant_text_excludes_internal_edges_and_preserves_code_units() {
        use crate::{DomArena, NodeKind, ShadowMode};
        use html5ever::{LocalName, Namespace, QualName};
        let mut arena = DomArena::default();
        let root = arena.insert(NodeKind::Element {
            name: QualName::new(
                None,
                Namespace::from("http://www.w3.org/1999/xhtml"),
                LocalName::from("div"),
            ),
        });
        let first = arena.insert(NodeKind::Text {
            data: from_units(&[0xd83d]),
        });
        let comment = arena.insert(NodeKind::Comment {
            data: "excluded comment".into(),
        });
        let pi = arena.insert(NodeKind::ProcessingInstruction {
            target: "target".into(),
            data: "excluded instruction".into(),
        });
        let second = arena.insert(NodeKind::CDataSection {
            data: from_units(&[0xde00]),
        });
        let shadow = arena.insert(NodeKind::ShadowRoot {
            mode: ShadowMode::Open,
            clonable: false,
        });
        let hidden = arena.insert(NodeKind::Text {
            data: "shadow".into(),
        });
        let hosted = arena.insert(NodeKind::Document);
        let foreign = arena.insert(NodeKind::Text {
            data: "hosted".into(),
        });
        // Internal ownership edges deliberately coexist with light children.
        arena.get_mut(root).unwrap().children = vec![first, comment, pi, second, shadow, hosted];
        arena.get_mut(shadow).unwrap().children = vec![hidden];
        arena.get_mut(hosted).unwrap().children = vec![foreign];
        assert_eq!(
            units(&arena.text_content(root).unwrap()),
            vec![0xd83d, 0xde00]
        );
        assert_eq!(arena.text_content(comment).unwrap(), "excluded comment");
        assert_eq!(arena.text_content(pi).unwrap(), "excluded instruction");
        assert_eq!(arena.text_content(shadow).unwrap(), "shadow");
        assert_eq!(arena.text_content(hosted).unwrap(), "hosted");
        arena.get_mut(second).unwrap().kind = NodeKind::Text {
            data: from_units(
                &format!("{PREFIX}[56321]")
                    .encode_utf16()
                    .collect::<Vec<_>>(),
            ),
        };
        let mut expected = vec![0xd83d];
        expected.extend(format!("{PREFIX}[56321]").encode_utf16());
        assert_eq!(units(&arena.text_content(root).unwrap()), expected);
    }

    #[test]
    fn descendant_text_handles_deep_trees_and_rejects_cycles() {
        use crate::{DomArena, NodeKind};
        use html5ever::{LocalName, Namespace, QualName};
        let mut arena = DomArena::default();
        let kind = NodeKind::Element {
            name: QualName::new(
                None,
                Namespace::from("http://www.w3.org/1999/xhtml"),
                LocalName::from("div"),
            ),
        };
        let root = arena.insert(kind.clone());
        let mut parent = root;
        for _ in 0..20_000 {
            let child = arena.insert(kind.clone());
            arena.get_mut(parent).unwrap().children.push(child);
            parent = child;
        }
        let text = arena.insert(NodeKind::Text {
            data: "deep".into(),
        });
        arena.get_mut(parent).unwrap().children.push(text);
        assert_eq!(arena.text_content(root).unwrap(), "deep");
        arena.get_mut(parent).unwrap().children.push(root);
        assert!(matches!(
            arena.text_content(root),
            Err(crate::DomError::Cycle)
        ));
    }

    #[test]
    fn lossless_units_and_literal_prefix_do_not_collide() {
        for value in [
            vec![0xd801],
            vec![0xdc01],
            vec![0xd83d, 0xde00],
            format!("{PREFIX}[56321]").encode_utf16().collect(),
        ] {
            assert_eq!(units(&from_units(&value)), value);
            assert_eq!(len(&from_units(&value)), value.len());
        }
        assert_eq!(usv(&from_units(&[0xdc01])), "\u{fffd}");
    }

    #[test]
    fn control_splices_and_opaque_offsets_count_code_units() {
        let mut dom = crate::DomRuntime::new();
        let node = dom.create_element_ns("http://www.w3.org/1999/xhtml", "textarea");
        dom.set_value(node, from_units(&[0xdc01, 0xd83d, 0xde00]), false)
            .unwrap();
        let range = dom.create_opaque_range(node, 1, 3).unwrap();
        assert!(dom.create_opaque_range(node, 0, 4).is_err());
        dom.replace_control_value(node, 1, 2, &from_units(&[0xd801]))
            .unwrap();
        assert_eq!(
            units(&dom.control_value(node).unwrap()),
            [0xdc01, 0xd801, 0xde00]
        );
        assert_eq!(dom.opaque_range(range).unwrap().end, 3);
        dom.set_value(node, from_units(&[0xdc01]), false).unwrap();
        assert!(dom.opaque_range(range).unwrap().end <= 1);
    }
}

#[cfg(test)]
mod ascii_case_tests {
    use super::*;
    #[test]
    fn ascii_case_preserves_lone_surrogates_and_reserved_prefix_text() {
        let original = from_units(&[65, 0xd800, 122, 0xdc00, 0x00c9]);
        assert_eq!(
            units(&ascii_lowercase(&original)),
            vec![97, 0xd800, 122, 0xdc00, 0x00c9]
        );
        assert_eq!(
            units(&ascii_uppercase(&original)),
            vec![65, 0xd800, 90, 0xdc00, 0x00c9]
        );
        let literal = format!("{PREFIX}[65]");
        let encoded = from_units(&literal.encode_utf16().collect::<Vec<_>>());
        assert_eq!(
            units(&ascii_uppercase(&encoded)),
            literal
                .to_ascii_uppercase()
                .encode_utf16()
                .collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod pointer_word_admission_tests {
    use super::*;
    #[test]
    fn admitted_dom_strings_match_canonical_surrogates_and_prefix_collision() {
        for units in [
            vec![],
            vec![65, 0xd83d, 0xde00],
            vec![0xd800, 65],
            PREFIX.encode_utf16().collect(),
        ] {
            let source = from_units(&units);
            assert_eq!(try_units(&source, units.len()).unwrap(), units);
            if !units.is_empty() {
                assert!(try_units(&source, units.len() - 1).is_err());
            }
        }
        let malformed = format!("{PREFIX}[not-a-unit]");
        assert_eq!(
            try_units(&malformed, malformed.len()).unwrap(),
            units(&malformed)
        );
    }
}

pub fn units_with_allocation<A: crate::projection_allocation::ProjectionAllocation>(
    value: &str,
    allocation: &mut A,
    check: &mut impl FnMut() -> Result<(), crate::DomError>,
) -> Result<Vec<u16>, A::Error> {
    use crate::projection_allocation::admitted_push;
    check().map_err(A::Error::from)?;
    let body = value
        .strip_prefix(PREFIX)
        .and_then(|v| v.strip_prefix('['))
        .and_then(|v| v.strip_suffix(']'));
    if let Some(body) = body {
        let mut output = Vec::new();
        let mut start = 0usize;
        let mut valid = true;
        if body.is_empty() {
            check().map_err(A::Error::from)?;
            return Ok(output);
        }
        for (index, ch) in body
            .char_indices()
            .chain(std::iter::once((body.len(), ',')))
        {
            check().map_err(A::Error::from)?;
            if ch != ',' {
                continue;
            }
            let part = &body[start..index];
            let mut left = 0usize;
            let mut right = part.len();
            for (offset, ch) in part.char_indices() {
                check().map_err(A::Error::from)?;
                if !ch.is_whitespace() {
                    left = offset;
                    break;
                }
                left = offset + ch.len_utf8();
            }
            for (offset, ch) in part.char_indices().rev() {
                check().map_err(A::Error::from)?;
                if !ch.is_whitespace() {
                    right = offset + ch.len_utf8();
                    break;
                }
                right = offset;
            }
            let token = if left <= right {
                &part[left..right]
            } else {
                ""
            };
            let digits = token.strip_prefix('+').unwrap_or(token);
            let mut number = 0u16;
            if digits.is_empty() {
                valid = false;
                break;
            }
            for byte in digits.bytes() {
                check().map_err(A::Error::from)?;
                if !byte.is_ascii_digit() {
                    valid = false;
                    break;
                }
                let Some(next) = number
                    .checked_mul(10)
                    .and_then(|n| n.checked_add(u16::from(byte - b'0')))
                else {
                    valid = false;
                    break;
                };
                number = next;
            }
            if !valid {
                break;
            }
            admitted_push(&mut output, number, allocation, check)?;
            start = index + 1;
        }
        if valid {
            check().map_err(A::Error::from)?;
            return Ok(output);
        }
        drop(output);
    }
    let mut output = Vec::new();
    for ch in value.chars() {
        check().map_err(A::Error::from)?;
        let mut units = [0u16; 2];
        for unit in ch.encode_utf16(&mut units) {
            admitted_push(&mut output, *unit, allocation, check)?;
        }
    }
    check().map_err(A::Error::from)?;
    Ok(output)
}

pub fn from_units_with_allocation<A: crate::projection_allocation::ProjectionAllocation>(
    units: &[u16],
    allocation: &mut A,
    check: &mut impl FnMut() -> Result<(), crate::DomError>,
) -> Result<String, A::Error> {
    use crate::projection_allocation::admitted_append;
    let mut unicode = String::new();
    let mut valid = true;
    for character in char::decode_utf16(units.iter().copied()) {
        check().map_err(A::Error::from)?;
        match character {
            Ok(ch) => {
                let mut bytes = [0u8; 4];
                admitted_append(&mut unicode, ch.encode_utf8(&mut bytes), allocation, check)?;
            }
            Err(_) => {
                valid = false;
                break;
            }
        }
    }
    check().map_err(A::Error::from)?;
    if valid && !unicode.starts_with(PREFIX) {
        return Ok(unicode);
    }
    // Drop Unicode scratch before preparing collision-safe/lone-surrogate form.
    drop(unicode);
    let mut output = String::new();
    admitted_append(&mut output, PREFIX, allocation, check)?;
    admitted_append(&mut output, "[", allocation, check)?;
    for (index, unit) in units.iter().enumerate() {
        check().map_err(A::Error::from)?;
        if index > 0 {
            admitted_append(&mut output, ",", allocation, check)?;
        }
        let mut digits = [0u8; 5];
        let mut value = *unit;
        let mut offset = digits.len();
        loop {
            offset -= 1;
            digits[offset] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        // Fixed ASCII stack buffer, no formatting/String allocation.
        let text = std::str::from_utf8(&digits[offset..]).expect("decimal ASCII");
        admitted_append(&mut output, text, allocation, check)?;
    }
    admitted_append(&mut output, "]", allocation, check)?;
    check().map_err(A::Error::from)?;
    Ok(output)
}

#[cfg(test)]
mod allocation_codec_controls {
    use super::*;
    use crate::projection_allocation::OrdinaryProjectionAllocation;
    #[test]
    fn units_match_original_scalar_and_reserved_prefix_rules() {
        for text in [
            "",
            "é😀",
            "\u{f0000}oryn-wtf16:[]",
            "\u{f0000}oryn-wtf16:[+65, 55296,65535]",
            "\u{f0000}oryn-wtf16:[65536]",
            "\u{f0000}oryn-wtf16:[-0]",
            "\u{f0000}oryn-wtf16:[1,,2]",
        ] {
            let expected = if let Some(body) = text
                .strip_prefix(PREFIX)
                .and_then(|v| v.strip_prefix('['))
                .and_then(|v| v.strip_suffix(']'))
            {
                if body.is_empty() {
                    Vec::new()
                } else {
                    body.split(',')
                        .map(|v| v.trim().parse::<u16>())
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap_or_else(|_| text.encode_utf16().collect())
                }
            } else {
                text.encode_utf16().collect()
            };
            assert_eq!(
                units_with_allocation(text, &mut OrdinaryProjectionAllocation, &mut || Ok(()))
                    .unwrap(),
                expected
            );
        }
    }
    #[test]
    fn from_units_matches_original_lone_surrogate_and_collision_rules() {
        let collision: Vec<u16> = PREFIX.encode_utf16().collect();
        for units in [vec![], vec![65, 55357, 56832], vec![55296, 65], collision] {
            let expected = match String::from_utf16(&units) {
                Ok(value) if !value.starts_with(PREFIX) => value,
                _ => format!(
                    "{PREFIX}[{}]",
                    units
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            };
            assert_eq!(
                from_units_with_allocation(&units, &mut OrdinaryProjectionAllocation, &mut || Ok(
                    ()
                ))
                .unwrap(),
                expected
            );
        }
    }
    #[test]
    fn codec_cancellation_does_not_publish_partial_value() {
        let input = format!("{PREFIX}[{}]", vec!["0"; 8192].join(","));
        let mut calls = 0usize;
        let result = units_with_allocation(&input, &mut OrdinaryProjectionAllocation, &mut || {
            calls += 1;
            if calls == 64 {
                Err(crate::DomError::ProjectionCancelled)
            } else {
                Ok(())
            }
        });
        assert!(matches!(result, Err(crate::DomError::ProjectionCancelled)));
        assert_eq!(calls, 64);
    }
}
