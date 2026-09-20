//! Session registry: bounded admission, per-session serialization and revocation.
use super::{CommandError, session::Session};
use oryn_native::{Browser, ContextOptions};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    ops::{Deref, DerefMut},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

struct Entry {
    session: Arc<tokio::sync::Mutex<Session>>,
    revoked: AtomicBool,
    cancellation: Arc<SessionCancellation>,
    _permit: OwnedSemaphorePermit,
}

/// Revocation notification is independent of the command mutex so a suspended
/// script can release its lease while deletion drains native work.
#[derive(Default)]
pub struct SessionCancellation {
    cancelled: AtomicBool,
    changed: tokio::sync::Notify,
}
impl SessionCancellation {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.changed.notify_waiters();
    }
    pub async fn cancelled(&self) {
        loop {
            let changed = self.changed.notified();
            if self.cancelled.load(Ordering::Acquire) {
                return;
            }
            changed.await;
        }
    }
}

type SessionRetirement = (String, Option<Result<(), CommandError>>);

pub struct Registry {
    entries: Mutex<BTreeMap<String, Arc<Entry>>>,
    capacity: Arc<Semaphore>,
    closing: AtomicBool,
    max_sessions: u32,
    retirements: Mutex<VecDeque<SessionRetirement>>,
}

/// Holding a lease serializes one command against other commands in its session.
/// It does not hold the registry lock, so another session can progress.
pub struct SessionLease {
    guard: OwnedMutexGuard<Session>,
    _entry: Arc<Entry>,
}
impl SessionLease {
    pub fn cancellation(&self) -> Arc<SessionCancellation> {
        self._entry.cancellation.clone()
    }
}
impl Deref for SessionLease {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.guard
    }
}
impl DerefMut for SessionLease {
    fn deref_mut(&mut self) -> &mut Session {
        &mut self.guard
    }
}

fn invalid_session() -> CommandError {
    CommandError {
        code: "invalid session id",
        message: "session is absent or revoked".into(),
    }
}

impl Registry {
    pub fn accepting_sessions(&self) -> bool {
        !self.closing.load(Ordering::Acquire) && self.capacity.available_permits() > 0
    }

    pub fn new(max_sessions: usize) -> Result<Self, CommandError> {
        if !(1..=64).contains(&max_sessions) {
            return Err(CommandError::invalid(
                "session capacity must be between 1 and 64",
            ));
        }
        Ok(Self {
            entries: Mutex::new(BTreeMap::new()),
            capacity: Arc::new(Semaphore::new(max_sessions)),
            closing: AtomicBool::new(false),
            max_sessions: max_sessions as u32,
            retirements: Mutex::new(VecDeque::new()),
        })
    }

    pub async fn create(
        &self,
        browser: &Browser,
        options: ContextOptions,
        capabilities: Map<String, Value>,
    ) -> Result<String, CommandError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(CommandError::session_not_created("driver is shutting down"));
        }
        let permit =
            self.capacity.clone().try_acquire_owned().map_err(|_| {
                CommandError::session_not_created("native session capacity exhausted")
            })?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| {
            CommandError::session_not_created("session identity entropy unavailable")
        })?;
        // UUIDv4 retains 122 random bits and the interoperable session-id shape.
        random[6] = (random[6] & 0x0f) | 0x40;
        random[8] = (random[8] & 0x3f) | 0x80;
        let hex = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let id = format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        );
        let mut session = Session::create(browser, options, capabilities).await?;
        {
            let mut entries = self.entries.lock().expect("WebDriver registry lock");
            if !self.closing.load(Ordering::Acquire) && !entries.contains_key(&id) {
                // Keep ownership local until publication succeeds; no native work
                // runs while holding the registry mutex.
                entries.insert(
                    id.clone(),
                    Arc::new(Entry {
                        session: Arc::new(tokio::sync::Mutex::new(session)),
                        revoked: AtomicBool::new(false),
                        cancellation: Arc::new(SessionCancellation::default()),
                        _permit: permit,
                    }),
                );
                return Ok(id);
            }
        }
        let cleanup = session.delete().await;
        Err(CommandError::session_not_created(match cleanup {
            Ok(()) => "session admission revoked during creation".into(),
            Err(error) => format!(
                "session admission revoked; cleanup failed: {}",
                error.message
            ),
        }))
    }

    pub async fn acquire(&self, id: &str) -> Result<SessionLease, CommandError> {
        let entry = self
            .entries
            .lock()
            .expect("WebDriver registry lock")
            .get(id)
            .cloned()
            .ok_or_else(invalid_session)?;
        if entry.revoked.load(Ordering::Acquire) {
            return Err(invalid_session());
        }
        let guard = entry.session.clone().lock_owned().await;
        // Requests already waiting when delete begins must not run afterwards.
        if entry.revoked.load(Ordering::Acquire) {
            return Err(invalid_session());
        }
        Ok(SessionLease {
            guard,
            _entry: entry,
        })
    }

    fn begin_retirement(&self, id: &str) {
        let mut records = self.retirements.lock().expect("session retirement lock");
        records.push_back((id.into(), None));
        while records.len() > 128 {
            // Active admission is at most 64; incomplete retirements retain their
            // permits and therefore cannot consume this entire history bound.
            let index = records
                .iter()
                .position(|(_, result)| result.is_some())
                .expect("bounded pending retirements");
            records.remove(index);
        }
    }
    fn complete_retirement(&self, id: &str, result: &Result<(), CommandError>) {
        let mut records = self.retirements.lock().expect("session retirement lock");
        if let Some((_, state)) = records.iter_mut().find(|(candidate, _)| candidate == id) {
            *state = Some(result.clone());
        }
    }
    pub fn retirement(&self, id: &str) -> Result<Value, CommandError> {
        let records = self.retirements.lock().expect("session retirement lock");
        let (_, state) = records
            .iter()
            .find(|(candidate, _)| candidate == id)
            .ok_or_else(invalid_session)?;
        Ok(match state {
            None => serde_json::json!({"schema_version":1,"session_id":id,"status":"pending"}),
            Some(Ok(())) => {
                serde_json::json!({"schema_version":1,"session_id":id,"status":"complete"})
            }
            Some(Err(error)) => {
                serde_json::json!({"schema_version":1,"session_id":id,"status":"failed","error":error.code})
            }
        })
    }

    pub async fn delete(&self, id: &str) -> Result<(), CommandError> {
        let entry = {
            let mut entries = self.entries.lock().expect("WebDriver registry lock");
            let entry = entries.remove(id).ok_or_else(invalid_session)?;
            self.begin_retirement(id);
            entry.revoked.store(true, Ordering::Release);
            entry.cancellation.cancel();
            entry
        };
        let result = entry.session.lock().await.delete().await;
        self.complete_retirement(id, &result);
        result
    }

    pub async fn shutdown(&self) -> Result<(), CommandError> {
        let entries = {
            let mut entries = self.entries.lock().expect("WebDriver registry lock");
            self.closing.store(true, Ordering::Release);
            let entries = std::mem::take(&mut *entries);
            for entry in entries.values() {
                entry.revoked.store(true, Ordering::Release);
                entry.cancellation.cancel();
            }
            entries
        };
        let mut failures = Vec::new();
        for (id, entry) in entries {
            self.begin_retirement(&id);
            let result = entry.session.lock().await.delete().await;
            self.complete_retirement(&id, &result);
            if let Err(error) = result {
                failures.push(error.message);
            }
        }
        // Creations admitted before closing may still be awaiting their initial
        // page. Their failed publication path must finish cleanup and release its
        // permit before shutdown can acknowledge completion.
        let _drained = self
            .capacity
            .clone()
            .acquire_many_owned(self.max_sessions)
            .await
            .map_err(|_| CommandError {
                code: "unknown error",
                message: "session drain unavailable".into(),
            })?;
        if failures.is_empty() {
            Ok(())
        } else {
            Err(CommandError {
                code: "unknown error",
                message: format!("session cleanup failed: {}", failures.join("; ")),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn retirement_receipt_requires_completed_native_cleanup_and_retains_failure() {
        let registry = Registry::new(1).unwrap();
        let id = registry
            .create(&Browser::new(), ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        assert!(registry.retirement(&id).is_err());
        registry.delete(&id).await.unwrap();
        assert_eq!(
            registry.retirement(&id).unwrap(),
            serde_json::json!({"schema_version":1,"session_id":id,"status":"complete"})
        );
        registry.begin_retirement("failed");
        assert_eq!(registry.retirement("failed").unwrap()["status"], "pending");
        registry.complete_retirement(
            "failed",
            &Err(CommandError {
                code: "unknown error",
                message: "actual cleanup error fixture".into(),
            }),
        );
        assert_eq!(registry.retirement("failed").unwrap()["status"], "failed");
        assert!(registry.retirement("foreign").is_err());
        for index in 0..150 {
            let key = format!("old-{index}");
            registry.begin_retirement(&key);
            registry.complete_retirement(&key, &Ok(()));
        }
        assert!(registry.retirement(&id).is_err());
        assert!(registry.retirements.lock().unwrap().len() <= 128);
    }

    #[tokio::test]
    async fn registry_bounds_sessions_and_revokes_deleted_identifiers() {
        let registry = Registry::new(1).unwrap();
        let browser = Browser::new();
        let id = registry
            .create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(
            id.split('-').map(str::len).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12]
        );
        assert_eq!(id.as_bytes()[14], b'4');
        assert!(matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
        assert!(
            id.bytes()
                .all(|byte| byte == b'-' || byte.is_ascii_hexdigit())
        );
        assert_eq!(
            registry
                .create(&browser, ContextOptions::loopback_test(), Map::new())
                .await
                .unwrap_err()
                .code,
            "session not created"
        );
        assert!(registry.acquire(&id).await.is_ok());
        registry.delete(&id).await.unwrap();
        assert_eq!(
            registry.acquire(&id).await.err().unwrap().code,
            "invalid session id"
        );
        let second = registry
            .create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        assert_ne!(id, second);
        registry.shutdown().await.unwrap();
        assert!(registry.acquire(&second).await.is_err());
        assert!(
            registry
                .create(&browser, ContextOptions::loopback_test(), Map::new())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn deletion_rejects_a_command_queued_before_revocation() {
        use std::{
            future::{Future, poll_fn},
            task::Poll,
        };
        let registry = Registry::new(1).unwrap();
        let browser = Browser::new();
        let id = registry
            .create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        let held = registry.acquire(&id).await.unwrap();
        let mut queued = Box::pin(registry.acquire(&id));
        assert!(
            poll_fn(|cx| Poll::Ready(queued.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        let mut deletion = Box::pin(registry.delete(&id));
        assert!(
            poll_fn(|cx| Poll::Ready(deletion.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(!registry.entries.lock().unwrap().contains_key(&id));
        drop(held);
        // Tokio's mutex gives the already queued request its turn first. It
        // must observe revocation and release the lock without running a command.
        assert_eq!(queued.await.err().unwrap().code, "invalid session id");
        deletion.await.unwrap();
        registry.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn held_session_lease_does_not_lock_another_session() {
        let registry = Registry::new(2).unwrap();
        let browser = Browser::new();
        let first = registry
            .create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        let second = registry
            .create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        let a = registry.acquire(&first).await.unwrap();
        let b = registry.acquire(&second).await.unwrap();
        assert!(a.current_page().is_ok());
        assert!(b.current_page().is_ok());
        let first_entry = registry.entries.lock().unwrap()[&first].clone();
        assert!(first_entry.session.try_lock().is_err());
        drop(a);
        drop(b);
        drop(first_entry);
        registry.shutdown().await.unwrap();
    }
}
