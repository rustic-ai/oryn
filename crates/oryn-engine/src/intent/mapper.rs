use oryn_common::intent::definition::IntentDefinition;
use oryn_common::intent::registry::IntentRegistry;
use oryn_common::protocol::{DetectedPatterns, ScanResult};
use std::collections::HashSet;

pub struct IntentMapper<'a> {
    registry: &'a IntentRegistry,
}

impl<'a> IntentMapper<'a> {
    pub fn new(registry: &'a IntentRegistry) -> Self {
        Self { registry }
    }

    /// Returns a list of available intents based on the scan result.
    pub fn available_intents(&self, result: &ScanResult) -> Vec<&IntentDefinition> {
        let mut intents = Vec::new();
        let mut seen = HashSet::new();

        if let Some(patterns) = &result.patterns {
            for pattern_key in self.extract_pattern_keys(patterns) {
                for intent in self.registry.get_by_pattern(&pattern_key) {
                    if seen.insert(intent.name.clone()) {
                        intents.push(intent);
                    }
                }
            }
        }

        // Also check for "always available" or global intents?
        // e.g. "scroll", "go_back" might be triggered by presence of scrollbar (in stats?)
        // For now, focus on Pattern-based.

        intents
    }

    fn extract_pattern_keys(&self, patterns: &DetectedPatterns) -> Vec<String> {
        let mut keys = Vec::new();
        if patterns.login.is_some() {
            keys.push("login_form".to_string());
        }
        if patterns.search.is_some() {
            keys.push("search_box".to_string());
        }
        if patterns.pagination.is_some() {
            keys.push("pagination".to_string());
        }
        if patterns.modal.is_some() {
            keys.push("modal".to_string());
        }
        if patterns.cookie_banner.is_some() {
            keys.push("cookie_banner".to_string());
        }
        keys
    }
}
