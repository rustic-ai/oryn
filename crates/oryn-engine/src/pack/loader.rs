use super::definition::PackMetadata;
use oryn_common::intent::definition::IntentDefinition;
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

use crate::intent::schema::Validatable;

impl PackLoader {
    pub async fn load_metadata(path: &Path) -> Result<PackMetadata, PackLoadError> {
        let manifest_path = path.join("pack.yaml");
        if !manifest_path.exists() {
            return Err(PackLoadError::ManifestNotFound(manifest_path));
        }

        let content = tokio::fs::read_to_string(&manifest_path).await?;
        let metadata: PackMetadata = serde_yaml::from_str(&content)?;
        Ok(metadata)
    }

    pub async fn load_pack(path: &Path) -> Result<LoadedPack, PackLoadError> {
        let metadata = Self::load_metadata(path).await?;

        let mut intents = Vec::new();
        for pattern in &metadata.intents {
            let full_pattern = path.join(pattern);
            let pattern_str =
                full_pattern
                    .to_str()
                    .ok_or(PackLoadError::Pattern(glob::PatternError {
                        pos: 0,
                        msg: "Invalid UTF-8 path",
                    }))?;

            for entry in glob::glob(pattern_str)? {
                let entry_path = entry?;
                if entry_path.is_file() {
                    let intent_content = tokio::fs::read_to_string(&entry_path).await?;
                    let intent: IntentDefinition = serde_yaml::from_str(&intent_content)?;

                    // Validate
                    if let Err(e) = intent.validate() {
                        eprintln!("Warning: Invalid intent in pack {:?}: {}", entry_path, e);
                        // We could fail hard, but maybe just skip invalid ones?
                        // For now fail hard as per general practice for robust systems?
                        // Actually let's skip and log to allow partial load,
                        // but return error if *nothing* loads?
                        // The prompt asked for validation. Let's strict fail matching `IntentLoader`.
                        // Actually `IntentLoader` skips. Let's skip here too.
                        continue;
                    }

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
