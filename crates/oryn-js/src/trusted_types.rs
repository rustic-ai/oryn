//! Engine-neutral Trusted Types brands and policy-name enforcement.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustedKind {
    Html,
    Script,
    ScriptUrl,
}

impl TrustedKind {
    pub const fn interface_name(self) -> &'static str {
        match self {
            Self::Html => "TrustedHTML",
            Self::Script => "TrustedScript",
            Self::ScriptUrl => "TrustedScriptURL",
        }
    }
}

/// Native attribute lookup uses already-normalized local names; namespace-aware
/// DOM writes must not lowercase XML/SVG attribute names.
pub fn attribute_type(
    tag: &str,
    attribute: &str,
    namespace: &str,
    attribute_namespace: Option<&str>,
) -> Option<TrustedKind> {
    const HTML: &str = "http://www.w3.org/1999/xhtml";
    const SVG: &str = "http://www.w3.org/2000/svg";
    const XLINK: &str = "http://www.w3.org/1999/xlink";
    let attribute_namespace = attribute_namespace.filter(|value| !value.is_empty());
    if namespace == SVG
        && tag == "script"
        && attribute == "href"
        && (attribute_namespace.is_none() || attribute_namespace == Some(XLINK))
    {
        return Some(TrustedKind::ScriptUrl);
    }
    if attribute_namespace.is_some() || !matches!(namespace, HTML | SVG) {
        return None;
    }
    match (namespace, tag, attribute) {
        (HTML, "script", "src") => Some(TrustedKind::ScriptUrl),
        (HTML, "iframe", "srcdoc") => Some(TrustedKind::Html),
        _ if oryn_dom::event_handlers::is_content_attribute(attribute) => Some(TrustedKind::Script),
        _ => None,
    }
}

/// Property sinks are distinct from content-attribute sinks. In particular,
/// assigning a string to an event-handler property does not compile that string.
pub fn property_type(tag: &str, property: &str, namespace: Option<&str>) -> Option<TrustedKind> {
    let namespace = namespace
        .filter(|namespace| !namespace.is_empty())
        .unwrap_or("http://www.w3.org/1999/xhtml");
    if !matches!(
        namespace,
        "http://www.w3.org/1999/xhtml" | "http://www.w3.org/2000/svg"
    ) {
        return None;
    }
    if matches!(property, "innerHTML" | "outerHTML") {
        return Some(TrustedKind::Html);
    }
    if namespace != "http://www.w3.org/1999/xhtml" {
        return None;
    }
    match (tag.to_ascii_lowercase().as_str(), property) {
        ("script", "src") => Some(TrustedKind::ScriptUrl),
        ("script", "text" | "textContent" | "innerText") => Some(TrustedKind::Script),
        ("iframe", "srcdoc") => Some(TrustedKind::Html),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedValue {
    kind: TrustedKind,
    value: String,
    policy_name: String,
}

impl TrustedValue {
    pub fn new(kind: TrustedKind, value: String, policy_name: String) -> Self {
        Self {
            kind,
            value,
            policy_name,
        }
    }

    pub const fn kind(&self) -> TrustedKind {
        self.kind
    }
    pub fn value(&self) -> &str {
        &self.value
    }
    pub fn policy_name(&self) -> &str {
        &self.policy_name
    }
}

/// Native host checks preserve full CSP policy/report provenance. Sink failure
/// checks run only after default-policy processing has produced no value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicyCheck {
    SinkRequired,
    Policy {
        name: String,
        already_created: bool,
        #[serde(default)]
        location: SourceLocation,
    },
    Sink {
        sink: String,
        input: String,
        #[serde(default)]
        location: SourceLocation,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub source_file: String,
    pub line: u32,
    pub column: u32,
}

impl PolicyCheck {
    pub fn payload_bytes(&self) -> usize {
        match self {
            Self::SinkRequired => 0,
            Self::Policy { name, location, .. } => {
                name.len().saturating_add(location.source_file.len())
            }
            Self::Sink {
                sink,
                input,
                location,
            } => sink
                .len()
                .saturating_add(input.len())
                .saturating_add(location.source_file.len()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedTypesConfig {
    pub require_script: bool,
    pub allowed_policy_names: BTreeSet<String>,
    pub allow_any_policy: bool,
    pub allow_duplicates: bool,
    pub deny_policy_creation: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TrustedTypesError {
    #[error("Content Security Policy denied Trusted Types policy {0:?}")]
    PolicyDenied(String),
    #[error("Trusted Types policy {0:?} already exists")]
    DuplicatePolicy(String),
    #[error("{sink} requires {required}")]
    TypeRequired {
        sink: String,
        required: &'static str,
    },
}

#[derive(Debug, Default)]
pub struct TrustedTypesRegistry {
    config: TrustedTypesConfig,
    created: BTreeSet<String>,
}

impl TrustedTypesRegistry {
    pub fn new(config: TrustedTypesConfig) -> Self {
        Self {
            config,
            created: BTreeSet::new(),
        }
    }

    /// Policy changes affect future admissions, not the realm's creation history.
    pub fn configure(&mut self, config: TrustedTypesConfig) {
        self.config = config;
    }

    pub fn create_policy(&mut self, name: &str) -> Result<(), TrustedTypesError> {
        let already_created = self.created.contains(name);
        if already_created && (!self.config.allow_duplicates || name == "default") {
            return Err(TrustedTypesError::DuplicatePolicy(name.into()));
        }
        if self.config.deny_policy_creation
            || !self.config.allow_any_policy && !self.config.allowed_policy_names.contains(name)
        {
            return Err(TrustedTypesError::PolicyDenied(name.into()));
        }
        self.created.insert(name.into());
        Ok(())
    }

    pub fn was_created(&self, name: &str) -> bool {
        self.created.contains(name)
    }

    pub fn requires(&self, kind: TrustedKind) -> bool {
        self.config.require_script
            && matches!(
                kind,
                TrustedKind::Html | TrustedKind::Script | TrustedKind::ScriptUrl
            )
    }

    pub fn validate<'a>(
        &self,
        sink: &str,
        required: TrustedKind,
        value: &'a TrustedValue,
    ) -> Result<&'a str, TrustedTypesError> {
        if value.kind == required {
            Ok(value.value())
        } else {
            Err(TrustedTypesError::TypeRequired {
                sink: sink.into(),
                required: required.interface_name(),
            })
        }
    }

    pub fn config(&self) -> &TrustedTypesConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_check_location_defaults_and_counts_toward_payload_capacity() {
        let check: PolicyCheck =
            serde_json::from_str(r#"{"kind":"sink","sink":"Element innerHTML","input":"raw"}"#)
                .unwrap();
        assert!(
            matches!(&check,PolicyCheck::Sink{location,..}if *location==SourceLocation::default())
        );
        let located = PolicyCheck::Policy {
            name: "name".into(),
            already_created: false,
            location: SourceLocation {
                source_file: "https://example.test/script.js".into(),
                line: 4,
                column: 8,
            },
        };
        assert_eq!(
            located.payload_bytes(),
            4 + "https://example.test/script.js".len()
        );
        let roundtrip: PolicyCheck =
            serde_json::from_str(&serde_json::to_string(&located).unwrap()).unwrap();
        assert_eq!(roundtrip, located);
    }

    #[test]
    fn attribute_sink_lookup_respects_expanded_names() {
        let html = "http://www.w3.org/1999/xhtml";
        let svg = "http://www.w3.org/2000/svg";
        assert_eq!(
            attribute_type("script", "src", html, None),
            Some(TrustedKind::ScriptUrl)
        );
        assert_eq!(
            attribute_type("script", "src", html, Some("urn:custom")),
            None
        );
        assert_eq!(attribute_type("script", "SRC", html, None), None);
        assert_eq!(
            attribute_type("script", "href", svg, Some("http://www.w3.org/1999/xlink")),
            Some(TrustedKind::ScriptUrl)
        );
        assert_eq!(
            attribute_type("script", "href", svg, Some("urn:custom")),
            None
        );
        assert_eq!(
            attribute_type("div", "onclick", html, None),
            Some(TrustedKind::Script)
        );
        assert_eq!(attribute_type("div", "oncustom", html, None), None);
        assert_eq!(attribute_type("div", "onclick", "urn:custom", None), None);
    }

    #[test]
    fn property_sinks_are_case_sensitive_and_distinct_from_attributes() {
        for name in ["text", "textContent", "innerText"] {
            assert_eq!(
                property_type("SCRIPT", name, None),
                Some(TrustedKind::Script)
            );
        }
        assert_eq!(
            property_type("script", "src", None),
            Some(TrustedKind::ScriptUrl)
        );
        assert_eq!(
            property_type("div", "innerHTML", None),
            Some(TrustedKind::Html)
        );
        assert_eq!(
            property_type("svg", "outerHTML", Some("http://www.w3.org/2000/svg")),
            Some(TrustedKind::Html)
        );
        for name in ["onclick", "onerror", "innerhtml", "textcontent", "SRC"] {
            assert_eq!(property_type("script", name, None), None);
        }
        assert_eq!(property_type("script", "src", Some("urn:custom")), None);
        assert_eq!(
            property_type("script", "src", Some("http://www.w3.org/2000/svg")),
            None
        );
    }

    #[test]
    fn default_policy_remains_unique_when_other_duplicates_are_allowed() {
        let mut registry = TrustedTypesRegistry::new(TrustedTypesConfig {
            allow_any_policy: true,
            allow_duplicates: true,
            ..Default::default()
        });
        registry.create_policy("ordinary").unwrap();
        registry.create_policy("ordinary").unwrap();
        registry.create_policy("default").unwrap();
        assert_eq!(
            registry.create_policy("default"),
            Err(TrustedTypesError::DuplicatePolicy("default".into()))
        );
    }

    #[test]
    fn policy_configuration_preserves_created_names() {
        let mut registry = TrustedTypesRegistry::new(TrustedTypesConfig {
            allow_any_policy: true,
            allow_duplicates: true,
            ..Default::default()
        });
        registry.create_policy("existing").unwrap();
        registry.create_policy("default").unwrap();
        registry.configure(TrustedTypesConfig {
            allow_any_policy: true,
            ..Default::default()
        });
        assert!(matches!(
            registry.create_policy("existing"),
            Err(TrustedTypesError::DuplicatePolicy(_))
        ));
        registry.configure(TrustedTypesConfig {
            allow_any_policy: true,
            allow_duplicates: true,
            ..Default::default()
        });
        registry.create_policy("existing").unwrap();
        assert!(matches!(
            registry.create_policy("default"),
            Err(TrustedTypesError::DuplicatePolicy(_))
        ));
    }

    #[test]
    fn policies_are_allowlisted_and_duplicates_are_rejected() {
        let mut registry = TrustedTypesRegistry::new(TrustedTypesConfig {
            allowed_policy_names: ["default".into(), "sanitizer".into()].into(),
            ..TrustedTypesConfig::default()
        });
        registry.create_policy("sanitizer").unwrap();
        assert!(matches!(
            registry.create_policy("sanitizer"),
            Err(TrustedTypesError::DuplicatePolicy(_))
        ));
        assert!(matches!(
            registry.create_policy("other"),
            Err(TrustedTypesError::PolicyDenied(_))
        ));
    }

    #[test]
    fn trusted_values_have_unforgeable_rust_brands() {
        let registry = TrustedTypesRegistry::new(TrustedTypesConfig {
            require_script: true,
            allow_any_policy: true,
            ..TrustedTypesConfig::default()
        });
        let html = TrustedValue::new(TrustedKind::Html, "<b>safe</b>".into(), "default".into());
        assert_eq!(
            registry
                .validate("Element.innerHTML", TrustedKind::Html, &html)
                .unwrap(),
            "<b>safe</b>"
        );
        assert!(
            registry
                .validate("eval", TrustedKind::Script, &html)
                .is_err()
        );
    }
}
