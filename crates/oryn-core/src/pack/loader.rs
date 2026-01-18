use super::definition::PackMetadata;
use crate::intent::definition::IntentDefinition;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PackLoadError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("Glob pattern error: {0}")]
    Pattern(#[from] glob::PatternError),
    #[error("Glob iteration error: {0}")]
    Glob(#[from] glob::GlobError),
    #[error("Pack manifest not found at {0}")]
    ManifestNotFound(PathBuf),
    #[error("Intent loader parsing not implemented yet")]
    IntentParseError, // Place holder until we connect the YAML loader
}

pub struct LoadedPack {
    pub metadata: PackMetadata,
    pub intents: Vec<IntentDefinition>,
    pub root_path: PathBuf,
}

pub struct PackLoader;

impl PackLoader {
    pub async fn load_pack(path: &Path) -> Result<LoadedPack, PackLoadError> {
        let manifest_path = path.join("pack.yaml");
        if !manifest_path.exists() {
            return Err(PackLoadError::ManifestNotFound(manifest_path));
        }

        let content = tokio::fs::read_to_string(&manifest_path).await?;
        let metadata: PackMetadata = serde_yaml::from_str(&content)?;

        let mut intents = Vec::new();
        for pattern in &metadata.intents {
            let full_pattern = path.join(pattern);
            let pattern_str = full_pattern.to_str().ok_or_else(|| {
                PackLoadError::Pattern(glob::PatternError {
                    pos: 0,
                    msg: "Invalid UTF-8 path",
                })
            })?;

            for entry in glob::glob(pattern_str)? {
                let entry_path = entry?;
                if entry_path.is_file() {
                    let intent_content = tokio::fs::read_to_string(&entry_path).await?;
                    // TODO: better error handling for individual intent parse failures?
                    // For now, we fail the whole pack load.
                    let intent: IntentDefinition = serde_yaml::from_str(&intent_content)?;
                    intents.push(intent);
                }
            }
        }

        Ok(LoadedPack {
            metadata,
            intents,
            root_path: path.to_path_buf(),
        })
    }
}
