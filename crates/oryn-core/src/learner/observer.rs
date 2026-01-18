use crate::command::Command;
use crate::config::schema::SecurityConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    pub command: Command,
    pub domain: String,
    pub timestamp: u64,
}

pub struct Observer {
    history: HashMap<String, Vec<ActionRecord>>, // domain -> history
    #[allow(dead_code)]
    security_config: SecurityConfig,
}

impl Observer {
    pub fn new(security_config: SecurityConfig) -> Self {
        Self {
            history: HashMap::new(),
            security_config,
        }
    }

    pub fn observe(&mut self, command: Command, domain: String) {
        // Filter out sensitive commands before recording if needed?
        // For now, we assume executed commands are safe, but we should probably
        // rely on mask_sensitive logic broadly.
        // However, the learner needs raw structure.
        // We will skip recording entirely if it seems sensitive or specific types.

        // Skip purely observational commands
        if self.should_ignore(&command) {
            return;
        }

        let record = ActionRecord {
            command,
            domain: domain.clone(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };

        self.history.entry(domain).or_default().push(record);
    }

    pub fn get_history(&self, domain: &str) -> Option<&Vec<ActionRecord>> {
        self.history.get(domain)
    }

    pub fn clear_history(&mut self, domain: &str) {
        self.history.remove(domain);
    }

    fn should_ignore(&self, command: &Command) -> bool {
        match command {
            // Ignore navigation/observation, focus on actions
            Command::Observe(_)
            | Command::Html(_)
            | Command::Text(_)
            | Command::Screenshot(_)
            | Command::Title
            | Command::Url
            | Command::Wait(_, _)
            | Command::Extract(_)
            | Command::Cookies(_)
            | Command::Storage(_)
            | Command::Tabs(_)
            | Command::Packs
            | Command::PackLoad(_)
            | Command::PackUnload(_)
            | Command::Intents(_)
            | Command::Define(_)
            | Command::Undefine(_)
            | Command::Export(_, _)
            | Command::RunIntent(_, _)
            | Command::Pdf(_)
            | Command::Learn(_) => true,

            // Keep actions
            Command::Click(_, _)
            | Command::Type(_, _, _)
            | Command::Clear(_)
            | Command::Press(_, _)
            | Command::Select(_, _)
            | Command::Check(_)
            | Command::Uncheck(_)
            | Command::Hover(_)
            | Command::Focus(_)
            | Command::Scroll(_, _)
            | Command::Submit(_)
            | Command::Login(_, _, _)
            | Command::Search(_, _)
            | Command::Dismiss(_, _)
            | Command::Accept(_, _)
            | Command::ScrollUntil(_, _, _) => false,

            // Navigation is needed for context switching, but usually we learn intents *within* a page.
            // Let's ignore GoTo for "intent" learning for now (assuming intents are page-local or specific flows).
            Command::GoTo(_) | Command::Back | Command::Forward | Command::Refresh(_) => true,
        }
    }
}
