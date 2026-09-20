//! Each native broker owner maps caller IDs to process-unique transport IDs.
//! Page/agent counters and high-bit values never become cancellation authority.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
static NEXT_NATIVE_ID: AtomicU64 = AtomicU64::new(1);
type Key = (Option<WorkerRoute>, u64);
#[derive(Clone)]
struct Identity {
    foreground_finished: Arc<AtomicBool>,
    background_cancel: Arc<AtomicBool>,
    native: u64,
    cancelled: Arc<tokio::sync::Notify>,
    cancel_state: Arc<AtomicU8>,
}
type Entries = Arc<Mutex<BTreeMap<Key, Identity>>>;
type Activity = Arc<Mutex<Option<std::time::Instant>>>;
struct Lease {
    foreground_finished: Arc<AtomicBool>,
    background_cancel: Arc<AtomicBool>,
    entries: Entries,
    activity: Activity,
    key: Key,
    native: u64,
    cancelled: Arc<tokio::sync::Notify>,
    cancel_state: Arc<AtomicU8>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if entries
            .get(&self.key)
            .is_some_and(|entry| entry.native == self.native)
        {
            entries.remove(&self.key);
            *self.activity.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(std::time::Instant::now());
        }
    }
}
// Lexical native admission authority. Deferred work must explicitly capture
// this value before handoff; the TLS slot itself never crosses an executor.
#[derive(Clone)]
pub(super) struct BackgroundOwner {
    network: Arc<Box<dyn NetworkBroker>>,
    leases: Vec<Arc<Lease>>,
    pub(super) cancelled: Arc<AtomicBool>,
}
thread_local! {static BACKGROUND_OWNER:std::cell::RefCell<Option<BackgroundOwner>>=const{std::cell::RefCell::new(None)};}
pub(super) fn capture_background_owner() -> Option<BackgroundOwner> {
    BACKGROUND_OWNER.with(|slot| slot.borrow().clone())
}
pub(super) fn with_background_owner<T>(
    owner: Option<BackgroundOwner>,
    work: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<BackgroundOwner>);
    impl Drop for Restore {
        fn drop(&mut self) {
            BACKGROUND_OWNER.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    // Captured absence is significant: an unrelated caller's ambient owner
    // must not become authority for work admitted without an owner.
    let _restore = Restore(BACKGROUND_OWNER.with(|slot| slot.replace(owner)));
    work()
}
impl BackgroundOwner {
    fn extend(network: Arc<Box<dyn NetworkBroker>>, lease: Arc<Lease>) -> Self {
        let mut owner = capture_background_owner().unwrap_or_else(|| Self {
            network,
            leases: Vec::new(),
            cancelled: lease.background_cancel.clone(),
        });
        owner.leases.push(lease);
        owner
    }
    pub(super) fn broker(&self) -> SharedNetworkBroker {
        SharedNetworkBroker {
            inner: Some(Arc::new(Box::new(IdentityBroker::new(
                self.network.clone(),
            )))),
            audit: Default::default(),
            audit_page_id: None,
            audit_worker_pid: None,
        }
    }
    pub(super) fn run<T>(&self, work: impl FnOnce() -> T) -> T {
        with_background_owner(Some(self.clone()), work)
    }
}
pub(super) struct IdentityBroker {
    inner: Arc<Box<dyn NetworkBroker>>,
    entries: Entries,
    activity: Activity,
}
impl IdentityBroker {
    pub(super) fn new(inner: Arc<Box<dyn NetworkBroker>>) -> Self {
        Self {
            inner,
            entries: Default::default(),
            activity: Default::default(),
        }
    }
    fn reserve(
        &self,
        route: Option<&WorkerRoute>,
        public: u64,
    ) -> Result<Arc<Lease>, NetworkError> {
        let key = (route.cloned(), public);
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if entries.contains_key(&key) {
            return Err(NetworkError::Transport(
                "network request identity already active".into(),
            ));
        }
        if entries.len() >= 1024 {
            return Err(NetworkError::Transport(
                "native network identity capacity".into(),
            ));
        }
        let native = NEXT_NATIVE_ID
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
            .map_err(|_| NetworkError::Transport("native network identities exhausted".into()))?;
        let foreground_finished = Arc::new(AtomicBool::new(false));
        let background_cancel = capture_background_owner()
            .map(|owner| owner.cancelled)
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        let cancelled = Arc::new(tokio::sync::Notify::new());
        let cancel_state = Arc::new(AtomicU8::new(0));
        entries.insert(
            key.clone(),
            Identity {
                foreground_finished: foreground_finished.clone(),
                background_cancel: background_cancel.clone(),
                native,
                cancelled: cancelled.clone(),
                cancel_state: cancel_state.clone(),
            },
        );
        *self.activity.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
        Ok(Arc::new(Lease {
            foreground_finished,
            background_cancel,
            entries: self.entries.clone(),
            activity: self.activity.clone(),
            key,
            native,
            cancelled,
            cancel_state,
        }))
    }
    fn start(
        &self,
        route: Option<&WorkerRoute>,
        mut request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        let runtime = operation_runtime();
        let public = request.request_id;
        let lease = self.reserve(route, public)?;
        request.request_id = lease.native;
        let mut operation = match BackgroundOwner::extend(self.inner.clone(), lease.clone())
            .run(|| self.inner.start_routed(route, request))
        {
            Ok(operation) => operation,
            Err(error) => {
                retire(self.inner.clone(), route.cloned(), lease);
                return Err(error);
            }
        };
        if operation.request_id != lease.native {
            retire(self.inner.clone(), route.cloned(), lease);
            return Err(NetworkError::Transport(
                "native operation identity mismatch".into(),
            ));
        }
        let demand = operation.demand.take();
        let (sender, events) = tokio::sync::mpsc::channel(8);
        let inner = self.inner.clone();
        let route = route.cloned();
        // A lane owns its lease through terminal delivery or acknowledged
        // cancellation. Failed cancellation keeps capacity; no Drop does IO.
        runtime.spawn(async move{
            let mut terminal=false;
            loop{
                let event=tokio::select!{event=operation.next_event()=>event,_=lease.cancelled.notified()=>{terminal=true;break;},_=sender.closed()=>break};
                let Some(mut event)=event else{break;};
                let id=match &mut event{NetworkEvent::Headers{request_id,..}|NetworkEvent::Chunk{request_id,..}|NetworkEvent::Complete{request_id,..}|NetworkEvent::Failed{request_id,..}=>request_id};
                if *id!=lease.native{
                    // A full output queue must not hide cancellation while we
                    // report a malformed upstream event. The original native
                    // lease remains owned until cancellation is acknowledged.
                    tokio::select!{
                        _=sender.send(NetworkEvent::Failed{request_id:public,error:"native network event identity mismatch".into()})=>{},
                        _=lease.cancelled.notified()=>{terminal=true;},
                    }
                    break;
                }
                *id=public;terminal=matches!(event,NetworkEvent::Complete{..}|NetworkEvent::Failed{..});if terminal{lease.foreground_finished.store(true,Ordering::Release);}
                let sent=tokio::select!{sent=sender.send(event)=>sent.is_ok(),_=lease.cancelled.notified()=>{terminal=true;false}};
                if !sent||terminal{break;}
            }
            if !terminal{loop{let broker=inner.clone();let route=route.clone();let identity=Identity{foreground_finished:lease.foreground_finished.clone(),background_cancel:lease.background_cancel.clone(),native:lease.native,cancelled:lease.cancelled.clone(),cancel_state:lease.cancel_state.clone()};
                if tokio::task::spawn_blocking(move||cancel_native(&broker,route.as_ref(),&identity).is_ok()).await.unwrap_or(false){break;}
                tokio::time::sleep(Duration::from_millis(100)).await;
            }}
            drop(lease);
        });
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id: public,
            events,
            demand,
        })
    }
}
struct IdentityRead {
    inner: Box<dyn Read + Send>,
    broker: Arc<Box<dyn NetworkBroker>>,
    lease: Option<Arc<Lease>>,
}
impl Drop for IdentityRead {
    fn drop(&mut self) {
        // Dropping an unread or failed body is cancellation, not terminal
        // transport evidence. Keep its identity until the broker acknowledges
        // cleanup; the destructor must never wait for that acknowledgement.
        if let Some(lease) = self.lease.take() {
            retire(self.broker.clone(), None, lease);
        }
    }
}
impl Read for IdentityRead {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let result = self.inner.read(output);
        // A zero-length read is not EOF. A real EOF ends transport ownership
        // even when the caller retains its exhausted response object.
        if !output.is_empty()
            && matches!(&result, Ok(0))
            && let Some(lease) = self.lease.take()
        {
            lease.foreground_finished.store(true, Ordering::Release);
        }
        result
    }
}
impl Drop for IdentityBroker {
    fn drop(&mut self) {
        let pending = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .map(|((route, _), identity)| (route.clone(), identity.clone()))
            .collect::<Vec<_>>();
        for (route, identity) in pending {
            identity.background_cancel.store(true, Ordering::Release);
            let inner = self.inner.clone();
            operation_runtime().spawn(async move {
                loop {
                    let inner = inner.clone();
                    let route = route.clone();
                    let identity = identity.clone();
                    if tokio::task::spawn_blocking(move || {
                        cancel_native(&inner, route.as_ref(), &identity).is_ok()
                    })
                    .await
                    .unwrap_or(false)
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            });
        }
    }
}
impl NetworkBroker for IdentityBroker {
    fn discard_response_policy_routed(
        &self,
        route: Option<&WorkerRoute>,
        token: &str,
    ) -> Result<(), NetworkError> {
        // Token authority is bound to its route, not this wrapper's request ID
        // namespace. Do not look up a possibly completed or reused numeric ID.
        self.inner.discard_response_policy_routed(route, token)
    }
    fn owned_operation_activity(&self) -> (usize, Option<std::time::Instant>) {
        // Admission, release and snapshots lock in this order, so a completed
        // request between polling checkpoints cannot disappear without a time.
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let changed = *self.activity.lock().unwrap_or_else(|e| e.into_inner());
        (entries.len(), changed)
    }
    fn drain_owned_operations(&self, timeout: Duration) -> Result<(), NetworkError> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if self
                .entries
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty()
            {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(NetworkError::Transport(
                    "native network operations remain owned".into(),
                ));
            }
            std::thread::park_timeout(Duration::from_millis(1));
        }
    }
    fn send(&self, mut request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let public = request.request_id;
        let lease = self.reserve(None, public)?;
        request.request_id = lease.native;
        let mut response = BackgroundOwner::extend(self.inner.clone(), lease.clone())
            .run(|| self.inner.send(request))?;
        if response.request_id != lease.native {
            return Err(NetworkError::Transport(
                "native response identity mismatch".into(),
            ));
        }
        lease.foreground_finished.store(true, Ordering::Release);
        response.request_id = public;
        Ok(response)
    }
    fn start_stream(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkStreamResponse, NetworkError> {
        let public = request.request_id;
        let lease = self.reserve(None, public)?;
        request.request_id = lease.native;
        let mut response = BackgroundOwner::extend(self.inner.clone(), lease.clone())
            .run(|| self.inner.start_stream(request))?;
        if response.request_id != lease.native {
            retire(self.inner.clone(), None, lease);
            return Err(NetworkError::Transport(
                "native stream identity mismatch".into(),
            ));
        }
        response.request_id = public;
        response.body = Box::new(IdentityRead {
            inner: response.body,
            broker: self.inner.clone(),
            lease: Some(lease),
        });
        Ok(response)
    }
    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        self.start(None, request)
    }
    fn start_routed(
        &self,
        route: Option<&WorkerRoute>,
        request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        // A routed envelope must reach its authenticating router unchanged.
        // That router's PageNetworkBroker owns the native request namespace.
        if route.is_some() {
            self.inner.start_routed(route, request)
        } else {
            self.start(None, request)
        }
    }
    fn cancel_operation(&self, public: u64) -> Result<(), NetworkError> {
        self.cancel_routed(None, public)
    }
    fn cancel_routed(&self, route: Option<&WorkerRoute>, public: u64) -> Result<(), NetworkError> {
        if route.is_some() {
            return self.inner.cancel_routed(route, public);
        }
        let identity = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(route.cloned(), public))
            .cloned();
        match identity {
            Some(identity) => cancel_native(&self.inner, route, &identity),
            None => Ok(()),
        }
    }
}
fn retire(inner: Arc<Box<dyn NetworkBroker>>, route: Option<WorkerRoute>, lease: Arc<Lease>) {
    operation_runtime().spawn(async move {
        loop {
            let broker = inner.clone();
            let route = route.clone();
            let identity = Identity {
                foreground_finished: lease.foreground_finished.clone(),
                background_cancel: lease.background_cancel.clone(),
                native: lease.native,
                cancelled: lease.cancelled.clone(),
                cancel_state: lease.cancel_state.clone(),
            };
            if tokio::task::spawn_blocking(move || {
                cancel_native(&broker, route.as_ref(), &identity).is_ok()
            })
            .await
            .unwrap_or(false)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        drop(lease);
    });
}
fn cancel_native(
    inner: &Arc<Box<dyn NetworkBroker>>,
    route: Option<&WorkerRoute>,
    identity: &Identity,
) -> Result<(), NetworkError> {
    identity.background_cancel.store(true, Ordering::Release);
    if identity.foreground_finished.load(Ordering::Acquire) {
        identity.cancelled.notify_one();
        return Ok(());
    }
    match identity
        .cancel_state
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
    {
        Ok(_) => {}
        Err(2) => return Ok(()),
        Err(_) => {
            return Err(NetworkError::Transport(
                "native cancellation acknowledgement pending".into(),
            ));
        }
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        inner.cancel_routed(route, identity.native)
    }))
    .unwrap_or_else(|_| {
        Err(NetworkError::Transport(
            "native cancellation panicked".into(),
        ))
    });
    identity
        .cancel_state
        .store(if result.is_ok() { 2 } else { 0 }, Ordering::Release);
    if result.is_ok() {
        identity.cancelled.notify_one();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    #[test]
    fn rejected_start_keeps_registered_native_identity_until_cancel_ack() {
        struct Reject {
            registered: Arc<Mutex<Vec<u64>>>,
            ack: Arc<AtomicBool>,
        }
        impl NetworkBroker for Reject {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                unreachable!()
            }
            fn start_operation(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkOperation, NetworkError> {
                self.registered.lock().unwrap().push(request.request_id);
                Err(NetworkError::Transport(
                    "registered then failed headers admission".into(),
                ))
            }
            fn cancel_operation(&self, id: u64) -> Result<(), NetworkError> {
                let mut active = self.registered.lock().unwrap();
                if !active.contains(&id) {
                    return Ok(());
                }
                if !self.ack.load(Ordering::Acquire) {
                    return Err(NetworkError::Transport("ACK unavailable".into()));
                }
                active.retain(|registered| *registered != id);
                Ok(())
            }
        }
        let registered = Arc::new(Mutex::new(Vec::new()));
        let ack = Arc::new(AtomicBool::new(false));
        let broker = SharedNetworkBroker::new(Reject {
            registered: registered.clone(),
            ack: ack.clone(),
        });
        assert!(broker.start_operation(request(91)).is_err());
        assert_eq!(broker.owned_operation_activity().0, 1);
        assert_eq!(registered.lock().unwrap().len(), 1);
        assert!(broker.drain_owned_operations(Duration::ZERO).is_err());
        ack.store(true, Ordering::Release);
        broker
            .drain_owned_operations(Duration::from_secs(2))
            .unwrap();
        assert!(registered.lock().unwrap().is_empty());
        assert_eq!(broker.owned_operation_activity().0, 0);
        assert!(
            broker.cancel_operation(91).is_ok(),
            "already released admission is idempotent"
        );
    }

    #[test]
    fn dropping_uncommitted_scope_signals_background_and_retains_ancestor_until_ack() {
        struct Capture {
            captured: Arc<Mutex<Option<BackgroundOwner>>>,
            ack: Arc<AtomicBool>,
            retained: Mutex<Vec<tokio::sync::mpsc::Sender<NetworkEvent>>>,
        }
        impl NetworkBroker for Capture {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                *self.captured.lock().unwrap() = capture_background_owner();
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![],
                    body: vec![],
                })
            }
            fn start_operation(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkOperation, NetworkError> {
                let (sender, events) = tokio::sync::mpsc::channel(1);
                self.retained.lock().unwrap().push(sender);
                Ok(NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id: request.request_id,
                    events,
                    demand: None,
                })
            }
            fn cancel_operation(&self, _: u64) -> Result<(), NetworkError> {
                if self.ack.load(Ordering::Acquire) {
                    Ok(())
                } else {
                    Err(NetworkError::Transport("ACK withheld".into()))
                }
            }
        }
        let captured = Arc::new(Mutex::new(None));
        let ack = Arc::new(AtomicBool::new(false));
        let parent = SharedNetworkBroker::new(Capture {
            captured: captured.clone(),
            ack: ack.clone(),
            retained: Mutex::new(Vec::new()),
        });
        let candidate = parent.clone().new_request_scope();
        let sibling = parent.clone().new_request_scope();
        candidate.send(request(80)).unwrap();
        let owner = captured.lock().unwrap().take().unwrap();
        let background = owner.broker();
        let operation = owner
            .run(|| background.start_operation(request(81)))
            .unwrap();
        drop(candidate);
        // SharedNetworkBroker releases blocking transports on its cleanup thread.
        // Wait for that dispatch, while keeping the cancellation ACK withheld.
        let deadline = Instant::now() + Duration::from_secs(2);
        while !owner.cancelled.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(
            owner.cancelled.load(Ordering::Acquire),
            "discarding the uncommitted document must stop its refresh"
        );
        drop(operation);
        assert!(background.drain_owned_operations(Duration::ZERO).is_err());
        assert!(
            parent.owned_operation_activity().0 > 0,
            "ancestor custody survives rejected cancellation"
        );
        assert_eq!(sibling.owned_operation_activity().0, 0);
        ack.store(true, Ordering::Release);
        background
            .drain_owned_operations(Duration::from_secs(2))
            .unwrap();
        drop(owner);
        drop(background);
        parent
            .drain_owned_operations(Duration::from_secs(2))
            .unwrap();
    }

    #[test]
    fn background_owner_is_nested_panic_safe_and_retained_until_cancel_ack() {
        struct Capture {
            captured: Arc<Mutex<Option<BackgroundOwner>>>,
            ack: Arc<AtomicBool>,
            retained: Mutex<Vec<tokio::sync::mpsc::Sender<NetworkEvent>>>,
        }
        impl NetworkBroker for Capture {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                *self.captured.lock().unwrap() = capture_background_owner();
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: Vec::new(),
                    body: Vec::new(),
                })
            }
            fn start_operation(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkOperation, NetworkError> {
                let (sender, events) = tokio::sync::mpsc::channel(1);
                self.retained.lock().unwrap().push(sender);
                Ok(NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id: request.request_id,
                    events,
                    demand: None,
                })
            }
            fn cancel_operation(&self, _: u64) -> Result<(), NetworkError> {
                if self.ack.load(Ordering::Acquire) {
                    Ok(())
                } else {
                    Err(NetworkError::Transport("ACK withheld".into()))
                }
            }
        }
        let captured = Arc::new(Mutex::new(None));
        let ack = Arc::new(AtomicBool::new(false));
        let base = SharedNetworkBroker::new(Capture {
            captured: captured.clone(),
            ack: ack.clone(),
            retained: Mutex::new(Vec::new()),
        });
        let document = base.clone().new_request_scope();
        let other = base.clone().new_request_scope();
        document.send(request(70)).unwrap();
        let owner = captured.lock().unwrap().take().unwrap();
        assert_eq!(document.owned_operation_activity().0, 1);
        assert_eq!(other.owned_operation_activity().0, 0);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || owner.run(|| panic!("restore span"))
            ))
            .is_err()
        );
        assert!(capture_background_owner().is_none());
        owner.run(|| {
            with_background_owner(None, || {
                assert!(
                    capture_background_owner().is_none(),
                    "captured absence must not inherit another request's owner"
                )
            });
            assert!(Arc::ptr_eq(
                &capture_background_owner().unwrap().cancelled,
                &owner.cancelled
            ));
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| with_background_owner(
                    None,
                    || {
                        assert!(capture_background_owner().is_none());
                        panic!("restore absent span");
                    }
                )))
                .is_err()
            );
            assert!(Arc::ptr_eq(
                &capture_background_owner().unwrap().cancelled,
                &owner.cancelled
            ));
        });
        assert!(capture_background_owner().is_none());
        let background = owner.broker();
        let operation = owner
            .run(|| background.start_operation(request(71)))
            .unwrap();
        assert!(
            document.cancel_operation(70).is_ok(),
            "completed foreground cancellation only signals background ownership"
        );
        assert!(owner.cancelled.load(Ordering::Acquire));
        drop(operation);
        assert!(background.drain_owned_operations(Duration::ZERO).is_err());
        assert_eq!(document.owned_operation_activity().0, 1);
        ack.store(true, Ordering::Release);
        background
            .drain_owned_operations(Duration::from_secs(2))
            .unwrap();
        drop(owner);
        document
            .drain_owned_operations(Duration::from_secs(2))
            .unwrap();
        assert_eq!(other.owned_operation_activity().0, 0);
    }
    #[test]
    fn abandoned_stream_retains_identity_until_cancellation_acknowledgement() {
        use std::sync::atomic::AtomicBool;
        struct StreamBroker {
            allow_cancel: Arc<AtomicBool>,
            cancelled: Arc<AtomicU64>,
            fail_read: bool,
        }
        struct InterruptedRead;
        impl Read for InterruptedRead {
            fn read(&mut self, _output: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(std::io::ErrorKind::Interrupted))
            }
        }
        impl NetworkBroker for StreamBroker {
            fn send(&self, _request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                unreachable!()
            }
            fn start_stream(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkStreamResponse, NetworkError> {
                let body: Box<dyn Read + Send> = if self.fail_read {
                    Box::new(InterruptedRead)
                } else {
                    Box::new(std::io::Cursor::new(vec![1]))
                };
                Ok(NetworkStreamResponse {
                    response_policy: None,
                    encoded_bytes: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: Vec::new(),
                    content_length: None,
                    upload: None,
                    body,
                })
            }
            fn cancel_operation(&self, id: u64) -> Result<(), NetworkError> {
                if !self.allow_cancel.load(Ordering::Acquire) {
                    return Err(NetworkError::Transport("cleanup not acknowledged".into()));
                }
                self.cancelled.store(id, Ordering::Release);
                Ok(())
            }
        }
        for fail_read in [false, true] {
            let allow_cancel = Arc::new(AtomicBool::new(false));
            let cancelled = Arc::new(AtomicU64::new(0));
            let broker = IdentityBroker::new(Arc::new(Box::new(StreamBroker {
                allow_cancel: allow_cancel.clone(),
                cancelled: cancelled.clone(),
                fail_read,
            })));
            let mut response = broker.start_stream(request(41)).unwrap();
            let native = broker
                .entries
                .lock()
                .unwrap()
                .get(&(None, 41))
                .unwrap()
                .native;
            if fail_read {
                assert_eq!(
                    response.body.read(&mut [0]).unwrap_err().kind(),
                    std::io::ErrorKind::Interrupted
                );
                assert_eq!(broker.entries.lock().unwrap().len(), 1);
            }
            drop(response);
            assert!(broker.drain_owned_operations(Duration::ZERO).is_err());
            assert_eq!(
                broker.owned_operation_activity().0,
                1,
                "cancellation retains activity until acknowledgement"
            );
            assert!(broker.start_stream(request(41)).is_err());
            assert_eq!(cancelled.load(Ordering::Acquire), 0);
            allow_cancel.store(true, Ordering::Release);
            broker
                .drain_owned_operations(Duration::from_secs(2))
                .unwrap();
            assert_eq!(broker.owned_operation_activity().0, 0);
            assert_eq!(cancelled.load(Ordering::Acquire), native);
        }
    }

    #[test]
    fn buffered_operations_between_polls_preserve_completion_time() {
        let state = Arc::new(Mutex::new(State::default()));
        let broker = SharedNetworkBroker::new(Recording(state));
        let first = broker.clone().new_request_scope();
        let sibling = broker.new_request_scope();
        assert_eq!(first.owned_operation_activity(), (0, None));
        let before = std::time::Instant::now();
        first.send(request(51)).unwrap();
        let snapshot = first.owned_operation_activity();
        assert_eq!(snapshot.0, 0);
        assert!(snapshot.1.is_some_and(|changed| changed >= before));
        assert_eq!(sibling.owned_operation_activity(), (0, None));
        assert_eq!(
            first.owned_operation_activity(),
            snapshot,
            "inspection must not restart quiet"
        );
    }

    #[test]
    fn exhausted_stream_releases_identity_before_reader_drop() {
        struct Buffered;
        impl NetworkBroker for Buffered {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: Vec::new(),
                    body: vec![7],
                })
            }
        }
        let broker = IdentityBroker::new(Arc::new(Box::new(Buffered)));
        let request = NetworkRequest {
            request_id: 17,
            method: "GET".into(),
            url: "https://example.test/".into(),
            headers: Vec::new(),
            body: Vec::new(),
            credentials: false,
            security: Default::default(),
            resolved_addrs: Vec::new(),
        };
        let mut response = broker.start_stream(request).unwrap();
        assert_eq!(broker.entries.lock().unwrap().len(), 1);
        assert_eq!(response.body.read(&mut []).unwrap(), 0);
        assert_eq!(broker.entries.lock().unwrap().len(), 1);
        let mut bytes = [0; 1];
        assert_eq!(response.body.read(&mut bytes).unwrap(), 1);
        assert_eq!(bytes, [7]);
        assert_eq!(broker.entries.lock().unwrap().len(), 1);
        assert_eq!(response.body.read(&mut bytes).unwrap(), 0);
        assert!(broker.entries.lock().unwrap().is_empty());
        broker.drain_owned_operations(Duration::ZERO).unwrap();
        assert_eq!(response.body.read(&mut bytes).unwrap(), 0);
    }

    #[derive(Default)]
    struct State {
        started: Vec<u64>,
        cancelled: Vec<u64>,
        senders: BTreeMap<u64, tokio::sync::mpsc::Sender<NetworkEvent>>,
        fail_cancel: bool,
        mismatch_burst: bool,
    }
    struct Recording(Arc<Mutex<State>>);
    impl NetworkBroker for Recording {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: Vec::new(),
                body: Vec::new(),
            })
        }
        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            let (sender, events) = tokio::sync::mpsc::channel(16);
            sender
                .try_send(NetworkEvent::Headers {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: Vec::new(),
                    content_length: None,
                    upload: None,
                })
                .unwrap();
            let mut state = self.0.lock().unwrap();
            if state.mismatch_burst {
                for _ in 0..7 {
                    sender
                        .try_send(NetworkEvent::Chunk {
                            request_id: request.request_id,
                            bytes: vec![1].into(),
                        })
                        .unwrap();
                }
                sender
                    .try_send(NetworkEvent::Complete {
                        encoded_body_size: None,
                        request_id: request.request_id.checked_add(1).unwrap(),
                    })
                    .unwrap();
            }
            state.started.push(request.request_id);
            state.senders.insert(request.request_id, sender);
            Ok(NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: request.request_id,
                events,
                demand: None,
            })
        }
        fn cancel_operation(&self, id: u64) -> Result<(), NetworkError> {
            let mut state = self.0.lock().unwrap();
            if std::mem::take(&mut state.fail_cancel) {
                return Err(NetworkError::Transport(
                    "injected cancellation failure".into(),
                ));
            }
            state.cancelled.push(id);
            state.senders.remove(&id);
            Ok(())
        }
    }
    fn request(id: u64) -> NetworkRequest {
        NetworkRequest {
            request_id: id,
            method: "GET".into(),
            url: "https://example.test/".into(),
            headers: Vec::new(),
            body: Vec::new(),
            credentials: false,
            security: Default::default(),
            resolved_addrs: Vec::new(),
        }
    }
    fn next(operation: &mut NetworkOperation) -> Option<NetworkEvent> {
        operation_runtime().block_on(async {
            tokio::time::timeout(Duration::from_secs(2), operation.next_event())
                .await
                .unwrap()
        })
    }
    #[test]
    fn distinct_native_scopes_preserve_public_ids_and_cannot_cancel_each_other() {
        let state = Arc::new(Mutex::new(State::default()));
        let base = SharedNetworkBroker::new(Recording(state.clone()));
        let first = base.clone().new_request_scope();
        let replacement = base.new_request_scope();
        let mut one = first.start_operation(request(u64::MAX)).unwrap();
        let mut two = replacement.start_operation(request(u64::MAX)).unwrap();
        assert!(matches!(
            next(&mut one),
            Some(NetworkEvent::Headers {
                request_id: u64::MAX,
                ..
            })
        ));
        assert!(matches!(
            next(&mut two),
            Some(NetworkEvent::Headers {
                request_id: u64::MAX,
                ..
            })
        ));
        let ids = state.lock().unwrap().started.clone();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        assert!(first.start_operation(request(u64::MAX)).is_err());
        first.cancel_operation(u64::MAX).unwrap();
        assert!(next(&mut one).is_none());
        first
            .drain_owned_operations(Duration::from_secs(2))
            .unwrap();
        assert!(replacement.drain_owned_operations(Duration::ZERO).is_err());
        {
            let state = state.lock().unwrap();
            assert_eq!(state.cancelled, vec![ids[0]]);
            assert!(state.senders.contains_key(&ids[1]));
        }
        replacement.cancel_operation(u64::MAX).unwrap();
        assert!(next(&mut two).is_none());
        replacement
            .drain_owned_operations(Duration::from_secs(2))
            .unwrap();
    }
    #[test]
    fn failed_cancel_preserves_native_identity_for_acknowledged_retry() {
        let state = Arc::new(Mutex::new(State {
            fail_cancel: true,
            ..Default::default()
        }));
        let broker = IdentityBroker::new(Arc::new(Box::new(Recording(state.clone()))));
        let mut operation = broker.start_operation(request(7)).unwrap();
        let _ = next(&mut operation);
        assert!(broker.cancel_operation(7).is_err());
        assert_eq!(broker.entries.lock().unwrap().len(), 1);
        assert!(broker.start_operation(request(7)).is_err());
        broker.cancel_operation(7).unwrap();
        assert!(next(&mut operation).is_none());
        assert_eq!(state.lock().unwrap().cancelled.len(), 1);
    }
    #[test]
    fn dropped_response_receiver_cancels_idle_transport_and_drain_waits_for_retry() {
        for audited in [false, true] {
            let state = Arc::new(Mutex::new(State {
                fail_cancel: true,
                ..Default::default()
            }));
            let broker = SharedNetworkBroker::new(Recording(state.clone()));
            if audited {
                broker.resource_audit().enable(1);
            }
            let mut operation = broker.start_operation(request(19)).unwrap();
            let _ = next(&mut operation);
            assert!(broker.drain_owned_operations(Duration::ZERO).is_err());
            drop(operation);
            broker
                .drain_owned_operations(Duration::from_secs(2))
                .unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.cancelled, state.started);
            assert!(state.senders.is_empty());
        }
    }
    #[test]
    fn cancellation_interrupts_mismatch_diagnostic_when_output_is_full() {
        let state = Arc::new(Mutex::new(State {
            mismatch_burst: true,
            ..Default::default()
        }));
        let broker = SharedNetworkBroker::new(Recording(state.clone()));
        let operation = broker.start_operation(request(29)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let consumed = {
                let state = state.lock().unwrap();
                state.senders.values().all(|sender| sender.capacity() == 16)
            };
            if operation.events.len() == 8 && consumed {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "mismatch did not reach blocked diagnostic delivery"
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        broker.cancel_operation(29).unwrap();
        broker
            .drain_owned_operations(Duration::from_secs(2))
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.cancelled, state.started);
        // The consumer stays alive and does not drain the saturated output.
        assert_eq!(operation.events.len(), 8);
    }
}
