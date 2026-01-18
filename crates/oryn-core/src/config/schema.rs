use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OrynConfig {
    #[serde(default)]
    pub intent_engine: IntentEngineConfig,
    #[serde(default)]
    pub packs: PacksConfig,
    #[serde(default)]
    pub learning: LearningConfig,
    #[serde(default)]
    pub security: SecurityConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentEngineConfig {
    #[serde(default = "default_timeout_ms")]
    pub default_timeout_ms: u64,
    #[serde(default = "default_step_timeout_ms")]
    pub step_timeout_ms: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,
    #[serde(default = "default_retry_delay_ms")]
    pub retry_delay_ms: u64,
    #[serde(default)]
    pub strict_mode: bool,
}

impl Default for IntentEngineConfig {
    fn default() -> Self {
        Self {
            default_timeout_ms: default_timeout_ms(),
            step_timeout_ms: default_step_timeout_ms(),
            max_retries: default_max_retries(),
            retry_delay_ms: default_retry_delay_ms(),
            strict_mode: false,
        }
    }
}

fn default_timeout_ms() -> u64 {
    30000
}

fn default_step_timeout_ms() -> u64 {
    10000
}

fn default_max_retries() -> usize {
    3
}

fn default_retry_delay_ms() -> u64 {
    1000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacksConfig {
    #[serde(default = "default_auto_load")]
    pub auto_load: bool,
    #[serde(default = "default_pack_paths")]
    pub pack_paths: Vec<PathBuf>,
}

impl Default for PacksConfig {
    fn default() -> Self {
        Self {
            auto_load: default_auto_load(),
            pack_paths: default_pack_paths(),
        }
    }
}

fn default_auto_load() -> bool {
    true
}

fn default_pack_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".oryn").join("packs"));
    }
    paths.push(PathBuf::from("./packs"));
    paths
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningConfig {
    #[serde(default = "default_learning_enabled")]
    pub enabled: bool,
    #[serde(default = "default_min_observations")]
    pub min_observations: usize,
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f64,
    #[serde(default)]
    pub auto_accept: bool,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: default_learning_enabled(),
            min_observations: default_min_observations(),
            min_confidence: default_min_confidence(),
            auto_accept: false,
        }
    }
}

fn default_learning_enabled() -> bool {
    false
}

fn default_min_observations() -> usize {
    3
}

fn default_min_confidence() -> f64 {
    0.75
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_sensitive_fields")]
    pub sensitive_fields: Vec<String>,
    #[serde(default = "default_redact_in_logs")]
    pub redact_in_logs: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            sensitive_fields: default_sensitive_fields(),
            redact_in_logs: default_redact_in_logs(),
        }
    }
}

fn default_sensitive_fields() -> Vec<String> {
    vec![
        "password".to_string(),
        "token".to_string(),
        "card_number".to_string(),
        "cvv".to_string(),
        "ssn".to_string(),
        "secret".to_string(),
    ]
}

fn default_redact_in_logs() -> bool {
    true
}
