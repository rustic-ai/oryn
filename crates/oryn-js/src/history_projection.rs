//! Parent-visible history count changes reserve an unknown projection before
//! local commit, then release author publication only after the count ACK.
use super::*;
pub trait NativeHistoryPublisher {
    fn begin(
        &self,
        creator: &crate::worker_wire::Creator,
        revision: u64,
        count: usize,
    ) -> Result<(), String>;
    fn publish(
        &self,
        creator: &crate::worker_wire::Creator,
        revision: u64,
        count: usize,
    ) -> Result<(), String>;
    fn abort(
        &self,
        creator: &crate::worker_wire::Creator,
        revision: u64,
        count: usize,
    ) -> Result<(), String>;
}
#[derive(Clone, Copy)]
enum Action {
    Abort,
    Publish,
}
#[derive(Clone)]
pub(super) struct Pending {
    creator: crate::worker_wire::Creator,
    revision: u64,
    count: usize,
    action: Action,
    backend: Rc<dyn NativeHistoryPublisher>,
}
#[derive(Default)]
pub(super) struct State {
    pub publisher: Option<Rc<dyn NativeHistoryPublisher>>,
    revision: u64,
    pub(super) pending: Option<Pending>,
}
pub(super) fn pending(state: &V8BindingState) -> bool {
    state
        .navigation_api
        .borrow()
        .history_projection
        .pending
        .is_some()
}
fn retry(state: &V8BindingState) {
    state.navigation_api.borrow_mut().publication_retry_at =
        Some(std::time::Instant::now() + std::time::Duration::from_millis(25));
}
pub(super) fn flush(state: &V8BindingState) -> Result<(), V8BindingError> {
    let pending = state
        .navigation_api
        .borrow()
        .history_projection
        .pending
        .clone();
    let Some(pending) = pending else {
        return Ok(());
    };
    if state.execution_retired()
        || matches!(&pending.creator,crate::worker_wire::Creator::Window{document_generation,stream_epoch,..}if *document_generation!=state.document_generation()||*stream_epoch!=state.document_authority_epoch())
    {
        state.navigation_api.borrow_mut().history_projection.pending = None;
        return Ok(());
    }
    let result = match pending.action {
        Action::Abort => pending
            .backend
            .abort(&pending.creator, pending.revision, pending.count),
        Action::Publish => {
            pending
                .backend
                .publish(&pending.creator, pending.revision, pending.count)
        }
    };
    if let Err(error) = result {
        retry(state);
        return Err(V8BindingError::Message(error));
    }
    let mut navigation = state.navigation_api.borrow_mut();
    if navigation
        .history_projection
        .pending
        .as_ref()
        .is_some_and(|entry| entry.revision == pending.revision)
    {
        navigation.history_projection.pending = None;
    }
    Ok(())
}
pub(crate) struct Reservation {
    state: Rc<V8BindingState>,
    committed: bool,
}
impl Reservation {
    pub fn commit(mut self) -> Result<(), V8BindingError> {
        self.committed = true;
        if let Some(pending) = self
            .state
            .navigation_api
            .borrow_mut()
            .history_projection
            .pending
            .as_mut()
        {
            pending.action = Action::Publish;
        }
        flush(&self.state)
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.committed {
            let _ = flush(&self.state);
        }
    }
}
pub(super) fn prepare(
    state: &Rc<V8BindingState>,
    count: usize,
) -> Result<Option<Reservation>, V8BindingError> {
    flush(state)?;
    let backend = state
        .navigation_api
        .borrow()
        .history_projection
        .publisher
        .clone();
    let Some(backend) = backend else {
        return Ok(None);
    };
    if state.document_prerendering() {
        return Ok(None);
    }
    let creator = state.native_worker_creator()?;
    if !matches!(
        &creator,
        crate::worker_wire::Creator::Window {
            target: crate::message_wire::WindowTarget::Root,
            ..
        }
    ) {
        return Ok(None);
    }
    if count == 0 || count > 4096 {
        return Err(V8BindingError::Message(
            "invalid full session history count".into(),
        ));
    }
    let revision = {
        let mut navigation = state.navigation_api.borrow_mut();
        let projection = &mut navigation.history_projection;
        let revision = projection.revision.checked_add(1).ok_or_else(|| {
            V8BindingError::Message("history projection revision exhausted".into())
        })?;
        projection.revision = revision;
        projection.pending = Some(Pending {
            creator: creator.clone(),
            revision,
            count,
            action: Action::Abort,
            backend: backend.clone(),
        });
        revision
    };
    if let Err(error) = backend.begin(&creator, revision, count) {
        retry(state);
        return Err(V8BindingError::Message(error));
    }
    Ok(Some(Reservation {
        state: state.clone(),
        committed: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Backend {
        known: Cell<Option<usize>>,
        previous: Cell<Option<usize>>,
        fail_begin: Cell<bool>,
        fail_publish: Cell<bool>,
        calls: RefCell<Vec<(&'static str, u64, usize)>>,
    }
    impl NativeHistoryPublisher for Backend {
        fn begin(
            &self,
            _: &crate::worker_wire::Creator,
            revision: u64,
            count: usize,
        ) -> Result<(), String> {
            self.calls.borrow_mut().push(("begin", revision, count));
            self.previous.set(self.known.replace(None));
            if self.fail_begin.replace(false) {
                Err("lost begin ACK".into())
            } else {
                Ok(())
            }
        }
        fn publish(
            &self,
            _: &crate::worker_wire::Creator,
            revision: u64,
            count: usize,
        ) -> Result<(), String> {
            self.calls.borrow_mut().push(("publish", revision, count));
            if self.fail_publish.get() {
                return Err("lost publish ACK".into());
            }
            self.known.set(Some(count));
            Ok(())
        }
        fn abort(
            &self,
            _: &crate::worker_wire::Creator,
            revision: u64,
            count: usize,
        ) -> Result<(), String> {
            self.calls.borrow_mut().push(("abort", revision, count));
            self.known.set(self.previous.get());
            Ok(())
        }
    }
    struct Messages;
    impl crate::message_wire::MessageTransport for Messages {
        fn request(
            &self,
            _: crate::message_wire::MessageRequest,
        ) -> Result<crate::message_wire::MessageResponse, String> {
            Ok(crate::message_wire::MessageResponse::Done)
        }
    }
    fn entry(id: &str) -> NavigationHistorySnapshot {
        NavigationHistorySnapshot {
            scroll: Default::default(),
            key: id.into(),
            id: id.into(),
            url: format!("https://example.test/{id}"),
            document_id: "one".into(),
            state: None,
            history_state: None,
        }
    }
    fn eval(scope: &mut v8::PinScope, source: &str) -> String {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_string(scope)
            .unwrap()
            .to_rust_string_lossy(scope)
    }
    #[test]
    fn failed_publish_blocks_author_notifications_until_exact_count_ack() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(std::sync::Arc::new(Messages));
        let backend = Rc::new(Backend::default());
        state.set_native_history_publisher(backend.clone());
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        assert_eq!(backend.known.get(), Some(1));
        eval(
            scope,
            "globalThis.changes=0;navigation.oncurrententrychange=()=>changes++",
        );
        backend.fail_publish.set(true);
        assert!(sync_history(scope, vec![entry("a"), entry("b")], 1, "push").is_err());
        assert!(!state.native_history_has_single_entry());
        assert_eq!(backend.known.get(), None);
        assert_eq!(eval(scope, "String(changes)"), "0");
        assert!(pending(&state));
        backend.fail_publish.set(false);
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state).unwrap();
        assert_eq!(backend.known.get(), Some(2));
        assert_eq!(eval(scope, "String(changes)"), "1");
        assert!(!pending(&state));
    }
    #[test]
    fn lost_begin_ack_aborts_without_committing_proposed_history() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(std::sync::Arc::new(Messages));
        let backend = Rc::new(Backend::default());
        state.set_native_history_publisher(backend.clone());
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        backend.fail_begin.set(true);
        assert!(sync_history(scope, vec![entry("a"), entry("b")], 1, "push").is_err());
        assert!(state.native_history_has_single_entry());
        assert_eq!(backend.known.get(), None);
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state).unwrap();
        assert_eq!(backend.known.get(), Some(1));
        assert!(
            backend
                .calls
                .borrow()
                .iter()
                .any(|(kind, _, count)| *kind == "abort" && *count == 2)
        );
        assert!(
            !backend
                .calls
                .borrow()
                .iter()
                .any(|(kind, _, count)| *kind == "publish" && *count == 2)
        );
    }
}
