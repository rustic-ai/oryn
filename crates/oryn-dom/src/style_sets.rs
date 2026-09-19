//! Native stylesheet set enablement shared by CSSOM, parser and layout.
use super::*;
impl DomArena {
    fn sheet_document_tree(&self, node: NodeId) -> Result<Option<NodeId>, DomError> {
        let mut next = Some(node);
        while let Some(node) = next {
            let record = self.get(node)?;
            match record.kind {
                NodeKind::Document => return Ok(Some(node)),
                NodeKind::ShadowRoot { .. } => return Ok(None),
                _ => {}
            }
            next = record.parent;
        }
        Ok(None)
    }
    pub fn initialize_preferred_style_set(&mut self, node: NodeId) -> Result<(), DomError> {
        let Some(document) = self.sheet_document_tree(node)? else {
            return Ok(());
        };
        if self
            .get(document)?
            .document_metadata
            .as_ref()
            .is_some_and(|meta| meta.preferred_style_set.is_some())
        {
            return Ok(());
        }
        let preferred = self
            .descendants_inclusive(node)?
            .into_iter()
            .find_map(|id| {
                if !self.is_css_stylesheet_owner(id).ok()? || self.stylesheet_alternate(id).ok()? {
                    return None;
                }
                self.stylesheet_title(id).ok().flatten()
            });
        if let Some(title) = preferred {
            self.get_mut(document)?
                .document_metadata
                .get_or_insert_with(|| Box::new(DocumentMetadata::default()))
                .preferred_style_set = Some(title);
        }
        Ok(())
    }
    pub fn stylesheet_disabled(&self, id: NodeId) -> Result<bool, DomError> {
        let node = self.get(id)?;
        if !node.stylesheet_parser_ready || !node.stylesheet_policy_allowed {
            return Ok(true);
        }
        if let Some(disabled) = node.stylesheet_disabled {
            return Ok(disabled);
        }
        if node.attributes.contains_null("disabled") {
            return Ok(true);
        }
        let alternate = self.stylesheet_alternate(id)?;
        let Some(document) = self.sheet_document_tree(id)? else {
            return Ok(false);
        };
        let Some(title) = self.stylesheet_title(id)? else {
            return Ok(alternate);
        };
        let selected = self
            .get(document)?
            .document_metadata
            .as_ref()
            .and_then(|meta| {
                meta.last_style_set
                    .as_ref()
                    .or(meta.preferred_style_set.as_ref())
            });
        Ok(selected != Some(&title))
    }
    pub fn stylesheet_set_names(&self, document: NodeId) -> Result<Vec<String>, DomError> {
        let mut names = Vec::new();
        for id in self.descendants_inclusive(document)? {
            if self.is_css_stylesheet_owner(id)?
                && let Some(name) = self.stylesheet_title(id)?
                && !names.contains(&name)
            {
                names.push(name)
            }
        }
        Ok(names)
    }
    pub fn selected_stylesheet_set(&self, document: NodeId) -> Result<Option<String>, DomError> {
        let mut selected = None;
        for id in self.descendants_inclusive(document)? {
            if self.is_css_stylesheet_owner(id)?
                && !self.stylesheet_disabled(id)?
                && let Some(title) = self.stylesheet_title(id)?
            {
                if selected.as_ref().is_some_and(|value| value != &title) {
                    return Ok(None);
                }
                selected = Some(title);
            }
        }
        Ok(Some(selected.unwrap_or_default()))
    }
}
impl DomRuntime {
    /// The setter updates last-set state; enableStyleSheetsForSet deliberately does not.
    pub fn select_stylesheet_set(
        &mut self,
        document: NodeId,
        name: &str,
        update_last: bool,
    ) -> Result<(), DomError> {
        self.arena.initialize_preferred_style_set(document)?;
        if update_last {
            self.arena
                .get_mut(document)?
                .document_metadata
                .get_or_insert_with(|| Box::new(DocumentMetadata::default()))
                .last_style_set = Some(name.into());
        }
        for id in self.arena.descendants_inclusive(document)? {
            if self.arena.is_css_stylesheet_owner(id)? {
                let title = self.arena.stylesheet_title(id)?;
                if let Some(title) = title {
                    self.set_stylesheet_disabled(id, title != name)?;
                }
            }
        }
        Ok(())
    }
}
