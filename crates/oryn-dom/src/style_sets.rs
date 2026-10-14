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
        let alternate = self.stylesheet_alternate(id)?;
        let title = self.stylesheet_title(id)?;
        self.stylesheet_disabled_with_metadata(id, title.as_deref(), alternate)
    }
    /// Same effective enablement algorithm with already parsed metadata.
    pub fn stylesheet_disabled_with_metadata(
        &self,
        id: NodeId,
        title: Option<&str>,
        alternate: bool,
    ) -> Result<bool, DomError> {
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
        let Some(document) = self.sheet_document_tree(id)? else {
            return Ok(false);
        };
        let Some(title) = title else {
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
        Ok(selected.map(String::as_str) != Some(title))
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

/// Borrowed prospective stylesheet state. Copy the referenced strings only
/// after dropping the DOM borrow for allocation admission, then revalidate.
pub struct StylesheetOwnerStateInput<'a> {
    pub parser_ready: bool,
    pub policy_allowed: bool,
    pub disabled_override: Option<bool>,
    pub disabled_attribute: bool,
    pub media_override: Option<&'a str>,
    pub html_media: &'a str,
    pub html_title: Option<&'a str>,
    pub html_alternate: bool,
    pub selected_set: Option<&'a str>,
    pub xml_pi: Option<&'a str>,
    pub in_document_tree: bool,
}
impl DomArena {
    pub fn stylesheet_owner_state_input(
        &self,
        id: NodeId,
    ) -> Result<StylesheetOwnerStateInput<'_>, DomError> {
        let node = self.get(id)?;
        let document = self.sheet_document_tree(id)?;
        let selected_set = document
            .and_then(|document| self.get(document).ok())
            .and_then(|document| document.document_metadata.as_ref())
            .and_then(|meta| {
                meta.last_style_set
                    .as_deref()
                    .or(meta.preferred_style_set.as_deref())
            });
        let xml_pi = match &node.kind {
            NodeKind::ProcessingInstruction { target, data } if target == "xml-stylesheet" => {
                let valid = if let Some(parent) = node.parent {
                    let parent = self.get(parent)?;
                    if !matches!(parent.kind, NodeKind::Document) {
                        false
                    } else {
                        let mut valid = true;
                        for child in &parent.children {
                            if *child == id {
                                break;
                            }
                            if matches!(self.get(*child)?.kind, NodeKind::Element { .. }) {
                                valid = false;
                                break;
                            }
                        }
                        valid
                    }
                } else {
                    false
                };
                valid.then_some(data.as_str())
            }
            _ => None,
        };
        Ok(StylesheetOwnerStateInput {
            parser_ready: node.stylesheet_parser_ready,
            policy_allowed: node.stylesheet_policy_allowed,
            disabled_override: node.stylesheet_disabled,
            disabled_attribute: node.attributes.contains_null("disabled"),
            media_override: node.stylesheet_media.as_deref(),
            html_media: node.attributes.get_null("media").map_or("", String::as_str),
            html_title: node
                .attributes
                .get_null("title")
                .filter(|title| !title.is_empty())
                .map(String::as_str),
            html_alternate: node.attributes.get_null("rel").is_some_and(|rel| {
                rel.split_ascii_whitespace()
                    .any(|value| value.eq_ignore_ascii_case("alternate"))
            }),
            selected_set,
            xml_pi,
            in_document_tree: document.is_some(),
        })
    }
}
