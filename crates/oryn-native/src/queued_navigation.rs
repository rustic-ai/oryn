//! Bounded ownership and observable terminal results for parent-queued navigation.
use oryn_js::navigation::{NavigationExecutionOutcome, NavigationExecutionToken};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize)]
pub enum OutcomeStage {
    Execution,
    Admission,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettlementAck {
    Accepted,
    Retired,
}
#[derive(Clone, Debug, Serialize)]
pub struct Outcome {
    pub stage: OutcomeStage,
    pub id: u64,
    pub target: Option<String>,
    pub result: Result<u64, String>,
    pub error_truncated: bool,
    pub execution: Option<NavigationExecutionOutcome>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub retired: bool,
    pub active: usize,
    pub outcomes: Vec<Outcome>,
    pub pruned_outcomes: u64,
}
#[derive(Debug, Default)]
struct State {
    retired: bool,
    next: u64,
    latest: std::collections::BTreeMap<Option<String>, u64>,
    active: usize,
    outcomes: VecDeque<Outcome>,
    pruned: u64,
    receipts: std::collections::BTreeMap<Vec<u8>, (u64, Option<String>, bool)>,
    admitted: std::collections::BTreeSet<Vec<u8>>,
    settled: VecDeque<(Vec<u8>, [u8; 32], bool)>,
}
#[derive(Debug, Default)]
pub(crate) struct Custody {
    state: Mutex<State>,
    changed: Condvar,
}
pub(crate) struct Ticket {
    owner: Arc<Custody>,
    id: u64,
    target: Option<String>,
    finished: bool,
    stage: OutcomeStage,
}
impl Custody {
    #[cfg(test)]
    pub(crate) fn admit(self: &Arc<Self>) -> Result<Ticket, String> {
        self.admit_target(None)
    }
    pub(crate) fn admit_target(self: &Arc<Self>, target: Option<String>) -> Result<Ticket, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "navigation custody poisoned")?;
        if state.retired {
            return Err("navigation page retired".into());
        }
        if state.active >= 32 {
            return Err("pending page navigation capacity exceeded".into());
        }
        let id = state
            .next
            .checked_add(1)
            .ok_or("navigation identity exhausted")?;
        state.next = id;
        state.active += 1;
        if target.is_none() {
            state.latest.clear();
        }
        state.latest.insert(target.clone(), id);
        Ok(Ticket {
            owner: self.clone(),
            id,
            target,
            finished: false,
            stage: OutcomeStage::Execution,
        })
    }
    pub(crate) fn retire(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.retired = true;
        for (token, (id, target, _)) in std::mem::take(&mut state.receipts) {
            settle_record(
                &mut state,
                token,
                id,
                target,
                NavigationExecutionOutcome::Failed {
                    message: "navigation page retired before execution settlement".into(),
                },
                true,
            );
        }
        self.changed.notify_all();
    }
    pub(crate) fn claim(&self, token: &NavigationExecutionToken) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "navigation custody poisoned")?;
        if state.retired {
            return Ok(false);
        }
        let Some((id, target, claimed)) = state.receipts.get(&token.0).cloned() else {
            return Ok(false);
        };
        if claimed {
            return Ok(true);
        }
        if state.latest.get(&target) != Some(&id) {
            state.receipts.remove(&token.0);
            settle_record(
                &mut state,
                token.0.clone(),
                id,
                target,
                NavigationExecutionOutcome::Canceled,
                false,
            );
            self.changed.notify_all();
            return Ok(false);
        }
        state.receipts.get_mut(&token.0).unwrap().2 = true;
        Ok(true)
    }
    /// Returns true only for the first accepted execution notification. Queue
    /// admission and claim alone never permit an auditing side effect.
    pub(crate) fn accept_execution(
        &self,
        token: &NavigationExecutionToken,
    ) -> Result<Option<bool>, String> {
        if token.0.len() != 32 {
            return Err("invalid navigation execution receipt".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "navigation custody poisoned")?;
        if state.retired || state.settled.iter().any(|(key, _, _)| key == &token.0) {
            return Ok(None);
        }
        let Some(entry) = state.receipts.get(&token.0) else {
            return Ok(None);
        };
        if !entry.2 {
            return Err("navigation execution receipt has not been claimed".into());
        }
        Ok(Some(state.admitted.insert(token.0.clone())))
    }
    pub(crate) fn fail_unclaimed(
        &self,
        token: &NavigationExecutionToken,
        message: String,
    ) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "navigation custody poisoned")?;
        if state.settled.iter().any(|(key, _, _)| key == &token.0) {
            return Ok(false);
        }
        let entry = state
            .receipts
            .get(&token.0)
            .ok_or("unknown navigation execution receipt")?;
        if entry.2 {
            return Ok(false);
        }
        let (id, target, _) = state.receipts.remove(&token.0).unwrap();
        settle_record(
            &mut state,
            token.0.clone(),
            id,
            target,
            NavigationExecutionOutcome::Failed { message },
            false,
        );
        self.changed.notify_all();
        Ok(true)
    }
    pub(crate) fn settle(
        &self,
        token: &NavigationExecutionToken,
        outcome: NavigationExecutionOutcome,
    ) -> Result<SettlementAck, String> {
        if token.0.len() != 32 {
            return Err("invalid navigation execution receipt".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "navigation custody poisoned")?;
        let fingerprint = execution_fingerprint(&outcome);
        if let Some((_, previous, retired)) =
            state.settled.iter().find(|(key, _, _)| key == &token.0)
        {
            if *retired {
                return Ok(SettlementAck::Retired);
            }
            return if previous == &fingerprint {
                Ok(SettlementAck::Accepted)
            } else {
                Err("conflicting navigation execution settlement".into())
            };
        }
        if !state
            .receipts
            .get(&token.0)
            .ok_or("unknown navigation execution receipt")?
            .2
        {
            return Err("navigation execution receipt has not been claimed".into());
        }
        let (id, target, _) = state.receipts.remove(&token.0).unwrap();
        settle_record(&mut state, token.0.clone(), id, target, outcome, false);
        self.changed.notify_all();
        Ok(SettlementAck::Accepted)
    }
    pub(crate) fn drain(&self, budget: Duration) -> Result<(), String> {
        let deadline = Instant::now()
            .checked_add(budget)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .state
            .lock()
            .map_err(|_| "navigation custody poisoned")?;
        while state.active != 0 {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or("queued navigation remains owned")?;
            state = self
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| "navigation custody poisoned")?
                .0;
        }
        Ok(())
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Snapshot {
            retired: state.retired,
            active: state.active,
            outcomes: state.outcomes.iter().cloned().collect(),
            pruned_outcomes: state.pruned,
        }
    }
}
impl Ticket {
    /// Move active custody into the page-owned receipt table, without an Arc
    /// cycle or treating dispatch acknowledgement as completed execution.
    pub(crate) fn execution_receipt(&mut self) -> Result<NavigationExecutionToken, String> {
        if self.finished {
            return Err("navigation ticket already transferred".into());
        }
        let mut token = vec![0; 32];
        getrandom::fill(&mut token).map_err(|_| "navigation receipt entropy unavailable")?;
        let mut state = self
            .owner
            .state
            .lock()
            .map_err(|_| "navigation custody poisoned")?;
        if state.retired || state.latest.get(&self.target) != Some(&self.id) {
            return Err("navigation superseded before execution transfer".into());
        }
        if state.receipts.contains_key(&token)
            || state.settled.iter().any(|(key, _, _)| key == &token)
        {
            return Err("navigation receipt collision".into());
        }
        state
            .receipts
            .insert(token.clone(), (self.id, self.target.clone(), false));
        self.finished = true;
        Ok(NavigationExecutionToken(token))
    }
    pub(crate) fn runnable(&self) -> bool {
        self.owner
            .state
            .lock()
            .map(|state| !state.retired && state.latest.get(&self.target) == Some(&self.id))
            .unwrap_or(false)
    }
    pub(crate) fn finish(mut self, result: Result<u64, String>) {
        self.publish(result);
    }
    pub(crate) fn finish_admission(mut self, result: Result<u64, String>) {
        self.stage = OutcomeStage::Admission;
        self.publish(result);
    }
    fn publish(&mut self, mut result: Result<u64, String>) {
        if self.finished {
            return;
        }
        let mut error_truncated = false;
        if let Err(message) = &mut result
            && message.len() > 4096
        {
            let mut boundary = 4096;
            while !message.is_char_boundary(boundary) {
                boundary -= 1;
            }
            message.truncate(boundary);
            error_truncated = true;
        }
        let mut state = self.owner.state.lock().unwrap_or_else(|e| e.into_inner());
        state.active -= 1;
        if state.outcomes.len() == 32 {
            state.outcomes.pop_front();
            state.pruned = state.pruned.saturating_add(1);
        }
        if state.latest.get(&self.target) == Some(&self.id) {
            state.latest.remove(&self.target);
        }
        state.outcomes.push_back(Outcome {
            stage: self.stage.clone(),
            id: self.id,
            target: self.target.clone(),
            result,
            error_truncated,
            execution: None,
        });
        self.finished = true;
        self.owner.changed.notify_all();
    }
}

fn normalize_execution(outcome: &mut NavigationExecutionOutcome) -> bool {
    if let NavigationExecutionOutcome::Failed { message } = outcome
        && message.len() > 4096
    {
        let mut end = 4096;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
        return true;
    }
    false
}
fn execution_fingerprint(outcome: &NavigationExecutionOutcome) -> [u8; 32] {
    Sha256::digest(
        serde_json::to_vec(outcome).expect("navigation execution outcome is serializable"),
    )
    .into()
}
fn settle_record(
    state: &mut State,
    token: Vec<u8>,
    id: u64,
    target: Option<String>,
    mut execution: NavigationExecutionOutcome,
    retired: bool,
) {
    state.admitted.remove(&token);
    let fingerprint = execution_fingerprint(&execution);
    let error_truncated = normalize_execution(&mut execution);
    let result = match &execution {
        NavigationExecutionOutcome::Committed {
            document_generation,
        } => Ok(*document_generation),
        NavigationExecutionOutcome::Completed => Ok(0),
        NavigationExecutionOutcome::Canceled => Err("navigation canceled".into()),
        NavigationExecutionOutcome::Failed { message } => Err(message.clone()),
    };
    state.active -= 1;
    if state.latest.get(&target) == Some(&id) {
        state.latest.remove(&target);
    }
    if state.outcomes.len() == 32 {
        state.outcomes.pop_front();
        state.pruned = state.pruned.saturating_add(1);
    }
    state.outcomes.push_back(Outcome {
        stage: OutcomeStage::Execution,
        id,
        target,
        result,
        error_truncated,
        execution: Some(execution.clone()),
    });
    if state.settled.len() == 64 {
        state.settled.pop_front();
    }
    state.settled.push_back((token, fingerprint, retired));
}
impl Drop for Ticket {
    fn drop(&mut self) {
        if !self.finished {
            self.publish(Err(
                "navigation task abandoned before terminal publication".into()
            ));
        }
    }
}

type Job = Box<dyn FnOnce() + Send + 'static>;
const MAX_QUEUED_BYTES: usize = 64 * 1024 * 1024;
static QUEUED_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct Bytes(usize);
impl Bytes {
    fn reserve(bytes: usize) -> Result<Self, String> {
        use std::sync::atomic::Ordering;
        QUEUED_BYTES
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|total| *total <= MAX_QUEUED_BYTES)
            })
            .map(|_| Self(bytes))
            .map_err(|_| "queued navigation byte capacity exceeded".into())
    }
}
impl Drop for Bytes {
    fn drop(&mut self) {
        QUEUED_BYTES.fetch_sub(self.0, std::sync::atomic::Ordering::AcqRel);
    }
}
struct Queue {
    jobs: VecDeque<(Job, Bytes)>,
    active: usize,
    stopping: bool,
}
struct Executor {
    queue: Mutex<Queue>,
    changed: Condvar,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}
static EXECUTOR: OnceLock<Result<Arc<Executor>, String>> = OnceLock::new();
pub(crate) fn submit(bytes: usize, job: impl FnOnce() + Send + 'static) -> Result<(), String> {
    EXECUTOR
        .get_or_init(Executor::new)
        .as_ref()
        .map_err(Clone::clone)?
        .submit(Box::new(job), bytes)
}
impl Executor {
    fn new() -> Result<Arc<Self>, String> {
        let pool = Arc::new(Self {
            queue: Mutex::new(Queue {
                jobs: VecDeque::new(),
                active: 0,
                stopping: false,
            }),
            changed: Condvar::new(),
            threads: Mutex::new(Vec::new()),
        });
        for lane in 0..4 {
            let worker = pool.clone();
            match std::thread::Builder::new()
                .name(format!("oryn-navigation-{lane}"))
                .spawn(move || worker.run())
            {
                Ok(thread) => pool
                    .threads
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(thread),
                Err(error) => {
                    pool.queue
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .stopping = true;
                    pool.changed.notify_all();
                    for thread in
                        std::mem::take(&mut *pool.threads.lock().unwrap_or_else(|e| e.into_inner()))
                    {
                        let _ = thread.join();
                    }
                    return Err(error.to_string());
                }
            }
        }
        Ok(pool)
    }
    fn submit(&self, job: Job, bytes: usize) -> Result<(), String> {
        let mut queue = self
            .queue
            .lock()
            .map_err(|_| "navigation executor poisoned")?;
        if queue.stopping || queue.active >= 128 {
            return Err("navigation executor capacity unavailable".into());
        }
        let bytes = Bytes::reserve(bytes)?;
        queue.active += 1;
        queue.jobs.push_back((job, bytes));
        self.changed.notify_one();
        Ok(())
    }
    fn run(&self) {
        loop {
            let job = {
                let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if let Some(job) = queue.jobs.pop_front() {
                        break job;
                    }
                    if queue.stopping {
                        return;
                    }
                    queue = self.changed.wait(queue).unwrap_or_else(|e| e.into_inner());
                }
            };
            let (job, _bytes) = job;
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
            self.queue.lock().unwrap_or_else(|e| e.into_inner()).active -= 1;
            self.changed.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepted_execution_requires_claim_is_idempotent_and_does_not_settle() {
        let page = Arc::new(Custody::default());
        let mut ticket = page.admit().unwrap();
        let receipt = ticket.execution_receipt().unwrap();
        assert!(page.accept_execution(&receipt).is_err());
        assert!(page.claim(&receipt).unwrap());
        assert_eq!(page.accept_execution(&receipt).unwrap(), Some(true));
        assert_eq!(page.accept_execution(&receipt).unwrap(), Some(false));
        assert_eq!(page.snapshot().active, 1);
        assert!(page.snapshot().outcomes.is_empty());
        page.settle(&receipt, NavigationExecutionOutcome::Completed)
            .unwrap();
        assert_eq!(page.accept_execution(&receipt).unwrap(), None);
        assert_eq!(page.snapshot().active, 0);
        assert!(page.state.lock().unwrap().admitted.is_empty());
    }
    #[test]
    fn execution_receipt_keeps_custody_until_exact_terminal_settlement() {
        let page = Arc::new(Custody::default());
        let foreign = Arc::new(Custody::default());
        let mut ticket = page.admit().unwrap();
        let token = ticket.execution_receipt().unwrap();
        drop(ticket);
        assert_eq!(page.snapshot().active, 1);
        assert!(page.snapshot().outcomes.is_empty());
        assert!(page.drain(Duration::ZERO).is_err());
        let outcome = NavigationExecutionOutcome::Committed {
            document_generation: 7,
        };
        assert!(foreign.settle(&token, outcome.clone()).is_err());
        assert!(
            page.settle(&token, outcome.clone()).is_err(),
            "publication requires an acknowledged execution owner"
        );
        assert!(page.claim(&token).unwrap());
        assert!(page.claim(&token).unwrap());
        page.settle(&token, outcome.clone()).unwrap();
        page.settle(&token, outcome).unwrap();
        assert!(
            page.settle(&token, NavigationExecutionOutcome::Completed)
                .is_err()
        );
        page.drain(Duration::ZERO).unwrap();
        let snapshot = page.snapshot();
        assert_eq!(snapshot.outcomes.len(), 1);
        assert!(matches!(
            snapshot.outcomes[0].stage,
            OutcomeStage::Execution
        ));
        assert_eq!(snapshot.outcomes[0].result, Ok(7));
        assert!(matches!(
            snapshot.outcomes[0].execution,
            Some(NavigationExecutionOutcome::Committed {
                document_generation: 7
            })
        ));
    }
    #[test]
    fn retired_receipts_publish_failure_and_do_not_accept_late_success() {
        let page = Arc::new(Custody::default());
        let mut ticket = page.admit().unwrap();
        let token = ticket.execution_receipt().unwrap();
        drop(ticket);
        assert!(page.claim(&token).unwrap());
        page.retire();
        page.drain(Duration::ZERO).unwrap();
        assert_eq!(page.snapshot().outcomes.len(), 1);
        assert!(page.snapshot().outcomes[0].result.is_err());
        assert_eq!(
            page.settle(&token, NavigationExecutionOutcome::Completed)
                .unwrap(),
            SettlementAck::Retired
        );
        assert!(
            page.snapshot().outcomes[0].result.is_err(),
            "disposal acknowledgement never replaces retirement evidence with success"
        );
        let mut unknown = token.0.clone();
        unknown[0] ^= 1;
        assert!(
            page.settle(
                &NavigationExecutionToken(unknown),
                NavigationExecutionOutcome::Completed
            )
            .is_err()
        );
        page.retire();
        assert_eq!(page.snapshot().outcomes.len(), 1);
    }
    #[test]
    fn truncated_execution_errors_keep_exact_replay_identity() {
        let page = Arc::new(Custody::default());
        let mut ticket = page.admit().unwrap();
        let token = ticket.execution_receipt().unwrap();
        let message = format!("{}a", "€".repeat(4096));
        assert!(page.claim(&token).unwrap());
        page.settle(
            &token,
            NavigationExecutionOutcome::Failed {
                message: message.clone(),
            },
        )
        .unwrap();
        page.settle(&token, NavigationExecutionOutcome::Failed { message })
            .unwrap();
        assert!(
            page.settle(
                &token,
                NavigationExecutionOutcome::Failed {
                    message: format!("{}b", "€".repeat(4096))
                }
            )
            .is_err()
        );
        let snapshot = page.snapshot();
        assert!(snapshot.outcomes[0].error_truncated);
        assert!(snapshot.outcomes[0].result.as_ref().unwrap_err().len() <= 4096);
    }
    #[test]
    fn transferred_receipts_share_admission_capacity_and_bound_replay_history() {
        let page = Arc::new(Custody::default());
        let mut receipts = Vec::new();
        for index in 0..32 {
            let mut ticket = page.admit_target(Some(format!("frame-{index}"))).unwrap();
            receipts.push(ticket.execution_receipt().unwrap());
        }
        assert!(
            page.admit().is_err(),
            "dispatch acknowledgement does not free admission capacity"
        );
        let oldest = receipts[0].clone();
        for receipt in receipts {
            assert!(page.claim(&receipt).unwrap());
            page.settle(&receipt, NavigationExecutionOutcome::Completed)
                .unwrap();
        }
        let mut newest = None;
        for _ in 0..64 {
            let mut ticket = page.admit().unwrap();
            let receipt = ticket.execution_receipt().unwrap();
            assert!(page.claim(&receipt).unwrap());
            page.settle(&receipt, NavigationExecutionOutcome::Completed)
                .unwrap();
            newest = Some(receipt);
        }
        let snapshot = page.snapshot();
        assert_eq!(snapshot.active, 0);
        assert_eq!(snapshot.outcomes.len(), 32);
        assert_eq!(snapshot.pruned_outcomes, 64);
        assert_eq!(page.state.lock().unwrap().settled.len(), 64);
        assert!(
            page.settle(&oldest, NavigationExecutionOutcome::Completed)
                .is_err(),
            "expired replay evidence cannot invent an acknowledgement"
        );
        page.settle(&newest.unwrap(), NavigationExecutionOutcome::Completed)
            .unwrap();
        assert_eq!(page.snapshot().pruned_outcomes, 64);
    }
    #[test]
    fn dispatch_failure_and_receiver_claim_have_one_terminal_owner() {
        for receiver_first in [false, true] {
            let page = Arc::new(Custody::default());
            let mut ticket = page.admit().unwrap();
            let receipt = ticket.execution_receipt().unwrap();
            drop(ticket);
            if receiver_first {
                assert!(page.claim(&receipt).unwrap());
                assert!(
                    !page
                        .fail_unclaimed(&receipt, "lost dispatch reply".into())
                        .unwrap()
                );
                assert_eq!(page.snapshot().active, 1);
                page.settle(
                    &receipt,
                    NavigationExecutionOutcome::Failed {
                        message: "receiver failure".into(),
                    },
                )
                .unwrap();
            } else {
                assert!(
                    page.fail_unclaimed(&receipt, "dispatch unavailable".into())
                        .unwrap()
                );
                assert!(
                    !page.claim(&receipt).unwrap(),
                    "late receiver must not begin effects"
                );
            }
            assert_eq!(page.snapshot().active, 0);
            assert_eq!(page.snapshot().outcomes.len(), 1);
        }
    }
    #[test]
    fn supersession_before_claim_rejects_effects_but_claimed_execution_retains_ownership() {
        for claimed in [false, true] {
            let page = Arc::new(Custody::default());
            let mut old = page.admit().unwrap();
            let receipt = old.execution_receipt().unwrap();
            drop(old);
            if claimed {
                assert!(page.claim(&receipt).unwrap());
            }
            let newest = page.admit().unwrap();
            assert_eq!(page.claim(&receipt).unwrap(), claimed);
            assert!(
                newest.runnable(),
                "settling the old receipt must not remove the newer target owner"
            );
            if claimed {
                assert_eq!(page.snapshot().active, 2);
                page.settle(&receipt, NavigationExecutionOutcome::Canceled)
                    .unwrap();
            }
            assert_eq!(page.snapshot().active, 1);
            assert_eq!(
                page.snapshot().outcomes[0].execution,
                Some(NavigationExecutionOutcome::Canceled)
            );
            newest.finish(Ok(8));
            page.drain(Duration::ZERO).unwrap();
        }
    }

    #[test]
    fn retirement_keeps_jobs_owned_and_terminal_failures_observable() {
        let page = Arc::new(Custody::default());
        let sibling = Arc::new(Custody::default());
        let running = page.admit().unwrap();
        let queued = page.admit().unwrap();
        page.retire();
        assert!(!queued.runnable());
        assert!(page.admit().is_err());
        assert!(page.drain(Duration::ZERO).is_err());
        drop(queued);
        running.finish(Err("navigation rejected by policy".into()));
        page.drain(Duration::ZERO).unwrap();
        let status = page.snapshot();
        assert_eq!(status.active, 0);
        assert_eq!(status.outcomes.len(), 2);
        assert!(
            status
                .outcomes
                .iter()
                .all(|outcome| outcome.result.is_err())
        );
        sibling.admit().unwrap().finish(Ok(7));
        assert_eq!(sibling.snapshot().outcomes[0].result, Ok(7));
    }
    #[test]
    fn admission_and_history_are_bounded_without_silent_loss() {
        let page = Arc::new(Custody::default());
        let tickets = (0..32).map(|_| page.admit().unwrap()).collect::<Vec<_>>();
        assert!(page.admit().is_err());
        drop(tickets);
        for _ in 0..4 {
            page.admit().unwrap().finish(Ok(1));
        }
        let status = page.snapshot();
        assert_eq!(status.active, 0);
        assert_eq!(status.outcomes.len(), 32);
        assert_eq!(status.pruned_outcomes, 4);
        page.admit().unwrap().finish(Err("€".repeat(4096)));
        let status = page.snapshot();
        let last = status.outcomes.last().unwrap();
        assert!(last.error_truncated);
        assert!(last.result.as_ref().unwrap_err().len() <= 4096);
    }

    #[test]
    fn rejected_executor_submission_releases_ticket_and_owned_captures() {
        let executor = Executor {
            queue: Mutex::new(Queue {
                jobs: VecDeque::new(),
                active: 128,
                stopping: false,
            }),
            changed: Condvar::new(),
            threads: Mutex::new(Vec::new()),
        };
        let page = Arc::new(Custody::default());
        let ticket = page.admit().unwrap();
        let capture = Arc::new(());
        let worker_capture = capture.clone();
        assert!(
            executor
                .submit(
                    Box::new(move || {
                        drop(worker_capture);
                        ticket.finish(Ok(1));
                    }),
                    1
                )
                .is_err()
        );
        assert_eq!(Arc::strong_count(&capture), 1);
        let status = page.snapshot();
        assert_eq!(status.active, 0);
        assert!(status.outcomes[0].result.is_err());
    }

    #[test]
    fn superseded_queued_navigation_keeps_custody_but_cannot_start_an_effect() {
        let page = Arc::new(Custody::default());
        let first = page.admit().unwrap();
        let latest = page.admit().unwrap();
        assert!(!first.runnable());
        assert!(latest.runnable());
        assert_eq!(page.snapshot().active, 2);
        first.finish(Err("superseded before execution".into()));
        latest.finish(Ok(3));
        let snapshot = page.snapshot();
        assert_eq!(snapshot.active, 0);
        assert!(snapshot.outcomes[0].result.is_err());
        assert_eq!(snapshot.outcomes[1].result, Ok(3));
    }

    #[test]
    fn oversized_navigation_payload_is_rejected_without_running_the_job() {
        let executor = Executor {
            queue: Mutex::new(Queue {
                jobs: VecDeque::new(),
                active: 0,
                stopping: false,
            }),
            changed: Condvar::new(),
            threads: Mutex::new(Vec::new()),
        };
        let page = Arc::new(Custody::default());
        let ticket = page.admit().unwrap();
        assert!(
            executor
                .submit(
                    Box::new(move || {
                        ticket.finish(Ok(1));
                        panic!("oversized job ran");
                    }),
                    MAX_QUEUED_BYTES + 1
                )
                .is_err()
        );
        assert_eq!(page.snapshot().active, 0);
        assert!(executor.queue.lock().unwrap().jobs.is_empty());
    }

    #[tokio::test]
    #[cfg(all(feature = "in-process-probe", feature = "v8-host"))]
    async fn native_queued_navigation_records_success_failure_and_closed_admission() {
        struct FailTransport;
        impl crate::network::NetworkBroker for FailTransport {
            fn send(
                &self,
                _: crate::network::NetworkRequest,
            ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                Err(crate::network::NetworkError::Transport(
                    "injected queued fetch failure".into(),
                ))
            }
        }
        let context = crate::Browser::in_process_probe().new_context_with_broker(
            Default::default(),
            crate::network::SharedNetworkBroker::new(FailTransport),
        );
        let page = context.new_page();
        let id = page.page_id();
        context
            .queue_resource_navigation(
                id,
                "data:text/html,%3Ctitle%3Equeued%3C/title%3E".into(),
                None,
                None,
                None,
            )
            .unwrap();
        let wait = |expected| {
            let context = context.clone();
            async move {
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let state = context.queued_navigation_status(id).unwrap();
                        if state.active == 0 && state.outcomes.len() == expected {
                            return state;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap()
            }
        };
        let first = wait(1).await;
        assert!(first.outcomes[0].result.is_ok());
        // Malformed URLs fail synchronously before an admission ticket exists.
        assert!(
            context
                .queue_resource_navigation(id, "http://[invalid".into(), None, None, None)
                .is_err()
        );
        let rejected = context.queued_navigation_status(id).unwrap();
        assert_eq!(rejected.active, 0);
        assert_eq!(rejected.outcomes.len(), 1);
        // A valid URL reaches the owned asynchronous fetch and records failure.
        context
            .queue_resource_navigation(id, "https://queued.test/fail".into(), None, None, None)
            .unwrap();
        let second = wait(2).await;
        assert!(second.outcomes[1].result.is_err());
        assert!(second.outcomes[1].id > second.outcomes[0].id);
        page.close().await.unwrap();
        let closed = context.queued_navigation_status(id).unwrap();
        assert!(closed.retired);
        assert_eq!(closed.active, 0);
        assert!(
            context
                .queue_resource_navigation(id, "data:text/html,closed".into(), None, None, None)
                .is_err()
        );
    }
}

#[cfg(test)]
mod frame_target_tests {
    use super::*;
    #[test]
    fn frame_supersession_is_target_specific_and_root_supersedes_descendants() {
        let custody = Arc::new(Custody::default());
        let first = custody.admit_target(Some("first".into())).unwrap();
        let second = custody.admit_target(Some("second".into())).unwrap();
        assert!(first.runnable());
        assert!(second.runnable());
        let replacement = custody.admit_target(Some("first".into())).unwrap();
        assert!(!first.runnable());
        assert!(second.runnable());
        assert!(replacement.runnable());
        first.finish(Err("superseded".into()));
        second.finish(Ok(4));
        let root = custody.admit().unwrap();
        assert!(!replacement.runnable());
        replacement.finish(Err("root navigation superseded frame".into()));
        root.finish(Ok(5));
        let snapshot = custody.snapshot();
        assert_eq!(snapshot.active, 0);
        assert_eq!(snapshot.outcomes[1].target.as_deref(), Some("second"));
        assert_eq!(snapshot.outcomes[1].result, Ok(4));
        assert_eq!(snapshot.outcomes.last().unwrap().target, None);
    }
}

#[cfg(test)]
mod admission_receipt_tests {
    use super::*;
    #[test]
    fn accepted_remote_navigation_is_not_reported_as_execution_completion() {
        let custody = Arc::new(Custody::default());
        let ticket = custody.admit().unwrap();
        ticket.finish_admission(Ok(7));
        let snapshot = custody.snapshot();
        assert_eq!(snapshot.active, 0);
        assert!(matches!(
            snapshot.outcomes[0].stage,
            OutcomeStage::Admission
        ));
        assert_eq!(snapshot.outcomes[0].result, Ok(7));
    }
}
