use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackMetadata {
    pub pack: String,
    pub version: String,
    pub description: String,
    pub domains: Vec<String>,
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub intents: Vec<String>,
    #[serde(default)]
    pub auto_load: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum PackTrust {
    Full,
    Verified,
    Sandboxed,
    Untrusted,
}
