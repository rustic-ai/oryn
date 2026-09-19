//! Private permission prompt presentation. No authored attribute creates it.
use super::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionPrompt {
    pub id: String,
    pub origin: String,
    pub geolocation: bool,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub protocol_url: Option<String>,
    pub active: u8,
}
impl DomRuntime {
    pub fn open_permission_prompt(
        &mut self,
        node: NodeId,
        prompt: PermissionPrompt,
    ) -> Result<bool, DomError> {
        if !self.is_connected(node)?
            || !(prompt.protocol.is_some()
                && self.is_element(node, "http://www.w3.org/1999/xhtml", "html")
                || self.is_element(node, "http://www.w3.org/1999/xhtml", "geolocation")
                || self.is_element(node, "http://www.w3.org/1999/xhtml", "usermedia")
                || (self.is_element(node, "http://www.w3.org/1999/xhtml", "camera")
                    || self.is_element(node, "http://www.w3.org/1999/xhtml", "microphone")))
        {
            return Ok(false);
        }
        if prompt
            .protocol_url
            .as_ref()
            .is_some_and(|url| url.len() > 65536)
            || prompt
                .protocol
                .as_ref()
                .is_some_and(|scheme| scheme.len() > 64)
            || prompt.id.len() > 128
            || prompt.origin.len() > 4096
            || prompt.active > 2
        {
            return Ok(false);
        }
        let old = self
            .top_layer()
            .filter(|entry| entry.kind == TopLayerKind::PermissionPrompt)
            .map(|entry| entry.node)
            .collect::<Vec<_>>();
        for old in old {
            self.remove_from_top_layer(old);
        }
        self.arena.get_mut(node)?.permission_prompt = Some(prompt);
        self.add_to_top_layer(node, TopLayerKind::PermissionPrompt)
    }
    pub fn permission_prompt_snapshot(
        &self,
        node: NodeId,
    ) -> Result<Option<PermissionPrompt>, DomError> {
        Ok(self.arena.get(node)?.permission_prompt.clone())
    }
    pub fn move_permission_prompt(&mut self, node: NodeId, delta: i32) -> Result<(), DomError> {
        if let Some(prompt) = self.arena.get_mut(node)?.permission_prompt.as_mut() {
            prompt.active = (i32::from(prompt.active) + delta).rem_euclid(3) as u8;
            self.record_mutation(Mutation::FormState { node });
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_prompt_cannot_be_created_by_attributes_and_closes_with_top_layer() {
        let mut dom =
            crate::html::parse("<usermedia id=u data-permission-prompt=allow></usermedia>")
                .into_runtime()
                .unwrap();
        let node = dom.find_by_id("u").unwrap();
        assert!(dom.permission_prompt_snapshot(node).unwrap().is_none());
        assert!(
            dom.open_permission_prompt(
                node,
                PermissionPrompt {
                    id: "parent-owned-id".into(),
                    origin: "https://example.test".into(),
                    geolocation: false,
                    protocol: None,
                    protocol_url: None,
                    active: 1
                }
            )
            .unwrap()
        );
        dom.move_permission_prompt(node, 1).unwrap();
        assert_eq!(
            dom.permission_prompt_snapshot(node)
                .unwrap()
                .unwrap()
                .active,
            2
        );
        dom.remove_from_top_layer(node);
        assert!(dom.permission_prompt_snapshot(node).unwrap().is_none());
    }
    #[test]
    fn protocol_prompt_is_private_and_binds_the_exact_handler_template() {
        let mut dom = crate::html::parse("<html><body>page</body></html>")
            .into_runtime()
            .unwrap();
        let root = dom.document_element().unwrap();
        let prompt = PermissionPrompt {
            id: "native-protocol-id".into(),
            origin: "https://handler.test".into(),
            geolocation: false,
            protocol: Some("web+demo".into()),
            protocol_url: Some("https://handler.test/?%s".into()),
            active: 2,
        };
        assert!(dom.open_permission_prompt(root, prompt.clone()).unwrap());
        assert_eq!(dom.permission_prompt_snapshot(root).unwrap(), Some(prompt));
        dom.remove_from_top_layer(root);
        assert!(dom.permission_prompt_snapshot(root).unwrap().is_none());
    }
}
