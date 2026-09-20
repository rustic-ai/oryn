//! IndexedDB completion ownership after the native response is accepted.
//! Native publication and each author dispatch have independent retained stages.
use super::native_platform::publication;
use super::*;
#[cfg(test)]
thread_local! {static FAIL_VALUE_ONCE:Cell<bool>=const{Cell::new(false)};static FAIL_AFTER_ACCOUNT_ONCE:Cell<bool>=const{Cell::new(false)};}

#[derive(Default)]
pub(super) struct Deliveries {
    queue: RefCell<VecDeque<Task>>,
    running: Cell<bool>,
    active: RefCell<Vec<u64>>,
}
#[derive(Clone)]
enum Step {
    ConnectionPrepare(u64, u64, u64, Option<u64>),
    ConnectionAck {
        token: String,
        name: String,
        event: u64,
    },
    FinalizeOpen(u64, u64, u64),
    FinalizeTransaction(u64, bool),
    Databases,
    Prepare,
    Publish,
    Activate(u64),
    TerminalPrepare(u64, &'static str),
    TerminalAfter(u64, &'static str),
    Retire {
        database: u64,
        token: String,
        name: String,
    },
    LegacyCheck {
        name: String,
        version: Option<u64>,
        old: u64,
    },
    WaitLegacy {
        name: String,
    },
    Event(u64, &'static str, Option<(u64, Option<u64>)>),
    AfterRequest {
        error: Option<String>,
    },
    AfterUpgrade(u64),
    Abort(u64),
    FailedUpgrade(u64),
    Finish(u64),
    QueueCompletion(u64),
    Flush,
    Poll,
    Checkpoint,
}
struct Event {
    notification: publication::Notification,
    id: u64,
}
struct Task {
    resolver: Option<v8::Global<v8::PromiseResolver>>,
    request: u64,
    action: IdbCompletionAction,
    response: Result<IdbHostResponse, String>,
    steps: VecDeque<Step>,
    value: Option<v8::Global<v8::Value>>,
    created: Option<u64>,
    transaction: Option<u64>,
    event: Option<Event>,
    threw: bool,
    prevented: bool,
    identity: (NodeId, u64, u64),
    finish_accounted: BTreeSet<u64>,
    legacy_started: bool,
    cleanup: Option<u64>,
    batch: Vec<v8::Global<v8::Value>>,
}
fn identity(state: &V8BindingState) -> (NodeId, u64, u64) {
    (
        state.document_root(),
        state.document_generation(),
        state.document_authority_epoch(),
    )
}
pub(super) fn running(state: &V8BindingState) -> bool {
    state.idb_deliveries.running.get()
}
pub(super) fn roots(state: &V8BindingState) -> Vec<u64> {
    let mut roots = state.idb_deliveries.active.borrow().clone();
    for task in state.idb_deliveries.queue.borrow().iter() {
        roots.extend(task.roots());
    }
    roots
}
pub(super) fn pending(state: &V8BindingState) -> bool {
    !state.idb_deliveries.queue.borrow().is_empty()
}
pub(super) fn enqueue(
    state: &V8BindingState,
    request: u64,
    action: IdbCompletionAction,
    response: Result<IdbHostResponse, String>,
) {
    state.idb_deliveries.queue.borrow_mut().push_back(Task {
        resolver: None,
        request,
        action,
        response,
        steps: VecDeque::from([Step::Prepare]),
        value: None,
        created: None,
        transaction: None,
        event: None,
        threw: false,
        prevented: false,
        identity: identity(state),
        finish_accounted: BTreeSet::new(),
        legacy_started: false,
        cleanup: None,
        batch: Vec::new(),
    });
}
pub(super) fn databases(
    state: &V8BindingState,
    resolver: v8::Global<v8::PromiseResolver>,
    response: Result<IdbHostResponse, String>,
) {
    state.idb_deliveries.queue.borrow_mut().push_back(Task {
        resolver: Some(resolver),
        request: 0,
        action: IdbCompletionAction::Unit,
        response,
        steps: VecDeque::from([Step::Databases]),
        value: None,
        created: None,
        transaction: None,
        event: None,
        threw: false,
        prevented: false,
        identity: identity(state),
        finish_accounted: BTreeSet::new(),
        legacy_started: false,
        cleanup: None,
        batch: Vec::new(),
    });
}
pub(super) fn finalize(
    state: &V8BindingState,
    transaction: u64,
    open: Option<(u64, u64)>,
    commit: bool,
) {
    let step = open.map_or(
        Step::FinalizeTransaction(transaction, commit),
        |(request, database)| Step::FinalizeOpen(request, database, transaction),
    );
    state.idb_deliveries.queue.borrow_mut().push_back(Task {
        resolver: None,
        request: transaction,
        action: IdbCompletionAction::Unit,
        response: Ok(IdbHostResponse::default()),
        steps: VecDeque::from([step]),
        value: None,
        created: None,
        transaction: Some(transaction),
        event: None,
        threw: false,
        prevented: false,
        identity: identity(state),
        finish_accounted: BTreeSet::new(),
        legacy_started: false,
        cleanup: None,
        batch: Vec::new(),
    });
}
pub(super) fn connection_events(state: &V8BindingState, database: u64, response: IdbHostResponse) {
    let steps = response
        .connection_events
        .into_iter()
        .map(|event| {
            Step::ConnectionPrepare(database, event.event, event.old_version, event.new_version)
        })
        .collect();
    state.idb_deliveries.queue.borrow_mut().push_back(Task {
        resolver: None,
        request: database,
        action: IdbCompletionAction::Unit,
        response: Ok(IdbHostResponse::default()),
        steps,
        value: None,
        created: None,
        transaction: None,
        event: None,
        threw: false,
        prevented: false,
        identity: identity(state),
        finish_accounted: BTreeSet::new(),
        legacy_started: false,
        cleanup: None,
        batch: Vec::new(),
    });
}
pub(super) fn terminal(state: &V8BindingState, target: u64, name: &'static str) {
    state.idb_deliveries.queue.borrow_mut().push_back(Task {
        resolver: None,
        request: target,
        action: IdbCompletionAction::Unit,
        response: Ok(IdbHostResponse::default()),
        steps: VecDeque::from([Step::TerminalPrepare(target, name)]),
        value: None,
        created: None,
        transaction: None,
        event: None,
        threw: false,
        prevented: false,
        identity: identity(state),
        finish_accounted: BTreeSet::new(),
        legacy_started: false,
        cleanup: None,
        batch: Vec::new(),
    });
}
pub(super) fn blocked(state: &V8BindingState, request: u64, old: u64, new: Option<u64>) {
    state.idb_deliveries.queue.borrow_mut().push_back(Task {
        resolver: None,
        request,
        action: IdbCompletionAction::Unit,
        response: Ok(IdbHostResponse::default()),
        steps: VecDeque::from([Step::Event(request, "blocked", Some((old, new)))]),
        value: None,
        created: None,
        transaction: None,
        event: None,
        threw: false,
        prevented: false,
        identity: identity(state),
        finish_accounted: BTreeSet::new(),
        legacy_started: false,
        cleanup: None,
        batch: Vec::new(),
    });
}
pub(super) fn deferred_failure(state: &V8BindingState, request: u64, open: bool) {
    if let Ok(mut record) = platform_object_mut(state, request)
        && let PlatformObjectRecord::IdbRequest(record) = &mut *record
    {
        record.done = true;
        record.result = None;
        record.error = Some("AbortError".into());
        if open {
            record.transaction_id = None;
        }
    }
    let task = Task {
        resolver: None,
        request,
        action: IdbCompletionAction::Unit,
        response: Err("AbortError".into()),
        steps: VecDeque::from([
            Step::Event(request, "error", None),
            Step::Finish(request),
            Step::Flush,
        ]),
        value: None,
        created: None,
        transaction: None,
        event: None,
        threw: false,
        prevented: false,
        identity: identity(state),
        finish_accounted: BTreeSet::new(),
        legacy_started: false,
        cleanup: None,
        batch: Vec::new(),
    };
    state.idb_deliveries.queue.borrow_mut().push_back(task);
}
pub(super) fn retire(state: &V8BindingState) {
    for task in state.idb_deliveries.queue.borrow_mut().drain(..) {
        if let Some(event) = task.event {
            event.notification.cancel(state);
        }
    }
}
pub(super) fn drain(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    if state.idb_deliveries.running.replace(true) {
        return Ok(());
    }
    struct Guard<'a>(&'a Deliveries);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.running.set(false);
            self.0.active.borrow_mut().clear();
        }
    }
    let _guard = Guard(&state.idb_deliveries);
    loop {
        let Some(mut task) = state.idb_deliveries.queue.borrow_mut().pop_front() else {
            return Ok(());
        };
        match task.poll(scope, state) {
            Ok(true) => {}
            Ok(false) => {
                state.idb_deliveries.queue.borrow_mut().push_front(task);
                return Ok(());
            }
            Err(error) => {
                state.idb_deliveries.queue.borrow_mut().push_front(task);
                return Err(error);
            }
        }
    }
}
impl Task {
    fn roots(&self) -> Vec<u64> {
        let mut roots = vec![self.request];
        for step in &self.steps {
            if let Step::FinalizeOpen(request, database, transaction) = step {
                roots.extend([*request, *database, *transaction]);
            }
        }
        roots.extend(self.cleanup);
        roots.extend(self.created);
        roots.extend(self.transaction);
        if let IdbCompletionAction::OpenFinalized {
            request_id,
            database_id,
            transaction_id,
        } = self.action
        {
            roots.extend([request_id, database_id, transaction_id]);
        }
        roots
    }
    fn add_front(&mut self, steps: impl IntoIterator<Item = Step>) {
        let steps = steps.into_iter().collect::<Vec<_>>();
        for step in steps.into_iter().rev() {
            self.steps.push_front(step);
        }
    }
    fn transaction_of(state: &V8BindingState, request: u64) -> Result<Option<u64>, V8BindingError> {
        Ok(match &*platform_object(state, request)? {
            PlatformObjectRecord::IdbRequest(request) => request.transaction_id,
            _ => None,
        })
    }
    fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<bool, V8BindingError> {
        loop {
            *state.idb_deliveries.active.borrow_mut() = self.roots();
            if state.execution_retired() || identity(state) != self.identity {
                if let Some(event) = self.event.take() {
                    event.notification.cancel(state);
                }
                return Ok(true);
            }
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            let Some(step) = self.steps.front().cloned() else {
                return Ok(true);
            };
            match step {
                Step::ConnectionPrepare(database, event, old, new) => {
                    let connection = match &*platform_object(state, database)? {
                        PlatformObjectRecord::IdbDatabase(record) if !record.closed => {
                            record.connection.clone().map(|token| {
                                (
                                    token,
                                    record.name.clone(),
                                    event > record.last_versionchange_event,
                                )
                            })
                        }
                        _ => None,
                    };
                    if let Some((token, name, fresh)) = connection {
                        // Custody has already moved from the accepted host receipt to
                        // this task. Advance the event number only with a retained
                        // listener continuation and native acknowledgement suffix.
                        if fresh
                            && let PlatformObjectRecord::IdbDatabase(record) =
                                &mut *platform_object_mut(state, database)?
                        {
                            record.last_versionchange_event = event;
                        }
                        self.steps.pop_front();
                        self.add_front([Step::ConnectionAck { token, name, event }]);
                        if fresh {
                            self.add_front([
                                Step::Event(database, "versionchange", Some((old, new))),
                                Step::Checkpoint,
                            ]);
                        }
                    } else {
                        self.steps.pop_front();
                    }
                }
                Step::ConnectionAck { token, name, event } => {
                    submit_platform_operation(
                        state,
                        PlatformHostRequest::IndexedDb {
                            origin: current_platform_origin(state),
                            command: IdbHostCommand::AcknowledgeConnection { token, name, event },
                        },
                        PendingPlatformOperation::WebSocketCommand,
                    )?;
                    self.steps.pop_front();
                }
                Step::FinalizeOpen(request, database, transaction) => {
                    if matches!(&*platform_object(state,transaction)?,PlatformObjectRecord::IdbTransaction(record)if record.aborted)
                    {
                        self.steps.pop_front();
                        self.add_front([
                            Step::Abort(transaction),
                            Step::FailedUpgrade(transaction),
                            Step::Flush,
                        ]);
                        continue;
                    }
                    let name = match &*platform_object(state, database)? {
                        PlatformObjectRecord::IdbDatabase(record) => record.name.clone(),
                        _ => return Err(V8BindingError::Allocation("IDB finalize database")),
                    };
                    let hidden = *self
                        .cleanup
                        .get_or_insert_with(|| new_internal_idb_request(state, None));
                    queue_idb(
                        scope,
                        state,
                        hidden,
                        IdbHostCommand::CommitUpgrade { name },
                        IdbCompletionAction::OpenFinalized {
                            request_id: request,
                            database_id: database,
                            transaction_id: transaction,
                        },
                    )?;
                    // The open request exposes the upgrade transaction through
                    // its complete event. TerminalAfter clears it before jobs
                    // queued by that callback and before open success.
                    self.steps.pop_front();
                    self.add_front([Step::Flush]);
                }
                Step::FinalizeTransaction(transaction, commit) => {
                    let database = match &*platform_object(state, transaction)? {
                        PlatformObjectRecord::IdbTransaction(record) => record.database_id,
                        _ => return Err(V8BindingError::Allocation("IDB finalize transaction")),
                    };
                    let database = match &*platform_object(state, database)? {
                        PlatformObjectRecord::IdbDatabase(record) => record.name.clone(),
                        _ => return Err(V8BindingError::Allocation("IDB finalize database")),
                    };
                    let token = format!("{}:{transaction}", state.instance_id);
                    let command = if commit {
                        IdbHostCommand::CommitTransaction { token, database }
                    } else {
                        IdbHostCommand::AbortTransaction { token, database }
                    };
                    let hidden = *self
                        .cleanup
                        .get_or_insert_with(|| new_internal_idb_request(state, None));
                    queue_idb(
                        scope,
                        state,
                        hidden,
                        command,
                        IdbCompletionAction::TransactionFinalized {
                            transaction_id: transaction,
                            event_type: if commit { "complete" } else { "abort" },
                        },
                    )?;
                    self.steps.pop_front();
                    self.add_front([Step::Flush]);
                }
                Step::Databases => {
                    if self.value.is_none() {
                        let value = match &self.response {
                            Ok(response) => {
                                let array = v8::Array::new(scope, response.databases.len() as i32);
                                for (index, database) in response.databases.iter().enumerate() {
                                    let value = v8::Object::new(scope);
                                    let name = v8_string(scope, &database.name)?;
                                    set_object_property(scope, value, "name", name)?;
                                    let version =
                                        v8::Number::new(scope, database.version as f64).into();
                                    set_object_property(scope, value, "version", version)?;
                                    if array.set_index(scope, index as u32, value.into())
                                        != Some(true)
                                    {
                                        return Err(V8BindingError::Allocation(
                                            "IDB database enumeration",
                                        ));
                                    }
                                }
                                array.into()
                            }
                            Err(error) => dom_exception_value(scope, &idb_error_name(error))?,
                        };
                        self.value = Some(v8::Global::new(scope, value));
                    }
                    let resolver = v8::Local::new(
                        scope,
                        self.resolver
                            .as_ref()
                            .ok_or(V8BindingError::Allocation("IDB enumeration resolver"))?,
                    );
                    let value = v8::Local::new(scope, self.value.as_ref().unwrap());
                    let settled = if self.response.is_ok() {
                        resolver.resolve(scope, value)
                    } else {
                        resolver.reject(scope, value)
                    };
                    if settled != Some(true) {
                        return Err(V8BindingError::Allocation("IDB enumeration settlement"));
                    }
                    self.steps.pop_front();
                }
                Step::Prepare => {
                    self.prepare(scope, state)?;
                    self.steps.pop_front();
                }
                Step::TerminalPrepare(target, name) => {
                    let database = {
                        let mut record = platform_object_mut(state, target)?;
                        let PlatformObjectRecord::IdbTransaction(transaction) = &mut *record else {
                            return Err(V8BindingError::Allocation("IDB terminal transaction"));
                        };
                        if name == "complete" && transaction.aborted {
                            self.steps.clear();
                            continue;
                        }
                        transaction.active = false;
                        transaction.finished = true;
                        transaction.settled = true;
                        (transaction.mode == "versionchange").then_some(transaction.database_id)
                    };
                    if let Some(database) = database
                        && let PlatformObjectRecord::IdbDatabase(database) =
                            &mut *platform_object_mut(state, database)?
                    {
                        database.upgrade_transaction_id = None;
                    }
                    self.steps.pop_front();
                    self.add_front([
                        Step::Event(target, name, None),
                        Step::TerminalAfter(target, name),
                    ]);
                }
                Step::TerminalAfter(target, name) => {
                    let request = match &*platform_object(state, target)? {
                        PlatformObjectRecord::IdbTransaction(transaction)
                            if transaction.mode == "versionchange" =>
                        {
                            transaction.request_id
                        }
                        _ => None,
                    };
                    if let Some(request) = request
                        && let PlatformObjectRecord::IdbRequest(request) =
                            &mut *platform_object_mut(state, request)?
                    {
                        request.transaction_id = None;
                        if name == "abort" {
                            request.result = None;
                            request.done = false;
                        }
                    }
                    self.steps.pop_front();
                }
                Step::Retire {
                    database,
                    token,
                    name,
                } => {
                    let hidden = *self
                        .cleanup
                        .get_or_insert_with(|| new_internal_idb_request(state, None));
                    queue_idb(
                        scope,
                        state,
                        hidden,
                        IdbHostCommand::RetireConnection { token, name },
                        IdbCompletionAction::Unit,
                    )?;
                    if let PlatformObjectRecord::IdbDatabase(database) =
                        &mut *platform_object_mut(state, database)?
                    {
                        database.connection = None;
                    }
                    self.steps.pop_front();
                }
                Step::LegacyCheck { name, version, old } => {
                    self.steps.pop_front();
                    if state.idb_database_blocks_change(&name) {
                        self.add_front([
                            Step::Event(self.request, "blocked", Some((old, version))),
                            Step::WaitLegacy { name },
                        ]);
                    } else {
                        self.add_front([Step::Prepare]);
                    }
                }
                Step::WaitLegacy { name } => {
                    if state.idb_database_blocks_change(&name) {
                        if let PlatformObjectRecord::IdbRequest(request) =
                            &mut *platform_object_mut(state, self.request)?
                        {
                            request.done = false;
                        }
                        state.blocked_idb_completions.borrow_mut().push_back((
                            self.request,
                            self.action.clone(),
                            self.response.clone().map_err(V8BindingError::Message)?,
                        ));
                        self.steps.clear();
                    } else {
                        self.steps.pop_front();
                        self.add_front([Step::Prepare]);
                    }
                }
                Step::Publish => {
                    let value = v8::Local::new(
                        scope,
                        self.value
                            .as_ref()
                            .ok_or(V8BindingError::Allocation("retained IDB result"))?,
                    );
                    set_idb_request_result(scope, state, self.request, value)?;
                    self.steps.pop_front();
                }
                Step::Activate(request) => {
                    state.activate_idb_request_transaction(request)?;
                    self.steps.pop_front();
                }
                Step::Event(target, name, version) => {
                    if matches!(name, "success" | "error" | "upgradeneeded")
                        && let PlatformObjectRecord::IdbRequest(request) =
                            &mut *platform_object_mut(state, target)?
                    {
                        request.done = true;
                        if request.internal {
                            self.threw = false;
                            self.prevented = false;
                            self.steps.pop_front();
                            continue;
                        }
                    }
                    if self.event.is_none() {
                        let event = if let Some((old, new)) = version {
                            idb_version_change_event(scope, old, new)?
                        } else {
                            create_event_object(
                                scope,
                                state,
                                name,
                                name == "error" || name == "abort",
                                name == "error",
                            )?
                        };
                        let id = event_id(scope, event)?;
                        let notification =
                            publication::pending_idb_dispatch(scope, state, target, name, event)?;
                        self.event = Some(Event { notification, id });
                    }
                    let event = self.event.as_mut().unwrap();
                    if !event.notification.resume(scope, state)? {
                        return Ok(false);
                    }
                    self.threw = event.notification.threw();
                    self.prevented = state
                        .events
                        .borrow()
                        .get(&event.id)
                        .is_some_and(|record| record.default_prevented);
                    event.notification.finish(scope, state);
                    self.event = None;
                    self.steps.pop_front();
                }
                Step::AfterRequest { error } => {
                    let transaction = Self::transaction_of(state, self.request)?;
                    let mut abort = None;
                    if let Some(id) = transaction {
                        let mut record = platform_object_mut(state, id)?;
                        if let PlatformObjectRecord::IdbTransaction(transaction) = &mut *record {
                            let must_abort = if error.is_some() {
                                self.threw || !self.prevented
                            } else {
                                self.threw && !transaction.explicit_commit
                            };
                            if must_abort {
                                transaction.aborted = true;
                                transaction.error = Some(if self.threw {
                                    "AbortError".into()
                                } else {
                                    error.clone().unwrap()
                                });
                                abort = Some(id);
                            }
                        }
                    }
                    self.steps.pop_front();
                    if let Some(id) = abort {
                        self.add_front([Step::Abort(id), Step::FailedUpgrade(id)]);
                    }
                }
                Step::AfterUpgrade(id) => {
                    let aborted = {
                        let mut record = platform_object_mut(state, id)?;
                        let PlatformObjectRecord::IdbTransaction(transaction) = &mut *record else {
                            return Err(V8BindingError::Allocation("IDB upgrade transaction"));
                        };
                        if self.threw {
                            transaction.aborted = true;
                            transaction.error = Some("AbortError".into());
                        }
                        if !transaction.aborted {
                            transaction.completion_requested = true;
                        }
                        transaction.aborted
                    };
                    self.steps.pop_front();
                    if aborted {
                        self.add_front([Step::Abort(id), Step::FailedUpgrade(id)]);
                    } else {
                        self.add_front([Step::QueueCompletion(id)]);
                    }
                }
                Step::Abort(id) => {
                    dispatch_or_queue_idb_abort(scope, state, id)?;
                    self.steps.pop_front();
                }
                Step::FailedUpgrade(id) => {
                    finalize_failed_idb_upgrade_retained(scope, state, id, &mut self.cleanup)?;
                    self.steps.pop_front();
                }
                Step::Finish(request) => {
                    let transaction = Self::transaction_of(state, request)?;
                    if !self.finish_accounted.contains(&request) {
                        state.active_idb_connections.borrow_mut().remove(&request);
                        remove_idb_request_from_queue(state, request)?;
                        if let Some(id) = transaction
                            && let PlatformObjectRecord::IdbTransaction(transaction) =
                                &mut *platform_object_mut(state, id)?
                        {
                            transaction.pending_requests =
                                transaction.pending_requests.saturating_sub(1);
                            if transaction.mode != "versionchange"
                                && transaction.pending_requests == 0
                            {
                                transaction.completion_requested = true;
                            }
                        }
                        self.finish_accounted.insert(request);
                        #[cfg(test)]
                        if FAIL_AFTER_ACCOUNT_ONCE.with(|flag| flag.replace(false)) {
                            return Err(V8BindingError::Allocation(
                                "injected IDB accounting suffix failure",
                            ));
                        }
                    }
                    if let Some(id) = transaction {
                        queue_idb_transaction_completion(state, id)?;
                    }
                    self.steps.pop_front();
                }
                Step::QueueCompletion(id) => {
                    queue_idb_transaction_completion(state, id)?;
                    self.steps.pop_front();
                }
                Step::Flush => {
                    state.flush_queued_idb_operations()?;
                    self.steps.pop_front();
                }
                Step::Poll => {
                    state.poll_idb_connections()?;
                    self.steps.pop_front();
                }
                Step::Checkpoint => {
                    scope.perform_microtask_checkpoint();
                    state.finish_platform_task_checkpoint()?;
                    self.steps.pop_front();
                }
            }
        }
    }
    fn prepare(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<(), V8BindingError> {
        #[cfg(test)]
        if FAIL_VALUE_ONCE.with(|flag| flag.replace(false)) {
            return Err(V8BindingError::Allocation(
                "injected IDB result preparation failure",
            ));
        }
        let response = match self.response.clone() {
            Ok(response) => response,
            Err(error) => {
                let name = idb_error_name(&error);
                if let IdbCompletionAction::OpenFinalized {
                    request_id: original,
                    database_id,
                    transaction_id,
                } = self.action.clone()
                {
                    let connection = {
                        let mut record = platform_object_mut(state, database_id)?;
                        let PlatformObjectRecord::IdbDatabase(database) = &mut *record else {
                            return Err(V8BindingError::Allocation("IDB database"));
                        };
                        database.closed = true;
                        database.upgrade_transaction_id = None;
                        database
                            .connection
                            .clone()
                            .map(|token| (token, database.name.clone()))
                    };
                    if let PlatformObjectRecord::IdbTransaction(transaction) =
                        &mut *platform_object_mut(state, transaction_id)?
                    {
                        transaction.aborted = true;
                        transaction.finished = true;
                        transaction.settled = true;
                        transaction.active = false;
                        transaction.error = Some(name.clone());
                    }
                    if let PlatformObjectRecord::IdbRequest(request) =
                        &mut *platform_object_mut(state, original)?
                    {
                        request.result = None;
                        request.error = Some(name);
                    }
                    if let Some((token, name)) = connection {
                        self.steps.push_back(Step::Retire {
                            database: database_id,
                            token,
                            name,
                        });
                    }
                    self.steps.extend([
                        Step::Event(transaction_id, "abort", None),
                        Step::Checkpoint,
                        Step::Event(original, "error", None),
                        Step::Finish(original),
                        Step::Finish(self.request),
                        Step::Flush,
                    ]);
                    return Ok(());
                }
                if let PlatformObjectRecord::IdbRequest(request) =
                    &mut *platform_object_mut(state, self.request)?
                {
                    request.error = Some(name.clone());
                    request.result = None;
                }
                self.steps.extend([
                    Step::Activate(self.request),
                    Step::Event(self.request, "error", None),
                    Step::AfterRequest { error: Some(name) },
                    Step::Finish(self.request),
                    Step::Flush,
                ]);
                return Ok(());
            }
        };
        let change = match &self.action {
            IdbCompletionAction::Open { name } if response.upgrade_needed => {
                Some((name.clone(), response.version))
            }
            IdbCompletionAction::DeleteDatabase { name } => Some((name.clone(), None)),
            _ => None,
        };
        if !response.connection_managed
            && !self.legacy_started
            && let Some((name, version)) = change
        {
            let notified = match &*platform_object(state, self.request)? {
                PlatformObjectRecord::IdbRequest(request) => request.versionchange_notified,
                _ => false,
            };
            let blockers = if notified {
                Vec::new()
            } else {
                state
                    .platform_objects
                    .borrow()
                    .iter()
                    .filter_map(|(id, record)| match record {
                        PlatformObjectRecord::IdbDatabase(database)
                            if database.name == name && !database.closed =>
                        {
                            Some((*id, database.version))
                        }
                        _ => None,
                    })
                    .collect()
            };
            if let PlatformObjectRecord::IdbRequest(request) =
                &mut *platform_object_mut(state, self.request)?
            {
                request.versionchange_notified = true;
            }
            self.legacy_started = true;
            for (id, old) in blockers {
                self.steps.extend([
                    Step::Event(id, "versionchange", Some((old, version))),
                    Step::Checkpoint,
                ]);
            }
            self.steps.push_back(Step::LegacyCheck {
                name,
                version,
                old: response.old_version.unwrap_or(0),
            });
            return Ok(());
        }
        match self.action.clone() {
            IdbCompletionAction::Open { name } => {
                let database_id = if let Some(id) = self.created {
                    id
                } else {
                    let definitions = response
                        .object_store_definitions
                        .iter()
                        .cloned()
                        .map(|definition| (definition.name.clone(), definition))
                        .collect::<BTreeMap<_, _>>();
                    let id = register_platform_object(
                        state,
                        PlatformObjectRecord::IdbDatabase(IdbDatabaseRecord {
                            connection: response.connection_managed.then(|| {
                                format!("{}:connection:{}", state.instance_id, self.request)
                            }),
                            last_versionchange_event: 0,
                            name,
                            version: response.version.unwrap_or(1),
                            old_version: response.old_version.unwrap_or(0),
                            object_stores: response.object_stores.clone(),
                            object_store_definitions: definitions.clone(),
                            upgrade_base_object_stores: response.object_stores.clone(),
                            upgrade_base_definitions: definitions,
                            upgrade_transaction_id: None,
                            closed: false,
                        }),
                    );
                    self.created = Some(id);
                    id
                };
                if response.upgrade_needed && self.transaction.is_none() {
                    let id = register_platform_object(
                        state,
                        PlatformObjectRecord::IdbTransaction(IdbTransactionRecord {
                            request_queue: VecDeque::new(),
                            abort_event_queued: false,
                            settled: false,
                            database_id,
                            request_id: Some(self.request),
                            mode: "versionchange".into(),
                            object_stores: response.object_stores.clone(),
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
                    self.transaction = Some(id);
                }
                if let Some(id) = self.transaction {
                    if let PlatformObjectRecord::IdbDatabase(database) =
                        &mut *platform_object_mut(state, database_id)?
                    {
                        database.upgrade_transaction_id = Some(id);
                    }
                    if let PlatformObjectRecord::IdbRequest(request) =
                        &mut *platform_object_mut(state, self.request)?
                    {
                        request.transaction_id = Some(id);
                    }
                    state.idb_transactions_to_deactivate.borrow_mut().insert(id);
                }
                if self.value.is_none() {
                    let object = new_platform_object(scope, state, "IDBDatabase", database_id)?;
                    self.value = Some(v8::Global::new(scope, v8::Local::<v8::Value>::from(object)));
                }
                self.steps.push_back(Step::Publish);
                if let Some(id) = self.transaction {
                    self.steps.extend([
                        Step::Event(
                            self.request,
                            "upgradeneeded",
                            Some((response.old_version.unwrap_or(0), response.version)),
                        ),
                        Step::AfterUpgrade(id),
                    ]);
                } else {
                    self.steps.extend([
                        Step::Event(self.request, "success", None),
                        Step::Finish(self.request),
                        Step::Flush,
                    ]);
                }
            }
            IdbCompletionAction::DeleteDatabase { .. } => {
                let value = v8::undefined(scope);
                self.value = Some(v8::Global::new(scope, v8::Local::<v8::Value>::from(value)));
                self.steps.extend([
                    Step::Publish,
                    Step::Event(
                        self.request,
                        "success",
                        Some((response.old_version.unwrap_or(0), None)),
                    ),
                    Step::Finish(self.request),
                    Step::Flush,
                ]);
            }
            IdbCompletionAction::OpenFinalized {
                request_id: original,
                database_id,
                transaction_id,
            } => {
                let closed = match &mut *platform_object_mut(state, database_id)? {
                    PlatformObjectRecord::IdbDatabase(database) => {
                        database.upgrade_transaction_id = None;
                        database.closed
                    }
                    _ => true,
                };
                if let PlatformObjectRecord::IdbTransaction(transaction) =
                    &mut *platform_object_mut(state, transaction_id)?
                {
                    transaction.finished = true;
                    transaction.settled = true;
                    transaction.active = false;
                }
                if closed
                    && let PlatformObjectRecord::IdbRequest(request) =
                        &mut *platform_object_mut(state, original)?
                {
                    request.result = None;
                    request.error = Some("AbortError".into());
                }
                self.steps.extend([
                    Step::Event(transaction_id, "complete", None),
                    Step::TerminalAfter(transaction_id, "complete"),
                    Step::Checkpoint,
                    Step::Event(original, if closed { "error" } else { "success" }, None),
                    Step::Finish(original),
                    Step::Finish(self.request),
                    Step::Flush,
                    Step::Poll,
                ]);
            }
            IdbCompletionAction::TransactionFinalized {
                transaction_id,
                event_type,
            } => {
                if let PlatformObjectRecord::IdbTransaction(transaction) =
                    &mut *platform_object_mut(state, transaction_id)?
                {
                    transaction.finished = true;
                    transaction.settled = true;
                    transaction.active = false;
                }
                let value = v8::undefined(scope);
                self.value = Some(v8::Global::new(scope, v8::Local::<v8::Value>::from(value)));
                self.steps.extend([
                    Step::Publish,
                    Step::Event(transaction_id, event_type, None),
                    Step::Finish(self.request),
                    Step::Flush,
                ]);
            }
            action => {
                if self.value.is_none() {
                    let value = self.build_value(scope, state, action, response)?;
                    self.value = Some(v8::Global::new(scope, value));
                }
                self.steps.extend([
                    Step::Publish,
                    Step::Activate(self.request),
                    Step::Event(self.request, "success", None),
                    Step::AfterRequest { error: None },
                    Step::Finish(self.request),
                    Step::Flush,
                ]);
            }
        }
        Ok(())
    }
    fn build_value<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        state: &Rc<V8BindingState>,
        action: IdbCompletionAction,
        response: IdbHostResponse,
    ) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
        let request_id = self.request;
        Ok(match action {
            IdbCompletionAction::Unit => v8::undefined(scope).into(),
            IdbCompletionAction::Value => {
                if response.value_present {
                    idb_json_to_value(
                        scope,
                        state,
                        response.value.as_ref().unwrap_or(&serde_json::Value::Null),
                    )?
                } else {
                    response
                        .records
                        .first()
                        .map(|record| idb_json_to_value(scope, state, &record.value))
                        .transpose()?
                        .unwrap_or_else(|| v8::undefined(scope).into())
                }
            }
            IdbCompletionAction::Key => response
                .key
                .as_ref()
                .or_else(|| response.records.first().map(|record| &record.primary_key))
                .map(|value| idb_json_to_value(scope, state, value))
                .transpose()?
                .unwrap_or_else(|| v8::undefined(scope).into()),
            IdbCompletionAction::Batch {
                count,
                direction,
                kind,
            } => {
                let records = normalize_idb_cursor_records(response.records, &direction);
                let limit = count
                    .filter(|count| *count != 0)
                    .unwrap_or(records.len())
                    .min(records.len());
                let array = v8::Array::new(scope, limit as i32);
                for (index, record) in records.iter().take(limit).enumerate() {
                    if let Some(value) = self.batch.get(index) {
                        let value = v8::Local::new(scope, value);
                        if array.set_index(scope, index as u32, value) != Some(true) {
                            return Err(V8BindingError::Allocation("IDB result array"));
                        }
                        continue;
                    }
                    let value = match kind {
                        IdbBatchKind::Values => idb_json_to_value(scope, state, &record.value)?,
                        IdbBatchKind::Keys => idb_json_to_value(scope, state, &record.primary_key)?,
                        IdbBatchKind::Records => {
                            let key = idb_json_to_value(scope, state, &record.key)?;
                            let primary_key = idb_json_to_value(scope, state, &record.primary_key)?;
                            let value = idb_json_to_value(scope, state, &record.value)?;
                            let id = if let Some(id) = self.created {
                                id
                            } else {
                                let id = register_platform_object(
                                    state,
                                    PlatformObjectRecord::IdbRecord {
                                        key: platform_listener_graph::Value::new(scope, key),
                                        primary_key: platform_listener_graph::Value::new(
                                            scope,
                                            primary_key,
                                        ),
                                        value: platform_listener_graph::Value::new(scope, value),
                                    },
                                );
                                self.created = Some(id);
                                id
                            };
                            let value = new_platform_object(scope, state, "IDBRecord", id)?;
                            self.created = None;
                            value.into()
                        }
                    };
                    self.batch.push(v8::Global::new(scope, value));
                    if array.set_index(scope, index as u32, value) != Some(true) {
                        return Err(V8BindingError::Allocation("IDB result array"));
                    }
                }
                array.into()
            }
            IdbCompletionAction::Count => v8::Number::new(
                scope,
                response.count.unwrap_or(response.records.len() as u64) as f64,
            )
            .into(),
            IdbCompletionAction::CursorRefresh { cursor_id } => {
                let (direction, with_value) = {
                    let record = platform_object(state, cursor_id)?;
                    let PlatformObjectRecord::IdbCursor(cursor) = &*record else {
                        return Err(V8BindingError::Allocation("IDBCursor record"));
                    };
                    (cursor.direction.clone(), cursor.with_value)
                };
                let records = normalize_idb_cursor_records(response.records, &direction);
                let next_index = (!records.is_empty()).then_some(0);
                {
                    let mut record = platform_object_mut(state, cursor_id)?;
                    let PlatformObjectRecord::IdbCursor(cursor) = &mut *record else {
                        return Err(V8BindingError::Allocation("IDBCursor record"));
                    };
                    cursor.records = records;
                    cursor.index = next_index.unwrap_or(cursor.records.len());
                    cursor.got_value = next_index.is_some();
                    cursor.iteration_pending = false;
                    cursor.key_value = None;
                    cursor.primary_key_value = None;
                    cursor.record_value = None;
                }
                match next_index {
                    Some(_) => new_platform_object(
                        scope,
                        state,
                        if with_value {
                            "IDBCursorWithValue"
                        } else {
                            "IDBCursor"
                        },
                        cursor_id,
                    )?
                    .into(),
                    None => v8::null(scope).into(),
                }
            }
            IdbCompletionAction::Cursor {
                source_id,
                direction,
                query,
                with_value,
            } => {
                let records = normalize_idb_cursor_records(response.records, &direction);
                if records.is_empty() {
                    v8::null(scope).into()
                } else {
                    let cursor_id = if let Some(id) = self.created {
                        id
                    } else {
                        let id = register_platform_object(
                            state,
                            PlatformObjectRecord::IdbCursor(IdbCursorRecord {
                                request_id,
                                source_id,
                                direction,
                                query,
                                records,
                                index: 0,
                                got_value: true,
                                iteration_pending: false,
                                with_value,
                                key_value: None,
                                primary_key_value: None,
                                record_value: None,
                            }),
                        );
                        self.created = Some(id);
                        id
                    };
                    new_platform_object(
                        scope,
                        state,
                        if with_value {
                            "IDBCursorWithValue"
                        } else {
                            "IDBCursor"
                        },
                        cursor_id,
                    )?
                    .into()
                }
            }
            _ => return Err(V8BindingError::Allocation("unexpected IDB value action")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
        let code = v8::String::new(scope, code).unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    fn records(scope: &mut v8::PinScope, state: &Rc<V8BindingState>) -> (u64, u64, u64) {
        let database = register_platform_object(
            state,
            PlatformObjectRecord::IdbDatabase(IdbDatabaseRecord {
                connection: None,
                last_versionchange_event: 0,
                name: "db".into(),
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
            state,
            PlatformObjectRecord::IdbTransaction(IdbTransactionRecord {
                request_queue: VecDeque::new(),
                abort_event_queued: false,
                settled: false,
                database_id: database,
                request_id: None,
                mode: "readwrite".into(),
                object_stores: vec!["records".into()],
                aborted: false,
                error: None,
                pending_requests: 2,
                completion_requested: false,
                completion_queued: false,
                explicit_commit: false,
                active: true,
                finished: false,
            }),
        );
        let request = new_idb_request(state, Some(transaction));
        for (name, interface, id) in [
            ("request", "IDBRequest", request),
            ("tx", "IDBTransaction", transaction),
            ("db", "IDBDatabase", database),
        ] {
            let value = new_platform_object(scope, state, interface, id).unwrap();
            let key = v8::String::new(scope, name).unwrap();
            scope
                .get_current_context()
                .global(scope)
                .set(scope, key.into(), value.into())
                .unwrap();
        }
        (request, transaction, database)
    }
    struct AckHost {
        reject: Cell<bool>,
        events: RefCell<Vec<u64>>,
    }
    impl PlatformHost for AckHost {
        fn submit(&self, _: HostOperationId, request: PlatformHostRequest) -> Result<(), String> {
            if let PlatformHostRequest::IndexedDb {
                command: IdbHostCommand::AcknowledgeConnection { event, .. },
                ..
            } = request
            {
                if self.reject.replace(false) {
                    return Err("injected acknowledgement rejection".into());
                }
                self.events.borrow_mut().push(event);
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
    fn versionchange_retains_listener_suffix_and_ack_without_repeating_callback() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(120), 1).unwrap();
        let (_, _, database) = records(scope, &state);
        if let PlatformObjectRecord::IdbDatabase(record) =
            &mut *platform_object_mut(&state, database).unwrap()
        {
            record.connection = Some("owned-connection".into());
        }
        let host = Rc::new(AckHost {
            reject: Cell::new(true),
            events: RefCell::new(Vec::new()),
        });
        *state.platform_host.borrow_mut() = Some(host.clone());
        eval(
            scope,
            "globalThis.calls=[];db.addEventListener('versionchange',e=>calls.push('first:'+e.newVersion));db.addEventListener('versionchange',()=>calls.push('second'))",
        );
        let response = IdbHostResponse {
            connection_events: vec![crate::platform::IdbConnectionEvent {
                event: 1,
                old_version: 1,
                new_version: Some(2),
            }],
            ..Default::default()
        };
        connection_events(&state, database, response.clone());
        publication::fail_after_next_callback();
        assert!(drain(scope, &state).is_err());
        assert!(eval(scope, "calls.join()==='first:2'").is_true());
        assert!(host.events.borrow().is_empty());
        assert!(drain(scope, &state).is_err());
        assert!(eval(scope, "calls.join()==='first:2,second'").is_true());
        drain(scope, &state).unwrap();
        assert_eq!(*host.events.borrow(), vec![1]);
        connection_events(&state, database, response);
        drain(scope, &state).unwrap();
        assert!(eval(scope, "calls.join()==='first:2,second'").is_true());
        assert_eq!(*host.events.borrow(), vec![1, 1]);
    }
    #[test]
    fn database_enumeration_is_retained_until_resume_and_drops_on_document_replacement() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(121), 1).unwrap();
        let resolver = v8::PromiseResolver::new(scope).unwrap();
        let promise = resolver.get_promise(scope);
        databases(
            &state,
            v8::Global::new(scope, resolver),
            Ok(IdbHostResponse::default()),
        );
        assert_eq!(promise.state(), v8::PromiseState::Pending);
        drain(scope, &state).unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Fulfilled);
        assert!(promise.result(scope).is_array());
        let resolver = v8::PromiseResolver::new(scope).unwrap();
        let promise = resolver.get_promise(scope);
        databases(
            &state,
            v8::Global::new(scope, resolver),
            Err("SecurityError".into()),
        );
        state
            .document_generation
            .set(state.document_generation() + 1);
        drain(scope, &state).unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Pending);
        assert!(!pending(&state));
    }
    #[test]
    fn finalization_native_admission_retries_same_hidden_request_once() {
        struct Host {
            reject: Cell<bool>,
            commits: Cell<usize>,
        }
        impl PlatformHost for Host {
            fn submit(
                &self,
                _: HostOperationId,
                request: PlatformHostRequest,
            ) -> Result<(), String> {
                if self.reject.replace(false) {
                    return Err("injected finalize admission".into());
                }
                if matches!(
                    request,
                    PlatformHostRequest::IndexedDb {
                        command: IdbHostCommand::CommitTransaction { .. },
                        ..
                    }
                ) {
                    self.commits.set(self.commits.get() + 1);
                }
                Ok(())
            }
            fn cancel(&self, _: HostOperationId) {}
            fn retire(&self) {}
            fn drain_completions(&self, _: usize) -> Vec<PlatformHostCompletion> {
                Vec::new()
            }
        }
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(122), 1).unwrap();
        let (_, transaction, _) = records(scope, &state);
        *state.security_origin.borrow_mut() = Some("https://db.example".into());
        let host = Rc::new(Host {
            reject: Cell::new(true),
            commits: Cell::new(0),
        });
        *state.platform_host.borrow_mut() = Some(host.clone());
        finalize(&state, transaction, None, true);
        assert!(drain(scope, &state).is_err());
        assert_eq!(state.queued_idb_operations.borrow().len(), 1);
        let hidden = state
            .idb_deliveries
            .queue
            .borrow()
            .front()
            .unwrap()
            .cleanup
            .unwrap();
        drain(scope, &state).unwrap();
        assert!(!pending(&state));
        assert_eq!(host.commits.get(), 1);
        assert!(state.pending_platform_operations.borrow().values().any(|operation|matches!(operation,PendingPlatformOperation::IndexedDb{request_id,..}if *request_id==hidden)));
        assert!(state.queued_idb_operations.borrow().is_empty());
    }
    #[test]
    fn result_event_and_accounting_failures_resume_without_repeating_author_prefix() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(114), 1).unwrap();
        let (request, transaction, _) = records(scope, &state);
        eval(
            scope,
            "globalThis.calls=[];globalThis.saved=null;request.addEventListener('success',e=>{calls.push('first');saved=e},{once:true});request.addEventListener('success',e=>calls.push(e===saved?'second':'changed'))",
        );
        enqueue(
            &state,
            request,
            IdbCompletionAction::Value,
            Ok(IdbHostResponse {
                value_present: true,
                value: Some(serde_json::json!({"answer":42})),
                ..Default::default()
            }),
        );
        FAIL_VALUE_ONCE.with(|flag| flag.set(true));
        assert!(drain(scope, &state).is_err());
        assert!(pending(&state));
        assert!(eval(scope, "calls.length===0").is_true());
        publication::fail_before_next_prepare();
        assert!(drain(scope, &state).is_err());
        assert!(eval(scope, "calls.length===0").is_true());
        publication::fail_after_next_callback();
        assert!(drain(scope, &state).is_err());
        assert!(eval(scope, "calls.join()==='first'&&request.result.answer===42").is_true());
        FAIL_AFTER_ACCOUNT_ONCE.with(|flag| flag.set(true));
        assert!(drain(scope, &state).is_err());
        assert!(eval(scope, "calls.join()==='first,second'").is_true());
        let count = || match &*platform_object(&state, transaction).unwrap() {
            PlatformObjectRecord::IdbTransaction(transaction) => transaction.pending_requests,
            _ => panic!(),
        };
        assert_eq!(count(), 1);
        drain(scope, &state).unwrap();
        assert_eq!(count(), 1);
        assert!(!pending(&state));
        assert!(eval(scope, "calls.join()==='first,second'&&saved.eventPhase===0").is_true());
    }
    #[test]
    fn error_path_preserves_parent_capture_bubbling_and_prevent_default_after_retry() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(115), 1).unwrap();
        let (request, transaction, _) = records(scope, &state);
        eval(
            scope,
            "globalThis.calls=[];db.addEventListener('error',()=>calls.push('capture'),true);request.addEventListener('error',e=>{calls.push('request');e.preventDefault()});tx.addEventListener('error',()=>calls.push('transaction'));db.addEventListener('error',()=>calls.push('database'))",
        );
        enqueue(
            &state,
            request,
            IdbCompletionAction::Unit,
            Err("ConstraintError".into()),
        );
        publication::fail_after_next_callback();
        assert!(drain(scope, &state).is_err());
        assert!(eval(scope, "calls.join()==='capture'").is_true());
        drain(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "calls.join()==='capture,request,transaction,database'"
            )
            .is_true()
        );
        let record = platform_object(&state, transaction).unwrap();
        let PlatformObjectRecord::IdbTransaction(transaction) = &*record else {
            panic!()
        };
        assert!(!transaction.aborted);
        assert_eq!(transaction.pending_requests, 1);
    }
    #[test]
    fn thrown_success_callback_and_reentrant_abort_keep_remaining_listener_suffix() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(116), 1).unwrap();
        let (request, transaction, _) = records(scope, &state);
        eval(
            scope,
            "globalThis.calls=[];request.addEventListener('success',()=>{calls.push('first');tx.abort();throw Error('author failure')});request.addEventListener('success',()=>calls.push('second'))",
        );
        enqueue(
            &state,
            request,
            IdbCompletionAction::Unit,
            Ok(IdbHostResponse::default()),
        );
        publication::fail_after_next_callback();
        assert!(drain(scope, &state).is_err());
        drain(scope, &state).unwrap();
        assert!(eval(scope, "calls.join()==='first,second'").is_true());
        let record = platform_object(&state, transaction).unwrap();
        let PlatformObjectRecord::IdbTransaction(transaction) = &*record else {
            panic!()
        };
        assert!(transaction.aborted);
        assert!(transaction.abort_event_queued);
    }
    #[test]
    fn deferred_error_and_document_retirement_never_replay_old_callbacks() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(117), 1).unwrap();
        let (request, _, _) = records(scope, &state);
        eval(
            scope,
            "globalThis.calls=[];request.addEventListener('error',()=>calls.push('first'));request.addEventListener('error',()=>calls.push('second'))",
        );
        deferred_failure(&state, request, true);
        publication::fail_after_next_callback();
        assert!(drain(scope, &state).is_err());
        assert!(eval(scope, "calls.join()==='first'").is_true());
        state
            .document_generation
            .set(state.document_generation() + 1);
        drain(scope, &state).unwrap();
        assert!(!pending(&state));
        assert!(eval(scope, "calls.join()==='first'").is_true());
    }
}
