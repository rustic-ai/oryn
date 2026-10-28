//! Native OpenType horizontal BASE coordinates, before canvas anchor projection.
use super::LayoutError;
use skrifa::raw::{
    FontRead,
    tables::{
        base::{Base, BaseCoord},
        layout::DeviceOrVariationIndex,
    },
    types::Tag,
};
use std::collections::BTreeMap;

/// Reads requested script, falling back only to the font's explicit DFLT script.
/// Positions remain in the table's reference coordinate system; callers must
/// subtract the roman baseline before treating them as alphabetic-relative.
pub fn read_horizontal(
    table: &[u8],
    script: Tag,
    scale: f64,
    coordinates: &[i16],
) -> Result<Option<BTreeMap<Tag, f64>>, LayoutError> {
    let invalid = |e: skrifa::raw::ReadError| LayoutError::InvalidRenderedTree(e.to_string());
    if !scale.is_finite()
        || scale <= 0.
        || coordinates.iter().any(|v| !(-16384..=16384).contains(v))
    {
        return Err(LayoutError::InvalidRenderedTree(
            "invalid BASE font instance".into(),
        ));
    }
    let base = Base::read(table.into()).map_err(invalid)?;
    let Some(axis) = base.horiz_axis() else {
        return Ok(None);
    };
    let axis = axis.map_err(invalid)?;
    let Some(tags) = axis.base_tag_list() else {
        return Ok(None);
    };
    let tags = tags.map_err(invalid)?;
    let scripts = axis.base_script_list().map_err(invalid)?;
    let records = scripts.base_script_records();
    let Some(record) = records
        .iter()
        .find(|r| r.base_script_tag() == script)
        .or_else(|| {
            records
                .iter()
                .find(|r| r.base_script_tag() == Tag::new(b"DFLT"))
        })
    else {
        return Ok(None);
    };
    let script = record.base_script(scripts.offset_data()).map_err(invalid)?;
    let Some(values) = script.base_values() else {
        return Ok(None);
    };
    let values = values.map_err(invalid)?;
    if values.base_coord_count() as usize != tags.baseline_tags().len() {
        return Err(LayoutError::InvalidRenderedTree(
            "BASE coordinate/tag count mismatch".into(),
        ));
    }
    let coords = coordinates
        .iter()
        .map(|v| skrifa::raw::types::F2Dot14::from_bits(*v))
        .collect::<Vec<_>>();
    let mut result = BTreeMap::new();
    for (i, tag) in tags.baseline_tags().iter().enumerate() {
        let coord = values.base_coords().get(i).map_err(invalid)?;
        let mut position = f64::from(coord.coordinate());
        if let BaseCoord::Format3(coord) = coord {
            if let Some(device) = coord.device() {
                if let DeviceOrVariationIndex::VariationIndex(index) = device.map_err(invalid)? {
                    let store = base
                        .item_var_store()
                        .ok_or_else(|| {
                            LayoutError::InvalidRenderedTree("missing BASE variation store".into())
                        })?
                        .map_err(invalid)?;
                    position += f64::from(
                        store
                            .compute_delta(index.into(), &coords)
                            .map_err(invalid)?,
                    );
                }
                // Device pixel hinting does not alter unhinted CSS-pixel metrics.
            }
        }
        let position = position * scale;
        if !position.is_finite() {
            return Err(LayoutError::InvalidRenderedTree(
                "BASE coordinate overflow".into(),
            ));
        }
        if result.insert(tag.get(), position).is_some() {
            return Err(LayoutError::InvalidRenderedTree(
                "duplicate BASE baseline tag".into(),
            ));
        }
    }
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn variable_base_coordinates_apply_instance_delta_and_dflt_fallback() {
        let mut data = Vec::new();
        for v in [1u16, 1, 12, 0] {
            data.extend(v.to_be_bytes());
        }
        data.extend(54u32.to_be_bytes());
        for v in [4u16, 10, 1] {
            data.extend(v.to_be_bytes());
        }
        data.extend(b"hang");
        data.extend(1u16.to_be_bytes());
        data.extend(b"DFLT");
        for v in [8u16, 6, 0, 0, 0, 1, 6, 3, 500, 6, 0, 0, 0x8000] {
            data.extend(v.to_be_bytes());
        }
        assert_eq!(data.len(), 54);
        data.extend(1u16.to_be_bytes());
        data.extend(12u32.to_be_bytes());
        data.extend(1u16.to_be_bytes());
        data.extend(22u32.to_be_bytes());
        for v in [1u16, 1, 0, 16384, 16384, 1, 1, 1, 0, 100] {
            data.extend(v.to_be_bytes());
        }
        let read = |coords: &[i16]| {
            read_horizontal(&data, Tag::new(b"latn"), 0.05, coords)
                .unwrap()
                .unwrap()[&Tag::new(b"hang")]
        };
        assert_eq!(read(&[0]), 25.);
        assert_eq!(read(&[8192]), 27.5);
        assert_eq!(read(&[16384]), 30.);
        assert!(read_horizontal(&data, Tag::new(b"latn"), 0.05, &[20000]).is_err());
    }
    #[test]
    fn native_base_coordinates_resolve_script_offsets_and_scale() {
        let mut data = Vec::new();
        let mut push = |v: u16| data.extend(v.to_be_bytes());
        for v in [1, 0, 8, 0, 4, 18, 3] {
            push(v);
        }
        for tag in [b"hang", b"ideo", b"romn"] {
            data.extend(tag);
        }
        data.extend(1u16.to_be_bytes());
        data.extend(b"latn");
        data.extend(8u16.to_be_bytes());
        for v in [
            6u16,
            0,
            0,
            2,
            3,
            10,
            14,
            18,
            1,
            500,
            1,
            (-125i16) as u16,
            1,
            0,
        ] {
            data.extend(v.to_be_bytes());
        }
        let p = read_horizontal(&data, Tag::new(b"latn"), 0.05, &[])
            .unwrap()
            .unwrap();
        assert_eq!(p[&Tag::new(b"hang")], 25.);
        assert_eq!(p[&Tag::new(b"ideo")], -6.25);
        assert_eq!(p[&Tag::new(b"romn")], 0.);
        assert!(
            read_horizontal(&data, Tag::new(b"hani"), 0.05, &[])
                .unwrap()
                .is_none()
        );
        assert!(read_horizontal(&data[..10], Tag::new(b"latn"), 0.05, &[]).is_err());
    }
}
