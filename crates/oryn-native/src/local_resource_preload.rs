//! Standalone native preload response custody. Each store belongs to one
//! document report queue; neither URL strings nor authored properties select it.
use crate::host::HostError;
use crate::script_load_failure::ScriptLoadFailure;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Key {
    pub destination: &'static str,
    pub url: String,
    pub policy: Option<String>,
    pub initiator: String,
    pub referrer_policy: String,
    pub encoding: String,
    pub nonce: Option<String>,
    pub integrity: Option<String>,
    pub cross_origin: Option<String>,
    pub parser_inserted: bool,
    pub quirks: bool,
}
impl Key {
    fn same_resource(&self, other: &Self) -> bool {
        let mut first = self.clone();
        let mut second = other.clone();
        first.policy = None;
        second.policy = None;
        first.parser_inserted = false;
        second.parser_inserted = false;
        first == second
    }
    fn normalize(mut self) -> Self {
        if let Ok(mut url) = url::Url::parse(&self.url) {
            url.set_fragment(None);
            self.url = url.into();
        }
        if let Some(value) = &mut self.cross_origin {
            *value = if value.eq_ignore_ascii_case("use-credentials") {
                "use-credentials"
            } else {
                "anonymous"
            }
            .into();
        }
        self
    }
    fn bytes(&self) -> usize {
        self.url.len()
            + self.policy.as_ref().map_or(0, String::len)
            + self.initiator.len()
            + self.referrer_policy.len()
            + self.encoding.len()
            + self.nonce.as_ref().map_or(0, String::len)
            + self.integrity.as_ref().map_or(0, String::len)
            + 128
    }
}
#[derive(Clone)]
pub(crate) enum Response {
    Script(String, String, bool, String),
    Style(String, String, String, String),
}
impl Response {
    fn bytes(&self) -> usize {
        match self {
            Self::Script(url, source, _, policy) => url.len() + source.len() + policy.len(),
            Self::Style(url, source, encoding, policy) => {
                url.len() + source.len() + encoding.len() + policy.len()
            }
        }
    }
}
pub(crate) struct Entry {
    key: Key,
    pub status: crate::render_request_status::Status,
    pub cancel: Arc<AtomicBool>,
    started: AtomicBool,
    claimed: AtomicBool,
    trace: Mutex<Vec<String>>,
    hash_evidence: Mutex<Option<crate::csp::ScriptHashEvidence>>,
    hash_consumption: Mutex<Option<[u8; 32]>>,
    expires: Instant,
    result: Mutex<Option<Result<Response, ScriptLoadFailure>>>,
    changed: Condvar,
    bytes: AtomicUsize,
    budget: Arc<AtomicUsize>,
}
impl Entry {
    pub(crate) fn same_request(&self, key: &Key) -> bool {
        self.key == key.clone().normalize()
    }
    pub(crate) fn trace(&self) -> Vec<String> {
        self.trace.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub(crate) fn set_trace(&self, trace: Vec<String>) {
        if trace.len() <= 11 && trace.iter().map(String::len).sum::<usize>() <= 256 * 1024 {
            *self.trace.lock().unwrap_or_else(|e| e.into_inner()) = trace;
        }
    }
    /// Called before complete, from the successful raw-byte producer. The first
    /// response owns evidence; neither decoded source nor later policy replaces it.
    pub(crate) fn set_hash_evidence(&self, evidence: crate::csp::ScriptHashEvidence) {
        let result = self.result.lock().unwrap_or_else(|e| e.into_inner());
        if result.is_some() || self.cancelled() {
            return;
        }
        let mut slot = self.hash_evidence.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(evidence);
        }
    }
    pub(crate) fn publish_hash_once(
        &self,
        key: &Key,
        publish: impl FnOnce(&crate::csp::ScriptHashEvidence) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        self.publish_consumption_once(key, |evidence| match evidence {
            Some(evidence) => publish(evidence),
            None => Ok(()),
        })
    }
    pub(crate) fn publish_consumption_once(
        &self,
        key: &Key,
        publish: impl FnOnce(Option<&crate::csp::ScriptHashEvidence>) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        if self.cancelled() {
            return Err(HostError::ScriptLoad(ScriptLoadFailure::Cancelled));
        }
        if !matches!(
            self.result
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref(),
            Some(Ok(_))
        ) {
            return Err(HostError::ScriptLoad(ScriptLoadFailure::Infrastructure(
                "preload response is not successfully complete".into(),
            )));
        }
        use sha2::Digest;
        let bytes = serde_json::to_vec(&key.clone().normalize()).map_err(|error| {
            HostError::ScriptLoad(ScriptLoadFailure::Infrastructure(error.to_string()))
        })?;
        let identity: [u8; 32] = sha2::Sha256::digest(&bytes).into();
        let mut receipt = self
            .hash_consumption
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = receipt.as_ref() {
            return if previous == &identity {
                Ok(())
            } else {
                Err(HostError::ScriptLoad(ScriptLoadFailure::Infrastructure(
                    "preload response already consumed by another request".into(),
                )))
            };
        }
        publish(
            self.hash_evidence
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref(),
        )?;
        *receipt = Some(identity);
        Ok(())
    }
    pub(crate) fn start(&self) -> bool {
        !self.started.swap(true, Ordering::AcqRel)
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire) || Instant::now() >= self.expires
    }
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.changed.notify_all();
    }
    pub(crate) fn complete(&self, result: Result<Response, HostError>) {
        self.status.finish();
        if self.cancelled() {
            self.cancel();
            return;
        }
        let mut slot = self.result.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return;
        }
        let result = result.map_err(|error| match error {
            HostError::ScriptLoad(failure) => failure,
            HostError::LimitExceeded { resource } => ScriptLoadFailure::Capacity(resource),
            other if self.key.destination == "stylesheet" => {
                ScriptLoadFailure::Fetch(other.to_string())
            }
            other => ScriptLoadFailure::Infrastructure(other.to_string()),
        });
        let bytes = result
            .as_ref()
            .map_or(0, Response::bytes)
            .saturating_add(32)
            .saturating_add(
                self.hash_evidence
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .map_or(0, |e| e.sha256.len() + e.sha384.len() + e.sha512.len() + 64),
            )
            .saturating_add(
                self.trace
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter()
                    .map(String::len)
                    .sum::<usize>(),
            );
        if self
            .budget
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|sum| *sum <= 32 * 1024 * 1024)
            })
            .is_err()
        {
            *slot = Some(Err(ScriptLoadFailure::Capacity(
                "standalone preload response budget".into(),
            )));
        } else {
            self.bytes.store(bytes, Ordering::Release);
            *slot = Some(result);
        }
        self.changed.notify_all();
    }
    pub(crate) fn result(&self) -> Option<Result<Response, ScriptLoadFailure>> {
        if self.cancelled() {
            self.cancel();
            return Some(Err(ScriptLoadFailure::Cancelled));
        }
        self.result
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub(crate) fn wait(&self, consumer: &AtomicBool) -> Result<Response, HostError> {
        let mut result = self.result.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if consumer.load(Ordering::Acquire) || self.cancelled() {
                return Err(HostError::ScriptLoad(
                    crate::script_load_failure::ScriptLoadFailure::Cancelled,
                ));
            }
            if let Some(result) = &*result {
                return result.clone().map_err(HostError::ScriptLoad);
            }
            let delay = self
                .expires
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(50));
            result = self
                .changed
                .wait_timeout(result, delay)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}
impl Drop for Entry {
    fn drop(&mut self) {
        self.cancel();
        self.budget
            .fetch_sub(self.bytes.load(Ordering::Acquire), Ordering::AcqRel);
    }
}
#[derive(Default)]
pub(crate) struct Store {
    entries: Mutex<VecDeque<Arc<Entry>>>,
    owners: Mutex<Vec<Weak<Entry>>>,
    budget: Arc<AtomicUsize>,
    retired: AtomicBool,
}
impl Store {
    pub(crate) fn admit(&self, key: Key) -> Result<Arc<Entry>, HostError> {
        let key = key.normalize();
        if key.bytes() > 256 * 1024 {
            return Err(HostError::LimitExceeded {
                resource: "standalone preload metadata".into(),
            });
        }
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if self.retired.load(Ordering::Acquire) {
            return Err(HostError::ScriptLoad(
                crate::script_load_failure::ScriptLoadFailure::Cancelled,
            ));
        }
        entries.retain(|entry| !entry.cancelled());
        if let Some(entry) = entries.iter().find(|entry| entry.key == key) {
            return Ok(entry.clone());
        }
        let mut owners = self.owners.lock().unwrap_or_else(|e| e.into_inner());
        owners.retain(|owner| owner.strong_count() != 0);
        if let Some(existing) = owners
            .iter()
            .filter_map(Weak::upgrade)
            .find(|entry| entry.key == key && !entry.cancelled())
        {
            return Ok(existing);
        }
        if entries.len() >= 64 || owners.len() >= 256 {
            return Err(HostError::LimitExceeded {
                resource: "standalone preload ownership".into(),
            });
        }
        let entry = Arc::new(Entry {
            key,
            status: Default::default(),
            cancel: Default::default(),
            started: AtomicBool::new(false),
            claimed: AtomicBool::new(false),
            trace: Mutex::new(Vec::new()),
            hash_evidence: Mutex::new(None),
            hash_consumption: Mutex::new(None),
            expires: Instant::now() + Duration::from_secs(60),
            result: Mutex::new(None),
            changed: Condvar::new(),
            bytes: AtomicUsize::new(0),
            budget: self.budget.clone(),
        });
        owners.push(Arc::downgrade(&entry));
        entries.push_back(entry.clone());
        Ok(entry)
    }
    pub(crate) fn claim(&self, key: Key, blocking: bool) -> Option<Arc<Entry>> {
        let key = key.normalize();
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if self.retired.load(Ordering::Acquire) {
            return None;
        }
        entries.retain(|entry| !entry.cancelled());
        let available = |entry: &&Arc<Entry>| !entry.claimed.load(Ordering::Acquire);
        let entry = entries
            .iter()
            .filter(available)
            .find(|entry| entry.key == key)
            .or_else(|| {
                entries
                    .iter()
                    .filter(available)
                    .find(|entry| entry.key.same_resource(&key))
            })?
            .clone();
        if entry.claimed.swap(true, Ordering::AcqRel) {
            return None;
        }
        if blocking {
            entry.status.promote()
        }
        Some(entry)
    }
    pub(crate) fn finish_claim(&self, entry: &Arc<Entry>, consumed: bool) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if consumed {
            entries.retain(|candidate| !Arc::ptr_eq(candidate, entry));
        } else {
            entry.claimed.store(false, Ordering::Release);
        }
    }
    pub(crate) fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        let retained = {
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            let mut owners = self.owners.lock().unwrap_or_else(|e| e.into_inner());
            let retained = owners.iter().filter_map(Weak::upgrade).collect::<Vec<_>>();
            entries.clear();
            owners.clear();
            retained
        };
        for entry in retained {
            entry.cancel();
        }
    }
    pub(crate) fn snapshot(self: &Arc<Self>) -> Snapshot {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let owners = self.owners.lock().unwrap_or_else(|e| e.into_inner());
        Snapshot {
            store: self.clone(),
            entries: entries
                .iter()
                .map(|entry| (entry.clone(), entry.claimed.load(Ordering::Acquire)))
                .collect(),
            owners: owners.iter().filter_map(Weak::upgrade).collect(),
            restored: AtomicBool::new(false),
        }
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        self.retire();
    }
}
pub(crate) struct Snapshot {
    store: Arc<Store>,
    entries: Vec<(Arc<Entry>, bool)>,
    owners: Vec<Arc<Entry>>,
    restored: AtomicBool,
}
impl Snapshot {
    pub(crate) fn retained_bytes(&self) -> usize {
        self.owners.iter().map(|entry| entry.key.bytes()).sum()
    }
    pub(crate) fn restore(&self) {
        if self.restored.swap(true, Ordering::AcqRel) {
            return;
        }
        let cancelled = {
            let mut entries = self.store.entries.lock().unwrap_or_else(|e| e.into_inner());
            let mut owners = self.store.owners.lock().unwrap_or_else(|e| e.into_inner());
            let cancelled = owners
                .iter()
                .filter_map(Weak::upgrade)
                .filter(|entry| {
                    !self
                        .owners
                        .iter()
                        .any(|baseline| Arc::ptr_eq(entry, baseline))
                })
                .collect::<Vec<_>>();
            entries.clear();
            if !self.store.retired.load(Ordering::Acquire) {
                for (entry, claimed) in &self.entries {
                    if !entry.cancelled() {
                        entry.claimed.store(*claimed, Ordering::Release);
                        entries.push_back(entry.clone());
                    }
                }
            }
            owners.retain(|owner| {
                owner.upgrade().is_some_and(|entry| {
                    self.owners
                        .iter()
                        .any(|baseline| Arc::ptr_eq(&entry, baseline))
                })
            });
            cancelled
        };
        for entry in cancelled {
            entry.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> Key {
        Key {
            destination: "script",
            url: "https://example.test/app.js".into(),
            policy: Some("native captured policy".into()),
            initiator: "https://example.test/".into(),
            referrer_policy: "origin".into(),
            encoding: "windows-1252".into(),
            nonce: None,
            integrity: None,
            cross_origin: None,
            parser_inserted: true,
            quirks: false,
        }
    }
    #[test]
    fn raw_hash_evidence_is_original_response_owned_and_consumption_is_retry_safe() {
        let store = Store::default();
        let entry = store.admit(key()).unwrap();
        let expected = crate::csp::ScriptHashEvidence::from_bytes(&[0xff, 0, 0x80], true);
        entry.set_hash_evidence(expected.clone());
        entry.set_hash_evidence(crate::csp::ScriptHashEvidence::from_bytes(
            b"replacement",
            true,
        ));
        entry.complete(Ok(Response::Script(
            key().url,
            "decoded replacement characters".into(),
            false,
            "origin".into(),
        )));
        let mut calls = 0;
        assert!(
            entry
                .publish_hash_once(&key(), |actual| {
                    assert_eq!(actual, &expected);
                    calls += 1;
                    Err(HostError::ScriptLoad(ScriptLoadFailure::Infrastructure(
                        "queue capacity".into(),
                    )))
                })
                .is_err()
        );
        entry
            .publish_hash_once(&key(), |actual| {
                assert_eq!(actual, &expected);
                calls += 1;
                Ok(())
            })
            .unwrap();
        entry
            .publish_hash_once(&key(), |_| panic!("lost acknowledgement replays report"))
            .unwrap();
        assert_eq!(calls, 2);
        let mut other = key();
        other.parser_inserted = false;
        assert!(
            entry
                .publish_hash_once(&other, |_| panic!("foreign consumption"))
                .is_err()
        );
    }
    #[test]
    fn opaque_hash_evidence_remains_masked_for_changed_consumer_policy() {
        let store = Store::default();
        let entry = store.admit(key()).unwrap();
        entry.set_hash_evidence(crate::csp::ScriptHashEvidence::from_bytes(
            b"private", false,
        ));
        entry.complete(Ok(Response::Script(
            key().url,
            "private".into(),
            true,
            "origin".into(),
        )));
        let mut consumer = key();
        consumer.policy = Some("new report-sha512 policy".into());
        entry
            .publish_hash_once(&consumer, |e| {
                assert!(!e.exposed);
                assert!(e.sha256.is_empty() && e.sha384.is_empty() && e.sha512.is_empty());
                Ok(())
            })
            .unwrap();
    }
    #[test]
    fn original_response_survives_link_drop_but_retires_with_document() {
        let store = Arc::new(Store::default());
        let link = store.admit(key()).unwrap();
        let consumer = store.claim(key(), true).unwrap();
        drop(link);
        consumer.complete(Ok(Response::Script(
            "https://example.test/app.js".into(),
            "original".into(),
            false,
            "origin".into(),
        )));
        assert!(
            matches!(consumer.wait(&AtomicBool::new(false)).unwrap(),Response::Script(_,source,_,_)if source=="original")
        );
        assert!(store.claim(key(), true).is_none());
        store.retire();
        assert!(consumer.wait(&AtomicBool::new(false)).is_err());
    }
    #[test]
    fn repeated_preload_after_consumer_claim_joins_original_without_second_claim() {
        let store = Store::default();
        let producer = store.admit(key()).unwrap();
        assert!(producer.start());
        let consumer = store.claim(key(), true).unwrap();
        let repeated = store.admit(key()).unwrap();
        assert!(Arc::ptr_eq(&producer, &repeated));
        assert!(!repeated.start());
        assert!(store.claim(key(), false).is_none());
        consumer.complete(Ok(Response::Script(
            key().url,
            "body".into(),
            false,
            "origin".into(),
        )));
        assert!(repeated.status.blocking());
    }
    #[test]
    fn exact_consumer_policy_wins_over_older_reuse_candidate() {
        let store = Store::default();
        let older = store.admit(key()).unwrap();
        let mut consumer = key();
        consumer.policy = Some("consumer policy".into());
        consumer.parser_inserted = false;
        let exact = store.admit(consumer.clone()).unwrap();
        exact.complete(Ok(Response::Script(
            consumer.url.clone(),
            "consumer body".into(),
            false,
            "origin".into(),
        )));
        let selected = store.claim(consumer.clone(), true).unwrap();
        assert!(Arc::ptr_eq(&selected, &exact));
        assert!(!older.status.blocking());
        assert!(
            matches!(selected.wait(&AtomicBool::new(false)).unwrap(),Response::Script(_,source,_,_)if source=="consumer body")
        );
        store.finish_claim(&selected, false);
        assert!(Arc::ptr_eq(
            &store.claim(consumer.clone(), false).unwrap(),
            &exact
        ));
        store.finish_claim(&selected, true);
        let fallback = store.claim(consumer.clone(), false).unwrap();
        assert!(Arc::ptr_eq(&fallback, &older));
        assert!(!fallback.same_request(&consumer));
    }
    #[test]
    fn decoding_and_policy_are_native_match_inputs() {
        let store = Store::default();
        let _source = store.admit(key()).unwrap();
        let mut wrong = key();
        wrong.encoding = "UTF-8".into();
        assert!(store.claim(wrong, false).is_none());
        let mut wrong = key();
        wrong.policy = Some("other policy".into());
        let candidate = store.claim(wrong.clone(), false).unwrap();
        assert!(!candidate.same_request(&wrong));
        store.finish_claim(&candidate, false);
        assert!(store.claim(key(), false).is_some());
    }
    #[test]
    fn replay_cancels_claimed_tentative_and_preserves_completed_baseline() {
        let store = Arc::new(Store::default());
        let baseline = store.admit(key()).unwrap();
        let snapshot = store.snapshot();
        baseline.complete(Ok(Response::Script(
            key().url,
            "baseline".into(),
            false,
            "origin".into(),
        )));
        let _claimed = store.claim(key(), false).unwrap();
        let mut temporary = key();
        temporary.url.push_str("?tentative");
        let provisional = store.admit(temporary.clone()).unwrap();
        let claimed = store.claim(temporary.clone(), false).unwrap();
        snapshot.restore();
        assert!(claimed.cancelled());
        assert!(provisional.cancelled());
        assert!(store.claim(key(), false).unwrap().result().unwrap().is_ok());
        let parsed = store.admit(temporary).unwrap();
        snapshot.restore();
        assert!(!parsed.cancelled());
    }
}
