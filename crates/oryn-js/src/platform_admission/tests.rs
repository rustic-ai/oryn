use super::*;

#[derive(Default)]
struct MockHost {
    completions: RefCell<VecDeque<PlatformHostCompletion>>,
    submitted: RefCell<Vec<HostOperationId>>,
    streams: RefCell<BTreeSet<u64>>,
    max_streams: Cell<usize>,
    reject: Cell<Option<HostOperationId>>,
    retired: Cell<bool>,
}
impl PlatformHost for MockHost {
    fn submit(&self, id: HostOperationId, request: PlatformHostRequest) -> Result<(), String> {
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
                    stream_id: id.0,
                    status: 200,
                    url: request.url,
                    headers: Vec::new(),
                }
            }
            PlatformHostRequest::HttpRead { stream_id, .. } => {
                assert!(self.streams.borrow_mut().remove(&stream_id));
                PlatformHostResult::HttpChunk {
                    stream_id,
                    bytes: Vec::new(),
                    done: true,
                }
            }
            PlatformHostRequest::HttpAbort { stream_id } => {
                assert!(self.streams.borrow_mut().remove(&stream_id));
                PlatformHostResult::HttpAborted
            }
            _ => PlatformHostResult::WebSocketClosed,
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
