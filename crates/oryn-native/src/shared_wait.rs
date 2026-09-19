//! Exactly-once cross-process waiter ownership. All enqueue/notify decisions
//! occur under the parent shared-memory registry lock; blocking waits never do.
use crate::agent_registry::AgentOwner;
use oryn_js::shared_memory::SharedMemoryToken;
pub(crate) use oryn_js::shared_memory::SharedWaitId as WaitId;
use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WaitOutcome {
    Pending,
    Ok,
    TimedOut,
    Cancelled,
}
pub(crate) struct Waiter {
    outcome: Mutex<WaitOutcome>,
    changed: Condvar,
    deadline: Option<Instant>,
}
impl Waiter {
    fn settle(&self, outcome: WaitOutcome) -> WaitOutcome {
        let mut state = self
            .outcome
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if *state == WaitOutcome::Pending {
            *state = outcome;
            self.changed.notify_all();
        }
        *state
    }
    pub(crate) fn poll(&self) -> WaitOutcome {
        let state = *self
            .outcome
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state == WaitOutcome::Pending
            && self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.settle(WaitOutcome::TimedOut)
        } else {
            state
        }
    }
    /// Bounded wait lets a native engine interruption checkpoint run even when
    /// the JavaScript timeout is infinite. Terminal outcomes remain stable.
    pub(crate) fn wait_slice(&self, budget: Duration) -> WaitOutcome {
        let _ = self.poll();
        let state = self
            .outcome
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let duration = self
            .deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(budget)
            .min(budget)
            .min(Duration::from_millis(50));
        let state = self
            .changed
            .wait_timeout_while(state, duration, |state| *state == WaitOutcome::Pending)
            .unwrap_or_else(|poison| poison.into_inner())
            .0;
        drop(state);
        self.poll()
    }
}
struct Entry {
    owner: AgentOwner,
    backing: SharedMemoryToken,
    offset: u64,
    order: u64,
    waiter: Arc<Waiter>,
}
#[derive(Default)]
pub(crate) struct WaitTable {
    entries: BTreeMap<WaitId, Entry>,
    next: u64,
}
impl WaitTable {
    pub(crate) fn retains(&self, backing: SharedMemoryToken) -> bool {
        self.entries.values().any(|entry| entry.backing == backing)
    }
    /// The caller performs the actual atomic memory comparison immediately
    /// before this call while holding the same registry lock as Notify.
    pub(crate) fn enqueue(
        &mut self,
        owner: &AgentOwner,
        backing: SharedMemoryToken,
        offset: u64,
        timeout: Option<Duration>,
    ) -> Result<WaitId, String> {
        if self.entries.len() >= 4096 {
            return Err("shared waiter quota exceeded".into());
        }
        let order = self
            .next
            .checked_add(1)
            .ok_or("shared waiter order exhausted")?;
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
        let id = WaitId(bytes);
        if bytes == [0; 32] || self.entries.contains_key(&id) {
            return Err("shared waiter identity collision".into());
        }
        let deadline = timeout.and_then(|timeout| Instant::now().checked_add(timeout));
        self.entries.insert(
            id,
            Entry {
                owner: owner.clone(),
                backing,
                offset,
                order,
                waiter: Arc::new(Waiter {
                    outcome: Mutex::new(WaitOutcome::Pending),
                    changed: Condvar::new(),
                    deadline,
                }),
            },
        );
        self.next = order;
        Ok(id)
    }
    pub(crate) fn get(&self, owner: &AgentOwner, id: WaitId) -> Result<Arc<Waiter>, String> {
        let entry = self.entries.get(&id).ok_or("shared waiter retired")?;
        if &entry.owner != owner {
            return Err("foreign shared waiter".into());
        }
        Ok(entry.waiter.clone())
    }
    pub(crate) fn notify(&self, backing: SharedMemoryToken, offset: u64, count: u64) -> u64 {
        let mut candidates = self
            .entries
            .values()
            .filter(|entry| entry.backing == backing && entry.offset == offset)
            .collect::<Vec<_>>();
        candidates.sort_by_key(|entry| entry.order);
        let mut notified = 0;
        for entry in candidates {
            if notified >= count {
                break;
            }
            if entry.waiter.poll() != WaitOutcome::Pending {
                continue;
            }
            if entry.waiter.settle(WaitOutcome::Ok) == WaitOutcome::Ok {
                notified += 1
            }
        }
        notified
    }
    pub(crate) fn cancel(&mut self, owner: &AgentOwner, id: WaitId) -> Result<WaitOutcome, String> {
        let waiter = self.get(owner, id)?;
        let outcome = waiter.settle(WaitOutcome::Cancelled);
        self.entries.remove(&id);
        Ok(outcome)
    }
    pub(crate) fn cancel_if_present(
        &mut self,
        owner: &AgentOwner,
        id: WaitId,
    ) -> Result<(), String> {
        if self.entries.contains_key(&id) {
            self.cancel(owner, id)?;
        }
        Ok(())
    }
    pub(crate) fn acknowledge(
        &mut self,
        owner: &AgentOwner,
        id: WaitId,
    ) -> Result<WaitOutcome, String> {
        let waiter = self.get(owner, id)?;
        let outcome = waiter.poll();
        if outcome != WaitOutcome::Pending {
            self.entries.remove(&id);
        }
        Ok(outcome)
    }
    pub(crate) fn retire(&mut self, owner: &AgentOwner) {
        self.entries.retain(|_, entry| {
            if &entry.owner == owner {
                entry.waiter.settle(WaitOutcome::Cancelled);
                false
            } else {
                true
            }
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::worker_wire::AgentId;
    #[test]
    fn notify_timeout_cancel_and_foreign_owner_preserve_one_terminal_result() {
        let owner = AgentOwner::Agent(AgentId(1));
        let foreign = AgentOwner::Agent(AgentId(2));
        let backing = SharedMemoryToken([1; 32]);
        let mut table = WaitTable::default();
        let first = table.enqueue(&owner, backing, 0, None).unwrap();
        let expired = table
            .enqueue(&owner, backing, 0, Some(Duration::ZERO))
            .unwrap();
        let other_offset = table.enqueue(&owner, backing, 4, None).unwrap();
        assert!(table.get(&foreign, first).is_err());
        assert_eq!(table.notify(backing, 0, 10), 1);
        assert_eq!(table.get(&owner, first).unwrap().poll(), WaitOutcome::Ok);
        assert_eq!(table.cancel(&owner, first).unwrap(), WaitOutcome::Ok);
        assert_eq!(
            table.acknowledge(&owner, expired).unwrap(),
            WaitOutcome::TimedOut
        );
        let waiter = table.get(&owner, other_offset).unwrap();
        table.retire(&owner);
        assert_eq!(
            waiter.wait_slice(Duration::from_secs(1)),
            WaitOutcome::Cancelled
        );
        assert!(table.entries.is_empty());
    }
    #[test]
    fn native_wait_blocks_without_table_lock_and_notification_wakes_it() {
        let owner = AgentOwner::Agent(AgentId(1));
        let backing = SharedMemoryToken([2; 32]);
        let mut table = WaitTable::default();
        let id = table.enqueue(&owner, backing, 8, None).unwrap();
        let waiter = table.get(&owner, id).unwrap();
        let thread = std::thread::spawn(move || waiter.wait_slice(Duration::from_millis(50)));
        table.notify(backing, 8, 1);
        let result = thread.join().unwrap();
        assert!(matches!(result, WaitOutcome::Ok | WaitOutcome::Pending));
        assert_eq!(table.acknowledge(&owner, id).unwrap(), WaitOutcome::Ok);
    }
}
