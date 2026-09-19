//! DOM namespace lookup, using native element/attribute names and ancestor links.
use super::*;
impl DomRuntime {
    fn namespace_element(&self, node: NodeId) -> Result<Option<NodeId>, DomError> {
        let record = self.arena().get(node)?;
        Ok(match &record.kind {
            NodeKind::Element { .. } => Some(node),
            NodeKind::Document => record.children.iter().copied().find(|n| {
                self.arena()
                    .get(*n)
                    .is_ok_and(|r| matches!(r.kind, NodeKind::Element { .. }))
            }),
            NodeKind::Attribute { owner, .. } => *owner,
            NodeKind::DocumentType { .. }
            | NodeKind::DocumentFragment
            | NodeKind::ShadowRoot { .. } => None,
            _ => {
                let mut parent = record.parent;
                while let Some(id) = parent {
                    let record = self.arena().get(id)?;
                    if matches!(record.kind, NodeKind::Element { .. }) {
                        return Ok(Some(id));
                    }
                    parent = record.parent;
                }
                None
            }
        })
    }
    pub fn lookup_namespace_uri(
        &self,
        node: NodeId,
        prefix: Option<&str>,
    ) -> Result<Option<String>, DomError> {
        let prefix = prefix.filter(|v| !v.is_empty());
        let mut at = self.namespace_element(node)?;
        while let Some(id) = at {
            let record = self.arena().get(id)?;
            if let NodeKind::Element { name } = &record.kind
                && !name.ns.is_empty()
                && name.prefix.as_ref().map(|p| p.as_ref()) == prefix
            {
                return Ok(Some(name.ns.to_string()));
            }
            for attribute in record.attributes.records() {
                if attribute.namespace.as_deref() != Some("http://www.w3.org/2000/xmlns/") {
                    continue;
                }
                if (prefix.is_none() && attribute.name == "xmlns")
                    || (attribute.prefix.as_deref() == Some("xmlns")
                        && prefix == Some(attribute.local_name.as_str()))
                {
                    return Ok((!attribute.value.is_empty()).then(|| attribute.value.clone()));
                }
            }
            at = record.parent.filter(|parent| {
                self.arena()
                    .get(*parent)
                    .is_ok_and(|r| matches!(r.kind, NodeKind::Element { .. }))
            });
        }
        Ok(None)
    }
    pub fn lookup_prefix(
        &self,
        node: NodeId,
        namespace: Option<&str>,
    ) -> Result<Option<String>, DomError> {
        let Some(namespace) = namespace.filter(|v| !v.is_empty()) else {
            return Ok(None);
        };
        let Some(original) = self.namespace_element(node)? else {
            return Ok(None);
        };
        let mut at = Some(original);
        while let Some(id) = at {
            let record = self.arena().get(id)?;
            if let NodeKind::Element { name } = &record.kind
                && name.ns.as_ref() == namespace
                && let Some(prefix) = name.prefix.as_ref()
                && self
                    .lookup_namespace_uri(original, Some(prefix.as_ref()))?
                    .as_deref()
                    == Some(namespace)
            {
                return Ok(Some(prefix.to_string()));
            }
            for a in record.attributes.records() {
                if a.namespace.as_deref() == Some("http://www.w3.org/2000/xmlns/")
                    && a.prefix.as_deref() == Some("xmlns")
                    && a.value == namespace
                    && self
                        .lookup_namespace_uri(original, Some(&a.local_name))?
                        .as_deref()
                        == Some(namespace)
                {
                    return Ok(Some(a.local_name.clone()));
                }
            }
            at = record.parent.filter(|parent| {
                self.arena()
                    .get(*parent)
                    .is_ok_and(|r| matches!(r.kind, NodeKind::Element { .. }))
            });
        }
        Ok(None)
    }
    pub fn is_default_namespace(
        &self,
        node: NodeId,
        namespace: Option<&str>,
    ) -> Result<bool, DomError> {
        Ok(
            self.lookup_namespace_uri(node, None)?.as_deref()
                == namespace.filter(|v| !v.is_empty()),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespace_shadowing_undeclaration_and_prefix_validation_are_live() {
        let mut dom = html::parse("<body><div id=outer><span id=inner>text</span></div></body>")
            .into_runtime()
            .unwrap();
        let outer = dom
            .get_element_by_id(dom.document(), "outer")
            .unwrap()
            .unwrap();
        let inner = dom
            .get_element_by_id(dom.document(), "inner")
            .unwrap()
            .unwrap();
        dom.set_attribute_ns(
            outer,
            Some("http://www.w3.org/2000/xmlns/".into()),
            "xmlns:p",
            "urn:outer",
        )
        .unwrap();
        assert_eq!(
            dom.lookup_namespace_uri(inner, Some("p"))
                .unwrap()
                .as_deref(),
            Some("urn:outer")
        );
        assert_eq!(
            dom.lookup_prefix(inner, Some("urn:outer"))
                .unwrap()
                .as_deref(),
            Some("p")
        );
        dom.set_attribute_ns(
            inner,
            Some("http://www.w3.org/2000/xmlns/".into()),
            "xmlns:p",
            "urn:inner",
        )
        .unwrap();
        assert_eq!(dom.lookup_prefix(inner, Some("urn:outer")).unwrap(), None);
        dom.set_attribute_ns(
            inner,
            Some("http://www.w3.org/2000/xmlns/".into()),
            "xmlns:p",
            "",
        )
        .unwrap();
        assert_eq!(dom.lookup_namespace_uri(inner, Some("p")).unwrap(), None);
    }
}

#[cfg(test)]
mod xpath_revision_tests {
    use super::*;
    #[test]
    fn document_revision_isolates_nonstructural_activity_and_never_reuses_identity() {
        let mut dom = html::parse("<body><div id=t></div></body>")
            .into_runtime()
            .unwrap();
        let document = dom.document();
        let target = dom.get_element_by_id(document, "t").unwrap().unwrap();
        let first = dom.xpath_document_revision(document).unwrap();
        assert!(dom.xpath_document_revision_matches(document, &first));
        dom.set_attribute(target, "data-x", "1").unwrap();
        assert!(!dom.xpath_document_revision_matches(document, &first));
        let second = dom.xpath_document_revision(document).unwrap();
        assert!(!Rc::ptr_eq(&first, &second));
        dom.remove_attribute(target, "data-x").unwrap();
        let third = dom.xpath_document_revision(document).unwrap();
        assert!(!Rc::ptr_eq(&first, &third) && !Rc::ptr_eq(&second, &third));
    }
}
