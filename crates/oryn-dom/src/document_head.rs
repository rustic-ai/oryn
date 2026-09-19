//! Native document-head membership for processing metadata directives.
use super::*;
impl DomRuntime {
    pub fn is_in_document_head(&self, node: NodeId) -> Result<bool, DomError> {
        let Some(document) = self.node_document(node)? else {
            return Ok(false);
        };
        let Some(html) = self
            .arena
            .get(document)?
            .children
            .iter()
            .copied()
            .find(|child| self.is_element(*child, "http://www.w3.org/1999/xhtml", "html"))
        else {
            return Ok(false);
        };
        let Some(head) = self
            .arena
            .get(html)?
            .children
            .iter()
            .copied()
            .find(|child| self.is_element(*child, "http://www.w3.org/1999/xhtml", "head"))
        else {
            return Ok(false);
        };
        let mut cursor = Some(node);
        while let Some(node) = cursor {
            if node == head {
                return Ok(true);
            }
            cursor = self.arena.get(node)?.parent;
        }
        Ok(false)
    }
}
