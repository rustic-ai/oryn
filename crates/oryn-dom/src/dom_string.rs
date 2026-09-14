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
