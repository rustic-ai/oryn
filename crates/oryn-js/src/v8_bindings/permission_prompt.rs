//! Trusted native permission popup input. Pending parent identities are private;
//! author events cannot answer them or create grants.
use super::*;
use crate::platform_control::{PermissionState, PromptChoice, PromptKind, Request, Response};
fn eligible(state: &V8BindingState, node: NodeId) -> bool {
    let dom = state.dom.borrow();
    let protocol = dom
        .permission_prompt_snapshot(node)
        .ok()
        .flatten()
        .is_some_and(|prompt| prompt.protocol.is_some());
    !state.execution_retired()
        && dom.node_document(node).ok().flatten() == Some(state.document_root())
        && dom.is_connected(node).unwrap_or(false)
        && (protocol || !dom.is_inert(node).unwrap_or(true))
}
fn close(state: &V8BindingState, node: NodeId) {
    state.dom.borrow_mut().remove_from_top_layer(node);
    state.permission_elements.borrow_mut().prompt = None;
    state.permission_elements.borrow_mut().prompt_pointer = None;
    state.layout_cache_key.borrow_mut().take();
}
pub(super) fn open(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    geo: bool,
) -> Result<(), V8BindingError> {
    if !eligible(state, node) || state.permission_elements.borrow().prompt.is_some() {
        return Ok(());
    }
    native_platform::publish_media_security(scope, state)?;
    let kind = if let Some(kind) = permission_elements::legacy_prompt_kind(state, node) {
        kind
    } else if geo {
        PromptKind::Geolocation
    } else if state
        .dom
        .borrow()
        .is_element(node, "http://www.w3.org/1999/xhtml", "camera")
    {
        PromptKind::Camera
    } else if state
        .dom
        .borrow()
        .is_element(node, "http://www.w3.org/1999/xhtml", "microphone")
    {
        PromptKind::Microphone
    } else {
        PromptKind::UserMedia
    };
    let response = native_platform::prompt_request(state, Request::PromptBegin { kind });
    let prompt = match response {
        Ok(Response::Prompt(prompt)) => prompt,
        Err(error) => return Err(error),
        _ => {
            return Err(V8BindingError::Allocation(
                "native permission prompt response",
            ));
        }
    };
    if prompt.state != PermissionState::Prompt {
        return permission_elements::prompt_result(
            scope,
            state,
            node,
            if prompt.state == PermissionState::Granted {
                PromptChoice::Allow
            } else {
                PromptChoice::Deny
            },
            prompt.state,
        );
    }
    let id = prompt.id.clone();
    let presentation = oryn_dom::PermissionPrompt {
        id: id.clone(),
        origin: prompt.origin,
        geolocation: geo,
        protocol: None,
        protocol_url: None,
        active: 2,
    };
    if !state
        .dom
        .borrow_mut()
        .open_permission_prompt(node, presentation)?
    {
        native_platform::prompt_request(state, Request::PromptCancel { id })?;
        return Ok(());
    }
    state.permission_elements.borrow_mut().prompt = Some((node, id));
    state.layout_cache_key.borrow_mut().take();
    Ok(())
}
pub(super) fn open_protocol(
    _scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    prompt: crate::platform_control::PermissionPrompt,
    scheme: String,
    url: String,
) -> Result<(), V8BindingError> {
    if prompt.kind != PromptKind::Protocol || prompt.state != PermissionState::Prompt {
        return Err(V8BindingError::Allocation("invalid native protocol prompt"));
    }
    let node = state
        .dom
        .borrow()
        .first_element_child(state.document_root())?;
    let Some(node) = node.filter(|node| {
        !state.execution_retired()
            && state.dom.borrow().node_document(*node).ok().flatten() == Some(state.document_root())
            && state.dom.borrow().is_connected(*node).unwrap_or(false)
    }) else {
        native_platform::prompt_request(state, Request::PromptCancel { id: prompt.id })?;
        return Ok(());
    };
    if state.permission_elements.borrow().prompt.is_some() {
        native_platform::prompt_request(state, Request::PromptCancel { id: prompt.id })?;
        return Ok(());
    }
    let id = prompt.id;
    let presentation = oryn_dom::PermissionPrompt {
        id: id.clone(),
        origin: prompt.origin,
        geolocation: false,
        protocol: Some(scheme),
        protocol_url: Some(url),
        active: 2,
    };
    if !state
        .dom
        .borrow_mut()
        .open_permission_prompt(node, presentation)?
    {
        native_platform::prompt_request(state, Request::PromptCancel { id })?;
        return Ok(());
    }
    state.permission_elements.borrow_mut().prompt = Some((node, id));
    state.layout_cache_key.borrow_mut().take();
    Ok(())
}
pub(super) fn cancel_protocol(
    state: &V8BindingState,
    scheme: &str,
    url: &str,
) -> Result<(), V8BindingError> {
    let pending = state.permission_elements.borrow().prompt.clone();
    if let Some((node, id)) = pending
        && state
            .dom
            .borrow()
            .permission_prompt_snapshot(node)?
            .is_some_and(|prompt| {
                prompt.protocol.as_deref() == Some(scheme)
                    && prompt.protocol_url.as_deref() == Some(url)
            })
    {
        native_platform::prompt_request(state, Request::PromptCancel { id })?;
        close(state, node);
    }
    Ok(())
}
fn answer(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    choice: PromptChoice,
) -> Result<(), V8BindingError> {
    let Some((node, id)) = state.permission_elements.borrow().prompt.clone() else {
        return Ok(());
    };
    if !eligible(state, node) {
        fixup(scope, state)?;
        return Ok(());
    }
    let protocol = state
        .dom
        .borrow()
        .permission_prompt_snapshot(node)?
        .is_some_and(|prompt| prompt.protocol.is_some());
    let response = native_platform::prompt_request(
        state,
        if choice == PromptChoice::Dismiss {
            Request::PromptCancel { id }
        } else {
            Request::PromptAnswer { id, choice }
        },
    )?;
    let permission = match response {
        Response::Prompt(prompt) => prompt.state,
        Response::PromptCanceled => PermissionState::Prompt,
        _ => {
            return Err(V8BindingError::Allocation(
                "native permission choice response",
            ));
        }
    };
    close(state, node);
    if protocol {
        Ok(())
    } else {
        permission_elements::prompt_result(scope, state, node, choice, permission)
    }
}
pub(super) struct Choice {
    activate: bool,
    security: bool,
    owner: Rc<V8BindingState>,
    input_owner: native_pointer::InputOwner,
    node: NodeId,
    id: String,
    choice: PromptChoice,
    protocol: bool,
    receipt: u64,
    permission: Option<PermissionState>,
    phase: u8,
}
impl Choice {
    fn new(
        state: &Rc<V8BindingState>,
        node: NodeId,
        id: String,
        choice: PromptChoice,
    ) -> Result<Self, V8BindingError> {
        let protocol = state
            .dom
            .borrow()
            .permission_prompt_snapshot(node)?
            .is_some_and(|prompt| prompt.protocol.is_some());
        Ok(Self {
            activate: false,
            security: false,
            owner: state.clone(),
            input_owner: native_pointer::InputOwner::new(state),
            node,
            id,
            choice,
            protocol,
            receipt: next_resolution_receipt()?,
            permission: None,
            phase: 0,
        })
    }
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        loop {
            if self.phase == 3 {
                return Ok(true);
            }
            if !self.input_owner.current() {
                self.phase = 3;
                return Ok(true);
            }
            let state = self.owner.clone();
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(false);
            }
            if self.phase < 2
                && (state.permission_elements.borrow().prompt.as_ref()
                    != Some(&(self.node, self.id.clone()))
                    || !eligible(&state, self.node))
            {
                self.phase = 3;
                return Ok(true);
            }
            if self.activate {
                state.notify_user_activation();
                self.activate = false;
            }
            if self.security {
                native_platform::publish_media_security(scope, &state)?;
                self.security = false;
            }
            match self.phase {
                0 => {
                    let response = native_platform::prompt_request(
                        &state,
                        Request::PromptResolve {
                            id: self.id.clone(),
                            choice: self.choice,
                            receipt: self.receipt,
                        },
                    )?;
                    self.permission = Some(match response {
                        Response::Prompt(prompt) => prompt.state,
                        Response::PromptCanceled => PermissionState::Prompt,
                        _ => {
                            return Err(V8BindingError::Allocation(
                                "native permission choice response",
                            ));
                        }
                    });
                    self.phase = 1;
                }
                1 => {
                    if !self.protocol {
                        permission_elements::admit_prompt_result(
                            &state,
                            self.node,
                            self.choice,
                            self.permission.unwrap(),
                        )?;
                    }
                    close(&state, self.node);
                    self.phase = 2;
                }
                _ => {
                    if !self.protocol {
                        permission_elements::resume_prompt_result(scope, &state, self.node)?;
                    }
                    self.phase = 3;
                }
            }
        }
    }
}
pub(super) fn prepare_keyboard(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    key: &str,
) -> Result<(bool, Option<Choice>), V8BindingError> {
    let Some((node, id)) = state.permission_elements.borrow().prompt.clone() else {
        return Ok((false, None));
    };
    if !eligible(state, node) {
        fixup(scope, state)?;
        return Ok((false, None));
    }
    let choice = match key {
        "Escape" => Some(PromptChoice::Dismiss),
        "Tab" | "ArrowRight" | "ArrowDown" => {
            state.dom.borrow_mut().move_permission_prompt(node, 1)?;
            None
        }
        "ArrowLeft" | "ArrowUp" => {
            state.dom.borrow_mut().move_permission_prompt(node, -1)?;
            None
        }
        "Enter" | " " => {
            let active = state
                .dom
                .borrow()
                .permission_prompt_snapshot(node)?
                .map(|prompt| prompt.active)
                .unwrap_or(2);
            Some(match active {
                0 => PromptChoice::Allow,
                1 => PromptChoice::Deny,
                _ => PromptChoice::Dismiss,
            })
        }
        _ => None,
    };
    let job = choice
        .map(|choice| Choice::new(state, node, id, choice))
        .transpose()?;
    state.layout_cache_key.borrow_mut().take();
    Ok((true, job))
}
pub(super) fn keyboard(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    key: &str,
) -> Result<bool, V8BindingError> {
    let (handled, job) = prepare_keyboard(scope, state, key)?;
    if let Some(mut job) = job
        && !job.poll(scope)?
    {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(handled)
}

pub(super) fn click(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    _target: NodeId,
    event: v8::Local<v8::Object>,
) -> Result<bool, V8BindingError> {
    let id = event_id(scope, event)?;
    if !state
        .events
        .borrow()
        .get(&id)
        .is_some_and(|event| event.is_trusted)
    {
        return Ok(false);
    }
    let Some((node, _)) = state.permission_elements.borrow().prompt.clone() else {
        return Ok(false);
    };
    if !eligible(state, node) {
        fixup(scope, state)?;
        return Ok(false);
    }
    let x = native_input_property(scope, event, "MouseEvent", "clientX")?
        .number_value(scope)
        .unwrap_or(f64::NAN);
    let y = native_input_property(scope, event, "MouseEvent", "clientY")?
        .number_value(scope)
        .unwrap_or(f64::NAN);
    refresh_layout_in_scope(scope, state)?;
    let action = {
        let layout = state.layout.borrow();
        layout
            .generated_nodes
            .iter()
            .rev()
            .find(|item| {
                item.parent_id == Some(node_key(node))
                    && item.tag.starts_with("::permission-")
                    && item.geometry.box_model.as_ref().is_some_and(|model| {
                        let r = model.border_box;
                        x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
                    })
            })
            .map(|item| item.tag.clone())
    };
    match action.as_deref() {
        Some("::permission-allow") => answer(scope, state, PromptChoice::Allow)?,
        Some("::permission-deny") => answer(scope, state, PromptChoice::Deny)?,
        Some("::permission-dismiss") | None => answer(scope, state, PromptChoice::Dismiss)?,
        _ => {}
    }
    state.permission_elements.borrow_mut().suppress_activation = Some(_target);
    Ok(true)
}
pub(super) fn fixup(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<(), V8BindingError> {
    let Some((node, id)) = state.permission_elements.borrow().prompt.clone() else {
        return Ok(());
    };
    if !eligible(state, node)
        || state
            .dom
            .borrow()
            .permission_prompt_snapshot(node)?
            .is_none()
        || (!state
            .dom
            .borrow()
            .permission_prompt_snapshot(node)?
            .is_some_and(|prompt| prompt.protocol.is_some())
            && css_suppresses_focus_in_scope(scope, state, node)?)
    {
        native_platform::prompt_request(state, Request::PromptCancel { id })?;
        close(state, node);
    }
    Ok(())
}

pub(super) fn retire(state: &V8BindingState) {
    let prompt = state.permission_elements.borrow().prompt.clone();
    if let Some((node, _)) = prompt {
        close(state, node);
    }
}

pub(super) fn prepare_accessibility(
    _scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    expected: &oryn_dom::PermissionPrompt,
    choice: PromptChoice,
) -> Result<Option<Choice>, V8BindingError> {
    if !eligible(state, node)
        || state
            .dom
            .borrow()
            .permission_prompt_snapshot(node)?
            .as_ref()
            != Some(expected)
        || state.permission_elements.borrow().prompt.as_ref() != Some(&(node, expected.id.clone()))
    {
        return Ok(None);
    }
    let mut job = Choice::new(state, node, expected.id.clone(), choice)?;
    job.activate = true;
    job.security = true;
    Ok(Some(job))
}

/// A single process-native namespace is shared by all retained popup inputs.
pub(super) fn next_resolution_receipt() -> Result<u64, V8BindingError> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |value| value.checked_add(1),
    )
    .map_err(|_| V8BindingError::Allocation("permission resolution identity exhausted"))
}
/// Intercept browser popup input before author PointerEvent listeners run.
pub(super) struct Pointer {
    owner: Rc<V8BindingState>,
    input_owner: native_pointer::InputOwner,
    input: NativePointerInput,
    action: NativePointerAction,
    target: Option<NodeId>,
    answer: Option<(NodeId, String, PromptChoice, bool)>,
    permission: Option<PermissionState>,
    receipt: Option<u64>,
    phase: u8,
    result: Option<Option<NodeId>>,
}
impl Pointer {
    pub(super) fn new(
        state: &Rc<V8BindingState>,
        input: &NativePointerInput,
        action: NativePointerAction,
    ) -> Self {
        Self {
            owner: state.clone(),
            input_owner: native_pointer::InputOwner::new(state),
            input: input.clone(),
            action,
            target: None,
            answer: None,
            permission: None,
            receipt: None,
            phase: 0,
            result: None,
        }
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<Option<NodeId>>, V8BindingError> {
        if self.result.is_some() {
            return Ok(self.result);
        }
        loop {
            if !self.input_owner.current() {
                self.result = Some(self.target);
                return Ok(self.result);
            }
            let state = self.owner.clone();
            if state.execution_suspended() || scope.is_execution_terminating() {
                return Ok(None);
            }
            match self.phase {
                0 => {
                    let Some((node, id)) = state.permission_elements.borrow().prompt.clone() else {
                        self.result = Some(None);
                        return Ok(self.result);
                    };
                    if !eligible(&state, node) {
                        fixup(scope, &state)?;
                        self.result = Some(None);
                        return Ok(self.result);
                    }
                    refresh_layout_in_scope(scope, &state)?;
                    let input = &self.input;
                    let choice = {
                        let layout = state.layout.borrow();
                        layout
                            .generated_nodes
                            .iter()
                            .rev()
                            .find(|item| {
                                item.parent_id == Some(node_key(node))
                                    && matches!(
                                        item.tag.as_str(),
                                        "::permission-allow"
                                            | "::permission-deny"
                                            | "::permission-dismiss"
                                    )
                                    && item.geometry.box_model.as_ref().is_some_and(|model| {
                                        let r = model.border_box;
                                        input.x >= r.x
                                            && input.x < r.right()
                                            && input.y >= r.y
                                            && input.y < r.bottom()
                                    })
                            })
                            .map(|item| item.tag.clone())
                    };
                    self.target = Some(node);
                    if self.action == NativePointerAction::Down && input.button == 0 {
                        state.notify_user_activation();
                        state.permission_elements.borrow_mut().prompt_pointer =
                            Some((input.id, choice));
                        self.result = Some(self.target);
                        return Ok(self.result);
                    }
                    if self.action == NativePointerAction::Cancel {
                        state.permission_elements.borrow_mut().prompt_pointer = None;
                    }
                    if self.action != NativePointerAction::Up || input.button != 0 {
                        self.result = Some(self.target);
                        return Ok(self.result);
                    }
                    let protocol = state
                        .dom
                        .borrow()
                        .permission_prompt_snapshot(node)?
                        .is_some_and(|prompt| prompt.protocol.is_some());
                    if state.permission_elements.borrow_mut().prompt_pointer.take()
                        != Some((input.id, choice.clone()))
                    {
                        self.result = Some(self.target);
                        return Ok(self.result);
                    }
                    let choice = match choice.as_deref() {
                        Some("::permission-allow") => PromptChoice::Allow,
                        Some("::permission-deny") => PromptChoice::Deny,
                        _ => PromptChoice::Dismiss,
                    };
                    self.answer = Some((node, id, choice, protocol));
                    self.phase = 1;
                }
                1 => {
                    let (node, id, choice, _) = self.answer.as_ref().unwrap();
                    if state.permission_elements.borrow().prompt.as_ref()
                        != Some(&(*node, id.clone()))
                        || !eligible(&state, *node)
                    {
                        self.result = Some(self.target);
                        return Ok(self.result);
                    }
                    if self.receipt.is_none() {
                        self.receipt = Some(next_resolution_receipt()?);
                    }
                    let response = native_platform::prompt_request(
                        &state,
                        Request::PromptResolve {
                            id: id.clone(),
                            choice: *choice,
                            receipt: self.receipt.unwrap(),
                        },
                    )?;
                    self.permission = Some(match response {
                        Response::Prompt(prompt) => prompt.state,
                        Response::PromptCanceled => PermissionState::Prompt,
                        _ => {
                            return Err(V8BindingError::Allocation(
                                "native permission choice response",
                            ));
                        }
                    });
                    self.phase = 2;
                }
                2 => {
                    let (node, id, choice, protocol) = self.answer.as_ref().unwrap();
                    if state.permission_elements.borrow().prompt.as_ref()
                        != Some(&(*node, id.clone()))
                        || !eligible(&state, *node)
                    {
                        self.result = Some(self.target);
                        return Ok(self.result);
                    }
                    if !*protocol {
                        permission_elements::admit_prompt_result(
                            &state,
                            *node,
                            *choice,
                            self.permission.unwrap(),
                        )?;
                    }
                    close(&state, *node);
                    self.phase = 3;
                }
                _ => {
                    let (node, _, _, protocol) = self.answer.as_ref().unwrap();
                    if !*protocol {
                        permission_elements::resume_prompt_result(scope, &state, *node)?;
                    }
                    self.result = Some(self.target);
                    return Ok(self.result);
                }
            }
        }
    }
}

#[cfg(test)]
mod keyboard_receipt_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Transport;
    impl crate::message_wire::MessageTransport for Transport {
        fn request(
            &self,
            _: crate::message_wire::MessageRequest,
        ) -> Result<crate::message_wire::MessageResponse, String> {
            Ok(crate::message_wire::MessageResponse::Done)
        }
    }
    impl crate::worker_wire::WorkerTransport for Transport {
        fn request(
            &self,
            _: crate::worker_wire::AgentServiceRequest,
        ) -> Result<crate::worker_wire::AgentServiceResponse, crate::worker_wire::WorkerError>
        {
            Ok(crate::worker_wire::AgentServiceResponse::Done)
        }
    }
    struct Backend {
        calls: Mutex<Vec<(String, PromptChoice, u64)>>,
    }
    impl crate::platform_control::Backend for Backend {
        fn request(
            &self,
            _: crate::worker_wire::Creator,
            request: Request,
        ) -> Result<Response, String> {
            let Request::PromptResolve {
                id,
                choice,
                receipt,
            } = request
            else {
                panic!("unexpected prompt request")
            };
            let mut calls = self.calls.lock().unwrap();
            calls.push((id, choice, receipt));
            if calls.len() == 1 {
                Err("lost response after accepted dismissal".into())
            } else {
                assert_eq!(calls[0], calls[1]);
                Ok(Response::PromptCanceled)
            }
        }
    }
    #[test]
    fn keyboard_dismissal_resumes_with_same_parent_resolution_identity() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(248), 1).unwrap();
        state.set_message_transport(Arc::new(Transport));
        state.set_worker_transport(Arc::new(Transport), None, None);
        let backend = Arc::new(Backend {
            calls: Mutex::new(Vec::new()),
        });
        state.set_native_platform_backend(backend.clone());
        let node = state
            .dom
            .borrow()
            .first_element_child(state.document_root())
            .unwrap()
            .unwrap();
        let id = "native-test-prompt".to_owned();
        state
            .dom
            .borrow_mut()
            .open_permission_prompt(
                node,
                oryn_dom::PermissionPrompt {
                    id: id.clone(),
                    origin: "https://prompt.test".into(),
                    geolocation: false,
                    protocol: Some("web+test".into()),
                    protocol_url: Some("https://prompt.test/handler".into()),
                    active: 2,
                },
            )
            .unwrap();
        state.permission_elements.borrow_mut().prompt = Some((node, id));
        let input = NativeKeyboardInput {
            key: "Escape".encode_utf16().collect(),
            down: true,
            ..Default::default()
        };
        let mut keyboard = native_keyboard::begin_receipt(state.clone(), &input).unwrap();
        assert!(native_keyboard::poll_receipt(scope, &mut keyboard).is_err());
        assert!(keyboard.retry_safe());
        assert!(state.permission_elements.borrow().prompt.is_some());
        state.set_execution_suspended(true);
        assert!(
            native_keyboard::poll_receipt(scope, &mut keyboard)
                .unwrap()
                .is_none()
        );
        assert_eq!(backend.calls.lock().unwrap().len(), 1);
        state.set_execution_suspended(false);
        assert_eq!(
            native_keyboard::poll_receipt(scope, &mut keyboard).unwrap(),
            Some(true)
        );
        assert_eq!(
            native_keyboard::poll_receipt(scope, &mut keyboard).unwrap(),
            Some(true)
        );
        assert_eq!(backend.calls.lock().unwrap().len(), 2);
        assert!(state.permission_elements.borrow().prompt.is_none());
    }
}
