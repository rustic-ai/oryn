//! Browser-owned modal prompts. No V8 handles cross the controller boundary.
//! A process host can forward `PromptBackend::request` over its context-service
//! channel; the parent owns a controller whose resolution bypasses the JS worker.
use crate::realm::RealmId;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptKind {
    Alert,
    Confirm,
    Prompt,
    BeforeUnload,
}
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct PromptToken {
    pub id: u64,
    pub realm: RealmId,
    pub document_generation: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PromptRequest {
    pub token: PromptToken,
    pub kind: PromptKind,
    pub text: String,
    pub default_value: String,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingPrompt {
    pub request: PromptRequest,
    pub input: String,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PromptResponse {
    Accepted(String),
    Dismissed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PromptError {
    NoPrompt,
    StalePrompt,
    AlreadyOpen,
    WrongKind,
    Retired,
    Cancelled,
    Poisoned,
    IdExhausted,
}

/// Implementations must support cancellation without waiting on the JS actor.
/// `request` is synchronous and must preserve the same native JS call stack.
pub trait PromptBackend: Send + Sync {
    fn request(&self, request: PromptRequest) -> Result<PromptResponse, PromptError>;
    fn cancel(&self, token: PromptToken);
}
/// Host watchdog accounting hook. Modal wall time must not consume a script's
/// active execution budget. Explicit cancellation remains independent of this hook.
pub trait PromptWaitObserver: Send + Sync {
    fn begin_wait(&self);
    fn end_wait(&self);
}
struct WaitGuard(Option<Arc<dyn PromptWaitObserver>>);
impl WaitGuard {
    fn new(observer: Option<Arc<dyn PromptWaitObserver>>) -> Self {
        if let Some(observer) = &observer {
            observer.begin_wait();
        }
        Self(observer)
    }
}
impl Drop for WaitGuard {
    fn drop(&mut self) {
        if let Some(observer) = &self.0 {
            observer.end_wait();
        }
    }
}

impl std::fmt::Display for PromptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PromptError {}

#[derive(Default)]
struct ControllerState {
    replacing_document: bool,
    next_id: u64,
    pending: Option<PendingPrompt>,
    response: Option<PromptResponse>,
    cancelled: bool,
    retired: bool,
    backend: Option<Arc<dyn PromptBackend>>,
    wait_observer: Option<Arc<dyn PromptWaitObserver>>,
    cancelled_tokens: std::collections::BTreeSet<PromptToken>,
}
#[derive(Default)]
pub struct PromptController {
    state: Mutex<ControllerState>,
    changed: Condvar,
}
/// Reserves a no-pending-prompt replacement before execution services retire.
/// No mutex is held across native cancellation callbacks.
pub(crate) struct DocumentReplacement<'a> {
    controller: &'a PromptController,
    committed: bool,
}
impl DocumentReplacement<'_> {
    pub(crate) fn commit(mut self) {
        // Admission already established mutex ownership and excluded new waits.
        // Recovering a poisoned lock here only closes the reserved transition;
        // it never admits a prompt or discards an existing wait.
        let mut state=self.controller.state.lock().unwrap_or_else(|error|error.into_inner());
        state.retired=false;state.cancelled=false;state.response=None;
        state.replacing_document=false;self.committed=true;
    }
}
impl Drop for DocumentReplacement<'_> {
    fn drop(&mut self){
        if !self.committed{
            let mut state=self.controller.state.lock().unwrap_or_else(|error|error.into_inner());
            state.replacing_document=false;
        }
    }
}

impl std::fmt::Debug for PromptController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.state.lock() {
            Ok(state) => f
                .debug_struct("PromptController")
                .field("pending", &state.pending)
                .field("retired", &state.retired)
                .finish(),
            Err(_) => f.write_str("PromptController { poisoned: true }"),
        }
    }
}

impl PromptController {
    pub(crate) fn prepare_document_replacement(&self)->Result<DocumentReplacement<'_>,PromptError>{
        let mut state=self.state.lock().map_err(|_|PromptError::Poisoned)?;
        if state.pending.is_some()||state.replacing_document{return Err(PromptError::AlreadyOpen);}
        state.replacing_document=true;
        Ok(DocumentReplacement{controller:self,committed:false})
    }
    pub fn new() -> Self {
        Self::default()
    }
    /// Reuse an initial Window for its committed document only after every old
    /// wait unwound. Preserve backend/watchdog wiring and monotonically increasing IDs.
    pub fn renew_document(&self) -> Result<(), PromptError> {
        let mut state = self.state.lock().map_err(|_| PromptError::Poisoned)?;
        if state.pending.is_some()||state.replacing_document {
            return Err(PromptError::AlreadyOpen);
        }
        state.retired = false;
        state.cancelled = false;
        state.response = None;
        Ok(())
    }
    pub fn set_backend(&self, backend: Arc<dyn PromptBackend>) -> Result<(), PromptError> {
        let mut state = self.state.lock().map_err(|_| PromptError::Poisoned)?;
        if state.retired {
            return Err(PromptError::Retired);
        }
        if state.pending.is_some() {
            return Err(PromptError::AlreadyOpen);
        }
        state.backend = Some(backend);
        Ok(())
    }
    pub fn set_wait_observer(
        &self,
        observer: Arc<dyn PromptWaitObserver>,
    ) -> Result<(), PromptError> {
        let mut state = self.state.lock().map_err(|_| PromptError::Poisoned)?;
        if state.retired {
            return Err(PromptError::Retired);
        }
        if state.pending.is_some() {
            return Err(PromptError::AlreadyOpen);
        }
        state.wait_observer = Some(observer);
        Ok(())
    }
    /// Linearize a short native-only publication with exact controller retirement.
    /// The callback must not invoke author code, prompt methods or wait for I/O.
    /// Long draft computation belongs before this admission, not under this lock.
    pub fn with_live_publication<T>(&self, publish: impl FnOnce() -> T) -> Result<T, PromptError> {
        let state = self.state.lock().map_err(|_| PromptError::Poisoned)?;
        if state.retired {
            return Err(PromptError::Retired);
        }
        let result = publish();
        drop(state);
        Ok(result)
    }
    pub fn inspect(&self) -> Result<Option<PendingPrompt>, PromptError> {
        let state = self.state.lock().map_err(|_| PromptError::Poisoned)?;
        if state.retired {
            return Err(PromptError::Retired);
        }
        Ok(if state.response.is_some() || state.cancelled {
            None
        } else {
            state.pending.clone()
        })
    }
    /// The watchdog must suspend script-time accounting while this is true;
    /// page cancellation/worker shutdown must still retire the controller.
    pub fn is_waiting(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.pending.is_some() && state.response.is_none() && !state.cancelled)
            .unwrap_or(false)
    }
    fn validate(
        state: &mut ControllerState,
        token: PromptToken,
    ) -> Result<&mut PendingPrompt, PromptError> {
        if state.retired {
            return Err(PromptError::Retired);
        }
        if state.response.is_some() || state.cancelled {
            return Err(PromptError::NoPrompt);
        }
        let pending = state.pending.as_mut().ok_or(PromptError::NoPrompt)?;
        if pending.request.token != token {
            return Err(PromptError::StalePrompt);
        }
        Ok(pending)
    }
    pub fn set_input(&self, token: PromptToken, input: String) -> Result<(), PromptError> {
        let mut state = self.state.lock().map_err(|_| PromptError::Poisoned)?;
        let pending = Self::validate(&mut state, token)?;
        if pending.request.kind != PromptKind::Prompt {
            return Err(PromptError::WrongKind);
        }
        pending.input = input;
        Ok(())
    }
    pub fn accept(&self, token: PromptToken) -> Result<(), PromptError> {
        self.resolve(token, true)
    }
    pub fn dismiss(&self, token: PromptToken) -> Result<(), PromptError> {
        self.resolve(token, false)
    }
    fn resolve(&self, token: PromptToken, accept: bool) -> Result<(), PromptError> {
        let mut state = self.state.lock().map_err(|_| PromptError::Poisoned)?;
        let pending = Self::validate(&mut state, token)?;
        let response = if accept {
            PromptResponse::Accepted(pending.input.clone())
        } else {
            PromptResponse::Dismissed
        };
        state.response = Some(response);
        self.changed.notify_all();
        Ok(())
    }
    pub fn cancel(&self, token: PromptToken) -> Result<(), PromptError> {
        let backend = {
            let mut state = self.state.lock().map_err(|_| PromptError::Poisoned)?;
            if state.retired || state.replacing_document {
                return Err(PromptError::Retired);
            }
            if state
                .pending
                .as_ref()
                .is_some_and(|pending| pending.request.token == token)
            {
                state.cancelled = true;
                self.changed.notify_all();
                state.backend.clone()
            } else {
                // Native IPC can deliver cancellation before the synchronous
                // request reaches its service worker. Remember this exact token;
                // it must not cancel a different generation's prompt.
                state.cancelled_tokens.insert(token);
                None
            }
        };
        if let Some(backend) = backend {
            backend.cancel(token);
        }
        Ok(())
    }
    pub fn retire(&self) {
        let cancel = if let Ok(mut state) = self.state.lock() {
            state.retired = true;
            let cancel = state.pending.as_ref().and_then(|p| {
                state
                    .backend
                    .clone()
                    .map(|backend| (backend, p.request.token))
            });
            self.changed.notify_all();
            cancel
        } else {
            None
        };
        if let Some((backend, token)) = cancel {
            backend.cancel(token);
        }
    }
    pub fn show(
        &self,
        realm: RealmId,
        document_generation: u64,
        kind: PromptKind,
        text: String,
        default_value: String,
    ) -> Result<PromptResponse, PromptError> {
        let token = {
            let mut state = self.state.lock().map_err(|_| PromptError::Poisoned)?;
            if state.retired||state.replacing_document {
                return Err(PromptError::Retired);
            }
            state.next_id = state
                .next_id
                .checked_add(1)
                .ok_or(PromptError::IdExhausted)?;
            PromptToken {
                id: state.next_id,
                realm,
                document_generation,
            }
        };
        self.wait_for_request(PromptRequest {
            token,
            kind,
            text,
            default_value,
        })
    }
    /// Parent-process entry point for a request received over trusted native IPC.
    /// Call on a context-service worker; inspect/resolve must use another thread.
    pub fn wait_for_request(&self, request: PromptRequest) -> Result<PromptResponse, PromptError> {
        let observer = self
            .state
            .lock()
            .map_err(|_| PromptError::Poisoned)?
            .wait_observer
            .clone();
        let _wait_guard = WaitGuard::new(observer);
        let mut state = self.state.lock().map_err(|_| PromptError::Poisoned)?;
        if state.retired || state.replacing_document {
            return Err(PromptError::Retired);
        }
        if state.cancelled_tokens.remove(&request.token) {
            return Err(PromptError::Cancelled);
        }
        if state.pending.is_some() {
            return Err(PromptError::AlreadyOpen);
        }
        state.pending = Some(PendingPrompt {
            input: request.default_value.clone(),
            request: request.clone(),
        });
        state.response = None;
        state.cancelled = false;
        self.changed.notify_all();
        if let Some(backend) = state.backend.clone() {
            drop(state);
            let response = backend.request(request);
            let mut state = self.state.lock().map_err(|_| PromptError::Poisoned)?;
            state.pending = None;
            state.response = None;
            if state.retired {
                return Err(PromptError::Retired);
            }
            if state.cancelled {
                return Err(PromptError::Cancelled);
            }
            return response;
        }
        loop {
            if state.retired {
                state.pending = None;
                state.response = None;
                return Err(PromptError::Retired);
            }
            if state.cancelled {
                state.pending = None;
                return Err(PromptError::Cancelled);
            }
            if let Some(response) = state.response.take() {
                state.pending = None;
                return Ok(response);
            }
            state = self
                .changed
                .wait(state)
                .map_err(|_| PromptError::Poisoned)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_replacement_excludes_waits_and_commits_after_retirement(){
        let controller=PromptController::new();
        let replacement=controller.prepare_document_replacement().unwrap();
        assert!(matches!(controller.prepare_document_replacement(),Err(PromptError::AlreadyOpen)));
        assert!(matches!(controller.renew_document(),Err(PromptError::AlreadyOpen)));
        controller.retire();replacement.commit();
        let state=controller.state.lock().unwrap();
        assert!(!state.retired);assert!(!state.replacing_document);assert!(state.pending.is_none());
    }
    #[test]
    fn abandoned_document_replacement_releases_exclusion_without_reviving_owner(){
        let controller=PromptController::new();controller.retire();
        drop(controller.prepare_document_replacement().unwrap());
        let state=controller.state.lock().unwrap();
        assert!(state.retired);assert!(!state.replacing_document);
    }
    #[test]
    fn live_publication_linearizes_retirement_and_releases_on_failure() {
        let controller = Arc::new(PromptController::new());
        let published = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (retire_tx, retire_rx) = std::sync::mpsc::channel();
        let publisher = controller.clone();
        let output = published.clone();
        let publication = std::thread::spawn(move || {
            publisher.with_live_publication(|| {
                entered_tx.send(()).unwrap();
                retire_rx.recv().unwrap();
                output.store(true, std::sync::atomic::Ordering::Release);
                Err::<(), &'static str>("native commit failure")
            })
        });
        entered_rx.recv().unwrap();
        let retiring = controller.clone();
        let output = published.clone();
        let retirement = std::thread::spawn(move || {
            retire_tx.send(()).unwrap();
            retiring.retire();
            assert!(output.load(std::sync::atomic::Ordering::Acquire));
        });
        assert_eq!(
            publication.join().unwrap().unwrap(),
            Err("native commit failure")
        );
        retirement.join().unwrap();
        assert_eq!(
            controller.with_live_publication(|| panic!("retired publication")),
            Err(PromptError::Retired)
        );
    }

    fn pending(controller: &PromptController) -> PendingPrompt {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut state = controller.state.lock().unwrap();
        loop {
            if let Some(prompt) = &state.pending {
                return prompt.clone();
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "prompt was not published");
            state = controller.changed.wait_timeout(state, left).unwrap().0;
        }
    }
    #[test]
    fn pending_prompt_identity_input_and_response_are_native_and_single_use() {
        let controller = Arc::new(PromptController::new());
        let worker = controller.clone();
        let task = std::thread::spawn(move || {
            worker.show(
                RealmId(4),
                8,
                PromptKind::Prompt,
                "Question".into(),
                "Initial".into(),
            )
        });
        let prompt = pending(&controller);
        assert_eq!(prompt.input, "Initial");
        assert!(controller.is_waiting());
        let mut stale = prompt.request.token;
        stale.document_generation += 1;
        assert_eq!(controller.accept(stale), Err(PromptError::StalePrompt));
        assert_eq!(
            controller.show(
                RealmId(4),
                8,
                PromptKind::Alert,
                "nested".into(),
                String::new()
            ),
            Err(PromptError::AlreadyOpen)
        );
        controller
            .set_input(prompt.request.token, "Unicode 👩‍💻".into())
            .unwrap();
        controller.accept(prompt.request.token).unwrap();
        assert_eq!(
            controller.dismiss(prompt.request.token),
            Err(PromptError::NoPrompt)
        );
        assert_eq!(
            task.join().unwrap(),
            Ok(PromptResponse::Accepted("Unicode 👩‍💻".into()))
        );
        assert!(controller.inspect().unwrap().is_none());
        assert!(!controller.is_waiting());
    }
    #[test]
    fn cancel_and_retire_wake_waiters_without_fabricating_user_acceptance() {
        for retire in [false, true] {
            let controller = Arc::new(PromptController::new());
            let worker = controller.clone();
            let task = std::thread::spawn(move || {
                worker.show(
                    RealmId(1),
                    2,
                    PromptKind::Confirm,
                    "Continue?".into(),
                    String::new(),
                )
            });
            let prompt = pending(&controller);
            assert_eq!(
                controller.set_input(prompt.request.token, "bad".into()),
                Err(PromptError::WrongKind)
            );
            if retire {
                controller.retire();
            } else {
                controller.cancel(prompt.request.token).unwrap();
            }
            assert_eq!(
                task.join().unwrap(),
                Err(if retire {
                    PromptError::Retired
                } else {
                    PromptError::Cancelled
                })
            );
            if retire {
                assert_eq!(controller.inspect(), Err(PromptError::Retired));
            } else {
                assert!(controller.inspect().unwrap().is_none());
            }
        }
    }
    #[test]
    fn cancellation_before_ipc_publication_is_generation_scoped() {
        let controller = PromptController::new();
        let token = PromptToken {
            id: 42,
            realm: RealmId(7),
            document_generation: 3,
        };
        controller.cancel(token).unwrap();
        assert_eq!(
            controller.wait_for_request(PromptRequest {
                token,
                kind: PromptKind::Alert,
                text: String::new(),
                default_value: String::new()
            }),
            Err(PromptError::Cancelled)
        );
        assert!(controller.inspect().unwrap().is_none());
    }
    struct Accounting {
        entered: std::sync::atomic::AtomicUsize,
        exited: std::sync::atomic::AtomicUsize,
    }
    impl PromptWaitObserver for Accounting {
        fn begin_wait(&self) {
            self.entered
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        fn end_wait(&self) {
            self.exited
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    #[test]
    fn modal_wait_accounting_balances_resolution_cancel_and_retirement() {
        for action in 0..3 {
            let controller = Arc::new(PromptController::new());
            let accounting = Arc::new(Accounting {
                entered: 0.into(),
                exited: 0.into(),
            });
            controller.set_wait_observer(accounting.clone()).unwrap();
            let worker = controller.clone();
            let task = std::thread::spawn(move || {
                worker.show(
                    RealmId(1),
                    1,
                    PromptKind::Confirm,
                    String::new(),
                    String::new(),
                )
            });
            let prompt = pending(&controller);
            assert_eq!(
                accounting.entered.load(std::sync::atomic::Ordering::SeqCst),
                1
            );
            assert_eq!(
                accounting.exited.load(std::sync::atomic::Ordering::SeqCst),
                0
            );
            match action {
                0 => controller.accept(prompt.request.token).unwrap(),
                1 => controller.cancel(prompt.request.token).unwrap(),
                _ => controller.retire(),
            }
            let _ = task.join().unwrap();
            assert_eq!(
                accounting.exited.load(std::sync::atomic::Ordering::SeqCst),
                1
            );
        }
    }
    struct Forwarder(Arc<PromptController>);
    impl PromptBackend for Forwarder {
        fn request(&self, request: PromptRequest) -> Result<PromptResponse, PromptError> {
            self.0.wait_for_request(request)
        }
        fn cancel(&self, token: PromptToken) {
            let _ = self.0.cancel(token);
        }
    }
    #[test]
    fn forwarded_prompt_can_be_resolved_or_retired_without_worker_mailbox() {
        for cancel in [false, true] {
            let parent = Arc::new(PromptController::new());
            let child = Arc::new(PromptController::new());
            child
                .set_backend(Arc::new(Forwarder(parent.clone())))
                .unwrap();
            let worker = child.clone();
            let task = std::thread::spawn(move || {
                worker.show(
                    RealmId(3),
                    9,
                    PromptKind::Alert,
                    "Message".into(),
                    String::new(),
                )
            });
            let prompt = pending(&parent);
            assert!(child.is_waiting());
            if cancel {
                child.retire();
            } else {
                parent.dismiss(prompt.request.token).unwrap();
            }
            assert_eq!(
                task.join().unwrap(),
                if cancel {
                    Err(PromptError::Retired)
                } else {
                    Ok(PromptResponse::Dismissed)
                }
            );
        }
    }
}
