//! HTML rendered-text fragment construction shared by innerText and outerText.
use super::{DomError, DomRuntime, NodeId, dom_string};

impl DomRuntime {
    /// Merge exactly one following Text node, retaining the first node's
    /// identity and applying normalize's live-range transfer rules. This is
    /// deliberately not parent.normalize(): outerText must leave farther
    /// adjacent text nodes separate.
    pub fn merge_next_text_node(&mut self, node: NodeId) -> Result<bool, DomError> {
        let record = self.arena().get(node)?;
        let super::NodeKind::Text { data } = &record.kind else {
            return Ok(false);
        };
        let length = super::utf16_len(data);
        let Some(parent) = record.parent else { return Ok(false) };
        let index = self.child_index(parent, node)? as usize;
        let Some(next) = self.arena().get(parent)?.children.get(index + 1).copied() else {
            return Ok(false);
        };
        let super::NodeKind::Text { data } = &self.arena().get(next)?.kind else {
            return Ok(false);
        };
        let data = data.clone();
        self.replace_character_data(node, length, 0, data)?;
        for sibling in self.normalize_text_boundaries_limited(node, length, 1)? {
            self.remove(sibling)?;
        }
        Ok(true)
    }
    /// Build the HTML text fragment without interpreting text as markup.
    /// CRLF is one break; bare CR and LF each produce an HTML br element.
    /// outerText needs an empty text node for an empty replacement, whereas
    /// innerText replaces children with an empty fragment in that case.
    pub fn create_rendered_text_fragment(
        &mut self,
        value: &str,
        retain_empty_text: bool,
    ) -> Result<NodeId, DomError> {
        let units = dom_string::units(value);
        let fragment = self.create_fragment();
        let mut start = 0;
        let mut cursor = 0;
        while cursor < units.len() {
            if matches!(units[cursor], 10 | 13) {
                if start < cursor {
                    let text = self.create_text(dom_string::from_units(&units[start..cursor]));
                    self.append_child(fragment, text)?;
                }
                let br = self.create_element("br");
                self.append_child(fragment, br)?;
                if units[cursor] == 13 && units.get(cursor + 1) == Some(&10) {
                    cursor += 1;
                }
                cursor += 1;
                start = cursor;
            } else {
                cursor += 1;
            }
        }
        if start < units.len() || (units.is_empty() && retain_empty_text) {
            let text = self.create_text(dom_string::from_units(&units[start..]));
            self.append_child(fragment, text)?;
        }
        Ok(fragment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeKind;

    #[test]
    fn adjacent_text_merge_does_not_normalize_the_remaining_run() {
        let mut dom = crate::html::parse("<body>").into_runtime().unwrap();
        let parent = dom.create_element("div");
        let first = dom.create_text("a");
        let second = dom.create_text(dom_string::from_units(&[0xd800]));
        let third = dom.create_text("c");
        for node in [first, second, third] { dom.append_child(parent, node).unwrap(); }
        let range = dom.create_live_range(
            crate::BoundaryPoint { container: second, offset: 1 },
            crate::BoundaryPoint { container: third, offset: 1 },
        ).unwrap();
        assert!(dom.merge_next_text_node(first).unwrap());
        assert_eq!(dom.arena().get(parent).unwrap().children, [first, third]);
        match &dom.arena().get(first).unwrap().kind {
            NodeKind::Text { data } => assert_eq!(dom_string::units(data), [97, 0xd800]),
            _ => panic!("merged node lost text identity"),
        }
        assert!(!dom.merge_next_text_node(third).unwrap());
        assert!(!dom.merge_next_text_node(parent).unwrap());
        let range = dom.live_range(range).unwrap();
        assert_eq!(range.start, crate::BoundaryPoint { container: first, offset: 2 });
        assert_eq!(range.end, crate::BoundaryPoint { container: third, offset: 1 });
    }

    #[test]
    fn rendered_text_fragment_preserves_utf16_and_normalizes_only_line_breaks() {
        let mut dom = crate::html::parse("<body>").into_runtime().unwrap();
        let value = dom_string::from_units(&[0xd800, 0, 9, 13, 10, 10, 13, 0xdc00]);
        let fragment = dom.create_rendered_text_fragment(&value, true).unwrap();
        let children = dom.arena().get(fragment).unwrap().children.clone();
        assert_eq!(children.len(), 5);
        for child in &children[1..4] {
            assert!(matches!(&dom.arena().get(*child).unwrap().kind,
                NodeKind::Element { name } if name.local.as_ref() == "br" && name.ns.as_ref() == "http://www.w3.org/1999/xhtml"));
        }
        match &dom.arena().get(children[0]).unwrap().kind {
            NodeKind::Text { data } => assert_eq!(dom_string::units(data), [0xd800, 0, 9]),
            _ => panic!("missing initial text"),
        }
        match &dom.arena().get(children[4]).unwrap().kind {
            NodeKind::Text { data } => assert_eq!(dom_string::units(data), [0xdc00]),
            _ => panic!("missing final text"),
        }
        let empty = dom.create_rendered_text_fragment("", false).unwrap();
        assert!(dom.arena().get(empty).unwrap().children.is_empty());
        let empty = dom.create_rendered_text_fragment("", true).unwrap();
        assert_eq!(dom.arena().get(empty).unwrap().children.len(), 1);
        let breaks = dom.create_rendered_text_fragment("\n\r\n\r", true).unwrap();
        assert_eq!(dom.arena().get(breaks).unwrap().children.len(), 3);
    }
}
