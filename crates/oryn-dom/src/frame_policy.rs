//! Native iframe container declaration, including the legacy boolean alias.
use super::{DomError, DomRuntime, NodeId};
impl DomRuntime {
    pub fn frame_permissions_policy_allow(&self, node: NodeId) -> Result<String, DomError> {
        let allow = self
            .attribute_in_null_namespace(node, "allow")?
            .unwrap_or("");
        if !self.is_element(node, "http://www.w3.org/1999/xhtml", "iframe")
            || self
                .attribute_in_null_namespace(node, "allowfullscreen")?
                .is_none()
            || allow
                .split(';')
                .any(|directive| directive.split_ascii_whitespace().next() == Some("fullscreen"))
        {
            return Ok(allow.to_owned());
        }
        Ok(if allow.is_empty() {
            "fullscreen *".into()
        } else {
            format!("{allow}; fullscreen *")
        })
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn explicit_fullscreen_declaration_overrides_legacy_attribute() {
        let dom=crate::html::parse("<iframe id=a allowfullscreen></iframe><iframe id=b allowfullscreen allow=\"fullscreen 'none'; camera *\"></iframe>").into_runtime().unwrap();
        assert_eq!(
            dom.frame_permissions_policy_allow(dom.find_by_id("a").unwrap())
                .unwrap(),
            "fullscreen *"
        );
        assert_eq!(
            dom.frame_permissions_policy_allow(dom.find_by_id("b").unwrap())
                .unwrap(),
            "fullscreen 'none'; camera *"
        );
    }
}
