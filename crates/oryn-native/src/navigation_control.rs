//! Parent-owned navigation lifecycle and cooperative cancellation.
//!
//! Native loaders publish milestones for the exact admitted operation. Waiting
//! for WebDriver readiness never requires taking the page actor's command lock.
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageLoadStrategy {
    None,
    Eager,
    Normal,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NavigationCommand {
    Navigate { url: String },
    Back,
    Forward,
    Refresh,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigationRequest {
    pub command: NavigationCommand,
    pub strategy: PageLoadStrategy,
    pub timeout_ms: Option<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NavigationId(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NavigationPhase {
    Queued,
    Started,
    Committed,
    Interactive,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum NavigationStop {
    #[error("page load deadline expired")]
    Timeout,
    #[error("navigation superseded by another navigation")]
    Superseded,
    #[error("navigation cancelled")]
    Cancelled,
    #[error("page retired")]
    Retired,
    #[error("server certificate rejected: {0}")]
    CertificateRejected(String),
    #[error("native navigation failed: {0}")]
    Failed(String),
    #[error("invalid navigation lifecycle transition")]
    InvalidTransition,
    #[error("navigation identity space exhausted")]
    IdentityExhausted,
    #[error("navigation state lock poisoned")]
    Poisoned,
}
/// Native-only events published through the page's authenticated context route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NavigationEvent {
    Started,
    Committed {
        document_generation: u64,
        url: String,
    },
    Interactive {
        document_generation: u64,
    },
    Complete {
        document_generation: u64,
    },
    CompleteWithoutCommit,
    Stopped(NavigationStop),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigationProgress {
    pub id: NavigationId,
    pub phase: NavigationPhase,
    pub document_generation: Option<u64>,
    pub url: Option<String>,
    pub stopped: Option<NavigationStop>,
}
#[derive(Debug)]
struct JobState {
    progress: NavigationProgress,
    finished: bool,
    readiness_reached: bool,
    #[cfg(unix)]
    navigation_grant: Option<crate::runtime::NavigationChainGrant>,
}
#[derive(Debug)]
pub struct NavigationJob {
    pub id: NavigationId,
    pub request: NavigationRequest,
    deadline: Option<Instant>,
    state: Mutex<JobState>,
    changed: Notify,
}
impl NavigationJob {
    #[cfg(unix)]
    pub(crate) fn install_navigation_grant(
        &self,
        install: impl FnOnce() -> crate::runtime::NavigationChainGrant,
    ) -> Result<(), NavigationStop> {
        let mut state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
        self.check_deadline(&mut state)?;
        if state.finished || state.navigation_grant.is_some() {
            return Err(NavigationStop::InvalidTransition);
        }
        state.navigation_grant = Some(install());
        Ok(())
    }
    pub fn publish(&self, event: NavigationEvent) -> Result<(), NavigationStop> {
        match event {
            NavigationEvent::Started => self.started(),
            NavigationEvent::Committed {
                document_generation,
                url,
            } => self.committed(document_generation, url),
            NavigationEvent::Interactive {
                document_generation,
            } => self.interactive(document_generation),
            NavigationEvent::Complete {
                document_generation,
            } => self.complete(document_generation),
            NavigationEvent::CompleteWithoutCommit => self.complete_without_commit(),
            NavigationEvent::Stopped(reason) => self.stop(reason),
        }
    }
    /// Called by native stream/redirect/task checkpoints. Timeout transitions are
    /// shared with the parent waiter, so a timed-out command cannot keep loading.
    pub fn checkpoint(&self) -> Result<(), NavigationStop> {
        let mut state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
        self.check_deadline(&mut state)
    }
    fn check_deadline(&self, state: &mut JobState) -> Result<(), NavigationStop> {
        if let Some(stop) = &state.progress.stopped {
            return Err(stop.clone());
        }
        let waiting_for_readiness = !state.readiness_reached;
        if !state.finished
            && waiting_for_readiness
            && self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            state.progress.stopped = Some(NavigationStop::Timeout);
            state.finished = true;
            #[cfg(unix)]
            state.navigation_grant.take();
            self.changed.notify_waiters();
            return Err(NavigationStop::Timeout);
        }
        Ok(())
    }
    pub fn progress(&self) -> Result<NavigationProgress, NavigationStop> {
        let mut state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
        let _ = self.check_deadline(&mut state);
        Ok(state.progress.clone())
    }
    pub fn stop(&self, reason: NavigationStop) -> Result<(), NavigationStop> {
        let mut state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
        if !state.finished {
            state.progress.stopped = Some(reason);
            state.finished = true;
            #[cfg(unix)]
            state.navigation_grant.take();
            self.changed.notify_waiters();
        }
        Ok(())
    }
    /// A milestone is published only by the operation's native loader. An old
    /// operation remains cancelled even when its delayed response arrives later.
    pub fn started(&self) -> Result<(), NavigationStop> {
        let mut state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
        self.check_deadline(&mut state)?;
        if state.finished || state.progress.phase != NavigationPhase::Queued {
            return Err(NavigationStop::InvalidTransition);
        }
        state.progress.phase = NavigationPhase::Started;
        state.readiness_reached = self.request.strategy == PageLoadStrategy::None;
        self.changed.notify_waiters();
        Ok(())
    }
    pub fn committed(&self, document_generation: u64, url: String) -> Result<(), NavigationStop> {
        let mut state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
        self.check_deadline(&mut state)?;
        if state.finished
            || state.progress.phase < NavigationPhase::Started
            || state.progress.document_generation == Some(document_generation)
        {
            return Err(NavigationStop::InvalidTransition);
        }
        // A scripted replacement before load belongs to the same outstanding
        // navigation. Old-document lifecycle events cannot satisfy its wait.
        state.progress.phase = NavigationPhase::Committed;
        state.progress.document_generation = Some(document_generation);
        state.progress.url = Some(url);
        self.changed.notify_waiters();
        Ok(())
    }
    pub fn interactive(&self, document_generation: u64) -> Result<(), NavigationStop> {
        self.document_milestone(document_generation, NavigationPhase::Interactive)
    }
    pub fn complete(&self, document_generation: u64) -> Result<(), NavigationStop> {
        self.document_milestone(document_generation, NavigationPhase::Complete)
    }
    fn document_milestone(
        &self,
        generation: u64,
        phase: NavigationPhase,
    ) -> Result<(), NavigationStop> {
        let mut state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
        self.check_deadline(&mut state)?;
        if state.finished
            || state.progress.document_generation != Some(generation)
            || state.progress.phase < NavigationPhase::Committed
            || phase < state.progress.phase
        {
            return Err(NavigationStop::InvalidTransition);
        }
        state.progress.phase = phase;
        state.finished = phase == NavigationPhase::Complete;
        #[cfg(unix)]
        if state.finished {
            state.navigation_grant.take();
        }
        state.readiness_reached |= phase >= self.readiness_phase();
        self.changed.notify_waiters();
        Ok(())
    }
    /// History boundary/no-op navigation or a cancelled beforeunload needs no new
    /// document. Native history owns the decision; the protocol does not invent it.
    pub fn complete_without_commit(&self) -> Result<(), NavigationStop> {
        let mut state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
        self.check_deadline(&mut state)?;
        if state.finished || state.progress.phase != NavigationPhase::Started {
            return Err(NavigationStop::InvalidTransition);
        }
        state.progress.phase = NavigationPhase::Complete;
        state.finished = true;
        state.readiness_reached = true;
        #[cfg(unix)]
        state.navigation_grant.take();
        self.changed.notify_waiters();
        Ok(())
    }
    fn readiness_phase(&self) -> NavigationPhase {
        match self.request.strategy {
            PageLoadStrategy::None => NavigationPhase::Started,
            PageLoadStrategy::Eager => NavigationPhase::Interactive,
            PageLoadStrategy::Normal => NavigationPhase::Complete,
        }
    }
    pub async fn wait_ready(&self) -> Result<NavigationProgress, NavigationStop> {
        let target = self.readiness_phase();
        loop {
            let changed = self.changed.notified();
            {
                let mut state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
                self.check_deadline(&mut state)?;
                if state.readiness_reached || state.progress.phase >= target {
                    return Ok(state.progress.clone());
                }
            }
            if let Some(deadline) = self.deadline {
                tokio::select! {
                    _=changed=>{},
                    _=tokio::time::sleep_until(tokio::time::Instant::from_std(deadline))=>{},
                }
            } else {
                changed.await;
            }
        }
    }
}
#[derive(Debug, Default)]
struct ControlState {
    next_id: u64,
    active: Option<Arc<NavigationJob>>,
    retired: bool,
}
#[derive(Debug, Default)]
pub struct NavigationControl {
    state: Mutex<ControlState>,
}
impl NavigationControl {
    pub fn begin(&self, request: NavigationRequest) -> Result<Arc<NavigationJob>, NavigationStop> {
        let deadline = if request.strategy == PageLoadStrategy::None {
            None
        } else {
            request
                .timeout_ms
                .map(|ms| {
                    Instant::now().checked_add(Duration::from_millis(ms)).ok_or(
                        NavigationStop::Failed("navigation deadline overflow".into()),
                    )
                })
                .transpose()?
        };
        let mut state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
        if state.retired {
            return Err(NavigationStop::Retired);
        }
        let id = state
            .next_id
            .checked_add(1)
            .ok_or(NavigationStop::IdentityExhausted)?;
        if let Some(previous) = &state.active {
            previous.stop(NavigationStop::Superseded)?;
        }
        let job = Arc::new(NavigationJob {
            id: NavigationId(id),
            request,
            deadline,
            state: Mutex::new(JobState {
                progress: NavigationProgress {
                    id: NavigationId(id),
                    phase: NavigationPhase::Queued,
                    document_generation: None,
                    url: None,
                    stopped: None,
                },
                finished: false,
                readiness_reached: false,
                #[cfg(unix)]
                navigation_grant: None,
            }),
            changed: Notify::new(),
        });
        state.next_id = id;
        state.active = Some(job.clone());
        Ok(job)
    }
    pub fn active(&self, id: NavigationId) -> Result<Arc<NavigationJob>, NavigationStop> {
        let state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
        if state.retired {
            return Err(NavigationStop::Retired);
        }
        state
            .active
            .as_ref()
            .filter(|job| job.id == id)
            .cloned()
            .ok_or(NavigationStop::Superseded)
    }
    /// Reset may release a retained cleanup capability only after its original
    /// operation is terminal. Early readiness does not release that custody.
    pub(crate) fn cleanup_is_drained(&self) -> Result<bool, NavigationStop> {
        let state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
        match state.active.as_ref() {
            None => Ok(true),
            Some(job) => Ok(job
                .state
                .lock()
                .map_err(|_| NavigationStop::Poisoned)?
                .finished),
        }
    }
    pub fn retire(&self) -> Result<(), NavigationStop> {
        let mut state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
        state.retired = true;
        if let Some(active) = state.active.take() {
            active.stop(NavigationStop::Retired)?;
        }
        Ok(())
    }
}

/// Acknowledgement of an exact native lifecycle request, not mere enqueue.
#[derive(Clone)]
pub(crate) struct PendingNavigationAck(Arc<Mutex<Option<Result<(), NavigationStop>>>>);
impl PendingNavigationAck {
    #[cfg(all(test, feature = "v8-host"))]
    pub(crate) fn test_pending() -> Self {
        Self::pending()
    }
    #[cfg(all(test, feature = "v8-host"))]
    pub(crate) fn test_finish(&self, result: Result<(), NavigationStop>) {
        self.finish(result)
    }
    #[cfg(feature = "v8-host")]
    fn pending() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }
    fn ready(result: Result<(), NavigationStop>) -> Self {
        Self(Arc::new(Mutex::new(Some(result))))
    }
    #[cfg(feature = "v8-host")]
    fn finish(&self, result: Result<(), NavigationStop>) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = Some(result);
    }
    pub(crate) fn poll(&self) -> std::task::Poll<Result<(), NavigationStop>> {
        match self.0.lock() {
            Ok(state) => state
                .clone()
                .map(std::task::Poll::Ready)
                .unwrap_or(std::task::Poll::Pending),
            Err(_) => std::task::Poll::Ready(Err(NavigationStop::Poisoned)),
        }
    }
}

#[cfg(all(unix, feature = "v8-host"))]
struct NavigationDelivery {
    request: crate::context_service::ContextServiceRequest,
    ack: PendingNavigationAck,
    // Includes queueing and the complete parent response, even if the actor retires.
    _custody: crate::context_service::ResourceJob,
}
#[cfg(all(unix, feature = "v8-host"))]
#[derive(Default)]
struct RemoteNavigationState {
    queue: std::collections::VecDeque<NavigationDelivery>,
    running: bool,
    outstanding: usize,
    stopped: Option<NavigationStop>,
    checkpoint: Option<PendingNavigationAck>,
}
#[cfg(all(unix, feature = "v8-host"))]
pub(crate) struct RemoteNavigation {
    client: crate::context_service::SharedContextClient,
    page_id: crate::PageId,
    id: NavigationId,
    terminal_custody: crate::context_service::ContextNavigationStop,
    state: Mutex<RemoteNavigationState>,
}
#[cfg(all(unix, feature = "v8-host"))]
impl RemoteNavigation {
    fn enqueue(
        self: &Arc<Self>,
        event: Option<NavigationEvent>,
    ) -> Result<PendingNavigationAck, NavigationStop> {
        let mut state = self.state.lock().map_err(|_| NavigationStop::Poisoned)?;
        if let Some(stop) = &state.stopped {
            return Err(stop.clone());
        }
        if event.is_none()
            && let Some(ack) = &state.checkpoint
            && ack.poll().is_pending()
        {
            return Ok(ack.clone());
        }
        if state.outstanding >= 256 {
            let error = NavigationStop::Failed("navigation acknowledgement queue exceeded".into());
            state.stopped = Some(error.clone());
            let queued = state.queue.len();
            for delivery in state.queue.drain(..) {
                delivery.ack.finish(Err(error.clone()));
            }
            state.outstanding -= queued;
            self.terminal_custody.cancel();
            return Err(error);
        }
        let custody = match self.client.track_control_job() {
            Ok(custody) => custody,
            Err(error) => {
                let error = NavigationStop::Failed(error.to_string());
                state.stopped = Some(error.clone());
                let queued = state.queue.len();
                for delivery in state.queue.drain(..) {
                    delivery.ack.finish(Err(error.clone()));
                }
                state.outstanding -= queued;
                self.terminal_custody.cancel();
                return Err(error);
            }
        };
        let ack = PendingNavigationAck::pending();
        let request = match event {
            Some(event) => {
                // A checkpoint enqueued before this event cannot flush it.
                state.checkpoint = None;
                crate::context_service::ContextServiceRequest::NavigationEvent {
                    page_id: self.page_id,
                    id: self.id,
                    event,
                }
            }
            None => {
                state.checkpoint = Some(ack.clone());
                crate::context_service::ContextServiceRequest::NavigationCheckpoint {
                    page_id: self.page_id,
                    id: self.id,
                }
            }
        };
        state.queue.push_back(NavigationDelivery {
            request,
            ack: ack.clone(),
            _custody: custody,
        });
        state.outstanding += 1;
        if !state.running {
            state.running = true;
            let owner = self.clone();
            if let Err(error) =
                crate::executable::resource_io_executor::submit(move || owner.drain())
            {
                let error = NavigationStop::Failed(format!(
                    "navigation acknowledgement admission: {error}"
                ));
                state.running = false;
                state.stopped = Some(error.clone());
                for delivery in state.queue.drain(..) {
                    delivery.ack.finish(Err(error.clone()));
                }
                state.outstanding = 0;
                self.terminal_custody.cancel();
                return Err(error);
            }
        }
        Ok(ack)
    }
    fn drain(self: Arc<Self>) {
        loop {
            let delivery = {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                match state.queue.pop_front() {
                    Some(delivery) => delivery,
                    None => {
                        state.running = false;
                        return;
                    }
                }
            };
            let terminal = matches!(
                &delivery.request,
                crate::context_service::ContextServiceRequest::NavigationEvent {
                    event: NavigationEvent::Complete { .. }
                        | NavigationEvent::CompleteWithoutCommit
                        | NavigationEvent::Stopped(_),
                    ..
                }
            );
            let mut parent_terminal = false;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.client.request(delivery.request)
            }))
            .map_err(|_| {
                NavigationStop::Failed("navigation acknowledgement transport panicked".into())
            })
            .and_then(|result| match result {
                Ok(crate::context_service::ContextServiceResponse::NavigationControl {
                    result,
                }) => {
                    parent_terminal = matches!(
                        &result,
                        Err(NavigationStop::Timeout
                            | NavigationStop::Superseded
                            | NavigationStop::Cancelled
                            | NavigationStop::Retired
                            | NavigationStop::Failed(_))
                    );
                    result
                }
                Err(error) => Err(NavigationStop::Failed(error.to_string())),
                _ => Err(NavigationStop::InvalidTransition),
            });
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.outstanding -= 1;
            if let Err(error) = &result {
                state.stopped = Some(error.clone());
                for queued in state.queue.drain(..) {
                    queued.ack.finish(Err(error.clone()));
                }
                state.outstanding = 0;
                state.running = false;
                if !parent_terminal {
                    self.terminal_custody.cancel();
                }
            }
            if parent_terminal || (terminal && result.is_ok()) {
                self.terminal_custody.finish();
            }
            delivery.ack.finish(result.clone());
            // delivery custody drops only after the parent response is consumed.
            if result.is_err() {
                return;
            }
        }
    }
}

#[derive(Clone)]
pub(crate) enum NavigationEmitter {
    Local(Arc<NavigationJob>),
    #[cfg(all(unix, feature = "v8-host"))]
    Remote(Arc<RemoteNavigation>),
}
impl NavigationEmitter {
    #[cfg(any(feature = "v8-host", test))]
    pub(crate) fn same_job(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Local(left), Self::Local(right)) => Arc::ptr_eq(left, right),
            #[cfg(all(unix, feature = "v8-host"))]
            (Self::Remote(left), Self::Remote(right)) => Arc::ptr_eq(left, right),
            #[allow(unreachable_patterns)]
            _ => false,
        }
    }
    #[cfg(all(unix, feature = "v8-host"))]
    pub(crate) fn remote(
        client: crate::context_service::SharedContextClient,
        page_id: crate::PageId,
        id: NavigationId,
    ) -> Result<Self, crate::RuntimeError> {
        let terminal_custody = client.reserve_navigation_stop(page_id, id)?;
        Ok(Self::Remote(Arc::new(RemoteNavigation {
            client,
            page_id,
            id,
            terminal_custody,
            state: Mutex::new(RemoteNavigationState::default()),
        })))
    }
    pub(crate) fn cached_stop(&self) -> Option<NavigationStop> {
        match self {
            Self::Local(job) => job.checkpoint().err(),
            #[cfg(all(unix, feature = "v8-host"))]
            Self::Remote(remote) => match remote.state.lock() {
                Ok(state) => state.stopped.clone(),
                Err(_) => Some(NavigationStop::Poisoned),
            },
        }
    }
    /// Nonblocking cached observation for parser callbacks. Actors must retain
    /// checkpoint_pending acknowledgements at yield/commit boundaries.
    pub(crate) fn checkpoint(&self) -> Result<(), NavigationStop> {
        self.cached_stop().map_or(Ok(()), Err)
    }
    #[cfg(any(feature = "v8-host", test))]
    pub(crate) fn checkpoint_pending(&self) -> Result<PendingNavigationAck, NavigationStop> {
        match self {
            Self::Local(job) => Ok(PendingNavigationAck::ready(job.checkpoint())),
            #[cfg(all(unix, feature = "v8-host"))]
            Self::Remote(remote) => remote.enqueue(None),
        }
    }
    /// Flush prior publications. A latched failure is a completed negative
    /// acknowledgement, never a successful flush or a reason to retry events.
    #[cfg(any(feature = "v8-host", test))]
    pub(crate) fn flush_pending(&self) -> Result<PendingNavigationAck, NavigationStop> {
        if let Some(stop) = self.cached_stop() {
            return Ok(PendingNavigationAck::ready(Err(stop)));
        }
        self.checkpoint_pending()
    }
    pub(crate) fn publish_pending(
        &self,
        event: NavigationEvent,
    ) -> Result<PendingNavigationAck, NavigationStop> {
        match self {
            Self::Local(job) => Ok(PendingNavigationAck::ready(job.publish(event))),
            #[cfg(all(unix, feature = "v8-host"))]
            Self::Remote(remote) => remote.enqueue(Some(event)),
        }
    }
    /// Accept a milestone in order. Remote acceptance is not a parent ACK;
    /// completion callers must await a subsequent ordered checkpoint ACK.
    pub(crate) fn publish(&self, event: NavigationEvent) -> Result<(), NavigationStop> {
        let ack = self.publish_pending(event)?;
        match ack.poll() {
            std::task::Poll::Ready(result) => result,
            std::task::Poll::Pending => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[tokio::test]
    async fn early_readiness_keeps_exact_navigation_grant_until_terminal_lifecycle() {
        let context = crate::Browser::in_process_probe().new_context();
        let page = crate::PageId(71);
        for strategy in [PageLoadStrategy::None, PageLoadStrategy::Eager] {
            let control = NavigationControl::default();
            let job = control.begin(request(strategy)).unwrap();
            job.install_navigation_grant(|| context.grant_navigation_chain(page))
                .unwrap();
            job.started().unwrap();
            if strategy == PageLoadStrategy::Eager {
                job.committed(1, "https://destination.test/".into())
                    .unwrap();
                job.interactive(1).unwrap();
            }
            job.wait_ready().await.unwrap();
            assert!(context.navigation_chain_is_preconfirmed(page));
            if strategy == PageLoadStrategy::None {
                job.committed(1, "https://destination.test/".into())
                    .unwrap();
            }
            job.complete(1).unwrap();
            assert!(!context.navigation_chain_is_preconfirmed(page));
        }
    }

    #[cfg(unix)]
    #[test]
    fn navigation_grants_release_on_stop_timeout_and_no_commit_without_revoking_replacement() {
        let context = crate::Browser::in_process_probe().new_context();
        let page = crate::PageId(72);
        let old = context.grant_navigation_chain(page);
        let current = context.grant_navigation_chain(page);
        drop(old);
        assert!(context.navigation_chain_is_preconfirmed(page));
        drop(current);
        assert!(!context.navigation_chain_is_preconfirmed(page));
        for terminal in ["stop", "timeout", "no-commit", "superseded"] {
            let control = NavigationControl::default();
            let mut job = control.begin(request(PageLoadStrategy::Normal)).unwrap();
            job.install_navigation_grant(|| context.grant_navigation_chain(page))
                .unwrap();
            job.started().unwrap();
            match terminal {
                "stop" => job.stop(NavigationStop::Cancelled).unwrap(),
                "timeout" => {
                    drop(control);
                    Arc::get_mut(&mut job).unwrap().deadline = Some(Instant::now());
                    assert_eq!(job.checkpoint(), Err(NavigationStop::Timeout));
                }
                "no-commit" => job.complete_without_commit().unwrap(),
                _ => {
                    control.begin(request(PageLoadStrategy::None)).unwrap();
                }
            }
            assert!(!context.navigation_chain_is_preconfirmed(page));
            let replacement = context.grant_navigation_chain(page);
            assert!(
                job.install_navigation_grant(|| panic!("terminal job cannot create a new grant"))
                    .is_err()
            );
            drop(job);
            assert!(context.navigation_chain_is_preconfirmed(page));
            drop(replacement);
        }
    }

    #[cfg(all(unix, feature = "v8-host"))]
    #[test]
    fn remote_milestones_are_ordered_and_retained_until_parent_acknowledgement() {
        use crate::context_service::{
            CONTEXT_SERVICE_PROTOCOL_VERSION, ContextServiceRequest, ContextServiceResponse,
            IpcContextClient, SharedContextClient,
        };
        use std::io::{BufRead, BufReader, Write};
        use std::sync::mpsc;
        let (local, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let mut reader = BufReader::new(peer.try_clone().unwrap());
            for index in 0..3 {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let envelope: serde_json::Value = serde_json::from_str(&line).unwrap();
                let request: ContextServiceRequest =
                    serde_json::from_value(envelope["request"].clone()).unwrap();
                match (index, request) {
                    (0 | 2, ContextServiceRequest::NavigationCheckpoint { page_id, id }) => {
                        assert_eq!(page_id, crate::PageId(7));
                        assert_eq!(id, NavigationId(9));
                    }
                    (
                        1,
                        ContextServiceRequest::NavigationEvent {
                            page_id,
                            id,
                            event: NavigationEvent::Started,
                        },
                    ) => {
                        assert_eq!(page_id, crate::PageId(7));
                        assert_eq!(id, NavigationId(9));
                    }
                    _ => panic!("out-of-order navigation publication"),
                }
                if index == 0 {
                    entered.send(()).unwrap();
                    wait.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                let result = if index == 2 {
                    Err(NavigationStop::Timeout)
                } else {
                    Ok(())
                };
                let response = serde_json::json!({"protocol_version":CONTEXT_SERVICE_PROTOCOL_VERSION,"route":null,
                    "correlation_id":envelope["message_id"],"response":ContextServiceResponse::NavigationControl{result},"error":null});
                writeln!(peer, "{response}").unwrap();
                line.clear();
                reader.read_line(&mut line).unwrap();
                let receipt: serde_json::Value = serde_json::from_str(&line).unwrap();
                assert_eq!(receipt["accepted"], true);
            }
        });
        let client = SharedContextClient::new(IpcContextClient::new(local));
        let emitter =
            NavigationEmitter::remote(client.clone(), crate::PageId(7), NavigationId(9)).unwrap();
        let same = emitter.clone();
        assert!(emitter.same_job(&same));
        let first = emitter.checkpoint_pending().unwrap();
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(first.poll().is_pending());
        let coalesced = emitter.checkpoint_pending().unwrap();
        assert!(Arc::ptr_eq(&first.0, &coalesced.0));
        let milestone = emitter.publish_pending(NavigationEvent::Started).unwrap();
        let flush = emitter.flush_pending().unwrap();
        assert!(!Arc::ptr_eq(&first.0, &flush.0));
        assert!(milestone.poll().is_pending());
        assert!(flush.poll().is_pending());
        assert!(client.retire_resource_jobs(Duration::ZERO).is_err());
        drop(emitter);
        drop(first);
        drop(coalesced);
        drop(milestone);
        release.send(()).unwrap();
        server.join().unwrap();
        client
            .drain_resource_teardown(Duration::from_secs(2))
            .unwrap();
        assert_eq!(
            flush.poll(),
            std::task::Poll::Ready(Err(NavigationStop::Timeout))
        );
        assert_eq!(same.cached_stop(), Some(NavigationStop::Timeout));
        assert_eq!(
            same.flush_pending().unwrap().poll(),
            std::task::Poll::Ready(Err(NavigationStop::Timeout))
        );
        assert!(matches!(
            same.publish_pending(NavigationEvent::CompleteWithoutCommit),
            Err(NavigationStop::Timeout)
        ));
    }

    #[test]
    fn local_acknowledgements_preserve_job_identity_and_terminal_failure() {
        let control = NavigationControl::default();
        let old =
            NavigationEmitter::Local(control.begin(request(PageLoadStrategy::Normal)).unwrap());
        assert_eq!(
            old.publish_pending(NavigationEvent::Started)
                .unwrap()
                .poll(),
            std::task::Poll::Ready(Ok(()))
        );
        let new =
            NavigationEmitter::Local(control.begin(request(PageLoadStrategy::Normal)).unwrap());
        assert!(!old.same_job(&new));
        assert!(new.same_job(&new.clone()));
        assert_eq!(
            old.flush_pending().unwrap().poll(),
            std::task::Poll::Ready(Err(NavigationStop::Superseded))
        );
        assert_eq!(
            new.checkpoint_pending().unwrap().poll(),
            std::task::Poll::Ready(Ok(()))
        );
    }

    #[test]
    fn cleanup_controller_stays_retained_after_early_readiness_until_terminal() {
        let control = NavigationControl::default();
        assert_eq!(control.cleanup_is_drained(), Ok(true));
        let first = control.begin(request(PageLoadStrategy::None)).unwrap();
        first.started().unwrap();
        assert_eq!(control.cleanup_is_drained(), Ok(false));
        first.stop(NavigationStop::Cancelled).unwrap();
        assert_eq!(control.cleanup_is_drained(), Ok(true));
        let replacement = control.begin(request(PageLoadStrategy::Eager)).unwrap();
        replacement.started().unwrap();
        replacement
            .committed(7, "https://example.test/".into())
            .unwrap();
        replacement.interactive(7).unwrap();
        assert_eq!(control.cleanup_is_drained(), Ok(false));
        replacement.complete(7).unwrap();
        assert_eq!(control.cleanup_is_drained(), Ok(true));
        let pending = control.begin(request(PageLoadStrategy::None)).unwrap();
        assert_eq!(control.cleanup_is_drained(), Ok(false));
        control.retire().unwrap();
        assert_eq!(control.cleanup_is_drained(), Ok(true));
        assert_eq!(pending.checkpoint(), Err(NavigationStop::Retired));
    }

    #[cfg(all(unix, feature = "v8-host"))]
    #[test]
    fn saturated_or_retired_remote_outbox_stops_parent_job_through_reserved_cleanup() {
        use crate::context_service::{
            ContextServiceRequest, ContextServiceResponse, ContextTransport, IpcContextClient,
            SharedContextClient, serve_context_transport,
        };
        use std::sync::mpsc;
        struct Service {
            job: Arc<NavigationJob>,
            entered: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
            stopped: mpsc::Sender<()>,
        }
        impl ContextTransport for Service {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                let result = match request {
                    ContextServiceRequest::NavigationCheckpoint { page_id, id } => {
                        assert_eq!(page_id, crate::PageId(77));
                        assert_eq!(id, self.job.id);
                        self.entered.send(()).unwrap();
                        self.release
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                        self.job.checkpoint()
                    }
                    ContextServiceRequest::NavigationEvent {
                        page_id,
                        id,
                        event: NavigationEvent::Stopped(reason),
                    } => {
                        assert_eq!(page_id, crate::PageId(77));
                        assert_eq!(id, self.job.id);
                        let result = self.job.stop(reason);
                        self.stopped.send(()).unwrap();
                        result
                    }
                    _ => panic!("discarded milestones must not reach parent"),
                };
                Ok(ContextServiceResponse::NavigationControl { result })
            }
        }
        for retire_resources in [false, true] {
            let control = NavigationControl::default();
            let job = control.begin(request(PageLoadStrategy::None)).unwrap();
            job.started().unwrap();
            let (worker, parent) = std::os::unix::net::UnixStream::pair().unwrap();
            let shutdown = parent.try_clone().unwrap();
            let (entered, waiting) = mpsc::channel();
            let (release, gate) = mpsc::channel();
            let (stopped, stop_seen) = mpsc::channel();
            let service = Arc::new(Service {
                job: job.clone(),
                entered,
                release: Mutex::new(gate),
                stopped,
            });
            let server = std::thread::spawn(move || serve_context_transport(parent, service));
            let client = SharedContextClient::new(IpcContextClient::new(worker));
            let emitter =
                NavigationEmitter::remote(client.clone(), crate::PageId(77), job.id).unwrap();
            let checkpoint = emitter.checkpoint_pending().unwrap();
            waiting.recv_timeout(Duration::from_secs(5)).unwrap();
            let mut accepted = Vec::new();
            let queued_count = if retire_resources { 2 } else { 255 };
            for _ in 0..queued_count {
                accepted.push(emitter.publish_pending(NavigationEvent::Started).unwrap());
            }
            if retire_resources {
                assert!(client.retire_resource_jobs(Duration::ZERO).is_err());
            }
            assert!(matches!(
                emitter.publish_pending(NavigationEvent::Started),
                Err(NavigationStop::Failed(_))
            ));
            assert!(accepted.iter().all(|ack| matches!(
                ack.poll(),
                std::task::Poll::Ready(Err(NavigationStop::Failed(_)))
            )));
            release.send(()).unwrap();
            stop_seen.recv_timeout(Duration::from_secs(5)).unwrap();
            client
                .drain_resource_teardown(Duration::from_secs(5))
                .unwrap();
            client.drain_script_cleanup(Duration::from_secs(5)).unwrap();
            assert!(job.progress().unwrap().stopped.is_some());
            assert!(checkpoint.poll().is_ready());
            assert!(matches!(
                emitter.flush_pending().unwrap().poll(),
                std::task::Poll::Ready(Err(_))
            ));
            drop(emitter);
            shutdown.shutdown(std::net::Shutdown::Both).unwrap();
            let _ = server.join().unwrap();
        }
    }

    fn request(strategy: PageLoadStrategy) -> NavigationRequest {
        NavigationRequest {
            command: NavigationCommand::Navigate {
                url: "https://example.test/".into(),
            },
            strategy,
            timeout_ms: None,
        }
    }
    #[tokio::test]
    async fn strategy_waits_for_native_milestone_and_matching_document() {
        for (strategy, target) in [
            (PageLoadStrategy::None, NavigationPhase::Started),
            (PageLoadStrategy::Eager, NavigationPhase::Interactive),
            (PageLoadStrategy::Normal, NavigationPhase::Complete),
        ] {
            let control = NavigationControl::default();
            let job = control.begin(request(strategy)).unwrap();
            job.started().unwrap();
            if target > NavigationPhase::Started {
                job.committed(7, "https://example.test/final".into())
                    .unwrap();
                assert_eq!(job.interactive(8), Err(NavigationStop::InvalidTransition));
                job.interactive(7).unwrap();
            }
            if target == NavigationPhase::Complete {
                job.complete(7).unwrap();
            }
            assert_eq!(job.wait_ready().await.unwrap().phase, target);
        }
    }
    #[tokio::test]
    async fn timeout_is_shared_with_loader_and_does_not_renew_on_redirects() {
        let control = NavigationControl::default();
        let mut options = request(PageLoadStrategy::Normal);
        options.timeout_ms = Some(0);
        let job = control.begin(options).unwrap();
        assert_eq!(job.wait_ready().await, Err(NavigationStop::Timeout));
        assert_eq!(job.checkpoint(), Err(NavigationStop::Timeout));
        assert_eq!(
            job.committed(1, "https://example.test/late".into()),
            Err(NavigationStop::Timeout)
        );
    }
    #[tokio::test]
    async fn supersession_and_retirement_wake_waiters_without_actor_access() {
        let control = NavigationControl::default();
        let old = control.begin(request(PageLoadStrategy::Normal)).unwrap();
        old.started().unwrap();
        let new = control.begin(request(PageLoadStrategy::Normal)).unwrap();
        assert_ne!(old.id, new.id);
        assert_eq!(old.wait_ready().await, Err(NavigationStop::Superseded));
        assert_eq!(
            old.committed(1, "late".into()),
            Err(NavigationStop::Superseded)
        );
        control.retire().unwrap();
        assert_eq!(new.wait_ready().await, Err(NavigationStop::Retired));
        assert!(matches!(
            control.begin(request(PageLoadStrategy::None)),
            Err(NavigationStop::Retired)
        ));
    }
    #[tokio::test]
    async fn readiness_releases_page_load_deadline_without_aborting_remaining_resources() {
        let control = NavigationControl::default();
        let mut job = control.begin(request(PageLoadStrategy::Eager)).unwrap();
        job.started().unwrap();
        job.committed(1, "https://example.test/".into()).unwrap();
        job.interactive(1).unwrap();
        // Keep only this reference so the source test can install an expired
        // clock after the eager milestone without a wall-clock sleep.
        drop(control);
        Arc::get_mut(&mut job).unwrap().deadline = Some(Instant::now());
        assert!(job.checkpoint().is_ok());
        assert_eq!(
            job.wait_ready().await.unwrap().phase,
            NavigationPhase::Interactive
        );
        job.complete(1).unwrap();
        let control = NavigationControl::default();
        let mut options = request(PageLoadStrategy::None);
        options.timeout_ms = Some(0);
        let job = control.begin(options).unwrap();
        job.started().unwrap();
        assert_eq!(
            job.wait_ready().await.unwrap().phase,
            NavigationPhase::Started
        );
    }

    #[tokio::test]
    async fn no_op_history_completion_needs_no_fabricated_document() {
        let control = NavigationControl::default();
        let job = control.begin(request(PageLoadStrategy::Normal)).unwrap();
        job.started().unwrap();
        job.complete_without_commit().unwrap();
        let progress = job.wait_ready().await.unwrap();
        assert_eq!(progress.document_generation, None);
        assert_eq!(job.started(), Err(NavigationStop::InvalidTransition));
        assert_eq!(job.complete(1), Err(NavigationStop::InvalidTransition));
    }

    #[tokio::test]
    async fn replacement_document_invalidates_old_load_without_revoking_eager_readiness() {
        for strategy in [PageLoadStrategy::Normal, PageLoadStrategy::Eager] {
            let control = NavigationControl::default();
            let mut job = control.begin(request(strategy)).unwrap();
            job.started().unwrap();
            job.committed(1, "first".into()).unwrap();
            job.interactive(1).unwrap();
            job.committed(2, "replacement".into()).unwrap();
            assert_eq!(job.complete(1), Err(NavigationStop::InvalidTransition));
            assert_eq!(
                job.committed(2, "duplicate".into()),
                Err(NavigationStop::InvalidTransition)
            );
            if strategy == PageLoadStrategy::Eager {
                drop(control);
                Arc::get_mut(&mut job).unwrap().deadline = Some(Instant::now());
                assert!(job.checkpoint().is_ok());
                assert_eq!(job.wait_ready().await.unwrap().document_generation, Some(2));
            }
            job.interactive(2).unwrap();
            job.complete(2).unwrap();
            assert_eq!(job.wait_ready().await.unwrap().document_generation, Some(2));
        }
    }
}
