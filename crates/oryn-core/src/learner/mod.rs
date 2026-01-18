pub mod observer;
pub mod proposer;
pub mod recognizer;
pub mod storage;

use serde::{Deserialize, Serialize};

pub use crate::config::schema::LearningConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLog {
    pub timestamp: std::time::SystemTime,
    pub domain: String,
    pub url: String,
    pub command: String, // String representation of the command
    pub input_snapshot: Option<Box<serde_json::Value>>, // Captured input state if any
}
