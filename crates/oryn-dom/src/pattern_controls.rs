//! HTML pattern constraints use the agent's ECMAScript engine, not a second
//! regex dialect. The service is native, thread-affine and cannot call page JS.
use super::{DomError, DomRuntime, NodeId};
use std::{fmt::Debug, rc::Rc};

pub trait PatternMatcher: Debug {
    /// None means syntactically invalid (HTML ignores that pattern).
    /// An engine/resource failure is an error, never a fabricated valid result.
    fn matches(&self, pattern: &str, value: &str) -> Result<Option<bool>, DomError>;
}

impl DomRuntime {
    pub fn set_pattern_matcher(&mut self, matcher: Rc<dyn PatternMatcher>) {
        self.pattern_matcher = Some(matcher);
    }

    pub fn control_pattern_mismatch(&self, node: NodeId) -> Result<bool, DomError> {
        if !self.is_element(node, "http://www.w3.org/1999/xhtml", "input") {
            return Ok(false);
        }
        let kind = self
            .attribute_in_null_namespace(node, "type")?
            .unwrap_or("text")
            .to_ascii_lowercase();
        // Missing and unknown types use the Text state.
        if matches!(
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
        ) {
            return Ok(false);
        }
        let Some(pattern) = self.attribute_in_null_namespace(node, "pattern")? else {
            return Ok(false);
        };
        let value = self.control_value(node)?;
        if value.is_empty() {
            return Ok(false);
        }
        let matcher = self
            .pattern_matcher
            .as_ref()
            .ok_or(DomError::PatternEvaluation("engine unavailable"))?;
        if kind == "email"
            && self
                .attribute_in_null_namespace(node, "multiple")?
                .is_some()
        {
            for (index, token) in value.split(',').enumerate() {
                if index >= 1024 {
                    return Err(DomError::PatternEvaluation("value count limit"));
                }
                match matcher
                    .matches(pattern, token.trim_matches(['\t', '\n', '\x0c', '\r', ' ']))?
                {
                    None => return Ok(false),
                    Some(false) => return Ok(true),
                    Some(true) => {}
                }
            }
            Ok(false)
        } else {
            Ok(matcher.matches(pattern, &value)? == Some(false))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Debug)]
    struct LiteralMatcher;
    impl PatternMatcher for LiteralMatcher {
        fn matches(&self, pattern: &str, value: &str) -> Result<Option<bool>, DomError> {
            Ok((pattern != "invalid").then_some(pattern == value))
        }
    }
    #[test]
    fn constraints_are_live_and_share_eligibility_and_group_rules() {
        let mut dom = crate::html::parse("<form id=f><input id=n pattern=ABC value=ABC></form>")
            .into_runtime()
            .unwrap();
        dom.set_pattern_matcher(Rc::new(LiteralMatcher));
        let n = dom.find_by_id("n").unwrap();
        let f = dom.find_by_id("f").unwrap();
        assert!(!dom.control_pattern_mismatch(n).unwrap());
        dom.set_value(n, "AB", false).unwrap();
        assert!(dom.control_pattern_mismatch(n).unwrap());
        assert!(!dom.control_validity(n).unwrap());
        assert_eq!(dom.validation_selector_state(f).unwrap(), Some(false));
        dom.set_attribute(n, "disabled", "").unwrap();
        assert!(dom.control_pattern_mismatch(n).unwrap());
        assert_eq!(dom.validation_selector_state(f).unwrap(), Some(true));
        dom.set_attribute(n, "type", "unknown").unwrap();
        assert!(dom.control_pattern_mismatch(n).unwrap());
        dom.set_attribute(n, "type", "number").unwrap();
        assert!(!dom.control_pattern_mismatch(n).unwrap());
        dom.set_attribute(n, "type", "email").unwrap();
        dom.set_attribute(n, "multiple", "").unwrap();
        dom.set_value(n, "ABC, ABC", false).unwrap();
        assert!(!dom.control_pattern_mismatch(n).unwrap());
        dom.set_value(n, "ABC, AB", false).unwrap();
        assert!(dom.control_pattern_mismatch(n).unwrap());
        dom.set_attribute(n, "pattern", "invalid").unwrap();
        assert!(!dom.control_pattern_mismatch(n).unwrap());
        dom.set_attribute(n, "pattern", "").unwrap();
        assert!(dom.control_pattern_mismatch(n).unwrap());
        dom.set_value(n, "", false).unwrap();
        assert!(!dom.control_pattern_mismatch(n).unwrap());
    }
}
