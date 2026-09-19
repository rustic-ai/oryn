//! Native selector state shares the live control and document authorities.
use super::*;
pub fn valid_custom_element_name(name: &str) -> bool {
    // The pinned HTML algorithm uses the relaxed element-local-name grammar,
    // then adds lowercase ASCII start, a hyphen, reserved-name and uppercase
    // constraints. With an ASCII-letter start only delimiters are forbidden;
    // the former XML-derived PCENChar whitelist is no longer applicable.
    let units = crate::dom_string::units(name);
    units
        .first()
        .is_some_and(|unit| (b'a' as u16..=b'z' as u16).contains(unit))
        && units.contains(&(b'-' as u16))
        && !units
            .iter()
            .any(|unit| matches!(*unit, 0 | 9 | 10 | 12 | 13 | 32 | 47 | 62 | 65..=90))
        && !matches!(
            name,
            "annotation-xml"
                | "color-profile"
                | "font-face"
                | "font-face-src"
                | "font-face-uri"
                | "font-face-format"
                | "font-face-name"
                | "missing-glyph"
        )
}

impl DomRuntime {
    pub fn element_focus_within(&self, node: NodeId) -> Result<bool, DomError> {
        let document = self.node_document(node)?;
        let mut current = self.focused();
        if current
            .map(|focused| self.node_document(focused))
            .transpose()?
            .flatten()
            != document
        {
            current = document
                .map(|document| self.focused_element_in(document))
                .transpose()?
                .flatten();
        }
        let mut visited = std::collections::BTreeSet::new();
        while let Some(focused) = current.filter(|focused| visited.insert(*focused)) {
            if focused == node {
                return Ok(true);
            }
            current = self.flat_tree_parent(focused)?;
        }
        Ok(false)
    }

    pub fn set_permission_granted(&mut self, node: NodeId, granted: bool) -> Result<(), DomError> {
        let eligible = self.is_element(node, "http://www.w3.org/1999/xhtml", "geolocation")
            || self.is_element(node, "http://www.w3.org/1999/xhtml", "usermedia")
            || (self.is_element(node, "http://www.w3.org/1999/xhtml", "camera")
                || self.is_element(node, "http://www.w3.org/1999/xhtml", "microphone"));
        let record = self.arena.get_mut(node)?;
        let granted = eligible && granted;
        if record.permission_granted != granted {
            record.permission_granted = granted;
            self.mutations.push(Mutation::FormState { node });
        }
        Ok(())
    }
    pub fn element_permission_granted(&self, node: NodeId) -> Result<bool, DomError> {
        let record = self.arena.get(node)?;
        Ok(record.permission_granted
            && matches!(&record.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&matches!(name.local.as_ref(),"geolocation"|"usermedia"|"camera"|"microphone")))
    }
    pub fn set_user_validity(&mut self, node: NodeId, value: bool) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        if record.user_validity != value {
            record.user_validity = value;
            self.mutations.push(Mutation::FormState { node });
        }
        Ok(())
    }
    pub fn element_user_validity(&self, node: NodeId) -> Result<Option<bool>, DomError> {
        let record = self.arena.get(node)?;
        let eligible = matches!(&record.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&matches!(name.local.as_ref(),"input"|"textarea"|"select"));
        if !eligible || !record.user_validity || !self.control_will_validate(node)? {
            return Ok(None);
        }
        Ok(Some(self.control_validity(node)?))
    }
    pub fn mark_form_user_validity(&mut self, form: NodeId) -> Result<(), DomError> {
        for control in self.submittable_form_controls(form)? {
            if self.is_element(control, "http://www.w3.org/1999/xhtml", "input")
                || self.is_element(control, "http://www.w3.org/1999/xhtml", "textarea")
                || self.is_element(control, "http://www.w3.org/1999/xhtml", "select")
            {
                self.set_user_validity(control, true)?;
            }
        }
        Ok(())
    }

    pub fn document_design_mode(&self, document: NodeId) -> Result<bool, DomError> {
        let record = self.arena.get(document)?;
        if !matches!(record.kind, NodeKind::Document) {
            return Err(DomError::NotADocument(document));
        }
        Ok(record
            .document_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.design_mode))
    }
    pub fn set_document_design_mode(
        &mut self,
        document: NodeId,
        enabled: bool,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(document)?;
        if !matches!(record.kind, NodeKind::Document) {
            return Err(DomError::NotADocument(document));
        }
        let metadata = record
            .document_metadata
            .get_or_insert_with(|| Box::new(DocumentMetadata::default()));
        if metadata.design_mode != enabled {
            metadata.design_mode = enabled;
            self.mutations.push(Mutation::FormState { node: document });
        }
        Ok(())
    }
    pub fn set_custom_element_defined(
        &mut self,
        node: NodeId,
        defined: bool,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        if record.custom_element_defined != defined {
            record.custom_element_defined = defined;
            self.mutations.push(Mutation::FormState { node });
        }
        Ok(())
    }
    pub fn element_defined(&self, node: NodeId) -> Result<bool, DomError> {
        let record = self.arena.get(node)?;
        Ok(match &record.kind {
            NodeKind::Element { name } => {
                name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
                    || !valid_custom_element_name(name.local.as_ref())
                    || record.custom_element_defined
            }
            _ => false,
        })
    }
    pub fn element_read_write(&self, node: NodeId) -> Result<bool, DomError> {
        let record = self.arena.get(node)?;
        if let NodeKind::Element { name } = &record.kind {
            if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                && matches!(name.local.as_ref(), "input" | "textarea")
            {
                let kind = record
                    .attributes
                    .get_null("type")
                    .map(|value| value.to_ascii_lowercase())
                    .unwrap_or_else(|| "text".into());
                let eligible = name.local.as_ref() == "textarea"
                    || !matches!(
                        kind.as_str(),
                        "hidden"
                            | "button"
                            | "submit"
                            | "reset"
                            | "image"
                            | "checkbox"
                            | "radio"
                            | "file"
                            | "color"
                            | "range"
                    );
                return Ok(eligible
                    && !self.control_disabled(node)?
                    && !record.attributes.contains_null("readonly"));
            }
        } else {
            return Ok(false);
        }
        let mut current = Some(node);
        let mut visited = BTreeSet::new();
        let mut shadow = false;
        while let Some(id) = current.filter(|id| visited.insert(*id)) {
            let record = self.arena.get(id)?;
            if let Some(value) = record.attributes.get_null("contenteditable") {
                if value.eq_ignore_ascii_case("false") {
                    return Ok(false);
                }
                if value.is_empty()
                    || value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("plaintext-only")
                {
                    return Ok(true);
                }
            }
            if matches!(record.kind, NodeKind::ShadowRoot { .. }) {
                shadow = true;
            }
            if matches!(record.kind, NodeKind::Document) {
                return Ok(!shadow && self.document_design_mode(id)?);
            }
            current = record.parent;
        }
        Ok(false)
    }
    pub fn element_default_state(&self, node: NodeId) -> Result<bool, DomError> {
        let record = self.arena.get(node)?;
        let NodeKind::Element { name } = &record.kind else {
            return Ok(false);
        };
        if name.ns.as_ref() != "http://www.w3.org/1999/xhtml" {
            return Ok(false);
        }
        if name.local.as_ref() == "option" {
            return Ok(record.attributes.contains_null("selected"));
        }
        if name.local.as_ref() == "input"
            && record.attributes.get_null("type").is_some_and(|value| {
                value.eq_ignore_ascii_case("checkbox") || value.eq_ignore_ascii_case("radio")
            })
        {
            return Ok(record.attributes.contains_null("checked"));
        }
        if !self.is_submit_button(node)? {
            return Ok(false);
        }
        let Some(form) = self.form_owner(node)? else {
            return Ok(false);
        };
        for candidate in self.submittable_form_controls(form)? {
            if self.is_submit_button(candidate)? {
                return Ok(candidate == node);
            }
        }
        Ok(false)
    }
    /// None excludes elements for which required has no HTML applicability;
    /// disabled and readonly do not change required/optional selector membership.
    pub fn element_required_state(&self, node: NodeId) -> Result<Option<bool>, DomError> {
        let record = self.arena.get(node)?;
        let NodeKind::Element { name } = &record.kind else {
            return Ok(None);
        };
        if name.ns.as_ref() != "http://www.w3.org/1999/xhtml" {
            return Ok(None);
        }
        let eligible = match name.local.as_ref() {
            "select" | "textarea" => true,
            "input" => {
                let kind = record
                    .attributes
                    .get_null("type")
                    .map(|value| value.to_ascii_lowercase())
                    .unwrap_or_default();
                !matches!(
                    kind.as_str(),
                    "hidden" | "range" | "color" | "submit" | "image" | "reset" | "button"
                )
            }
            _ => false,
        };
        Ok(eligible.then(|| record.attributes.contains_null("required")))
    }
    pub fn element_placeholder_shown(&self, node: NodeId) -> Result<bool, DomError> {
        let record = self.arena.get(node)?;
        let NodeKind::Element { name } = &record.kind else {
            return Ok(false);
        };
        if name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
            || !record.attributes.contains_null("placeholder")
        {
            return Ok(false);
        }
        let kind = record
            .attributes
            .get_null("type")
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_else(|| "text".into());
        let input = name.local.as_ref() == "input"
            && !matches!(
                kind.as_str(),
                "hidden"
                    | "date"
                    | "month"
                    | "week"
                    | "time"
                    | "datetime-local"
                    | "range"
                    | "color"
                    | "checkbox"
                    | "radio"
                    | "file"
                    | "submit"
                    | "image"
                    | "reset"
                    | "button"
            );
        Ok((name.local.as_ref() == "textarea" || input)
            && self.control_edit_value(node)?.is_empty())
    }
    pub fn document_target(&self, document: NodeId) -> Result<Option<NodeId>, DomError> {
        let Some(url) = self
            .arena
            .get(document)?
            .document_metadata
            .as_ref()
            .and_then(|metadata| metadata.url.as_ref())
        else {
            return Ok(None);
        };
        let Ok(url) = url::Url::parse(url) else {
            return Ok(None);
        };
        let Some(fragment) = url.fragment().filter(|fragment| !fragment.is_empty()) else {
            return Ok(None);
        };
        let mut bytes = Vec::with_capacity(fragment.len());
        let input = fragment.as_bytes();
        let mut i = 0;
        while i < input.len() {
            if input[i] == b'%'
                && i + 2 < input.len()
                && let (Some(a), Some(b)) = (
                    (input[i + 1] as char).to_digit(16),
                    (input[i + 2] as char).to_digit(16),
                )
            {
                bytes.push((a * 16 + b) as u8);
                i += 3;
                continue;
            }
            bytes.push(input[i]);
            i += 1;
        }
        let name = String::from_utf8_lossy(&bytes);
        let mut pending = self
            .arena
            .get(document)?
            .children
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        let mut anchors = None;
        while let Some(id) = pending.pop() {
            let record = self.arena.get(id)?;
            if matches!(
                record.kind,
                NodeKind::Document | NodeKind::ShadowRoot { .. }
            ) {
                continue;
            }
            if record
                .attributes
                .get_null("id")
                .is_some_and(|value| value == name.as_ref())
            {
                return Ok(Some(id));
            }
            if anchors.is_none()
                && matches!(&record.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&name.local.as_ref()=="a")
                && record
                    .attributes
                    .get_null("name")
                    .is_some_and(|value| value == name.as_ref())
            {
                anchors = Some(id);
            }
            pending.extend(record.children.iter().rev().copied());
        }
        Ok(anchors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn user_validity_is_committed_native_state_not_dirty_value() {
        let mut dom=html::parse("<form id=f><input id=i required><select id=s required><option value=''>Choose</option></select><input id=d required disabled></form>").into_runtime().unwrap();
        let i = dom.find_by_id("i").unwrap();
        let f = dom.find_by_id("f").unwrap();
        let select = dom.find_by_id("s").unwrap();
        let disabled = dom.find_by_id("d").unwrap();
        assert_eq!(dom.element_user_validity(i).unwrap(), None);
        dom.set_value(i, "script", false).unwrap();
        assert_eq!(dom.element_user_validity(i).unwrap(), None);
        dom.set_value(i, "", false).unwrap();
        dom.edit_control_value(i, 0, 0, "x").unwrap();
        assert_eq!(dom.element_user_validity(i).unwrap(), None);
        assert!(dom.take_control_change(i).unwrap());
        assert_eq!(dom.element_user_validity(i).unwrap(), Some(true));
        dom.set_value(i, "", false).unwrap();
        assert_eq!(dom.element_user_validity(i).unwrap(), Some(false));
        dom.set_attribute(i, "disabled", "").unwrap();
        assert_eq!(dom.element_user_validity(i).unwrap(), None);
        dom.remove_attribute(i, "disabled").unwrap();
        assert_eq!(dom.element_user_validity(i).unwrap(), Some(false));
        dom.reset_control_value(i, "".into()).unwrap();
        assert_eq!(dom.element_user_validity(i).unwrap(), None);
        dom.mark_form_user_validity(f).unwrap();
        assert_eq!(dom.element_user_validity(i).unwrap(), Some(false));
        assert_eq!(dom.element_user_validity(select).unwrap(), Some(false));
        assert_eq!(dom.element_user_validity(disabled).unwrap(), None);
        dom.reset_select(select).unwrap();
        assert_eq!(dom.element_user_validity(select).unwrap(), None);
        assert_eq!(dom.element_user_validity(f).unwrap(), None);
    }
    #[test]
    fn user_validity_change_records_invalidation_even_for_same_value() {
        let mut dom = html::parse("<input id=i required>").into_runtime().unwrap();
        let i = dom.find_by_id("i").unwrap();
        let cursor = dom.mutation_cursor();
        dom.set_user_validity(i, true).unwrap();
        assert!(
            dom.mutations_since(cursor)
                .mutations
                .iter()
                .any(|change| matches!(change,Mutation::FormState{node} if *node==i))
        );
        let cursor = dom.mutation_cursor();
        dom.set_user_validity(i, true).unwrap();
        assert!(dom.mutations_since(cursor).mutations.is_empty());
    }
    #[test]
    fn native_control_state_tracks_value_readonly_disabled_and_designmode() {
        let mut dom =
            html::parse("<input id=i placeholder=x><div id=e><span id=child></span></div>")
                .into_runtime()
                .unwrap();
        let input = dom.find_by_id("i").unwrap();
        let child = dom.find_by_id("child").unwrap();
        assert!(dom.element_placeholder_shown(input).unwrap());
        assert!(dom.element_read_write(input).unwrap());
        dom.set_attribute(input, "type", "unknown-type").unwrap();
        assert!(dom.element_placeholder_shown(input).unwrap());
        assert_eq!(
            dom.set_document_design_mode(input, true),
            Err(DomError::NotADocument(input))
        );
        dom.set_value(input, "value", false).unwrap();
        assert!(!dom.element_placeholder_shown(input).unwrap());
        dom.set_attribute(input, "readonly", "").unwrap();
        assert!(!dom.element_read_write(input).unwrap());
        dom.set_document_design_mode(dom.document(), true).unwrap();
        assert!(dom.element_read_write(child).unwrap());
        dom.set_attribute(child, "contenteditable", "false")
            .unwrap();
        assert!(!dom.element_read_write(child).unwrap());
    }
    #[test]
    fn default_selector_preserves_default_checked_and_form_tree_order() {
        let mut dom=html::parse("<input id=outer type=image form=f><form id=f><button id=b>submit</button><input id=c type=checkbox checked><option id=o selected>x</option></form>").into_runtime().unwrap();
        let c = dom.find_by_id("c").unwrap();
        assert!(
            dom.element_default_state(dom.find_by_id("outer").unwrap())
                .unwrap()
        );
        assert!(
            !dom.element_default_state(dom.find_by_id("b").unwrap())
                .unwrap()
        );
        assert!(dom.element_default_state(c).unwrap());
        assert!(
            dom.element_default_state(dom.find_by_id("o").unwrap())
                .unwrap()
        );
        dom.remove_attribute(c, "checked").unwrap();
        assert!(!dom.element_default_state(c).unwrap());
    }
    #[test]
    fn range_selectors_share_native_numeric_limits_and_periodic_time() {
        let mut dom=html::parse("<input id=n type=number min=2 max=8 step=2 value=3 disabled><input id=u type=number><input id=t type=time min=23:00 max=01:00 value=12:00><input id=r type=range>").into_runtime().unwrap();
        let n = dom.find_by_id("n").unwrap();
        assert_eq!(dom.element_range_state(n).unwrap(), Some(true));
        assert_eq!(
            dom.element_range_state(dom.find_by_id("u").unwrap())
                .unwrap(),
            None
        );
        assert_eq!(
            dom.element_range_state(dom.find_by_id("t").unwrap())
                .unwrap(),
            Some(false)
        );
        assert_eq!(
            dom.element_range_state(dom.find_by_id("r").unwrap())
                .unwrap(),
            Some(true)
        );
        dom.set_value(n, "9", false).unwrap();
        assert_eq!(dom.element_range_state(n).unwrap(), Some(false));
        dom.set_value(n, "", false).unwrap();
        assert_eq!(dom.element_range_state(n).unwrap(), Some(true));
    }
    #[test]
    fn required_selector_uses_type_applicability_not_validation_candidacy() {
        let mut dom=html::parse("<input id=x required disabled><input id=r type=range required><div id=d required></div>").into_runtime().unwrap();
        let x = dom.find_by_id("x").unwrap();
        assert_eq!(dom.element_required_state(x).unwrap(), Some(true));
        assert_eq!(
            dom.element_required_state(dom.find_by_id("r").unwrap())
                .unwrap(),
            None
        );
        assert_eq!(
            dom.element_required_state(dom.find_by_id("d").unwrap())
                .unwrap(),
            None
        );
        dom.remove_attribute(x, "required").unwrap();
        assert_eq!(dom.element_required_state(x).unwrap(), Some(false));
    }
    #[test]
    fn defined_comes_from_upgrade_state_not_registry_name_or_internals() {
        let mut dom = html::parse(
            "<x-ready id=x></x-ready><div id=d></div><annotation-xml id=r></annotation-xml>",
        )
        .into_runtime()
        .unwrap();
        let x = dom.find_by_id("x").unwrap();
        assert!(!dom.element_defined(x).unwrap());
        assert!(dom.element_defined(dom.find_by_id("d").unwrap()).unwrap());
        assert!(dom.element_defined(dom.find_by_id("r").unwrap()).unwrap());
        dom.set_custom_element_defined(x, true).unwrap();
        assert!(dom.element_defined(x).unwrap());
        dom.set_custom_element_defined(x, false).unwrap();
        assert!(!dom.element_defined(x).unwrap());
        assert!(valid_custom_element_name("x-a:b"));
        assert!(valid_custom_element_name("x-é"));
    }
    #[test]
    fn target_uses_decoded_fragment_document_order_and_excludes_shadow() {
        let mut dom = html::parse("<a name='é' id=anchor></a><div id='é'></div>")
            .into_runtime()
            .unwrap();
        let document = dom.document();
        dom.arena_mut()
            .get_mut(document)
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .url = Some("https://example.test/#%C3%A9".into());
        assert_eq!(
            dom.document_target(document).unwrap(),
            Some(dom.find_by_id("é").unwrap())
        );
    }
}

#[cfg(test)]
mod permission_selector_tests {
    use super::*;
    #[test]
    fn granted_state_is_native_element_specific_and_revoked_on_adoption() {
        let mut dom = DomRuntime::new();
        let root = dom.document();
        let geo = dom.create_element("geolocation");
        let div = dom.create_element("div");
        dom.append_child(root, geo).unwrap();
        dom.append_child(root, div).unwrap();
        dom.set_attribute(div, "granted", "").unwrap();
        dom.set_permission_granted(div, true).unwrap();
        assert!(!dom.element_permission_granted(div).unwrap());
        dom.set_permission_granted(geo, true).unwrap();
        assert!(dom.matches_selector(geo, ":granted").unwrap());
        let destination = dom.create(NodeKind::Document);
        dom.retain_native(destination).unwrap();
        dom.retain_wrapper(geo).unwrap();
        let document = dom.arena().document_id(destination).unwrap();
        let identity = dom.arena().node_ref(geo).unwrap();
        dom.adopt_node(document, identity).unwrap();
        assert!(!dom.element_permission_granted(geo).unwrap());
    }
}

#[cfg(test)]
mod custom_name_tests {
    use super::*;
    #[test]
    fn custom_names_follow_relaxed_html_grammar_instead_of_xml_ranges() {
        for name in [
            "x-!",
            "x-:",
            "x-<",
            "x-\u{ffff}",
            "x-\u{10ffff}",
            "annotation-xml-custom",
        ] {
            assert!(valid_custom_element_name(name), "{name:?}");
        }
        for name in [
            "",
            "x",
            "X-a",
            "x-A",
            "x- ",
            "x-/",
            "x->",
            "x-\0",
            "annotation-xml",
            "font-face",
            "missing-glyph",
        ] {
            assert!(!valid_custom_element_name(name), "{name:?}");
        }
        assert!(valid_custom_element_name(&crate::dom_string::from_units(
            &[120, 45, 0xd800]
        )));
        assert!(!valid_custom_element_name(&crate::dom_string::from_units(
            &[120, 45, 0xd800, 47]
        )));
        assert!(!valid_custom_element_name(&crate::dom_string::from_units(
            &[120, 45, 0xd800, 65]
        )));
    }
}
