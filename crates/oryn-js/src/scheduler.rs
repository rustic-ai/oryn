//! Browser task scheduling independent of the JavaScript engine adapter.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque},
    time::Duration,
};

use crate::realm::RealmId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskSource {
    Parser,
    Script,
    UserInteraction,
    Networking,
    Timer,
    PostedMessage,
    Storage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task<T> {
    pub source: TaskSource,
    pub realm: RealmId,
    pub document_generation: u64,
    pub action_id: Option<u64>,
    pub payload: T,
}

#[derive(Debug)]
struct Timer<T> {
    due_ms: u64,
    sequence: u64,
    interval_ms: Option<u64>,
    task: Task<T>,
}

impl<T> PartialEq for Timer<T> {
    fn eq(&self, other: &Self) -> bool {
        (self.due_ms, self.sequence) == (other.due_ms, other.sequence)
    }
}

impl<T> Eq for Timer<T> {}

impl<T> PartialOrd for Timer<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Timer<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.due_ms, self.sequence).cmp(&(other.due_ms, other.sequence))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointReport {
    pub ran_task: bool,
    pub run_microtasks: bool,
    pub deliver_custom_element_reactions: bool,
    pub deliver_mutation_observers: bool,
    pub flush_style: bool,
    pub flush_semantics: bool,
}

impl CheckpointReport {
    const IDLE: Self = Self {
        ran_task: false,
        run_microtasks: false,
        deliver_custom_element_reactions: false,
        deliver_mutation_observers: false,
        flush_style: false,
        flush_semantics: false,
    };
}

#[derive(Debug)]
struct SuspendedDocument<T> {
    started_ms: u64,
    tasks: VecDeque<(Option<u64>, Task<T>)>,
    timers: Vec<Timer<T>>,
}

#[derive(Debug)]
pub struct PageScheduler<T> {
    now_ms: u64,
    next_sequence: u64,
    tasks: VecDeque<(Option<u64>, Task<T>)>,
    timers: BinaryHeap<Reverse<Timer<T>>>,
    suspended: BTreeMap<(RealmId, u64), SuspendedDocument<T>>,
    pending_network: BTreeSet<u64>,
    network_quiet_since_ms: Option<u64>,
    network_idle_window: Duration,
}

impl<T> Default for PageScheduler<T> {
    fn default() -> Self {
        Self {
            now_ms: 0,
            next_sequence: 1,
            tasks: VecDeque::new(),
            timers: BinaryHeap::new(),
            suspended: BTreeMap::new(),
            pending_network: BTreeSet::new(),
            network_quiet_since_ms: Some(0),
            network_idle_window: Duration::from_millis(500),
        }
    }
}

impl<T> PageScheduler<T> {
    pub const fn now_ms(&self) -> u64 {
        self.now_ms
    }

    pub fn next_timer_delay_ms(&self) -> Option<u64> {
        // `advance_to` moves due timers into the ordinary task queue. Report
        // those tasks as immediately runnable before considering a later
        // timer; otherwise a caller can sleep until the next future deadline
        // while an already-due callback is stranded behind it.
        if !self.tasks.is_empty() {
            return Some(0);
        }
        self.timers
            .peek()
            .map(|timer| timer.0.due_ms.saturating_sub(self.now_ms))
    }

    /// Temporary dispatch exclusion does not alter timer deadlines or detach
    /// the tasks, unlike parking a Document in the back/forward cache.
    pub fn next_timer_delay_ms_if(
        &self,
        mut eligible: impl FnMut(&Task<T>) -> bool,
    ) -> Option<u64> {
        if self.tasks.iter().any(|(_, task)| eligible(task)) {
            return Some(0);
        }
        self.timers
            .iter()
            .filter(|timer| eligible(&timer.0.task))
            .map(|timer| timer.0.due_ms.saturating_sub(self.now_ms))
            .min()
    }

    pub fn has_ready_task(&self) -> bool {
        !self.tasks.is_empty()
    }

    pub fn queue(&mut self, task: Task<T>) {
        if let Some(paused) = self
            .suspended
            .get_mut(&(task.realm, task.document_generation))
        {
            paused.tasks.push_back((None, task));
        } else {
            self.tasks.push_back((None, task));
        }
    }

    pub fn set_timeout(&mut self, delay: Duration, task: Task<T>) -> u64 {
        self.add_timer_with(delay, None, |_| task)
    }

    pub fn set_interval(&mut self, interval: Duration, task: Task<T>) -> u64 {
        self.add_timer_with(interval, Some(interval.as_millis() as u64), |_| task)
    }

    pub fn set_timeout_with<F>(&mut self, delay: Duration, task: F) -> u64
    where
        F: FnOnce(u64) -> Task<T>,
    {
        self.add_timer_with(delay, None, task)
    }

    pub fn set_interval_with<F>(&mut self, interval: Duration, task: F) -> u64
    where
        F: FnOnce(u64) -> Task<T>,
    {
        self.add_timer_with(interval, Some(interval.as_millis() as u64), task)
    }

    fn add_timer_with<F>(&mut self, delay: Duration, interval_ms: Option<u64>, task: F) -> u64
    where
        F: FnOnce(u64) -> Task<T>,
    {
        let id = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        let task = task(id);
        let key = (task.realm, task.document_generation);
        let now = self
            .suspended
            .get(&key)
            .map_or(self.now_ms, |paused| paused.started_ms);
        let timer = Timer {
            due_ms: now.saturating_add(delay.as_millis() as u64),
            sequence: id,
            interval_ms,
            task,
        };
        if let Some(paused) = self.suspended.get_mut(&key) {
            paused.timers.push(timer);
        } else {
            self.timers.push(Reverse(timer));
        }
        id
    }

    pub fn cancel_timer(&mut self, id: u64) {
        // Cancellation removes both future and already-queued occurrences.
        // Keeping tombstones until a distant deadline retained cancelled
        // timers and reported spurious wakeups (and leaked unknown IDs).
        self.timers.retain(|timer| timer.0.sequence != id);
        self.tasks.retain(|(timer_id, _)| *timer_id != Some(id));
        for paused in self.suspended.values_mut() {
            paused.timers.retain(|timer| timer.sequence != id);
            paused.tasks.retain(|(timer_id, _)| *timer_id != Some(id));
        }
    }

    /// Freeze one document's active-time timers and queued task sources while
    /// unrelated documents on this agent continue making progress.
    pub fn suspend_document(&mut self, realm: RealmId, generation: u64) -> bool {
        let key = (realm, generation);
        if self.suspended.contains_key(&key) {
            return false;
        }
        let mut paused = SuspendedDocument {
            started_ms: self.now_ms,
            tasks: VecDeque::new(),
            timers: Vec::new(),
        };
        let matches =
            |task: &Task<T>| task.realm == realm && task.document_generation == generation;
        let mut runnable = VecDeque::new();
        for item in self.tasks.drain(..) {
            if matches(&item.1) {
                paused.tasks.push_back(item);
            } else {
                runnable.push_back(item);
            }
        }
        self.tasks = runnable;
        let mut timers = BinaryHeap::new();
        for Reverse(timer) in self.timers.drain() {
            if matches(&timer.task) {
                paused.timers.push(timer);
            } else {
                timers.push(Reverse(timer));
            }
        }
        self.timers = timers;
        self.suspended.insert(key, paused);
        true
    }

    pub fn resume_document(&mut self, realm: RealmId, generation: u64) -> bool {
        let Some(mut paused) = self.suspended.remove(&(realm, generation)) else {
            return false;
        };
        let elapsed = self.now_ms.saturating_sub(paused.started_ms);
        self.tasks.append(&mut paused.tasks);
        for mut timer in paused.timers {
            timer.due_ms = timer.due_ms.saturating_add(elapsed);
            self.timers.push(Reverse(timer));
        }
        true
    }

    pub fn begin_network(&mut self, operation_id: u64) -> bool {
        self.network_quiet_since_ms = None;
        self.pending_network.insert(operation_id)
    }

    pub fn finish_network(&mut self, operation_id: u64) -> bool {
        let removed = self.pending_network.remove(&operation_id);
        if removed && self.pending_network.is_empty() {
            self.network_quiet_since_ms = Some(self.now_ms);
        }
        removed
    }

    pub fn advance_to(&mut self, now_ms: u64)
    where
        T: Clone,
    {
        self.now_ms = self.now_ms.max(now_ms);
        while self
            .timers
            .peek()
            .is_some_and(|timer| timer.0.due_ms <= self.now_ms)
        {
            let Reverse(mut timer) = self.timers.pop().expect("peeked timer");
            self.tasks
                .push_back((Some(timer.sequence), timer.task.clone()));
            if let Some(interval_ms) = timer.interval_ms {
                timer.due_ms = timer.due_ms.saturating_add(interval_ms.max(1));
                self.timers.push(Reverse(timer));
            }
        }
    }

    pub fn run_checkpoint<F>(&mut self, run_task: F) -> CheckpointReport
    where
        F: FnMut(Task<T>),
    {
        self.run_checkpoint_if(|_| true, run_task)
    }

    pub fn run_checkpoint_if<F>(
        &mut self,
        mut eligible: impl FnMut(&Task<T>) -> bool,
        mut run_task: F,
    ) -> CheckpointReport
    where
        F: FnMut(Task<T>),
    {
        let Some(index) = self.tasks.iter().position(|(_, task)| eligible(task)) else {
            return CheckpointReport::IDLE;
        };
        let (_, task) = self.tasks.remove(index).expect("selected eligible task");
        run_task(task);
        CheckpointReport {
            ran_task: true,
            run_microtasks: true,
            deliver_custom_element_reactions: true,
            deliver_mutation_observers: true,
            flush_style: true,
            flush_semantics: true,
        }
    }

    pub fn network_idle(&self) -> bool {
        self.pending_network.is_empty()
            && self.network_quiet_since_ms.is_some_and(|quiet_since| {
                self.now_ms.saturating_sub(quiet_since)
                    >= self.network_idle_window.as_millis() as u64
            })
    }

    pub fn cancel_document(&mut self, realm: RealmId, document_generation: u64) {
        self.suspended.remove(&(realm, document_generation));
        self.tasks.retain(|(_, task)| {
            task.realm != realm || task.document_generation != document_generation
        });
        self.timers = std::mem::take(&mut self.timers)
            .into_iter()
            .filter(|timer| {
                timer.0.task.realm != realm
                    || timer.0.task.document_generation != document_generation
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(value: &'static str) -> Task<&'static str> {
        Task {
            source: TaskSource::Timer,
            realm: RealmId(1),
            document_generation: 2,
            action_id: None,
            payload: value,
        }
    }

    #[test]
    fn dispatch_exclusion_preserves_exact_document_queue_order_and_deadlines() {
        let mut scheduler = PageScheduler::default();
        scheduler.set_timeout(Duration::from_millis(5), task("blocked-first"));
        scheduler.set_timeout(Duration::from_millis(5), task("blocked-second"));
        let mut newer = task("new-document");
        newer.document_generation = 3;
        scheduler.set_timeout(Duration::from_millis(8), newer);
        scheduler.advance_to(100);
        let eligible =
            |task: &Task<&str>| task.realm != RealmId(1) || task.document_generation != 2;
        assert_eq!(scheduler.next_timer_delay_ms_if(eligible), Some(0));
        let mut values = Vec::new();
        assert!(
            scheduler
                .run_checkpoint_if(eligible, |task| values.push(task.payload))
                .ran_task
        );
        assert_eq!(values, ["new-document"]);
        assert_eq!(scheduler.next_timer_delay_ms_if(eligible), None);
        assert_eq!(scheduler.next_timer_delay_ms(), Some(0));
        while scheduler
            .run_checkpoint(|task| values.push(task.payload))
            .ran_task
        {}
        assert_eq!(values, ["new-document", "blocked-first", "blocked-second"]);
        assert_eq!(scheduler.now_ms(), 100);
    }

    #[test]
    fn timers_run_by_deadline_then_registration_order() {
        let mut scheduler = PageScheduler::default();
        scheduler.set_timeout(Duration::from_millis(10), task("late"));
        scheduler.set_timeout(Duration::from_millis(5), task("first"));
        scheduler.set_timeout(Duration::from_millis(5), task("second"));
        scheduler.advance_to(10);
        let mut values = Vec::new();
        while scheduler
            .run_checkpoint(|task| values.push(task.payload))
            .ran_task
        {}
        assert_eq!(values, ["first", "second", "late"]);
    }

    #[test]
    fn cancellation_removes_future_and_queued_occurrences_without_affecting_peer_tasks() {
        let mut scheduler = PageScheduler::default();
        let interval = scheduler.set_interval(Duration::from_millis(1), task("cancelled"));
        scheduler.advance_to(5);
        scheduler.queue(task("ordinary"));
        scheduler.cancel_timer(interval);
        scheduler.cancel_timer(u64::MAX);
        let mut values = Vec::new();
        while scheduler
            .run_checkpoint(|task| values.push(task.payload))
            .ran_task
        {}
        assert_eq!(values, ["ordinary"]);
        assert_eq!(scheduler.next_timer_delay_ms(), None);
        let future = scheduler.set_timeout(Duration::from_secs(1000), task("future"));
        scheduler.cancel_timer(future);
        assert_eq!(scheduler.next_timer_delay_ms(), None);
        scheduler.advance_to(1_000_000);
        assert!(!scheduler.has_ready_task());
    }

    #[test]
    fn ready_timer_task_preempts_the_next_future_deadline() {
        let mut scheduler = PageScheduler::default();
        scheduler.set_timeout(Duration::from_millis(4), task("ready"));
        scheduler.set_timeout(Duration::from_secs(10), task("future"));

        scheduler.advance_to(4);

        assert_eq!(scheduler.next_timer_delay_ms(), Some(0));
        let mut values = Vec::new();
        scheduler.run_checkpoint(|task| values.push(task.payload));
        assert_eq!(values, ["ready"]);
        assert_eq!(scheduler.next_timer_delay_ms(), Some(9_996));
    }

    #[test]
    fn network_idle_requires_a_real_quiet_window() {
        let mut scheduler = PageScheduler::<()>::default();
        scheduler.advance_to(500);
        assert!(scheduler.network_idle());
        scheduler.begin_network(9);
        scheduler.advance_to(2_000);
        assert!(!scheduler.network_idle());
        assert!(scheduler.finish_network(9));
        scheduler.advance_to(2_499);
        assert!(!scheduler.network_idle());
        scheduler.advance_to(2_500);
        assert!(scheduler.network_idle());
    }

    #[test]
    fn navigation_cancels_only_the_replaced_document() {
        let mut scheduler = PageScheduler::default();
        scheduler.queue(task("old"));
        scheduler.queue(Task {
            document_generation: 3,
            payload: "new",
            ..task("new")
        });
        scheduler.cancel_document(RealmId(1), 2);
        let mut values = Vec::new();
        scheduler.run_checkpoint(|task| values.push(task.payload));
        assert_eq!(values, ["new"]);
    }
    #[test]
    fn parked_document_freezes_timers_and_tasks_without_stalling_sibling() {
        let mut scheduler = PageScheduler::default();
        scheduler.set_timeout(Duration::from_millis(10), task("parked timer"));
        scheduler.queue(task("parked ready"));
        scheduler.set_timeout(
            Duration::from_millis(10),
            Task {
                realm: RealmId(9),
                ..task("sibling")
            },
        );
        assert!(scheduler.suspend_document(RealmId(1), 2));
        assert!(!scheduler.suspend_document(RealmId(1), 2));
        scheduler.advance_to(100);
        let mut values = Vec::new();
        scheduler.run_checkpoint(|task| values.push(task.payload));
        assert_eq!(values, ["sibling"]);
        assert!(!scheduler.has_ready_task());
        scheduler.queue(task("queued while parked"));
        assert!(scheduler.resume_document(RealmId(1), 2));
        scheduler.run_checkpoint(|task| values.push(task.payload));
        scheduler.run_checkpoint(|task| values.push(task.payload));
        assert_eq!(values, ["sibling", "parked ready", "queued while parked"]);
        scheduler.advance_to(109);
        assert!(!scheduler.has_ready_task());
        scheduler.advance_to(110);
        scheduler.run_checkpoint(|task| values.push(task.payload));
        assert_eq!(values.last(), Some(&"parked timer"));
    }

    #[test]
    fn cancelled_parked_timer_and_retired_document_never_reappear_on_resume() {
        let mut scheduler = PageScheduler::default();
        let id = scheduler.set_timeout(Duration::from_millis(10), task("cancelled"));
        scheduler.suspend_document(RealmId(1), 2);
        scheduler.cancel_timer(id);
        scheduler.advance_to(100);
        scheduler.resume_document(RealmId(1), 2);
        scheduler.advance_to(1000);
        assert!(!scheduler.has_ready_task());
        scheduler.suspend_document(RealmId(1), 2);
        scheduler.set_interval(Duration::from_millis(1), task("retired"));
        scheduler.queue(task("retired ready"));
        scheduler.cancel_document(RealmId(1), 2);
        assert!(!scheduler.resume_document(RealmId(1), 2));
        scheduler.advance_to(2000);
        assert!(!scheduler.has_ready_task());
    }
}
