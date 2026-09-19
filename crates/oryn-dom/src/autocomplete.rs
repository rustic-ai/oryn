//! Shared autofill token interpretation for IDL reflection and form history.
use crate::{DomError, DomRuntime, NodeId};

fn contact(field: &str) -> bool {
    matches!(
        field,
        "tel"
            | "tel-country-code"
            | "tel-national"
            | "tel-area-code"
            | "tel-local"
            | "tel-local-prefix"
            | "tel-local-suffix"
            | "tel-extension"
            | "email"
            | "impp"
    )
}
fn normal(field: &str) -> bool {
    matches!(
        field,
        "name"
            | "honorific-prefix"
            | "given-name"
            | "additional-name"
            | "family-name"
            | "honorific-suffix"
            | "nickname"
            | "username"
            | "new-password"
            | "current-password"
            | "one-time-code"
            | "organization-title"
            | "organization"
            | "street-address"
            | "address-line1"
            | "address-line2"
            | "address-line3"
            | "address-level4"
            | "address-level3"
            | "address-level2"
            | "address-level1"
            | "country"
            | "country-name"
            | "postal-code"
            | "cc-name"
            | "cc-given-name"
            | "cc-additional-name"
            | "cc-family-name"
            | "cc-number"
            | "cc-exp"
            | "cc-exp-month"
            | "cc-exp-year"
            | "cc-csc"
            | "cc-type"
            | "transaction-currency"
            | "transaction-amount"
            | "language"
            | "bday"
            | "bday-day"
            | "bday-month"
            | "bday-year"
            | "sex"
            | "url"
            | "photo"
    )
}

fn tokens(value: &str, anchor: bool) -> String {
    let words = value
        .split_ascii_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let Some(last) = words.last() else {
        return String::new();
    };
    if matches!(last.as_str(), "on" | "off") {
        return if words.len() == 1 && !anchor {
            last.clone()
        } else {
            String::new()
        };
    }
    let mut end = words.len();
    if last == "webauthn" {
        end -= 1;
        if end == 0 {
            return "webauthn".into();
        }
    }
    let field = &words[end - 1];
    end -= 1;
    let is_contact = contact(field);
    if !is_contact && !normal(field) {
        return String::new();
    }
    if is_contact
        && end > 0
        && matches!(
            words[end - 1].as_str(),
            "home" | "work" | "mobile" | "fax" | "pager"
        )
    {
        end -= 1;
    }
    if end > 0 && matches!(words[end - 1].as_str(), "shipping" | "billing") {
        end -= 1;
    }
    if end > 0 && words[end - 1].starts_with("section-") {
        end -= 1;
    }
    if end != 0 {
        return String::new();
    }
    words.join(" ")
}

impl DomRuntime {
    pub fn autocomplete_value(&self, node: NodeId) -> Result<String, DomError> {
        let value = self
            .attribute_in_null_namespace(node, "autocomplete")?
            .unwrap_or("");
        if self.is_element(node, "http://www.w3.org/1999/xhtml", "form") {
            return Ok(if value.eq_ignore_ascii_case("off") {
                "off"
            } else {
                "on"
            }
            .into());
        }
        let anchor = self.is_element(node, "http://www.w3.org/1999/xhtml", "input")
            && self.effective_input_type(node)? == "hidden";
        Ok(tokens(value, anchor))
    }
    pub fn control_autofill_enabled(&self, node: NodeId) -> Result<bool, DomError> {
        let value = self.autocomplete_value(node)?;
        if !value.is_empty() {
            return Ok(value != "off");
        }
        match self.form_owner(node)? {
            Some(form) => Ok(self.autocomplete_value(form)? != "off"),
            None => Ok(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_pinned_field_keyword_normalizes_and_contact_prefixes_are_restricted() {
        let fields = "name honorific-prefix given-name additional-name family-name honorific-suffix nickname username new-password current-password one-time-code organization-title organization street-address address-line1 address-line2 address-line3 address-level4 address-level3 address-level2 address-level1 country country-name postal-code cc-name cc-given-name cc-additional-name cc-family-name cc-number cc-exp cc-exp-month cc-exp-year cc-csc cc-type transaction-currency transaction-amount language bday bday-day bday-month bday-year sex url photo tel tel-country-code tel-national tel-area-code tel-local tel-local-prefix tel-local-suffix tel-extension email impp";
        for field in fields.split_whitespace() {
            assert_eq!(
                tokens(&format!(" \t{}\r\n", field.to_ascii_uppercase()), false),
                field
            );
            let combined = format!("section-test billing {field} webauthn");
            assert_eq!(tokens(&combined, false), combined);
            assert_eq!(tokens(&combined, true), combined);
        }
        for mode in ["home", "work", "mobile", "fax", "pager"] {
            for field in [
                "tel",
                "tel-country-code",
                "tel-national",
                "tel-area-code",
                "tel-local",
                "tel-local-prefix",
                "tel-local-suffix",
                "tel-extension",
                "email",
                "impp",
            ] {
                let combined = format!("section-x shipping {mode} {field} webauthn");
                assert_eq!(tokens(&combined, false), combined);
            }
            assert_eq!(tokens(&format!("{mode} name"), false), "");
        }
        for invalid in [
            "name name",
            "on webauthn",
            "off webauthn",
            "webauthn username",
            "shipping billing email",
            "work home email",
            "section-x section-y name",
            "email\u{b}",
            "email\u{a0}",
        ] {
            assert_eq!(tokens(invalid, false), "", "{invalid}");
        }
    }
    #[test]
    fn token_categories_order_and_hidden_anchor() {
        for (source, expected) in [
            (" HOME\ntel", "home tel"),
            ("shipping   country", "shipping country"),
            ("billing work email", "billing work email"),
            ("section-FOO bday", "section-foo bday"),
            ("username webauthn", "username webauthn"),
            (
                "section-LOGIN shipping work tel webauthn",
                "section-login shipping work tel webauthn",
            ),
            ("webauthn", "webauthn"),
            ("\tOFF ", "off"),
            ("foo off", ""),
            ("foo section-bar billing work tel webauthn", ""),
            ("home name", ""),
            ("billing section-x name", ""),
            ("call-sign", ""),
            ("\u{a0}name", ""),
            ("   ", ""),
        ] {
            assert_eq!(tokens(source, false), expected, "{source}");
        }
        assert_eq!(tokens("on", true), "");
        assert_eq!(tokens("off", true), "");
        assert_eq!(tokens("section-x username", true), "section-x username");
    }
    #[test]
    fn missing_invalid_and_explicit_control_values_follow_form_defaults() {
        let mut dom = crate::html::parse("<form id=f autocomplete=off><input id=x></form>")
            .into_runtime()
            .unwrap();
        let input = dom.find_by_id("x").unwrap();
        let form = dom.find_by_id("f").unwrap();
        for value in ["", "unknown", "home name"] {
            dom.set_attribute(input, "autocomplete", value).unwrap();
            assert_eq!(dom.autocomplete_value(input).unwrap(), "");
            assert!(!dom.control_autofill_enabled(input).unwrap());
        }
        for value in ["on", "email", "section-a billing work tel webauthn"] {
            dom.set_attribute(input, "autocomplete", value).unwrap();
            assert!(dom.control_autofill_enabled(input).unwrap());
        }
        dom.set_attribute(form, "autocomplete", " on ").unwrap();
        assert_eq!(dom.autocomplete_value(form).unwrap(), "on");
        dom.set_attribute(input, "autocomplete", " OFF ").unwrap();
        assert_eq!(dom.autocomplete_value(input).unwrap(), "off");
        assert!(!dom.control_autofill_enabled(input).unwrap());
    }
}
