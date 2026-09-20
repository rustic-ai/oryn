//! Response authority is owned metadata, not a bare string. Copies share one
//! cleanup action; dropping an unread queued response releases its native grant.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::{Arc, Mutex};

type Cleanup = Box<dyn FnOnce() + Send + Sync>;

enum State {
    Detached,
    Armed(Cleanup),
    Committed,
}

struct Inner {
    token: String,
    state: Mutex<State>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if let State::Armed(cleanup) = std::mem::replace(state, State::Committed) {
            cleanup();
        }
    }
}

#[derive(Clone)]
pub struct ResponsePolicyToken(Arc<Inner>);

impl ResponsePolicyToken {
    pub fn as_str(&self) -> &str {
        &self.0.token
    }

    /// IPC decode installs cleanup before publishing this value into a queue.
    /// Deserialization alone cannot create native cancellation authority.
    pub(super) fn attach_cleanup(&self, cleanup: impl FnOnce() + Send + Sync + 'static) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert!(
            matches!(*state, State::Detached),
            "response cleanup is attached once"
        );
        *state = State::Armed(Box::new(cleanup));
    }

    /// Called only after the native commit acknowledgement. All retained copies
    /// observe this transition, so their later drops cannot cancel the document.
    #[cfg(any(all(unix, feature = "v8-host"), test))]
    pub(crate) fn committed(&self) {
        *self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = State::Committed;
    }
}

impl From<String> for ResponsePolicyToken {
    fn from(token: String) -> Self {
        Self(Arc::new(Inner {
            token,
            state: Mutex::new(State::Detached),
        }))
    }
}
impl From<&str> for ResponsePolicyToken {
    fn from(token: &str) -> Self {
        token.to_owned().into()
    }
}
impl std::ops::Deref for ResponsePolicyToken {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl std::fmt::Display for ResponsePolicyToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}
impl std::fmt::Debug for ResponsePolicyToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ResponsePolicyToken([redacted])")
    }
}
impl PartialEq for ResponsePolicyToken {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}
impl Eq for ResponsePolicyToken {}
impl Serialize for ResponsePolicyToken {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for ResponsePolicyToken {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn owned(value: &str, count: &Arc<AtomicUsize>) -> ResponsePolicyToken {
        let token = ResponsePolicyToken::from(value);
        let count = count.clone();
        token.attach_cleanup(move || {
            count.fetch_add(1, Ordering::SeqCst);
        });
        token
    }

    #[test]
    fn only_the_last_copy_releases_an_abandoned_grant() {
        let count = Arc::new(AtomicUsize::new(0));
        let token = owned("original", &count);
        let copy = token.clone();
        drop(token);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        drop(copy);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn acknowledged_commit_disarms_every_retained_copy() {
        let count = Arc::new(AtomicUsize::new(0));
        let token = owned("committed", &count);
        let copy = token.clone();
        copy.committed();
        drop(copy);
        drop(token);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn dropping_an_unread_response_queue_releases_its_grant() {
        let count = Arc::new(AtomicUsize::new(0));
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        sender
            .try_send(super::super::NetworkEvent::Headers {
                response_policy: Some(owned("queued", &count)),
                timing: None,
                request_id: 1,
                status: 200,
                headers: Vec::new(),
                content_length: Some(0),
                upload: None,
            })
            .unwrap();
        drop(receiver);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn body_completion_does_not_release_the_retained_header_grant() {
        let count = Arc::new(AtomicUsize::new(0));
        let token = owned("headers", &count);
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        sender
            .try_send(super::super::NetworkEvent::Complete {
                request_id: 1,
                encoded_body_size: Some(0),
            })
            .unwrap();
        receiver.try_recv().unwrap();
        drop(receiver);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        token.committed();
        drop(token);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn wire_encoding_contains_only_the_opaque_token() {
        let count = Arc::new(AtomicUsize::new(0));
        let token = owned("wire-value", &count);
        let wire = serde_json::to_string(&token).unwrap();
        assert_eq!(wire, "\"wire-value\"");
        let decoded: ResponsePolicyToken = serde_json::from_str(&wire).unwrap();
        assert_eq!(decoded, token);
        drop(decoded);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert!(!format!("{token:?}").contains("wire-value"));
        drop(token);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn late_predecessor_drop_cannot_release_a_successor() {
        let old_count = Arc::new(AtomicUsize::new(0));
        let new_count = Arc::new(AtomicUsize::new(0));
        let old = owned("predecessor", &old_count);
        let new = owned("successor", &new_count);
        drop(old);
        assert_eq!(old_count.load(Ordering::SeqCst), 1);
        assert_eq!(new_count.load(Ordering::SeqCst), 0);
        new.committed();
        drop(new);
        assert_eq!(new_count.load(Ordering::SeqCst), 0);
    }
}
