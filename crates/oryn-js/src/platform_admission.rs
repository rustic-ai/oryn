//! Bounded admission between a realm and its parent-owned platform endpoint.
//! Queuing grants no authority: requests retain their original route and policy.
use crate::platform::*;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write;
use std::rc::Rc;

pub const ACTIVE_LIMIT: usize = 32;
pub const PENDING_LIMIT: usize = ACTIVE_LIMIT + 256;
const QUEUED_BYTE_LIMIT: usize = 16 * 1024 * 1024;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
enum Kind {
    Start,
    Read(u64),
    Abort(u64),
    Other,
}
impl Kind {
    fn of(request: &PlatformHostRequest) -> Self {
        match request {
            PlatformHostRequest::HttpStart { .. } | PlatformHostRequest::ImageStart { .. } => {
                Self::Start
            }
            PlatformHostRequest::HttpRead { stream_id, .. } => Self::Read(*stream_id),
            PlatformHostRequest::HttpAbort { stream_id } => Self::Abort(*stream_id),
            _ => Self::Other,
        }
    }
}
struct Queued {
    id: HostOperationId,
    request: PlatformHostRequest,
    bytes: usize,
}
#[derive(Default)]
struct State {
    active: BTreeMap<HostOperationId, Kind>,
    streams: BTreeSet<u64>,
    cancelled: BTreeSet<HostOperationId>,
    queued: VecDeque<Queued>,
    ready: VecDeque<PlatformHostCompletion>,
    bytes: usize,
}
impl State {
    fn can_start(&self, kind: Kind) -> bool {
        self.active.len() < ACTIVE_LIMIT
            && (!matches!(kind, Kind::Start)
                || self.streams.len()
                    + self
                        .active
                        .values()
                        .filter(|kind| matches!(kind, Kind::Start))
                        .count()
                    < ACTIVE_LIMIT)
    }
}

pub struct AdmissionHost {
    host: Rc<dyn PlatformHost>,
    state: RefCell<State>,
    retired: Cell<bool>,
}
impl AdmissionHost {
    pub fn new(host: Rc<dyn PlatformHost>) -> Self {
        Self {
            host,
            state: RefCell::new(State::default()),
            retired: Cell::new(false),
        }
    }

    fn pump(&self) {
        loop {
            let next = {
                let mut state = self.state.borrow_mut();
                // Skip resource-blocked starts, not runnable requests. Reads
                // and aborts release the leases needed by those starts.
                let Some(index) = state
                    .queued
                    .iter()
                    .position(|queued| state.can_start(Kind::of(&queued.request)))
                else {
                    break;
                };
                let next = state.queued.remove(index).expect("selected queue entry");
                state.bytes -= next.bytes;
                state.active.insert(next.id, Kind::of(&next.request));
                next
            };
            if let Err(error) = self.host.submit(next.id, next.request) {
                let mut state = self.state.borrow_mut();
                state.active.remove(&next.id);
                state.ready.push_back(PlatformHostCompletion {
                    operation_id: next.id,
                    result: Err(error),
                });
            }
        }
    }

    fn receive(&self, completion: PlatformHostCompletion) {
        let id = completion.operation_id;
        let (kind, cancelled) = {
            let mut state = self.state.borrow_mut();
            (state.active.remove(&id), state.cancelled.remove(&id))
        };
        let mut abort = None;
        {
            let mut state = self.state.borrow_mut();
            match (&completion.result, kind) {
                (Ok(PlatformHostResult::HttpStarted { stream_id, .. }), Some(Kind::Start)) => {
                    state.streams.insert(*stream_id);
                    if cancelled {
                        abort = Some(*stream_id);
                    }
                }
                (
                    Ok(PlatformHostResult::HttpChunk { done: true, .. }),
                    Some(Kind::Read(stream)),
                )
                | (Err(_), Some(Kind::Read(stream)))
                | (Ok(PlatformHostResult::HttpAborted), Some(Kind::Abort(stream))) => {
                    state.streams.remove(&stream);
                }
                (_, Some(Kind::Read(stream))) if cancelled => {
                    abort = Some(stream);
                }
                _ => (),
            }
            if let Some(stream) = abort {
                // Cancellation can race a successfully opened stream. Reuse
                // this still-undelivered operation's correlation for cleanup,
                // then deliver exactly one completion to its cancellation sink.
                state.active.insert(id, Kind::Abort(stream));
                state.cancelled.insert(id);
            } else {
                state.ready.push_back(completion);
            }
        }
        if let Some(stream_id) = abort
            && let Err(error) = self
                .host
                .submit(id, PlatformHostRequest::HttpAbort { stream_id })
        {
            let mut state = self.state.borrow_mut();
            state.active.remove(&id);
            state.cancelled.remove(&id);
            // Keep the stream lease: failed cleanup is not proof of release.
            state.ready.push_back(PlatformHostCompletion {
                operation_id: id,
                result: Err(error),
            });
        }
    }
}

// Count serialized request bytes without allocating another payload. This is
// an admission budget, not a claim to measure total Rust or V8 heap usage.
fn request_bytes(request: &PlatformHostRequest, limit: usize) -> Result<usize, String> {
    struct Counter {
        bytes: usize,
        limit: usize,
    }
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.bytes) {
                return Err(std::io::Error::other(
                    "queued platform request byte budget exceeded",
                ));
            }
            self.bytes += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { bytes: 0, limit };
    serde_json::to_writer(&mut counter, request).map_err(|error| error.to_string())?;
    Ok(counter.bytes)
}

impl PlatformHost for AdmissionHost {
    fn submit(&self, id: HostOperationId, request: PlatformHostRequest) -> Result<(), String> {
        if self.retired.get() {
            return Err("execution environment is retired".into());
        }
        let kind = Kind::of(&request);
        let mut state = self.state.borrow_mut();
        if state.active.contains_key(&id)
            || state.queued.iter().any(|entry| entry.id == id)
            || state.ready.iter().any(|entry| entry.operation_id == id)
        {
            return Err("duplicate platform operation identity".into());
        }
        if state.active.len() + state.queued.len() + state.ready.len() >= PENDING_LIMIT {
            return Err("platform admission count budget exceeded".into());
        }
        if state.can_start(kind)
            && !state
                .queued
                .iter()
                .any(|entry| state.can_start(Kind::of(&entry.request)))
        {
            state.active.insert(id, kind);
            drop(state);
            if let Err(error) = self.host.submit(id, request) {
                self.state.borrow_mut().active.remove(&id);
                return Err(error);
            }
        } else {
            if state.queued.len() >= PENDING_LIMIT - ACTIVE_LIMIT {
                return Err("platform admission queue budget exceeded".into());
            }
            let bytes = request_bytes(&request, QUEUED_BYTE_LIMIT.saturating_sub(state.bytes))?;
            state.bytes += bytes;
            state.queued.push_back(Queued { id, request, bytes });
            drop(state);
            self.pump();
        }
        Ok(())
    }

    fn cancel(&self, id: HostOperationId) {
        if self.retired.get() {
            return;
        }
        let mut state = self.state.borrow_mut();
        if let Some(index) = state.queued.iter().position(|entry| entry.id == id) {
            let entry = state.queued.remove(index).expect("queued cancellation");
            state.bytes -= entry.bytes;
            state.ready.push_back(PlatformHostCompletion {
                operation_id: id,
                result: Err("operation cancelled before dispatch".into()),
            });
        } else if state.active.contains_key(&id) && state.cancelled.insert(id) {
            drop(state);
            // Do not free an active slot until the host acknowledges it.
            self.host.cancel(id);
        }
    }

    fn retire(&self) {
        if !self.retired.replace(true) {
            self.host.retire();
            *self.state.borrow_mut() = State::default();
        }
    }

    fn drain_completions(&self, limit: usize) -> Vec<PlatformHostCompletion> {
        if self.retired.get() || limit == 0 {
            return Vec::new();
        }
        let budget = limit.min(ACTIVE_LIMIT);
        let ready = self.state.borrow().ready.len().min(budget);
        for completion in self.host.drain_completions(budget - ready) {
            self.receive(completion);
        }
        let mut output = Vec::new();
        {
            let mut state = self.state.borrow_mut();
            for _ in 0..budget {
                let Some(completion) = state.ready.pop_front() else {
                    break;
                };
                output.push(completion);
            }
        }
        self.pump();
        output
    }
}
