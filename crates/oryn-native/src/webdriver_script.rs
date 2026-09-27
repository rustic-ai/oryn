//! Typed, thread-safe envelopes for thread-affine native WebDriver execution.
use serde::{Deserialize, Serialize};
#[path = "webdriver_value.rs"]
mod value;
pub use value::ScriptValue;

pub const ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";
pub const SHADOW_KEY: &str = "shadow-6066-11e4-a52e-4f735466cecf";
pub const MAX_SCRIPT_BYTES: usize = 1024 * 1024;
pub const MAX_VALUE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_VALUE_NODES: usize = 100_000;
pub const MAX_VALUE_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptRequest {
    pub body: Vec<u16>,
    pub args: Vec<ScriptValue>,
    pub asynchronous: bool,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ScriptId(pub u64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ScriptPoll {
    Pending,
    Complete(ScriptValue),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ScriptError {
    pub code: String,
    pub message: String,
}
impl ScriptError {
    pub fn javascript(message: impl Into<String>) -> Self {
        Self {
            code: "javascript error".into(),
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid argument".into(),
            message: message.into(),
        }
    }
    pub fn timeout() -> Self {
        Self {
            code: "script timeout".into(),
            message: "script deadline exceeded".into(),
        }
    }
    pub fn retired() -> Self {
        Self::javascript("script document or realm was retired")
    }
    pub fn stale(shadow: bool) -> Self {
        Self {
            code: if shadow {
                "detached shadow root"
            } else {
                "stale element reference"
            }
            .into(),
            message: "script node reference is no longer attached to its document".into(),
        }
    }
}
impl From<super::HostError> for ScriptError {
    fn from(error: super::HostError) -> Self {
        match error {
            super::HostError::LimitExceeded { resource } => Self {
                code: "script timeout".into(),
                message: format!("native execution exceeded {resource} limit"),
            },
            other => Self::javascript(other.to_string()),
        }
    }
}

pub(crate) struct ValueBudget {
    nodes: usize,
    bytes: usize,
}
impl ValueBudget {
    pub(crate) fn new() -> Self {
        Self { nodes: 0, bytes: 0 }
    }
    pub(crate) fn consume(&mut self, depth: usize, bytes: usize) -> Result<(), ScriptError> {
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or_else(|| ScriptError::javascript("script result node limit"))?;
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| ScriptError::javascript("script result byte limit"))?;
        if depth > MAX_VALUE_DEPTH || self.nodes > MAX_VALUE_NODES || self.bytes > MAX_VALUE_BYTES {
            return Err(ScriptError::javascript(
                "script value exceeds native serialization limits",
            ));
        }
        Ok(())
    }
}

/// Supplied only by the trusted session actor, never by script request JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowHandleBinding {
    pub realm: oryn_js::realm::RealmId,
    pub document_generation: u64,
    pub stream_epoch: u64,
    pub handle: String,
    pub top_level: bool,
}
pub const WINDOW_KEY: &str = "window-fcc6-11e5-b4f8-330a88ab9d7f";
pub const FRAME_KEY: &str = "frame-075b-4da1-b6ba-e579c2d3230a";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebDriverFrameTarget {
    Top,
    Parent,
    Index(u32),
    ElementReference(String),
    ElementNode(oryn_dom::NodeId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationRealmSnapshot {
    pub realm: oryn_js::realm::RealmId,
    pub parent_realm: Option<oryn_js::realm::RealmId>,
    pub frame: Option<oryn_dom::NodeId>,
    pub frame_generation: Option<u64>,
    #[serde(default)]
    pub frame_token: Option<String>,
    pub document_root: oryn_dom::NodeId,
    pub document_generation: u64,
    pub stream_epoch: u64,
    /// Stable native capability identity, distinct from replaced DOM input.
    pub document_authority_epoch: u64,
    pub url: String,
    pub live: bool,
}

impl AutomationRealmSnapshot {
    /// Exact selected-document capability. URL is observational metadata and
    /// can change through same-document navigation without replacing authority.
    pub fn same_document_as(&self, selected: &Self) -> bool {
        self.same_navigable_as(selected)
            && (self.parent_realm.is_some()
                || (self.frame_generation.is_none() && selected.frame_generation.is_none()))
            && self.realm == selected.realm
            && self.document_root == selected.document_root
            && self.document_generation == selected.document_generation
            && self.stream_epoch == selected.stream_epoch
            && self.document_authority_epoch == selected.document_authority_epoch
    }

    /// Navigable identity survives a document commit but not replacement of
    /// its embedding frame connection. Page identity is checked by the caller.
    pub(crate) fn same_navigable_as(&self, selected: &Self) -> bool {
        if !self.live || !selected.live {
            return false;
        }
        match (self.parent_realm, selected.parent_realm) {
            (None, None) => {
                self.frame.is_none()
                    && selected.frame.is_none()
                    && self.frame_token.is_none()
                    && selected.frame_token.is_none()
            }
            (Some(parent), Some(selected_parent)) => {
                parent == selected_parent
                    && self.frame.is_some()
                    && self.frame == selected.frame
                    && self.frame_generation.is_some()
                    && self.frame_generation == selected.frame_generation
                    && self.frame_token.is_some()
                    && self.frame_token == selected.frame_token
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptContext {
    pub selected: Option<AutomationRealmSnapshot>,
    pub handles: Vec<WindowHandleBinding>,
    pub top_window_handle: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ElementCommand {
    Find {
        root: Option<String>,
        selectors: Vec<String>,
    },
    Accessibility {
        reference: String,
        role: bool,
    },
    AccessibilityProperties {
        reference: String,
        accessibility_node: bool,
    },
    Focus {
        reference: String,
    },
    FileInput {
        reference: String,
    },
    SetFiles {
        reference: String,
        files: Vec<crate::upload::WebDriverUpload>,
    },
    PrepareClick {
        reference: String,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FrameCommand {
    Inventory,
    /// Native OIL continuation of one completed action; not a public OIL command.
    ResolveActionSelection {
        current: AutomationRealmSnapshot,
        action_id: oryn_common::v3::ActionId,
    },
    NamedInventory,
    NavigateServiceWindow {
        current: AutomationRealmSnapshot,
        url: String,
        authority: oryn_js::navigation::ServiceNavigationAdmission,
    },
    NavigateScriptWindow {
        receipt: Option<oryn_js::navigation::NavigationExecutionToken>,
        current: AutomationRealmSnapshot,
        url: String,
        replace: bool,
        authority: oryn_js::navigation::ScriptNavigationAdmission,
    },
    NavigateFormWindow {
        current: AutomationRealmSnapshot,
        url: String,
        post: Option<oryn_js::navigation::PostResource>,
        referrer_policy: Option<String>,
        authority: Option<oryn_js::navigation::FormNavigationAuthority>,
    },
    Element {
        current: Option<AutomationRealmSnapshot>,
        command: ElementCommand,
    },
    FocusWindow {
        current: AutomationRealmSnapshot,
    },
    NavigateWindow {
        current: AutomationRealmSnapshot,
        url: String,
        initiator_url: String,
        referrer_policy: String,
    },
    Select {
        current: Option<AutomationRealmSnapshot>,
        target: WebDriverFrameTarget,
    },
    SelectSemantic {
        current: Option<AutomationRealmSnapshot>,
        reference: oryn_common::v3::SemanticRef,
    },
    NavigateSelectedFormWindow {
        selected: AutomationRealmSnapshot,
        url: String,
        post: Option<oryn_js::navigation::PostResource>,
        referrer_policy: Option<String>,
        authority: Option<oryn_js::navigation::FormNavigationAuthority>,
    },
    NavigateSelectedDocumentWindow {
        receipt: Option<oryn_js::navigation::NavigationExecutionToken>,
        selected: Option<AutomationRealmSnapshot>,
        url: String,
        source: oryn_dom::DocumentProvenance,
        referrer_policy: Option<String>,
        user_initiated: bool,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FrameResponse {
    NavigationAdmitted(AutomationRealmSnapshot),
    NamedInventory(Vec<(String, AutomationRealmSnapshot)>),
    Value(ScriptValue),
    Inventory(Vec<AutomationRealmSnapshot>),
    Selected(AutomationRealmSnapshot),
    NavigationStarted { document_generation: u64 },
}

#[cfg(test)]
mod selected_navigable_tests {
    use super::*;

    #[test]
    fn selected_document_task_preserves_native_source_and_involvement() {
        let command = FrameCommand::NavigateSelectedDocumentWindow {
            receipt: None,
            selected: Some(frame()),
            url: "https://target.test/next".into(),
            source: oryn_dom::DocumentProvenance {
                capability: vec![1, 2, 3],
                origin: "https://source.test".into(),
                origin_key: "native-origin".into(),
                inert: false,
            },
            referrer_policy: Some("no-referrer".into()),
            user_initiated: true,
        };
        let encoded = serde_json::to_vec(&command).unwrap();
        assert_eq!(
            serde_json::from_slice::<FrameCommand>(&encoded).unwrap(),
            command
        );
        let response = FrameResponse::NavigationStarted {
            document_generation: 7,
        };
        assert_eq!(
            serde_json::from_slice::<FrameResponse>(&serde_json::to_vec(&response).unwrap())
                .unwrap(),
            response
        );
    }

    fn frame() -> AutomationRealmSnapshot {
        let dom = oryn_dom::html::parse("<iframe id=frame></iframe>")
            .into_runtime()
            .unwrap();
        AutomationRealmSnapshot {
            realm: oryn_js::realm::RealmId(2),
            parent_realm: Some(oryn_js::realm::RealmId(1)),
            frame: dom.find_by_id("frame"),
            frame_generation: Some(7),
            frame_token: Some("native-frame-7".into()),
            document_root: dom.document(),
            document_generation: 1,
            stream_epoch: 0,
            document_authority_epoch: 0,
            url: "https://target.test/before".into(),
            live: true,
        }
    }

    #[test]
    fn selected_frame_survives_document_commit_but_not_connection_replacement() {
        let selected = frame();
        let mut current = selected.clone();
        current.realm = oryn_js::realm::RealmId(3);
        current.document_generation = 2;
        current.stream_epoch = 4;
        current.document_authority_epoch = 4;
        current.url = "https://target.test/after".into();
        assert!(current.same_navigable_as(&selected));
        current.frame_generation = Some(8);
        assert!(!current.same_navigable_as(&selected));
        current.frame_generation = selected.frame_generation;
        current.frame_token = Some("replacement".into());
        assert!(!current.same_navigable_as(&selected));
        current.frame_token = selected.frame_token.clone();
        current.parent_realm = Some(oryn_js::realm::RealmId(9));
        assert!(!current.same_navigable_as(&selected));
        current.parent_realm = selected.parent_realm;
        current.live = false;
        assert!(!current.same_navigable_as(&selected));
    }

    #[test]
    fn missing_native_frame_identity_cannot_match_a_selected_destination() {
        let selected = frame();
        let mut current = selected.clone();
        current.frame_token = None;
        let mut malformed = selected.clone();
        malformed.frame_token = None;
        assert!(!current.same_navigable_as(&malformed));
        current = selected.clone();
        current.frame_generation = None;
        malformed = selected.clone();
        malformed.frame_generation = None;
        assert!(!current.same_navigable_as(&malformed));
        let command = FrameCommand::NavigateSelectedFormWindow {
            selected,
            url: "https://target.test/accepted".into(),
            post: Some(oryn_js::navigation::PostResource {
                bytes: b"x=original".to_vec(),
                content_type: "application/x-www-form-urlencoded".into(),
            }),
            referrer_policy: Some("no-referrer".into()),
            authority: None,
        };
        let encoded = serde_json::to_vec(&command).unwrap();
        assert_eq!(
            serde_json::from_slice::<FrameCommand>(&encoded).unwrap(),
            command
        );
    }
    #[test]
    fn selected_document_requires_live_authority_and_complete_connection() {
        let selected = frame();
        assert!(selected.same_document_as(&selected));
        let mut changed = selected.clone();
        changed.url = "https://target.test/same-document".into();
        assert!(changed.same_document_as(&selected));
        let mutations: [fn(&mut AutomationRealmSnapshot); 6] = [
            |value: &mut AutomationRealmSnapshot| value.live = false,
            |value: &mut AutomationRealmSnapshot| value.document_authority_epoch += 1,
            |value: &mut AutomationRealmSnapshot| value.stream_epoch += 1,
            |value: &mut AutomationRealmSnapshot| value.frame_token = None,
            |value: &mut AutomationRealmSnapshot| value.frame_generation = None,
            |value: &mut AutomationRealmSnapshot| value.parent_realm = None,
        ];
        for mutate in mutations {
            let mut changed = selected.clone();
            mutate(&mut changed);
            assert!(!changed.same_document_as(&selected));
            assert!(!selected.same_document_as(&changed));
        }
        let mut malformed_root = selected.clone();
        malformed_root.parent_realm = None;
        malformed_root.frame = None;
        malformed_root.frame_token = None;
        assert!(!malformed_root.same_document_as(&malformed_root));
        malformed_root.frame_generation = None;
        assert!(malformed_root.same_document_as(&malformed_root));
    }
}
