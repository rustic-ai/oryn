//! Exact-route ownership for resource jobs queued outside the page actor.
use crate::worker_route::WorkerRoute;
use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
struct State {
    sealed: bool,
    active: usize,
    network_active: usize,
    last_activity: Option<Instant>,
}
#[derive(Default)]
struct Scope {
    state: Mutex<State>,
    changed: Condvar,
    timings: Mutex<super::resource_timing_scope::Queue>,
}
#[derive(Default)]
pub(super) struct Registry {
    routes: Arc<Mutex<BTreeMap<Option<WorkerRoute>, Arc<Scope>>>>,
    document: Option<Arc<Registry>>,
}
pub(crate) struct ResourceJob(
    Arc<Scope>,
    bool,
    #[cfg(any(feature = "v8-host", test))] Option<Box<ResourceJob>>,
);

impl Registry {
    pub(super) fn finish_timing(
        &self,
        route: Option<&WorkerRoute>,
        observation: super::resource_timing_scope::Observation,
        result: &Result<super::ContextServiceResponse, super::RuntimeError>,
    ) {
        if let Some(document) = &self.document {
            return document.finish_timing(route, observation, result);
        }
        if let Ok(scope) = self.scope(route) {
            scope
                .timings
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .finish(observation, result);
        }
    }
    #[cfg(any(feature = "v8-host", test))]
    pub(super) fn drain_timings(
        &self,
        route: Option<&WorkerRoute>,
    ) -> Vec<super::resource_timing_scope::Record> {
        if let Some(document) = &self.document {
            return document.drain_timings(route);
        }
        let entries = self.routes.lock().unwrap_or_else(|e| e.into_inner());
        let Some(scope) = entries.get(&route.cloned()) else {
            return Vec::new();
        };

        scope
            .timings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
    }

    /// Keep route-wide teardown custody while exposing only this document's
    /// activity to its wait predicate. Clones of the returned view share it.
    #[cfg(any(feature = "v8-host", test))]
    pub(super) fn for_document(&self) -> Self {
        Self {
            routes: self.routes.clone(),
            document: Some(Arc::new(Self::default())),
        }
    }
    /// Extend the activity ancestry without including a sibling document.
    /// A child job holds custody in every ancestor and in the transport route.
    #[cfg(any(feature = "v8-host", test))]
    pub(super) fn for_child_document(&self) -> Self {
        Self {
            routes: self.routes.clone(),
            document: Some(Arc::new(match &self.document {
                Some(document) => document.for_child_document(),
                None => Self::default(),
            })),
        }
    }
    fn scope(&self, route: Option<&WorkerRoute>) -> Result<Arc<Scope>, String> {
        let mut entries = self.routes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(scope) = entries.get(&route.cloned()) {
            return Ok(scope.clone());
        }
        if entries.len() >= 4096 {
            return Err("resource route lifetime capacity exceeded".into());
        }
        let scope = Arc::new(Scope::default());
        entries.insert(route.cloned(), scope.clone());
        Ok(scope)
    }

    /// Inspect without allocating a scope or changing its admission state.
    #[cfg(any(feature = "v8-host", test))]
    pub(super) fn activity(&self, route: Option<&WorkerRoute>) -> (usize, Option<Instant>) {
        if let Some(document) = &self.document {
            return document.activity(route);
        }
        let entries = self.routes.lock().unwrap_or_else(|e| e.into_inner());
        let Some(scope) = entries.get(&route.cloned()) else {
            return (0, None);
        };
        let state = scope.state.lock().unwrap_or_else(|e| e.into_inner());
        (state.network_active, state.last_activity)
    }

    #[cfg(any(feature = "v8-host", test))]
    pub(super) fn acquire(&self, route: Option<&WorkerRoute>) -> Result<ResourceJob, String> {
        self.acquire_kind(route, true)
    }

    #[cfg(any(feature = "v8-host", test))]
    pub(super) fn acquire_control(
        &self,
        route: Option<&WorkerRoute>,
    ) -> Result<ResourceJob, String> {
        self.acquire_kind(route, false)
    }

    #[cfg(any(feature = "v8-host", test))]
    fn acquire_kind(
        &self,
        route: Option<&WorkerRoute>,
        network: bool,
    ) -> Result<ResourceJob, String> {
        let scope = self.scope(route)?;
        {
            let mut state = scope.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.sealed {
                return Err("resource route retired".into());
            }
            if state.active >= 1024 {
                return Err("resource route job capacity exceeded".into());
            }
            state.active += 1;
            if network {
                state.network_active += 1;
                state.last_activity = Some(Instant::now());
            }
        }
        let mut job = ResourceJob(scope, network, None);
        if let Some(document) = &self.document {
            job.2 = Some(Box::new(document.acquire_kind(route, network)?));
        }
        Ok(job)
    }

    pub(super) fn retire_and_drain(
        &self,
        route: Option<&WorkerRoute>,
        budget: Duration,
    ) -> Result<(), String> {
        let scope = self.scope(route)?;
        let deadline = Instant::now()
            .checked_add(budget)
            .unwrap_or_else(Instant::now);
        let mut state = scope.state.lock().unwrap_or_else(|e| e.into_inner());
        state.sealed = true;
        while state.active != 0 {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err("resource route jobs remain pending".into());
            };
            state = scope
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        Ok(())
    }
}
impl Drop for ResourceJob {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.active -= 1;
        if self.1 {
            state.network_active -= 1;
            state.last_activity = Some(Instant::now());
        }
        self.0.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descendant_activity_counts_ancestors_without_counting_siblings() {
        let registry = Registry::default();
        let root = registry.for_document();
        let child = root.for_child_document();
        let sibling = root.for_child_document();
        let grandchild = child.for_child_document();
        let replacement = root.for_child_document();
        let job = grandchild.acquire(None).unwrap();
        for ancestor in [&registry, &root, &child, &grandchild] {
            assert_eq!(ancestor.activity(None).0, 1);
        }
        assert_eq!(sibling.activity(None), (0, None));
        assert_eq!(replacement.activity(None), (0, None));
        let root_job = root.acquire(None).unwrap();
        assert_eq!(root.activity(None).0, 2);
        assert_eq!(child.activity(None).0, 1);
        let control = sibling.acquire_control(None).unwrap();
        assert_eq!(sibling.activity(None), (0, None));
        drop(root_job);
        let completed = Instant::now();
        drop(job);
        for ancestor in [&registry, &root, &child, &grandchild] {
            let (active, changed) = ancestor.activity(None);
            assert_eq!(active, 0);
            assert!(changed.is_some_and(|at| at >= completed));
        }
        assert!(registry.retire_and_drain(None, Duration::ZERO).is_err());
        assert!(replacement.acquire(None).is_err());
        assert_eq!(replacement.activity(None), (0, None));
        drop(control);
        registry.retire_and_drain(None, Duration::ZERO).unwrap();
    }

    #[test]
    fn document_activity_is_separate_but_route_teardown_retains_all_documents() {
        let registry = Registry::default();
        let route = WorkerRoute::issue(1, 2, 3).unwrap();
        let first = registry.for_document();
        let second = registry.for_document();
        let job = first.acquire(Some(&route)).unwrap();
        assert_eq!(first.activity(Some(&route)).0, 1);
        assert_eq!(second.activity(Some(&route)), (0, None));
        assert_eq!(registry.activity(Some(&route)).0, 1);
        assert!(
            registry
                .retire_and_drain(Some(&route), Duration::ZERO)
                .is_err()
        );
        assert!(
            second.acquire(Some(&route)).is_err(),
            "a document view cannot reopen a sealed transport route"
        );
        assert_eq!(second.activity(Some(&route)), (0, None));
        let completed = Instant::now();
        drop(job);
        assert_eq!(first.activity(Some(&route)).0, 0);
        assert!(
            first
                .activity(Some(&route))
                .1
                .is_some_and(|at| at >= completed)
        );
        assert_eq!(second.activity(Some(&route)), (0, None));
        registry
            .retire_and_drain(Some(&route), Duration::ZERO)
            .unwrap();
    }

    #[test]
    fn dropping_document_view_does_not_release_its_outstanding_job() {
        let registry = Registry::default();
        let document = registry.for_document();
        let job = document.acquire(None).unwrap();
        drop(document);
        assert!(registry.retire_and_drain(None, Duration::ZERO).is_err());
        drop(job);
        registry.retire_and_drain(None, Duration::ZERO).unwrap();
    }

    #[test]
    fn timeout_seals_only_exact_route_and_retry_observes_owned_job_release() {
        let registry = Registry::default();
        let first = WorkerRoute::issue(1, 1, 1).unwrap();
        let replacement = WorkerRoute::issue(1, 1, 2).unwrap();
        let other = WorkerRoute::issue(1, 2, 1).unwrap();
        let job = registry.acquire(Some(&first)).unwrap();
        assert!(
            registry
                .retire_and_drain(Some(&first), Duration::ZERO)
                .is_err()
        );
        assert!(registry.acquire(Some(&first)).is_err());
        drop(registry.acquire(Some(&replacement)).unwrap());
        drop(registry.acquire(Some(&other)).unwrap());
        drop(job);
        registry
            .retire_and_drain(Some(&first), Duration::ZERO)
            .unwrap();
        assert!(registry.acquire(Some(&first)).is_err());
    }
    #[test]
    fn control_acknowledgements_retain_teardown_without_resetting_network_quiet() {
        let registry = Registry::default();
        let job = registry.acquire_control(None).unwrap();
        assert_eq!(registry.activity(None), (0, None));
        assert!(registry.retire_and_drain(None, Duration::ZERO).is_err());
        drop(job);
        assert_eq!(registry.activity(None), (0, None));
        registry.retire_and_drain(None, Duration::ZERO).unwrap();
    }

    #[test]
    fn activity_tracks_exact_route_overlap_and_completed_jobs() {
        let registry = Registry::default();
        let route = WorkerRoute::issue(1, 2, 3).unwrap();
        let other = WorkerRoute::issue(1, 2, 4).unwrap();
        assert_eq!(registry.activity(Some(&route)), (0, None));
        assert!(registry.routes.lock().unwrap().is_empty());
        let first = registry.acquire(Some(&route)).unwrap();
        let second = registry.acquire(Some(&route)).unwrap();
        assert_eq!(registry.activity(Some(&route)).0, 2);
        assert_eq!(registry.activity(Some(&other)), (0, None));
        drop(first);
        assert_eq!(registry.activity(Some(&route)).0, 1);
        let terminal = Instant::now();
        drop(second);
        let (active, changed) = registry.activity(Some(&route));
        assert_eq!(active, 0);
        assert!(changed.is_some_and(|at| at >= terminal));
        registry
            .retire_and_drain(Some(&route), Duration::ZERO)
            .unwrap();
        assert_eq!(registry.activity(Some(&route)), (active, changed));
    }

    #[test]
    fn closure_owns_job_until_execution_or_rejected_submission_drops_it() {
        let registry = Registry::default();
        let job = registry.acquire(None).unwrap();
        let queued = move || drop(job);
        assert!(registry.retire_and_drain(None, Duration::ZERO).is_err());
        drop(queued);
        registry.retire_and_drain(None, Duration::ZERO).unwrap();
    }
}

#[cfg(test)]
mod timing_scope_tests {
    use super::*;
    fn observation() -> super::super::resource_timing_scope::Observation {
        super::super::resource_timing_scope::Observation::Start(Box::new(
            super::super::resource_timing_scope::Record {
                render_blocking: false,
                details: None,
                creator: oryn_js::worker_wire::Creator::Window {
                    target: oryn_js::message_wire::WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                },
                #[cfg(feature = "v8-host")]
                name: "https://example.test/script.js".into(),
                initiator: "script".into(),
                started: Instant::now(),
                finished: Instant::now(),
            },
        ))
    }
    #[test]
    fn timing_queues_do_not_follow_activity_ancestry() {
        let parent = Registry::default().for_document();
        let first = parent.for_child_document();
        let second = parent.for_child_document();
        let failure = Err(super::super::RuntimeError::Network(
            "response failed".into(),
        ));
        first.finish_timing(None, observation(), &failure);
        assert!(parent.drain_timings(None).is_empty());
        assert!(second.drain_timings(None).is_empty());
        assert_eq!(first.drain_timings(None).len(), 1);
        assert!(first.drain_timings(None).is_empty());
    }
}
