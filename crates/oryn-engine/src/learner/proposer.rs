use super::recognizer::Pattern;
use oryn_common::intent::define_parser::parse_define;
use oryn_common::intent::definition::{IntentDefinition, IntentTier};

#[derive(Default)]
pub struct Proposer;

impl Proposer {
    pub fn new() -> Self {
        Self
    }

    pub fn propose(&self, pattern: &Pattern) -> Option<IntentDefinition> {
        // Convert pattern steps to define syntax
        // Pattern steps are raw strings from logs
        // e.g. "click 'Button'", "type 'Input' 'Value'"

        let mut define_str = format!(
            "define auto_discovered_{}:\n  steps:\n",
            pattern.occurrence_count
        );

        // Naive joining
        for step in &pattern.steps {
            define_str.push_str(&format!("    - {}\n", step));
        }

        match parse_define(&define_str) {
            Ok(mut def) => {
                def.description = Some(format!(
                    "Auto-discovered pattern from {} occurrences",
                    pattern.occurrence_count
                ));
                def.tier = IntentTier::Discovered;
                Some(def)
            }
            Err(_) => None, // Parsing failed, maybe invalid steps
        }
    }
}
