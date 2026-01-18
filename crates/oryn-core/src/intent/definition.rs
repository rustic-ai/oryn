use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

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
    pub version: String,
    pub tier: IntentTier,
    #[serde(default)]
    pub triggers: IntentTriggers,
    #[serde(default)]
    pub parameters: Vec<ParameterDef>,
    pub steps: Vec<Step>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default)]
    pub attempts: usize,
    #[serde(default)]
    pub backoff: u64,
}
