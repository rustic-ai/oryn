//! Shared text-control editing and constraint state. Script assignments and
//! user editing deliberately take different paths; sensitivity is independent.
use super::{DomError, DomRuntime, NodeId, dom_string};

impl DomRuntime {
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
            .attribute_in_null_namespace(node, "type")
            .ok()
            .flatten()
            .unwrap_or("text")
            .to_ascii_lowercase();
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
        let current = self.control_value(node)?;
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
        self.replace_control_value(node, start, end, &dom_string::from_units(&units))?;
        self.arena.get_mut(node)?.user_edited_value = true;
        if self.control_value(node)? != current
            && self.arena.get(node)?.pending_edit_value.is_none()
        {
            self.arena.get_mut(node)?.pending_edit_value = Some(current);
        }
        Ok(caret)
    }

    pub fn take_control_change(&mut self, node: NodeId) -> Result<bool, DomError> {
        let previous = self.arena.get_mut(node)?.pending_edit_value.take();
        match previous {
            Some(previous) => Ok(self.control_value(node)? != previous),
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
