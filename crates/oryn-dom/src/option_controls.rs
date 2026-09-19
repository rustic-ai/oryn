//! Native option state. Select list normalization is integrated separately.
use super::{DomArena, DomError, DomRuntime, Mutation, Node, NodeId, NodeKind};

const HTML: &str = "http://www.w3.org/1999/xhtml";

impl Node {
    pub fn selected(&self) -> bool {
        self.selectedness
            .unwrap_or_else(|| self.attributes.contains_null("selected"))
    }
}

impl DomRuntime {
    pub fn set_select_length(&mut self, select: NodeId, length: u32) -> Result<(), DomError> {
        let options = self.select_options(select)?;
        let length = length as usize;
        if length > options.len() {
            if length > 100_000 {
                return Ok(());
            }
            self.append_new_options(select, (length - options.len()) as u32, || true)?;
        } else {
            for option in options.into_iter().skip(length).rev() {
                self.remove(option)?;
            }
        }
        Ok(())
    }

    /// Append freshly created blank options as one fragment-equivalent batch.
    /// No existing node is moved and no observable fragment is exposed. Keep
    /// structural journal entries, but expose one compound child-list record.
    /// Returns false if creation is interrupted before insertion begins.
    pub fn append_new_options(
        &mut self,
        select: NodeId,
        count: u32,
        mut continue_work: impl FnMut() -> bool,
    ) -> Result<bool, DomError> {
        if !self.is_element(select, HTML, "select") {
            return Err(DomError::NotAnElement(select));
        }
        if count == 0 {
            return Ok(true);
        }
        let insertion_index = self.arena.get(select)?.children.len() as u32;
        let previous = self
            .arena
            .get(select)?
            .children
            .iter()
            .rev()
            .copied()
            .find(|child| {
                !matches!(
                    self.arena.get(*child).map(|node| &node.kind),
                    Ok(NodeKind::ShadowRoot { .. })
                )
            });
        let mut added_nodes = Vec::new();
        for _ in 0..count {
            if !continue_work() {
                return Ok(false);
            }
            added_nodes.push(self.create_element("option"));
        }
        // All children are fresh leaves: bypass repeated list scans and
        // selection normalization in the general single-node insertion path.
        for child in &added_nodes {
            self.arena.append_child(select, *child)?;
            if self.connected.contains(&select) {
                self.mark_connected(*child)?;
            }
        }
        self.adjust_ranges_for_insertion(select, insertion_index, count)?;
        for (index, child) in added_nodes.iter().enumerate() {
            self.mutations.push(Mutation::Inserted {
                node: *child,
                parent: Some(select),
                siblings: super::MutationSiblings {
                    previous: index
                        .checked_sub(1)
                        .map(|index| added_nodes[index])
                        .or(previous),
                    next: added_nodes.get(index + 1).copied(),
                    state_preserving_move: false,
                },
                observer_suppressed: true,
            });
        }
        self.mutations.push(Mutation::ChildrenReplaced {
            node: select,
            added_nodes,
            removed_nodes: Vec::new(),
            siblings: super::MutationSiblings {
                previous,
                next: None,
                state_preserving_move: false,
            },
        });
        self.normalize_select(select)?;
        Ok(true)
    }

    pub fn remove_select_option(&mut self, select: NodeId, index: i32) -> Result<(), DomError> {
        if index >= 0
            && let Some(option) = self.select_options(select)?.get(index as usize).copied()
        {
            self.remove(option)?;
        }
        Ok(())
    }

    pub fn option_nearest_select(&self, option: NodeId) -> Result<Option<NodeId>, DomError> {
        let mut cursor = self.parent(option)?;
        let mut in_optgroup = false;
        while let Some(node) = cursor {
            if ["datalist", "hr", "option"]
                .iter()
                .any(|tag| self.is_element(node, HTML, tag))
            {
                return Ok(None);
            }
            if self.is_element(node, HTML, "optgroup") {
                if in_optgroup {
                    return Ok(None);
                }
                in_optgroup = true;
            }
            if self.is_element(node, HTML, "select") {
                return Ok(Some(node));
            }
            if matches!(
                self.arena.get(node)?.kind,
                NodeKind::ShadowRoot { .. } | NodeKind::Document
            ) {
                break;
            }
            cursor = self.parent(node)?;
        }
        Ok(None)
    }

    pub(super) fn option_owners_in_subtree(
        &self,
        root: NodeId,
    ) -> Result<Vec<(NodeId, Option<NodeId>)>, DomError> {
        self.arena
            .descendants_inclusive(root)?
            .into_iter()
            .filter(|node| self.is_element(*node, HTML, "option"))
            .map(|node| Ok((node, self.option_nearest_select(node)?)))
            .collect()
    }

    pub(super) fn initialize_selects(&mut self) -> Result<(), DomError> {
        let selects: Vec<_> = self
            .arena
            .iter()
            .filter_map(|(node, _)| self.is_element(node, HTML, "select").then_some(node))
            .collect();
        for select in selects {
            self.normalize_select(select)?;
        }
        Ok(())
    }

    fn option_selection_changed(&mut self, option: NodeId) -> Result<(), DomError> {
        let Some(select) = self.option_nearest_select(option)? else {
            return Ok(());
        };
        let options = self.select_options(select)?;
        if !options.contains(&option) {
            return Ok(());
        }
        if self.option_selected(option)?
            && self
                .attribute_in_null_namespace(select, "multiple")?
                .is_none()
        {
            for other in options {
                if other != option && self.option_selected(other)? {
                    self.set_option_selectedness(other, false)?;
                }
            }
        }
        self.normalize_select(select)
    }

    /// The native list of options, not an unrestricted descendant selector.
    pub fn select_options(&self, select: NodeId) -> Result<Vec<NodeId>, DomError> {
        self.arena.select_options(select)
    }

    pub fn option_disabled(&self, option: NodeId) -> Result<bool, DomError> {
        if self
            .attribute_in_null_namespace(option, "disabled")?
            .is_some()
        {
            return Ok(true);
        }
        let mut cursor = self.parent(option)?;
        while let Some(node) = cursor {
            if self.is_element(node, HTML, "optgroup") {
                return Ok(self
                    .attribute_in_null_namespace(node, "disabled")?
                    .is_some());
            }
            if ["select", "hr", "datalist", "option"]
                .iter()
                .any(|tag| self.is_element(node, HTML, tag))
            {
                break;
            }
            cursor = self.parent(node)?;
        }
        Ok(false)
    }

    /// Text fallback strips/collapses ASCII whitespace, excluding script text.
    pub fn option_text(&self, option: NodeId) -> Result<String, DomError> {
        let mut pending: Vec<_> = self
            .arena
            .get(option)?
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        let mut units = Vec::new();
        while let Some(node) = pending.pop() {
            let record = self.arena.get(node)?;
            if matches!(
                record.kind,
                NodeKind::Document | NodeKind::ShadowRoot { .. }
            ) || matches!(&record.kind, NodeKind::Element { name } if name.local.as_ref() == "script"
                    && matches!(name.ns.as_ref(), HTML | "http://www.w3.org/2000/svg"))
            {
                continue;
            }
            if let NodeKind::Text { data } = &record.kind {
                units.extend(super::dom_string::units(data));
            }
            pending.extend(record.children.iter().rev().copied());
        }
        let mut collapsed = Vec::new();
        let mut space = false;
        for unit in units {
            if matches!(unit, 9 | 10 | 12 | 13 | 32) {
                space = !collapsed.is_empty();
            } else {
                if space {
                    collapsed.push(32);
                }
                collapsed.push(unit);
                space = false;
            }
        }
        Ok(super::dom_string::from_units(&collapsed))
    }

    pub fn option_value(&self, option: NodeId) -> Result<String, DomError> {
        self.option_reflected_text(option, "value")
    }

    pub fn option_label(&self, option: NodeId) -> Result<String, DomError> {
        self.option_reflected_text(option, "label")
    }

    fn option_reflected_text(&self, option: NodeId, name: &str) -> Result<String, DomError> {
        match self.attribute_in_null_namespace(option, name)? {
            Some(value) => Ok(value.to_owned()),
            None => self.option_text(option),
        }
    }

    pub fn option_index(&self, option: NodeId) -> Result<usize, DomError> {
        let Some(select) = self.option_nearest_select(option)? else {
            return Ok(0);
        };
        Ok(self
            .select_options(select)?
            .iter()
            .position(|node| *node == option)
            .unwrap_or(0))
    }

    /// IDL reflection, not the display-size algorithm used by normalization.
    pub fn select_size(&self, select: NodeId) -> Result<u32, DomError> {
        Ok(self
            .attribute_in_null_namespace(select, "size")?
            .and_then(|value| {
                let value = value.trim_start_matches(['\t', '\n', '\x0c', '\r', ' ']);
                let value = value.strip_prefix('+').unwrap_or(value);
                let digits = value.bytes().take_while(u8::is_ascii_digit).count();
                value[..digits]
                    .parse::<u32>()
                    .ok()
                    .filter(|size| *size <= i32::MAX as u32)
            })
            .unwrap_or(0))
    }

    /// Eligibility is distinct from validity: disabled controls can still have
    /// a missing value, but constraint validation must not dispatch for them.
    pub fn select_will_validate(&self, select: NodeId) -> Result<bool, DomError> {
        if !self.is_element(select, HTML, "select") || self.control_disabled(select)? {
            return Ok(false);
        }
        Ok(!self
            .arena
            .ancestors(select)?
            .iter()
            .any(|ancestor| self.is_element(*ancestor, HTML, "datalist")))
    }

    pub fn select_display_size(&self, select: NodeId) -> Result<u32, DomError> {
        let size = self
            .attribute_in_null_namespace(select, "size")?
            .and_then(|value| {
                let value = value.trim_start_matches(['\t', '\n', '\x0c', '\r', ' ']);
                let value = value.strip_prefix('+').unwrap_or(value);
                let digits = value.bytes().take_while(u8::is_ascii_digit).count();
                value[..digits].parse::<u32>().ok().filter(|size| *size > 0)
            });
        Ok(size.unwrap_or(
            if self
                .attribute_in_null_namespace(select, "multiple")?
                .is_some()
            {
                4
            } else {
                1
            },
        ))
    }

    pub fn select_value_missing(&self, select: NodeId) -> Result<bool, DomError> {
        if self
            .attribute_in_null_namespace(select, "required")?
            .is_none()
        {
            return Ok(false);
        }
        let options = self.select_options(select)?;
        let selected = options
            .iter()
            .copied()
            .filter(|option| self.arena.get(*option).is_ok_and(Node::selected))
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Ok(true);
        }
        let first = options[0];
        Ok(selected.len() == 1
            && selected[0] == first
            && self.select_display_size(select)? == 1
            && self.parent(first)? == Some(select)
            && self.option_value(first)?.is_empty())
    }

    pub fn set_custom_validity(&mut self, node: NodeId, message: String) -> Result<(), DomError> {
        if self.arena.get(node)?.custom_validity_message != message {
            self.arena.get_mut(node)?.custom_validity_message = message;
            self.mutations.push(Mutation::FormState { node });
        }
        Ok(())
    }

    pub fn select_validity(&self, select: NodeId) -> Result<bool, DomError> {
        Ok(!self.select_value_missing(select)? && self.custom_validity_message(select)?.is_empty())
    }

    pub fn custom_validity_message(&self, node: NodeId) -> Result<&str, DomError> {
        Ok(&self.arena.get(node)?.custom_validity_message)
    }

    /// Run the selectedness-setting algorithm only at its specified triggers;
    /// getters must not call it and undo an explicitly empty selection.
    pub fn normalize_select(&mut self, select: NodeId) -> Result<(), DomError> {
        let options = self.select_options(select)?;
        if self
            .attribute_in_null_namespace(select, "multiple")?
            .is_some()
        {
            return self.update_selected_content(select);
        }
        let selected = options
            .iter()
            .copied()
            .rfind(|id| self.arena.get(*id).is_ok_and(Node::selected));
        if let Some(last) = selected {
            for option in options {
                if option != last && self.option_selected(option)? {
                    self.set_option_selectedness(option, false)?;
                }
            }
        } else {
            let size = self.select_display_size(select)?;
            if size == 1 {
                for option in options {
                    if !self.option_disabled(option)? {
                        self.set_option_selectedness(option, true)?;
                        break;
                    }
                }
            }
        }
        self.update_selected_content(select)
    }

    pub fn select_selected_index(&self, select: NodeId) -> Result<i32, DomError> {
        Ok(self
            .select_options(select)?
            .iter()
            .position(|id| self.arena.get(*id).is_ok_and(Node::selected))
            .map_or(-1, |index| index as i32))
    }

    pub fn set_select_selected_index(
        &mut self,
        select: NodeId,
        index: i32,
    ) -> Result<(), DomError> {
        for (position, option) in self.select_options(select)?.into_iter().enumerate() {
            self.set_option_selectedness(option, position as i32 == index)?;
            if position as i32 == index {
                self.arena.get_mut(option)?.dirty_selectedness = true;
            }
        }
        self.update_selected_content(select)
    }

    pub fn select_value(&self, select: NodeId) -> Result<String, DomError> {
        for option in self.select_options(select)? {
            if self.option_selected(option)? {
                return self.option_value(option);
            }
        }
        Ok(String::new())
    }

    pub fn set_select_value(&mut self, select: NodeId, value: &str) -> Result<(), DomError> {
        let mut found = false;
        for option in self.select_options(select)? {
            let selected = !found && self.option_value(option)? == value;
            self.set_option_selectedness(option, selected)?;
            if selected {
                self.arena.get_mut(option)?.dirty_selectedness = true;
                found = true;
            }
        }
        self.update_selected_content(select)
    }

    pub fn reset_select(&mut self, select: NodeId) -> Result<(), DomError> {
        self.set_user_validity(select, false)?;
        for option in self.select_options(select)? {
            self.reset_option_selectedness(option)?;
        }
        self.normalize_select(select)
    }

    pub fn option_selected(&self, option: NodeId) -> Result<bool, DomError> {
        Ok(self.arena.get(option)?.selected())
    }

    /// Set selectedness without changing dirtiness or the content attribute.
    /// Used by factory initialization, reset and select normalization.
    pub fn set_option_selectedness(
        &mut self,
        option: NodeId,
        selected: bool,
    ) -> Result<(), DomError> {
        self.arena.get_mut(option)?.selectedness = Some(selected);
        self.mutations.push(Mutation::FormState { node: option });
        Ok(())
    }

    /// IDL/user state changes make subsequent defaultSelected changes inert.
    pub fn set_option_selected(&mut self, option: NodeId, selected: bool) -> Result<(), DomError> {
        self.arena.get_mut(option)?.dirty_selectedness = true;
        self.set_option_selectedness(option, selected)?;
        self.option_selection_changed(option)
    }

    pub fn reset_option_selectedness(&mut self, option: NodeId) -> Result<(), DomError> {
        self.arena.get_mut(option)?.dirty_selectedness = false;
        let selected = self
            .attribute_in_null_namespace(option, "selected")?
            .is_some();
        self.set_option_selectedness(option, selected)
    }

    pub(super) fn option_attribute_changed(
        &mut self,
        option: NodeId,
        name: &str,
    ) -> Result<(), DomError> {
        if name == "selected"
            && self.is_element(option, "http://www.w3.org/1999/xhtml", "option")
            && !self.arena.get(option)?.dirty_selectedness
        {
            let selected = self
                .attribute_in_null_namespace(option, "selected")?
                .is_some();
            self.set_option_selectedness(option, selected)?;
            self.option_selection_changed(option)?;
        }
        if matches!(name, "multiple" | "size") && self.is_element(option, HTML, "select") {
            self.normalize_select(option)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append(dom: &mut DomRuntime, parent: NodeId, tag: &str) -> NodeId {
        let node = dom.create_element(tag);
        dom.append_child(parent, node).unwrap();
        node
    }

    #[test]
    fn select_required_value_uses_placeholder_identity_not_empty_value_alone() {
        let mut dom = DomRuntime::new();
        let select = dom.create_element("select");
        assert!(dom.select_will_validate(select).unwrap());
        assert!(!dom.select_value_missing(select).unwrap());
        dom.set_attribute(select, "required", "").unwrap();
        assert!(dom.select_value_missing(select).unwrap());
        let first = append(&mut dom, select, "option");
        assert!(dom.select_value_missing(select).unwrap());
        let second = append(&mut dom, select, "option");
        dom.set_option_selected(second, true).unwrap();
        assert!(!dom.select_value_missing(select).unwrap());
        dom.set_option_selected(first, true).unwrap();
        dom.set_attribute(first, "disabled", "").unwrap();
        assert!(dom.select_value_missing(select).unwrap());
        dom.set_attribute(first, "value", "chosen").unwrap();
        assert!(!dom.select_value_missing(select).unwrap());
        dom.set_attribute(first, "value", "").unwrap();
        dom.set_attribute(select, "size", "2").unwrap();
        assert!(!dom.select_value_missing(select).unwrap());
        dom.set_select_selected_index(select, -1).unwrap();
        assert!(dom.select_value_missing(select).unwrap());
        dom.set_attribute(select, "multiple", "").unwrap();
        dom.remove_attribute(select, "size").unwrap();
        assert_eq!(dom.select_display_size(select).unwrap(), 4);
        dom.set_option_selected(first, true).unwrap();
        assert!(!dom.select_value_missing(select).unwrap());
        dom.set_attribute(select, "readonly", "").unwrap();
        assert!(dom.select_will_validate(select).unwrap());
        dom.set_attribute(select, "disabled", "").unwrap();
        assert!(!dom.select_will_validate(select).unwrap());
        dom.remove_attribute(select, "disabled").unwrap();
        let datalist = dom.create_element("datalist");
        dom.append_child(datalist, select).unwrap();
        assert!(!dom.select_will_validate(select).unwrap());
    }

    #[test]
    fn blank_option_batch_growth_and_interruption() {
        let mut dom = DomRuntime::new();
        let select = dom.create_element("select");
        let started = std::time::Instant::now();
        dom.set_select_length(select, 100_000).unwrap();
        eprintln!("100000 blank options: {:?}", started.elapsed());
        let options = dom.select_options(select).unwrap();
        assert_eq!(options.len(), 100_000);
        assert_eq!(dom.select_selected_index(select).unwrap(), 0);
        assert!(
            options
                .iter()
                .all(|option| dom.parent(*option).unwrap() == Some(select))
        );
        dom.set_select_length(select, 100_001).unwrap();
        assert_eq!(dom.select_options(select).unwrap().len(), 100_000);
        // Indexed growth must not inherit the length setter's upper bound.
        assert!(dom.append_new_options(select, 1, || true).unwrap());
        assert_eq!(dom.select_options(select).unwrap().len(), 100_001);
        let other = dom.create_element("select");
        let mut remaining = 3;
        assert!(
            !dom.append_new_options(other, 10, || {
                remaining -= 1;
                remaining > 0
            })
            .unwrap()
        );
        assert!(dom.select_options(other).unwrap().is_empty());
    }

    #[test]
    fn option_owner_barriers_do_not_reset_an_unrelated_empty_selection() {
        let mut dom = DomRuntime::new();
        let select = dom.create_element("select");
        append(&mut dom, select, "option");
        for tag in ["datalist", "hr", "option"] {
            let barrier = append(&mut dom, select, tag);
            dom.set_select_selected_index(select, -1).unwrap();
            let excluded = append(&mut dom, barrier, "option");
            assert_eq!(dom.option_nearest_select(excluded).unwrap(), None);
            assert_eq!(dom.option_index(excluded).unwrap(), 0);
            assert_eq!(dom.select_selected_index(select).unwrap(), -1);
        }
        let group = append(&mut dom, select, "optgroup");
        let nested = append(&mut dom, group, "optgroup");
        let excluded = append(&mut dom, nested, "option");
        assert_eq!(dom.option_nearest_select(excluded).unwrap(), None);
        assert_eq!(dom.select_selected_index(select).unwrap(), -1);
        dom.append_child(group, excluded).unwrap();
        assert_eq!(dom.option_nearest_select(excluded).unwrap(), Some(select));
        assert_eq!(dom.select_selected_index(select).unwrap(), 0);
    }

    #[test]
    fn option_list_prunes_nested_select_option_datalist_hr_and_optgroups() {
        let mut dom = DomRuntime::new();
        let select = dom.create_element("select");
        let wrapper = append(&mut dom, select, "div");
        let first = append(&mut dom, wrapper, "option");
        append(&mut dom, first, "option");
        for tag in ["select", "datalist", "hr"] {
            let root = append(&mut dom, select, tag);
            append(&mut dom, root, "option");
        }
        let group = append(&mut dom, select, "optgroup");
        let second = append(&mut dom, group, "option");
        let nested = append(&mut dom, group, "optgroup");
        append(&mut dom, nested, "option");
        let third = append(&mut dom, select, "option");
        assert_eq!(
            dom.select_options(select).unwrap(),
            vec![first, second, third]
        );
        dom.insert_before(select, third, Some(wrapper)).unwrap();
        assert_eq!(
            dom.select_options(select).unwrap(),
            vec![third, first, second]
        );
    }

    #[test]
    fn normalization_and_explicit_empty_selection_are_distinct() {
        let mut dom = DomRuntime::new();
        let select = dom.create_element("select");
        let first = append(&mut dom, select, "option");
        let second = append(&mut dom, select, "option");
        dom.set_attribute(first, "disabled", "").unwrap();
        // Disabling an already selected option does not itself deselect it.
        dom.set_select_selected_index(select, -1).unwrap();
        dom.normalize_select(select).unwrap();
        assert_eq!(dom.select_selected_index(select).unwrap(), 1);
        dom.set_select_selected_index(select, -1).unwrap();
        assert_eq!(dom.select_selected_index(select).unwrap(), -1);
        assert_eq!(dom.select_value(select).unwrap(), "");
        dom.set_attribute(select, "size", "2").unwrap();
        dom.normalize_select(select).unwrap();
        assert_eq!(dom.select_selected_index(select).unwrap(), -1);
        dom.set_attribute(first, "selected", "").unwrap();
        dom.set_attribute(second, "selected", "").unwrap();
        dom.normalize_select(select).unwrap();
        assert!(!dom.option_selected(first).unwrap());
        assert!(dom.option_selected(second).unwrap());
        assert!(
            dom.attribute_in_null_namespace(first, "selected")
                .unwrap()
                .is_some()
        );
        dom.set_attribute(select, "multiple", "").unwrap();
        dom.reset_select(select).unwrap();
        assert!(dom.option_selected(first).unwrap() && dom.option_selected(second).unwrap());
        assert!(!dom.arena.get(first).unwrap().dirty_selectedness);
    }

    #[test]
    fn values_select_first_match_and_preserve_defaults_and_utf16() {
        let mut dom = DomRuntime::new();
        let select = dom.create_element("select");
        let first = append(&mut dom, select, "option");
        let second = append(&mut dom, select, "option");
        let text = super::super::dom_string::from_units(&[0xD800]);
        dom.set_text_content(first, &text).unwrap();
        dom.set_attribute(second, "value", &text).unwrap();
        dom.set_attribute(second, "selected", "").unwrap();
        dom.set_select_value(select, &text).unwrap();
        assert_eq!(dom.select_selected_index(select).unwrap(), 0);
        assert_eq!(dom.select_value(select).unwrap(), text);
        assert!(
            dom.attribute_in_null_namespace(first, "selected")
                .unwrap()
                .is_none()
        );
        assert!(
            dom.attribute_in_null_namespace(second, "selected")
                .unwrap()
                .is_some()
        );
        dom.set_select_value(select, "missing").unwrap();
        assert_eq!(dom.select_selected_index(select).unwrap(), -1);
        dom.reset_select(select).unwrap();
        assert_eq!(dom.select_selected_index(select).unwrap(), 1);
    }

    #[test]
    fn option_text_collapses_only_ascii_whitespace_and_skips_scripts() {
        let mut dom = DomRuntime::new();
        let option = dom.create_element("option");
        dom.set_inner_html(
            option,
            " a\t<span> b&nbsp;c </span><script>ignored</script> d ",
        )
        .unwrap();
        assert_eq!(dom.option_text(option).unwrap(), "a b\u{a0}c d");
        dom.set_attribute(option, "value", "").unwrap();
        assert_eq!(dom.option_value(option).unwrap(), "");
        let select = dom.create_element("select");
        let group = append(&mut dom, select, "optgroup");
        dom.set_attribute(group, "disabled", "").unwrap();
        dom.append_child(group, option).unwrap();
        let enabled = append(&mut dom, select, "option");
        dom.normalize_select(select).unwrap();
        assert!(!dom.option_selected(option).unwrap());
        assert!(dom.option_selected(enabled).unwrap());
    }

    #[test]
    fn parser_clone_and_import_preserve_selectedness_and_dirtiness() {
        let mut dom = crate::html::parse("<select><option id=o selected>text</option></select>")
            .into_runtime()
            .unwrap();
        let option = dom.get_element_by_id(dom.document(), "o").unwrap().unwrap();
        assert!(dom.option_selected(option).unwrap());
        dom.remove(option).unwrap();
        dom.set_option_selected(option, false).unwrap();
        for deep in [false, true] {
            let copy = dom.clone_node(option, deep).unwrap();
            assert!(!dom.option_selected(copy).unwrap());
            assert!(dom.arena.get(copy).unwrap().dirty_selectedness);
            dom.set_attribute(copy, "selected", "").unwrap();
            assert!(!dom.option_selected(copy).unwrap());
        }
        let mut other = DomRuntime::new();
        let copy = other.import_subtree(dom.arena(), option).unwrap();
        assert!(!other.option_selected(copy).unwrap());
        assert!(other.arena.get(copy).unwrap().dirty_selectedness);
        other.reset_option_selectedness(copy).unwrap();
        assert!(other.option_selected(copy).unwrap());
        assert!(!dom.option_selected(option).unwrap());
    }

    #[test]
    fn attribute_node_updates_respect_option_dirtiness() {
        let mut dom = DomRuntime::new();
        let option = dom.create_element("option");
        dom.set_attribute(option, "selected", "").unwrap();
        dom.set_option_selectedness(option, false).unwrap();
        let attribute = dom.attribute_node(option, "selected").unwrap().unwrap();
        dom.set_attribute_node_value(attribute, "yes").unwrap();
        assert!(dom.option_selected(option).unwrap());
        dom.set_option_selected(option, false).unwrap();
        dom.set_attribute_node_value(attribute, "again").unwrap();
        assert!(!dom.option_selected(option).unwrap());
    }

    #[test]
    fn select_lifecycle_normalizes_only_relevant_transitions() {
        let mut dom = DomRuntime::new();
        let first = dom.create_element("select");
        dom.retain_native(first).unwrap();
        let second = dom.create_element("select");
        dom.retain_native(second).unwrap();
        let a = append(&mut dom, first, "option");
        let b = append(&mut dom, first, "option");
        assert_eq!(dom.select_selected_index(first).unwrap(), 0);
        dom.set_option_selected(b, true).unwrap();
        assert!(!dom.option_selected(a).unwrap() && dom.option_selected(b).unwrap());
        dom.append_child(second, b).unwrap();
        assert_eq!(dom.select_selected_index(first).unwrap(), 0);
        assert_eq!(dom.select_selected_index(second).unwrap(), 0);
        dom.set_select_selected_index(first, -1).unwrap();
        dom.set_attribute(first, "class", "unrelated").unwrap();
        append(&mut dom, first, "span");
        dom.append_child(first, a).unwrap();
        assert_eq!(dom.select_selected_index(first).unwrap(), -1);
        let c = append(&mut dom, first, "option");
        assert_eq!(dom.select_selected_index(first).unwrap(), 0);
        dom.set_attribute(c, "selected", "").unwrap();
        assert!(!dom.option_selected(a).unwrap() && dom.option_selected(c).unwrap());
        dom.remove(c).unwrap();
        assert!(dom.option_selected(a).unwrap());
        dom.set_attribute(first, "multiple", "").unwrap();
        dom.append_child(first, b).unwrap();
        assert!(dom.option_selected(a).unwrap() && dom.option_selected(b).unwrap());
        dom.remove_attribute(first, "multiple").unwrap();
        assert!(!dom.option_selected(a).unwrap() && dom.option_selected(b).unwrap());
        dom.release_native(first).unwrap();
        dom.release_native(second).unwrap();
        assert!(dom.roots.get(&first).is_none_or(|roots| roots.native == 0));
        assert!(dom.roots.get(&second).is_none_or(|roots| roots.native == 0));
    }

    #[test]
    fn selectedness_and_default_are_independent_until_reset() {
        let mut dom = DomRuntime::new();
        let option = dom.create_element("option");
        assert!(!dom.option_selected(option).unwrap());
        dom.set_attribute(option, "selected", "").unwrap();
        assert!(dom.option_selected(option).unwrap());
        // Option(..., true, false) overrides state but does not make it dirty.
        dom.set_option_selectedness(option, false).unwrap();
        assert!(!dom.option_selected(option).unwrap());
        assert!(
            dom.attribute_in_null_namespace(option, "selected")
                .unwrap()
                .is_some()
        );
        dom.set_attribute(option, "selected", "again").unwrap();
        assert!(dom.option_selected(option).unwrap());
        dom.set_option_selected(option, false).unwrap();
        dom.remove_attribute(option, "selected").unwrap();
        dom.set_attribute(option, "selected", "").unwrap();
        assert!(!dom.option_selected(option).unwrap());
        dom.reset_option_selectedness(option).unwrap();
        assert!(dom.option_selected(option).unwrap());
        dom.remove_attribute(option, "selected").unwrap();
        assert!(!dom.option_selected(option).unwrap());
    }
}

impl DomArena {
    pub fn select_options(&self, select: NodeId) -> Result<Vec<NodeId>, DomError> {
        let record = self.get(select)?;
        if !matches!(&record.kind, NodeKind::Element { name } if name.ns.as_ref() == HTML && name.local.as_ref() == "select")
        {
            return Ok(Vec::new());
        }
        let mut pending: Vec<_> = record
            .children
            .iter()
            .rev()
            .map(|id| (*id, false))
            .collect();
        let mut options = Vec::new();
        while let Some((node, in_optgroup)) = pending.pop() {
            let record = self.get(node)?;
            if matches!(
                record.kind,
                NodeKind::Document | NodeKind::ShadowRoot { .. }
            ) {
                continue;
            }
            let tag = match &record.kind {
                NodeKind::Element { name } if name.ns.as_ref() == HTML => name.local.as_ref(),
                _ => "",
            };
            if tag == "option" {
                options.push(node);
            }
            if matches!(tag, "option" | "select" | "hr" | "datalist")
                || tag == "optgroup" && in_optgroup
            {
                continue;
            }
            pending.extend(
                record
                    .children
                    .iter()
                    .rev()
                    .map(|id| (*id, in_optgroup || tag == "optgroup")),
            );
        }
        Ok(options)
    }
}

#[cfg(test)]
mod grouped_single_insertion_tests {

    #[test]
    fn same_select_move_preserves_explicit_empty_but_new_owner_normalizes() {
        let mut dom=crate::html::parse("<select id=s><option id=a>A</option><option id=b>B</option></select><select id=t></select>").into_runtime().unwrap();
        let s = dom.find_by_id("s").unwrap();
        let t = dom.find_by_id("t").unwrap();
        let a = dom.find_by_id("a").unwrap();
        let b = dom.find_by_id("b").unwrap();
        dom.set_select_selected_index(s, -1).unwrap();
        dom.insert_dom_node(s, a, None).unwrap();
        assert_eq!(dom.select_selected_index(s).unwrap(), -1);
        dom.insert_dom_node(s, a, Some(b)).unwrap();
        assert_eq!(dom.select_selected_index(s).unwrap(), -1);
        dom.insert_dom_node(t, a, None).unwrap();
        assert_eq!(dom.select_selected_index(t).unwrap(), 0);
        assert_eq!(dom.select_selected_index(s).unwrap(), 0);
    }
}
