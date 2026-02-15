use super::SessionLog;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct ObservationStorage {
    // In-memory storage for MVP. In reality, this would be a database or file-backed.
    logs: Arc<Mutex<HashMap<String, Vec<SessionLog>>>>,
}

impl ObservationStorage {
    pub fn new() -> Self {
        Self {
            logs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn record(&self, log: SessionLog) {
        let mut logs = self.logs.lock().unwrap();
        logs.entry(log.domain.clone()).or_default().push(log);
    }

    pub fn get_history(&self, domain: &str) -> Vec<SessionLog> {
        let logs = self.logs.lock().unwrap();
        logs.get(domain).cloned().unwrap_or_default()
    }
}
