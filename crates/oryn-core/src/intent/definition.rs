use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// =============================================================================
// Multi-Page Flow Definitions
// =============================================================================

/// A multi-page flow definition that orchestrates workflows spanning multiple page navigations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowDefinition {
    /// Optional explicit start page. If not specified, the first page in the list is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    /// The pages that make up this flow.
    pub pages: Vec<PageDef>,
}

/// Definition of a single page in a multi-page flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageDef {
    /// Unique name for this page within the flow.
    pub name: String,
    /// URL regex pattern that identifies this page.
    pub url_pattern: String,
    /// Actions to execute on this page (intent references or inline steps).
    #[serde(default)]
    pub intents: Vec<PageAction>,
    /// What to do after this page completes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<PageTransition>,
    /// Optional error handler page name to transition to on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_error: Option<String>,
    /// Data extraction rules to run after page actions complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract: Option<HashMap<String, Value>>,
}

/// An action to execute on a page - either a reference to another intent or inline steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PageAction {
    /// Reference to another intent by name.
    IntentRef(String),
    /// Inline steps to execute.
    Inline { steps: Vec<Step> },
}

/// Transition specification for moving between pages in a flow.
/// Supports two YAML formats:
/// - `next: { page: "page_name" }` - transition to another page
/// - `next: end` - end the flow
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PageTransition {
    /// Transition to another page by name.
    Page { page: String },
    /// End the flow successfully (string "end").
    End(EndMarker),
}

/// Marker for the "end" transition. Deserializes from the string "end".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndMarker;

impl Serialize for EndMarker {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str("end")
    }
}

impl<'de> Deserialize<'de> for EndMarker {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s == "end" {
            Ok(EndMarker)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected 'end', found '{s}'"
            )))
        }
    }
}

impl PageTransition {
    /// Create a page transition.
    pub fn to_page(name: impl Into<String>) -> Self {
        PageTransition::Page { page: name.into() }
    }

    /// Create an end transition.
    pub fn end() -> Self {
        PageTransition::End(EndMarker)
    }

    /// Check if this is an end transition.
    pub fn is_end(&self) -> bool {
        matches!(self, PageTransition::End(_))
    }

    /// Get the target page name if this is a page transition.
    pub fn target_page(&self) -> Option<&str> {
        match self {
            PageTransition::Page { page } => Some(page),
            PageTransition::End(_) => None,
        }
    }
}

/// Tier of an intent definition, determining its priority and origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentTier {
    BuiltIn,
    Loaded,
    Discovered,
}

/// A complete definition of an intent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub version: String,
    pub tier: IntentTier,
    #[serde(default)]
    pub triggers: IntentTriggers,
    #[serde(default)]
    pub parameters: Vec<ParameterDef>,
    /// Steps for single-page intents. Either `steps` or `flow` should be provided.
    #[serde(default)]
    pub steps: Vec<Step>,
    /// Multi-page flow definition. Either `steps` or `flow` should be provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<FlowDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<SuccessCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<FailureCondition>,
    #[serde(default)]
    pub options: IntentOptions,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntentTriggers {
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterDef {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: ParamType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    String,
    Number,
    Boolean,
    Object,
    Array,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Step {
    Action(ActionStep),
    Branch(BranchStepWrapper),
    Loop(LoopStepWrapper),
    Try(TryStepWrapper),
    Checkpoint(CheckpointStepWrapper),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionStep {
    pub action: ActionType,
    #[serde(default)]
    pub target: Option<TargetSpec>,
    #[serde(default)]
    pub on_error: Option<Vec<Step>>,
    #[serde(flatten)]
    pub options: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchStepWrapper {
    pub branch: BranchDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchDef {
    #[serde(rename = "if")]
    pub condition: Condition,
    #[serde(rename = "then")]
    pub then_steps: Vec<Step>,
    #[serde(rename = "else", default)]
    pub else_steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopStepWrapper {
    pub loop_: LoopDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopDef {
    pub over: String,
    #[serde(rename = "as")]
    pub as_var: String,
    pub steps: Vec<Step>,
    pub max: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TryStepWrapper {
    #[serde(rename = "try")]
    pub try_: TryDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TryDef {
    pub steps: Vec<Step>,
    pub catch: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointStepWrapper {
    pub checkpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Click,
    Type,
    Select,
    Check,
    Uncheck,
    Clear,
    Scroll,
    Wait,
    FillForm,
    Intent,
    Execute,
    // Navigation actions for multi-page flows
    Navigate,
    GoBack,
    GoForward,
    Refresh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetSpec {
    #[serde(flatten)]
    pub kind: TargetKind,
    #[serde(default)]
    pub fallback: Option<Box<TargetSpec>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TargetKind {
    Pattern {
        pattern: String,
    },
    Role {
        role: String,
    },
    Text {
        text: String,
        #[serde(default)]
        match_type: MatchType,
    },
    Selector {
        selector: String,
    },
    Id {
        id: u64,
    },
    Near {
        near: Box<TargetSpec>,
        anchor: Box<TargetSpec>,
    },
    Inside {
        inside: Box<TargetSpec>,
        container: Box<TargetSpec>,
    },
    After {
        after: Box<TargetSpec>,
        anchor: Box<TargetSpec>,
    },
    Before {
        before: Box<TargetSpec>,
        anchor: Box<TargetSpec>,
    },
    Contains {
        contains: Box<TargetSpec>,
        content: Box<TargetSpec>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    /// Exact string match.
    Exact,
    /// Substring match (default).
    #[default]
    Contains,
    /// Regex matching - intentionally not implemented.
    ///
    /// The resolver's scoring system handles exact/contains matching well,
    /// and no builtin intents currently require regex matching.
    /// This variant exists for future extensibility if a clear use case emerges.
    Regex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    PatternExists(String),
    PatternGone(String),
    Visible(TargetSpec),
    Hidden(TargetSpec),
    UrlContains(Vec<String>),
    UrlMatches(String),
    TextContains {
        text: String,
        within: Option<TargetSpec>,
    },
    Count {
        selector: String,
        min: Option<usize>,
        max: Option<usize>,
    },
    Expression(String),
    All(Vec<Condition>),
    Any(Vec<Condition>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessCondition {
    pub conditions: Vec<Condition>,
    pub extract: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureCondition {
    pub conditions: Vec<Condition>,
    pub recovery: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentOptions {
    #[serde(default = "default_timeout")]
    pub timeout: u64, // milliseconds
    #[serde(default)]
    pub retry: RetryConfig,
    #[serde(default)]
    pub checkpoint: bool,
}

impl Default for IntentOptions {
    fn default() -> Self {
        Self {
            timeout: default_timeout(),
            retry: RetryConfig::default(),
            checkpoint: false,
        }
    }
}

fn default_timeout() -> u64 {
    30000
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetryConfig {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: usize,
    #[serde(default = "default_delay_ms")]
    pub delay_ms: u64,
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
}

fn default_max_attempts() -> usize {
    3
}

fn default_delay_ms() -> u64 {
    1000
}

fn default_backoff_multiplier() -> f64 {
    2.0
}
