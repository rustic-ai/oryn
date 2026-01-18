use crate::intent::definition::IntentDefinition;
use crate::intent::registry::IntentRegistry;
use crate::intent::schema::{Validatable, ValidationError};
use glob::glob;
use std::fs;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum LoaderError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Glob error: {0}")]
    Glob(#[from] glob::PatternError),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_yaml::Error),
    #[error("Validation error: {0}")]
    Validation(#[from] ValidationError),
}

pub struct IntentLoader;

impl IntentLoader {
    pub fn load_from_dir(path: &Path, registry: &mut IntentRegistry) -> Result<usize, LoaderError> {
        let pattern = path.join("**/*.yaml");
        let pattern_str = pattern.to_str().unwrap_or("*.yaml");
        let mut count = 0;

        for file_path in glob(pattern_str)?.flatten() {
            let content = match fs::read_to_string(&file_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed to read file {:?}: {}", file_path, e);
                    continue;
                }
            };

            let intent = match serde_yaml::from_str::<IntentDefinition>(&content) {
                Ok(i) => {
                    if let Err(e) = i.validate() {
                        eprintln!("Failed to validate intent from {:?}: {}", file_path, e);
                        continue;
                    }
                    i
                }
                Err(e) => {
                    eprintln!("Failed to parse intent from {:?}: {}", file_path, e);
                    continue;
                }
            };

            if registry.register(intent) {
                count += 1;
            }
        }

        Ok(count)
    }
}
