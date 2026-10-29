//! Allocation-free projection of the canonical mutation journal.
use crate::{DomRuntime, Mutation, MutationCursor, NodeId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttributePresenceChange {
    pub node: NodeId,
    pub was_present: bool,
    pub is_present: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttributePresenceRead {
    /// Exact matched sequence, or the current journal end if no change matched.
    pub next: MutationCursor,
    pub truncated: bool,
    pub change: Option<AttributePresenceChange>,
}

impl DomRuntime {
    /// Independent consumers acknowledge `next` only after accepting `change`.
    /// No strings, removal leases or unrelated mutations are cloned. Null-
    /// namespace presence changes are distinct from attribute value changes.
    pub fn next_attribute_presence_change(
        &self, cursor: MutationCursor, local_name: &str,
    ) -> AttributePresenceRead {
        let journal = &self.mutations;
        let oldest = journal.entries.front().map_or(journal.next, |(sequence, _)| *sequence);
        if cursor.0.saturating_add(1) < oldest {
            return AttributePresenceRead { next: cursor, truncated: true, change: None };
        }
        let start = journal.entries.partition_point(|(sequence, _)| *sequence <= cursor.0);
        for (sequence, mutation) in journal.entries.range(start..) {
            if let Mutation::Attribute { node, name, namespace, old_value, new_value } = mutation {
                if name == local_name && namespace.is_none()
                    && old_value.is_some() != new_value.is_some()
                {
                    return AttributePresenceRead {
                        next: MutationCursor(*sequence), truncated: false,
                        change: Some(AttributePresenceChange {
                            node: *node, was_present: old_value.is_some(), is_present: new_value.is_some(),
                        }),
                    };
                }
            }
        }
        AttributePresenceRead { next: MutationCursor(journal.next.saturating_sub(1)), truncated: false, change: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_presence_projection_preserves_sequence_namespace_and_independent_cursor() {
        let mut dom = crate::html::parse("<body></body>").into_runtime().unwrap();
        let node = dom.create_element("details");
        let cursor = dom.mutation_cursor();
        dom.set_attribute(node, "data-large", "x".repeat(1024 * 1024)).unwrap();
        dom.set_attribute_ns(node, Some("urn:test".into()), "p:open", "").unwrap();
        dom.set_attribute(node, "open", "").unwrap();
        dom.set_attribute(node, "open", "changed-value").unwrap();
        dom.remove_attribute(node, "open").unwrap();
        let first = dom.next_attribute_presence_change(cursor, "open");
        assert_eq!(first.change, Some(AttributePresenceChange { node, was_present: false, is_present: true }));
        assert!(!first.truncated);
        assert_eq!(dom.next_attribute_presence_change(cursor, "open"), first);
        let second = dom.next_attribute_presence_change(first.next, "open");
        assert_eq!(second.change, Some(AttributePresenceChange { node, was_present: true, is_present: false }));
        assert!(second.next.0 > first.next.0);
        let end = dom.next_attribute_presence_change(second.next, "open");
        assert!(end.change.is_none());
        assert_eq!(end.next, dom.mutation_cursor());
        assert!(!dom.mutations_since(cursor).mutations.is_empty());
    }

    #[test]
    fn attribute_presence_projection_does_not_hide_eviction() {
        let mut dom = crate::html::parse("<body></body>").into_runtime().unwrap();
        let node = dom.create_element("details");
        let cursor = dom.mutation_cursor();
        for index in 0..crate::MutationJournal::MAX_ENTRIES + 1 {
            dom.set_attribute(node, "data-count", index.to_string()).unwrap();
        }
        let read = dom.next_attribute_presence_change(cursor, "open");
        assert!(read.truncated);
        assert_eq!(read.next, cursor);
        assert!(read.change.is_none());
    }
}
