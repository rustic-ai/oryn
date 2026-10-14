use super::*;

#[test]
fn owned_admission_respects_active_budget_and_rejection_cleanup() {
    use crate::idb_owned_admission::{Admission, Ledger, Scope};
    use std::sync::Arc;
    let ledger = Arc::new(Ledger::default());
    let host = Rc::new(MockHost::default());
    let adapter = AdmissionHost::new(host.clone());
    let prepare = || {
        let names = vec!["store".to_owned()];
        let scope = Scope::copy_names(&ledger, &names, 1024 * 1024, 0).unwrap();
        let admission = Admission::new("tx".into(), "db".into(), scope, "readonly".into(), None);
        admission.prepare(&ledger, 1024 * 1024, 0).unwrap()
    };
    for id in 0..ACTIVE_LIMIT {
        adapter
            .state
            .borrow_mut()
            .active
            .insert(HostOperationId(id as u64), Kind::Other);
    }
    assert!(
        adapter
            .submit_owned_idb_admission(
                HostOperationId(1000),
                "https://fixture.invalid".into(),
                prepare(),
            )
            .is_err()
    );
    assert_eq!(adapter.state.borrow().active.len(), ACTIVE_LIMIT);
    assert!(host.submitted.borrow().is_empty());
    assert_eq!(ledger.live_bytes(), 0);
    adapter.state.borrow_mut().active.clear();
    // MockHost intentionally does not support owned transfer; fail-closed
    // endpoint rejection must remove the provisional active correlation.
    assert!(
        adapter
            .submit_owned_idb_admission(
                HostOperationId(1000),
                "https://fixture.invalid".into(),
                prepare(),
            )
            .is_err()
    );
    assert!(adapter.state.borrow().active.is_empty());
    assert!(host.submitted.borrow().is_empty());
    assert_eq!(ledger.live_bytes(), 0);
    adapter.retire();
    assert!(
        adapter
            .submit_owned_idb_admission(
                HostOperationId(1001),
                "https://fixture.invalid".into(),
                prepare(),
            )
            .is_err()
    );
    assert_eq!(ledger.live_bytes(), 0);
}

#[derive(Default)]
struct MockHost {
    completions: RefCell<VecDeque<PlatformHostCompletion>>,
    submitted: RefCell<Vec<HostOperationId>>,
    streams: RefCell<BTreeSet<u64>>,
    max_streams: Cell<usize>,
    reject: Cell<Option<HostOperationId>>,
    retired: Cell<bool>,
    receipts: RefCell<Vec<(HostOperationId, bool)>>,
    abandoned: RefCell<Vec<u64>>,
    retained_cleanup: RefCell<BTreeSet<u64>>,
    cleanup_limit: Cell<Option<usize>>,
}
impl PlatformHost for MockHost {
    fn abandon_http_stream(&self, stream: u64) {
        self.abandoned.borrow_mut().push(stream);
        if self.streams.borrow_mut().remove(&stream) && self.cleanup_limit.get().is_some() {
            self.retained_cleanup.borrow_mut().insert(stream);
        }
    }
    fn acknowledge_completion(&self, id: HostOperationId, accepted: bool) {
        self.receipts.borrow_mut().push((id, accepted));
    }
    fn submit(&self, id: HostOperationId, request: PlatformHostRequest) -> Result<(), String> {
        if matches!(Kind::of(&request), Kind::Start)
            && self
                .cleanup_limit
                .get()
                .is_some_and(|limit| self.retained_cleanup.borrow().len() >= limit)
        {
            return Err("native retained cleanup capacity".into());
        }
        if self.reject.get() == Some(id) {
            return Err("injected submit failure".into());
        }
        self.submitted.borrow_mut().push(id);
        let result = match request {
            PlatformHostRequest::HttpStart { request } => {
                self.streams.borrow_mut().insert(id.0);
                self.max_streams
                    .set(self.max_streams.get().max(self.streams.borrow().len()));
                PlatformHostResult::HttpStarted {
                    timing: None,
                    internal_response: None,
                    response_type: None,
                    upload: None,
                    cors_filtered: false,
                    redirected: false,
                    stream_id: id.0,
                    status: 200,
                    url: request.url,
                    headers: Vec::new(),
                }
            }
            PlatformHostRequest::HttpRead { stream_id, .. } => {
                assert!(self.streams.borrow_mut().remove(&stream_id));
                PlatformHostResult::HttpChunk {
                    encoded_body_size: None,
                    stream_id,
                    bytes: Vec::new(),
                    done: true,
                }
            }
            PlatformHostRequest::HttpAbort { stream_id } => {
                assert!(self.streams.borrow_mut().remove(&stream_id));
                PlatformHostResult::HttpAborted
            }
            _ => PlatformHostResult::WebSocketClosed {
                code: 1000,
                reason: String::new(),
                was_clean: true,
            },
        };
        self.completions
            .borrow_mut()
            .push_back(PlatformHostCompletion {
                operation_id: id,
                result: Ok(result),
            });
        Ok(())
    }
    fn cancel(&self, _: HostOperationId) {}
    fn retire(&self) {
        self.retired.set(true);
        self.streams.borrow_mut().clear();
    }
    fn drain_completions(&self, limit: usize) -> Vec<PlatformHostCompletion> {
        (0..limit)
            .filter_map(|_| self.completions.borrow_mut().pop_front())
            .collect()
    }
}
fn start() -> PlatformHostRequest {
    PlatformHostRequest::HttpStart {
        request: PlatformHttpRequest {
            upgrade_mixed_image: false,
            fetch: None,
            use_cors_preflight: false,
            method: "GET".into(),
            url: "https://fixture.test/".into(),
            headers: vec![],
            body: vec![],
            credentials: "omit".into(),
            initiator_url: None,
            referrer_policy: None,
        },
    }
}
fn other() -> PlatformHostRequest {
    PlatformHostRequest::WebSocketClose {
        socket_id: 1,
        code: None,
        reason: String::new(),
    }
}

#[test]
fn burst_drains_with_bounded_operations_and_streams() {
    let mock = Rc::new(MockHost::default());
    let host = AdmissionHost::new(mock.clone());
    for id in 1..=288 {
        host.submit(HostOperationId(id), start()).unwrap();
    }
    assert_eq!(mock.submitted.borrow().len(), ACTIVE_LIMIT);
    let mut completed = 0;
    let mut headers = 0;
    for _ in 0..1200 {
        for completion in host.drain_completions(1) {
            match completion.result.unwrap() {
                PlatformHostResult::HttpStarted { stream_id, .. } => {
                    headers += 1;
                    host.submit(
                        HostOperationId(1000 + stream_id),
                        PlatformHostRequest::HttpRead {
                            stream_id,
                            max_bytes: 1024,
                        },
                    )
                    .unwrap();
                }
                PlatformHostResult::HttpChunk { done: true, .. } => completed += 1,
                _ => panic!("unexpected completion"),
            }
        }
        assert!(host.state.borrow().active.len() <= ACTIVE_LIMIT);
        if completed == 288 {
            break;
        }
    }
    assert_eq!((headers, completed), (288, 288));
    assert_eq!(mock.max_streams.get(), ACTIVE_LIMIT);
    assert!(host.state.borrow().streams.is_empty());
    assert_eq!(host.state.borrow().bytes, 0);
    assert!(host.state.borrow().queued.is_empty());
}

#[test]
fn queued_cancellation_is_bounded_and_never_dispatches() {
    let mock = Rc::new(MockHost::default());
    let host = AdmissionHost::new(mock.clone());
    for id in 0..PENDING_LIMIT {
        host.submit(HostOperationId(id as u64), other()).unwrap();
    }
    let bytes = host.state.borrow().bytes;
    host.cancel(HostOperationId(32));
    assert!(host.state.borrow().bytes < bytes);
    assert!(host.submit(HostOperationId(999), other()).is_err());
    let completion = host.drain_completions(1).pop().unwrap();
    assert_eq!(completion.operation_id, HostOperationId(32));
    assert!(completion.result.is_err());
    host.submit(HostOperationId(999), other()).unwrap();
    for _ in 0..PENDING_LIMIT {
        host.drain_completions(32);
    }
    assert!(!mock.submitted.borrow().contains(&HostOperationId(32)));
}

#[test]
fn cancellation_racing_headers_closes_stream_before_acknowledgment() {
    let mock = Rc::new(MockHost::default());
    let host = AdmissionHost::new(mock.clone());
    host.submit(HostOperationId(1), start()).unwrap();
    host.cancel(HostOperationId(1));
    assert_eq!(host.state.borrow().active.len(), 1);
    assert!(host.drain_completions(1).is_empty());
    assert!(mock.streams.borrow().is_empty());
    let completion = host.drain_completions(1).pop().unwrap();
    assert_eq!(completion.operation_id, HostOperationId(1));
    assert!(matches!(
        completion.result,
        Ok(PlatformHostResult::HttpAborted)
    ));
    assert!(host.state.borrow().streams.is_empty());
    assert!(host.state.borrow().active.is_empty());
    assert!(host.drain_completions(1).is_empty());
}

#[test]
fn byte_limit_duplicate_identity_and_retirement_fail_closed() {
    let mock = Rc::new(MockHost::default());
    let host = AdmissionHost::new(mock.clone());
    for id in 0..ACTIVE_LIMIT {
        host.submit(HostOperationId(id as u64), other()).unwrap();
    }
    assert!(host.submit(HostOperationId(0), other()).is_err());
    let mut request = start();
    if let PlatformHostRequest::HttpStart { request } = &mut request {
        request.body = vec![255; QUEUED_BYTE_LIMIT / 4];
    }
    assert!(
        host.submit(HostOperationId(99), request)
            .unwrap_err()
            .contains("byte budget")
    );
    assert_eq!(host.state.borrow().bytes, 0);
    host.submit(HostOperationId(100), other()).unwrap();
    host.retire();
    host.retire();
    assert!(mock.retired.get());
    assert_eq!(host.state.borrow().bytes, 0);
    assert!(host.submit(HostOperationId(101), other()).is_err());
    assert!(host.drain_completions(32).is_empty());
    assert!(!mock.submitted.borrow().contains(&HostOperationId(100)));
}

#[test]
fn queued_submission_failure_is_delivered_once_and_releases_slot() {
    let mock = Rc::new(MockHost::default());
    let host = AdmissionHost::new(mock.clone());
    for id in 0..33 {
        host.submit(HostOperationId(id), other()).unwrap();
    }
    mock.reject.set(Some(HostOperationId(32)));
    assert_eq!(host.drain_completions(32).len(), 32);
    let completion = host.drain_completions(32).pop().unwrap();
    assert_eq!(completion.operation_id, HostOperationId(32));
    assert_eq!(completion.result, Err("injected submit failure".into()));
    assert!(host.state.borrow().active.is_empty());
    assert!(host.drain_completions(32).is_empty());
}

#[test]
fn cancelled_open_rejects_native_receipt_even_when_abort_submission_fails() {
    let native = Rc::new(MockHost::default());
    let host = AdmissionHost::new(native.clone());
    let id = HostOperationId(901);
    host.submit(id, start()).unwrap();
    host.cancel(id);
    native.reject.set(Some(id));
    let completion = host.drain_completions(1);
    assert_eq!(completion.len(), 1);
    assert!(completion[0].result.is_err());
    assert_eq!(&*native.receipts.borrow(), &[(id, false)]);
    assert!(
        !host.state.borrow().streams.contains(&id.0),
        "delivery lease transferred to native cleanup"
    );
}

#[test]
fn queued_read_failure_and_abort_cancellation_signal_retained_cleanup() {
    for cancel in [false, true] {
        let host = Rc::new(MockHost::default());
        let admission = AdmissionHost::new(host.clone());
        host.streams.borrow_mut().insert(811);
        host.streams.borrow_mut().insert(812);
        let operation = HostOperationId(900);
        {
            let mut state = admission.state.borrow_mut();
            state.streams.insert(811);
            state.streams.insert(812);
            for id in 0..ACTIVE_LIMIT {
                state.active.insert(HostOperationId(id as u64), Kind::Other);
            }
        }
        let request = if cancel {
            PlatformHostRequest::HttpAbort { stream_id: 811 }
        } else {
            PlatformHostRequest::HttpRead {
                stream_id: 811,
                max_bytes: 1,
            }
        };
        admission.submit(operation, request).unwrap();
        assert_eq!(admission.state.borrow().queued.len(), 1);
        if cancel {
            admission.cancel(operation);
        } else {
            host.reject.set(Some(operation));
            admission.state.borrow_mut().active.clear();
            admission.pump();
        }
        assert_eq!(*host.abandoned.borrow(), vec![811]);
        assert!(!host.streams.borrow().contains(&811));
        assert!(host.streams.borrow().contains(&812));
        assert!(!admission.state.borrow().streams.contains(&811));
        assert!(admission.state.borrow().streams.contains(&812));
        assert!(
            admission
                .state
                .borrow()
                .ready
                .iter()
                .any(
                    |completion| completion.operation_id == operation && completion.result.is_err()
                )
        );
        assert!(
            !host.retired.get(),
            "exact native cleanup preserves sibling streams"
        );
    }
}

#[test]
fn cleanup_transfer_reopens_delivery_quota_without_claiming_physical_release() {
    let native = Rc::new(MockHost::default());
    native.cleanup_limit.set(Some(2));
    let admission = AdmissionHost::new(native.clone());
    for stream in 0..ACTIVE_LIMIT as u64 {
        native.streams.borrow_mut().insert(stream);
        admission.state.borrow_mut().streams.insert(stream);
    }
    admission.submit(HostOperationId(1000), start()).unwrap();
    assert_eq!(admission.state.borrow().queued.len(), 1);
    admission.abandon_http_stream(0);
    admission.pump();
    assert!(
        native.retained_cleanup.borrow().contains(&0),
        "no physical ACK yet"
    );
    assert!(admission.state.borrow().queued.is_empty());
    let completion = admission.drain_completions(1);
    assert!(matches!(
        completion[0].result,
        Ok(PlatformHostResult::HttpStarted {
            stream_id: 1000,
            ..
        })
    ));
    assert_eq!(admission.state.borrow().streams.len(), ACTIVE_LIMIT);
    admission.abandon_http_stream(1);
    assert_eq!(native.retained_cleanup.borrow().len(), 2);
    assert!(
        admission.submit(HostOperationId(1001), start()).is_err(),
        "native cleanup bound independently prevents unbounded physical ownership"
    );
    native.retained_cleanup.borrow_mut().remove(&0); // terminal native cleanup ACK
    admission.submit(HostOperationId(1002), start()).unwrap();
    assert!(native.retained_cleanup.borrow().contains(&1));
}

#[test]
fn asynchronous_abort_failure_transfers_exact_stream_and_keeps_sibling() {
    let native = Rc::new(MockHost::default());
    native.cleanup_limit.set(Some(2));
    let admission = AdmissionHost::new(native.clone());
    for stream in [71, 72] {
        native.streams.borrow_mut().insert(stream);
        admission.state.borrow_mut().streams.insert(stream);
    }
    admission
        .state
        .borrow_mut()
        .active
        .insert(HostOperationId(8), Kind::Abort(71));
    admission.receive(PlatformHostCompletion {
        operation_id: HostOperationId(8),
        result: Err("abort transport failure".into()),
    });
    assert_eq!(*native.retained_cleanup.borrow(), BTreeSet::from([71]));
    assert_eq!(admission.state.borrow().streams, BTreeSet::from([72]));
    assert_eq!(admission.drain_completions(1).len(), 1);
}

#[test]
fn immediate_stream_rejection_leaves_one_cleanup_transfer_to_caller() {
    for request in [
        PlatformHostRequest::HttpRead {
            stream_id: 91,
            max_bytes: 1,
        },
        PlatformHostRequest::HttpAbort { stream_id: 91 },
    ] {
        let native = Rc::new(MockHost::default());
        let admission = AdmissionHost::new(native.clone());
        native.streams.borrow_mut().extend([91, 92]);
        admission.state.borrow_mut().streams.extend([91, 92]);
        let id = HostOperationId(701);
        native.reject.set(Some(id));
        assert!(admission.submit(id, request).is_err());
        assert!(
            native.abandoned.borrow().is_empty(),
            "synchronous caller still owns cleanup"
        );
        admission.abandon_http_stream(91);
        assert_eq!(*native.abandoned.borrow(), vec![91]);
        assert!(!admission.state.borrow().active.contains_key(&id));
        assert!(!native.streams.borrow().contains(&91));
        assert!(native.streams.borrow().contains(&92));
    }
}
