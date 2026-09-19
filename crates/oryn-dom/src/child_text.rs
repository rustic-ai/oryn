//! HTML child text collection: only direct Text and CDATA children contribute.
use crate::{DomArena, DomError, NodeId, NodeKind, dom_string};
impl DomArena {
    pub fn child_text_units(&self, node: NodeId, max_units: usize) -> Result<Vec<u16>, DomError> {
        let mut result = Vec::new();
        for child in &self.get(node)?.children {
            if let NodeKind::Text { data } | NodeKind::CDataSection { data } =
                &self.get(*child)?.kind
            {
                // Bound the encoded storage before allocating its decoded temporary.
                if data.len() > max_units.saturating_mul(6).saturating_add(64) {
                    return Err(DomError::ChildTextCapacity);
                }
                let units = dom_string::units(data);
                if result
                    .len()
                    .checked_add(units.len())
                    .is_none_or(|n| n > max_units)
                {
                    return Err(DomError::ChildTextCapacity);
                }
                result.extend(units);
            }
        }
        Ok(result)
    }
    /// Trusted Types and DOM algorithms consume the original code units.
    /// Keep this distinct from scalar source transport to avoid changing policy
    /// inputs or comparing a replacement character with a trusted surrogate.
    pub fn child_text_dom_string(
        &self,
        node: NodeId,
        max_units: usize,
    ) -> Result<String, DomError> {
        Ok(dom_string::from_units(
            &self.child_text_units(node, max_units)?,
        ))
    }
    /// Source consumers accept Unicode scalar text, never the internal WTF16 encoding.
    /// Concatenate before conversion so split surrogate pairs remain intact.
    pub fn child_text_source(&self, node: NodeId, max_bytes: usize) -> Result<String, DomError> {
        let units = self.child_text_units(node, max_bytes)?;
        let mut source = String::new();
        for scalar in char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)) {
            if source.len().saturating_add(scalar.len_utf8()) > max_bytes {
                return Err(DomError::ChildTextCapacity);
            }
            source.push(scalar);
        }
        Ok(source)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_ignores_nested_elements_and_joins_split_surrogates_before_conversion() {
        let mut dom = crate::DomRuntime::new();
        let script = dom.create_element("script");
        let first = dom.create_text(dom_string::from_units(&[0xd83d]));
        let nested = dom.create_element("span");
        let hidden = dom.create_text("throw Error('nested')");
        dom.append_child(nested, hidden).unwrap();
        let last = dom.create_text(dom_string::from_units(&[0xde00, 0xd800]));
        for child in [first, nested, last] {
            dom.append_child(script, child).unwrap();
        }
        assert_eq!(
            dom.arena().child_text_units(script, 3).unwrap(),
            vec![0xd83d, 0xde00, 0xd800]
        );
        assert_eq!(
            dom_string::units(&dom.arena().child_text_dom_string(script, 3).unwrap()),
            vec![0xd83d, 0xde00, 0xd800]
        );
        assert_eq!(dom.arena().child_text_source(script, 7).unwrap(), "😀�");
        assert!(matches!(
            dom.arena().child_text_source(script, 6),
            Err(DomError::ChildTextCapacity)
        ));
    }
}
