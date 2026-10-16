//! Preallocated publication ticket; rejection never allocates or borrows edges.
use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Status {
    Pending,
    Rejected,
    Published,
    WaitingWeak,
    Retired,
}
pub(super) struct Ticket {
    status: Rc<Cell<Status>>,
    ready: Rc<Cell<usize>>,
}
impl Ticket {
    pub(super) fn published(self) {
        if self.status.get() != Status::Pending {
            return;
        }
        self.status.set(Status::Published);
        self.ready.set(
            self.ready
                .get()
                .checked_add(1)
                .expect("publication candidates fit native registry"),
        );
    }
}
pub(super) fn register_record(
    state: &V8BindingState,
    record: PlatformObjectRecord,
) -> Result<(u64, Ticket), V8BindingError> {
    let status = Rc::new(Cell::new(Status::Pending));
    let mut edges = state
        .platform_edges
        .try_borrow_mut()
        .map_err(|_| V8BindingError::Allocation("unpublished edges borrow"))?;
    let mut objects = state
        .platform_objects
        .try_borrow_mut()
        .map_err(|_| V8BindingError::Allocation("unpublished objects borrow"))?;
    let id = state.next_platform_object_id.get();
    let next = id
        .checked_add(1)
        .filter(|_| id != 0)
        .ok_or(V8BindingError::Allocation("unpublished object identity"))?;
    if objects.contains_key(&id) {
        return Err(V8BindingError::Allocation("unpublished identity collision"));
    }
    edges.registered(id, &record);
    edges.unpublished_candidates.insert(id, status.clone());
    objects.insert(id, record);
    state.next_platform_object_id.set(next);
    let ready = edges.unpublished_ready.clone();
    Ok((id, Ticket { status, ready }))
}
pub(super) fn request(
    state: &V8BindingState,
    transaction_id: Option<u64>,
) -> Result<(u64, Ticket), V8BindingError> {
    register_record(
        state,
        PlatformObjectRecord::IdbRequest(IdbRequestRecord {
            versionchange_notified: false,
            failure_event_queued: false,
            internal: true,
            open_request: false,
            result: None,
            error: None,
            transaction_id,
            source_id: None,
            done: false,
        }),
    )
}
impl Drop for Ticket {
    fn drop(&mut self) {
        if self.status.get() == Status::Pending {
            self.status.set(Status::Rejected);
            self.ready.set(
                self.ready
                    .get()
                    .checked_add(1)
                    .expect("publication candidates fit native registry"),
            );
        }
    }
}
#[cfg(test)]
pub(super) fn discard(state: &V8BindingState, id: u64) {
    // Legacy non-ticket caller: acquire all guards before any side effect.
    let Ok(mut objects) = state.platform_objects.try_borrow_mut() else {
        return;
    };
    let Ok(mut edges) = state.platform_edges.try_borrow_mut() else {
        return;
    };
    let Ok(watchers) = state.platform_listener_watchers.try_borrow() else {
        return;
    };
    if watchers.contains_key(&id) {
        return;
    }
    edges.discard_unpublished(id);
    objects.remove(&id);
}
pub(super) fn drain(state: &V8BindingState) {
    let Ok(mut objects) = state.platform_objects.try_borrow_mut() else {
        return;
    };
    let Ok(mut edges) = state.platform_edges.try_borrow_mut() else {
        return;
    };
    let Ok(watchers) = state.platform_listener_watchers.try_borrow() else {
        return;
    };
    // Stable numeric cursor; no temporary vector or allocation during cleanup.
    let mut after = edges.unpublished_cursor;
    for _ in 0..1 {
        let next = match after {
            None => edges.unpublished_candidates.first_key_value(),
            Some(id) => edges
                .unpublished_candidates
                .range((std::ops::Bound::Excluded(id), std::ops::Bound::Unbounded))
                .next(),
        }
        .map(|(id, status)| (*id, status.get()));
        let Some((id, status)) = next else {
            edges.unpublished_cursor = None;
            break;
        };
        after = Some(id);
        edges.unpublished_cursor = after;
        match status {
            Status::Pending => {}
            Status::Published => {
                retire(&mut edges, id);
            }
            Status::Rejected => {
                if !watchers.contains_key(&id) {
                    edges.discard_unpublished(id);
                    objects.remove(&id);
                    retire(&mut edges, id);
                } else {
                    if let Some(status) = edges.unpublished_candidates.get(&id) {
                        status.set(Status::WaitingWeak);
                    }
                    edges
                        .unpublished_ready
                        .set(edges.unpublished_ready.get() - 1);
                }
            }
            Status::WaitingWeak => {}
            Status::Retired => {}
        }
    }
}
/// All candidate removal paths invalidate the shared ticket before removal.
pub(super) fn retire(edges: &mut platform_listener_graph::Edges, id: u64) {
    if let Some(status) = edges.unpublished_candidates.remove(&id) {
        let previous = status.replace(Status::Retired);
        if matches!(previous, Status::Rejected | Status::Published) {
            edges.unpublished_ready.set(
                edges
                    .unpublished_ready
                    .get()
                    .checked_sub(1)
                    .expect("retiring a ready publication candidate"),
            );
        }
    }
}
pub(super) fn pending(state: &V8BindingState) -> bool {
    state
        .platform_edges
        .try_borrow()
        .map_or(true, |edges| edges.unpublished_ready.get() != 0)
}
pub(super) fn weak_released(state: &V8BindingState, id: u64) -> bool {
    let Ok(edges) = state.platform_edges.try_borrow() else {
        return false;
    };
    if let Some(status) = edges.unpublished_candidates.get(&id)
        && status.get() == Status::WaitingWeak
    {
        status.set(Status::Rejected);
        edges.unpublished_ready.set(
            edges
                .unpublished_ready
                .get()
                .checked_add(1)
                .expect("native candidates fit registry"),
        );
    }
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ticket_registration_rejection_and_pending_are_atomic() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let next = state.next_platform_object_id.get();
        let edges = state.platform_edges.borrow_mut();
        assert!(request(&state, None).is_err());
        assert_eq!(state.next_platform_object_id.get(), next);
        drop(edges);
        let (id, ticket) = request(&state, None).unwrap();
        drain(&state);
        assert!(state.platform_objects.borrow().contains_key(&id));
        let releases = state.platform_listener_releases.borrow_mut();
        let objects = state.platform_objects.borrow();
        drop(ticket);
        drain(&state);
        assert!(objects.contains_key(&id));
        drop(objects);
        drop(releases);
        drain(&state); // cursor wraps before revisiting the rejected record.
        drain(&state);
        assert!(!state.platform_objects.borrow().contains_key(&id));
        let (id, ticket) = request(&state, None).unwrap();
        ticket.published();
        drain(&state); // completed prior ID left the cursor at the range end.
        drain(&state);
        assert!(state.platform_objects.borrow().contains_key(&id));
        assert!(
            !state
                .platform_edges
                .borrow()
                .unpublished_candidates
                .contains_key(&id)
        );
        discard(&state, id);
        for published in [false, true] {
            let (retiring_id, retiring_ticket) = request(&state, None).unwrap();
            if published {
                retiring_ticket.published();
                assert!(pending(&state));
                state
                    .platform_edges
                    .borrow_mut()
                    .discard_unpublished(retiring_id);
            } else {
                state
                    .platform_edges
                    .borrow_mut()
                    .discard_unpublished(retiring_id);
                drop(retiring_ticket); // stale Drop must not resurrect ready work.
            }
            state.platform_objects.borrow_mut().remove(&retiring_id);
            assert!(!pending(&state));
            assert_eq!(state.platform_edges.borrow().unpublished_ready.get(), 0);
            assert!(
                !state
                    .platform_edges
                    .borrow()
                    .unpublished_candidates
                    .contains_key(&retiring_id)
            );
        }
        let (id, ticket) = request(&state, None).unwrap();
        let root = {
            v8::scope!(let inner,scope);
            let wrapper = new_platform_object(inner, &state, "IDBRequest", id).unwrap();
            v8::Global::new(inner, wrapper)
        };
        drop(ticket);
        for _ in 0..3 {
            drain(&state);
        }
        assert!(state.platform_objects.borrow().contains_key(&id));
        assert!(!pending(&state));
        assert!(matches!(
            state
                .platform_edges
                .borrow()
                .unpublished_candidates
                .get(&id)
                .map(|s| s.get()),
            Some(Status::WaitingWeak)
        ));
        let serial = state
            .platform_listener_watchers
            .borrow()
            .get(&id)
            .unwrap()
            .0;
        // The nested handle scope is closed; this is the last wrapper root.
        drop(root);
        for _ in 0..3 {
            scope.low_memory_notification();
            if state
                .platform_listener_releases
                .borrow()
                .contains(&(id, serial))
            {
                break;
            }
        }
        assert!(
            state
                .platform_listener_releases
                .borrow()
                .contains(&(id, serial))
        );
        let edges = state.platform_edges.borrow_mut();
        platform_listener_graph::reclaim(&state);
        assert!(
            state
                .platform_listener_releases
                .borrow()
                .contains(&(id, serial))
        );
        drop(edges);
        for _ in 0..3 {
            scope.low_memory_notification();
            platform_listener_graph::reclaim(&state);
        }
        assert!(!state.platform_objects.borrow().contains_key(&id));
    }
}
