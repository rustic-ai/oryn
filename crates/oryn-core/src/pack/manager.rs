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
    // Index of available packs (scanned but not necessarily loaded)
    available_packs: HashMap<String, PackMetadata>,
    // Map domain -> pack name for auto-loading
    domain_map: HashMap<String, String>,
}

impl PackManager {
    pub async fn new(registry: IntentRegistry, pack_paths: Vec<PathBuf>) -> Self {
        let mut manager = Self {
            loaded_packs: HashMap::new(),
            registry,
            pack_paths: pack_paths.clone(),
            available_packs: HashMap::new(),
            domain_map: HashMap::new(),
        };

        // Scan for available packs on startup
        manager.scan_available_packs().await;

        manager
    }

    /// Scans pack_paths for valid packs and populates available_packs and domain_map
    async fn scan_available_packs(&mut self) {
        for path in &self.pack_paths {
            // Read directory
            if let Ok(mut entries) = tokio::fs::read_dir(path).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    if entry.path().is_dir() {
                        // Try to load metadata only (cheap)
                        if let Ok(metadata) = PackLoader::load_metadata(&entry.path()).await {
                            let name = metadata.pack.clone();

                            // Map domains to this pack
                            for domain in &metadata.domains {
                                self.domain_map.insert(domain.clone(), name.clone());
                            }

                            self.available_packs.insert(name, metadata);
                        }
                    }
                }
            }
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

    pub fn list_available(&self) -> Vec<&PackMetadata> {
        self.available_packs.values().collect()
    }

    pub async fn load_pack_by_name(&mut self, name: &str) -> Result<(), PackError> {
        // Check if already loaded
        if self.loaded_packs.contains_key(name) {
            return Ok(());
        }

        // Find path
        for path in &self.pack_paths {
            let pack_dir = path.join(name);
            if pack_dir.exists() {
                return self.load_pack_from_dir(&pack_dir).await;
            }
        }
        Err(PackError::NotFound(name.to_string()))
    }

    pub async fn load_pack_from_dir(&mut self, path: &std::path::Path) -> Result<(), PackError> {
        let mut loaded = PackLoader::load_pack(path).await?;

        // Register intents
        let mut registered_names = Vec::new();
        for intent in loaded.intents {
            // Register as Loaded tier to override built-ins if needed
            let mut intent = intent;
            intent.tier = crate::intent::definition::IntentTier::Loaded;
            registered_names.push(intent.name.clone());
            self.registry.register(intent);
        }

        // Update metadata with actual loaded intents
        loaded.metadata.intents = registered_names;

        self.loaded_packs
            .insert(loaded.metadata.pack.clone(), loaded.metadata);
        Ok(())
    }

    pub fn unload_pack(&mut self, name: &str) -> Result<(), PackError> {
        if let Some(metadata) = self.loaded_packs.remove(name) {
            // Remove intents associated with this pack
            for intent_name in &metadata.intents {
                self.registry.unregister(intent_name);
            }
            Ok(())
        } else {
            Err(PackError::NotFound(name.to_string()))
        }
    }

    pub fn should_auto_load(&self, url: &str) -> Option<String> {
        if let Ok(parsed) = url::Url::parse(url)
            && let Some(domain) = parsed.host_str()
        {
            // Check exact match
            if let Some(pack_name) = self.domain_map.get(domain) {
                // Only auto-load if not already loaded
                if !self.loaded_packs.contains_key(pack_name) {
                    return Some(pack_name.clone());
                }
            }

            // TODO: Support wildcard domains or suffix matching if needed
            // e.g. *.github.com -> github-pack
        }
        None
    }
}
