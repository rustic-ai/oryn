//! Native module request types. Type is part of module identity, not a filename hint.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleType {
    #[default]
    JavaScript,
    Json,
    Css,
    Text,
}

impl ModuleType {
    /// HTML fetch-destination-from-module-type; initiator remains script.
    pub(crate) fn fetch_destination(self) -> &'static str {
        match self {
            Self::JavaScript => "script",
            Self::Css => "style",
            Self::Json => "json",
            Self::Text => "text",
        }
    }
    pub(crate) fn csp_destination(self) -> crate::csp::CspDestination {
        match self {
            Self::JavaScript => crate::csp::CspDestination::Script,
            Self::Css => crate::csp::CspDestination::Style,
            Self::Json | Self::Text => crate::csp::CspDestination::Connect,
        }
    }

    pub fn from_attributes<'a>(
        attributes: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, String> {
        let mut module_type = None;
        for (key, value) in attributes {
            if key != "type" {
                return Err(format!("unsupported import attribute {key:?}"));
            }
            if module_type.is_some() {
                return Err("duplicate import type attribute".into());
            }
            module_type = Some(match value {
                "json" => Self::Json,
                "css" => Self::Css,
                "text" => Self::Text,
                _ => return Err(format!("unsupported module type {value:?}")),
            });
        }
        Ok(module_type.unwrap_or_default())
    }

    pub fn mime_allowed(self, headers: &[(String, String)]) -> bool {
        // Text modules deliberately accept any representation MIME type.
        if self == Self::Text {
            return true;
        }
        if self == Self::JavaScript {
            return oryn_net::script_response_mime_allowed(headers, true);
        }
        oryn_common::mime_type::MimeType::from_headers(headers).is_some_and(|mime| {
            let essence = mime.essence();
            match self {
                Self::Json => {
                    essence == "application/json"
                        || essence == "text/json"
                        || essence.ends_with("+json")
                }
                Self::Css => essence == "text/css",
                Self::JavaScript | Self::Text => unreachable!(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_modules_accept_representations_without_mime_or_extension_authority() {
        assert_eq!(
            ModuleType::from_attributes([("type", "text")]).unwrap(),
            ModuleType::Text
        );
        assert!(ModuleType::Text.mime_allowed(&[]));
        for mime in [
            "text/plain",
            "application/json",
            "text/javascript",
            "image/png",
        ] {
            assert!(ModuleType::Text.mime_allowed(&[("content-type".into(), mime.into())]));
        }
        assert!(ModuleType::from_attributes([("type", "Text")]).is_err());
    }

    #[test]
    fn import_type_is_exact_and_unknown_attributes_reject() {
        assert_eq!(
            ModuleType::from_attributes([]).unwrap(),
            ModuleType::JavaScript
        );
        assert_eq!(
            ModuleType::from_attributes([("type", "json")]).unwrap(),
            ModuleType::Json
        );
        assert_eq!(
            ModuleType::from_attributes([("type", "css")]).unwrap(),
            ModuleType::Css
        );
        for attributes in [
            vec![("type", "javascript")],
            vec![("type", "JSON")],
            vec![("type", " json")],
            vec![("Type", "json")],
            vec![("unknown", "x")],
            vec![("type", "json"), ("type", "json")],
        ] {
            assert!(ModuleType::from_attributes(attributes).is_err());
        }
    }

    #[test]
    fn module_validation_uses_extracted_mime_and_ignores_wildcard_tail() {
        for (kind, valid) in [
            (ModuleType::JavaScript, "text/javascript"),
            (ModuleType::Json, "application/json"),
            (ModuleType::Css, "text/css"),
        ] {
            let headers = vec![
                ("content-type".into(), "text/plain".into()),
                ("content-type".into(), valid.into()),
                ("content-type".into(), "*/*".into()),
            ];
            assert!(kind.mime_allowed(&headers));
            let headers = vec![
                ("content-type".into(), valid.into()),
                ("content-type".into(), "text/plain".into()),
            ];
            assert!(!kind.mime_allowed(&headers));
        }
    }

    #[test]
    fn module_mime_is_strict_and_independent_of_suffix_or_nosniff() {
        for (mime, json, css) in [
            ("application/json", true, false),
            ("TEXT/JSON; charset=windows-1252", true, false),
            ("application/manifest+json", true, false),
            ("text/css", false, true),
            ("text/javascript", false, false),
            ("text/plain", false, false),
        ] {
            let headers = vec![("Content-Type".into(), mime.into())];
            assert_eq!(ModuleType::Json.mime_allowed(&headers), json, "{mime}");
            assert_eq!(ModuleType::Css.mime_allowed(&headers), css, "{mime}");
        }
        assert!(!ModuleType::Json.mime_allowed(&[]));
        assert!(!ModuleType::Css.mime_allowed(&[]));
    }
}
