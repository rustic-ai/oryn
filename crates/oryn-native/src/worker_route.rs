//! Parent-issued authority for an execution environment's broker binding.
//!
//! IDs describe a destination; the random grant authorizes it. Grants stay in
//! native transport objects, never in page globals or diagnostic formatting.

use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerRoute {
    pub process_session: u64,
    pub environment: u64,
    pub generation: u64,
    grant: [u8; 32],
}

impl WorkerRoute {
    pub(crate) fn issue(
        process_session: u64,
        environment: u64,
        generation: u64,
    ) -> Result<Self, crate::runtime::RuntimeError> {
        // Environment zero is the existing parent's prewarm-only LEGACY page.
        // It still requires a fresh grant and is reset before a user page binds.
        if process_session == 0 {
            return Err(crate::runtime::RuntimeError::ProtocolViolation(
                "invalid route identity".into(),
            ));
        }
        let mut grant = [0; 32];
        getrandom::fill(&mut grant).map_err(|error| {
            crate::runtime::RuntimeError::SandboxUnavailable(format!("route entropy: {error}"))
        })?;
        Ok(Self {
            process_session,
            environment,
            generation,
            grant,
        })
    }
}

impl std::fmt::Debug for WorkerRoute {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkerRoute")
            .field("process_session", &self.process_session)
            .field("environment", &self.environment)
            .field("generation", &self.generation)
            .field("grant", &"[redacted]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authority_is_not_reconstructed_from_identifiers() {
        let first = WorkerRoute::issue(1, 1, 0).unwrap();
        let second = WorkerRoute::issue(1, 1, 0).unwrap();
        assert_ne!(first, second);
        assert!(format!("{first:?}").contains("[redacted]"));
        let encoded = serde_json::to_string(&first).unwrap();
        assert_eq!(
            serde_json::from_str::<WorkerRoute>(&encoded).unwrap(),
            first
        );
        assert!(WorkerRoute::issue(0, 1, 0).is_err());
        assert!(WorkerRoute::issue(1, 0, 0).is_ok());
    }
}
