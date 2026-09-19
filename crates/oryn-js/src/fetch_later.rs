//! Protocol-neutral deferred Fetch capability. Admission freezes requester
//! authority in the native backend before the source Document can retire.
use crate::platform::PlatformHttpRequest;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
// First eight bytes: nonzero per-Document monotonic sequence, big endian.
// Remaining 24 bytes: native cryptographic entropy. Sequence is consumed even
// when admission fails, allowing bounded backend replay tombstones.
pub struct DeferredFetchToken(pub Vec<u8>);
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeferredFetchError {
    Quota(String),
    Type(String),
    Transport(String),
}
impl std::fmt::Display for DeferredFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Quota(message) | Self::Type(message) | Self::Transport(message) => {
                f.write_str(message)
            }
        }
    }
}
impl std::error::Error for DeferredFetchError {}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeferredFetchState {
    Pending,
    Activated,
    Canceled,
}
pub trait DeferredFetchBackend {
    /// Exact replay of the same capability/request is idempotent. Different
    /// request bytes under an existing capability must be rejected.
    fn prepare(
        &self,
        token: &DeferredFetchToken,
        request: PlatformHttpRequest,
        activate_after: Option<std::time::Duration>,
    ) -> Result<(), DeferredFetchError>;
    /// Activation is irrevocable. Retry must never issue a second HTTP request.
    fn activate(
        &self,
        token: &DeferredFetchToken,
    ) -> Result<DeferredFetchState, DeferredFetchError>;
    /// Cancellation of an already activated capability is a successful no-op.
    fn cancel(&self, token: &DeferredFetchToken) -> Result<DeferredFetchState, DeferredFetchError>;
    fn state(&self, token: &DeferredFetchToken) -> Result<DeferredFetchState, DeferredFetchError>;
    /// Release native terminal-result custody after durable local observation.
    fn acknowledge(&self, token: &DeferredFetchToken) -> Result<(), DeferredFetchError>;
}
pub fn request_bytes(request: &PlatformHttpRequest) -> Option<usize> {
    request.headers.iter().try_fold(
        request
            .url
            .split('#')
            .next()
            .unwrap_or("")
            .len()
            .checked_add(request.body.len())?,
        |bytes, (name, value)| bytes.checked_add(name.len())?.checked_add(value.len()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_bytes_exclude_fragment_and_include_each_header_and_body_byte() {
        let request = PlatformHttpRequest {
            upgrade_mixed_image: false,
            fetch: None,
            use_cors_preflight: false,
            method: "POST".into(),
            url: "https://a.test/p#fragment".into(),
            headers: vec![("x".into(), "yz".into())],
            body: vec![0, 1],
            credentials: "include".into(),
            initiator_url: None,
            referrer_policy: None,
        };
        assert_eq!(
            request_bytes(&request),
            Some("https://a.test/p".len() + 3 + 2)
        );
    }
}
