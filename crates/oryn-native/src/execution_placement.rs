//! Trusted parent diagnostics. No transferable capabilities or virtual addresses.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionPlacement {
    pub mode: String,
    pub process_id: u32,
    pub attested: bool,
    pub binary_sha256: Option<String>,
    pub bootstrap_sha256: Option<String>,
}
impl ExecutionPlacement {
    pub(crate) fn thread() -> Self {
        Self {
            mode: "in_process_thread".into(),
            process_id: std::process::id(),
            attested: false,
            binary_sha256: None,
            bootstrap_sha256: None,
        }
    }
}
pub(crate) fn owner(owner: &crate::agent_registry::AgentOwner) -> serde_json::Value {
    use crate::agent_registry::AgentOwner;
    match owner {
        AgentOwner::Agent(id) => serde_json::json!({"kind":"agent","id":id.0}),
        AgentOwner::Window {
            page,
            target,
            generation,
            epoch,
        } => {
            let frame = match target {
                oryn_js::message_wire::WindowTarget::Root => None,
                oryn_js::message_wire::WindowTarget::Frame(token) => {
                    Some(fingerprint(token.as_bytes()))
                }
            };
            serde_json::json!({"kind":"window","id":page.0,"frame":frame,"document_generation":generation,"stream_epoch":epoch})
        }
    }
}
pub(crate) fn fingerprint(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
