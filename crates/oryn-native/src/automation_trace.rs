//! Bounded, page-isolate-owned automation observations. Realm identity comes
//! from the host, never from the script-supplied event payload.
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize)]
pub(crate) struct Event {
    sequence: usize,
    context_id: u32,
    document_generation: u64,
    id: u64,
    operation: String,
    outcome: String,
    backend: &'static str,
}

#[derive(Default)]
pub(crate) struct AutomationTrace {
    events: Vec<Event>,
    operations: BTreeMap<(u32, u64, u64), (String, bool)>,
    invalid: bool,
    overflow: bool,
}

impl AutomationTrace {
    pub fn record(
        &mut self,
        context: u32,
        generation: u64,
        id: u64,
        operation: &str,
        outcome: &str,
    ) -> bool {
        if self.events.len() >= 4096 {
            self.overflow = true;
            return false;
        }
        let key = (context, generation, id);
        let valid = id > 0
            && !operation.is_empty()
            && operation.len() <= 96
            && operation
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            && match outcome {
                "started" => !self.operations.contains_key(&key),
                "completed" | "unsupported" | "failed" => self
                    .operations
                    .get(&key)
                    .is_some_and(|(name, terminal)| name == operation && !terminal),
                _ => false,
            };
        if !valid {
            self.invalid = true;
            return false;
        }
        self.operations
            .insert(key, (operation.into(), outcome != "started"));
        self.events.push(Event {
            sequence: self.events.len(),
            context_id: context,
            document_generation: generation,
            id,
            operation: operation.into(),
            outcome: outcome.into(),
            backend: "oryn",
        });
        true
    }

    pub fn invalidate(&mut self) {
        self.invalid = true;
    }

    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"schema_version":1, "scope":"page_isolate",
            "events":self.events, "overflow":self.overflow,
            "complete":!self.invalid && !self.overflow && self.operations.values().all(|(_, terminal)| *terminal)})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn realm_ids_disambiguate_operations_and_invalid_transitions_fail_closed() {
        let mut trace = AutomationTrace::default();
        for realm in [0, 1] {
            assert!(trace.record(realm, 1, 1, "click", "started"));
        }
        assert_eq!(trace.snapshot()["complete"], false);
        for realm in [1, 0] {
            assert!(trace.record(realm, 1, 1, "click", "completed"));
        }
        assert_eq!(trace.snapshot()["complete"], true);
        assert!(!trace.record(0, 1, 1, "click", "completed"));
        assert_eq!(trace.snapshot()["complete"], false);
        assert!(
            AutomationTrace::default().snapshot()["events"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn observations_are_bounded_and_navigation_generations_are_distinct() {
        let mut trace = AutomationTrace::default();
        for generation in 0..2048 {
            assert!(trace.record(0, generation, 1, "click", "started"));
            assert!(trace.record(0, generation, 1, "click", "completed"));
        }
        assert!(!trace.record(0, 2048, 1, "click", "started"));
        assert_eq!(trace.snapshot()["overflow"], true);
        assert_eq!(trace.snapshot()["events"].as_array().unwrap().len(), 4096);
    }
}
