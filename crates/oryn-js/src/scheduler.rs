//! Browser task scheduling independent of the JavaScript engine adapter.

use std::{
    cmp::Reverse,
    collections::{BTreeSet, BinaryHeap, VecDeque},
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
pub struct PageScheduler<T> {
    now_ms: u64,
    next_sequence: u64,
    tasks: VecDeque<Task<T>>,
    timers: BinaryHeap<Reverse<Timer<T>>>,
    cancelled_timers: BTreeSet<u64>,
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
            cancelled_timers: BTreeSet::new(),
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
        self.timers
            .peek()
            .map(|timer| timer.0.due_ms.saturating_sub(self.now_ms))
    }

    pub fn has_ready_task(&self) -> bool {
        !self.tasks.is_empty()
    }

    pub fn queue(&mut self, task: Task<T>) {
        self.tasks.push_back(task);
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
        self.timers.push(Reverse(Timer {
            due_ms: self.now_ms.saturating_add(delay.as_millis() as u64),
            sequence: id,
            interval_ms,
            task: task(id),
        }));
        id
    }

    pub fn cancel_timer(&mut self, id: u64) {
        self.cancelled_timers.insert(id);
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
            if self.cancelled_timers.remove(&timer.sequence) {
                continue;
            }
            self.tasks.push_back(timer.task.clone());
            if let Some(interval_ms) = timer.interval_ms {
                timer.due_ms = timer.due_ms.saturating_add(interval_ms.max(1));
                self.timers.push(Reverse(timer));
            }
        }
    }

    pub fn run_checkpoint<F>(&mut self, mut run_task: F) -> CheckpointReport
    where
        F: FnMut(Task<T>),
    {
        let Some(task) = self.tasks.pop_front() else {
            return CheckpointReport::IDLE;
        };
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
        self.tasks
            .retain(|task| task.realm != realm || task.document_generation != document_generation);
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
}
