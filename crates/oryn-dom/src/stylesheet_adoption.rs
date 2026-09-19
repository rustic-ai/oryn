use crate::{DomArena, DomError, NodeId};
impl DomArena {
    /// Invalidate response authority while preserving the owner and CSSOM wrapper identity.
    pub fn reset_adopted_stylesheet(&mut self, node: NodeId) -> Result<(), DomError> {
        let record = self.get_mut(node)?;
        record.stylesheet_override = None;
        record.stylesheet_base_url = None;
        record.stylesheet_policy_allowed = false;
        record.stylesheet_parser_ready = true;
        self.bump_stylesheet_revision(node);
        Ok(())
    }
}

impl crate::DomRuntime {
    pub fn notify_stylesheet_resource_changed(&mut self, node: NodeId) -> Result<(), DomError> {
        self.arena().get(node)?;
        self.record_mutation(crate::Mutation::StyleSheets { node });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn adoption_invalidates_response_and_revision_without_resetting_author_controls() {
        let mut dom = crate::DomRuntime::new();
        let sheet = dom.create_element("link");
        dom.set_stylesheet_text(sheet, "body{color:red}".into())
            .unwrap();
        dom.set_stylesheet_disabled(sheet, true).unwrap();
        dom.set_stylesheet_media(sheet, "print".into()).unwrap();
        dom.arena_mut().get_mut(sheet).unwrap().stylesheet_base_url =
            Some("https://source.test/old.css".into());
        let revision = dom.arena().stylesheet_revision(sheet).unwrap();
        dom.arena_mut().reset_adopted_stylesheet(sheet).unwrap();
        let record = dom.arena().get(sheet).unwrap();
        assert!(record.stylesheet_override.is_none() && record.stylesheet_base_url.is_none());
        assert!(!record.stylesheet_policy_allowed && record.stylesheet_parser_ready);
        assert_eq!(record.stylesheet_disabled, Some(true));
        assert_eq!(record.stylesheet_media.as_deref(), Some("print"));
        assert_ne!(dom.arena().stylesheet_revision(sheet).unwrap(), revision);
    }
}
