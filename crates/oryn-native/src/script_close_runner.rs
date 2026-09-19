//! Dedicated bounded executor for admitted close operations. Registry phases
//! survive a failed task; actor requests always reuse the original token.
use super::script_close_admission::{Admission, Phase};
use super::*;
use std::sync::Weak;
#[derive(Default)]
pub(super) struct Runner {
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl Runner {
    fn start(&self, context: Weak<BrowserContextInner>) -> Result<(), RuntimeError> {
        let mut slot = self
            .thread
            .lock()
            .map_err(|_| RuntimeError::ContextService("close runner lock poisoned".into()))?;
        if slot.as_ref().is_some_and(|thread| !thread.is_finished()) {
            return Ok(());
        }
        if let Some(previous) = slot.take() {
            let _ = previous.join();
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(4)
            .build()
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        *slot = Some(
            std::thread::Builder::new()
                .name("oryn-script-close-admission".into())
                .spawn(move || runtime.block_on(supervise(context)))
                .map_err(|error| RuntimeError::ContextService(error.to_string()))?,
        );
        Ok(())
    }
}
impl BrowserContext {
    pub(crate) fn ensure_script_close_runner(&self) -> Result<(), RuntimeError> {
        self.inner
            .script_close_runner
            .start(Arc::downgrade(&self.inner))
    }
    pub(crate) fn start_admitted_script_close(
        &self,
        admission: Admission,
    ) -> Result<(), RuntimeError> {
        let valid = self
            .inner
            .script_close_admissions
            .lock()
            .map_err(|_| RuntimeError::ContextService("close admission registry poisoned".into()))?
            .get(admission.target, admission.token)
            .is_some();
        if !valid {
            return Err(RuntimeError::ProtocolViolation(
                "close runner admission missing".into(),
            ));
        }
        self.inner
            .script_close_runner
            .start(Arc::downgrade(&self.inner))
    }
}
async fn supervise(context: Weak<BrowserContextInner>) {
    let mut tasks = tokio::task::JoinSet::new();
    let mut running = BTreeMap::new();
    loop {
        let Some(inner) = context.upgrade() else {
            break;
        };
        let snapshot = inner
            .script_close_admissions
            .lock()
            .map(|registry| registry.active_snapshot())
            .unwrap_or_default();
        let shutdown = inner.workers_retired.load(Ordering::Acquire);
        drop(inner);
        for admission in snapshot {
            let key = (admission.target, admission.token);
            if running.values().any(|existing| *existing == key) {
                continue;
            }
            let weak = context.clone();
            let handle = tasks.spawn(async move {
                run(weak, admission).await;
            });
            running.insert(handle.id(), key);
        }
        if shutdown && tasks.is_empty() {
            break;
        }
        tokio::select! {
            result=tasks.join_next_with_id(),if !tasks.is_empty()=>{match result{Some(Ok((id,())))=>{running.remove(&id);},Some(Err(error))=>{running.remove(&error.id());},None=>{}}},
            _=tokio::time::sleep(Duration::from_millis(25))=>{},
        }
    }
}
fn finish(context: &BrowserContext, admission: &Admission, phase: Phase) {
    if let Ok(mut registry) = context.inner.script_close_admissions.lock() {
        let _ = registry.finish(admission.target, admission.token, phase);
    }
}
fn current(context: &BrowserContext, admission: &Admission) -> Option<Phase> {
    context
        .inner
        .script_close_admissions
        .lock()
        .ok()?
        .get(admission.target, admission.token)
        .map(|entry| entry.phase.clone())
}
fn target(
    context: &BrowserContext,
    page: PageId,
) -> Option<(
    PageHandle,
    bool,
    bool,
    Option<Arc<crate::script_close_custody::Job>>,
)> {
    let pages = context.inner.group.pages.lock().ok()?;
    let record = pages.get(&page)?;
    Some((
        record.handle.retained_clone(),
        record.is_live(),
        record.physical_close_complete,
        record.script_close.clone(),
    ))
}
async fn run(weak: Weak<BrowserContextInner>, admission: Admission) {
    loop {
        let Some(inner) = weak.upgrade() else { return };
        let context = BrowserContext { inner };
        let Some(phase) = current(&context, &admission) else {
            return;
        };
        if context.inner.workers_retired.load(Ordering::Acquire) {
            finish(&context, &admission, Phase::Canceled);
            return;
        }
        let Some((page, live, physical, cleanup)) = target(&context, admission.target) else {
            finish(&context, &admission, Phase::Canceled);
            return;
        };
        if phase == Phase::Closing {
            if physical {
                finish(&context, &admission, Phase::Closed);
                return;
            }
            // Once departure was accepted, source retirement cannot revoke
            // physical cleanup. A lost spawn ACK retries the retained job.
            if let Some(cleanup) = cleanup {
                let _ = cleanup.start();
            } else if live {
                let _ = context.queue_script_close(admission.target);
            } else {
                finish(&context, &admission, Phase::Canceled);
                return;
            }
        } else if !live {
            finish(&context, &admission, Phase::Canceled);
            return;
        } else {
            match phase {
                Phase::Querying => match page.script_close_history().await {
                    Ok(proof) => {
                        if !context
                            .script_close_eligible(admission.target, Some(proof.single_entry))
                        {
                            finish(&context, &admission, Phase::Canceled);
                            return;
                        }
                        match context.root_navigation_creator(admission.target) {
                            Ok(current) if current != proof.creator => {
                                finish(&context, &admission, Phase::Canceled);
                                return;
                            }
                            Err(_) => {
                                drop(context);
                                tokio::time::sleep(Duration::from_millis(25)).await;
                                continue;
                            }
                            _ => {}
                        }
                        if let Ok(mut registry) = context.inner.script_close_admissions.lock() {
                            if registry
                                .preparing(admission.target, admission.token, proof.creator)
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    Err(_) => {} // Retain query custody; next turn checks retirement.
                },
                Phase::Preparing(expected) => match page
                    .prepare_script_close(admission.token, expected.clone())
                    .await
                {
                    Ok(crate::context_service::ScriptClosePreparation::Allowed(proof)) => {
                        // Actor selected the active Document when the admitted task ran.
                        // The token binds this reply to the traversable even after navigation.
                        let _departed_creator = proof.creator;
                        if let Ok(mut registry) = context.inner.script_close_admissions.lock() {
                            if registry
                                .closing(admission.target, admission.token, &expected)
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    Ok(crate::context_service::ScriptClosePreparation::Canceled) => {
                        finish(&context, &admission, Phase::Canceled);
                        return;
                    }
                    Ok(crate::context_service::ScriptClosePreparation::Pending) | Err(_) => {}
                },
                _ => return,
            }
        }
        drop(context);
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[cfg(all(test, feature = "in-process-probe"))]
mod tests {
    use super::*;
    async fn fixture(
        canceled: bool,
    ) -> (
        BrowserContext,
        PageHandle,
        Admission,
        tokio::task::JoinHandle<()>,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let context = Browser::in_process_probe().new_context();
        let source = context.new_page();
        let target = context.new_page();
        let creator = context
            .native_initial_document_creator(source.page_id())
            .unwrap();
        let proof = crate::context_service::ScriptCloseHistoryProof {
            creator: context
                .native_initial_document_creator(target.page_id())
                .unwrap(),
            single_entry: true,
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let mut fake = target.detached_clone();
        fake.sender = Some(tx);
        fake.worker = None;
        context
            .inner
            .group
            .pages
            .lock()
            .unwrap()
            .get_mut(&target.page_id())
            .unwrap()
            .handle = fake;
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = count.clone();
        let mut query_attempts = 0;
        let mut preparation_attempts = 0;
        let mut expected_token = None;
        let actor = tokio::spawn(async move {
            while let Some(command) = rx.recv().await {
                match command {
                    PageCommand::ScriptCloseHistory { reply } => {
                        query_attempts += 1;
                        let _ = reply.send(if query_attempts == 1 {
                            Err(RuntimeError::ContextService("lost query ACK".into()))
                        } else {
                            Ok(proof.clone())
                        });
                    }
                    PageCommand::PrepareScriptClose {
                        token,
                        expected,
                        reply,
                    } => {
                        assert_eq!(expected, proof.creator);
                        assert_eq!(*expected_token.get_or_insert(token), token);
                        preparation_attempts += 1;
                        observed.fetch_add(1, Ordering::SeqCst);
                        let result = if canceled {
                            Ok(crate::context_service::ScriptClosePreparation::Canceled)
                        } else {
                            match preparation_attempts {
                                1 => Ok(crate::context_service::ScriptClosePreparation::Pending),
                                2 => Err(RuntimeError::ContextService("lost departure ACK".into())),
                                _ => Ok(crate::context_service::ScriptClosePreparation::Allowed(
                                    proof.clone(),
                                )),
                            }
                        };
                        let _ = reply.send(result);
                    }
                    PageCommand::Close { reply } => {
                        let _ = reply.send(());
                        break;
                    }
                    _ => panic!("unexpected close runner request"),
                }
            }
        });
        let (admission, fresh) = context
            .inner
            .script_close_admissions
            .lock()
            .unwrap()
            .admit(source.page_id(), creator, target.page_id())
            .unwrap();
        assert!(fresh);
        context.inner.group.close(source.page_id());
        (context, target, admission, actor, count)
    }
    #[tokio::test(flavor = "current_thread")]
    async fn runner_retries_same_token_and_closes_after_source_retirement() {
        let (context, target, admission, actor, count) = fixture(false).await;
        context
            .start_admitted_script_close(admission.clone())
            .unwrap();
        context
            .start_admitted_script_close(admission.clone())
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if current(&context, &admission).is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await
            }
        })
        .await
        .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert!(
            context.inner.group.pages.lock().unwrap()[&target.page_id()].physical_close_complete
        );
        actor.await.unwrap();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn canceled_departure_releases_runner_registry_without_physical_close() {
        let (context, target, admission, actor, count) = fixture(true).await;
        context
            .start_admitted_script_close(admission.clone())
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if current(&context, &admission).is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await
            }
        })
        .await
        .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(
            !context.inner.group.pages.lock().unwrap()[&target.page_id()].physical_close_complete
        );
        assert!(context.inner.group.pages.lock().unwrap()[&target.page_id()].is_live());
        actor.abort();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn context_shutdown_cancels_admission_before_actor_queries() {
        let (context, _target, admission, actor, count) = fixture(false).await;
        context.inner.workers_retired.store(true, Ordering::Release);
        context
            .start_admitted_script_close(admission.clone())
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if current(&context, &admission).is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await
            }
        })
        .await
        .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 0);
        actor.abort();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn public_close_sets_sticky_state_before_departure_and_canceled_unload_is_not_repeated() {
        use super::super::script_close_history_projection::Change;
        use oryn_js::{
            message_wire::{MessageRequest, WindowTarget},
            worker_wire::Creator,
        };
        let context = Browser::in_process_probe().new_context();
        let target = context.new_page();
        let id = target.page_id();
        context
            .install_document_security(id, "https://closing.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        context
            .change_script_close_history(id, &creator, Change::Begin, 1, 1)
            .unwrap();
        context
            .change_script_close_history(id, &creator, Change::Publish, 1, 1)
            .unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let mut fake = target.detached_clone();
        fake.sender = Some(tx);
        fake.worker = None;
        context
            .inner
            .group
            .pages
            .lock()
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .handle = fake;
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = count.clone();
        let expected = creator.clone();
        let actor = tokio::spawn(async move {
            while let Some(command) = rx.recv().await {
                match command {
                    PageCommand::PrepareScriptClose {
                        expected: owner,
                        reply,
                        ..
                    } => {
                        assert_eq!(owner, expected);
                        observed.fetch_add(1, Ordering::SeqCst);
                        let _ = reply
                            .send(Ok(crate::context_service::ScriptClosePreparation::Canceled));
                    }
                    _ => panic!(
                        "public accepted close must not query history again or physically close canceled unloading"
                    ),
                }
            }
        });
        assert!(!context.script_close_state(id, &creator, id, None).unwrap());
        assert!(
            context
                .request_script_close(id, &creator, id, None)
                .unwrap()
        );
        assert!(
            context.script_close_state(id, &creator, id, None).unwrap(),
            "closed becomes true synchronously"
        );
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "target departure runs in a later task"
        );
        // A later history change cannot revoke already admitted definitely-close.
        context
            .change_script_close_history(id, &creator, Change::Begin, 2, 2)
            .unwrap();
        context
            .change_script_close_history(id, &creator, Change::Publish, 2, 2)
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if context
                    .inner
                    .script_close_admissions
                    .lock()
                    .unwrap()
                    .active_snapshot()
                    .is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await
            }
        })
        .await
        .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(context.script_close_state(id, &creator, id, None).unwrap());
        assert!(context.inner.group.pages.lock().unwrap()[&id].is_live());
        assert!(!context.inner.group.pages.lock().unwrap()[&id].physical_close_complete);
        assert!(
            context
                .request_script_close(id, &creator, id, None)
                .unwrap()
        );
        assert!(
            context
                .inner
                .script_close_admissions
                .lock()
                .unwrap()
                .active_snapshot()
                .is_empty()
        );
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        actor.abort();
    }
}

// Only raw V8 Window tests need to hold a real admitted close's departure.
// This fixture changes actor transport, never native creator/group/closing state.
#[cfg(all(test, feature = "v8-host"))]
pub(crate) struct HeldScriptCloseDeparture {
    context: BrowserContext,
    page: PageId,
    previous: Option<PageHandle>,
    receiver: tokio::sync::mpsc::Receiver<PageCommand>,
    reply: Option<
        tokio::sync::oneshot::Sender<
            Result<crate::context_service::ScriptClosePreparation, RuntimeError>,
        >,
    >,
    token: Option<u64>,
}
#[cfg(all(test, feature = "v8-host"))]
impl BrowserContext {
    pub(crate) fn hold_script_close_departure_for_test(
        &self,
        page: PageId,
    ) -> HeldScriptCloseDeparture {
        assert!(
            !self
                .inner
                .script_close_admissions
                .lock()
                .unwrap()
                .active_snapshot()
                .iter()
                .any(|entry| entry.target == page)
        );
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let previous = {
            let mut pages = self.inner.group.pages.lock().unwrap();
            let record = pages.get_mut(&page).unwrap();
            let mut controlled = record.handle.detached_clone();
            controlled.sender = Some(sender);
            controlled.worker = None;
            std::mem::replace(&mut record.handle, controlled)
        };
        HeldScriptCloseDeparture {
            context: self.clone(),
            page,
            previous: Some(previous),
            receiver,
            reply: None,
            token: None,
        }
    }
}
#[cfg(all(test, feature = "v8-host"))]
impl HeldScriptCloseDeparture {
    pub(crate) async fn receive(&mut self) -> (u64, oryn_js::worker_wire::Creator) {
        let command = tokio::time::timeout(Duration::from_secs(5), self.receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let PageCommand::PrepareScriptClose {
            token,
            expected,
            reply,
        } = command
        else {
            panic!("expected native close preparation")
        };
        let registry = self.context.inner.script_close_admissions.lock().unwrap();
        let admission = registry.get(self.page, token).unwrap();
        assert_eq!(admission.phase, Phase::Preparing(expected.clone()));
        self.token = Some(token);
        self.reply = Some(reply);
        (token, expected)
    }
    pub(crate) async fn cancel(&mut self) {
        let token = self.token.unwrap();
        self.reply
            .take()
            .unwrap()
            .send(Ok(crate::context_service::ScriptClosePreparation::Canceled))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if self
                    .context
                    .inner
                    .script_close_admissions
                    .lock()
                    .unwrap()
                    .get(self.page, token)
                    .is_none()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            self.receiver.try_recv().is_err(),
            "canceled departure must not request physical close"
        );
    }
}
#[cfg(all(test, feature = "v8-host"))]
impl Drop for HeldScriptCloseDeparture {
    fn drop(&mut self) {
        // No asynchronous waits, joins, retries or panicking unwraps during unwinding. The
        // fixture started with no admission, so an outstanding target receipt
        // belongs to this held actor even if receive() was never reached.
        let mut registry = self
            .context
            .inner
            .script_close_admissions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let token = self.token.or_else(|| {
            registry
                .active_snapshot()
                .into_iter()
                .find(|entry| entry.target == self.page)
                .map(|entry| entry.token)
        });
        if let Some(token) = token {
            let _ = registry.finish(self.page, token, Phase::Canceled);
        }
        drop(registry);
        self.reply.take();
        if let Some(previous) = self.previous.take() {
            let mut pages = self
                .context
                .inner
                .group
                .pages
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(record) = pages.get_mut(&self.page) {
                record.handle = previous;
            }
        }
    }
}
