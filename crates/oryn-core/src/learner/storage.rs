use crate::intent::definition::IntentDefinition;
use std::path::PathBuf;
use tokio::fs;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_yaml::Error),
}

pub struct LearnerStorage {
    base_path: PathBuf,
}

impl LearnerStorage {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    pub fn default_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".oryn")
            .join("learned")
    }

    pub async fn save_intent(
        &self,
        domain: &str,
        intent: &IntentDefinition,
    ) -> Result<(), StorageError> {
        let name = &intent.name;
        let domain_dir = self.base_path.join(domain);

        if !domain_dir.exists() {
            fs::create_dir_all(&domain_dir).await?;
        }

        let file_path = domain_dir.join(format!("{}.yaml", name));
        let yaml = serde_yaml::to_string(intent)?;
        fs::write(file_path, yaml).await?;

        Ok(())
    }

    pub async fn load_intents(&self, domain: &str) -> Result<Vec<IntentDefinition>, StorageError> {
        let domain_dir = self.base_path.join(domain);
        if !domain_dir.exists() {
            return Ok(Vec::new());
        }

        let mut intents = Vec::new();
        let mut entries = fs::read_dir(domain_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                let content = fs::read_to_string(&path).await?;
                // Ignore malformed files for now, or log them?
                if let Ok(intent) = serde_yaml::from_str::<IntentDefinition>(&content) {
                    intents.push(intent);
                }
            }
        }

        Ok(intents)
    }

    pub async fn delete_intent(&self, domain: &str, name: &str) -> Result<(), StorageError> {
        let file_path = self.base_path.join(domain).join(format!("{}.yaml", name));
        if file_path.exists() {
            fs::remove_file(file_path).await?;
        }
        Ok(())
    }
}
