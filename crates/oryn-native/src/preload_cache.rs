//! Single-use HTML preload responses, separate from the shared HTTP cache.
//! Entries never cross an authenticated Document lifetime or request mode.
use oryn_common::v3::PageId;
use oryn_js::worker_wire::{AgentFetchResponse, Creator};
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    pub page: PageId,
    pub creator: Creator,
    pub url: String,
    pub cors: bool,
    pub isolation: crate::agent_isolation::DocumentIsolationPolicy,
    pub credentials: String,
    pub integrity: String,
    pub range: Option<String>,
}
#[derive(Clone)]
pub(crate) struct Response {
    pub timing_allowed: bool,
    pub response: AgentFetchResponse,
    pub hops: Vec<String>,
}
struct Stored {
    key: Key,
    response: Response,
    expires: Instant,
}
#[derive(Default)]
pub(crate) struct Cache {
    entries: Mutex<VecDeque<Stored>>,
}
impl Cache {
    pub(crate) fn insert(&self, key: Key, response: Response) {
        if key.url.len() > 64 * 1024
            || response.response.body.len() > 16 * 1024 * 1024
            || response.hops.len() > 21
            || response.hops.iter().map(String::len).sum::<usize>() > 256 * 1024
            || response
                .response
                .headers
                .iter()
                .map(|(n, v)| n.len() + v.len())
                .sum::<usize>()
                > 256 * 1024
        {
            return;
        }
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|e| e.expires > Instant::now() && e.key != key);
        while entries.len() >= 64
            || entries
                .iter()
                .map(|e| e.response.response.body.len())
                .sum::<usize>()
                .saturating_add(response.response.body.len())
                > 64 * 1024 * 1024
        {
            entries.pop_front();
        }
        entries.push_back(Stored {
            key,
            response,
            expires: Instant::now() + Duration::from_secs(30),
        });
    }
    pub(crate) fn take(&self, key: &Key) -> Option<Response> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|e| e.expires > Instant::now());
        let index = entries.iter().position(|e| &e.key == key)?;
        Some(entries.remove(index)?.response)
    }
    pub(crate) fn clear(&self) {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}
pub(crate) struct Candidate {
    pub key: Key,
    pub response: Response,
}
#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::message_wire::WindowTarget;
    fn key() -> Key {
        Key {
            page: PageId(1),
            creator: Creator::Window {
                target: WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
            url: "https://example.test/a".into(),
            cors: false,
            isolation: Default::default(),
            credentials: "include".into(),
            integrity: String::new(),
            range: None,
        }
    }
    fn response() -> Response {
        Response {
            timing_allowed: false,
            response: AgentFetchResponse {
                timing: None,
                redirected: true,
                range_requested: false,
                status: 200,
                status_text: String::new(),
                headers: Vec::new(),
                body: b"image".to_vec(),
                response_url: "https://cdn.test/a".into(),
                response_type: "opaque".into(),
            },
            hops: vec!["https://example.test/a".into(), "https://cdn.test/a".into()],
        }
    }
    #[test]
    fn preload_reuse_requires_exact_document_cors_credentials_integrity_and_url() {
        let cache = Cache::default();
        let original = key();
        cache.insert(original.clone(), response());
        for changed in 0..8 {
            let mut other = original.clone();
            match changed {
                0 => other.page = PageId(2),
                1 => {
                    other.creator = Creator::Window {
                        target: WindowTarget::Root,
                        document_generation: 2,
                        stream_epoch: 0,
                    }
                }
                2 => other.cors = true,
                3 => other.credentials = "omit".into(),
                4 => other.integrity = "sha256-other".into(),
                5 => other.url.push_str("?other"),
                6 => other.range = Some("bytes=0-".into()),
                _ => {
                    other.isolation.embedder =
                        crate::agent_isolation::EmbedderPolicy::Credentialless
                }
            };
            assert!(cache.take(&other).is_none());
        }
        let taken = cache.take(&original).unwrap();
        assert_eq!(taken.response.response_type, "opaque");
        assert_eq!(taken.hops.len(), 2);
        assert!(cache.take(&original).is_none());
    }
    #[test]
    fn expired_preloads_cannot_be_reused() {
        let cache = Cache::default();
        cache.insert(key(), response());
        cache.entries.lock().unwrap()[0].expires = Instant::now();
        assert!(cache.take(&key()).is_none());
    }
}
