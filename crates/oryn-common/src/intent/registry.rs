use crate::intent::definition::{IntentDefinition, IntentTier};
use std::collections::HashMap;

/// Registry for managing all available intents in the system.
///
/// The registry handles intent storage, lookup priorities, and
/// pattern-based availability matching.
#[derive(Debug, Default)]
pub struct IntentRegistry {
    /// Intents indexed by name.
    /// Since multiple definitions might exist (built-in vs loaded),
    /// we store the one with the highest priority tier.
    intents: HashMap<String, IntentDefinition>,

    /// Secondary index for pattern matching: Pattern Name -> List of Intent Names
    /// Used to quickly find which intents are enabled by a specific pattern.
    patterns_to_intents: HashMap<String, Vec<String>>,
}

impl IntentRegistry {
    pub fn new() -> Self {
        Self {
            intents: HashMap::new(),
            patterns_to_intents: HashMap::new(),
        }
    }

    /// Register a new intent definition.
    /// Returns true if the intent was added/updated, false if it was ignored
    /// due to lower priority.
    pub fn register(&mut self, definition: IntentDefinition) -> bool {
        let name = definition.name.clone();

        // check priority if intent already exists
        if let Some(existing) = self.intents.get(&name)
            && !Self::is_higher_priority(&definition.tier, &existing.tier)
        {
            return false;
        }

        // Index triggers for quick lookup
        self.index_triggers(&definition);

        // Store definition
        self.intents.insert(name, definition);
        true
    }

    /// Unregister an intent by name.
    /// Returns true if the intent was removed.
    pub fn unregister(&mut self, name: &str) -> bool {
        if let Some(def) = self.intents.remove(name) {
            // Remove from patterns index
            for pattern in &def.triggers.patterns {
                if let Some(list) = self.patterns_to_intents.get_mut(pattern)
                    && let Some(pos) = list.iter().position(|x| x == name)
                {
                    list.remove(pos);
                }
            }
            true
        } else {
            false
        }
    }

    /// Retrieve an intent definition by name.
    pub fn get(&self, name: &str) -> Option<&IntentDefinition> {
        self.intents.get(name)
    }

    /// List all registered intents.
    pub fn list(&self) -> Vec<&IntentDefinition> {
        self.intents.values().collect()
    }

    /// Find all intents that are triggered by the given pattern.
    pub fn get_by_pattern(&self, pattern: &str) -> Vec<&IntentDefinition> {
        self.patterns_to_intents
            .get(pattern)
            .map(|names| names.iter().filter_map(|name| self.get(name)).collect())
            .unwrap_or_default()
    }

    /// Check if the new tier has higher priority than the old tier.
    /// Priority order: Loaded > BuiltIn > Discovered
    fn is_higher_priority(new_tier: &IntentTier, old_tier: &IntentTier) -> bool {
        match (new_tier, old_tier) {
            (IntentTier::Loaded, IntentTier::BuiltIn) => true,
            (IntentTier::Loaded, IntentTier::Discovered) => true,
            (IntentTier::BuiltIn, IntentTier::Discovered) => true,
            // Same tier replacements are allowed (e.g. newer version of loaded intent)
            (a, b) if a == b => true,
            _ => false,
        }
    }

    fn index_triggers(&mut self, definition: &IntentDefinition) {
        for pattern in &definition.triggers.patterns {
            self.patterns_to_intents
                .entry(pattern.clone())
                .or_default()
                .push(definition.name.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::definition::IntentTier;

    fn mock_intent(name: &str, tier: IntentTier) -> IntentDefinition {
        IntentDefinition {
            name: name.to_string(),
            description: None,
            version: "1.0".to_string(),
            tier,
            triggers: Default::default(),
            parameters: vec![],
            steps: vec![],
            flow: None,
            success: None,
            failure: None,
            options: Default::default(),
        }
    }

    #[test]
    fn test_registration_priority() {
        let mut registry = IntentRegistry::new();

        // 1. Register BuiltIn (baseline)
        let builtin = mock_intent("login", IntentTier::BuiltIn);
        assert!(registry.register(builtin));
        assert_eq!(registry.get("login").unwrap().tier, IntentTier::BuiltIn);

        // 2. Register Discovered (should NOT override BuiltIn)
        let discovered = mock_intent("login", IntentTier::Discovered);
        assert!(!registry.register(discovered));
        assert_eq!(registry.get("login").unwrap().tier, IntentTier::BuiltIn);

        // 3. Register Loaded (should override BuiltIn)
        let loaded = mock_intent("login", IntentTier::Loaded);
        assert!(registry.register(loaded));
        assert_eq!(registry.get("login").unwrap().tier, IntentTier::Loaded);
    }
}
