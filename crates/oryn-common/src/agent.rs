//! Stable agent-facing intent contract shared by Rust, Python, CLI JSON and MCP.
//!
//! This is an orchestration IR, not another resolver. Native adapters convert
//! `Target` into the existing semantic/OIL resolver and execute the resulting
//! generation-checked `SemanticRef` through the existing action engine.

use serde::{Deserialize, Serialize};

use crate::v3::{ProjectionProfile, Revision, SemanticRef};

pub const AGENT_INTENT_CONTRACT_VERSION: u16 = 2;

/// Versioned wire envelope used by subprocess, MCP, and future transports.
/// Keeping the envelope in the shared contract prevents each adapter from
/// inventing its own JSON shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentRequest {
    pub contract_version: u16,
    pub intent: Intent,
}

impl IntentRequest {
    pub fn new(intent: Intent) -> Self {
        Self {
            contract_version: AGENT_INTENT_CONTRACT_VERSION,
            intent,
        }
    }

    pub fn validate(self) -> Result<Intent, String> {
        if self.contract_version != AGENT_INTENT_CONTRACT_VERSION {
            return Err(format!(
                "unsupported agent intent contract version {}; expected {}",
                self.contract_version, AGENT_INTENT_CONTRACT_VERSION
            ));
        }
        Ok(self.intent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Reference {
        reference: SemanticRef,
    },
    Alias {
        alias: u32,
    },
    Text {
        text: String,
    },
    Role {
        role: String,
    },
    Css {
        selector: String,
    },
    XPath {
        expression: String,
    },
    Near {
        target: Box<Target>,
        anchor: Box<Target>,
    },
    Inside {
        target: Box<Target>,
        container: Box<Target>,
    },
    After {
        target: Box<Target>,
        anchor: Box<Target>,
    },
    Before {
        target: Box<Target>,
        anchor: Box<Target>,
    },
    Contains {
        target: Box<Target>,
        content: Box<Target>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    Text {
        value: String,
    },
    /// Parent-owned secret selected by its non-sensitive label. The value is
    /// never serialized into an intent, trace, observation, Python object or
    /// MCP message.
    Secret {
        label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractKind {
    Text,
    Html,
    Links,
    Images,
    Tables,
    Url,
    Title,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveOptions {
    #[serde(default = "default_projection_profile")]
    pub profile: ProjectionProfile,
    #[serde(default)]
    pub since: Option<Revision>,
    #[serde(default)]
    pub include_geometry: bool,
    #[serde(default)]
    pub viewport_only: bool,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default)]
    pub include_structures: bool,
}

impl Default for ObserveOptions {
    fn default() -> Self {
        Self {
            profile: ProjectionProfile::Compact,
            since: None,
            include_geometry: false,
            viewport_only: false,
            include_hidden: false,
            include_structures: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case", deny_unknown_fields)]
pub enum Intent {
    Navigate {
        url: String,
    },
    Back,
    Forward,
    Refresh,
    Observe {
        #[serde(default)]
        options: ObserveOptions,
    },
    Click {
        target: Target,
        #[serde(default)]
        force: bool,
    },
    Type {
        target: Target,
        input: Input,
    },
    Clear {
        target: Target,
    },
    Check {
        target: Target,
        #[serde(default = "default_true")]
        checked: bool,
    },
    Select {
        target: Target,
        value: String,
    },
    Submit {
        target: Target,
    },
    Focus {
        target: Target,
    },
    Hover {
        target: Target,
    },
    /// Read text from the context-owned virtual clipboard. This never accesses
    /// the host operating-system clipboard.
    ClipboardRead,
    /// Replace text in the context-owned virtual clipboard.
    ClipboardWrite {
        value: String,
    },
    /// Select parent-imported file labels for a native file input. Labels are
    /// non-sensitive and resolve to opaque context-owned upload handles.
    Upload {
        target: Target,
        files: Vec<String>,
    },
    Extract {
        kind: ExtractKind,
        #[serde(default)]
        selector: Option<String>,
    },
    WaitRevision {
        after: Revision,
        timeout_ms: u64,
    },
}

const fn default_true() -> bool {
    true
}

const fn default_projection_profile() -> ProjectionProfile {
    ProjectionProfile::Compact
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_intent_serializes_only_the_label() {
        let intent = Intent::Type {
            target: Target::Role {
                role: "password".into(),
            },
            input: Input::Secret {
                label: "account-password".into(),
            },
        };
        let encoded = serde_json::to_string(&intent).unwrap();
        assert!(encoded.contains("account-password"));
        assert!(!encoded.contains("value"));
        assert_eq!(serde_json::from_str::<Intent>(&encoded).unwrap(), intent);
    }

    #[test]
    fn request_rejects_an_unknown_contract_version() {
        let request = IntentRequest {
            contract_version: AGENT_INTENT_CONTRACT_VERSION + 1,
            intent: Intent::Back,
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn secret_input_rejects_a_value_field_even_when_a_label_is_present() {
        let encoded = serde_json::json!({
            "intent": "type",
            "target": {"kind": "role", "role": "textbox"},
            "input": {"kind": "secret", "label": "password", "value": "must-not-enter"}
        });
        assert!(serde_json::from_value::<Intent>(encoded).is_err());
    }

    #[test]
    fn clipboard_intents_are_explicitly_context_virtual() {
        let request = IntentRequest::new(Intent::ClipboardWrite {
            value: "copied".into(),
        });
        assert_eq!(request.contract_version, 2);
        assert_eq!(
            request.validate().unwrap(),
            Intent::ClipboardWrite {
                value: "copied".into()
            }
        );
    }
}
