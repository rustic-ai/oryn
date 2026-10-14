//! Parent-owned response evidence; this is not authorization to start a fetch.
use crate::{agent_registry::AgentOwner, runtime::RuntimeError};
use sha2::Digest;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StylesheetResponseProof {
    pub id: u64,
    pub origin_clean: bool,
    pub body_sha256: [u8; 32],
}
pub struct ResponseGuard {
    client: crate::context_service::SharedContextClient,
    page: oryn_common::v3::PageId,
    creator: oryn_js::worker_wire::Creator,
    pub proof: StylesheetResponseProof,
    _cleanup: crate::context_service::script_cleanup_custody::Ticket,
}
/// Reserve before any request which can publish parent response evidence.
pub struct PendingResponse {
    client: crate::context_service::SharedContextClient,
    page: oryn_common::v3::PageId,
    creator: oryn_js::worker_wire::Creator,
    cleanup: crate::context_service::script_cleanup_custody::Ticket,
}
#[cfg(any(test, feature = "v8-host"))]
impl PendingResponse {
    pub fn reserve(
        client: crate::context_service::SharedContextClient,
        page: oryn_common::v3::PageId,
        creator: oryn_js::worker_wire::Creator,
    ) -> Result<Self, RuntimeError> {
        let cleanup =
            crate::context_service::script_cleanup_custody::Ticket::reserve(client.clone(), page)
                .map_err(RuntimeError::ContextService)?;
        Ok(Self {
            client,
            page,
            creator,
            cleanup,
        })
    }
    pub fn accept(self, proof: StylesheetResponseProof) -> ResponseGuard {
        self.cleanup.attach_request(
            crate::context_service::ContextServiceRequest::ReleaseStylesheetResponse {
                page_id: self.page,
                creator: self.creator.clone(),
                id: proof.id,
            },
        );
        ResponseGuard {
            client: self.client,
            page: self.page,
            creator: self.creator,
            proof,
            _cleanup: self.cleanup,
        }
    }
}
impl ResponseGuard {
    /// Owner cancellation starts native custody cleanup without blocking the
    /// caller. Publication must also test the owning job's local cancel flag.
    pub fn cancel(&self) {
        self._cleanup.cancel();
    }
    pub fn validate(&self, url: &str, source: &str) -> Result<Option<bool>, RuntimeError> {
        let body_sha256 = sha2::Sha256::digest(source.as_bytes()).into();
        match self.client.request(
            crate::context_service::ContextServiceRequest::ValidateStylesheetResponse {
                page_id: self.page,
                creator: self.creator.clone(),
                id: self.proof.id,
                url: url.into(),
                body_sha256,
            },
        )? {
            crate::context_service::ContextServiceResponse::StylesheetResponseValidated {
                origin_clean,
            } => Ok(origin_clean),
            _ => Err(failure("invalid stylesheet response validation reply")),
        }
    }
}
fn failure(message: &str) -> RuntimeError {
    RuntimeError::ProtocolViolation(message.into())
}
pub(crate) fn issued_owner(
    page: oryn_common::v3::PageId,
    creator: &oryn_js::worker_wire::Creator,
) -> Result<AgentOwner, RuntimeError> {
    match creator {
        oryn_js::worker_wire::Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } => Ok(AgentOwner::Window {
            page,
            target: target.clone(),
            generation: *document_generation,
            epoch: *stream_epoch,
        }),
        _ => Err(failure("stylesheet response cleanup requires window owner")),
    }
}
/// Used only after successful parent CORS checks; redirect taint is accumulated.
pub(crate) fn http_origin_clean(cors_mode: bool, redirect_origin_clean: bool) -> bool {
    cors_mode || redirect_origin_clean
}
