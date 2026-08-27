use std::{
    cmp::Reverse,
    collections::{BinaryHeap, VecDeque},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkKind {
    Task,
    Microtask,
    Timer,
}

#[derive(Debug)]
struct Timer<T> {
    due_ms: u64,
    sequence: u64,
    work: T,
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

#[derive(Debug)]
pub struct EventLoop<T> {
    now_ms: u64,
    next_sequence: u64,
    tasks: VecDeque<T>,
    microtasks: VecDeque<T>,
    timers: BinaryHeap<Reverse<Timer<T>>>,
}

impl<T> Default for EventLoop<T> {
    fn default() -> Self {
        Self {
            now_ms: 0,
            next_sequence: 0,
            tasks: VecDeque::new(),
            microtasks: VecDeque::new(),
            timers: BinaryHeap::new(),
        }
    }
}

impl<T> EventLoop<T> {
    pub fn queue_task(&mut self, work: T) {
        self.tasks.push_back(work);
    }

    pub fn queue_microtask(&mut self, work: T) {
        self.microtasks.push_back(work);
    }

    pub fn set_timeout(&mut self, delay_ms: u64, work: T) {
        self.timers.push(Reverse(Timer {
            due_ms: self.now_ms.saturating_add(delay_ms),
            sequence: self.next_sequence,
            work,
        }));
        self.next_sequence += 1;
    }

    pub fn advance_to(&mut self, now_ms: u64) {
        self.now_ms = self.now_ms.max(now_ms);
        while self
            .timers
            .peek()
            .is_some_and(|timer| timer.0.due_ms <= self.now_ms)
        {
            let timer = self.timers.pop().expect("peeked timer").0;
            self.tasks.push_back(timer.work);
        }
    }

    pub fn run_checkpoint<F>(&mut self, mut run: F) -> bool
    where
        F: FnMut(WorkKind, T, &mut Self),
    {
        let Some(task) = self.tasks.pop_front() else {
            return false;
        };
        run(WorkKind::Task, task, self);
        while let Some(microtask) = self.microtasks.pop_front() {
            run(WorkKind::Microtask, microtask, self);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microtasks_drain_after_each_task_before_the_next_timer() {
        let mut loop_ = EventLoop::default();
        loop_.queue_task("script");
        loop_.set_timeout(0, "timer");
        loop_.advance_to(0);
        let mut order = Vec::new();
        loop_.run_checkpoint(|_, work, loop_| {
            order.push(work);
            if work == "script" {
                loop_.queue_microtask("promise");
            }
        });
        loop_.run_checkpoint(|_, work, _| order.push(work));

        assert_eq!(order, ["script", "promise", "timer"]);
    }

    #[test]
    fn equal_deadline_timers_keep_registration_order() {
        let mut loop_ = EventLoop::default();
        loop_.set_timeout(5, "first");
        loop_.set_timeout(5, "second");
        loop_.advance_to(5);
        let mut order = Vec::new();
        while loop_.run_checkpoint(|_, work, _| order.push(work)) {}
        assert_eq!(order, ["first", "second"]);
    }
}
