//! Document-owned, single-use preload custody. Lookup uses the complete native
//! request, including authorization and decoding inputs; URLs alone never join.
use super::{
    ContextScriptLoad, ContextServiceRequest as Request, ContextServiceResponse as Response,
    RuntimeError,
};
use crate::script_load_failure::ScriptLoadFailure;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
const LIMIT: usize = 32 * 1024 * 1024;
fn response_bytes(response: &Response) -> Option<usize> {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .filter(|total| *total <= LIMIT)
                .ok_or_else(|| std::io::Error::other("preload response budget"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, response).ok()?;
    Some(count.0)
}

#[derive(PartialEq, Eq)]
struct Key(Vec<u8>);
#[derive(Clone, Copy)]
enum Kind {
    Script,
    Style,
}
fn key(request: &Request) -> Result<Key, RuntimeError> {
    match request {
        Request::LoadScript {
            module: false,
            script_policy: Some(_),
            ..
        } => {}
        Request::LoadPageResource {
            destination,
            post: None,
            download: None,
            script_authority: None,
            form_authority: None,
            ..
        } if destination == "stylesheet" => {
            if !matches!(request,Request::LoadPageResource{initiator,..}if initiator.stylesheet.as_ref().is_some_and(|sheet|sheet.policy.is_some()))
            {
                return Err(RuntimeError::ProtocolViolation(
                    "preload stylesheet policy snapshot absent".into(),
                ));
            }
        }
        _ => {
            return Err(RuntimeError::ProtocolViolation(
                "preload request is not a classic script or stylesheet".into(),
            ));
        }
    }
    let mut normalized = request.clone();
    let normalize_cors = |value: &mut Option<String>| {
        if let Some(current) = value {
            *current = if current.eq_ignore_ascii_case("use-credentials") {
                "use-credentials"
            } else {
                "anonymous"
            }
            .into();
        }
    };
    let requested = match &mut normalized {
        Request::LoadScript {
            url, cross_origin, ..
        } => {
            normalize_cors(cross_origin);
            url
        }
        Request::LoadPageResource { url, initiator, .. } => {
            if let Some(sheet) = &mut initiator.stylesheet {
                sheet.render_blocking = false;
                normalize_cors(&mut sheet.cross_origin);
            }
            url
        }
        _ => unreachable!(),
    };
    if let Ok(mut url) = url::Url::parse(requested) {
        url.set_fragment(None);
        *requested = url.to_string();
    }
    let key = serde_json::to_vec(&normalized)
        .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
    if key.len() > 256 * 1024 {
        return Err(RuntimeError::ContextService(
            "preload request metadata limit".into(),
        ));
    }
    Ok(Key(key))
}
pub(super) fn consumption_key(request: &Request) -> Result<Vec<u8>, RuntimeError> {
    key(request).map(|key| key.0)
}
pub(super) fn reuse_key(request: &Request) -> Result<Vec<u8>, RuntimeError> {
    let Key(bytes) = key(request)?;
    let mut request: Request = serde_json::from_slice(&bytes)
        .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
    match &mut request {
        Request::LoadScript {
            script_policy,
            parser_inserted,
            ..
        } => {
            *script_policy = None;
            *parser_inserted = false;
        }
        Request::LoadPageResource { initiator, .. } => {
            if let Some(sheet) = &mut initiator.stylesheet {
                sheet.policy = None;
            }
        }
        _ => unreachable!(),
    }
    serde_json::to_vec(&request).map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))
}
pub(crate) struct Entry {
    pub(super) status: crate::render_request_status::Status,
    kind: Kind,
    key: Key,
    reuse_key: Vec<u8>,
    proof: Mutex<Option<String>>,
    expires: Instant,
    cancelled: AtomicBool,
    claimed: AtomicBool,
    started: AtomicBool,
    changed: Condvar,
    lease: Mutex<Option<ContextScriptLoad>>,
    result: Mutex<Option<Result<Response, ScriptLoadFailure>>>,
    bytes: AtomicUsize,
    budget: Arc<AtomicUsize>,
}
impl Entry {
    pub(super) fn proof(&self) -> Option<String> {
        self.proof.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub(super) fn same_request(&self, request: &Request) -> bool {
        key(request).is_ok_and(|key| self.key == key)
    }
    pub(super) fn start(&self) -> bool {
        !self.started.swap(true, Ordering::AcqRel)
    }
    pub(super) fn attach(&self, lease: ContextScriptLoad) -> Result<(), RuntimeError> {
        let mut current = self.lease.lock().unwrap_or_else(|e| e.into_inner());
        if self.cancelled.load(Ordering::Acquire) {
            lease.cancel();
            return Err(RuntimeError::Closed);
        }
        *current = Some(lease);
        Ok(())
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.expires
    }
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.changed.notify_all();
        if let Some(lease) = self
            .lease
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            lease.cancel();
        }
    }
    pub(super) fn complete(&self, result: Result<Response, RuntimeError>) {
        self.status.finish();
        if self.cancelled() {
            self.cancel();
            return;
        }
        let mut slot = self.result.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return;
        }
        let result=result.map_err(ScriptLoadFailure::decoded).and_then(|response|{
   let response=match response{Response::PreloadedResource{proof,response}=>{*self.proof.lock().unwrap_or_else(|e|e.into_inner())=proof;*response},response=>response};
   let valid=matches!((&self.kind,&response),(Kind::Script,Response::ScriptSource{module:false,..})|(Kind::Style,Response::PageResource{stream_id:None,..})|(_,Response::ScriptLoadFailed{..}))&&(!matches!(&response,Response::PageResource{destination,..}if destination!="stylesheet"));
   if valid{Ok(response)}else{Err(ScriptLoadFailure::Infrastructure("preload response kind mismatch".into()))}
  });
        let bytes = result.as_ref().map_or(Some(0), response_bytes);
        let Some(bytes) = bytes else {
            *slot = Some(Err(ScriptLoadFailure::Capacity(
                "preload retained response budget".into(),
            )));
            self.changed.notify_all();
            return;
        };
        let accepted = self
            .budget
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|total| *total <= LIMIT)
            })
            .is_ok();
        if accepted {
            self.bytes.store(bytes, Ordering::Release);
            *slot = Some(result);
        } else {
            *slot = Some(Err(ScriptLoadFailure::Capacity(
                "preload retained response budget".into(),
            )));
        }
        self.changed.notify_all();
        // The response is fully received; the shared producer lease stays attached
        // until all native consumers and the retained link publication release it.
    }
    pub(super) fn wait(&self) -> Result<Response, RuntimeError> {
        let mut result = self.result.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if self.cancelled() {
                drop(result);
                self.cancel();
                return Err(RuntimeError::Closed);
            }
            if let Some(response) = result.as_ref() {
                return response.clone().map_err(ScriptLoadFailure::runtime);
            }
            let wait = self
                .expires
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(50));
            result = self
                .changed
                .wait_timeout(result, wait)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
    pub(crate) fn outcome(&self) -> Option<Result<Response, ScriptLoadFailure>> {
        if self.cancelled() {
            self.cancel();
            return Some(Err(ScriptLoadFailure::Cancelled));
        }
        self.result
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub(crate) fn result(&self) -> Option<Result<Response, RuntimeError>> {
        self.outcome()
            .map(|result| result.map_err(ScriptLoadFailure::runtime))
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
pub(super) struct Store {
    entries: Mutex<VecDeque<Arc<Entry>>>,
    owners: Mutex<Vec<Weak<Entry>>>,
    budget: Arc<AtomicUsize>,
    retired: AtomicBool,
}
impl Store {
    pub(super) fn admit(&self, request: &Request) -> Result<Arc<Entry>, RuntimeError> {
        let reuse_key = reuse_key(request)?;
        let key = key(request)?;
        if self.retired.load(Ordering::Acquire) {
            return Err(RuntimeError::Closed);
        }
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if self.retired.load(Ordering::Acquire) {
            return Err(RuntimeError::Closed);
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
            return Err(RuntimeError::ContextService(
                "preload ownership limit".into(),
            ));
        }
        let entry = Arc::new(Entry {
            status: Default::default(),
            kind: if matches!(request, Request::LoadScript { .. }) {
                Kind::Script
            } else {
                Kind::Style
            },
            key,
            reuse_key,
            proof: Mutex::new(None),
            expires: Instant::now() + Duration::from_secs(60),
            cancelled: AtomicBool::new(false),
            claimed: AtomicBool::new(false),
            started: AtomicBool::new(false),
            changed: Condvar::new(),
            lease: Mutex::new(None),
            result: Mutex::new(None),
            bytes: AtomicUsize::new(0),
            budget: self.budget.clone(),
        });
        owners.push(Arc::downgrade(&entry));
        entries.push_back(entry.clone());
        Ok(entry)
    }
    pub(super) fn claim(&self, request: &Request, blocking: bool) -> Option<Arc<Entry>> {
        if self.retired.load(Ordering::Acquire) {
            return None;
        }
        let candidate = reuse_key(request).ok()?;
        let exact = key(request).ok()?;
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if self.retired.load(Ordering::Acquire) {
            return None;
        }
        entries.retain(|entry| !entry.cancelled());
        let available = |entry: &&Arc<Entry>| !entry.claimed.load(Ordering::Acquire);
        let entry = entries
            .iter()
            .filter(available)
            .find(|entry| entry.key == exact)
            .or_else(|| {
                entries
                    .iter()
                    .filter(available)
                    .find(|entry| entry.reuse_key == candidate)
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
    pub(super) fn finish_claim(&self, entry: &Arc<Entry>, consumed: bool) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if consumed {
            entries.retain(|candidate| !Arc::ptr_eq(candidate, entry));
        } else {
            entry.claimed.store(false, Ordering::Release);
        }
    }
    pub(super) fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        // Admission takes entries before owners. Never hold owners while acquiring
        // entries, and never call the transport cancellation machinery under either.
        let owned = {
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            let mut owners = self.owners.lock().unwrap_or_else(|e| e.into_inner());
            let owned = owners.iter().filter_map(Weak::upgrade).collect::<Vec<_>>();
            entries.clear();
            owners.clear();
            owned
        };
        for entry in owned {
            entry.cancel();
        }
    }
}
// An encoding restart keeps the same Document and authority epoch. Preserve
// exact baseline producers, and explicitly revoke tentative producers even if
// an old parser consumer has already claimed one from the lookup queue.
pub(crate) struct Snapshot {
    store: Arc<Store>,
    entries: Vec<(Arc<Entry>, bool)>,
    owners: Vec<Arc<Entry>>,
    restored: AtomicBool,
}
impl Snapshot {
    pub(crate) fn retained_bytes(&self) -> usize {
        self.owners
            .iter()
            .map(|entry| entry.key.0.len() + 128)
            .sum()
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
impl Store {
    pub(super) fn snapshot(self: &Arc<Self>) -> Snapshot {
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
#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        Request::LoadScript {
            fallback_encoding: Some("windows-1252".into()),
            parser_inserted: true,
            script_policy: Some("native-policy".into()),
            page_id: oryn_common::v3::PageId(1),
            creator: oryn_js::worker_wire::Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
            url: "https://example.test/script.js".into(),
            module: false,
            nonce: None,
            integrity: None,
            cross_origin: None,
            initiator_url: Some("https://example.test/".into()),
            referrer_policy: Some("no-referrer".into()),
            referrer_source: None,
        }
    }
    fn response() -> Response {
        Response::ScriptSource {
            timing: None,
            url: "https://example.test/script.js".into(),
            source: "void 0".into(),
            module: false,
            muted_errors: false,
            referrer_policy: "no-referrer".into(),
        }
    }
    #[test]
    fn full_preparation_key_and_single_use_claim_preserve_original_custody() {
        let store = Store::default();
        let link = store.admit(&request()).unwrap();
        assert!(link.start());
        assert!(!store.admit(&request()).unwrap().start());
        let mut other = request();
        if let Request::LoadScript {
            fallback_encoding, ..
        } = &mut other
        {
            *fallback_encoding = Some("UTF-8".into())
        }
        assert!(store.claim(&other, true).is_none());
        if let Request::LoadScript {
            fallback_encoding,
            script_policy,
            ..
        } = &mut other
        {
            *fallback_encoding = Some("windows-1252".into());
            *script_policy = Some("other-policy".into())
        }
        let candidate = store.claim(&other, true).unwrap();
        assert!(
            !candidate.same_request(&other),
            "policy changes require consuming proof"
        );
        store.finish_claim(&candidate, false);
        let consumer = store.claim(&request(), true).unwrap();
        assert!(store.claim(&request(), true).is_none());
        drop(link);
        assert!(!consumer.cancelled());
        consumer.complete(Ok(response()));
        assert!(consumer.status.blocking());
        assert!(consumer.result().unwrap().is_ok());
        store.retire();
        assert!(consumer.cancelled());
        assert!(consumer.result().unwrap().is_err());
    }
    #[test]
    fn encoding_restart_revokes_claimed_tentative_producer_and_restores_baseline_once() {
        let store = Arc::new(Store::default());
        let baseline = store.admit(&request()).unwrap();
        let snapshot = store.snapshot();
        let claimed_baseline = store.claim(&request(), false).unwrap();
        let mut tentative = request();
        if let Request::LoadScript { url, .. } = &mut tentative {
            *url = "https://example.test/wrong-encoding.js".into();
        }
        let transient = store.admit(&tentative).unwrap();
        let claimed = store.claim(&tentative, false).unwrap();
        snapshot.restore();
        assert!(transient.cancelled());
        assert!(claimed.cancelled());
        assert!(!claimed_baseline.cancelled());
        assert!(Arc::ptr_eq(
            &store.claim(&request(), false).unwrap(),
            &baseline
        ));
        assert!(store.claim(&tentative, false).is_none());
        let reparsed = store.admit(&tentative).unwrap();
        snapshot.restore();
        assert!(!reparsed.cancelled());
    }
    #[test]
    fn repeated_link_after_claim_retains_original_request_status() {
        let store = Store::default();
        let producer = store.admit(&request()).unwrap();
        assert!(producer.start());
        let consumer = store.claim(&request(), true).unwrap();
        let repeated = store.admit(&request()).unwrap();
        assert!(Arc::ptr_eq(&producer, &repeated));
        assert!(!repeated.start());
        assert!(store.claim(&request(), false).is_none());
        consumer.complete(Ok(response()));
        assert!(repeated.status.blocking());
    }
    #[test]
    fn exact_preload_is_not_shadowed_by_an_older_policy_variant() {
        let store = Store::default();
        let older = store.admit(&request()).unwrap();
        let mut consumer = request();
        if let Request::LoadScript {
            script_policy,
            parser_inserted,
            ..
        } = &mut consumer
        {
            *script_policy = Some("consumer-policy".into());
            *parser_inserted = false;
        }
        let exact = store.admit(&consumer).unwrap();
        exact.complete(Ok(response()));
        let selected = store.claim(&consumer, true).unwrap();
        assert!(Arc::ptr_eq(&selected, &exact));
        assert!(selected.same_request(&consumer));
        assert!(!older.status.blocking());
        store.finish_claim(&selected, false);
        assert!(Arc::ptr_eq(&store.claim(&consumer, false).unwrap(), &exact));
        store.finish_claim(&selected, true);
        let fallback = store.claim(&consumer, false).unwrap();
        assert!(Arc::ptr_eq(&fallback, &older));
        assert!(!fallback.same_request(&consumer));
    }
    #[test]
    fn completed_preload_keeps_nonblocking_status_and_charges_response_once() {
        let store = Store::default();
        let preload = store.admit(&request()).unwrap();
        preload.complete(Ok(response()));
        let before = store.budget.load(Ordering::Acquire);
        preload.complete(Ok(response()));
        assert_eq!(store.budget.load(Ordering::Acquire), before);
        let consumer = store.claim(&request(), true).unwrap();
        assert!(!consumer.status.blocking());
        store.finish_claim(&consumer, true);
        drop(preload);
        drop(consumer);
        assert_eq!(store.budget.load(Ordering::Acquire), 0);
    }
    #[test]
    fn native_consumer_joins_original_preload_after_link_owner_is_dropped() {
        use super::super::{ContextTransport, SharedContextClient};
        use std::sync::mpsc;
        struct Transport {
            next: AtomicUsize,
            fetches: Arc<AtomicUsize>,
            entered: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl ContextTransport for Transport {
            fn request(&self, request: Request) -> Result<Response, RuntimeError> {
                match request {
                    Request::ReserveScriptLoad { .. } => Ok(Response::ScriptLoadReserved {
                        token: self.next.fetch_add(1, Ordering::AcqRel) as u64 + 1,
                    }),
                    Request::PreloadResource { .. } => {
                        self.fetches.fetch_add(1, Ordering::AcqRel);
                        self.entered.send(()).unwrap();
                        self.release
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                        Ok(Response::PreloadedResource {
                            proof: Some("fixture-proof".into()),
                            response: Box::new(response()),
                        })
                    }
                    Request::AuthorizePreloadConsumption {
                        proof,
                        request: consumer,
                    } => {
                        assert_eq!(proof, "fixture-proof");
                        assert_eq!(
                            consumption_key(&consumer).unwrap(),
                            consumption_key(&self::request()).unwrap()
                        );
                        Ok(Response::PreloadConsumptionAuthorized { allowed: true })
                    }
                    Request::RunScriptLoad { .. } => {
                        panic!("consumer must reuse authenticated preload response")
                    }
                    Request::CancelScriptLoad { .. } => Ok(Response::ScriptLoadCancelled),
                    _ => Err(RuntimeError::ProtocolViolation(
                        "unexpected preload test operation".into(),
                    )),
                }
            }
        }
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let fetches = Arc::new(AtomicUsize::new(0));
        let client = SharedContextClient(
            Arc::new(Transport {
                next: AtomicUsize::new(0),
                fetches: fetches.clone(),
                entered: entered_tx,
                release: Mutex::new(release_rx),
            }),
            None,
            Default::default(),
            None,
            Default::default(),
        );
        let link = client.begin_resource_preload(request()).unwrap();
        let status = link.status.clone();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(link);
        let consumer = client.clone();
        let thread = std::thread::spawn(move || {
            let Request::LoadScript {
                page_id, creator, ..
            } = request()
            else {
                unreachable!()
            };
            let lease = consumer.reserve_script_load(page_id, creator).unwrap();
            consumer.request(Request::RunScriptLoad {
                render_blocking: true,
                page_id,
                token: lease.token,
                request: Box::new(request()),
            })
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !status.blocking() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        release_tx.send(()).unwrap();
        assert!(matches!(
            thread.join().unwrap().unwrap(),
            Response::ScriptSource { .. }
        ));
        assert_eq!(fetches.load(Ordering::Acquire), 1);
        let entries = client.drain_resource_timings();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].initiator, "link");
        assert!(entries[0].render_blocking);
        client.drain_script_cleanup(Duration::from_secs(2)).unwrap();
        assert_eq!(client.resource_job_activity().0, 0);
    }
}
