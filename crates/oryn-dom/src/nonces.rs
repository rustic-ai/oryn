//! The nonce IDL value is independent of the public content attribute.
use super::*;
impl DomRuntime {
    pub fn element_nonce_for_csp(&self, node: NodeId) -> Result<Option<&str>, DomError> {
        let record = self.arena.get(node)?;
        if record.parser_duplicate_attributes {
            return Ok(None);
        }
        if matches!(&record.kind,NodeKind::Element{name} if matches!(name.local.as_ref(),"script"|"style"|"link"))
            && record.attributes.iter().any(|(name, value)| {
                [name.as_str(), value.as_str()].iter().any(|value| {
                    value
                        .as_bytes()
                        .windows(5)
                        .any(|part| part.eq_ignore_ascii_case(b"<link"))
                        || value
                            .as_bytes()
                            .windows(6)
                            .any(|part| part.eq_ignore_ascii_case(b"<style"))
                        || value
                            .as_bytes()
                            .windows(7)
                            .any(|part| part.eq_ignore_ascii_case(b"<script"))
                })
            })
        {
            return Ok(None);
        }
        self.element_nonce(node)
    }

    pub fn element_nonce(&self, node: NodeId) -> Result<Option<&str>, DomError> {
        let record = self.arena.get(node)?;
        Ok(record
            .cryptographic_nonce
            .as_deref()
            .or_else(|| record.attributes.get_null("nonce").map(String::as_str)))
    }
    pub fn set_element_nonce(&mut self, node: NodeId, value: String) -> Result<(), DomError> {
        self.arena.get_mut(node)?.cryptographic_nonce = Some(value);
        Ok(())
    }
    pub fn set_document_nonce_hiding(
        &mut self,
        document: NodeId,
        hide: bool,
    ) -> Result<(), DomError> {
        let previous = self
            .arena
            .get(document)?
            .document_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.hide_nonces);
        self.arena
            .get_mut(document)?
            .document_metadata
            .get_or_insert_with(|| Box::new(DocumentMetadata::default()))
            .hide_nonces = hide;
        if hide && !previous {
            for node in self.arena.descendants_inclusive(document)? {
                if self.arena.node_document(node)? == Some(document) && self.is_connected(node)? {
                    self.hide_nonce_on_insertion(node)?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn hide_nonce_on_insertion(&mut self, node: NodeId) -> Result<(), DomError> {
        let Some(document) = self.arena.node_document(node)? else {
            return Ok(());
        };
        if !self
            .arena
            .get(document)?
            .document_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.hide_nonces)
        {
            return Ok(());
        }
        let record = self.arena.get(node)?;
        if !matches!(&record.kind,NodeKind::Element{name} if matches!(name.ns.as_ref(),"http://www.w3.org/1999/xhtml"|"http://www.w3.org/2000/svg"))
            || !record.attributes.contains_null("nonce")
        {
            return Ok(());
        }
        let nonce = self.element_nonce(node)?.map(str::to_owned);
        let Some(old) = record
            .attributes
            .namespaced(None, "nonce")
            .filter(|entry| !entry.value.is_empty())
            .cloned()
        else {
            return Ok(());
        };
        let key = old.key();
        let attribute = self
            .arena
            .attribute_nodes
            .get(&(node, key.clone()))
            .copied();
        if let Some(attribute) = attribute {
            self.arena.get(attribute)?;
        }
        let mut data = old.clone();
        data.value.clear();
        let data = Arc::new(data);
        // Hiding is a native nonce operation. Validate the materialized Attr
        // before publishing either view, then commit both views and its one
        // attribute journal row under the same uninterrupted native borrow.
        self.arena.get_mut(node)?.attributes.insert(data.clone());
        self.arena.get_mut(node)?.cryptographic_nonce = nonce;
        if let Some(attribute) = attribute
            && let NodeKind::Attribute { data: current, .. } =
                &mut self.arena.get_mut(attribute)?.kind
        {
            *current = data;
        }
        self.record_mutation(Mutation::Attribute {
            node,
            name: key.local_name,
            namespace: None,
            old_value: Some(old.value.clone()),
            new_value: Some(String::new()),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn header_policy_hides_on_insertion_and_cloning_retains_internal_nonce() {
        let mut dom = html::parse("<body></body>").into_runtime().unwrap();
        let root = dom.document();
        dom.set_document_nonce_hiding(root, true).unwrap();
        let body = dom.query_selector(root, "body").unwrap().unwrap();
        let script = dom.create_element("script");
        dom.set_attribute_ns(script, None, "nonce", "abc").unwrap();
        assert_eq!(dom.attribute(script, "nonce").unwrap(), Some("abc"));
        dom.append_child(body, script).unwrap();
        assert_eq!(dom.attribute(script, "nonce").unwrap(), Some(""));
        assert_eq!(dom.element_nonce(script).unwrap(), Some("abc"));
        let clone = dom.clone_node(script, false).unwrap();
        assert_eq!(dom.element_nonce(clone).unwrap(), Some("abc"));
        dom.append_child(body, clone).unwrap();
        assert_eq!(dom.element_nonce(clone).unwrap(), Some("abc"));
        dom.set_attribute_ns(script, None, "nonce", "foo").unwrap();
        assert_eq!(dom.attribute(script, "nonce").unwrap(), Some("foo"));
        assert_eq!(dom.element_nonce(script).unwrap(), Some("foo"));
        dom.set_element_nonce(script, "bar".into()).unwrap();
        assert_eq!(dom.attribute(script, "nonce").unwrap(), Some("foo"));
        assert_eq!(dom.element_nonce(script).unwrap(), Some("bar"));
        dom.remove_child(body, script).unwrap();
        dom.append_child(body, script).unwrap();
        assert_eq!(dom.attribute(script, "nonce").unwrap(), Some(""));
        assert_eq!(dom.element_nonce(script).unwrap(), Some("bar"));
    }
}
