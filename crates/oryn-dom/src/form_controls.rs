use super::{DomError, DomRuntime, Mutation, Node, NodeId, NodeKind};

const HTML: &str = "http://www.w3.org/1999/xhtml";

fn valid_email_address(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&byte))
        && domain.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

impl Node {
    pub fn checked(&self) -> bool {
        self.checkedness
            .unwrap_or_else(|| self.attributes.contains_null("checked"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    fn by_id(dom: &DomRuntime, id: &str) -> NodeId {
        dom.get_element_by_id(dom.document(), id).unwrap().unwrap()
    }

    #[test]
    fn email_type_mismatch_uses_html_not_mail_server_address_rules() {
        let mut dom = html::parse("<input id=e type=EMAIL>")
            .into_runtime()
            .unwrap();
        let node = by_id(&dom, "e");
        for value in [
            "",
            "a@b",
            ".a..@example.test",
            "a+b@example.test",
            "a@xn--caf-dma.test",
        ] {
            dom.set_value(node, value, false).unwrap();
            assert!(!dom.input_email_type_mismatch(node).unwrap(), "{value}");
        }
        for value in [
            "not-an-email",
            "@example.test",
            "a@",
            "a@@b",
            "a@-b",
            "a@b-",
            "a@b..c",
            "a b@c",
            "é@b",
            "a@b,c@d",
        ] {
            dom.set_value(node, value, false).unwrap();
            assert!(dom.input_email_type_mismatch(node).unwrap(), "{value}");
        }
        dom.set_value(node, format!("a@{}", "b".repeat(64)), false)
            .unwrap();
        assert!(dom.input_email_type_mismatch(node).unwrap());
        dom.set_attribute(node, "multiple", "").unwrap();
        dom.set_value(node, "a@b, c@d", false).unwrap();
        assert!(!dom.input_email_type_mismatch(node).unwrap());
        for value in ["a@b,", ",a@b", "a@b,,c@d"] {
            dom.set_value(node, value, false).unwrap();
            assert!(dom.input_email_type_mismatch(node).unwrap(), "{value}");
        }
        dom.set_attribute(node, "type", "text").unwrap();
        assert!(!dom.input_email_type_mismatch(node).unwrap());
    }

    #[test]
    fn submit_classification_rejects_non_elements_without_reading_attributes() {
        let mut dom =
            html::parse("<button id=b>Submit</button><input id=i type=image><div id=d></div>")
                .into_runtime()
                .unwrap();
        let text = dom.create_text("text");
        for node in [dom.document(), text, by_id(&dom, "d")] {
            assert!(!dom.is_submit_button(node).unwrap());
        }
        assert!(dom.is_submit_button(by_id(&dom, "b")).unwrap());
        assert!(dom.is_submit_button(by_id(&dom, "i")).unwrap());
    }

    #[test]
    fn email_sanitization_preserves_dirty_value_and_attribute_transition_semantics() {
        let mut dom = html::parse("<input id=e type=email multiple value=' a@b , c@d '>")
            .into_runtime()
            .unwrap();
        let e = by_id(&dom, "e");
        assert_eq!(dom.control_value(e).unwrap(), "a@b,c@d");
        dom.remove_attribute(e, "multiple").unwrap();
        assert_eq!(dom.control_value(e).unwrap(), "a@b,c@d");
        assert!(!dom.arena.get(e).unwrap().dirty_value);
        dom.set_attribute(e, "value", " c@d ").unwrap();
        assert_eq!(dom.control_value(e).unwrap(), "c@d");
        dom.set_value(e, "\r\n x@y \t", false).unwrap();
        assert_eq!(dom.control_value(e).unwrap(), "x@y");
        dom.set_attribute(e, "value", "default@host").unwrap();
        assert_eq!(dom.control_value(e).unwrap(), "x@y");
        dom.reset_control_value(e, "default@host".into()).unwrap();
        dom.set_attribute(e, "value", " next@host ").unwrap();
        assert_eq!(dom.control_value(e).unwrap(), "next@host");
        dom.set_value(e, " a@b , c@d ", false).unwrap();
        dom.set_attribute(e, "multiple", "").unwrap();
        assert_eq!(dom.control_value(e).unwrap(), "a@b,c@d");
        dom.remove_attribute(e, "multiple").unwrap();
        assert_eq!(dom.control_value(e).unwrap(), "a@b,c@d");
        assert!(dom.control_type_mismatch(e).unwrap());
    }

    #[test]
    fn url_type_mismatch_requires_an_absolute_url_and_sanitizes_values() {
        let mut dom = html::parse("<input id=u type=url>").into_runtime().unwrap();
        let u = by_id(&dom, "u");
        for value in [
            "",
            "https://example.test/path",
            "mailto:a@b",
            "\r\n https://example.test \t",
        ] {
            dom.set_value(u, value, false).unwrap();
            assert!(!dom.control_type_mismatch(u).unwrap(), "{value}");
        }
        for value in ["abc", "/relative", "http://[", "https://"] {
            dom.set_value(u, value, false).unwrap();
            assert!(dom.control_type_mismatch(u).unwrap(), "{value}");
        }
    }

    #[test]
    fn form_collection_membership_is_not_submission_eligibility() {
        let dom = html::parse("<!doctype html><input id=before form=f><form id=f><fieldset id=fs disabled><input id=hidden type=hidden><input id=radio type=radio><button id=button></button></fieldset><select id=select></select><textarea id=textarea></textarea><output id=output></output><object id=object></object><input id=image type=IMAGE><label id=label></label><meter id=meter></meter><progress id=progress></progress><input id=redirected form=other></form><form id=other></form><input id=after form=f>").into_runtime().unwrap();
        let expected = [
            "before", "fs", "hidden", "radio", "button", "select", "textarea", "output", "object",
            "after",
        ]
        .map(|id| by_id(&dom, id));
        assert_eq!(
            dom.built_in_form_controls(by_id(&dom, "f")).unwrap(),
            expected
        );
        assert_eq!(
            dom.built_in_form_controls(by_id(&dom, "other")).unwrap(),
            vec![by_id(&dom, "redirected")]
        );
    }

    #[test]
    fn form_collection_recomputes_ownership_order_and_type_changes() {
        let mut dom = html::parse(
            "<form id=f><input id=inner></form><input id=external form=f><form id=g></form>",
        )
        .into_runtime()
        .unwrap();
        let (f, g, inner, external) = (
            by_id(&dom, "f"),
            by_id(&dom, "g"),
            by_id(&dom, "inner"),
            by_id(&dom, "external"),
        );
        assert_eq!(
            dom.built_in_form_controls(f).unwrap(),
            vec![inner, external]
        );
        dom.append_child(f, external).unwrap();
        dom.append_child(f, inner).unwrap();
        assert_eq!(
            dom.built_in_form_controls(f).unwrap(),
            vec![external, inner]
        );
        dom.set_attribute(external, "form", "g").unwrap();
        assert_eq!(dom.built_in_form_controls(f).unwrap(), vec![inner]);
        assert_eq!(dom.built_in_form_controls(g).unwrap(), vec![external]);
        dom.set_attribute(inner, "type", "image").unwrap();
        assert!(dom.built_in_form_controls(f).unwrap().is_empty());
        dom.set_attribute(inner, "type", "text").unwrap();
        dom.set_attribute(f, "id", "renamed").unwrap();
        assert_eq!(dom.built_in_form_controls(f).unwrap(), vec![inner]);
        dom.remove(f).unwrap();
        // Explicit form association does not resolve in a disconnected tree.
        assert_eq!(dom.built_in_form_controls(f).unwrap(), vec![inner]);
        assert!(dom.built_in_form_controls(g).unwrap().is_empty());
        dom.remove_attribute(external, "form").unwrap();
        assert_eq!(
            dom.built_in_form_controls(f).unwrap(),
            vec![external, inner]
        );
    }

    #[test]
    fn form_collection_uses_first_id_match_and_does_not_inherit_form_attribute() {
        let mut dom=html::parse("<span id=f></span><form id=f><input id=inside><fieldset id=fs form=g><input id=child></fieldset></form><input id=outside form=f><form id=g></form>").into_runtime().unwrap();
        let blocker = by_id(&dom, "f");
        let form = dom
            .arena()
            .iter()
            .find_map(|(id, _)| dom.is_element(id, HTML, "form").then_some(id))
            .unwrap();
        let (inside, fs, child, outside, g) = (
            by_id(&dom, "inside"),
            by_id(&dom, "fs"),
            by_id(&dom, "child"),
            by_id(&dom, "outside"),
            by_id(&dom, "g"),
        );
        assert_eq!(
            dom.built_in_form_controls(form).unwrap(),
            vec![inside, child]
        );
        assert_eq!(dom.built_in_form_controls(g).unwrap(), vec![fs]);
        dom.remove(blocker).unwrap();
        assert_eq!(
            dom.built_in_form_controls(form).unwrap(),
            vec![inside, child, outside]
        );
    }

    #[test]
    fn form_collection_named_matches_preserve_tree_order_and_deduplicate() {
        let mut dom = html::parse("<form id=f><input name=group id=a><button id=group name=group></button><input name=Group id=c><input type=image name=group></form>").into_runtime().unwrap();
        let (f, a, b, c) = (
            by_id(&dom, "f"),
            by_id(&dom, "a"),
            by_id(&dom, "group"),
            by_id(&dom, "c"),
        );
        assert_eq!(
            dom.named_built_in_form_controls(f, "group").unwrap(),
            vec![a, b]
        );
        assert_eq!(
            dom.named_built_in_form_controls(f, "Group").unwrap(),
            vec![c]
        );
        assert!(dom.named_built_in_form_controls(f, "").unwrap().is_empty());
        dom.set_attribute(a, "name", "different").unwrap();
        assert_eq!(
            dom.named_built_in_form_controls(f, "group").unwrap(),
            vec![b]
        );
        dom.set_attribute(a, "id", "group").unwrap();
        assert_eq!(
            dom.named_built_in_form_controls(f, "group").unwrap(),
            vec![a, b]
        );
    }

    #[test]
    fn form_collection_follows_nearest_dynamic_form_owner() {
        let mut dom = html::parse("<form id=outer><input id=c></form>")
            .into_runtime()
            .unwrap();
        let (outer, c) = (by_id(&dom, "outer"), by_id(&dom, "c"));
        let inner = dom.create_element("form");
        dom.append_child(outer, inner).unwrap();
        dom.append_child(inner, c).unwrap();
        assert!(dom.built_in_form_controls(outer).unwrap().is_empty());
        assert_eq!(dom.built_in_form_controls(inner).unwrap(), vec![c]);
        dom.set_attribute(c, "form", "outer").unwrap();
        assert_eq!(dom.built_in_form_controls(outer).unwrap(), vec![c]);
        assert!(dom.built_in_form_controls(inner).unwrap().is_empty());
    }

    #[test]
    fn form_collection_stays_within_its_document_or_shadow_tree() {
        for mode in [crate::ShadowMode::Open, crate::ShadowMode::Closed] {
            let mut dom = html::parse("<form id=f><div id=host></div><input id=light></form>")
                .into_runtime()
                .unwrap();
            let (outer, host, light) =
                (by_id(&dom, "f"), by_id(&dom, "host"), by_id(&dom, "light"));
            let shadow = dom.create(NodeKind::ShadowRoot {
                mode,
                clonable: false,
            });
            dom.append_child(host, shadow).unwrap();
            let inner = dom.create_element("form");
            dom.set_attribute(inner, "id", "f").unwrap();
            dom.append_child(shadow, inner).unwrap();
            let external = dom.create_element("input");
            dom.set_attribute(external, "form", "f").unwrap();
            dom.append_child(shadow, external).unwrap();
            assert_eq!(dom.built_in_form_controls(outer).unwrap(), vec![light]);
            assert_eq!(dom.built_in_form_controls(inner).unwrap(), vec![external]);
            dom.remove(inner).unwrap();
            assert!(dom.form_owner(external).unwrap().is_none());
            assert_eq!(dom.built_in_form_controls(outer).unwrap(), vec![light]);
        }
    }

    #[test]
    fn checkedness_is_dirty_state_not_an_attribute() {
        let mut dom = html::parse("<input id=c type=checkbox checked>")
            .into_runtime()
            .unwrap();
        let c = by_id(&dom, "c");
        dom.set_checked(c, false).unwrap();
        assert!(!dom.checked(c).unwrap());
        assert!(
            dom.attribute_in_null_namespace(c, "checked")
                .unwrap()
                .is_some()
        );
        dom.set_attribute(c, "checked", "false").unwrap();
        assert!(!dom.checked(c).unwrap());
        dom.set_indeterminate(c, true).unwrap();
        let clone = dom.clone_node(c, false).unwrap();
        assert!(!dom.checked(clone).unwrap());
        assert!(dom.arena().get(clone).unwrap().indeterminate);
        dom.reset_checkedness(c).unwrap();
        assert!(dom.checked(c).unwrap());
        dom.remove_attribute(c, "checked").unwrap();
        assert!(!dom.checked(c).unwrap());
        dom.set_attribute(c, "checked", "").unwrap();
        assert!(dom.checked(c).unwrap());
    }

    #[test]
    fn radio_groups_use_tree_form_and_case_sensitive_nonempty_name() {
        let mut dom = html::parse("<form id=f><input id=a type=radio name=g checked><input id=b type=radio name=g checked><input id=c type=radio name=G checked></form><input id=d type=radio name=g checked><input id=e type=radio name=g form=f>").into_runtime().unwrap();
        let (a, b, c, d, e) = (
            by_id(&dom, "a"),
            by_id(&dom, "b"),
            by_id(&dom, "c"),
            by_id(&dom, "d"),
            by_id(&dom, "e"),
        );
        assert!(!dom.checked(a).unwrap());
        assert!(dom.checked(b).unwrap() && dom.checked(c).unwrap() && dom.checked(d).unwrap());
        assert!(!dom.arena().get(a).unwrap().dirty_checkedness);
        dom.set_checked(e, true).unwrap();
        assert!(!dom.checked(b).unwrap());
        assert!(dom.checked(d).unwrap());
        dom.set_attribute(e, "form", "missing").unwrap();
        assert!(!dom.checked(d).unwrap());
        dom.set_attribute(c, "name", "g").unwrap();
        dom.set_checked(a, true).unwrap();
        assert!(!dom.checked(c).unwrap());
    }

    #[test]
    fn disconnected_insertion_does_not_group_until_form_owner_or_connection_changes() {
        let mut dom = html::parse("<div id=container></div><form id=f></form>")
            .into_runtime()
            .unwrap();
        let root = by_id(&dom, "container");
        dom.remove(root).unwrap();
        let a = dom.create_element("input");
        let b = dom.create_element("input");
        for node in [a, b] {
            dom.set_attribute(node, "type", "radio").unwrap();
            dom.set_attribute(node, "name", "g").unwrap();
            dom.set_checked(node, true).unwrap();
            dom.append_child(root, node).unwrap();
        }
        assert!(dom.checked(a).unwrap() && dom.checked(b).unwrap());
        let form = by_id(&dom, "f");
        dom.append_child(form, root).unwrap();
        assert!(!dom.checked(a).unwrap() && dom.checked(b).unwrap());
    }

    #[test]
    fn form_id_changes_reassociate_external_radios() {
        let mut dom = html::parse("<form id=f><input id=a name=g type=radio checked></form><input id=b name=g type=radio form=other checked>").into_runtime().unwrap();
        let (a, b, f) = (by_id(&dom, "a"), by_id(&dom, "b"), by_id(&dom, "f"));
        dom.set_attribute(f, "id", "other").unwrap();
        assert!(!dom.checked(a).unwrap() && dom.checked(b).unwrap());
    }
}

impl DomRuntime {
    pub(super) fn sanitized_input_value(
        &self,
        node: NodeId,
        value: &str,
    ) -> Result<String, DomError> {
        if !self.is_element(node, HTML, "input") {
            return Ok(value.into());
        }
        let kind = self
            .attribute_in_null_namespace(node, "type")?
            .unwrap_or("text");
        if super::numeric_controls::numeric_kind(kind) {
            return self.sanitize_numeric_value(node, &kind.to_ascii_lowercase(), value);
        }
        if kind.eq_ignore_ascii_case("email") || kind.eq_ignore_ascii_case("url") {
            if kind.eq_ignore_ascii_case("email")
                && self
                    .attribute_in_null_namespace(node, "multiple")?
                    .is_some()
            {
                return Ok(value
                    .split(',')
                    .map(|token| token.trim_matches(['\t', '\n', '\x0c', '\r', ' ']))
                    .collect::<Vec<_>>()
                    .join(","));
            }
            return Ok(value
                .replace(['\r', '\n'], "")
                .trim_matches(['\t', '\n', '\x0c', '\r', ' '])
                .into());
        }
        Ok(value.into())
    }

    pub(super) fn text_input_attribute_transition_value(
        &self,
        node: NodeId,
        name: &str,
        value: &str,
    ) -> Result<Option<String>, DomError> {
        if self.is_element(node, HTML, "input")
            && matches!(name, "type" | "multiple" | "min" | "max" | "step")
            && !self.is_file_input(node)
            && (self
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|kind| {
                    kind.eq_ignore_ascii_case("email")
                        || kind.eq_ignore_ascii_case("url")
                        || super::numeric_controls::numeric_kind(kind)
                })
                || (name == "type"
                    && (value.eq_ignore_ascii_case("email")
                        || value.eq_ignore_ascii_case("url")
                        || super::numeric_controls::numeric_kind(value))))
        {
            return self.control_value(node).map(Some);
        }
        Ok(None)
    }

    pub(super) fn sanitize_after_input_attribute(
        &mut self,
        node: NodeId,
        name: &str,
        old_value: Option<String>,
    ) -> Result<(), DomError> {
        if !self.is_element(node, HTML, "input") {
            return Ok(());
        }
        if name == "value" && !self.arena.get(node)?.dirty_value {
            self.arena.get_mut(node)?.value = None;
        } else if let Some(value) = old_value
            && !self.is_file_input(node)
        {
            let value = self.sanitized_input_value(node, &value)?;
            self.arena.get_mut(node)?.value = Some(value);
        }
        Ok(())
    }

    pub fn reset_control_value(&mut self, node: NodeId, value: String) -> Result<(), DomError> {
        self.set_value(node, value, false)?;
        self.arena.get_mut(node)?.dirty_value = false;
        self.arena.get_mut(node)?.pending_edit_value = None;
        Ok(())
    }

    pub fn control_will_validate(&self, node: NodeId) -> Result<bool, DomError> {
        if self.is_element(node, HTML, "select") {
            return self.select_will_validate(node);
        }
        let input = self.is_element(node, HTML, "input");
        let button = self.is_element(node, HTML, "button");
        if (!input && !button && !self.is_element(node, HTML, "textarea"))
            || self.control_disabled(node)?
        {
            return Ok(false);
        }
        let kind = self
            .attribute_in_null_namespace(node, "type")?
            .unwrap_or("text")
            .to_ascii_lowercase();
        if input && matches!(kind.as_str(), "hidden" | "button" | "reset") {
            return Ok(false);
        }
        if self
            .attribute_in_null_namespace(node, "readonly")?
            .is_some()
            && !button
        {
            return Ok(false);
        }
        if button && !self.is_submit_button(node)? {
            return Ok(false);
        }
        Ok(!self
            .arena
            .ancestors(node)?
            .iter()
            .any(|id| self.is_element(*id, HTML, "datalist")))
    }

    pub fn control_value_missing(&self, node: NodeId) -> Result<bool, DomError> {
        if self.is_element(node, HTML, "select") {
            return self.select_value_missing(node);
        }
        if self.is_radio(node) {
            if self
                .attribute_in_null_namespace(node, "name")?
                .unwrap_or("")
                .is_empty()
            {
                return Ok(false);
            }
            let peers = self
                .arena
                .iter()
                .filter(|(peer, _)| {
                    *peer == node || self.same_radio_group(node, *peer).unwrap_or(false)
                })
                .collect::<Vec<_>>();
            return Ok(peers
                .iter()
                .any(|(_, record)| record.attributes.contains_null("required"))
                && !peers.iter().any(|(_, record)| record.checked()));
        }
        let record = self.arena.get(node)?;
        if !record.attributes.contains_null("required") {
            return Ok(false);
        }
        if self.is_file_input(node) {
            return Ok(record.selected_files.is_empty());
        }
        if self.is_element(node, HTML, "input") {
            match record
                .attributes
                .get_null("type")
                .map(|s| s.to_ascii_lowercase())
                .as_deref()
            {
                Some("checkbox") => return Ok(!record.checked()),
                Some("hidden" | "button" | "reset" | "submit" | "image" | "range" | "color") => {
                    return Ok(false);
                }
                _ => {}
            }
        } else if !self.is_element(node, HTML, "textarea") {
            return Ok(false);
        }
        Ok(!self.control_disabled(node)?
            && !record.attributes.contains_null("readonly")
            && self.control_value(node)?.is_empty())
    }

    pub fn control_validity(&self, node: NodeId) -> Result<bool, DomError> {
        let numeric = self.control_numeric_validity(node)?;
        let (too_short, too_long) = self.control_length_validity(node)?;
        Ok(!self.control_value_missing(node)?
            && !too_short
            && !too_long
            && !self.control_type_mismatch(node)?
            && !self.control_pattern_mismatch(node)?
            && !numeric.range_underflow
            && !numeric.range_overflow
            && !numeric.step_mismatch
            && self.custom_validity_message(node)?.is_empty())
    }

    /// Shared truth for selector matching and computed style. Eligibility is
    /// separate from the live ValidityState flags of a barred control.
    pub fn validation_selector_state(&self, node: NodeId) -> Result<Option<bool>, DomError> {
        if self.is_element(node, HTML, "form") {
            return Ok(Some(self.invalid_form_controls(node)?.is_empty()));
        }
        if self.is_element(node, HTML, "fieldset") {
            for (child, _) in self.arena.iter() {
                if self.arena.ancestors(child)?.contains(&node)
                    && self.control_will_validate(child)?
                    && !self.control_validity(child)?
                {
                    return Ok(Some(false));
                }
            }
            return Ok(Some(true));
        }
        if self.control_will_validate(node)? {
            Ok(Some(self.control_validity(node)?))
        } else {
            Ok(None)
        }
    }

    pub fn invalid_form_controls(&self, form: NodeId) -> Result<Vec<NodeId>, DomError> {
        self.submittable_form_controls(form)?
            .into_iter()
            .filter_map(|node| {
                match self
                    .control_will_validate(node)
                    .and_then(|eligible| Ok(eligible && !self.control_validity(node)?))
                {
                    Ok(true) => Some(Ok(node)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .collect()
    }

    pub fn is_submit_button(&self, node: NodeId) -> Result<bool, DomError> {
        if !self.is_element(node, HTML, "input") && !self.is_element(node, HTML, "button") {
            return Ok(false);
        }
        let kind = self
            .attribute_in_null_namespace(node, "type")?
            .unwrap_or("");
        if self.is_element(node, HTML, "input") {
            return Ok(kind.eq_ignore_ascii_case("submit") || kind.eq_ignore_ascii_case("image"));
        }
        Ok(self.is_element(node, HTML, "button")
            && (kind.eq_ignore_ascii_case("submit")
                || (!kind.eq_ignore_ascii_case("button")
                    && !kind.eq_ignore_ascii_case("reset")
                    && self.attribute_in_null_namespace(node, "command")?.is_none()
                    && self
                        .attribute_in_null_namespace(node, "commandfor")?
                        .is_none()
                    && !self
                        .parent(node)?
                        .is_some_and(|parent| self.is_element(parent, HTML, "select")))))
    }

    pub fn control_type_mismatch(&self, node: NodeId) -> Result<bool, DomError> {
        if self.is_element(node, HTML, "input")
            && self
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("url"))
        {
            let value = self.control_value(node)?;
            return Ok(!value.is_empty() && url::Url::parse(&value).is_err());
        }
        self.input_email_type_mismatch(node)
    }

    /// Email-state type mismatch, independent of requiredness and whether the
    /// control is currently a candidate for constraint validation.
    /// https://html.spec.whatwg.org/multipage/input.html#email-state-(type=email)
    pub fn input_email_type_mismatch(&self, node: NodeId) -> Result<bool, DomError> {
        if !self.is_element(node, HTML, "input")
            || !self
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("email"))
        {
            return Ok(false);
        }
        let value = self.control_value(node)?;
        if value.is_empty() {
            return Ok(false);
        }
        let valid = if self
            .attribute_in_null_namespace(node, "multiple")?
            .is_some()
        {
            value.split(',').all(|token| {
                valid_email_address(token.trim_matches(['\t', '\n', '\x0c', '\r', ' ']))
            })
        } else {
            valid_email_address(&value)
        };
        Ok(!valid)
    }

    /// Built-in listed descendants of a fieldset, independent of form owner.
    /// Unlike form.elements, this includes image inputs. FACE membership awaits
    /// the custom-element internals registry.
    pub fn built_in_fieldset_controls(&self, fieldset: NodeId) -> Result<Vec<NodeId>, DomError> {
        let record = self.arena.get(fieldset)?;
        if !self.is_element(fieldset, HTML, "fieldset") {
            return Ok(Vec::new());
        }
        let mut pending: Vec<_> = record.children.iter().rev().copied().collect();
        let mut controls = Vec::new();
        while let Some(node) = pending.pop() {
            let record = self.arena.get(node)?;
            if matches!(
                record.kind,
                NodeKind::Document | NodeKind::ShadowRoot { .. }
            ) {
                continue;
            }
            pending.extend(record.children.iter().rev().copied());
            if matches!(&record.kind, NodeKind::Element { name }
                if name.ns.as_ref() == HTML && matches!(name.local.as_ref(),
                    "button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea"))
            {
                controls.push(node);
            }
        }
        Ok(controls)
    }

    /// Built-in listed controls represented by form.elements, in tree order.
    /// This is not the successful-controls filter used for submission: disabled,
    /// unnamed, unchecked and hidden controls remain members. Form-associated
    /// custom elements require the separate custom-element internals registry.
    pub fn built_in_form_controls(&self, form: NodeId) -> Result<Vec<NodeId>, DomError> {
        self.collect_form_controls(form, false)
    }

    pub fn submittable_form_controls(&self, form: NodeId) -> Result<Vec<NodeId>, DomError> {
        self.collect_form_controls(form, true)
    }

    fn collect_form_controls(
        &self,
        form: NodeId,
        include_images: bool,
    ) -> Result<Vec<NodeId>, DomError> {
        self.arena.get(form)?;
        if !self.is_element(form, HTML, "form") {
            return Ok(Vec::new());
        }
        let root = self.control_tree_root(form)?;
        let mut pending = vec![root];
        let mut controls = Vec::new();
        while let Some(node) = pending.pop() {
            let record = self.arena.get(node)?;
            if node != root
                && matches!(
                    record.kind,
                    NodeKind::Document | NodeKind::ShadowRoot { .. }
                )
            {
                continue;
            }
            pending.extend(record.children.iter().rev().copied());
            let NodeKind::Element { name } = &record.kind else {
                continue;
            };
            if name.ns.as_ref() != HTML
                || !matches!(
                    name.local.as_ref(),
                    "button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea"
                )
            {
                continue;
            }
            if !include_images
                && name.local.as_ref() == "input"
                && self
                    .attribute_in_null_namespace(node, "type")?
                    .is_some_and(|value| value.eq_ignore_ascii_case("image"))
            {
                continue;
            }
            if self.form_owner(node)? == Some(form) {
                controls.push(node);
            }
        }
        Ok(controls)
    }

    /// All matching listed controls, without ID-over-name precedence or
    /// duplicate entries when both attributes match. The binding layer decides
    /// whether to expose null, a single Element, or a live RadioNodeList.
    pub fn named_built_in_form_controls(
        &self,
        form: NodeId,
        name: &str,
    ) -> Result<Vec<NodeId>, DomError> {
        if name.is_empty() {
            self.arena.get(form)?;
            return Ok(Vec::new());
        }
        self.built_in_form_controls(form)?
            .into_iter()
            .filter_map(|node| {
                let matches = self.attribute_in_null_namespace(node, "id").and_then(|id| {
                    self.attribute_in_null_namespace(node, "name")
                        .map(|value| id == Some(name) || value == Some(name))
                });
                match matches {
                    Ok(true) => Some(Ok(node)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .collect()
    }

    /// HTML integer parsing accepts an initial signed decimal prefix, not
    /// JavaScript's number syntax or Rust's whole-string integer syntax.
    pub fn parsed_tab_index(&self, node: NodeId) -> Result<Option<i32>, DomError> {
        let Some(value) = self.attribute_in_null_namespace(node, "tabindex")? else {
            return Ok(None);
        };
        let value = value.trim_start_matches(['\t', '\n', '\x0c', '\r', ' ']);
        let sign = usize::from(value.starts_with(['+', '-']));
        let digits = value[sign..].bytes().take_while(u8::is_ascii_digit).count();
        Ok((digits > 0)
            .then(|| value[..sign + digits].parse().ok())
            .flatten())
    }

    pub fn control_disabled(&self, node: NodeId) -> Result<bool, DomError> {
        let record = self.arena.get(node)?;
        if !matches!(&record.kind, NodeKind::Element { name } if name.ns.as_ref() == HTML
            && matches!(name.local.as_ref(), "input" | "button" | "select" | "textarea" | "fieldset" | "option" | "optgroup"))
        {
            return Ok(false);
        }
        if self
            .attribute_in_null_namespace(node, "disabled")?
            .is_some()
        {
            return Ok(true);
        }
        let mut cursor = record.parent;
        while let Some(parent) = cursor {
            if self.is_element(parent, HTML, "fieldset")
                && self
                    .attribute_in_null_namespace(parent, "disabled")?
                    .is_some()
            {
                let first_legend = self
                    .children(parent)?
                    .iter()
                    .copied()
                    .find(|child| self.is_element(*child, HTML, "legend"));
                let exempt = first_legend.is_some_and(|legend| {
                    legend == node
                        || self
                            .arena
                            .ancestors(node)
                            .is_ok_and(|ancestors| ancestors.contains(&legend))
                });
                if !exempt {
                    return Ok(true);
                }
            }
            cursor = self.parent(parent)?;
        }
        Ok(false)
    }

    pub fn is_labelable(&self, node: NodeId) -> bool {
        self.arena.get(node).is_ok_and(|record| matches!(&record.kind,
            NodeKind::Element { name } if name.ns.as_ref() == HTML &&
            (matches!(name.local.as_ref(), "button" | "meter" | "output" | "progress" | "select" | "textarea")
             || (name.local.as_ref() == "input" && !record.attributes.get_null("type").is_some_and(|kind| kind.eq_ignore_ascii_case("hidden"))))))
    }

    pub fn label_control(&self, label: NodeId) -> Result<Option<NodeId>, DomError> {
        if let Some(id) = self.attribute_in_null_namespace(label, "for")? {
            if id.is_empty() {
                return Ok(None);
            }
            return Ok(self
                .get_element_by_id(self.control_tree_root(label)?, id)?
                .filter(|node| self.is_labelable(*node)));
        }
        let mut pending = self
            .children(label)?
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        while let Some(node) = pending.pop() {
            if self.is_labelable(node) {
                return Ok(Some(node));
            }
            let record = self.arena.get(node)?;
            if !matches!(record.kind, NodeKind::ShadowRoot { .. }) {
                pending.extend(record.children.iter().rev().copied());
            }
        }
        Ok(None)
    }

    pub fn is_interactive_content(&self, node: NodeId) -> bool {
        self.arena.get(node).is_ok_and(|record| matches!(&record.kind,
            NodeKind::Element { name } if name.ns.as_ref() == HTML &&
            (matches!(name.local.as_ref(), "button" | "details" | "embed" | "iframe" | "label" | "select" | "textarea")
             || (name.local.as_ref() == "input" && !record.attributes.get_null("type").is_some_and(|kind| kind.eq_ignore_ascii_case("hidden")))
             || (name.local.as_ref() == "a" && record.attributes.contains_null("href"))
             || (matches!(name.local.as_ref(), "audio" | "video") && record.attributes.contains_null("controls"))
             || (name.local.as_ref() == "img" && record.attributes.contains_null("usemap")))))
    }

    pub fn checked(&self, node: NodeId) -> Result<bool, DomError> {
        Ok(self.arena.get(node)?.checked())
    }

    /// IDL/user checkedness changes are not content-attribute mutations.
    pub fn set_checked(&mut self, node: NodeId, checked: bool) -> Result<(), DomError> {
        self.arena.get_mut(node)?.dirty_checkedness = true;
        self.set_checkedness(node, checked)
    }

    pub fn set_checkedness(&mut self, node: NodeId, checked: bool) -> Result<(), DomError> {
        self.arena.get_mut(node)?.checkedness = Some(checked);
        self.mutations.push(Mutation::FormState { node });
        if checked {
            self.enforce_radio_group(node)?;
        }
        Ok(())
    }

    pub fn reset_checkedness(&mut self, node: NodeId) -> Result<(), DomError> {
        self.arena.get_mut(node)?.dirty_checkedness = false;
        let checked = self.attribute_in_null_namespace(node, "checked")?.is_some();
        self.set_checkedness(node, checked)
    }

    pub fn set_indeterminate(&mut self, node: NodeId, value: bool) -> Result<(), DomError> {
        self.arena.get_mut(node)?.indeterminate = value;
        self.mutations.push(Mutation::FormState { node });
        Ok(())
    }

    pub fn is_radio(&self, node: NodeId) -> bool {
        self.is_element(node, HTML, "input")
            && self
                .attribute_in_null_namespace(node, "type")
                .ok()
                .flatten()
                .is_some_and(|kind| kind.eq_ignore_ascii_case("radio"))
    }

    pub fn is_indeterminate(&self, node: NodeId) -> Result<bool, DomError> {
        if self.is_element(node, HTML, "progress") {
            return Ok(self.attribute_in_null_namespace(node, "value")?.is_none());
        }
        if self.is_radio(node) {
            if self.checked(node)? {
                return Ok(false);
            }
            for (peer, record) in self.arena.iter() {
                if record.checked() && self.same_radio_group(node, peer)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        Ok(self.is_element(node, HTML, "input")
            && self
                .attribute_in_null_namespace(node, "type")?
                .is_some_and(|kind| kind.eq_ignore_ascii_case("checkbox"))
            && self.arena.get(node)?.indeterminate)
    }

    pub fn control_tree_root(&self, mut node: NodeId) -> Result<NodeId, DomError> {
        loop {
            let record = self.arena.get(node)?;
            if matches!(
                record.kind,
                NodeKind::Document | NodeKind::ShadowRoot { .. }
            ) {
                return Ok(node);
            }
            match record.parent {
                Some(parent) => node = parent,
                None => return Ok(node),
            }
        }
    }

    /// Resolve explicit association before ancestor association; an unmatched
    /// explicit form attribute does not fall back to an ancestor form.
    pub fn form_owner(&self, node: NodeId) -> Result<Option<NodeId>, DomError> {
        let root = self.control_tree_root(node)?;
        if let Some(id) = self.attribute_in_null_namespace(node, "form")? {
            if !self.is_connected(node)? {
                return Ok(None);
            }
            let candidate = self.get_element_by_id(root, id)?;
            return Ok(candidate.filter(|candidate| self.is_element(*candidate, HTML, "form")));
        }
        let mut cursor = self.parent(node)?;
        while let Some(parent) = cursor {
            if self.is_element(parent, HTML, "form") {
                return Ok(Some(parent));
            }
            if parent == root {
                break;
            }
            cursor = self.parent(parent)?;
        }
        Ok(None)
    }

    pub fn same_radio_group(&self, a: NodeId, b: NodeId) -> Result<bool, DomError> {
        if !self.is_radio(a) || !self.is_radio(b) {
            return Ok(false);
        }
        let name = self.attribute_in_null_namespace(a, "name")?.unwrap_or("");
        Ok(!name.is_empty()
            && self.attribute_in_null_namespace(b, "name")? == Some(name)
            && self.control_tree_root(a)? == self.control_tree_root(b)?
            && self.form_owner(a)? == self.form_owner(b)?)
    }

    fn enforce_radio_group(&mut self, node: NodeId) -> Result<(), DomError> {
        if !self.is_radio(node) || !self.checked(node)? {
            return Ok(());
        }
        let mut peers = Vec::new();
        for (other, record) in self.arena.iter() {
            if other != node && record.checked() && self.same_radio_group(node, other)? {
                peers.push(other);
            }
        }
        for peer in peers {
            self.arena.get_mut(peer)?.checkedness = Some(false);
            self.mutations.push(Mutation::FormState { node: peer });
        }
        Ok(())
    }

    pub(super) fn checked_attribute_changed(
        &mut self,
        node: NodeId,
        name: &str,
    ) -> Result<(), DomError> {
        if name == "id" {
            self.refresh_radio_owners()?;
        }
        if !self.is_element(node, HTML, "input") {
            return Ok(());
        }
        if name == "checked" && !self.arena.get(node)?.dirty_checkedness {
            let checked = self.attribute_in_null_namespace(node, "checked")?.is_some();
            self.set_checkedness(node, checked)?;
        } else if matches!(name, "name" | "type" | "form") {
            self.enforce_radio_group(node)?;
        }
        if self.is_radio(node) {
            self.radio_form_owners.insert(node, self.form_owner(node)?);
        } else {
            self.radio_form_owners.remove(&node);
        }
        Ok(())
    }

    pub(super) fn initialize_checkedness(&mut self) -> Result<(), DomError> {
        for (node, _) in self.arena.iter() {
            if self.is_radio(node) {
                self.radio_form_owners.insert(node, None);
            }
        }
        self.refresh_radio_owners()?;
        let checked = self
            .arena
            .descendants_inclusive(self.document())?
            .into_iter()
            .filter(|node| self.is_radio(*node) && self.checked(*node).unwrap_or(false))
            .collect::<Vec<_>>();
        // Preserve parser order: the last checked radio in a group wins.
        for node in checked {
            self.set_checkedness(node, true)?;
        }
        Ok(())
    }

    pub(super) fn reconcile_inserted_radios(&mut self, root: NodeId) -> Result<(), DomError> {
        for node in self.arena.descendants_inclusive(root)? {
            if self.is_radio(node) {
                self.radio_form_owners.entry(node).or_insert(None);
            }
        }
        self.refresh_radio_owners()?;
        let checked = self
            .arena
            .descendants_inclusive(root)?
            .into_iter()
            .filter(|node| self.is_radio(*node) && self.checked(*node).unwrap_or(false))
            .collect::<Vec<_>>();
        for node in checked {
            if self.is_connected(node)? {
                self.set_checkedness(node, true)?;
            }
        }
        Ok(())
    }

    pub(super) fn refresh_radio_owners(&mut self) -> Result<(), DomError> {
        let radios = self
            .radio_form_owners
            .keys()
            .copied()
            .filter(|node| self.is_radio(*node))
            .collect::<Vec<_>>();
        let mut owners = std::collections::BTreeMap::new();
        let mut changed = Vec::new();
        for node in radios {
            let owner = self.form_owner(node)?;
            if self.radio_form_owners.get(&node).copied().flatten() != owner
                && self.checked(node)?
            {
                changed.push(node);
            }
            owners.insert(node, owner);
        }
        self.radio_form_owners = owners;
        for node in changed {
            self.set_checkedness(node, true)?;
        }
        Ok(())
    }
}
