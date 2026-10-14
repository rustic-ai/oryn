//! Retain queued IndexedDB requests across native submission failures.
use super::*;

pub(super) fn flush(state: &V8BindingState) -> Result<(), V8BindingError> {
    while state.pending_platform_operations.borrow().len()
        < crate::platform_admission::PENDING_LIMIT
    {
        let selected = {
            let queue = state.queued_idb_operations.borrow();
            queue
                .iter()
                .position(|queued| state.idb_operation_ready(&queued.pending))
        };
        let Some(index) = selected else { break };
        // Keep the original request and completion action in the queue until
        // the host accepts submission. A rejection transfers no ownership.
        let (request, pending, connection, request_id) = {
            let queue = state.queued_idb_operations.borrow();
            let queued = &queue[index];
            let PendingPlatformOperation::IndexedDb { request_id, action } = &queued.pending else {
                return Err(V8BindingError::Allocation(
                    "unexpected queued IndexedDB completion",
                ));
            };
            (
                queued.request.clone(),
                PendingPlatformOperation::IndexedDb {
                    request_id: *request_id,
                    action: action.clone(),
                },
                queued
                    .pending
                    .connection_request()
                    .map(|(id, name)| (id, name.to_owned())),
                *request_id,
            )
        };
        state
            .idb_retry_requests
            .borrow_mut()
            .insert(request_id, request.clone());
        // Only accepted connection requests occupy the serialization slot.
        // No author callback runs during the nonblocking native submission.
        submit_platform_operation(state, request, pending)?;
        state
            .queued_idb_operations
            .borrow_mut()
            .remove(index)
            .expect("admitted queued IDB request");
        if let Some((id, name)) = connection {
            state.active_idb_connections.borrow_mut().insert(id, name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Host {
        reject: Cell<bool>,
        accepted: RefCell<Vec<String>>,
    }
    impl PlatformHost for Host {
        fn submit(&self, _: HostOperationId, request: PlatformHostRequest) -> Result<(), String> {
            if self.reject.replace(false) {
                return Err("injected native admission failure".into());
            }
            if let PlatformHostRequest::IndexedDb {
                command: IdbHostCommand::OpenConnection { token, .. },
                ..
            } = request
            {
                self.accepted.borrow_mut().push(token);
            }
            Ok(())
        }
        fn cancel(&self, _: HostOperationId) {}
        fn retire(&self) {}
        fn drain_completions(&self, _: usize) -> Vec<PlatformHostCompletion> {
            Vec::new()
        }
    }
    #[test]
    fn native_admission_failure_preserves_request_action_and_database_order() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(113), 1).unwrap();
        let host = Rc::new(Host {
            reject: Cell::new(true),
            accepted: RefCell::new(Vec::new()),
        });
        *state.platform_host.borrow_mut() = Some(host.clone());
        for id in [1, 2] {
            state
                .queued_idb_operations
                .borrow_mut()
                .push_back(QueuedIdbOperation {
                    request: PlatformHostRequest::IndexedDb {
                        origin: "https://db.example".into(),
                        command: IdbHostCommand::OpenConnection {
                            token: format!("connection-{id}"),
                            name: "db".into(),
                            version: 1,
                        },
                    },
                    pending: PendingPlatformOperation::IndexedDb {
                        request_id: id,
                        action: IdbCompletionAction::Open { name: "db".into() },
                    },
                });
        }
        assert!(flush(&state).is_err());
        assert_eq!(state.queued_idb_operations.borrow().len(), 2);
        assert!(state.active_idb_connections.borrow().is_empty());
        assert!(state.idb_retry_requests.borrow().contains_key(&1));
        assert!(state.pending_platform_operations.borrow().is_empty());
        flush(&state).unwrap();
        assert_eq!(*host.accepted.borrow(), vec!["connection-1"]);
        assert_eq!(state.queued_idb_operations.borrow().len(), 1);
        assert_eq!(
            state
                .active_idb_connections
                .borrow()
                .get(&1)
                .map(String::as_str),
            Some("db")
        );
        flush(&state).unwrap();
        assert_eq!(
            host.accepted.borrow().len(),
            1,
            "later database opens cannot overtake accepted unfinished opens"
        );
        state.active_idb_connections.borrow_mut().remove(&1);
        flush(&state).unwrap();
        assert_eq!(
            *host.accepted.borrow(),
            vec!["connection-1", "connection-2"]
        );
        assert!(state.queued_idb_operations.borrow().is_empty());
        state.pending_platform_operations.borrow_mut().clear();
        state.active_idb_connections.borrow_mut().clear();
        *state.security_origin.borrow_mut() = Some("https://db.example".into());
        let database = register_platform_object(
            &state,
            PlatformObjectRecord::IdbDatabase(IdbDatabaseRecord {
                connection: Some("existing".into()),
                last_versionchange_event: 0,
                name: "transactions".into(),
                version: 1,
                old_version: 1,
                object_stores: vec!["records".into()],
                object_store_definitions: BTreeMap::new(),
                upgrade_base_object_stores: Vec::new(),
                upgrade_base_definitions: BTreeMap::new(),
                upgrade_transaction_id: None,
                closed: false,
            }),
        );
        let transaction = register_platform_object(
            &state,
            PlatformObjectRecord::IdbTransaction(IdbTransactionRecord {
                request_queue: VecDeque::new(),
                abort_event_queued: false,
                settled: false,
                database_id: database,
                request_id: None,
                mode: "readonly".into(),
                durability: "default".into(),
                object_stores: vec!["records".into()],
                scope_custody: None,
                aborted: false,
                error: None,
                pending_requests: 0,
                completion_requested: false,
                completion_queued: false,
                explicit_commit: false,
                active: true,
                finished: false,
            }),
        );
        let request = new_idb_request(&state, Some(transaction));
        host.reject.set(true);
        assert!(
            submit_idb(
                scope,
                &state,
                request,
                IdbHostCommand::Get {
                    database: "transactions".into(),
                    store: "records".into(),
                    key: serde_json::json!(1)
                },
                IdbCompletionAction::Value
            )
            .is_err()
        );
        {
            let record = platform_object(&state, transaction).unwrap();
            let PlatformObjectRecord::IdbTransaction(transaction) = &*record else {
                panic!()
            };
            assert_eq!(transaction.pending_requests, 1);
            assert_eq!(transaction.request_queue, VecDeque::from([request]));
        }
        assert_eq!(state.queued_idb_operations.borrow().len(), 1);
        flush(&state).unwrap();
        {
            let record = platform_object(&state, transaction).unwrap();
            let PlatformObjectRecord::IdbTransaction(transaction) = &*record else {
                panic!()
            };
            assert_eq!(
                transaction.pending_requests, 1,
                "retry must not increment accounting again"
            );
            assert_eq!(transaction.request_queue, VecDeque::from([request]));
        }
    }
}
