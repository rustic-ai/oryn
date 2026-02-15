use super::schema::OrynConfig;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse config file: {0}")]
    Parse(#[from] serde_yaml::Error),
}

pub struct ConfigLoader;

impl ConfigLoader {
    /// Load from default locations:
    /// 1. ./oryn.yaml
    /// 2. ~/.oryn/config.yaml
    /// 3. Default configuration
    pub async fn load_default() -> Result<OrynConfig, ConfigError> {
        // Check current directory
        let local_config = PathBuf::from("./oryn.yaml");
        if local_config.exists() {
            return Self::load_from(&local_config).await;
        }

        // Check home directory
        if let Some(home) = dirs::home_dir() {
            let home_config = home.join(".oryn").join("config.yaml");
            if home_config.exists() {
                return Self::load_from(&home_config).await;
            }
        }

        // Return default
        Ok(OrynConfig::default())
    }

    pub async fn load_from(path: &Path) -> Result<OrynConfig, ConfigError> {
        let content = tokio::fs::read_to_string(path).await?;
        let config: OrynConfig = serde_yaml::from_str(&content)?;
        Ok(config)
    }
}
