//! Native PiP ownership and retained event/promise publication.
use super::native_platform::publication;
use super::*;
use crate::media_capture::{Request, Response};
struct Session {
    node: NodeId,
    asset: String,
    window: v8::Global<v8::Object>,
    window_id: u64,
    size: Rc<Cell<(u32, u32)>>,
    epoch: u64,
}
struct Task {
    node: NodeId,
    name: &'static str,
    window: v8::Global<v8::Object>,
    window_id: u64,
    resolver: Option<v8::Global<v8::PromiseResolver>>,
    event: Option<v8::Global<v8::Object>>,
    dispatch: Option<publication::Notification>,
    epoch: u64,
}
#[derive(Default)]
pub(super) struct State {
    session: Option<Session>,
    tasks: VecDeque<Task>,
    cleanup: BTreeSet<String>,
    polling: bool,
    retry: Option<Instant>,
    next_poll: Option<Instant>,
}
fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "PictureInPictureWindow".into(),
        member: name.into(),
    }
}
pub(super) fn enabled(state: &V8BindingState) -> bool {
    state.binding_global == BindingGlobal::Window
        && !state.execution_retired()
        && state.native_permissions_policy_allows("picture-in-picture")
}
pub(super) fn element(
    state: &V8BindingState,
    root: NodeId,
) -> Result<Option<NodeId>, V8BindingError> {
    let Some(node) = state
        .picture_in_picture
        .borrow()
        .session
        .as_ref()
        .map(|session| session.node)
    else {
        return Ok(None);
    };
    let dom = state.dom.borrow();
    let node = dom.retarget_node(node, Some(root))?;
    Ok((dom.control_tree_root(node)? == root).then_some(node))
}
fn reserve(state: &V8BindingState, count: usize) -> Result<(), V8BindingError> {
    let mut pip = state.picture_in_picture.borrow_mut();
    if pip.tasks.len() + count > 64 {
        return Err(error("QuotaExceededError"));
    }
    pip.tasks
        .try_reserve(count)
        .map_err(|_| V8BindingError::Allocation("PiP task"))
}
fn close(
    state: &V8BindingState,
    session: Session,
    resolver: Option<v8::Global<v8::PromiseResolver>>,
) {
    match native_platform::media_request(
        state,
        Request::UrlPictureInPicture {
            id: session.asset.clone(),
            enabled: false,
        },
    ) {
        Ok(_) => {}
        Err(_) => {
            state
                .picture_in_picture
                .borrow_mut()
                .cleanup
                .insert(session.asset);
        }
    }
    session.size.set((0, 0));
    state.picture_in_picture.borrow_mut().tasks.push_back(Task {
        node: session.node,
        name: "leavepictureinpicture",
        window: session.window,
        window_id: session.window_id,
        resolver,
        event: None,
        dispatch: None,
        epoch: session.epoch,
    });
}
pub(super) fn invalidate(state: &V8BindingState, node: NodeId) {
    let session = {
        let mut pip = state.picture_in_picture.borrow_mut();
        if pip
            .session
            .as_ref()
            .is_some_and(|session| session.node == node)
        {
            pip.session.take()
        } else {
            None
        }
    };
    if let Some(session) = session {
        close(state, session, None)
    }
}
pub(super) fn promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    exit: bool,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let resolver =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("PiP promise"))?;
    let promise = resolver.get_promise(scope);
    let result = (|| -> Result<(), V8BindingError> {
        if state.execution_retired()
            || state.execution_suspended()
            || state.binding_global != BindingGlobal::Window
        {
            return Err(error("InvalidStateError"));
        }
        reserve(state, 2)?;
        if exit {
            if node != state.document_root() {
                return Err(error("InvalidStateError"));
            }
            let session = state
                .picture_in_picture
                .borrow_mut()
                .session
                .take()
                .ok_or(error("InvalidStateError"))?;
            close(state, session, Some(v8::Global::new(scope, resolver)));
            return Ok(());
        }
        state.refresh_native_container_policy()?;
        if !state.native_permissions_report_only_allows("picture-in-picture") {
            reporting::permission_native(state, "picture-in-picture", "report");
        }
        if !enabled(state) {
            reporting::permission_native(state, "picture-in-picture", "enforce");
            return Err(error("SecurityError"));
        }
        let asset = media_element::picture_in_picture_asset(scope, state, node)?;
        if state
            .dom
            .borrow()
            .attribute_in_null_namespace(node, "disablepictureinpicture")?
            .is_some()
        {
            return Err(error("InvalidStateError"));
        }
        let existing_window = {
            let pip = state.picture_in_picture.borrow();
            pip.session
                .as_ref()
                .filter(|session| session.node == node)
                .map(|session| v8::Local::new(scope, &session.window))
        };
        if let Some(window) = existing_window {
            resolver
                .resolve(scope, window.into())
                .ok_or(V8BindingError::JavaScriptException)?;
            return Ok(());
        }
        if state.picture_in_picture.borrow().session.is_none()
            && !state.has_transient_user_activation()
        {
            return Err(error("NotAllowedError"));
        }
        let size = Rc::new(Cell::new((0, 0)));
        let id = register_platform_object(
            state,
            PlatformObjectRecord::PictureInPictureWindow(size.clone()),
        );
        let window = new_platform_object(scope, state, "PictureInPictureWindow", id)?;
        let global = v8::Global::new(scope, window);
        let pending = v8::Global::new(scope, resolver);
        let response = native_platform::media_request(
            state,
            Request::UrlPictureInPicture {
                id: asset.clone(),
                enabled: true,
            },
        )?;
        let Response::UrlPictureInPicture(Some(dimensions)) = response else {
            return Err(error("NotSupportedError"));
        };
        size.set(dimensions);
        state.consume_transient_user_activation();
        let previous = state.picture_in_picture.borrow_mut().session.take();
        if let Some(previous) = previous {
            close(state, previous, None)
        }
        let epoch = state.document_stream_epoch();
        state.picture_in_picture.borrow_mut().session = Some(Session {
            node,
            asset,
            window: global.clone(),
            window_id: id,
            size,
            epoch,
        });
        state.picture_in_picture.borrow_mut().tasks.push_back(Task {
            node,
            name: "enterpictureinpicture",
            window: global,
            window_id: id,
            resolver: Some(pending),
            event: None,
            dispatch: None,
            epoch,
        });
        media_element::picture_in_picture_refresh(state, node);
        Ok(())
    })();
    if let Err(failure) = result {
        let name = match &failure {
            V8BindingError::Unsupported { member, .. } => {
                member.split(':').next().unwrap_or("NotSupportedError")
            }
            _ => "NotSupportedError",
        };
        let value = dom_exception_value(scope, name)?;
        resolver
            .reject(scope, value)
            .ok_or(V8BindingError::JavaScriptException)?;
    }
    Ok(promise.into())
}
pub(super) fn delay(state: &V8BindingState) -> Option<Duration> {
    if state.execution_retired() || state.execution_suspended() {
        return None;
    }
    let pip = state.picture_in_picture.borrow();
    (!pip.tasks.is_empty() || !pip.cleanup.is_empty() || pip.session.is_some()).then(|| {
        pip.retry.map_or(
            if pip.tasks.is_empty() {
                pip.next_poll.map_or(Duration::ZERO, |deadline| {
                    deadline.saturating_duration_since(Instant::now())
                })
            } else {
                Duration::ZERO
            },
            |deadline| deadline.saturating_duration_since(Instant::now()),
        )
    })
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.execution_retired()
        || state.execution_suspended()
        || state.picture_in_picture.borrow().polling
    {
        return Ok(0);
    }
    if state
        .picture_in_picture
        .borrow()
        .retry
        .is_some_and(|deadline| deadline > Instant::now())
    {
        return Ok(0);
    }
    let cleanup = state
        .picture_in_picture
        .borrow()
        .cleanup
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    for id in cleanup {
        let result = native_platform::media_request(
            state,
            Request::UrlPictureInPicture {
                id: id.clone(),
                enabled: false,
            },
        );
        if result.is_ok()
            || matches!(&result,Err(V8BindingError::Unsupported{member,..})if member.starts_with("InvalidStateError"))
        {
            state.picture_in_picture.borrow_mut().cleanup.remove(&id);
        }
    }
    let active = state
        .picture_in_picture
        .borrow()
        .session
        .as_ref()
        .map(|session| {
            (
                session.node,
                session.asset.clone(),
                session.size.clone(),
                session.epoch,
            )
        });
    if let Some((node, asset, size, epoch)) = active {
        if epoch != state.document_stream_epoch()
            || media_element::picture_in_picture_asset(scope, state, node)
                .ok()
                .as_deref()
                != Some(asset.as_str())
        {
            invalidate(state, node)
        } else {
            match native_platform::media_request(
                state,
                Request::UrlPictureInPictureState { id: asset },
            ) {
                Ok(Response::UrlPictureInPicture(None)) => invalidate(state, node),
                Ok(Response::UrlPictureInPicture(Some(dimensions))) if size.get() != dimensions => {
                    reserve(state, 1)?;
                    size.set(dimensions);
                    let mut pip = state.picture_in_picture.borrow_mut();
                    if let Some(session) = &pip.session {
                        let task = Task {
                            node,
                            name: "resize",
                            window: session.window.clone(),
                            window_id: session.window_id,
                            resolver: None,
                            event: None,
                            dispatch: None,
                            epoch,
                        };
                        pip.tasks.push_back(task);
                    }
                }
                _ => {}
            }
        }
    }
    state.picture_in_picture.borrow_mut().next_poll =
        Some(Instant::now() + Duration::from_millis(50));
    let Some(mut task) = state.picture_in_picture.borrow_mut().tasks.pop_front() else {
        return Ok(0);
    };
    state.picture_in_picture.borrow_mut().polling = true;
    struct Guard<'a>(&'a V8BindingState);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.picture_in_picture.borrow_mut().polling = false;
        }
    }
    let _guard = Guard(state);
    let result = (|| -> Result<bool, V8BindingError> {
        if task.epoch != state.document_stream_epoch() {
            if let Some(dispatch) = &task.dispatch {
                dispatch.cancel(state);
            }
            if let Some(resolver) = &task.resolver {
                let resolver = v8::Local::new(scope, resolver);
                let reason = dom_exception_value(scope, "AbortError")?;
                resolver
                    .reject(scope, reason)
                    .ok_or(V8BindingError::JavaScriptException)?;
            }
            return Ok(true);
        }
        if task.event.is_none() {
            let event = create_event_object_with_interface(
                scope,
                state,
                if task.name == "resize" {
                    "Event"
                } else {
                    "PictureInPictureEvent"
                },
                task.name,
                false,
                false,
            )?;
            let id = event_id(scope, event)?;
            {
                let mut events = state.events.borrow_mut();
                let record = events
                    .get_mut(&id)
                    .ok_or(V8BindingError::Allocation("PiP event"))?;
                record.is_trusted = true;
                record.picture_in_picture_window = Some(task.window.clone());
            }
            task.event = Some(v8::Global::new(scope, event));
        }
        let event = v8::Local::new(scope, task.event.as_ref().unwrap());
        if task.dispatch.is_none() {
            task.dispatch = Some(if task.name == "resize" {
                publication::pending_platform_dispatch(
                    scope,
                    state,
                    task.window_id,
                    task.name,
                    event,
                )
            } else {
                let node = wrap_node(scope, state, task.node)?;
                publication::pending_dom_dispatch(scope, state, node, task.name, event)
            });
        }
        let dispatch = task.dispatch.as_mut().unwrap();
        if !dispatch.resume(scope, state)? {
            return Ok(false);
        }
        dispatch.finish(scope, state);
        if let Some(resolver) = &task.resolver {
            let resolver = v8::Local::new(scope, resolver);
            let value = if task.name == "enterpictureinpicture" {
                v8::Local::new(scope, &task.window).into()
            } else {
                v8::undefined(scope).into()
            };
            resolver
                .resolve(scope, value)
                .ok_or(V8BindingError::JavaScriptException)?;
        }
        Ok(true)
    })();
    match result {
        Ok(true) => {
            state.picture_in_picture.borrow_mut().retry = None;
            Ok(1)
        }
        result => {
            let mut pip = state.picture_in_picture.borrow_mut();
            pip.tasks.push_front(task);
            pip.retry = Some(Instant::now() + Duration::from_millis(10));
            result.map(|_| 0)
        }
    }
}
pub(super) fn retire(state: &V8BindingState) {
    let mut old = std::mem::take(&mut *state.picture_in_picture.borrow_mut());
    if let Some(session) = old.session.take() {
        let _ = native_platform::media_request(
            state,
            Request::UrlPictureInPicture {
                id: session.asset,
                enabled: false,
            },
        );
    }
    for task in old.tasks {
        if let Some(dispatch) = task.dispatch {
            dispatch.cancel(state);
        }
    }
}
pub(super) fn initialize_event(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    event: v8::Local<v8::Object>,
    init: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let init = init.to_object(scope).ok_or(error("TypeError"))?;
    let key = v8_string(scope, "pictureInPictureWindow")?;
    let value = init
        .get(scope, key)
        .ok_or(V8BindingError::JavaScriptException)?;
    let window = v8::Local::<v8::Object>::try_from(value).map_err(|_| error("TypeError"))?;
    let owner = binding_state_for_receiver(scope, window).ok_or(error("TypeError"))?;
    let id = stateful_id(scope, window)?;
    if !matches!(
        &*platform_object(&owner, id)?,
        PlatformObjectRecord::PictureInPictureWindow(_)
    ) {
        return Err(error("TypeError"));
    }
    let event_id = event_id(scope, event)?;
    state
        .events
        .borrow_mut()
        .get_mut(&event_id)
        .ok_or(error("TypeError"))?
        .picture_in_picture_window = Some(v8::Global::new(scope, window));
    Ok(())
}
pub(super) fn event_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let owner = binding_state_for_receiver(scope, event).ok_or(error("TypeError"))?;
    let id = event_id(scope, event)?;
    let value = owner
        .events
        .borrow()
        .get(&id)
        .and_then(|event| event.picture_in_picture_window.clone())
        .ok_or(error("TypeError"))?;
    Ok(v8::Local::new(scope, value).into())
}

#[cfg(test)]
mod tests {
    use super::super::native_user_activation::test_transport::Messages;
    use super::*;
    use std::sync::{Arc, Mutex};
    fn register(state: &V8BindingState) -> Arc<Messages> {
        let messages = Arc::new(Messages::new(1, 0));
        state.set_message_transport(messages.clone());
        state.native_worker_creator().unwrap();
        messages.assert_lifecycle(0);
        messages
    }

    #[derive(Default)]
    struct Backend {
        open: Mutex<bool>,
        size: Mutex<(u32, u32)>,
        commands: Mutex<Vec<bool>>,
    }
    impl crate::platform_control::Backend for Backend {
        fn request(
            &self,
            creator: crate::worker_wire::Creator,
            request: crate::platform_control::Request,
        ) -> Result<crate::platform_control::Response, String> {
            if creator
                != (crate::worker_wire::Creator::Window {
                    target: crate::message_wire::WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                })
            {
                return Err("foreign PiP creator".into());
            }
            let crate::platform_control::Request::Media { request } = request else {
                return Err("unexpected PiP request".into());
            };
            let response = match request {
                Request::UrlSample { .. } => Response::UrlFrame(
                    serde_json::json!({"width":1,"height":1,"data":BASE64.encode([0,0,0,255])}),
                ),
                Request::UrlPictureInPicture { enabled, .. } => {
                    *self.open.lock().unwrap() = enabled;
                    self.commands.lock().unwrap().push(enabled);
                    Response::UrlPictureInPicture(enabled.then(|| *self.size.lock().unwrap()))
                }
                Request::UrlPictureInPictureState { .. } => Response::UrlPictureInPicture(
                    (*self.open.lock().unwrap()).then(|| *self.size.lock().unwrap()),
                ),
                Request::UrlRelease { .. } => Response::Stopped,
                _ => return Err("unexpected media request".into()),
            };
            Ok(crate::platform_control::Response::Media { response })
        }
    }
    struct Activation;
    impl crate::user_activation::ActivationBackend for Activation {
        fn notify(&self, _: crate::worker_wire::Creator) -> Result<(), String> {
            Ok(())
        }
        fn transient(&self, _: crate::worker_wire::Creator, _: bool) -> Result<bool, String> {
            Ok(true)
        }
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, text: &str) -> v8::Local<'s, v8::Value> {
        let text = v8::String::new(scope, text).unwrap();
        v8::Script::compile(scope, text, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn pip_policy_reports_and_retained_entry_resize_exit_share_native_identity() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<video id=v></video>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let backend = Arc::new(Backend::default());
        *backend.size.lock().unwrap() = (320, 180);
        let messages = register(&state);
        state.set_native_platform_backend(backend.clone());
        state.set_activation_backend(Arc::new(Activation));
        let node = dom.borrow().find_by_id("v").unwrap();
        media_element::url_decoded(
            &state,
            node,
            &serde_json::json!({"id":"video","duration":10.,"width":1,"height":1}),
            true,
        )
        .unwrap();
        state.set_native_permissions_policy(Rc::new(|_| false));
        assert!(eval(scope,"globalThis.reports=new ReportingObserver(()=>{},{types:['permissions-policy-violation']});reports.observe();globalThis.v=document.getElementById('v');globalThis.denied='';v.requestPictureInPicture().catch(e=>denied=e.name);!document.pictureInPictureEnabled").is_true());
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "denied==='SecurityError'").is_true());
        assert!(backend.commands.lock().unwrap().is_empty());
        assert!(eval(scope,"reports.takeRecords().some(r=>r.body.featureId==='picture-in-picture' && r.body.disposition==='enforce')").is_true());
        state.set_native_permissions_policy(Rc::new(|_| true));
        state.set_native_permissions_report_only(Rc::new(|_| false));
        assert!(eval(scope,"globalThis.order=[];globalThis.windowPip=null;v.onenterpictureinpicture=e=>{windowPip=e.pictureInPictureWindow;order.push('enter');windowPip.onresize=()=>order.push('resize')};v.onleavepictureinpicture=e=>order.push(e.pictureInPictureWindow===windowPip?'leave':'wrong');v.requestPictureInPicture().then(w=>order.push(w===windowPip?'resolved':'wrong'));document.pictureInPictureElement===v").is_true());
        assert!(eval(scope,"reports.takeRecords().some(r=>r.body.featureId==='picture-in-picture' && r.body.disposition==='report')").is_true());
        state.set_execution_suspended(true);
        assert_eq!(poll(scope, &state).unwrap(), 0);
        state.set_execution_suspended(false);
        native_platform::publication::fail_after_next_callback();
        assert!(poll(scope, &state).is_err());
        assert!(eval(scope, "order.join()==='enter'").is_true());
        state.picture_in_picture.borrow_mut().retry = None;
        poll(scope, &state).unwrap();
        scope.perform_microtask_checkpoint();
        assert!(eval(scope,"order.join()==='enter,resolved' && windowPip instanceof PictureInPictureWindow && windowPip.width===320 && new PictureInPictureEvent('x',{pictureInPictureWindow:windowPip}).pictureInPictureWindow===windowPip").is_true());
        // Promise resolution reads an authored then getter on the existing native
        // window. Reentry must not retain the PiP registry borrow or open again.
        assert!(eval(scope,"globalThis.sameWindow=0;Object.defineProperty(windowPip,'then',{configurable:true,get(){delete windowPip.then;v.requestPictureInPicture().then(w=>sameWindow+=Number(w===windowPip));return undefined}});v.requestPictureInPicture().then(w=>sameWindow+=Number(w===windowPip));true").is_true());
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "sameWindow===2").is_true());
        assert_eq!(*backend.commands.lock().unwrap(), vec![true]);
        *backend.size.lock().unwrap() = (400, 200);
        poll(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "order.join()==='enter,resolved,resize' && windowPip.height===200"
            )
            .is_true()
        );
        eval(
            scope,
            "document.exitPictureInPicture().then(()=>order.push('exited'))",
        );
        poll(scope, &state).unwrap();
        scope.perform_microtask_checkpoint();
        assert!(eval(scope,"document.pictureInPictureElement===null && order.join()==='enter,resolved,resize,leave,exited'").is_true());
        assert_eq!(*backend.commands.lock().unwrap(), vec![true, false]);
        messages.assert_lifecycle(0);
        state.retire_execution();
        messages.assert_lifecycle(1);
    }
    #[test]
    fn metadata_and_activation_failures_do_not_open_native_surface() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<video id=v></video>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let backend = Arc::new(Backend::default());
        let messages = register(&state);
        state.set_native_platform_backend(backend.clone());
        let node = dom.borrow().find_by_id("v").unwrap();
        assert!(eval(scope,r#"(()=>{
   for(const [method,receiver] of [[HTMLVideoElement.prototype.requestPictureInPicture,{}],[HTMLVideoElement.prototype.requestPictureInPicture,document.createElement('audio')],[Document.prototype.exitPictureInPicture,document.createElement('div')]]){let rejected=false;try{method.call(receiver)}catch(error){rejected=error instanceof TypeError}if(!rejected)return false}
   const request=HTMLVideoElement.prototype.requestPictureInPicture;
   if(request.length!==0||request.name!=='requestPictureInPicture'||Object.hasOwn(request,'prototype'))return false;
   const result=document.exitPictureInPicture();globalThis.emptyExit='pending';result.catch(error=>emptyExit=error.name);return result instanceof Promise;
  })()"#).is_true());
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "emptyExit==='InvalidStateError'").is_true());
        eval(
            scope,
            "globalThis.failure='';document.getElementById('v').requestPictureInPicture().catch(e=>failure=e.name)",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "failure==='InvalidStateError'").is_true());
        media_element::url_decoded(
            &state,
            node,
            &serde_json::json!({"id":"video","duration":10.,"width":1,"height":1}),
            true,
        )
        .unwrap();
        eval(
            scope,
            "document.getElementById('v').requestPictureInPicture().catch(e=>failure=e.name)",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "failure==='NotAllowedError'").is_true());
        assert!(backend.commands.lock().unwrap().is_empty());
        messages.assert_lifecycle(0);
        state.retire_execution();
        messages.assert_lifecycle(1);
    }
}
