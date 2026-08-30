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

    pub fn create_policy(&mut self, name: &str) -> Result<(), TrustedTypesError> {
        let already_created = self.created.contains(name);
        if already_created && !self.config.allow_duplicates {
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
