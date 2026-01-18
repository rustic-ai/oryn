use super::definition::PackMetadata;
use super::loader::{PackLoadError, PackLoader};
use crate::intent::registry::IntentRegistry;
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PackError {
    #[error("Load error: {0}")]
    Load(#[from] PackLoadError),
    #[error("Pack not found: {0}")]
    NotFound(String),
}

pub struct PackManager {
    loaded_packs: HashMap<String, PackMetadata>, // Map pack name to metadata
    registry: IntentRegistry,
    pack_paths: Vec<PathBuf>,
}

impl PackManager {
    pub fn new(registry: IntentRegistry, pack_paths: Vec<PathBuf>) -> Self {
        Self {
            loaded_packs: HashMap::new(),
            registry,
            pack_paths,
        }
    }

    pub fn registry(&self) -> &IntentRegistry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut IntentRegistry {
        &mut self.registry
    }

    pub fn list_loaded(&self) -> Vec<&PackMetadata> {
        self.loaded_packs.values().collect()
    }

    pub async fn load_pack_by_name(&mut self, name: &str) -> Result<(), PackError> {
        // Simple resolution: check each pack path for a directory with the name
        for path in &self.pack_paths {
            let pack_dir = path.join(name);
            if pack_dir.exists() {
                return self.load_pack_from_dir(&pack_dir).await;
            }
        }
        Err(PackError::NotFound(name.to_string()))
    }

    pub async fn load_pack_from_dir(&mut self, path: &std::path::Path) -> Result<(), PackError> {
        let loaded = PackLoader::load_pack(path).await?;

        // Register intents
        for _intent in loaded.intents {
            // Register as discovered tiers? Or builtin?
            // For now, let's assume they are Discovered or have a new tier.
            // self.registry.register(intent);
            // TODO: Expose register method on registry that takes a definition
        }

        self.loaded_packs
            .insert(loaded.metadata.pack.clone(), loaded.metadata);
        Ok(())
    }

    pub fn unload_pack(&mut self, name: &str) -> Result<(), PackError> {
        if self.loaded_packs.remove(name).is_some() {
            // TODO: Remove intents associated with this pack from registry
            // This requires registry to track source of intents
            Ok(())
        } else {
            Err(PackError::NotFound(name.to_string()))
        }
    }

    pub fn should_auto_load(&self, url: &str) -> Option<String> {
        // Heuristic: check if URL contains any known domain?
        // Or we need an index of all available packs and their domains.
        // For MVP, maybe we scan all packs once on startup to build an index?
        // Or simpler: mapping domain -> pack name
        if url.contains("github.com") {
            // This is hardcoded for now as an example, ideally we check against available packs
            return Some("github.com".to_string());
        }
        None
    }
}
