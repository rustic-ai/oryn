use super::storage::ObservationStorage;
use super::{LearningConfig, SessionLog};
use std::time::SystemTime;

pub struct Observer {
    storage: ObservationStorage,
    config: LearningConfig,
}

impl Observer {
    pub fn new(config: LearningConfig, storage: ObservationStorage) -> Self {
        Self { storage, config }
    }

    pub fn record(&self, domain: &str, url: &str, command: &str) {
        if !self.config.enabled {
            return;
        }

        let log = SessionLog {
            timestamp: SystemTime::now(),
            domain: domain.to_string(),
            url: url.to_string(),
            command: command.to_string(),
            input_snapshot: None, // snapshot capture not implemented yet
        };

        self.storage.record(log);
    }

    pub fn get_storage(&self) -> &ObservationStorage {
        &self.storage
    }

    pub fn get_history(&self, domain: &str) -> Vec<SessionLog> {
        self.storage.get_history(domain)
    }
}
