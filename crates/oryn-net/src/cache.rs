//! Context-partitioned, bounded HTTP memory cache.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CacheKey {
    pub partition: String,
    pub method: String,
    pub url: String,
    pub credential_partition: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    #[serde(default)]
    pub encoded_body_size: Option<u64>,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub stored_at_ms: u64,
    pub freshness_lifetime_ms: u64,
    pub vary: Vec<(String, String)>,
}

impl CacheEntry {
    pub fn is_fresh(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.stored_at_ms) < self.freshness_lifetime_ms
    }

    pub fn matches_vary(&self, request_headers: &[(String, String)]) -> bool {
        self.vary.iter().all(|(name, expected)| {
            request_headers
                .iter()
                .filter(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
                .collect::<Vec<_>>()
                .join(", ")
                == *expected
        })
    }
}

#[derive(Debug)]
pub struct MemoryCache {
    max_entries: usize,
    max_bytes: usize,
    max_entry_bytes: usize,
    bytes: usize,
    entries: BTreeMap<CacheKey, CacheEntry>,
    order: VecDeque<CacheKey>,
}

impl Default for MemoryCache {
    fn default() -> Self {
        Self::new(256, 32 * 1024 * 1024, 8 * 1024 * 1024)
    }
}

impl MemoryCache {
    pub fn new(max_entries: usize, max_bytes: usize, max_entry_bytes: usize) -> Self {
        Self {
            max_entries,
            max_bytes,
            max_entry_bytes,
            bytes: 0,
            entries: BTreeMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn get(
        &mut self,
        key: &CacheKey,
        request_headers: &[(String, String)],
        now_ms: u64,
    ) -> Option<&CacheEntry> {
        self.get_matching(key, request_headers, Some(now_ms))
    }

    /// Stale representations retain validators for native conditional requests.
    pub fn get_matching(
        &mut self,
        key: &CacheKey,
        request_headers: &[(String, String)],
        fresh_at: Option<u64>,
    ) -> Option<&CacheEntry> {
        let usable = self.entries.get(key).is_some_and(|entry| {
            fresh_at.is_none_or(|now| entry.is_fresh(now)) && entry.matches_vary(request_headers)
        });
        if !usable {
            return None;
        }
        self.order.retain(|candidate| candidate != key);
        self.order.push_back(key.clone());
        self.entries.get(key)
    }

    pub fn insert(&mut self, key: CacheKey, entry: CacheEntry) -> bool {
        if key.method != "GET"
            || entry.body.len() > self.max_entry_bytes
            || self.max_entries == 0
            || self.max_bytes == 0
            || !cacheable_status(entry.status, &entry.headers)
            || has_directive(&entry.headers, "no-store")
            || entry.vary.iter().any(|(name, _)| name == "*")
        {
            return false;
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(previous.body.len());
            self.order.retain(|candidate| candidate != &key);
        }
        while self.entries.len() >= self.max_entries
            || self.bytes.saturating_add(entry.body.len()) > self.max_bytes
        {
            let Some(oldest) = self.order.pop_front() else {
                return false;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(removed.body.len());
            }
        }
        self.bytes = self.bytes.saturating_add(entry.body.len());
        self.order.push_back(key.clone());
        self.entries.insert(key, entry);
        true
    }

    pub fn invalidate_url(&mut self, partition: &str, url: &str) {
        let keys = self
            .entries
            .keys()
            .filter(|key| key.partition == partition && key.url == url)
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes = self.bytes.saturating_sub(entry.body.len());
            }
            self.order.retain(|candidate| candidate != &key);
        }
    }

    pub const fn bytes(&self) -> usize {
        self.bytes
    }
}

fn has_directive(headers: &[(String, String)], directive: &str) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("cache-control")
            && value
                .split(',')
                .any(|item| item.trim().eq_ignore_ascii_case(directive))
    })
}

fn cacheable_status(status: u16, headers: &[(String, String)]) -> bool {
    if !(200..=599).contains(&status) || status == 304 {
        return false;
    }
    // The default status list grants heuristic caching, not an exclusive
    // whitelist. Explicit freshness or public permits other final statuses.
    matches!(
        status,
        200 | 203 | 204 | 206 | 300 | 301 | 308 | 404 | 405 | 410 | 414 | 501
    ) || has_directive(headers, "public")
        || headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("expires")
                || (name.eq_ignore_ascii_case("cache-control")
                    && value.split(',').any(|part| {
                        part.trim().split_once('=').is_some_and(|(name, value)| {
                            name.eq_ignore_ascii_case("max-age")
                                && value.trim().trim_matches('"').parse::<u64>().is_ok()
                        })
                    }))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(url: &str) -> CacheKey {
        CacheKey {
            partition: "context-1".into(),
            method: "GET".into(),
            url: url.into(),
            credential_partition: "include".into(),
        }
    }

    fn entry(body: &[u8]) -> CacheEntry {
        CacheEntry {
            encoded_body_size: None,
            status: 200,
            headers: vec![],
            body: body.into(),
            stored_at_ms: 100,
            freshness_lifetime_ms: 1_000,
            vary: vec![],
        }
    }

    #[test]
    fn explicit_cache_permission_extends_default_statuses_without_overriding_no_store() {
        for status in [204, 299, 400, 499, 500, 502, 503, 504, 599] {
            for permission in ["max-age=3600", "public"] {
                let mut cache = MemoryCache::default();
                let mut value = entry(b"cached");
                value.status = status;
                value.headers = vec![("Cache-Control".into(), permission.into())];
                assert!(
                    cache.insert(key("https://a.test/status"), value.clone()),
                    "{status} {permission}"
                );
                value
                    .headers
                    .push(("cache-control".into(), "no-store".into()));
                assert!(!cache.insert(key("https://a.test/denied"), value));
            }
        }
        for (status, permission) in [
            (299, ""),
            (500, "max-age=invalid"),
            (101, "public"),
            (304, "max-age=3600"),
        ] {
            let mut cache = MemoryCache::default();
            let mut value = entry(b"denied");
            value.status = status;
            value.headers = vec![("cache-control".into(), permission.into())];
            assert!(!cache.insert(key("https://a.test/denied"), value));
        }
    }

    #[test]
    fn cache_is_bounded_and_partitioned() {
        let mut cache = MemoryCache::new(2, 5, 4);
        assert!(cache.insert(key("https://a.test/1"), entry(b"12")));
        assert!(cache.insert(key("https://a.test/2"), entry(b"34")));
        assert!(cache.insert(key("https://a.test/3"), entry(b"5")));
        assert!(cache.get(&key("https://a.test/1"), &[], 101).is_none());
        assert_eq!(cache.bytes(), 3);
    }

    #[test]
    fn no_store_and_stale_entries_are_not_reused() {
        let mut cache = MemoryCache::default();
        let mut denied = entry(b"private");
        denied
            .headers
            .push(("cache-control".into(), "no-store".into()));
        assert!(!cache.insert(key("https://a.test/private"), denied));
        assert!(cache.insert(key("https://a.test/fresh"), entry(b"ok")));
        assert!(
            cache
                .get(&key("https://a.test/fresh"), &[], 2_000)
                .is_none()
        );
    }
}
