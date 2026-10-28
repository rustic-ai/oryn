//! HTML rendered-text fragment construction shared by innerText and outerText.
use super::{DomError, DomRuntime, NodeId, dom_string};

impl DomRuntime {
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
