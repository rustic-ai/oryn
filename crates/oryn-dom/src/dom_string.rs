//! Lossless UTF-16 adapter for the existing String-backed DOM storage.
//! Ordinary Unicode stays unchanged. The reserved prefix is escaped too, so
//! author strings resembling an encoded value round-trip without collisions.
pub const PREFIX: &str = "\u{f0000}oryn-wtf16:";

pub fn units(value: &str) -> Vec<u16> {
    if let Some(body) = value
        .strip_prefix(PREFIX)
        .and_then(|v| v.strip_prefix('['))
        .and_then(|v| v.strip_suffix(']'))
    {
        if body.is_empty() {
            return Vec::new();
        }
        if let Ok(units) = body.split(',').map(|v| v.trim().parse::<u16>()).collect() {
            return units;
        }
    }
    value.encode_utf16().collect()
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
    if let Ok(value) = String::from_utf16(units)
        && !value.starts_with(PREFIX)
    {
        return value;
    }
    format!(
        "{PREFIX}[{}]",
        units
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
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
