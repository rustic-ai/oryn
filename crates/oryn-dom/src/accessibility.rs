//! Default ARIA semantics are native state, never content attributes.
use super::*;
impl Node {
    pub fn accessibility_attribute(&self, name: &str) -> Option<&String> {
        self.attributes
            .get_null(name)
            .or_else(|| self.default_accessibility.get(name))
            .or_else(|| {
                if name == "role" {
                    self.submit_behavior.as_ref().map(|behavior| &behavior.role)
                } else {
                    None
                }
            })
    }
    pub fn default_accessibility_attribute(&self, name: &str) -> Option<&String> {
        self.default_accessibility.get(name)
    }
    pub fn accessibility_attributes(&self) -> std::borrow::Cow<'_, AttributeList> {
        if self.default_accessibility.is_empty() && self.submit_behavior.is_none() {
            return std::borrow::Cow::Borrowed(&self.attributes);
        }
        let mut attributes = self.attributes.clone();
        for (name, value) in &self.default_accessibility {
            if !attributes.contains_null(name) {
                attributes.insert(Arc::new(ContentAttribute {
                    namespace: None,
                    local_name: name.clone(),
                    name: name.clone(),
                    prefix: None,
                    value: value.clone(),
                }));
            }
        }
        if let Some(behavior) = &self.submit_behavior
            && !attributes.contains_null("role")
        {
            attributes.insert(Arc::new(ContentAttribute {
                namespace: None,
                local_name: "role".into(),
                name: "role".into(),
                prefix: None,
                value: behavior.role.clone(),
            }));
        }
        std::borrow::Cow::Owned(attributes)
    }
}
impl DomRuntime {
    pub fn set_default_accessibility_attribute(
        &mut self,
        node: NodeId,
        name: &str,
        value: Option<String>,
    ) -> Result<(), DomError> {
        let values = &mut self.arena.get_mut(node)?.default_accessibility;
        let changed = match value {
            Some(value) => {
                if values.get(name) == Some(&value) {
                    false
                } else {
                    values.insert(name.into(), value);
                    true
                }
            }
            None => values.remove(name).is_some(),
        };
        if changed {
            let record = self.arena.get_mut(node)?;
            record.accessibility_reference_revision =
                record.accessibility_reference_revision.wrapping_add(1);
            self.mutations.push(Mutation::FormState { node });
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authored_values_override_defaults_without_changing_dom_or_clone() {
        let mut dom = DomRuntime::new();
        let node = dom.create_element("x-control");
        dom.set_default_accessibility_attribute(node, "role", Some("button".into()))
            .unwrap();
        assert!(
            dom.attribute_in_null_namespace(node, "role")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            dom.arena()
                .get(node)
                .unwrap()
                .accessibility_attribute("role")
                .map(String::as_str),
            Some("button")
        );
        dom.set_attribute(node, "role", "").unwrap();
        assert_eq!(
            dom.arena()
                .get(node)
                .unwrap()
                .accessibility_attribute("role")
                .map(String::as_str),
            Some("")
        );
        let cloned = dom.clone_node(node, false).unwrap();
        assert!(
            dom.arena()
                .get(cloned)
                .unwrap()
                .default_accessibility_attribute("role")
                .is_none()
        );
    }
}

impl Node {
    pub fn accessibility_reference_revision(&self) -> u64 {
        self.accessibility_reference_revision
    }
    pub fn default_accessibility_references(&self, name: &str) -> Option<&[NodeId]> {
        self.default_accessibility_references
            .get(name)
            .map(Vec::as_slice)
    }
    pub fn explicit_accessibility_references(&self, name: &str) -> Option<&[NodeId]> {
        self.explicit_accessibility_references
            .get(name)
            .map(Vec::as_slice)
    }
}
impl DomArena {
    fn accessibility_tree_root(&self, mut node: NodeId) -> Result<NodeId, DomError> {
        loop {
            let record = self.get(node)?;
            if matches!(
                record.kind,
                NodeKind::ShadowRoot { .. } | NodeKind::Document
            ) {
                return Ok(node);
            }
            match record.parent {
                Some(parent) => node = parent,
                None => return Ok(node),
            }
        }
    }
    pub fn accessibility_reference_in_scope(
        &self,
        source: NodeId,
        target: NodeId,
        internals: bool,
    ) -> bool {
        let Ok(root) = self.accessibility_tree_root(target) else {
            return false;
        };
        let mut cursor = Some(source);
        while let Some(node) = cursor {
            if node == root {
                return true;
            }
            cursor = self.get(node).ok().and_then(|node| node.parent);
        }
        if internals {
            let mut cursor = Some(target);
            while let Some(node) = cursor {
                if node == source {
                    return true;
                }
                cursor = self.get(node).ok().and_then(|node| node.parent);
            }
        }
        false
    }
    /// Effective native relation: authored/explicit element state suppresses
    /// defaults even when its references are empty, missing or out of scope.
    pub fn accessibility_relation(&self, source: NodeId, name: &str) -> Option<Vec<NodeId>> {
        let node = self.get(source).ok()?;
        if let Some(targets) = node.explicit_accessibility_references(name) {
            return Some(
                targets
                    .iter()
                    .copied()
                    .filter(|target| self.accessibility_reference_in_scope(source, *target, false))
                    .collect(),
            );
        }
        if let Some(ids) = node.attributes.get_null(name) {
            let root = self.accessibility_tree_root(source).ok()?;
            let mut result = Vec::new();
            for token in ids.split_ascii_whitespace() {
                if let Some(target) =
                    self.descendants_inclusive(root)
                        .ok()?
                        .into_iter()
                        .find(|target| {
                            self.get(*target).is_ok_and(|node| {
                                node.attributes.get_null("id").is_some_and(|id| id == token)
                            }) && self.accessibility_tree_root(*target).ok() == Some(root)
                        })
                    && !result.contains(&target)
                {
                    result.push(target);
                }
            }
            return Some(result);
        }
        node.default_accessibility_references(name).map(|targets| {
            targets
                .iter()
                .copied()
                .filter(|target| self.accessibility_reference_in_scope(source, *target, true))
                .collect()
        })
    }
}
impl DomRuntime {
    pub fn set_accessibility_references(
        &mut self,
        node: NodeId,
        name: &str,
        internals: bool,
        targets: Option<Vec<NodeId>>,
    ) -> Result<(), DomError> {
        self.arena.get(node)?;
        if let Some(targets) = &targets {
            for target in targets {
                if !matches!(self.arena.get(*target)?.kind, NodeKind::Element { .. }) {
                    return Err(DomError::NotAnElement(*target));
                }
            }
        }
        let protected = if self.is_gc_root(node) {
            targets.clone().unwrap_or_default()
        } else {
            Vec::new()
        };
        for (index, target) in protected.iter().enumerate() {
            if let Err(error) = self.retain_native(*target) {
                // Only the successfully acquired prefix belongs to this call.
                let _ =
                    release_reference_protection(protected[..index].iter().copied(), |target| {
                        self.release_native(target)
                    });
                return Err(error);
            }
        }
        let result = (|| -> Result<(), DomError> {
            if !internals {
                if targets.is_some() {
                    self.set_attribute(node, name, "")?;
                } else {
                    self.remove_attribute(node, name)?;
                }
            }
            let record = self.arena.get_mut(node)?;
            record.accessibility_reference_revision =
                record.accessibility_reference_revision.wrapping_add(1);
            let map = if internals {
                &mut record.default_accessibility_references
            } else {
                &mut record.explicit_accessibility_references
            };
            let previous = if let Some(targets) = targets {
                self.internal_targets.extend(targets.iter().copied());
                map.insert(name.into(), targets)
            } else {
                map.remove(name)
            };
            self.mutations.push(Mutation::FormState { node });
            let mut result = Ok(());
            for target in previous.into_iter().flatten() {
                if self.is_gc_root(node) && target != node && self.arena.get(target).is_ok() {
                    let cleanup = self.reclaim_if_unrooted(target);
                    if result.is_ok() {
                        result = cleanup;
                    }
                }
            }
            result
        })();
        let cleanup = release_reference_protection(protected, |target| self.release_native(target));
        result.and(cleanup)
    }
    pub(super) fn clear_explicit_accessibility_reference(
        &mut self,
        node: NodeId,
        name: &str,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        let previous = record.explicit_accessibility_references.remove(name);
        if previous.is_some() {
            record.accessibility_reference_revision =
                record.accessibility_reference_revision.wrapping_add(1);
            self.mutations.push(Mutation::FormState { node });
        }
        for target in previous.into_iter().flatten() {
            if self.is_gc_root(node) && target != node && self.arena.get(target).is_ok() {
                self.reclaim_if_unrooted(target)?;
            }
        }
        Ok(())
    }
}

// Each acquired root is consumed exactly once. A release error may occur
// after its decrement committed, so it must never replay that decrement or
// prevent cleanup of independent roots in the suffix.
fn release_reference_protection(
    targets: impl IntoIterator<Item = NodeId>,
    mut release: impl FnMut(NodeId) -> Result<(), DomError>,
) -> Result<(), DomError> {
    let mut result = Ok(());
    for target in targets {
        let cleanup = release(target);
        if result.is_ok() {
            result = cleanup;
        }
    }
    result
}

#[cfg(test)]
mod reference_tests {
    use super::*;
    #[test]
    fn failed_reference_retain_unwinds_only_successful_prefix() {
        let mut dom = DomRuntime::new();
        let source = dom.create_element("x-source");
        let first = dom.create_element("span");
        let second = dom.create_element("span");
        for node in [source, first, second] {
            dom.retain_native(node).unwrap();
        }
        dom.roots.get_mut(&second).unwrap().native = u32::MAX;
        let revision = dom
            .arena
            .get(source)
            .unwrap()
            .accessibility_reference_revision();
        assert!(matches!(
            dom.set_accessibility_references(
                source,
                "aria-controls",
                true,
                Some(vec![first, second])
            ),
            Err(DomError::MigrationCapacity)
        ));
        assert_eq!(dom.roots.get(&first).unwrap().native, 1);
        assert_eq!(dom.roots.get(&second).unwrap().native, u32::MAX);
        assert_eq!(
            dom.arena
                .get(source)
                .unwrap()
                .accessibility_reference_revision(),
            revision
        );
        assert!(
            dom.arena
                .get(source)
                .unwrap()
                .default_accessibility_references("aria-controls")
                .is_none()
        );
    }
    #[test]
    fn postcommit_release_error_consumes_once_and_cleans_independent_suffix() {
        let mut dom = DomRuntime::new();
        let first = dom.create_element("span");
        let second = dom.create_element("span");
        for node in [first, second] {
            dom.retain_native(node).unwrap();
            dom.retain_native(node).unwrap();
        }
        let mut visited = Vec::new();
        let result = release_reference_protection([first, second], |node| {
            visited.push(node);
            let mut committed = false;
            dom.release_native_with_commit(node, || committed = true)?;
            assert!(committed);
            if node == first {
                Err(DomError::MigrationCapacity)
            } else {
                Ok(())
            }
        });
        assert!(matches!(result, Err(DomError::MigrationCapacity)));
        assert_eq!(visited, vec![first, second]);
        for node in [first, second] {
            assert_eq!(dom.roots.get(&node).unwrap().native, 1);
        }
    }
    #[test]
    fn native_relations_retain_targets_without_permanent_gc_roots() {
        let mut dom = DomRuntime::new();
        let source = dom.create_element("x-source");
        let target = dom.create_element("span");
        dom.retain_native(source).unwrap();
        dom.set_accessibility_references(source, "aria-labelledby", true, Some(vec![target]))
            .unwrap();
        dom.retain_native(target).unwrap();
        dom.release_native(target).unwrap();
        assert!(dom.arena().get(target).is_ok());
        dom.set_accessibility_references(target, "aria-controls", true, Some(vec![source]))
            .unwrap();
        dom.release_native(source).unwrap();
        assert!(dom.arena().get(source).is_err());
        assert!(dom.arena().get(target).is_err());
    }
    #[test]
    fn repeated_explicit_assignment_preserves_unwrapped_native_target() {
        let mut dom = DomRuntime::new();
        let source = dom.create_element("x-source");
        let target = dom.create_element("span");
        dom.retain_native(source).unwrap();
        dom.set_accessibility_references(source, "aria-labelledby", false, Some(vec![target]))
            .unwrap();
        dom.set_accessibility_references(source, "aria-labelledby", false, Some(vec![target]))
            .unwrap();
        assert!(dom.arena().get(target).is_ok());
        dom.set_attribute(source, "aria-labelledby", "missing")
            .unwrap();
        assert!(dom.arena().get(target).is_err());
        dom.release_native(source).unwrap();
    }
}

impl DomArena {
    /// Create an isolated semantic projection. No live node, attribute, text,
    /// mutation journal or author-visible clone receives this layout metadata.
    pub fn with_accessibility_generated_content(
        &self,
        content: &BTreeMap<NodeId, (String, String)>,
    ) -> Self {
        let mut snapshot = self.clone();
        snapshot.accessibility_generated_content = content
            .iter()
            .filter(|(id, _)| self.get(**id).is_ok())
            .map(|(id, value)| (*id, value.clone()))
            .collect();
        snapshot
    }
    pub fn accessibility_generated_content(&self, node: NodeId) -> Option<(&str, &str)> {
        self.accessibility_generated_content
            .get(&node)
            .map(|(before, after)| (before.as_str(), after.as_str()))
    }
    /// Native layout projection only: (display:none, computed hidden visibility).
    pub fn set_accessibility_computed_visibility(
        &mut self,
        values: &BTreeMap<NodeId, (bool, bool)>,
    ) {
        self.accessibility_computed_visibility = values
            .iter()
            .filter(|(id, _)| self.get(**id).is_ok())
            .map(|(id, value)| (*id, *value))
            .collect();
    }
    pub fn set_accessibility_computed_text_style(
        &mut self,
        values: &BTreeMap<NodeId, (String, String)>,
    ) {
        self.accessibility_computed_text_style = values
            .iter()
            .filter(|(id, _)| self.get(**id).is_ok())
            .map(|(id, value)| (*id, value.clone()))
            .collect();
    }
    pub fn accessibility_computed_text_style(&self, node: NodeId) -> Option<(&str, &str)> {
        self.accessibility_computed_text_style
            .get(&node)
            .map(|(display, transform)| (display.as_str(), transform.as_str()))
    }
    pub fn accessibility_computed_visibility(&self, node: NodeId) -> Option<(bool, bool)> {
        self.accessibility_computed_visibility.get(&node).copied()
    }
}
