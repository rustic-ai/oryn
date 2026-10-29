//! Interest relationships and selector state belong to the native DOM.
use super::*;

impl DomRuntime {
    pub fn set_parser_shadow_disabled(
        &mut self,
        document: NodeId,
        name: String,
    ) -> Result<(), DomError> {
        self.arena.document_id(document)?;
        self.parser_shadow_disabled
            .entry(document)
            .or_default()
            .insert(name);
        Ok(())
    }
    pub fn parser_shadow_disabled(&self, document: NodeId, name: &str) -> bool {
        self.parser_shadow_disabled
            .get(&document)
            .is_some_and(|names| names.contains(name))
    }

    pub fn shadow_reference_target(&self, node: NodeId) -> Result<Option<&str>, DomError> {
        let record = self.arena.get(node)?;
        if !matches!(record.kind, NodeKind::ShadowRoot { .. }) {
            return Err(DomError::NotAnElement(node));
        }
        Ok(record.shadow_reference_target.as_deref())
    }
    pub fn set_shadow_reference_target(
        &mut self,
        node: NodeId,
        value: Option<String>,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        if !matches!(record.kind, NodeKind::ShadowRoot { .. }) {
            return Err(DomError::NotAnElement(node));
        }
        if record.shadow_reference_target != value {
            record.shadow_reference_target = value;
            self.mutations.push(Mutation::FormState { node });
        }
        Ok(())
    }
    pub fn resolve_shadow_reference_target(
        &self,
        target: NodeId,
    ) -> Result<Option<NodeId>, DomError> {
        let mut target = target;
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(target) {
                return Ok(None);
            }
            let shadow = self
                .arena
                .get(target)?
                .children
                .iter()
                .copied()
                .find(|node| {
                    self.arena
                        .get(*node)
                        .is_ok_and(|record| matches!(record.kind, NodeKind::ShadowRoot { .. }))
                });
            let Some(shadow) = shadow else {
                return Ok(Some(target));
            };
            let Some(id) = self.shadow_reference_target(shadow)? else {
                return Ok(Some(target));
            };
            if id.is_empty() {
                return Ok(None);
            }
            let Some(next) = self.get_element_by_id(shadow, id)? else {
                return Ok(None);
            };
            target = next;
        }
    }
    pub fn supports_interest_invocation(&self, node: NodeId) -> bool {
        self.arena.get(node).is_ok_and(|record|matches!(&record.kind,NodeKind::Element{name}
            if (name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&matches!(name.local.as_ref(),"a"|"area"|"button"))
            ||(name.ns.as_ref()=="http://www.w3.org/2000/svg"&&name.local.as_ref()=="a")))
    }
    /// The shared native element-reference store supplies GC tracing and
    /// invalidation on attribute writes; interestfor is a single ID, not an
    /// ARIA whitespace-separated ID list.
    pub fn interest_for_element(&self, source: NodeId) -> Result<Option<NodeId>, DomError> {
        let record = self.arena.get(source)?;
        if !self.supports_interest_invocation(source) {
            return Ok(None);
        }
        if let Some(references) = record.explicit_accessibility_references("interestfor") {
            let Some(target) = references.first().copied() else {
                return Ok(None);
            };
            let target_root = self.control_tree_root(target)?;
            let mut cursor = Some(source);
            let mut visited = BTreeSet::new();
            while let Some(node) = cursor.filter(|node| visited.insert(*node)) {
                if node == target_root {
                    return Ok(Some(target));
                }
                // Shadow roots retain their host as parent. Crossing outward
                // permits a shadow invoker to reference the containing tree.
                cursor = self.arena.get(node)?.parent;
            }
            return Ok(None);
        }
        let Some(id) = record
            .attributes
            .get_null("interestfor")
            .filter(|id| !id.is_empty())
        else {
            return Ok(None);
        };
        self.get_element_by_id(self.control_tree_root(source)?, id)
    }
    pub fn set_interest_for_element(
        &mut self,
        source: NodeId,
        target: Option<NodeId>,
    ) -> Result<(), DomError> {
        if !self.supports_interest_invocation(source) {
            return Err(DomError::NotAnElement(source));
        }
        self.set_accessibility_references(
            source,
            "interestfor",
            false,
            target.map(|node| vec![node]),
        )
    }
    pub fn command_for_element(&self, source: NodeId) -> Result<Option<NodeId>, DomError> {
        if !self.is_element(source, "http://www.w3.org/1999/xhtml", "button") {
            return Ok(None);
        }
        self.invoker_reference_element(source, "commandfor")
    }
    pub fn popover_target_element(&self, source: NodeId) -> Result<Option<NodeId>, DomError> {
        if !self.is_element(source, "http://www.w3.org/1999/xhtml", "button")
            && !self.is_element(source, "http://www.w3.org/1999/xhtml", "input") {
            return Ok(None);
        }
        self.invoker_reference_element(source, "popovertarget")
    }
    fn invoker_reference_element(&self, source: NodeId, attribute: &str) -> Result<Option<NodeId>, DomError> {
        let record = self.arena.get(source)?;
        if let Some(references) = record.explicit_accessibility_references(attribute) {
            let Some(target) = references.first().copied() else {
                return Ok(None);
            };
            let target_root = self.control_tree_root(target)?;
            let mut cursor = Some(source);
            let mut visited = BTreeSet::new();
            while let Some(node) = cursor.filter(|node| visited.insert(*node)) {
                if node == target_root {
                    return Ok(Some(target));
                }
                // Shadow roots retain their host as parent. Crossing outward
                // permits a shadow invoker to reference the containing tree.
                cursor = self.arena.get(node)?.parent;
            }
            return Ok(None);
        }
        let Some(id) = record
            .attributes
            .get_null(attribute)
            .filter(|id| !id.is_empty())
        else {
            return Ok(None);
        };
        self.get_element_by_id(self.control_tree_root(source)?, id)
    }
    pub fn set_command_for_element(
        &mut self,
        source: NodeId,
        target: Option<NodeId>,
    ) -> Result<(), DomError> {
        if !self.is_element(source, "http://www.w3.org/1999/xhtml", "button") {
            return Err(DomError::NotAnElement(source));
        }
        self.set_accessibility_references(
            source,
            "commandfor",
            false,
            target.map(|node| vec![node]),
        )
    }
    pub fn interest_invoker_target(&self, source: NodeId) -> Result<Option<NodeId>, DomError> {
        if !self.supports_interest_invocation(source)
            || !self.is_connected(source)?
            || self.is_inert(source)?
        {
            return Ok(None);
        }
        if self.is_element(source, "http://www.w3.org/1999/xhtml", "button")
            && self.control_disabled(source)?
        {
            return Ok(None);
        }
        let target = match self.interest_for_element(source)? {
            Some(target) => self.resolve_shadow_reference_target(target)?,
            None => None,
        };
        Ok(target.filter(|target| {
            *target != source
                && self.is_connected(*target).unwrap_or(false)
                && self.node_document(*target).ok() == self.node_document(source).ok()
        }))
    }
    pub fn element_interest_source(&self, node: NodeId) -> Result<bool, DomError> {
        Ok(self.arena.get(node)?.interest_source)
    }
    pub fn element_interest_target(&self, node: NodeId) -> Result<bool, DomError> {
        Ok(self.arena.get(node)?.interest_target)
    }
    pub fn set_interest_selector_state(
        &mut self,
        node: NodeId,
        source: bool,
        target: bool,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        if record.interest_source != source || record.interest_target != target {
            record.interest_source = source;
            record.interest_target = target;
            self.mutations.push(Mutation::FormState { node });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_reference_retention_attribute_reset_and_clone_state() {
        let mut dom = DomRuntime::new();
        let source = dom.create_element("button");
        let target = dom.create_element("div");
        dom.retain_native(source).unwrap();
        dom.retain_native(target).unwrap();
        dom.set_interest_for_element(source, Some(target)).unwrap();
        assert_eq!(
            dom.attribute_in_null_namespace(source, "interestfor")
                .unwrap(),
            Some("")
        );
        dom.release_native(target).unwrap();
        assert!(dom.arena().get(target).is_ok());
        dom.set_interest_selector_state(source, true, false)
            .unwrap();
        assert!(dom.matches_selector(source, ":interest-source").unwrap());
        let clone = dom.clone_node(source, false).unwrap();
        assert!(!dom.element_interest_source(clone).unwrap());
        dom.set_attribute(source, "interestfor", "missing").unwrap();
        assert!(dom.arena().get(target).is_err());
        assert_eq!(dom.interest_for_element(source).unwrap(), None);
    }
    #[test]
    fn native_invoker_eligibility_and_exact_single_id() {
        let parsed = html::parse(
            "<body><button id=s interestfor='a b'></button><div id='a b'></div><input id=i interestfor='a b'></body>",
        );
        let mut dom = parsed.into_runtime().unwrap();
        let root = dom.document();
        let source = dom.get_element_by_id(root, "s").unwrap().unwrap();
        let target = dom.get_element_by_id(root, "a b").unwrap().unwrap();
        assert_eq!(dom.interest_invoker_target(source).unwrap(), Some(target));
        let input = dom.get_element_by_id(root, "i").unwrap().unwrap();
        assert!(!dom.supports_interest_invocation(input));
        dom.set_attribute(source, "disabled", "").unwrap();
        assert_eq!(dom.interest_invoker_target(source).unwrap(), None);
    }
    #[test]
    fn declarative_reference_target_resolves_without_exposing_shadow_target() {
        let parsed = html::parse(
            "<body><button id=s interestfor=h></button><div id=h><template shadowrootmode=open shadowrootreferencetarget=t><div id=t></div></template></div></body>",
        );
        let mut dom = parsed.into_runtime().unwrap();
        let document = dom.document();
        let source = dom.get_element_by_id(document, "s").unwrap().unwrap();
        let host = dom.get_element_by_id(document, "h").unwrap().unwrap();
        let shadow = dom
            .arena()
            .get(host)
            .unwrap()
            .children
            .iter()
            .copied()
            .find(|node| {
                matches!(
                    dom.arena().get(*node).unwrap().kind,
                    NodeKind::ShadowRoot { .. }
                )
            })
            .unwrap();
        let target = dom.get_element_by_id(shadow, "t").unwrap().unwrap();
        assert_eq!(dom.interest_for_element(source).unwrap(), Some(host));
        assert_eq!(dom.interest_invoker_target(source).unwrap(), Some(target));
        dom.set_shadow_reference_target(shadow, None).unwrap();
        assert_eq!(dom.interest_invoker_target(source).unwrap(), Some(host));
        dom.set_shadow_reference_target(shadow, Some(String::new()))
            .unwrap();
        assert_eq!(dom.interest_invoker_target(source).unwrap(), None);
    }
}
