//! ProcessingInstruction pseudo-attributes are ordered live CharacterData.
use super::*;
use crate::projection_allocation::{
    OrdinaryProjectionAllocation, ProjectionAllocation, admitted_append, admitted_push,
};
fn space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}
fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && !value
            .chars()
            .any(|ch| space(ch) || matches!(ch, '\0' | '/' | '=' | '>'))
}
fn copy<A: ProjectionAllocation>(
    value: &str,
    allocation: &mut A,
    check: &mut impl FnMut() -> Result<(), DomError>,
) -> Result<String, A::Error> {
    let mut result = String::new();
    admitted_append(&mut result, value, allocation, check)?;
    Ok(result)
}

fn parsed<A: ProjectionAllocation>(
    data: &str,
    allocation: &mut A,
    check: &mut impl FnMut() -> Result<(), DomError>,
) -> Result<Vec<(String, String)>, A::Error> {
    let encoded = crate::html::encode_dom_string_with_allocation(data, allocation, check)?;
    let mut rest = encoded.as_str();
    let mut values = Vec::new();
    loop {
        check().map_err(A::Error::from)?;
        rest = rest.trim_start_matches(space);
        if rest.is_empty() {
            return Ok(values);
        }
        let end = rest.find(|ch| space(ch) || ch == '=').unwrap_or(rest.len());
        let name = &rest[..end];
        if !valid_name(name) {
            return Ok(Vec::new());
        }
        rest = rest[end..].trim_start_matches(space);
        let Some(tail) = rest.strip_prefix('=') else {
            return Ok(Vec::new());
        };
        rest = tail.trim_start_matches(space);
        let Some(quote @ ('\'' | '"')) = rest.chars().next() else {
            return Ok(Vec::new());
        };
        rest = &rest[quote.len_utf8()..];
        let Some(end) = rest.find(quote) else {
            return Ok(Vec::new());
        };
        // Canonical character-reference state machine, borrowed input, admitted output.
        let allocation_cell = RefCell::new(&mut *allocation);
        let value = html5ever::tokenizer::decode_attribute_references(
            &rest[..end],
            &mut |bytes| allocation_cell.borrow_mut().reserve(bytes),
            &mut || check().map_err(A::Error::from),
            &mut |error| allocation_cell.borrow().allocation_error(error),
        )?;
        drop(allocation_cell);
        let key = crate::html::decode_dom_string_with_allocation(name, allocation, check)?;
        let value = crate::html::decode_dom_string_with_allocation(&value, allocation, check)?;
        if !values.iter().any(|(existing, _)| existing == &key) {
            admitted_push(&mut values, (key, value), allocation, check)?;
        }
        rest = &rest[end + quote.len_utf8()..];
        if !rest.is_empty() && !rest.starts_with(space) {
            return Ok(Vec::new());
        }
    }
}
impl DomArena {
    pub fn processing_instruction_attributes_admitted<A: ProjectionAllocation>(
        &self,
        node: NodeId,
        allocation: &mut A,
        check: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Vec<(String, String)>, A::Error> {
        let NodeKind::ProcessingInstruction { data, .. } =
            &self.get(node).map_err(A::Error::from)?.kind
        else {
            return Err(A::Error::from(DomError::StaleNode(node)));
        };
        parsed(data, allocation, check)
    }
    pub fn processing_instruction_attributes(
        &self,
        node: NodeId,
    ) -> Result<Vec<(String, String)>, DomError> {
        self.processing_instruction_attributes_admitted(
            node,
            &mut OrdinaryProjectionAllocation,
            &mut || Ok(()),
        )
    }
}
impl DomRuntime {
    pub fn prepare_processing_instruction_attribute<A: ProjectionAllocation>(
        &self,
        node: NodeId,
        name: &str,
        value: Option<&str>,
        allocation: &mut A,
        check: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Option<String>, A::Error> {
        let mut values = self
            .arena
            .processing_instruction_attributes_admitted(node, allocation, check)?;
        match (values.iter().position(|(key, _)| key == name), value) {
            (Some(index), Some(value)) => values[index].1 = copy(value, allocation, check)?,
            (None, Some(value)) => {
                let key = copy(name, allocation, check)?;
                let value = copy(value, allocation, check)?;
                admitted_push(&mut values, (key, value), allocation, check)?;
            }
            (Some(index), None) => {
                values.remove(index);
            }
            (None, None) => return Ok(None),
        }
        let xml = self.node_document(node).map_err(A::Error::from)?
            .and_then(|document| self.arena.get(document).ok())
            .and_then(|record| record.document_metadata.as_ref())
            .is_some_and(|metadata| !metadata.is_html);
        let mut output = String::new();
        for (index, (key, value)) in values.iter().enumerate() {
            if index > 0 {
                admitted_append(&mut output, " ", allocation, check)?;
            }
            let key = crate::html::encode_dom_string_with_allocation(key, allocation, check)?;
            admitted_append(&mut output, &key, allocation, check)?;
            admitted_append(&mut output, "=\"", allocation, check)?;
            // Match the owning document's attribute serialization algorithm.
            let value = crate::html::encode_dom_string_with_allocation(value, allocation, check)?;
            for ch in value.chars() {
                let mut bytes = [0; 4];
                let text = match ch {
                    '&' => "&amp;",
                    '"' => "&quot;",
                    '\u{a0}' if !xml => "&nbsp;",
                    '<' if xml => "&lt;",
                    '>' if xml => "&gt;",
                    '\t' if xml => "&#x9;",
                    '\n' if xml => "&#xA;",
                    '\r' if xml => "&#xD;",
                    _ => ch.encode_utf8(&mut bytes),
                };
                admitted_append(&mut output, text, allocation, check)?;
            }
            admitted_append(&mut output, "\"", allocation, check)?;
        }
        Ok(Some(crate::html::decode_dom_string_with_allocation(
            &output, allocation, check,
        )?))
    }
    pub fn set_processing_instruction_attribute(
        &mut self,
        node: NodeId,
        name: &str,
        value: Option<&str>,
    ) -> Result<(), DomError> {
        if let Some(data) = self.prepare_processing_instruction_attribute(
            node,
            name,
            value,
            &mut OrdinaryProjectionAllocation,
            &mut || Ok(()),
        )? {
            self.set_text(node, data)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pi_order_data_escape_and_character_data_journal() {
        let mut dom = DomRuntime::new();
        let pi = dom.create_processing_instruction("t", "a=\"b\" x=\"yy\"");
        dom.set_processing_instruction_attribute(pi, "x", Some("a\"&<>"))
            .unwrap();
        assert_eq!(
            dom.arena.processing_instruction_attributes(pi).unwrap(),
            vec![("a".into(), "b".into()), ("x".into(), "a\"&<>".into())]
        );
        assert!(
            matches!(dom.drain_mutations().last(),Some(Mutation::CharacterData{node,..}) if *node==pi)
        );
        dom.set_processing_instruction_attribute(pi, "x", None)
            .unwrap();
        dom.set_processing_instruction_attribute(pi, "$foo", Some("1"))
            .unwrap();
        dom.set_processing_instruction_attribute(pi, "_bar", Some("2"))
            .unwrap();
        assert_eq!(
            dom.arena
                .processing_instruction_attributes(pi)
                .unwrap()
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "$foo", "_bar"]
        );
        dom.set_text(pi, "a=b x=\"yy\"").unwrap();
        assert!(
            dom.arena
                .processing_instruction_attributes(pi)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn preparation_failure_never_changes_pi_data_or_journal() {
        struct Failure;
        impl ProjectionAllocation for Failure {
            type Error = DomError;
            fn reserve(&mut self, _: usize) -> Result<(), DomError> {
                Err(DomError::SemanticProjectionCapacity)
            }
            fn allocation_error(&self, _: std::collections::TryReserveError) -> DomError {
                DomError::SemanticProjectionCapacity
            }
        }
        let mut dom = DomRuntime::new();
        let pi = dom.create_processing_instruction("t", "a=\"b\"");
        dom.drain_mutations();
        assert!(
            dom.prepare_processing_instruction_attribute(
                pi,
                "new",
                Some("value"),
                &mut Failure,
                &mut || Ok(())
            )
            .is_err()
        );
        assert!(dom.drain_mutations().is_empty());
        assert_eq!(
            dom.arena.processing_instruction_attributes(pi).unwrap(),
            vec![("a".into(), "b".into())]
        );
    }
}
