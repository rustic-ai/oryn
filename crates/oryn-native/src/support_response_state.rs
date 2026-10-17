//! Fixed metadata stored inside an already admitted parent response occurrence.
//! No URL/String/Creator copy or independent evidence registry/capacity.
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SupportOwner {
    pub page: u64,
    /// Parent-issued occurrence identity, never a worker-submitted V8 instance.
    pub occurrence: u64,
    pub document_generation: u64,
    pub stream_epoch: u64,
    pub authority_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SupportReceipt {
    pub descriptor: [u8; 32],
    pub candidate: [u8; 32],
    pub parent: SupportOwner,
    pub child: SupportOwner,
    pub url_sha256: [u8; 32],
    pub body_sha256: [u8; 32],
    pub body_bytes: u64,
}

/// Created only after matching a coordinator-installed pinned descriptor.
/// The enclosing resource slot owns admission and its sticky cancellation flag.
pub(crate) struct SupportResponseState {
    descriptor: [u8; 32],
    candidate: [u8; 32],
    parent: SupportOwner,
    child: Option<SupportOwner>,
    url_sha256: [u8; 32],
    expected_body: [u8; 32],
    digest: Sha256,
    bytes: u64,
    terminal: Option<SupportReceipt>,
    cancelled: bool,
}

impl SupportResponseState {
    pub(crate) fn new(descriptor: [u8; 32], candidate: [u8; 32],
        parent: SupportOwner, url: &str, expected_body: [u8; 32]) -> Self {
        Self { descriptor, candidate, parent, child: None,
            url_sha256: Sha256::digest(url.as_bytes()).into(), expected_body,
            digest: Sha256::new(), bytes: 0, terminal: None, cancelled: false }
    }
    pub(crate) fn bind(&mut self, parent: SupportOwner, child: SupportOwner) -> bool {
        if self.cancelled || self.terminal.is_some() || self.child.is_some()
            || self.parent != parent || parent.page != child.page { return false; }
        self.child = Some(child);
        true
    }
    pub(crate) fn chunk(&mut self, bytes: &[u8], limit: usize) -> bool {
        if self.cancelled || self.child.is_none() || self.terminal.is_some() { return false; }
        let Some(next) = self.bytes.checked_add(bytes.len() as u64) else { self.cancel(); return false; };
        if next > limit as u64 { self.cancel(); return false; }
        self.digest.update(bytes);
        self.bytes = next;
        true
    }
    /// Caller holds current-parent/current-child validation through publication.
    pub(crate) fn finish(&mut self, parent: SupportOwner, child: SupportOwner) -> bool {
        if self.cancelled || self.terminal.is_some() || parent != self.parent
            || self.child != Some(child) { return false; }
        let body_sha256: [u8; 32] = self.digest.clone().finalize().into();
        if body_sha256 != self.expected_body { self.cancel(); return false; }
        self.terminal = Some(SupportReceipt { descriptor: self.descriptor,
            candidate: self.candidate, parent, child, url_sha256: self.url_sha256,
            body_sha256, body_bytes: self.bytes });
        true
    }
    pub(crate) fn cancel(&mut self) { self.cancelled = true; self.terminal = None; }
    /// Call only under the same current-owner validation used for publication.
    pub(crate) fn read(&self, parent: SupportOwner, child: SupportOwner) -> Option<SupportReceipt> {
        (!self.cancelled && self.parent == parent && self.child == Some(child))
            .then_some(self.terminal).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner(instance: u64) -> SupportOwner {
        SupportOwner { page: 1, occurrence: instance, document_generation: instance,
            stream_epoch: 0, authority_epoch: 4 }
    }
    #[test]
    fn exact_support_occurrence_terminal_and_sticky_retirement() {
        let mut state = SupportResponseState::new([1;32], [2;32], owner(1),
            "https://fixture.test/child", Sha256::digest(b"abc").into());
        assert!(!state.chunk(b"unbound", 3));
        assert!(state.bind(owner(1), owner(2)));
        assert!(!state.bind(owner(1), owner(2)));
        assert!(state.chunk(b"ab", 3)); assert!(state.chunk(b"c", 3));
        assert!(!state.finish(owner(1), owner(3)));
        assert!(state.finish(owner(1), owner(2)));
        assert!(!state.finish(owner(1), owner(2)));
        assert!(state.read(owner(1), owner(2)).is_some());
        assert!(state.read(owner(1), owner(3)).is_none());
        state.cancel(); assert!(state.read(owner(1), owner(2)).is_none());
        assert!(!state.bind(owner(1), owner(2)));
    }
}
