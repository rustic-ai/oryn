//! Shared text-control editing and constraint state. Script assignments and
//! user editing deliberately take different paths; sensitivity is independent.
use super::{DomError, DomRuntime, NodeId, dom_string};

impl DomRuntime {
    /// HTML input state uses Text for missing, empty, or unknown keywords.
    /// Keep the authored attribute intact; native consumers use this state.
    pub fn effective_input_type(&self, node: NodeId) -> Result<String, DomError> {
        let kind = self
            .attribute_in_null_namespace(node, "type")?
            .unwrap_or("text")
            .to_ascii_lowercase();
        Ok(
            if matches!(
                kind.as_str(),
                "hidden"
                    | "text"
                    | "search"
                    | "tel"
                    | "url"
                    | "email"
                    | "password"
                    | "date"
                    | "month"
                    | "week"
                    | "time"
                    | "datetime-local"
                    | "number"
                    | "range"
                    | "color"
                    | "checkbox"
                    | "radio"
                    | "file"
                    | "submit"
                    | "image"
                    | "reset"
                    | "button"
            ) {
                kind
            } else {
                "text".into()
            },
        )
    }

    /// Public variable-length selection excludes Email and picker states.
    pub fn supports_selection_range(&self, node: NodeId) -> Result<bool, DomError> {
        Ok(
            self.is_element(node, "http://www.w3.org/1999/xhtml", "textarea")
                || (self.is_element(node, "http://www.w3.org/1999/xhtml", "input")
                    && matches!(
                        self.effective_input_type(node)?.as_str(),
                        "text" | "search" | "tel" | "url" | "password"
                    )),
        )
    }
    /// select() also applies to textual picker controls, and otherwise does nothing.
    pub fn supports_select_all(&self, node: NodeId) -> Result<bool, DomError> {
        Ok(
            self.is_element(node, "http://www.w3.org/1999/xhtml", "textarea")
                || (self.is_element(node, "http://www.w3.org/1999/xhtml", "input")
                    && !matches!(
                        self.effective_input_type(node)?.as_str(),
                        "hidden"
                            | "range"
                            | "checkbox"
                            | "radio"
                            | "submit"
                            | "image"
                            | "reset"
                            | "button"
                    )),
        )
    }

    pub fn textarea_default_value(&self, node: NodeId) -> Result<String, DomError> {
        let mut units = Vec::new();
        for child in &self.arena.get(node)?.children {
            if let super::NodeKind::Text { data } | super::NodeKind::CDataSection { data } =
                &self.arena.get(*child)?.kind
            {
                units.extend(dom_string::units(data));
            }
        }
        Ok(dom_string::from_units(&units))
    }
    pub fn is_text_control(&self, node: NodeId) -> bool {
        const HTML: &str = "http://www.w3.org/1999/xhtml";
        if self.is_element(node, HTML, "textarea") {
            return true;
        }
        if !self.is_element(node, HTML, "input") {
            return false;
        }
        let kind = self
            .effective_input_type(node)
            .unwrap_or_else(|_| "text".into());
        !matches!(
            kind.as_str(),
            "hidden"
                | "date"
                | "month"
                | "week"
                | "time"
                | "datetime-local"
                | "number"
                | "range"
                | "color"
                | "checkbox"
                | "radio"
                | "file"
                | "submit"
                | "image"
                | "reset"
                | "button"
        )
    }

    /// Native editing includes number controls, whose public selection API is
    /// still excluded by is_text_control.
    pub fn is_editable_text_control(&self, node: NodeId) -> bool {
        self.is_temporal_control(node)
            || self.is_text_control(node)
            || (self.is_element(node, "http://www.w3.org/1999/xhtml", "input")
                && self
                    .attribute_in_null_namespace(node, "type")
                    .ok()
                    .flatten()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("number")))
    }
    pub fn control_edit_value(&self, node: NodeId) -> Result<String, DomError> {
        if self.is_temporal_control(node) {
            return self.temporal_display_value(node);
        }
        let record = self.arena.get(node)?;
        if self.is_editable_text_control(node)
            && !self.is_text_control(node)
            && record.user_edited_value
            && let Some(value) = &record.value
        {
            return Ok(value.clone());
        }
        self.control_value(node)
    }
    pub fn control_bad_input(&self, node: NodeId) -> Result<bool, DomError> {
        let record = self.arena.get(node)?;
        if !record.user_edited_value
            || !self.is_element(node, "http://www.w3.org/1999/xhtml", "input")
        {
            return Ok(false);
        }
        let kind = self
            .attribute_in_null_namespace(node, "type")?
            .unwrap_or("text");
        Ok(super::numeric_controls::numeric_kind(kind)
            && record.value.as_ref().is_some_and(|value| !value.is_empty())
            && self.control_value(node)?.is_empty())
    }

    pub fn control_length_limit(
        &self,
        node: NodeId,
        attribute: &str,
    ) -> Result<Option<usize>, DomError> {
        let Some(raw) = self.attribute_in_null_namespace(node, attribute)? else {
            return Ok(None);
        };
        let raw = raw.trim_start_matches(['\t', '\n', '\x0c', '\r', ' ']);
        let raw = raw.strip_prefix('+').unwrap_or(raw);
        let digits: String = raw.chars().take_while(char::is_ascii_digit).collect();
        Ok(digits
            .parse::<u32>()
            .ok()
            .filter(|n| *n <= i32::MAX as u32)
            .map(|n| n as usize))
    }

    /// (tooShort, tooLong), measured in UTF-16 units, not Unicode scalars.
    pub fn control_length_validity(&self, node: NodeId) -> Result<(bool, bool), DomError> {
        let record = self.arena.get(node)?;
        if !self.is_text_control(node) || !record.dirty_value || !record.user_edited_value {
            return Ok((false, false));
        }
        let length = dom_string::len(&self.control_value(node)?);
        Ok((
            length != 0
                && self
                    .control_length_limit(node, "minlength")?
                    .is_some_and(|min| length < min),
            self.control_length_limit(node, "maxlength")?
                .is_some_and(|max| length > max),
        ))
    }

    /// Change native disclosure metadata without resetting the value, edit
    /// history, dirty flag, or the pending user change transaction.
    pub fn set_control_sensitive(&mut self, node: NodeId, sensitive: bool) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        if record.sensitive_value != sensitive {
            record.sensitive_value = sensitive;
            self.record_mutation(crate::Mutation::FormState { node });
        }
        Ok(())
    }

    pub fn set_user_control_value(
        &mut self,
        node: NodeId,
        value: &str,
        sensitive: bool,
    ) -> Result<(), DomError> {
        let mut units = self.normalized_control_edit_units(node, value);
        if self.is_text_control(node)
            && let Some(max) = self.control_length_limit(node, "maxlength")?
        {
            truncate_at_scalar_boundary(&mut units, max);
        }
        self.set_value(node, dom_string::from_units(&units), sensitive)?;
        self.arena.get_mut(node)?.user_edited_value = true;
        self.set_user_validity(node, true)?;
        // The OIL replacement action commits its own change event.
        self.arena.get_mut(node)?.pending_edit_value = None;
        Ok(())
    }

    /// Replace an actual user-edit interval, preserving opaque ranges and
    /// returning the caret position after maxlength truncation.
    pub fn edit_control_value(
        &mut self,
        node: NodeId,
        start: u32,
        end: u32,
        replacement: &str,
    ) -> Result<u32, DomError> {
        if self.is_temporal_control(node) {
            return self.replace_temporal_edit(node, start, end, replacement);
        }
        let current = self.control_edit_value(node)?;
        let length = dom_string::len(&current);
        if start > end || end as usize > length {
            return Err(super::OpaqueRangeError::InvalidOffset.into());
        }
        let mut units = self.normalized_control_edit_units(node, replacement);
        if self.is_text_control(node)
            && let Some(max) = self.control_length_limit(node, "maxlength")?
        {
            truncate_at_scalar_boundary(
                &mut units,
                max.saturating_sub(length - (end - start) as usize),
            );
        }
        if start == end && units.is_empty() {
            return Ok(start);
        }
        let caret = start + units.len() as u32;
        if self.is_editable_text_control(node) && !self.is_text_control(node) {
            let mut value = dom_string::units(&current);
            value.splice(start as usize..end as usize, units);
            let sensitive = self.arena.get(node)?.sensitive_value;
            self.set_value(node, dom_string::from_units(&value), sensitive)?;
        } else {
            self.replace_control_value(node, start, end, &dom_string::from_units(&units))?;
        }
        self.arena.get_mut(node)?.user_edited_value = true;
        if self.control_edit_value(node)? != current
            && self.arena.get(node)?.pending_edit_value.is_none()
        {
            self.arena.get_mut(node)?.pending_edit_value = Some(current);
        }
        Ok(caret)
    }

    pub fn take_control_change(&mut self, node: NodeId) -> Result<bool, DomError> {
        let previous = self.arena.get_mut(node)?.pending_edit_value.take();
        if previous.is_some() {
            self.set_user_validity(node, true)?;
        }
        match previous {
            Some(previous) => Ok(self.control_edit_value(node)? != previous),
            None => Ok(false),
        }
    }

    pub(super) fn normalized_control_edit_units(&self, node: NodeId, value: &str) -> Vec<u16> {
        let units = dom_string::units(value);
        if self.is_element(node, "http://www.w3.org/1999/xhtml", "textarea") {
            let mut result = Vec::with_capacity(units.len());
            let mut previous_cr = false;
            for unit in units {
                if unit != 10 || !previous_cr {
                    result.push(if unit == 13 { 10 } else { unit });
                }
                previous_cr = unit == 13;
            }
            result
        } else if self.is_text_control(node) {
            units
                .into_iter()
                .filter(|unit| !matches!(unit, 10 | 13))
                .collect()
        } else {
            units
        }
    }
}

fn truncate_at_scalar_boundary(units: &mut Vec<u16>, max: usize) {
    if units.len() <= max {
        return;
    }
    let max = if max > 0
        && (0xd800..=0xdbff).contains(&units[max - 1])
        && (0xdc00..=0xdfff).contains(&units[max])
    {
        max - 1
    } else {
        max
    };
    units.truncate(max);
}

#[cfg(test)]
mod tests {
    #[test]
    fn editing_provenance_defaults_and_utf16_limits_are_native() {
        let mut dom =
            crate::html::parse("<textarea id=t minlength=5 maxlength=10>default</textarea>")
                .into_runtime()
                .unwrap();
        let t = dom.find_by_id("t").unwrap();
        dom.set_text_content(t, "abc").unwrap();
        assert_eq!(dom.control_value(t).unwrap(), "abc");
        assert_eq!(dom.control_length_validity(t).unwrap(), (false, false));
        dom.set_user_control_value(t, "abc", false).unwrap();
        assert_eq!(dom.control_length_validity(t).unwrap(), (true, false));
        dom.set_text_content(t, "changed default").unwrap();
        assert_eq!(dom.control_value(t).unwrap(), "abc");
        assert_eq!(dom.control_length_validity(t).unwrap(), (true, false));
        dom.set_value(t, "abc", false).unwrap();
        assert_eq!(dom.control_length_validity(t).unwrap(), (false, false));
        dom.set_user_control_value(t, "👨‍👩‍👧‍👦", false).unwrap();
        assert_eq!(dom.control_value(t).unwrap(), "👨‍👩‍👧‍");
        dom.set_attribute(t, "maxlength", "2").unwrap();
        assert_eq!(dom.control_length_validity(t).unwrap(), (false, true));
        dom.reset_control_value(t, "changed default".into())
            .unwrap();
        dom.set_text_content(t, "fresh").unwrap();
        assert_eq!(dom.control_value(t).unwrap(), "fresh");
        assert_eq!(dom.control_length_validity(t).unwrap(), (false, false));
        let nested = dom.create_element("span");
        dom.set_text_content(nested, "ignored nested content")
            .unwrap();
        dom.append_child(t, nested).unwrap();
        assert_eq!(dom.textarea_default_value(t).unwrap(), "fresh");
        assert_eq!(dom.control_value(t).unwrap(), "fresh");
        dom.set_attribute(t, "maxlength", "4").unwrap();
        dom.set_user_control_value(t, "a\r\nb\rc", false).unwrap();
        assert_eq!(dom.control_value(t).unwrap(), "a\nb\n");
        dom.set_attribute(t, "maxlength", "0").unwrap();
        dom.set_user_control_value(t, "x", false).unwrap();
        assert_eq!(dom.control_value(t).unwrap(), "");
        assert_eq!(dom.control_length_validity(t).unwrap(), (false, false));
        for raw in ["-1", "bad", "2147483648"] {
            dom.set_attribute(t, "maxlength", raw).unwrap();
            assert_eq!(dom.control_length_limit(t, "maxlength").unwrap(), None);
        }
        dom.set_attribute(t, "maxlength", " +5rest").unwrap();
        assert_eq!(dom.control_length_limit(t, "maxlength").unwrap(), Some(5));
        dom.set_value(t, "abcd", false).unwrap();
        assert_eq!(dom.edit_control_value(t, 1, 3, "😀😀").unwrap(), 3);
        assert_eq!(dom.control_value(t).unwrap(), "a😀d");
    }
}

#[cfg(test)]
mod numeric_user_edit_regressions {

    #[test]
    fn native_number_edit_retains_incomplete_text_without_exposing_it_as_value() {
        let mut dom = crate::html::parse("<input id=n type=number placeholder=Number>")
            .into_runtime()
            .unwrap();
        let node = dom.find_by_id("n").unwrap();
        assert_eq!(dom.edit_control_value(node, 0, 0, "1").unwrap(), 1);
        assert_eq!(dom.edit_control_value(node, 1, 1, "e").unwrap(), 2);
        assert_eq!(dom.control_edit_value(node).unwrap(), "1e");
        assert_eq!(dom.control_value(node).unwrap(), "");
        assert!(dom.control_bad_input(node).unwrap());
        assert!(!dom.control_validity(node).unwrap());
        assert!(!dom.element_placeholder_shown(node).unwrap());
        let clone = dom.clone_node(node, false).unwrap();
        assert_eq!(dom.control_value(clone).unwrap(), "");
        assert!(!dom.control_bad_input(clone).unwrap());
        dom.edit_control_value(node, 2, 2, "2").unwrap();
        assert_eq!(dom.control_value(node).unwrap(), "1e2");
        assert!(!dom.control_bad_input(node).unwrap());
        dom.set_value(node, "invalid", false).unwrap();
        assert_eq!(dom.control_value(node).unwrap(), "");
        assert!(
            !dom.control_bad_input(node).unwrap(),
            "script sanitization is not failed user input"
        );
        dom.set_user_control_value(node, "-", false).unwrap();
        assert!(dom.control_bad_input(node).unwrap());
        dom.reset_control_value(node, String::new()).unwrap();
        assert!(!dom.control_bad_input(node).unwrap());
        assert_eq!(dom.control_edit_value(node).unwrap(), "");
    }
}
