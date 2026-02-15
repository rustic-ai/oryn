use oryn_common::intent::definition::{IntentDefinition, IntentTier};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("Intent already exists: {0}")]
    AlreadyExists(String),
    #[error("Intent not found: {0}")]
    NotFound(String),
    #[error("Export error: {0}")]
    ExportError(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_yaml::Error),
}

pub struct SessionIntent {
    pub definition: IntentDefinition,
    pub created_at: Instant,
    pub invocation_count: usize,
}

pub struct SessionIntentManager {
    intents: HashMap<String, SessionIntent>,
}

impl Default for SessionIntentManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionIntentManager {
    pub fn new() -> Self {
        Self {
            intents: HashMap::new(),
        }
    }

    pub fn define(&mut self, mut definition: IntentDefinition) -> Result<(), SessionError> {
        if self.intents.contains_key(&definition.name) {
            return Err(SessionError::AlreadyExists(definition.name.clone()));
        }

        // Ensure tier is Discovered (or Session specific?)
        definition.tier = IntentTier::Discovered;

        self.intents.insert(
            definition.name.clone(),
            SessionIntent {
                definition,
                created_at: Instant::now(),
                invocation_count: 0,
            },
        );
        Ok(())
    }

    pub fn undefine(&mut self, name: &str) -> Result<(), SessionError> {
        if self.intents.remove(name).is_some() {
            Ok(())
        } else {
            Err(SessionError::NotFound(name.to_string()))
        }
    }

    pub fn get(&self, name: &str) -> Option<&SessionIntent> {
        self.intents.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut SessionIntent> {
        self.intents.get_mut(name)
    }

    pub fn list(&self) -> Vec<&SessionIntent> {
        self.intents.values().collect()
    }

    pub fn export(&self, name: &str, path: &Path) -> Result<(), SessionError> {
        let intent = self
            .get(name)
            .ok_or_else(|| SessionError::NotFound(name.to_string()))?;
        let f = std::fs::File::create(path)?;
        serde_yaml::to_writer(f, &intent.definition)?;
        Ok(())
    }
}
