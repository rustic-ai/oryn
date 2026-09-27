//! Failure provenance retained before crossing script loader boundaries.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum ScriptLoadFailure {
    #[error("script fetch failed: {0}")]
    Fetch(String),
    #[error("script loading infrastructure failed: {0}")]
    Infrastructure(String),
    #[error("script load cancelled")]
    Cancelled,
    #[error("script load exceeded {0}")]
    Capacity(String),
    #[error("script integrity failed: {0}")]
    Integrity(String),
}

impl ScriptLoadFailure {
    pub fn is_fetch_failure(&self) -> bool {
        matches!(self, Self::Fetch(_))
    }

    #[cfg(any(test, feature = "v8-host"))]
    pub(crate) fn worker(error: oryn_js::worker_wire::WorkerError) -> Self {
        use oryn_js::worker_wire::ScriptFailureKind;
        match error.script_failure {
            Some(ScriptFailureKind::Fetch) => Self::Fetch(error.to_string()),
            Some(ScriptFailureKind::Integrity) => Self::Integrity(error.to_string()),
            Some(ScriptFailureKind::Cancelled) => Self::Cancelled,
            Some(ScriptFailureKind::Capacity) => Self::Capacity(error.message),
            Some(ScriptFailureKind::Infrastructure) | None => {
                Self::Infrastructure(error.to_string())
            }
        }
    }
    pub(crate) fn into_worker(self) -> oryn_js::worker_wire::WorkerError {
        use oryn_js::worker_wire::{ScriptFailureKind, WorkerError};
        let (name, kind) = match &self {
            Self::Fetch(_) => ("NetworkError", ScriptFailureKind::Fetch),
            Self::Integrity(_) => ("NetworkError", ScriptFailureKind::Integrity),
            Self::Infrastructure(_) => ("InvalidStateError", ScriptFailureKind::Infrastructure),
            Self::Cancelled => ("AbortError", ScriptFailureKind::Cancelled),
            Self::Capacity(_) => ("QuotaExceededError", ScriptFailureKind::Capacity),
        };
        WorkerError::new(name, self.to_string()).with_script_failure(kind)
    }

    pub(crate) fn runtime(self) -> crate::RuntimeError {
        match self {
            Self::Capacity(resource) => crate::RuntimeError::WorkerLimitExceeded { resource },
            failure => crate::RuntimeError::ContextService(failure.to_string()),
        }
    }

    pub(crate) fn network(error: crate::network::NetworkError) -> Self {
        use crate::network::NetworkError;
        match error {
            NetworkError::Cancelled => Self::Cancelled,
            NetworkError::ResponseTooLarge { limit } => {
                Self::Capacity(format!("{limit}-byte response limit"))
            }
            error @ (NetworkError::Unavailable
            | NetworkError::Policy(_)
            | NetworkError::Replay(_)) => Self::Infrastructure(error.to_string()),
            error => Self::Fetch(error.to_string()),
        }
    }

    pub(crate) fn decoded(error: crate::RuntimeError) -> Self {
        match error {
            crate::RuntimeError::Network(message) => Self::Fetch(message),
            error @ crate::RuntimeError::CertificateRejected(_) => Self::Fetch(error.to_string()),
            crate::RuntimeError::WorkerLimitExceeded { resource } => Self::Capacity(resource),
            error => error.into(),
        }
    }
}

impl From<crate::RuntimeError> for ScriptLoadFailure {
    fn from(error: crate::RuntimeError) -> Self {
        Self::Infrastructure(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_wire_provenance_survives_serialization_without_exception_name_inference() {
        use oryn_js::worker_wire::{ScriptFailureKind, WorkerError};
        let unclassified = WorkerError::new("NetworkError", "HTTP 404");
        assert!(!ScriptLoadFailure::worker(unclassified).is_fetch_failure());
        for kind in [
            ScriptFailureKind::Fetch,
            ScriptFailureKind::Infrastructure,
            ScriptFailureKind::Cancelled,
            ScriptFailureKind::Capacity,
        ] {
            let error =
                WorkerError::new("NetworkError", "identical text").with_script_failure(kind);
            let wire = serde_json::to_vec(&error).unwrap();
            let decoded = serde_json::from_slice(&wire).unwrap();
            assert_eq!(
                ScriptLoadFailure::worker(decoded).is_fetch_failure(),
                kind == ScriptFailureKind::Fetch
            );
        }
        let old: WorkerError =
            serde_json::from_str(r#"{"name":"NetworkError","message":"legacy"}"#).unwrap();
        assert!(old.script_failure.is_none());
    }

    #[test]
    fn network_failure_classification_uses_variants_not_message_text() {
        use crate::network::NetworkError;
        for error in [
            NetworkError::Cancelled,
            NetworkError::Unavailable,
            NetworkError::ResponseTooLarge { limit: 8 },
            NetworkError::Replay("HTTP 404".into()),
        ] {
            assert!(!ScriptLoadFailure::network(error).is_fetch_failure());
        }
        assert!(
            ScriptLoadFailure::network(NetworkError::Transport(
                "cancelled capacity unavailable".into()
            ))
            .is_fetch_failure()
        );
        for failure in [
            ScriptLoadFailure::Fetch("HTTP 404".into()),
            ScriptLoadFailure::Cancelled,
            ScriptLoadFailure::Capacity("source bytes".into()),
            ScriptLoadFailure::Infrastructure("admission".into()),
        ] {
            let encoded = serde_json::to_vec(&failure).unwrap();
            assert_eq!(
                serde_json::from_slice::<ScriptLoadFailure>(&encoded).unwrap(),
                failure
            );
        }
    }
}

#[cfg(test)]
mod certificate_script_load_tests {
    use super::*;
    #[test]
    fn certificate_script_load_rejection_remains_fetch_failure() {
        let failure = ScriptLoadFailure::decoded(crate::RuntimeError::CertificateRejected(
            "UnknownIssuer".into(),
        ));
        assert!(failure.is_fetch_failure());
        assert!(matches!(failure, ScriptLoadFailure::Fetch(_)));
        let other = ScriptLoadFailure::decoded(crate::RuntimeError::ContextService(
            "certificate rejected".into(),
        ));
        assert!(!other.is_fetch_failure());
    }
}
