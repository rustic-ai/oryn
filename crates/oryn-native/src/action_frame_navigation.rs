//! Action-local observation of embedded navigation completion. A single bounded
//! counter joins existing loader observers and at most 1024 native history
//! receipts; it owns no request and never retries.
use super::{AutomationRealmSnapshot, HostError};
use oryn_js::navigation::{NavigationExecutionObserver, NavigationExecutionOutcome};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    sealed: bool,
    selected_source: Option<AutomationRealmSnapshot>,
    selected_handoff: bool,
    selected_url: Option<String>,
    selected_observer: Option<oryn_js::navigation::NavigationExecutionObserverIdentity>,
    selected_commit: Option<CommittedSelection>,
    admitted: bool,
    pending: usize,
    joint: Vec<oryn_js::joint_history::NativeJointTraversalReceipt>,
    failure: Option<NavigationExecutionOutcome>,
}
#[derive(Clone, Default)]
pub(super) struct Capture(Arc<Mutex<State>>);
pub(crate) struct Receipt(Capture);
#[derive(Clone)]
pub(crate) struct CommittedSelection {
    pub(crate) source: AutomationRealmSnapshot,
    pub(crate) destination: AutomationRealmSnapshot,
}
impl Capture {
    pub(super) fn for_selected(selected_source: Option<AutomationRealmSnapshot>) -> Self {
        Self(Arc::new(Mutex::new(State {
            selected_source,
            ..Default::default()
        })))
    }
    /// Only a Location handoff from the exact action source may defer that
    /// source's replacement. This is scheduling custody, never input authority.
    pub(super) fn note_selected_handoff(
        &self,
        source: &AutomationRealmSnapshot,
        url: &str,
        observer: &NavigationExecutionObserver,
    ) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if !state.sealed
            && state
                .selected_source
                .as_ref()
                .is_some_and(|selected| selected.same_document_as(source))
        {
            state.selected_handoff = true;
            state.selected_url = Some(url.to_owned());
            state.selected_observer = Some(observer.identity());
        }
    }
    /// Called only at successful loader/lifecycle settlement, before its exact
    /// observer publishes completion. Preserve the first installed successor.
    pub(super) fn record_selected_commit(
        &self,
        observer: &NavigationExecutionObserver,
        destination: AutomationRealmSnapshot,
    ) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if !state.selected_handoff
            || state.failure.is_some()
            || state.selected_commit.is_some()
            || !state
                .selected_observer
                .as_ref()
                .is_some_and(|id| id.matches(observer))
        {
            return;
        }
        let Some(source) = state
            .selected_source
            .as_ref()
            .filter(|source| {
                source.same_navigable_as(&destination) && !source.same_document_as(&destination)
            })
            .cloned()
        else {
            return;
        };
        state.selected_commit = Some(CommittedSelection {
            source,
            destination,
        });
    }
    pub(super) fn defers_selected_handoff(&self, source: &AutomationRealmSnapshot) -> bool {
        let state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        !state.sealed
            && state.selected_handoff
            && state
                .selected_source
                .as_ref()
                .is_some_and(|selected| selected.same_document_as(source))
    }
    pub(super) fn open(&self) -> bool {
        !self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .sealed
    }
    pub(super) fn receipt(&self) -> Receipt {
        Receipt(self.clone())
    }
    pub(super) fn observe_joint(
        &self,
        receipt: oryn_js::joint_history::NativeJointTraversalReceipt,
    ) -> Result<(), HostError> {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state.sealed {
            return Ok(());
        }
        if state.joint.len() >= 1024 {
            return Err(HostError::LimitExceeded {
                resource: "action joint traversal receipts".into(),
            });
        }
        state
            .joint
            .try_reserve(1)
            .map_err(|_| HostError::LimitExceeded {
                resource: "action joint traversal receipt allocation".into(),
            })?;
        state.admitted = true;
        state.joint.push(receipt);
        Ok(())
    }
    pub(super) fn observe(
        &self,
        previous: Option<NavigationExecutionObserver>,
    ) -> Result<Option<NavigationExecutionObserver>, HostError> {
        {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            if state.sealed {
                return Ok(previous);
            }
            state.pending =
                state
                    .pending
                    .checked_add(1)
                    .ok_or_else(|| HostError::LimitExceeded {
                        resource: "action frame navigation counter".into(),
                    })?;
            state.admitted = true;
        }
        let capture = self.clone();
        let admission = previous.clone();
        let observer = NavigationExecutionObserver::with_admission(
            move || {
                if let Some(previous) = &admission {
                    previous.admitted();
                }
            },
            move |outcome| {
                if let Some(previous) = &previous {
                    previous.complete(outcome.clone());
                }
                let mut state = capture.0.lock().unwrap_or_else(|error| error.into_inner());
                state.pending = state.pending.saturating_sub(1);
                if state.failure.is_none()
                    && matches!(
                        outcome,
                        NavigationExecutionOutcome::Canceled
                            | NavigationExecutionOutcome::Failed { .. }
                    )
                {
                    state.failure = Some(outcome);
                }
            },
        );
        // This attachment observes an already admitted loader handoff. Preserve
        // the old observer's idempotent admission and loader-owned terminal rules.
        observer.admitted();
        observer.loader_owned();
        Ok(Some(observer))
    }
}
impl Receipt {
    pub(crate) fn selected_navigation_url(&self) -> Option<String> {
        self.0
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .selected_url
            .clone()
    }
    pub(crate) fn committed_selection(&self) -> Option<CommittedSelection> {
        let state = self.0.0.lock().unwrap_or_else(|error| error.into_inner());
        (state.sealed && state.pending == 0 && state.joint.is_empty() && state.failure.is_none())
            .then(|| state.selected_commit.clone())
            .flatten()
    }
    pub(crate) fn finish(self) -> Option<Self> {
        let admitted = {
            let mut state = self.0.0.lock().unwrap_or_else(|error| error.into_inner());
            state.sealed = true;
            state.admitted
        };
        admitted.then_some(self)
    }
    pub(crate) fn poll(&self) -> Option<NavigationExecutionOutcome> {
        let mut state = self.0.0.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(failure) = &state.failure {
            return Some(failure.clone());
        }
        let mut joint_failure = None;
        state.joint.retain(|receipt| match receipt.poll() {
            None => true,
            Some(Ok(())) => false,
            Some(Err(error)) => {
                if joint_failure.is_none() {
                    joint_failure = Some(match error {
                        oryn_js::joint_history::JointTraversalFailure::Canceled
                        | oryn_js::joint_history::JointTraversalFailure::Retired => {
                            NavigationExecutionOutcome::Canceled
                        }
                        oryn_js::joint_history::JointTraversalFailure::Failed(message) => {
                            NavigationExecutionOutcome::Failed { message }
                        }
                    });
                }
                false
            }
        });
        if let Some(failure) = joint_failure {
            state.failure = Some(failure.clone());
            return Some(failure);
        }
        (state.sealed && state.pending == 0 && state.joint.is_empty())
            .then_some(NavigationExecutionOutcome::Completed)
    }
}
impl Drop for Receipt {
    fn drop(&mut self) {
        self.0
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .sealed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_capture_joins_terminal_loader_and_preserves_prior_observer_once() {
        let capture = Capture::default();
        let receipt = capture.receipt();
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let saved = outcomes.clone();
        let prior =
            NavigationExecutionObserver::new(move |outcome| saved.lock().unwrap().push(outcome));
        let first = capture.observe(Some(prior)).unwrap().unwrap();
        let second = capture.observe(None).unwrap().unwrap();
        let receipt = receipt.finish().unwrap();
        assert!(receipt.poll().is_none());
        first.complete(NavigationExecutionOutcome::Completed);
        assert!(receipt.poll().is_none());
        second.complete(NavigationExecutionOutcome::Committed {
            document_generation: 17,
        });
        assert_eq!(receipt.poll(), Some(NavigationExecutionOutcome::Completed));
        first.complete(NavigationExecutionOutcome::Canceled);
        assert_eq!(
            *outcomes.lock().unwrap(),
            [NavigationExecutionOutcome::Completed]
        );
    }
    #[test]
    fn embedded_capture_reports_dropped_loader_owner_instead_of_waiting_forever() {
        let capture = Capture::default();
        let receipt = capture.receipt();
        let observer = capture.observe(None).unwrap().unwrap();
        let receipt = receipt.finish().unwrap();
        drop(observer);
        assert!(
            matches!(receipt.poll(), Some(NavigationExecutionOutcome::Failed { message })
            if message.contains("owner dropped before terminal settlement"))
        );
    }
    #[test]
    fn sealed_capture_does_not_adopt_later_background_navigation() {
        let capture = Capture::default();
        assert!(capture.receipt().finish().is_none());
        assert!(!capture.open());
        assert!(capture.observe(None).unwrap().is_none());
    }
    #[test]
    fn embedded_capture_preserves_cancellation_while_other_loader_is_still_owned() {
        let capture = Capture::default();
        let receipt = capture.receipt();
        let first = capture.observe(None).unwrap().unwrap();
        let second = capture.observe(None).unwrap().unwrap();
        let receipt = receipt.finish().unwrap();
        first.complete(NavigationExecutionOutcome::Canceled);
        assert_eq!(receipt.poll(), Some(NavigationExecutionOutcome::Canceled));
        second.complete(NavigationExecutionOutcome::Completed);
        assert_eq!(receipt.poll(), Some(NavigationExecutionOutcome::Canceled));
    }
    fn selected_source() -> AutomationRealmSnapshot {
        AutomationRealmSnapshot {
            realm: oryn_js::realm::RealmId(2),
            parent_realm: Some(oryn_js::realm::RealmId(1)),
            frame: Some(oryn_dom::NodeId {
                slot: 4,
                generation: 0,
            }),
            frame_generation: Some(1),
            frame_token: Some("owned-frame".into()),
            document_root: oryn_dom::NodeId {
                slot: 8,
                generation: 0,
            },
            document_generation: 3,
            stream_epoch: 1,
            document_authority_epoch: 1,
            url: "https://selected-handoff.test/source".into(),
            live: true,
        }
    }

    #[test]
    fn selected_action_handoff_guard_requires_exact_source_and_releases_when_receipt_seals() {
        let source = selected_source();
        let capture = Capture::for_selected(Some(source.clone()));
        let receipt = capture.receipt();
        assert!(!capture.defers_selected_handoff(&source));
        let observer = capture.observe(None).unwrap().unwrap();
        let mut sibling = source.clone();
        sibling.realm = oryn_js::realm::RealmId(3);
        sibling.frame_token = Some("sibling-frame".into());
        capture.note_selected_handoff(&sibling, "https://selected-handoff.test/next", &observer);
        assert!(!capture.defers_selected_handoff(&source));
        let mut reopened = source.clone();
        reopened.stream_epoch += 1;
        capture.note_selected_handoff(&reopened, "https://selected-handoff.test/next", &observer);
        assert!(!capture.defers_selected_handoff(&source));
        capture.note_selected_handoff(&source, "https://selected-handoff.test/next", &observer);
        assert!(capture.defers_selected_handoff(&source));
        assert!(!capture.defers_selected_handoff(&reopened));
        let receipt = receipt.finish().unwrap();
        assert!(!capture.defers_selected_handoff(&source));
        assert!(
            receipt.poll().is_none(),
            "sealing releases scheduling, not loader custody"
        );
        observer.complete(NavigationExecutionOutcome::Committed {
            document_generation: 4,
        });
        assert_eq!(receipt.poll(), Some(NavigationExecutionOutcome::Completed));
        capture.note_selected_handoff(&source, "https://selected-handoff.test/next", &observer);
        assert!(!capture.defers_selected_handoff(&source));
    }

    #[test]
    fn oil_selected_navigation_capture_requires_exact_observer_and_completed_commit() {
        let source = selected_source();
        let capture = Capture::for_selected(Some(source.clone()));
        let receipt = capture.receipt();
        let observer = capture.observe(None).unwrap().unwrap();
        let wrong = NavigationExecutionObserver::new(|_| {});
        capture.note_selected_handoff(&source, "https://selected-handoff.test/next", &observer);
        let mut destination = source.clone();
        destination.document_generation += 1;
        destination.document_root.slot += 1;
        destination.url = "https://selected-handoff.test/next".into();
        capture.record_selected_commit(&wrong, destination.clone());
        assert!(capture.0.lock().unwrap().selected_commit.is_none());
        let mut disconnected = destination.clone();
        disconnected.frame_token = Some("replacement connection".into());
        capture.record_selected_commit(&observer, disconnected);
        assert!(capture.0.lock().unwrap().selected_commit.is_none());
        capture.record_selected_commit(&observer, destination.clone());
        let receipt = receipt.finish().unwrap();
        assert!(receipt.committed_selection().is_none());
        observer.complete(NavigationExecutionOutcome::Committed {
            document_generation: destination.document_generation,
        });
        assert_eq!(receipt.poll(), Some(NavigationExecutionOutcome::Completed));
        let committed = receipt.committed_selection().unwrap();
        assert!(committed.source.same_document_as(&source));
        assert!(committed.destination.same_document_as(&destination));
        let mut later = destination.clone();
        later.stream_epoch += 1;
        capture.record_selected_commit(&observer, later);
        assert!(
            receipt
                .committed_selection()
                .unwrap()
                .destination
                .same_document_as(&destination)
        );
    }

    #[test]
    fn oil_selected_navigation_capture_never_promotes_canceled_or_failed_loader() {
        for outcome in [
            NavigationExecutionOutcome::Canceled,
            NavigationExecutionOutcome::Failed {
                message: "original transport failure".into(),
            },
        ] {
            let source = selected_source();
            let capture = Capture::for_selected(Some(source.clone()));
            let receipt = capture.receipt();
            let observer = capture.observe(None).unwrap().unwrap();
            capture.note_selected_handoff(&source, "https://selected-handoff.test/next", &observer);
            let mut destination = source.clone();
            destination.document_generation += 1;
            capture.record_selected_commit(&observer, destination);
            let receipt = receipt.finish().unwrap();
            observer.complete(outcome.clone());
            assert_eq!(receipt.poll(), Some(outcome));
            assert!(receipt.committed_selection().is_none());
        }
    }
}
