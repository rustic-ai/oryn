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
    pub fn accessibility_reference_in_scope(
        &self,
        source: NodeId,
        target: NodeId,
        internals: bool,
    ) -> bool {
        self.relation_in_scope_with_checkpoint(source, target, internals, &mut || Ok(()))
            .unwrap_or(false)
    }
    pub fn accessibility_relation(&self, source: NodeId, name: &str) -> Option<Vec<NodeId>> {
        self.accessibility_relation_with_checkpoint(source, name, &mut || Ok(()))
            .ok()
            .flatten()
    }
    fn relation_tree_root_with_checkpoint(
        &self,
        mut node: NodeId,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Option<NodeId>, DomError> {
        let mut seen = BTreeSet::new();
        loop {
            checkpoint()?;
            if !seen.insert(node) {
                return Err(DomError::Cycle);
            }
            let Ok(record) = self.get(node) else {
                return Ok(None);
            };
            if matches!(
                record.kind,
                NodeKind::ShadowRoot { .. } | NodeKind::Document
            ) {
                return Ok(Some(node));
            }
            match record.parent {
                Some(parent) => node = parent,
                None => return Ok(Some(node)),
            }
        }
    }

    fn relation_in_scope_with_checkpoint(
        &self,
        source: NodeId,
        target: NodeId,
        internals: bool,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<bool, DomError> {
        let Some(root) = self.relation_tree_root_with_checkpoint(target, checkpoint)? else {
            return Ok(false);
        };
        let mut cursor = Some(source);
        let mut seen = BTreeSet::new();
        while let Some(node) = cursor {
            checkpoint()?;
            if !seen.insert(node) {
                return Err(DomError::Cycle);
            }
            if node == root {
                return Ok(true);
            }
            cursor = self.get(node).ok().and_then(|node| node.parent);
        }
        if internals {
            let mut cursor = Some(target);
            seen.clear();
            while let Some(node) = cursor {
                checkpoint()?;
                if !seen.insert(node) {
                    return Err(DomError::Cycle);
                }
                if node == source {
                    return Ok(true);
                }
                cursor = self.get(node).ok().and_then(|node| node.parent);
            }
        }
        Ok(false)
    }

    pub fn accessibility_relation_with_allocation<
        A: crate::projection_allocation::ProjectionAllocation,
    >(
        &self,
        source: NodeId,
        name: &str,
        allocation: &mut A,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Option<Vec<NodeId>>, A::Error> {
        checkpoint().map_err(A::Error::from)?;
        let Ok(node) = self.get(source) else {
            return Ok(None);
        };
        if let Some(targets) = node.explicit_accessibility_references(name) {
            let mut result = Vec::new();
            for &target in targets {
                checkpoint().map_err(A::Error::from)?;
                if self.relation_in_scope_with_checkpoint(source, target, false, checkpoint)? {
                    crate::projection_allocation::admitted_push(
                        &mut result,
                        target,
                        allocation,
                        checkpoint,
                    )?;
                }
            }
            checkpoint().map_err(A::Error::from)?;
            return Ok(Some(result));
        }
        if let Some(ids) = node.attributes.get_null_with_checkpoint(name, checkpoint)? {
            let Some(root) = self.relation_tree_root_with_checkpoint(source, checkpoint)? else {
                return Ok(None);
            };
            let mut result = Vec::new();
            let token_start = allocation.scratch_mark();
            let tokens = relation_tokens_with_allocation(ids, allocation, checkpoint)?;
            let token_end = allocation.scratch_mark();
            for token in &tokens {
                let token = *token;
                let scratch_start = allocation.scratch_mark();
                checkpoint().map_err(A::Error::from)?;
                // Preserve the original full descendant validation before ID lookup.
                let mut descendants = Vec::new();
                let mut pending = Vec::new();
                crate::projection_allocation::admitted_push(
                    &mut pending,
                    root,
                    allocation,
                    checkpoint,
                )?;
                let mut seen = BTreeSet::new();
                while let Some(current) = pending.pop() {
                    checkpoint().map_err(A::Error::from)?;
                    if !seen.insert(current) {
                        return Err(A::Error::from(DomError::Cycle));
                    }
                    let Ok(record) = self.get(current) else {
                        return Ok(None);
                    };
                    crate::projection_allocation::admitted_push(
                        &mut descendants,
                        current,
                        allocation,
                        checkpoint,
                    )?;
                    for &child in record.children.iter().rev() {
                        checkpoint().map_err(A::Error::from)?;
                        crate::projection_allocation::admitted_push(
                            &mut pending,
                            child,
                            allocation,
                            checkpoint,
                        )?;
                    }
                }
                let scratch_end = allocation.scratch_mark();
                for &target in &descendants {
                    checkpoint().map_err(A::Error::from)?;
                    if relation_id_matches(self, target, token, checkpoint)?
                        && self.relation_tree_root_with_checkpoint(target, checkpoint)?
                            == Some(root)
                    {
                        let mut duplicate = false;
                        for &prior in &result {
                            checkpoint().map_err(A::Error::from)?;
                            if prior == target {
                                duplicate = true;
                                break;
                            }
                        }
                        if !duplicate {
                            crate::projection_allocation::admitted_push(
                                &mut result,
                                target,
                                allocation,
                                checkpoint,
                            )?;
                        }
                        break;
                    }
                }
                drop(descendants);
                drop(pending);
                allocation.release_scratch_range(scratch_start..scratch_end);
            }
            drop(tokens);
            allocation.release_scratch_range(token_start..token_end);
            checkpoint().map_err(A::Error::from)?;
            return Ok(Some(result));
        }
        let Some(targets) = node.default_accessibility_references(name) else {
            checkpoint().map_err(A::Error::from)?;
            return Ok(None);
        };
        let mut result = Vec::new();
        for &target in targets {
            checkpoint().map_err(A::Error::from)?;
            if self.relation_in_scope_with_checkpoint(source, target, true, checkpoint)? {
                crate::projection_allocation::admitted_push(
                    &mut result,
                    target,
                    allocation,
                    checkpoint,
                )?;
            }
        }
        checkpoint().map_err(A::Error::from)?;
        Ok(Some(result))
    }
    pub fn accessibility_relation_with_checkpoint(
        &self,
        source: NodeId,
        name: &str,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Option<Vec<NodeId>>, DomError> {
        self.accessibility_relation_with_allocation(
            source,
            name,
            &mut crate::projection_allocation::OrdinaryProjectionAllocation,
            checkpoint,
        )
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

impl Node {
    pub fn accessibility_attribute_with_checkpoint<E>(
        &self,
        name: &str,
        check: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<Option<&String>, E> {
        if let Some(value) = self.attributes.get_null_with_checkpoint(name, check)? {
            return Ok(Some(value));
        }
        // Iteration rather than an unchecked tree key comparison also checks
        // arbitrarily long default attribute names.
        for (key, value) in &self.default_accessibility {
            check()?;
            let mut equal = key.len() == name.len();
            if equal {
                for (left, right) in key.bytes().zip(name.bytes()) {
                    check()?;
                    if left != right {
                        equal = false;
                        break;
                    }
                }
            }
            if equal {
                check()?;
                return Ok(Some(value));
            }
        }
        check()?;
        Ok(if name == "role" {
            self.submit_behavior.as_ref().map(|behavior| &behavior.role)
        } else {
            None
        })
    }
}

#[cfg(test)]
mod effective_attribute_checkpoint_tests {
    use super::*;
    #[test]
    fn author_then_default_then_submit_role_precedence() {
        let mut dom = DomRuntime::new();
        let id = dom.create_element("button");
        let node = dom.arena.get_mut(id).unwrap();
        node.submit_behavior = Some(SubmitButtonBehavior::default());
        assert_eq!(
            node.accessibility_attribute_with_checkpoint("role", &mut || Ok::<_, &str>(()))
                .unwrap()
                .map(String::as_str),
            Some("button")
        );
        node.default_accessibility
            .insert("role".into(), "checkbox".into());
        assert_eq!(
            node.accessibility_attribute_with_checkpoint("role", &mut || Ok::<_, &str>(()))
                .unwrap()
                .map(String::as_str),
            Some("checkbox")
        );
        node.attributes.insert(Arc::new(ContentAttribute {
            namespace: None,
            local_name: "role".into(),
            name: "role".into(),
            prefix: None,
            value: "switch".into(),
        }));
        assert_eq!(
            node.accessibility_attribute_with_checkpoint("role", &mut || Ok::<_, &str>(()))
                .unwrap()
                .map(String::as_str),
            Some("switch")
        );
        let effective = node
            .accessibility_attributes_with_checkpoint(&mut || Ok::<_, &str>(()))
            .unwrap();
        assert_eq!(
            effective.get_null("role").map(String::as_str),
            Some("switch")
        );
    }

    #[test]
    fn final_lookup_cancellation_is_not_published() {
        let mut attributes = AttributeList::default();
        attributes.insert(Arc::new(ContentAttribute {
            namespace: None,
            local_name: "".into(),
            name: "".into(),
            prefix: None,
            value: "value".into(),
        }));
        let mut calls = 0;
        assert_eq!(
            attributes.get_null_with_checkpoint("", &mut || {
                calls += 1;
                if calls == 4 { Err("cancel") } else { Ok(()) }
            }),
            Err("cancel")
        );
    }
}

use crate::attributes::{
    ProducerFailure, ordinary_allocation_failure, shared_effective_attributes,
};
impl Node {
    pub fn accessibility_attributes_with_checkpoint<E>(
        &self,
        check: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<std::borrow::Cow<'_, AttributeList>, E> {
        match shared_effective_attributes(
            self,
            &mut |_| Ok::<_, std::collections::TryReserveError>(()),
            &mut |error| error,
            check,
        ) {
            Ok(value) => Ok(value),
            Err(ProducerFailure::Checkpoint(error)) => Err(error),
            Err(ProducerFailure::Allocation(_)) => ordinary_allocation_failure(1, 1),
            Err(ProducerFailure::AllocationLayout { bytes, align, .. }) => {
                ordinary_allocation_failure(bytes, align)
            }
            Err(ProducerFailure::Capacity) => panic!("ordinary accessibility capacity overflow"),
        }
    }
    pub fn accessibility_attributes_with_allocation<
        A: crate::projection_allocation::ProjectionAllocation,
    >(
        &self,
        allocation: &mut A,
        check: &mut impl FnMut() -> Result<(), crate::DomError>,
    ) -> Result<std::borrow::Cow<'_, AttributeList>, A::Error> {
        let allocator = std::cell::RefCell::new(allocation);
        match shared_effective_attributes(
            self,
            &mut |bytes| allocator.borrow_mut().reserve(bytes),
            &mut |error| allocator.borrow().allocation_error(error),
            check,
        ) {
            Ok(value) => Ok(value),
            Err(ProducerFailure::Checkpoint(error)) => Err(A::Error::from(error)),
            Err(ProducerFailure::Allocation(error)) => Err(error),
            Err(ProducerFailure::AllocationLayout { error, .. }) => Err(error),
            Err(ProducerFailure::Capacity) => {
                Err(A::Error::from(crate::DomError::SemanticProjectionCapacity))
            }
        }
    }
}

#[cfg(test)]
mod effective_allocation_controls {
    use super::*;
    #[derive(Debug)]
    struct TestError;
    impl From<crate::DomError> for TestError {
        fn from(_: crate::DomError) -> Self {
            Self
        }
    }
    struct Allocator {
        calls: usize,
        fail: usize,
    }
    impl crate::projection_allocation::ProjectionAllocation for Allocator {
        type Error = TestError;
        fn reserve(&mut self, _: usize) -> Result<(), Self::Error> {
            self.calls += 1;
            if self.calls == self.fail {
                Err(TestError)
            } else {
                Ok(())
            }
        }
        fn allocation_error(&self, _: std::collections::TryReserveError) -> Self::Error {
            TestError
        }
    }
    #[test]
    fn admitted_effective_attributes_preserve_precedence_and_failure_source() {
        let mut dom = DomRuntime::new();
        let id = dom.create_element("button");
        let node = dom.arena.get_mut(id).unwrap();
        node.submit_behavior = Some(SubmitButtonBehavior::default());
        node.default_accessibility
            .insert("aria-label".into(), "value".repeat(1024));
        for (author, default, expected) in [
            (false, false, "button"),
            (false, true, "checkbox"),
            (true, true, "switch"),
        ] {
            if default {
                node.default_accessibility
                    .insert("role".into(), "checkbox".into());
            }
            if author {
                node.attributes.insert(Arc::new(ContentAttribute {
                    namespace: None,
                    local_name: "role".into(),
                    name: "role".into(),
                    prefix: None,
                    value: "switch".into(),
                }));
            }
            let mut allocation = Allocator {
                calls: 0,
                fail: usize::MAX,
            };
            let result = node
                .accessibility_attributes_with_allocation(&mut allocation, &mut || Ok(()))
                .unwrap();
            assert_eq!(result.get_null("role").map(String::as_str), Some(expected));
            let total = allocation.calls;
            let before = format!("{node:?}");
            for boundary in 1..=total {
                let mut allocation = Allocator {
                    calls: 0,
                    fail: boundary,
                };
                let result =
                    node.accessibility_attributes_with_allocation(&mut allocation, &mut || Ok(()));
                assert!(result.is_err());
                assert_eq!(format!("{node:?}"), before);
            }
        }
    }
}

fn relation_tokens_with_allocation<'a, A: crate::projection_allocation::ProjectionAllocation>(
    value: &'a str,
    allocation: &mut A,
    check: &mut impl FnMut() -> Result<(), DomError>,
) -> Result<Vec<&'a str>, A::Error> {
    check().map_err(A::Error::from)?;
    let mut output = Vec::new();
    let mut start = None;
    for (index, byte) in value.bytes().enumerate() {
        check().map_err(A::Error::from)?;
        if byte.is_ascii_whitespace() {
            if let Some(begin) = start.take() {
                crate::projection_allocation::admitted_push(
                    &mut output,
                    &value[begin..index],
                    allocation,
                    check,
                )?;
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(begin) = start {
        crate::projection_allocation::admitted_push(
            &mut output,
            &value[begin..],
            allocation,
            check,
        )?;
    }
    check().map_err(A::Error::from)?;
    Ok(output)
}
fn relation_id_matches(
    dom: &DomArena,
    target: NodeId,
    token: &str,
    check: &mut impl FnMut() -> Result<(), DomError>,
) -> Result<bool, DomError> {
    check()?;
    let Ok(node) = dom.get(target) else {
        return Ok(false);
    };
    let Some(id) = node.attributes.get_null_with_checkpoint("id", check)? else {
        return Ok(false);
    };
    if id.len() != token.len() {
        return Ok(false);
    }
    for (left, right) in id.bytes().zip(token.bytes()) {
        check()?;
        if left != right {
            return Ok(false);
        }
    }
    check()?;
    Ok(true)
}

#[cfg(test)]
mod relation_allocation_controls {
    use super::*;
    struct Allocation {
        calls: usize,
        fail: usize,
    }
    impl crate::projection_allocation::ProjectionAllocation for Allocation {
        type Error = DomError;
        fn reserve(&mut self, _: usize) -> Result<(), DomError> {
            self.calls += 1;
            if self.calls == self.fail {
                Err(DomError::SemanticProjectionCapacity)
            } else {
                Ok(())
            }
        }
        fn allocation_error(&self, _: std::collections::TryReserveError) -> DomError {
            DomError::SemanticProjectionCapacity
        }
    }
    #[test]
    fn explicit_relation_reservation_failure_preserves_dom_and_identity() {
        let mut dom = DomRuntime::new();
        let source = dom.create_element("x-source");
        let target = dom.create_element("span");
        dom.retain_native(source).unwrap();
        dom.set_accessibility_references(source, "aria-labelledby", false, Some(vec![target]))
            .unwrap();
        let expected = dom
            .arena()
            .accessibility_relation(source, "aria-labelledby");
        let mut allocation = Allocation {
            calls: 0,
            fail: usize::MAX,
        };
        assert_eq!(
            dom.arena()
                .accessibility_relation_with_allocation(
                    source,
                    "aria-labelledby",
                    &mut allocation,
                    &mut || Ok(())
                )
                .unwrap(),
            expected
        );
        for fail in 1..=allocation.calls {
            let mut allocation = Allocation { calls: 0, fail };
            assert!(matches!(
                dom.arena().accessibility_relation_with_allocation(
                    source,
                    "aria-labelledby",
                    &mut allocation,
                    &mut || Ok(())
                ),
                Err(DomError::SemanticProjectionCapacity)
            ));
            assert_eq!(
                dom.arena()
                    .accessibility_relation(source, "aria-labelledby"),
                expected
            );
            assert!(dom.arena().get(target).is_ok());
        }
    }
    #[test]
    fn long_relation_token_scan_is_cancelled_inside_scan() {
        let text = "x".repeat(8192);
        let mut allocation = Allocation {
            calls: 0,
            fail: usize::MAX,
        };
        let mut calls = 0;
        let result = relation_tokens_with_allocation(&text, &mut allocation, &mut || {
            calls += 1;
            if calls == 64 {
                Err(DomError::ProjectionCancelled)
            } else {
                Ok(())
            }
        });
        assert!(matches!(result, Err(DomError::ProjectionCancelled)));
        assert_eq!(calls, 64);
        assert_eq!(
            allocation.calls, 0,
            "not a post-scan allocation cancellation"
        );
    }
}
