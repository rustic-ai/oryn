//! Exact-document history projection. A reserved change is never exposed as
//! the previous known count, and lost acknowledgements are safe to replay.
use super::*;
use oryn_js::worker_wire::Creator;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Begin,
    Publish,
    Abort,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disposition {
    Pending,
    Published,
    Aborted,
}
#[derive(Clone, Debug)]
pub(super) struct Projection {
    creator: Creator,
    revision: u64,
    proposed: usize,
    known: Option<usize>,
    disposition: Disposition,
}
impl Projection {
    fn new(creator: Creator) -> Self {
        Self {
            creator,
            revision: 0,
            proposed: 0,
            known: None,
            disposition: Disposition::Aborted,
        }
    }
    fn apply(&mut self, change: Change, revision: u64, count: usize) -> Result<(), RuntimeError> {
        let invalid = || {
            RuntimeError::ProtocolViolation("invalid or stale history projection transition".into())
        };
        if revision == 0 || count == 0 || count > 4096 || revision < self.revision {
            return Err(invalid());
        }
        if revision == self.revision {
            if count != self.proposed {
                return Err(invalid());
            }
            return match (change, self.disposition) {
                (Change::Begin, Disposition::Pending | Disposition::Published) => Ok(()),
                (Change::Publish, Disposition::Pending) => {
                    self.known = Some(count);
                    self.disposition = Disposition::Published;
                    Ok(())
                }
                (Change::Publish, Disposition::Published) => Ok(()),
                (Change::Abort, Disposition::Pending) => {
                    self.disposition = Disposition::Aborted;
                    Ok(())
                }
                // A late abort cannot roll back a locally committed revision.
                (Change::Abort, Disposition::Published | Disposition::Aborted) => Ok(()),
                _ => Err(invalid()),
            };
        }
        // Only one local mutation may be in flight. Abort also handles a
        // begin request that never arrived, retaining a replay tombstone.
        if self.disposition == Disposition::Pending || change == Change::Publish {
            return Err(invalid());
        }
        self.revision = revision;
        self.proposed = count;
        self.disposition = if change == Change::Begin {
            Disposition::Pending
        } else {
            Disposition::Aborted
        };
        Ok(())
    }
    pub(super) fn count(&self, creator: &Creator) -> Option<usize> {
        if &self.creator != creator || self.disposition == Disposition::Pending {
            None
        } else {
            self.known
        }
    }
}
// At most eight blocked close readers across this parent process. The IPC
// client permits32 outstanding operations and parent dispatch64, so close
// waits alone cannot occupy every slot needed by Begin/Publish/Abort.
static WAITERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct WaitPermit;
impl WaitPermit {
    fn acquire() -> Result<Self, RuntimeError> {
        WAITERS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 8).then_some(count + 1)
            })
            .map_err(|_| {
                RuntimeError::ContextService("history projection wait capacity unavailable".into())
            })?;
        Ok(Self)
    }
}
impl Drop for WaitPermit {
    fn drop(&mut self) {
        WAITERS.fetch_sub(1, Ordering::AcqRel);
    }
}
#[derive(Default, Debug)]
pub(super) struct WaitSignal {
    epoch: Mutex<u64>,
    changed: std::sync::Condvar,
}
impl WaitSignal {
    fn notify(&self) {
        let mut epoch = self.epoch.lock().unwrap_or_else(|error| error.into_inner());
        *epoch = epoch.saturating_add(1);
        self.changed.notify_all();
    }
}
impl BrowserContext {
    pub(crate) fn change_script_close_history(
        &self,
        page: PageId,
        creator: &Creator,
        change: Change,
        revision: u64,
        count: usize,
    ) -> Result<(), RuntimeError> {
        let Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "history projection requires the active root Document".into(),
            ));
        };
        self.with_window_worker_creator_pages(
            page,
            &oryn_js::message_wire::WindowTarget::Root,
            *document_generation,
            *stream_epoch,
            false,
            false,
            |_, _, _, pages| {
                let record = pages
                    .get_mut(&page)
                    .ok_or(RuntimeError::PageNotFound(page))?;
                if record
                    .history_projection
                    .as_ref()
                    .is_none_or(|projection| &projection.creator != creator)
                {
                    let mut projection = Projection::new(creator.clone());
                    projection.apply(change, revision, count)?;
                    record.history_projection = Some(projection);
                } else {
                    record
                        .history_projection
                        .as_mut()
                        .unwrap()
                        .apply(change, revision, count)?;
                }
                record.history_projection_wake.notify();
                Ok(())
            },
        )
    }
    pub(crate) fn wait_script_close_history(
        &self,
        page: PageId,
        creator: &Creator,
    ) -> Result<usize, RuntimeError> {
        let signal = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| {
                RuntimeError::ContextService("history projection wait registry poisoned".into())
            })?
            .get(&page)
            .ok_or(RuntimeError::PageNotFound(page))?
            .history_projection_wake
            .clone();
        if let Some(count) = self.projected_script_close_history(page, creator)? {
            return Ok(count);
        }
        let _permit = WaitPermit::acquire()?;
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            let observed = *signal.epoch.lock().map_err(|_| {
                RuntimeError::ContextService("history projection wait poisoned".into())
            })?;
            if let Some(count) = self.projected_script_close_history(page, creator)? {
                return Ok(count);
            }
            let pending = self
                .inner
                .group
                .pages
                .lock()
                .map_err(|_| {
                    RuntimeError::ContextService("history projection wait registry poisoned".into())
                })?
                .get(&page)
                .and_then(|record| record.history_projection.as_ref())
                .is_some_and(|projection| {
                    projection.creator == *creator && projection.disposition == Disposition::Pending
                });
            if !pending {
                return Err(RuntimeError::ContextService(
                    "native history projection not yet published".into(),
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RuntimeError::ContextService(
                    "native history projection publication pending".into(),
                ));
            }
            let epoch = signal.epoch.lock().map_err(|_| {
                RuntimeError::ContextService("history projection wait poisoned".into())
            })?;
            if *epoch != observed {
                continue;
            }
            // All page/native authority locks are released. A separate bounded
            // context-dispatch task can publish on the same multiplexed socket.
            drop(
                signal
                    .changed
                    .wait_timeout(epoch, remaining.min(Duration::from_millis(25)))
                    .map_err(|_| {
                        RuntimeError::ContextService("history projection wait poisoned".into())
                    })?,
            );
        }
    }
    pub(crate) fn projected_script_close_history(
        &self,
        page: PageId,
        creator: &Creator,
    ) -> Result<Option<usize>, RuntimeError> {
        let Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "history projection requires the active root Document".into(),
            ));
        };
        self.with_window_worker_creator_pages(
            page,
            &oryn_js::message_wire::WindowTarget::Root,
            *document_generation,
            *stream_epoch,
            false,
            false,
            |_, _, _, pages| {
                Ok(pages
                    .get(&page)
                    .and_then(|record| record.history_projection.as_ref())
                    .and_then(|projection| projection.count(creator)))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn creator(generation: u64) -> Creator {
        Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 0,
        }
    }
    #[test]
    fn reservation_hides_previous_count_and_abort_restores_it() {
        let owner = creator(1);
        let mut state = Projection::new(owner.clone());
        state.apply(Change::Begin, 1, 1).unwrap();
        state.apply(Change::Publish, 1, 1).unwrap();
        state.apply(Change::Begin, 2, 2).unwrap();
        assert_eq!(state.count(&owner), None);
        state.apply(Change::Begin, 2, 2).unwrap();
        state.apply(Change::Abort, 2, 2).unwrap();
        assert_eq!(state.count(&owner), Some(1));
        assert!(state.apply(Change::Begin, 2, 2).is_err());
        assert!(state.apply(Change::Publish, 2, 2).is_err());
    }
    #[test]
    fn lost_publish_ack_and_late_abort_do_not_rollback() {
        let owner = creator(1);
        let mut state = Projection::new(owner.clone());
        state.apply(Change::Begin, 1, 2).unwrap();
        state.apply(Change::Publish, 1, 2).unwrap();
        state.apply(Change::Publish, 1, 2).unwrap();
        state.apply(Change::Abort, 1, 2).unwrap();
        assert_eq!(state.count(&owner), Some(2));
        assert_eq!(state.count(&creator(2)), None);
        assert!(state.apply(Change::Begin, 1, 1).is_err());
        assert!(state.apply(Change::Publish, 2, 1).is_err());
    }
    #[test]
    fn absent_begin_abort_is_idempotent_and_does_not_invent_count() {
        let owner = creator(1);
        let mut state = Projection::new(owner.clone());
        state.apply(Change::Abort, 4, 2).unwrap();
        state.apply(Change::Abort, 4, 2).unwrap();
        assert_eq!(state.count(&owner), None);
        assert!(state.apply(Change::Begin, 4, 2).is_err());
        assert!(state.apply(Change::Begin, 3, 1).is_err());
        assert!(state.apply(Change::Begin, 5, 0).is_err());
        state.apply(Change::Begin, 5, 1).unwrap();
        assert!(state.apply(Change::Begin, 6, 2).is_err());
        state.apply(Change::Publish, 5, 1).unwrap();
        assert_eq!(state.count(&owner), Some(1));
    }
    #[test]
    fn replacement_document_cannot_inherit_or_rewrite_previous_projection() {
        use oryn_js::message_wire::MessageRequest;
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://history.test/".into(), &[])
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
        let old = creator(1);
        context
            .change_script_close_history(id, &old, Change::Begin, 9, 1)
            .unwrap();
        context
            .change_script_close_history(id, &old, Change::Publish, 9, 1)
            .unwrap();
        assert_eq!(
            context.projected_script_close_history(id, &old).unwrap(),
            Some(1)
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let current = creator(2);
        assert!(context.projected_script_close_history(id, &old).is_err());
        assert!(
            context
                .change_script_close_history(id, &old, Change::Begin, 10, 2)
                .is_err()
        );
        assert_eq!(
            context
                .projected_script_close_history(id, &current)
                .unwrap(),
            None
        );
        context
            .change_script_close_history(id, &current, Change::Begin, 1, 3)
            .unwrap();
        assert_eq!(
            context
                .projected_script_close_history(id, &current)
                .unwrap(),
            None
        );
        context
            .change_script_close_history(id, &current, Change::Publish, 1, 3)
            .unwrap();
        assert_eq!(
            context
                .projected_script_close_history(id, &current)
                .unwrap(),
            Some(3)
        );
        let frame = Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Frame("f".repeat(64)),
            document_generation: 2,
            stream_epoch: 0,
        };
        assert!(
            context
                .change_script_close_history(id, &frame, Change::Begin, 2, 1)
                .is_err()
        );
    }

    #[test]
    fn waiting_close_reads_published_or_aborted_count_without_holding_authority_locks() {
        use oryn_js::message_wire::MessageRequest;
        for (completion, expected) in [(Change::Publish, 2), (Change::Abort, 1)] {
            let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
            let page = context.new_page();
            let id = page.page_id();
            let owner = creator(1);
            context
                .install_document_security(id, "https://history.test/".into(), &[])
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
            context
                .change_script_close_history(id, &owner, Change::Begin, 1, 1)
                .unwrap();
            context
                .change_script_close_history(id, &owner, Change::Publish, 1, 1)
                .unwrap();
            context
                .change_script_close_history(id, &owner, Change::Begin, 2, 2)
                .unwrap();
            let waiter_context = context.clone();
            let waiter_owner = owner.clone();
            let waiter = std::thread::spawn(move || {
                waiter_context.wait_script_close_history(id, &waiter_owner)
            });
            std::thread::sleep(Duration::from_millis(10));
            context
                .change_script_close_history(id, &owner, completion, 2, 2)
                .unwrap();
            assert_eq!(waiter.join().unwrap().unwrap(), expected);
        }
    }
    #[test]
    fn unpublished_pending_and_replaced_history_are_errors_never_ineligible_counts() {
        use oryn_js::message_wire::MessageRequest;
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page();
        let id = page.page_id();
        let owner = creator(1);
        context
            .install_document_security(id, "https://history.test/".into(), &[])
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
        assert!(context.wait_script_close_history(id, &owner).is_err());
        context
            .change_script_close_history(id, &owner, Change::Begin, 1, 1)
            .unwrap();
        assert!(
            context.wait_script_close_history(id, &owner).is_err(),
            "bounded timeout does not invent count zero"
        );
        let waiter_context = context.clone();
        let waiter_owner = owner.clone();
        let waiter =
            std::thread::spawn(move || waiter_context.wait_script_close_history(id, &waiter_owner));
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            waiter.join().unwrap().is_err(),
            "wait revalidates exact document authority"
        );
    }
}
