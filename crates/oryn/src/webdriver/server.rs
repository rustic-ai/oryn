//! HTTP command dispatch over native session ownership.
use super::{
    CommandError,
    auth::DriverAuth,
    capabilities::{NativeIdentity, negotiate},
    http::{MAX_BODY_BYTES, secured},
    registry::Registry,
};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::any,
};
use oryn_native::{Browser, ContextOptions, host::webdriver_script::ScriptValue};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::{
    sync::{Semaphore, oneshot},
    task::{JoinError, JoinSet},
};

pub struct Driver {
    browser: Browser,
    options: ContextOptions,
    sessions: Registry,
    scripts: super::script_tasks::ScriptTasks,
    commands: Mutex<JoinSet<Result<(), CommandError>>>,
    command_capacity: Arc<Semaphore>,
    command_bytes: Arc<Semaphore>,
    closing: AtomicBool,
    task_failure: Mutex<Option<CommandError>>,
}

impl Driver {
    pub fn new(
        browser: Browser,
        options: ContextOptions,
        max_sessions: usize,
    ) -> Result<Self, CommandError> {
        let sessions = Registry::new(max_sessions)?;
        Ok(Self {
            browser,
            options,
            sessions,
            scripts: super::script_tasks::ScriptTasks::new(max_sessions * 4),
            commands: Mutex::new(JoinSet::new()),
            command_capacity: Arc::new(Semaphore::new(max_sessions * 4)),
            command_bytes: Arc::new(Semaphore::new(64 * 1024 * 1024)),
            closing: AtomicBool::new(false),
            task_failure: Mutex::new(None),
        })
    }

    pub fn router(self: Arc<Self>, auth: Arc<DriverAuth>) -> Router {
        secured(Router::new().fallback(any(dispatch)).with_state(self), auth)
    }

    fn record_completion(&self, result: Result<Result<(), CommandError>, JoinError>) {
        let error = match result {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error,
            Err(_) => CommandError {
                code: "unknown error",
                message: "native command task failed".into(),
            },
        };
        self.task_failure
            .lock()
            .expect("driver failure lock")
            .get_or_insert(error);
    }

    fn start_command(
        self: &Arc<Self>,
        method: String,
        path: String,
        body: Vec<u8>,
    ) -> Result<oneshot::Receiver<Result<ScriptValue, CommandError>>, CommandError> {
        let mut commands = self.commands.lock().expect("driver command lock");
        if self.closing.load(Ordering::Acquire) {
            return Err(CommandError::session_not_created("driver is shutting down"));
        }
        while let Some(result) = commands.try_join_next() {
            self.record_completion(result);
        }
        let permit = self
            .command_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| CommandError {
                code: "unknown error",
                message: "driver command capacity exhausted".into(),
            })?;
        let bytes = u32::try_from(body.len().max(1))
            .map_err(|_| CommandError::invalid("command body too large"))?;
        let byte_permit = self
            .command_bytes
            .clone()
            .try_acquire_many_owned(bytes)
            .map_err(|_| CommandError {
                code: "unknown error",
                message: "driver command byte capacity exhausted".into(),
            })?;
        let (reply, response) = oneshot::channel();
        let driver = self.clone();
        commands.spawn(async move {
            let _permit = permit;
            let _byte_permit = byte_permit;
            // This task belongs to the driver, not the HTTP connection. Dropping
            // a response receiver cannot detach untracked native work.
            let result = driver.command(&method, &path, &body).await;
            if let Err(result) = reply.send(result) {
                // A newly created session whose response was lost has no client
                // owner. Revoke and drain it immediately instead of leaking it.
                if method == "POST"
                    && path == "/session"
                    && let Ok(value) = result
                    && let Some(id) = value
                        .get("sessionId")
                        .and_then(|value| value.as_string().ok())
                    && let Err(error) = driver.sessions.delete(&id).await
                {
                    // Another deletion/shutdown may already own cleanup.
                    if error.code != "invalid session id" {
                        return Err(error);
                    }
                }
            }
            Ok(())
        });
        Ok(response)
    }

    pub async fn shutdown(&self) -> Result<(), CommandError> {
        let mut commands = {
            let mut tasks = self.commands.lock().expect("driver command lock");
            self.closing.store(true, Ordering::Release);
            std::mem::take(&mut *tasks)
        };
        self.scripts.close_admission();
        let sessions = self.sessions.shutdown().await;
        let scripts = self.scripts.drain().await;
        while let Some(result) = commands.join_next().await {
            self.record_completion(result);
        }
        if let Some(error) = self
            .task_failure
            .lock()
            .expect("driver failure lock")
            .clone()
        {
            return Err(error);
        }
        sessions.and(scripts)
    }

    async fn command(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Result<ScriptValue, CommandError> {
        let fields = || -> Result<Value, CommandError> {
            let value: Value = serde_json::from_slice(body)
                .map_err(|_| CommandError::invalid("command body must be JSON"))?;
            if !value.is_object() {
                return Err(CommandError::invalid(
                    "command parameters must be an object",
                ));
            }
            Ok(value)
        };
        let parts = path
            .strip_prefix('/')
            .unwrap_or(path)
            .split('/')
            .collect::<Vec<_>>();
        let allowed = command_methods(&parts).ok_or_else(unknown_command)?;
        if !allowed.contains(&method) {
            return Err(CommandError {
                code: "unknown method",
                message: "HTTP method is not allowed for this command".into(),
            });
        }
        if parts == ["status"] && method == "GET" {
            return Ok(json!({"ready":!self.closing.load(Ordering::Acquire) && self.sessions.accepting_sessions(),"message":"native Oryn command service"}).into());
        }
        if parts == ["session"] && method == "POST" {
            let request = fields()?;
            let platform = match std::env::consts::OS {
                "macos" => "mac",
                name => name,
            };
            let capabilities = negotiate(
                &request,
                NativeIdentity {
                    version: env!("CARGO_PKG_VERSION"),
                    platform,
                },
            )?;
            let id = self
                .sessions
                .create(&self.browser, self.options.clone(), capabilities.clone())
                .await?;
            return Ok(json!({"sessionId":id,"capabilities":capabilities}).into());
        }
        let ["session", id, tail @ ..] = parts.as_slice() else {
            return Err(unknown_command());
        };
        if id.is_empty() {
            return Err(unknown_command());
        }
        if tail.is_empty() && method == "DELETE" {
            self.sessions.delete(id).await?;
            return Ok(ScriptValue::Null);
        }
        if method == "GET" && tail == ["oryn", "diagnostics", "retirement"] {
            return self.sessions.retirement(id).map(ScriptValue::from);
        }
        if method == "GET" && tail == ["oryn", "diagnostics", "health"] {
            return self.sessions.health(id).map(ScriptValue::from);
        }
        let mut session = self.sessions.acquire(id).await?;
        if tail == ["alert", "text"] {
            let page = session.current_page()?;
            if method == "GET" {
                return super::prompts::text(&page);
            }
            let text = std::str::from_utf8(body)
                .map_err(|_| CommandError::invalid("command body must be UTF-8 JSON"))?;
            let value = ScriptValue::from_json_text(text)
                .map_err(|error| CommandError::invalid(error.message))?;
            super::prompts::set_text(&page, &value)?;
            return Ok(ScriptValue::Null);
        }
        if let ("POST", ["execute", mode @ ("sync" | "async")]) = (method, tail) {
            let text = std::str::from_utf8(body)
                .map_err(|_| CommandError::invalid("command body must be UTF-8 JSON"))?;
            let mut value = ScriptValue::from_json_text(text)
                .map_err(|error| CommandError::invalid(error.message))?;
            let page = session.current_page()?;
            if let Some(text) = super::prompts::handle_pending(&page, &session.prompt_policy)? {
                return Err(CommandError {
                    code: "unexpected alert open",
                    message: text,
                });
            }
            let asynchronous = *mode == "async";
            let timeout = session.timeouts.script;
            let parsed = super::script::request(&value, asynchronous, timeout)?;
            let authorized =
                session.authorize_script_arguments(&ScriptValue::Array(parsed.args))?;
            if let ScriptValue::Object(entries) = &mut value {
                let (_, args) = entries
                    .iter_mut()
                    .find(|(key, _)| key.iter().copied().eq("args".encode_utf16()))
                    .ok_or_else(|| CommandError::invalid("script args are missing"))?;
                *args = authorized;
            }
            let context = session.execution_context().await?;
            let cancellation = session.cancellation();
            let execute_page = page.clone();
            let result = self
                .scripts
                .execute(page, body.len(), cancellation, async move {
                    super::script::execute(&execute_page, context, &value, asynchronous, timeout)
                        .await
                })
                .await?;
            return session.finalize_script_result(result);
        }
        let handles_prompts = matches!(
            tail,
            ["url"]
                | ["title"]
                | ["window", "rect"]
                | ["window", "new"]
                | ["frame"]
                | ["frame", "parent"]
                | ["print"]
                | ["screenshot"]
                | ["back"]
                | ["forward"]
                | ["refresh"]
                | ["actions"]
        ) || tail.first() == Some(&"element")
            || tail == ["elements"]
            || (tail.first() == Some(&"oryn") && tail != ["oryn", "diagnostics", "navigation"])
            || tail == ["permissions"]
            || tail == ["custom-handlers", "set-mode"]
            || tail == ["window", "minimize"]
            || (method == "DELETE" && tail == ["window"]);
        if handles_prompts
            && let Some(text) =
                super::prompts::handle_pending(&session.current_page()?, &session.prompt_policy)?
        {
            return Err(CommandError {
                code: "unexpected alert open",
                message: text,
            });
        }
        if tail == ["cookie"] && matches!(method, "GET" | "DELETE") {
            let cookies = session.cookies(method == "DELETE").await?;
            return Ok(if method == "DELETE" {
                ScriptValue::Null
            } else {
                ScriptValue::from(json!(cookies))
            });
        }
        if let ("GET", ["cookie", name]) = (method, tail) {
            return session
                .cookies(false)
                .await?
                .into_iter()
                .find(|cookie| cookie.get("name").and_then(Value::as_str) == Some(*name))
                .map(ScriptValue::from)
                .ok_or_else(|| CommandError {
                    code: "no such cookie",
                    message: "no associated cookie has this name".into(),
                });
        }
        if method == "POST"
            && matches!(
                tail,
                ["element"]
                    | ["elements"]
                    | ["element", _, "element"]
                    | ["element", _, "elements"]
                    | ["oryn", "elements"]
            )
        {
            let value = fields()?;
            let selectors = if tail == ["oryn", "elements"] {
                value
                    .get("selectors")
                    .and_then(Value::as_array)
                    .ok_or_else(|| CommandError::invalid("selectors must be an array"))?
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| CommandError::invalid("selector must be a string"))
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                if value.get("using").and_then(Value::as_str) != Some("css selector") {
                    return Err(CommandError::invalid(
                        "only the CSS locator strategy is supported by this Core executor",
                    ));
                }
                vec![
                    value
                        .get("value")
                        .and_then(Value::as_str)
                        .ok_or_else(|| CommandError::invalid("CSS selector must be a string"))?
                        .to_owned(),
                ]
            };
            let root = if tail.len() == 3 && tail[0] == "element" {
                Some(tail[1].to_owned())
            } else {
                None
            };
            let cancellation = session.cancellation();
            let result = session.find_elements(root, selectors, cancellation).await?;
            if tail.last() == Some(&"element") {
                let ScriptValue::Array(values) = result else {
                    return Err(unknown_command());
                };
                return values.into_iter().next().ok_or_else(|| CommandError {
                    code: "no such element",
                    message: "CSS locator matched no element".into(),
                });
            }
            return Ok(result);
        }
        if let (
            "GET",
            [
                "element",
                reference,
                property @ ("computedlabel" | "computedrole"),
            ],
        ) = (method, tail)
        {
            return session
                .element_command(
                    oryn_native::host::webdriver_script::ElementCommand::Accessibility {
                        reference: (*reference).into(),
                        role: *property == "computedrole",
                    },
                )
                .await;
        }
        if method == "POST" {
            match tail {
                ["permissions"] | ["oryn", "permissions"] => {
                    session.set_permission(&fields()?).await?;
                    return Ok(ScriptValue::Null);
                }
                ["oryn", "geolocation"] => {
                    session.set_geolocation(&fields()?)?;
                    return Ok(ScriptValue::Null);
                }
                ["custom-handlers", "set-mode"] => {
                    session.set_registration_mode(&fields()?)?;
                    return Ok(ScriptValue::Null);
                }
                ["window", "minimize"] => {
                    session.set_window_hidden(true)?;
                    return super::window_rect::get(&session.current_page()?, session.context()?)
                        .await
                        .map(ScriptValue::from);
                }
                ["window", "rect"] => {
                    super::window_rect::validate(&fields()?)?;
                    session.set_window_hidden(false)?;
                }
                _ => {}
            }
        }
        if let ("POST", ["oryn", "accessibility", kind @ ("element" | "node")]) = (method, tail) {
            let value = fields()?;
            let reference = if *kind == "element" {
                value
                    .get("element")
                    .and_then(|value| value.get(oryn_native::host::webdriver_script::ELEMENT_KEY))
                    .and_then(Value::as_str)
            } else {
                value.get("id").and_then(Value::as_str)
            }
            .ok_or_else(|| CommandError::invalid("native accessibility reference required"))?;
            return session
                .element_command(
                    oryn_native::host::webdriver_script::ElementCommand::AccessibilityProperties {
                        reference: reference.into(),
                        accessibility_node: *kind == "node",
                    },
                )
                .await;
        }
        if let ("POST", ["element", reference, operation @ ("click" | "value")]) = (method, tail) {
            use oryn_native::{
                host::webdriver_script::ElementCommand,
                webdriver_actions::{
                    Action, ActionCommand, ActionsRequest, Origin, PointerKind, PointerProperties,
                    Source, SourceKind,
                },
            };
            let keyboard = *operation == "value";
            let actions = if keyboard {
                let text = std::str::from_utf8(body)
                    .map_err(|_| CommandError::invalid("command body must be UTF-8 JSON"))?;
                let value = ScriptValue::from_json_text(text).map_err(super::script::error)?;
                let Some(ScriptValue::String(text)) = value.get("text") else {
                    return Err(CommandError::invalid("element text must be a string"));
                };
                if session
                    .element_command(ElementCommand::FileInput {
                        reference: (*reference).into(),
                    })
                    .await?
                    == ScriptValue::Bool(true)
                {
                    if session
                        .capabilities
                        .get("strictFileInteractability")
                        .and_then(Value::as_bool)
                        == Some(true)
                    {
                        session
                            .element_command(ElementCommand::Focus {
                                reference: (*reference).into(),
                            })
                            .await?;
                    }
                    return session
                        .upload_element_files((*reference).into(), text)
                        .await;
                }
                super::element_input::keys(text)?
            } else {
                vec![
                    Action::PointerMove {
                        x: 0,
                        y: 0,
                        origin: Origin::Element((*reference).into()),
                        duration_ms: Some(0),
                        properties: PointerProperties::default(),
                    },
                    Action::PointerButton {
                        down: true,
                        button: 0,
                        properties: PointerProperties::default(),
                    },
                    Action::PointerButton {
                        down: false,
                        button: 0,
                        properties: PointerProperties::default(),
                    },
                ]
            };
            let request = ActionsRequest {
                sources: vec![Source {
                    id: session.element_input_source(keyboard),
                    kind: if keyboard {
                        SourceKind::Key
                    } else {
                        SourceKind::Pointer(PointerKind::Mouse)
                    },
                    actions,
                }],
            };
            // Validate the complete generated sequence before scrolling, focusing,
            // or dispatching focus handlers with observable author side effects.
            request.validate().map_err(super::script::error)?;
            let prepared = session
                .element_command(if keyboard {
                    ElementCommand::Focus {
                        reference: (*reference).into(),
                    }
                } else {
                    ElementCommand::PrepareClick {
                        reference: (*reference).into(),
                    }
                })
                .await?;
            if !keyboard && prepared == ScriptValue::Bool(true) {
                return Ok(ScriptValue::Null);
            }
            let command = ActionCommand::Perform(request);
            let page = session.current_page()?;
            let context = session.execution_context().await?;
            let cancellation = session.cancellation();
            let execute_page = page.clone();
            return self
                .scripts
                .execute(page, body.len(), cancellation, async move {
                    execute_page
                        .execute_webdriver_actions_in_context(command, context)
                        .await
                        .map_err(super::script::error)?;
                    Ok(ScriptValue::Null)
                })
                .await;
        }
        if tail == ["actions"] && matches!(method, "POST" | "DELETE") {
            use oryn_native::webdriver_actions::{ActionCommand, ActionsRequest};
            let command = if method == "DELETE" {
                ActionCommand::Release
            } else {
                let text = std::str::from_utf8(body)
                    .map_err(|_| CommandError::invalid("command body must be UTF-8 JSON"))?;
                let value = ScriptValue::from_json_text(text).map_err(super::script::error)?;
                ActionCommand::Perform(ActionsRequest::parse(&value).map_err(super::script::error)?)
            };
            let page = session.current_page()?;
            let context = session.execution_context().await?;
            let cancellation = session.cancellation();
            let execute_page = page.clone();
            return self
                .scripts
                .execute(page, body.len(), cancellation, async move {
                    execute_page
                        .execute_webdriver_actions_in_context(command, context)
                        .await
                        .map_err(super::script::error)?;
                    Ok(ScriptValue::Null)
                })
                .await;
        }
        if method == "POST" && matches!(tail, ["url"] | ["back"] | ["forward"] | ["refresh"]) {
            use oryn_native::navigation_control::NavigationCommand;
            let page = session.current_page()?;
            let command = match tail {
                ["url"] => super::navigation::navigate_command(&fields()?)?,
                ["back"] => NavigationCommand::Back,
                ["forward"] => NavigationCommand::Forward,
                _ => NavigationCommand::Refresh,
            };
            let request =
                super::navigation::request(command, &session.capabilities, session.timeouts)?;
            let job = page
                .navigation_control()
                .map_err(|error| CommandError {
                    code: "no such window",
                    message: error.to_string(),
                })?
                .begin(request)
                .map_err(super::navigation::error)?;
            session.reset_frame_for_navigation();
            return self
                .scripts
                .navigate(page, body.len(), session.cancellation(), job)
                .await;
        }
        let result: Result<Value, CommandError> = match (method, tail) {
            ("POST", ["oryn", "qualification", "worker-crash"]) => {
                let value = fields()?;
                if !value.as_object().is_some_and(|fields| fields.is_empty()) {
                    return Err(CommandError::invalid(
                        "worker crash takes no caller-selected process or page",
                    ));
                }
                session.crash_qualification_worker().await
            }
            ("GET", ["oryn", "diagnostics", "navigation"]) => session.queued_navigation_evidence(),
            ("POST", ["oryn", "diagnostics", "placement"]) => {
                let input = fields()?;
                let nonce = input
                    .get("nonce")
                    .and_then(Value::as_str)
                    .ok_or_else(|| CommandError::invalid("placement nonce is required"))?;
                session.execution_placement(nonce)
            }
            ("POST", ["alert", "accept"]) => {
                super::prompts::accept(&session.current_page()?)?;
                Ok(Value::Null)
            }
            ("POST", ["alert", "dismiss"]) => {
                super::prompts::dismiss(&session.current_page()?)?;
                Ok(Value::Null)
            }
            ("GET", ["timeouts"]) => Ok(session.timeouts.to_json()),
            ("POST", ["timeouts"]) => {
                session.set_timeouts(&fields()?)?;
                Ok(Value::Null)
            }
            ("GET", ["window", "handles"]) => Ok(json!(session.window_handles()?)),
            ("GET", ["window"]) => Ok(json!(session.current_window()?)),
            ("POST", ["window"]) => {
                let value = fields()?;
                let handle = value
                    .get("handle")
                    .and_then(Value::as_str)
                    .ok_or_else(|| CommandError::invalid("handle must be a string"))?;
                session.switch_window(handle)?;
                Ok(Value::Null)
            }
            ("POST", ["window", "new"]) => {
                let value = fields()?;
                if value.get("type").is_some_and(|value| !value.is_string()) {
                    return Err(CommandError::invalid("window type must be a string"));
                }
                let handle = session.new_window().await?;
                Ok(json!({"handle":handle,"type":"window"}))
            }
            ("DELETE", ["window"]) => {
                let handles = session.close_window().await?;
                if handles.is_empty() {
                    // Release the command lease before retiring the registry entry.
                    drop(session);
                    self.sessions.delete(id).await?;
                }
                Ok(json!(handles))
            }
            ("GET", ["url"] | ["title"]) => {
                let page = session.current_page()?;
                let observation = page.observe().await.map_err(|error| CommandError {
                    code: "unknown error",
                    message: error.to_string(),
                })?;
                Ok(json!(if tail[0] == "url" {
                    observation.page.url
                } else {
                    observation.page.title
                }))
            }
            ("POST", ["frame"]) => {
                use oryn_native::host::webdriver_script::{ELEMENT_KEY, WebDriverFrameTarget};
                let fields = fields()?;
                let id = fields
                    .get("id")
                    .ok_or_else(|| CommandError::invalid("frame id is required"))?;
                let target = match id {
                    Value::Null => WebDriverFrameTarget::Top,
                    Value::Number(number) => {
                        let index =
                            number
                                .as_u64()
                                .filter(|index| *index <= 65535)
                                .ok_or_else(|| {
                                    CommandError::invalid(
                                        "frame index must be an integer from 0 to 65535",
                                    )
                                })?;
                        WebDriverFrameTarget::Index(index as u32)
                    }
                    Value::Object(object) => WebDriverFrameTarget::ElementReference(
                        object
                            .get(ELEMENT_KEY)
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                CommandError::invalid("frame requires an element reference")
                            })?
                            .into(),
                    ),
                    _ => {
                        return Err(CommandError::invalid(
                            "frame id must be null, index, or element reference",
                        ));
                    }
                };
                session.switch_frame(target).await?;
                Ok(Value::Null)
            }
            ("POST", ["frame", "parent"]) => {
                session
                    .switch_frame(oryn_native::host::webdriver_script::WebDriverFrameTarget::Parent)
                    .await?;
                Ok(Value::Null)
            }
            ("GET", ["window", "rect"]) => {
                super::window_rect::get(&session.current_page()?, session.context()?).await
            }
            ("POST", ["window", "rect"]) => {
                super::window_rect::set(&session.current_page()?, session.context()?, &fields()?)
                    .await
            }
            ("POST", ["print"]) => Ok(Value::String(
                super::print::capture(&session.current_page()?, &fields()?).await?,
            )),
            ("GET", ["screenshot"]) => {
                let page = session.current_page()?;
                Ok(Value::String(super::screenshot::capture(&page).await?))
            }
            _ => Err(unknown_command()),
        };
        result.map(ScriptValue::from)
    }
}

fn command_methods(parts: &[&str]) -> Option<&'static [&'static str]> {
    match parts {
        ["status"] => Some(&["GET"]),
        ["session"] => Some(&["POST"]),
        ["session", id] if !id.is_empty() => Some(&["DELETE"]),
        ["session", id, tail @ ..] if !id.is_empty() => match tail {
            ["timeouts"] | ["url"] | ["window", "rect"] | ["alert", "text"] => {
                Some(&["GET", "POST"])
            }
            ["alert", "accept"] | ["alert", "dismiss"] => Some(&["POST"]),
            ["window"] => Some(&["GET", "POST", "DELETE"]),
            ["title"] | ["window", "handles"] | ["screenshot"] => Some(&["GET"]),
            ["actions"] => Some(&["POST", "DELETE"]),
            ["permissions"]
            | ["oryn", "permissions"]
            | ["oryn", "geolocation"]
            | ["custom-handlers", "set-mode"]
            | ["window", "minimize"] => Some(&["POST"]),
            ["oryn", "accessibility", "element" | "node"] => Some(&["POST"]),
            ["cookie"] => Some(&["GET", "DELETE"]),
            ["cookie", _] => Some(&["GET"]),
            ["element"]
            | ["elements"]
            | ["element", _, "element"]
            | ["element", _, "elements"]
            | ["element", _, "click"]
            | ["element", _, "value"]
            | ["oryn", "elements"] => Some(&["POST"]),
            ["element", _, "computedlabel"] | ["element", _, "computedrole"] => Some(&["GET"]),
            ["oryn", "diagnostics", "navigation" | "retirement"] => Some(&["GET"]),
            ["oryn", "qualification", "worker-crash"] | ["oryn", "diagnostics", "placement"] => {
                Some(&["POST"])
            }
            ["window", "new"]
            | ["execute", "sync"]
            | ["execute", "async"]
            | ["print"]
            | ["back"]
            | ["forward"]
            | ["refresh"]
            | ["frame"]
            | ["frame", "parent"] => Some(&["POST"]),
            _ => None,
        },
        _ => None,
    }
}

fn unknown_command() -> CommandError {
    CommandError {
        code: "unknown command",
        message: "unknown WebDriver command".into(),
    }
}

fn error_response(error: CommandError) -> Response {
    let status = match error.code {
        "invalid argument"
        | "invalid selector"
        | "element not interactable"
        | "element click intercepted" => StatusCode::BAD_REQUEST,
        "invalid session id"
        | "no such alert"
        | "no such cookie"
        | "no such window"
        | "no such frame"
        | "no such element"
        | "no such shadow root"
        | "stale element reference"
        | "detached shadow root"
        | "unknown command" => StatusCode::NOT_FOUND,
        "unknown method" => StatusCode::METHOD_NOT_ALLOWED,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    match error.wire_response().to_json_text() {
        Ok(text)=>(status,[(axum::http::header::CONTENT_TYPE,"application/json; charset=utf-8")],text).into_response(),
        Err(_)=>(StatusCode::INTERNAL_SERVER_ERROR,Json(json!({"value":{"error":"unknown error","message":"error response exceeded native serialization limit","stacktrace":""}}))).into_response(),
    }
}

async fn dispatch(State(driver): State<Arc<Driver>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return error_response(CommandError::invalid("invalid command body")),
    };
    let response = match driver.start_command(
        parts.method.to_string(),
        parts.uri.path().into(),
        bytes.to_vec(),
    ) {
        Ok(response) => response,
        Err(error) => return error_response(error),
    };
    match response.await {
        Ok(Ok(value)) => match value.to_json_text() {
            Ok(text) => (
                [(
                    axum::http::header::CONTENT_TYPE,
                    "application/json; charset=utf-8",
                )],
                format!("{{\"value\":{text}}}"),
            )
                .into_response(),
            Err(error) => error_response(CommandError {
                code: "unknown error",
                message: error.message,
            }),
        },
        Ok(Err(error)) => error_response(error),
        Err(_) => error_response(CommandError {
            code: "unknown error",
            message: "native command task failed".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_script_routes_preserve_structured_values_and_callback_completion() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let base = format!(
            "/session/{}",
            created.get("sessionId").unwrap().as_string().unwrap()
        );
        let result=driver.command("POST",&format!("{base}/execute/sync"),
            br#"{"script":"return {sum:arguments[0]+arguments[1],nested:[true,null]}","args":[2,3]}"#).await.unwrap();
        assert_eq!(
            result,
            ScriptValue::from(json!({"sum":5,"nested":[true,null]}))
        );
        let result=driver.command("POST",&format!("{base}/execute/async"),
            br#"{"script":"const done=arguments[arguments.length-1]; setTimeout(()=>done({ready:true}),0)","args":[]}"#).await.unwrap();
        assert_eq!(result, ScriptValue::from(json!({"ready":true})));
        let failure = driver
            .command(
                "POST",
                &format!("{base}/execute/sync"),
                br#"{"script":"throw new Error('deliberate')","args":[]}"#,
            )
            .await
            .unwrap_err();
        assert_eq!(failure.code, "javascript error");
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn script_http_wire_preserves_surrogates_and_actual_session_window_handles() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let base = format!(
            "/session/{}",
            created.get("sessionId").unwrap().as_string().unwrap()
        );
        let path = format!("{base}/execute/sync");
        let value = driver
            .command(
                "POST",
                &path,
                br#"{"script":"return arguments[0]","args":[{"\ud800":"\udfff"}]}"#,
            )
            .await
            .unwrap();
        assert_eq!(value.to_json_text().unwrap(), r#"{"\ud800":"\udfff"}"#);
        let value = driver
            .command("POST", &path, br#"{"script":"return '\ud800'","args":[]}"#)
            .await
            .unwrap();
        assert_eq!(value, ScriptValue::String(vec![0xd800]));
        let handle = driver
            .command("GET", &format!("{base}/window"), b"")
            .await
            .unwrap();
        let value = driver
            .command("POST", &path, br#"{"script":"return window","args":[]}"#)
            .await
            .unwrap();
        assert_eq!(
            value.get(oryn_native::host::webdriver_script::WINDOW_KEY),
            Some(&handle)
        );
        let body = format!(
            "{{\"script\":\"return arguments[0]===window\",\"args\":[{}]}}",
            value.to_json_text().unwrap()
        );
        assert_eq!(
            driver
                .command("POST", &path, body.as_bytes())
                .await
                .unwrap(),
            ScriptValue::Bool(true)
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn native_permission_popup_accepts_only_a_fresh_trusted_choice() {
        let mut options = ContextOptions::loopback_test();
        options.media_devices = oryn_native::MediaDeviceConfiguration::SyntheticFixtureV1;
        let driver = Driver::new(Browser::new(), options, 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let id = created.get("sessionId").unwrap().as_string().unwrap();
        let base = format!("/session/{id}");
        let page = {
            let session = driver.sessions.acquire(&id).await.unwrap();
            session.current_page().unwrap()
        };
        page.load_html(
            "https://prompt.test/",
            "<body><usermedia id=capture tabindex=0></usermedia></body>",
        )
        .await
        .unwrap();
        let script = format!("{base}/execute/sync");
        driver.command("POST",&script,br#"{"script":"globalThis.capture=document.getElementById('capture');globalThis.ready=new Promise((resolve,reject)=>{capture.onstream=()=>resolve(capture.stream.getTracks().map(t=>t.kind));capture.onerror=()=>reject(new Error(capture.error.name+': '+capture.error.message))});return new Promise(resolve=>setTimeout(resolve,1100))","args":[]}"#).await.unwrap();
        let element = driver
            .command(
                "POST",
                &format!("{base}/element"),
                br##"{"using":"css selector","value":"#capture"}"##,
            )
            .await
            .unwrap();
        let element = element
            .get(oryn_native::host::webdriver_script::ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        driver
            .command("POST", &format!("{base}/element/{element}/click"), b"{}")
            .await
            .unwrap();
        assert_eq!(driver.command("POST",&script,br#"{"script":"capture.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));globalThis.authorKeys=0;for(const type of ['keydown','keyup'])document.addEventListener(type,e=>{authorKeys++;e.preventDefault();e.stopImmediatePropagation()},true);return navigator.permissions.query({name:'camera'}).then(p=>[p.state,capture.stream])","args":[]}"#).await.unwrap(),ScriptValue::from(json!(["prompt",null])));
        driver.command("POST",&format!("{base}/actions"),br#"{"actions":[{"type":"key","id":"permission-choice","actions":[{"type":"keyDown","value":"\ue014"},{"type":"keyUp","value":"\ue014"},{"type":"keyDown","value":"\ue007"},{"type":"keyUp","value":"\ue007"}]}]}"#).await.unwrap();
        assert_eq!(
            driver
                .command("POST", &script, br#"{"script":"return ready","args":[]}"#)
                .await
                .unwrap(),
            ScriptValue::from(json!(["audio", "video"]))
        );
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return authorKeys","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::from(json!(0))
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn native_permission_visibility_uses_canvas_alpha_and_filter_group_pixels() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let id = created.get("sessionId").unwrap().as_string().unwrap();
        let base = format!("/session/{id}");
        let page = {
            let session = driver.sessions.acquire(&id).await.unwrap();
            session.current_page().unwrap()
        };
        page.load_html("https://visibility.test/","<body><install id=control></install><canvas id=ink width=300 height=150 style='position:fixed;z-index:20;pointer-events:none'></canvas></body>").await.unwrap();
        let script = format!("{base}/execute/sync");
        driver.command("POST",&script,br#"{"script":"const r=control.getBoundingClientRect();ink.style.left=r.left+'px';ink.style.top=r.top+'px';ink.style.width=r.width+'px';ink.style.height=r.height+'px';globalThis.context=ink.getContext('2d');context.fillRect(0,0,300,150);context.clearRect(0,0,300,150);return new Promise(resolve=>setTimeout(resolve,1200))","args":[]}"#).await.unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"if(!control.isValid)throw new Error(control.invalidReason);return true","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::Bool(true)
        );
        driver.command("POST",&script,br#"{"script":"context.fillRect(0,0,300,150);return new Promise(resolve=>setTimeout(resolve,1200))","args":[]}"#).await.unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return control.isValid","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::Bool(false)
        );
        driver.command("POST",&script,br#"{"script":"ink.style.filter='opacity(0)';return new Promise(resolve=>setTimeout(resolve,1200))","args":[]}"#).await.unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return control.isValid","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::Bool(true)
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn native_option_click_uses_select_container_and_rechecks_author_callback_detachment() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let id = created.get("sessionId").unwrap().as_string().unwrap();
        let base = format!("/session/{id}");
        let page = {
            let session = driver.sessions.acquire(&id).await.unwrap();
            session.current_page().unwrap()
        };
        page.load_html("https://options.test/","<body><select id=s><option>A</option><option id=b>B</option><option id=d disabled>D</option></select><select id=m multiple><option id=c>C</option></select></body>").await.unwrap();
        let script = format!("{base}/execute/sync");
        driver.command("POST",&script,br#"{"script":"globalThis.events=[];for(const type of ['mouseover','mousemove','mousedown','input','change','mouseup','click'])s.addEventListener(type,e=>events.push([e.type,e.isTrusted,e.target===s]));return null","args":[]}"#).await.unwrap();
        let find = |selector: &str| {
            serde_json::to_vec(&json!({"using":"css selector","value":selector})).unwrap()
        };
        let option = driver
            .command("POST", &format!("{base}/element"), &find("#b"))
            .await
            .unwrap();
        let option = option
            .get(oryn_native::host::webdriver_script::ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        driver
            .command("POST", &format!("{base}/element/{option}/click"), b"{}")
            .await
            .unwrap();
        assert_eq!(driver.command("POST",&script,br#"{"script":"return [s.selectedIndex,document.activeElement===s,s.matches(':open'),events.map(e=>e[0]).join(','),events.every(e=>e[1]&&e[2])]","args":[]}"#).await.unwrap(),ScriptValue::from(json!([1,true,false,"mouseover,mousemove,mousedown,input,change,mouseup,click",true])));
        let disabled = driver
            .command("POST", &format!("{base}/element"), &find("#d"))
            .await
            .unwrap();
        let disabled = disabled
            .get(oryn_native::host::webdriver_script::ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        driver
            .command(
                "POST",
                &script,
                br#"{"script":"events=[];return null","args":[]}"#,
            )
            .await
            .unwrap();
        driver
            .command("POST", &format!("{base}/element/{disabled}/click"), b"{}")
            .await
            .unwrap();
        assert_eq!(driver.command("POST",&script,br#"{"script":"return [s.selectedIndex,events.some(e=>e[0]==='input'||e[0]==='change')]","args":[]}"#).await.unwrap(),ScriptValue::from(json!([1,false])));
        let multiple = driver
            .command("POST", &format!("{base}/element"), &find("#c"))
            .await
            .unwrap();
        let multiple = multiple
            .get(oryn_native::host::webdriver_script::ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        for expected in [true, false] {
            driver
                .command("POST", &format!("{base}/element/{multiple}/click"), b"{}")
                .await
                .unwrap();
            assert_eq!(
                driver
                    .command(
                        "POST",
                        &script,
                        br#"{"script":"return c.selected","args":[]}"#
                    )
                    .await
                    .unwrap(),
                ScriptValue::Bool(expected)
            );
        }
        driver.command("POST",&script,br#"{"script":"events=[];globalThis.removedOption=b;s.addEventListener('input',()=>removedOption.remove(),{once:true});return null","args":[]}"#).await.unwrap();
        driver
            .command("POST", &format!("{base}/element/{option}/click"), b"{}")
            .await
            .unwrap();
        assert_eq!(driver.command("POST",&script,br#"{"script":"return [removedOption.isConnected,events.some(e=>e[0]==='change'||e[0]==='mouseup'||e[0]==='click')]","args":[]}"#).await.unwrap(),ScriptValue::from(json!([false,false])));
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn pinned_bluetooth_helper_uses_native_test_state_without_fabricating_devices() {
        let mut options = ContextOptions::loopback_test();
        options.bluetooth_testing = true;
        let driver = Driver::new(Browser::new(), options, 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let id = created.get("sessionId").unwrap().as_string().unwrap();
        let base = format!("/session/{id}");
        let page = {
            let session = driver.sessions.acquire(&id).await.unwrap();
            session.current_page().unwrap()
        };
        page.load_html("https://bluetooth.test/", "<body></body>")
            .await
            .unwrap();
        assert_eq!(driver.command("POST",&format!("{base}/execute/sync"),br#"{"script":"return Promise.all([navigator.bluetooth.getDevices(),navigator.bluetooth.test.allResponsesConsumed(),navigator.bluetooth.getAvailability(),navigator.bluetooth.requestDevice({filters:[]}).then(()=>false,e=>e instanceof TypeError)]).then(([devices,consumed,available,rejected])=>[devices.length,consumed,available,rejected])","args":[]}"#).await.unwrap(),ScriptValue::from(json!([0,true,false,true])));
        driver.shutdown().await.unwrap();
    }

    #[test]
    fn diagnostic_and_fault_routes_have_explicit_method_admission() {
        assert_eq!(
            command_methods(&["session", "owned", "oryn", "diagnostics", "navigation"]),
            Some(&["GET"][..])
        );
        assert_eq!(
            command_methods(&["session", "owned", "oryn", "qualification", "worker-crash"]),
            Some(&["POST"][..])
        );
        assert!(
            command_methods(&["session", "owned", "oryn", "qualification", "kill-pid"]).is_none()
        );
    }

    #[tokio::test]
    async fn worker_crash_control_rejects_caller_selected_targets_before_injection() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let id = created.get("sessionId").unwrap().as_string().unwrap();
        let path = format!("/session/{id}/oryn/qualification/worker-crash");
        for body in [br#"{"pid":1}"#.as_slice(), br#"{"page_id":1}"#.as_slice()] {
            assert_eq!(
                driver.command("POST", &path, body).await.unwrap_err().code,
                "invalid argument"
            );
        }
        assert!(
            driver
                .command("GET", &format!("/session/{id}/window"), b"")
                .await
                .is_ok()
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn navigation_diagnostics_preserve_session_owned_closed_page_history() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 2).unwrap();
        let mut sessions = Vec::new();
        for _ in 0..2 {
            let created = driver
                .command("POST", "/session", br#"{"capabilities":{}}"#)
                .await
                .unwrap();
            sessions.push(
                created
                    .get("sessionId")
                    .unwrap()
                    .as_string()
                    .unwrap()
                    .to_owned(),
            );
        }
        let first = format!("/session/{}", sessions[0]);
        let second = format!("/session/{}", sessions[1]);
        let a = driver
            .command("GET", &format!("{first}/oryn/diagnostics/navigation"), b"")
            .await
            .unwrap();
        let b = driver
            .command("GET", &format!("{second}/oryn/diagnostics/navigation"), b"")
            .await
            .unwrap();
        assert_ne!(a, b);
        driver
            .command("POST", &format!("{first}/window/new"), br#"{"type":"tab"}"#)
            .await
            .unwrap();
        driver
            .command("DELETE", &format!("{first}/window"), b"")
            .await
            .unwrap();
        let evidence = driver
            .command("GET", &format!("{first}/oryn/diagnostics/navigation"), b"")
            .await
            .unwrap();
        let encoded: serde_json::Value =
            serde_json::from_str(&evidence.to_json_text().unwrap()).unwrap();
        assert_eq!(encoded["schema_version"], 1);
        let pages = encoded["pages"].as_array().unwrap();
        assert_eq!(pages.len(), 2);
        assert!(pages.iter().any(|page| page["snapshot"]["retired"] == true));
        assert!(
            pages
                .iter()
                .all(|page| page["snapshot"]["pruned_outcomes"] == 0)
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn native_camera_video_only_and_install_policy_bindings_follow_pinned_callers() {
        let mut options = ContextOptions::loopback_test();
        options.media_devices = oryn_native::MediaDeviceConfiguration::SyntheticFixtureV1;
        let driver = Driver::new(Browser::new(), options, 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let id = created.get("sessionId").unwrap().as_string().unwrap();
        let base = format!("/session/{id}");
        let page = {
            let session = driver.sessions.acquire(&id).await.unwrap();
            session.current_page().unwrap()
        };
        page.load_html("https://camera.test/","<body><camera id=capture></camera><install id=install tabindex=0></install><span tabindex=0 id=focusable>Focus</span></body>").await.unwrap();
        let script = format!("{base}/execute/sync");
        assert_eq!(driver.command("POST",&script,br#"{"script":"globalThis.capture=document.getElementById('capture');const i=document.getElementById('install'),f=document.getElementById('focusable');f.focus();i.focus();return [capture instanceof HTMLCameraElement,capture.track,capture.error,capture.ontrack,i instanceof HTMLInstallElement,document.activeElement===f]","args":[]}"#).await.unwrap(),ScriptValue::from(json!([true,null,null,null,true,true])));
        driver
            .command(
                "POST",
                &format!("{base}/permissions"),
                br#"{"descriptor":{"name":"camera"},"state":"granted"}"#,
            )
            .await
            .unwrap();
        driver
            .command(
                "POST",
                &format!("{base}/permissions"),
                br#"{"descriptor":{"name":"microphone"},"state":"denied"}"#,
            )
            .await
            .unwrap();
        driver.command("POST",&script,br#"{"script":"capture.setConstraints({width:640,height:480,frameRate:{exact:30},echoCancellation:true});globalThis.ready=new Promise((resolve,reject)=>{capture.ontrack=e=>resolve([e.isTrusted,capture.track instanceof MediaStreamTrack,capture.track.kind,capture.track.getSettings().width]);capture.onerror=()=>reject(new Error(capture.error.name+': '+capture.error.message))});return new Promise(resolve=>setTimeout(resolve,1100))","args":[]}"#).await.unwrap();
        let element = driver
            .command(
                "POST",
                &format!("{base}/element"),
                br##"{"using":"css selector","value":"#capture"}"##,
            )
            .await
            .unwrap();
        let element = element
            .get(oryn_native::host::webdriver_script::ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        driver
            .command("POST", &format!("{base}/element/{element}/click"), b"{}")
            .await
            .unwrap();
        assert_eq!(
            driver
                .command("POST", &script, br#"{"script":"return ready","args":[]}"#)
                .await
                .unwrap(),
            ScriptValue::from(json!([true, true, "video", 640]))
        );
        assert_eq!(driver.command("POST",&script,br#"{"script":"capture.track.stop();return new Promise(resolve=>{capture.onerror=e=>resolve([e.isTrusted,capture.error.name]);capture.click()})","args":[]}"#).await.unwrap(),ScriptValue::from(json!([true,"InvalidStateError"])));
        driver
            .command(
                "POST",
                &format!("{base}/permissions"),
                br#"{"descriptor":{"name":"camera"},"state":"denied"}"#,
            )
            .await
            .unwrap();
        driver.command("POST",&script,br#"{"script":"globalThis.denied=new Promise(resolve=>capture.oncancel=e=>resolve([capture.track,capture.error.name,e.isTrusted]));return new Promise(resolve=>setTimeout(resolve,1100))","args":[]}"#).await.unwrap();
        driver
            .command("POST", &format!("{base}/element/{element}/click"), b"{}")
            .await
            .unwrap();
        assert_eq!(
            driver
                .command("POST", &script, br#"{"script":"return denied","args":[]}"#)
                .await
                .unwrap(),
            ScriptValue::from(json!([null, "NotAllowedError", true]))
        );
        assert_eq!(driver.command("POST",&script,br#"{"script":"return Promise.all([false,true].map(sandbox=>new Promise(resolve=>{const frame=document.createElement('iframe');if(sandbox)frame.sandbox='allow-scripts allow-same-origin';frame.allow='web-app-installation *';frame.srcdoc='<install id=i></install>';frame.onload=()=>{const i=frame.contentDocument.getElementById('i');resolve([i instanceof frame.contentWindow.HTMLInstallElement,i.isValid,i.invalidReason])};document.body.append(frame)})))","args":[]}"#).await.unwrap(),ScriptValue::from(json!([[true,false,"illegal_subframe"],[true,false,"illegal_sandbox"]])));
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn native_usermedia_capture_tracks_stop_and_clone_only_with_explicit_fixture_devices() {
        let mut options = ContextOptions::loopback_test();
        options.media_devices = oryn_native::MediaDeviceConfiguration::SyntheticFixtureV1;
        let driver = Driver::new(Browser::new(), options, 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let id = created.get("sessionId").unwrap().as_string().unwrap();
        let base = format!("/session/{id}");
        let page = {
            let session = driver.sessions.acquire(&id).await.unwrap();
            session.current_page().unwrap()
        };
        page.load_html(
            "https://capture.test/",
            "<body><usermedia id=capture></usermedia></body>",
        )
        .await
        .unwrap();
        let script = format!("{base}/execute/sync");
        for name in ["camera", "microphone"] {
            let body =
                serde_json::to_vec(&json!({"descriptor":{"name":name},"state":"granted"})).unwrap();
            driver
                .command("POST", &format!("{base}/permissions"), &body)
                .await
                .unwrap();
        }
        driver.command("POST",&script,br#"{"script":"globalThis.capture=document.getElementById('capture');capture.setConstraints({video:{width:1280,height:720,frameRate:{exact:30}},audio:{sampleSize:16}});globalThis.ready=new Promise((resolve,reject)=>{capture.onstream=resolve;capture.onerror=()=>reject(new Error(capture.error.name+': '+capture.error.message))});return new Promise(resolve=>setTimeout(resolve,600))","args":[]}"#).await.unwrap();
        let element = driver
            .command(
                "POST",
                &format!("{base}/element"),
                br##"{"using":"css selector","value":"#capture"}"##,
            )
            .await
            .unwrap();
        let element = element
            .get(oryn_native::host::webdriver_script::ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        driver
            .command("POST", &format!("{base}/element/{element}/click"), b"{}")
            .await
            .unwrap();
        assert_eq!(driver.command("POST",&script,br#"{"script":"return ready.then(()=>{globalThis.stream=capture.stream;globalThis.video=stream.getVideoTracks()[0];globalThis.audio=stream.getAudioTracks()[0];return [stream instanceof MediaStream,video instanceof MediaStreamTrack,audio.kind,video.kind,video.getSettings().width,video.getSettings().height,stream.active,video.readyState]})","args":[]}"#).await.unwrap(),ScriptValue::from(json!([true,true,"audio","video",1280,720,true,"live"])));
        assert_eq!(driver.command("POST",&script,br#"{"script":"globalThis.player=document.createElement('video');player.muted=true;document.body.append(player);player.srcObject=stream;return player.play().then(()=>{const canvas=document.createElement('canvas');canvas.width=2;canvas.height=2;canvas.getContext('2d').drawImage(player,0,0,2,2);const pixel=canvas.getContext('2d').getImageData(0,0,1,1).data;player.pause();return [player.srcObject===stream,player.videoWidth,player.videoHeight,player.readyState,player.paused,pixel[3]]})","args":[]}"#).await.unwrap(),ScriptValue::from(json!([true,1280,720,4,true,255])));
        driver.command("POST",&script,br#"{"script":"const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');svg.setAttribute('width','80');svg.setAttribute('height','80');const foreign=document.createElementNS(svg.namespaceURI,'foreignObject');foreign.setAttribute('width','80');foreign.setAttribute('height','80');svg.append(foreign);foreign.append(player);player.style.cssText='width:40px;height:40px';document.body.append(svg);return null","args":[]}"#).await.unwrap();
        let capture_snapshot = page.paint_snapshot().await.unwrap();
        let captured_video = capture_snapshot
            .svg_foreign_objects
            .iter()
            .flat_map(|object| object.nodes.iter())
            .find(|node| node.tag == "video")
            .unwrap();
        let captured_frame = &capture_snapshot.images[captured_video.image_resource.unwrap()];
        assert_eq!(captured_frame.media_type, "image/png");
        assert!(captured_frame.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(driver.command("POST",&script,br#"{"script":"let rejected=false;try{player.srcObject={getTracks(){return stream.getTracks()}}}catch(e){rejected=e instanceof TypeError}player.srcObject=null;return [rejected,player.srcObject,player.videoWidth,player.readyState]","args":[]}"#).await.unwrap(),ScriptValue::from(json!([true,null,0,0])));
        assert_eq!(driver.command("POST",&script,br#"{"script":"return video.applyConstraints({width:{exact:320},height:{exact:240}}).then(()=>[video.getSettings().width,video.getSettings().height,video.getCapabilities().width.max,video.getConstraints().width.exact])","args":[]}"#).await.unwrap(),ScriptValue::from(json!([320,240,1920,320])));
        assert_eq!(driver.command("POST",&script,br#"{"script":"return video.applyConstraints({width:{exact:9000}}).then(()=>false,e=>e instanceof OverconstrainedError&&e.constraint==='width'&&video.getSettings().width===320)","args":[]}"#).await.unwrap(),ScriptValue::Bool(true));
        assert_eq!(driver.command("POST",&script,br#"{"script":"globalThis.copy=video.clone();video.stop();audio.stop();return [stream.active,video.readyState,copy.readyState,copy.id!==video.id,new MediaStream([copy]).active]","args":[]}"#).await.unwrap(),ScriptValue::from(json!([false,"ended","live",true,true])));
        driver.command("POST",&script,br#"{"script":"globalThis.ended=new Promise(resolve=>copy.onended=()=>resolve(copy.readyState));return null","args":[]}"#).await.unwrap();
        driver
            .command(
                "POST",
                &format!("{base}/permissions"),
                br#"{"descriptor":{"name":"camera"},"state":"denied"}"#,
            )
            .await
            .unwrap();
        assert_eq!(
            driver
                .command("POST", &script, br#"{"script":"return ended","args":[]}"#)
                .await
                .unwrap(),
            ScriptValue::from(json!("ended"))
        );
        driver.command("POST",&script,br#"{"script":"globalThis.canceled=new Promise(resolve=>{capture.oncancel=e=>resolve([e.isTrusted,capture.stream===null,capture.error.name]);capture.onerror=()=>resolve('unexpected error')});return null","args":[]}"#).await.unwrap();
        driver
            .command("POST", &format!("{base}/element/{element}/click"), b"{}")
            .await
            .unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return canceled","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::from(json!([true, true, "NotAllowedError"]))
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn native_platform_routes_share_permission_location_and_visibility_state() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let id = created.get("sessionId").unwrap().as_string().unwrap();
        let base = format!("/session/{id}");
        let page = {
            let session = driver.sessions.acquire(&id).await.unwrap();
            session.current_page().unwrap()
        };
        page.load_html("https://platform.test/", "<body></body>")
            .await
            .unwrap();
        let script = format!("{base}/execute/sync");
        driver.command("POST",&format!("{base}/permissions"),br#"{"descriptor":{"name":"geolocation"},"state":"granted","origin":"https://platform.test","embeddedOrigin":"https://embedded.test"}"#).await.unwrap();
        assert_eq!(driver.command("POST",&script,br#"{"script":"return navigator.permissions.query({name:'geolocation'}).then(x=>x.state)","args":[]}"#).await.unwrap(),ScriptValue::from(json!("prompt")));
        assert_eq!(driver.command("POST",&format!("{base}/permissions"),br#"{"descriptor":{"name":"geolocation"},"state":"granted","origin":"https://platform.test","embeddedOrigin":false}"#).await.unwrap_err().code,"invalid argument");
        driver
            .command(
                "POST",
                &format!("{base}/permissions"),
                br#"{"descriptor":{"name":"geolocation"},"state":"granted"}"#,
            )
            .await
            .unwrap();
        assert_eq!(driver.command("POST",&script,br#"{"script":"return navigator.permissions.query({name:'geolocation'}).then(x=>x.state)","args":[]}"#).await.unwrap(),ScriptValue::from(json!("granted")));
        let handle = driver
            .command("GET", &format!("{base}/window"), b"")
            .await
            .unwrap()
            .as_string()
            .unwrap();
        let coordinates=serde_json::to_vec(&json!({"contexts":[handle],"coordinates":{"latitude":50,"longitude":0,"accuracy":100}})).unwrap();
        driver
            .command("POST", &format!("{base}/oryn/geolocation"), &coordinates)
            .await
            .unwrap();
        assert_eq!(driver.command("POST",&script,br#"{"script":"return new Promise((resolve,reject)=>navigator.geolocation.getCurrentPosition(p=>resolve([p.coords.latitude,p.coords.longitude,p.coords.accuracy]),reject))","args":[]}"#).await.unwrap(),ScriptValue::from(json!([50,0,100])));
        assert_eq!(driver.command("POST",&script,br#"{"script":"return new Promise((resolve,reject)=>navigator.geolocation.getCurrentPosition(p=>resolve([p instanceof GeolocationPosition,p.coords instanceof GeolocationCoordinates,p.toJSON().coords.latitude,p.coords.toJSON().accuracy]),reject,{maximumAge:10000,timeout:0}))","args":[]}"#).await.unwrap(),ScriptValue::from(json!([true,true,50,100])));
        assert_eq!(driver.command("POST",&script,br#"{"script":"return new Promise(resolve=>navigator.geolocation.getCurrentPosition(()=>resolve('unexpected'),e=>resolve([e instanceof GeolocationPositionError,e.code,e.TIMEOUT]),{timeout:0,maximumAge:0}))","args":[]}"#).await.unwrap(),ScriptValue::from(json!([true,3,3])));
        let frame=driver.command("POST",&script,br#"{"script":"return new Promise(resolve=>{const f=document.createElement('iframe');f.srcdoc='<body></body>';f.onload=()=>resolve(f.contentWindow);document.body.append(f)})","args":[]}"#).await.unwrap();
        let frame = frame
            .get(oryn_native::host::webdriver_script::FRAME_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        let invalid = serde_json::to_vec(
            &json!({"contexts":[frame],"coordinates":{"latitude":4,"longitude":2}}),
        )
        .unwrap();
        assert_eq!(
            driver
                .command("POST", &format!("{base}/oryn/geolocation"), &invalid)
                .await
                .unwrap_err()
                .code,
            "invalid argument"
        );
        driver
            .command("POST", &format!("{base}/window/minimize"), b"{}")
            .await
            .unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return [document.visibilityState,document.hidden]","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::from(json!(["hidden", true]))
        );
        assert_eq!(
            driver
                .command(
                    "POST",
                    &format!("{base}/window/rect"),
                    br#"{"width":false}"#
                )
                .await
                .unwrap_err()
                .code,
            "invalid argument"
        );
        page.load_html("https://platform.test/replaced", "<body>replacement</body>")
            .await
            .unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return document.hidden","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::Bool(true)
        );
        driver
            .command("POST", &format!("{base}/window/rect"), b"{}")
            .await
            .unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return document.hidden","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::Bool(false)
        );
        driver.command("POST",&script,br#"{"script":"globalThis.moves=[];window.onmove=e=>moves.push([e.type,e.isTrusted,e.target===window,screenX,screenY]);return screenLeft","args":[]}"#).await.unwrap();
        let rect = driver
            .command(
                "POST",
                &format!("{base}/window/rect"),
                br#"{"x":100,"y":200}"#,
            )
            .await
            .unwrap();
        assert_eq!(rect.get("x"), Some(&ScriptValue::from(json!(100))));
        assert_eq!(rect.get("y"), Some(&ScriptValue::from(json!(200))));
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return [moves,screenLeft,screenTop]","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::from(json!([[["move", true, true, 100, 200]], 100, 200]))
        );
        driver
            .command(
                "POST",
                &format!("{base}/window/rect"),
                br#"{"x":100,"y":200}"#,
            )
            .await
            .unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return moves.length","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::from(json!(1))
        );
        driver
            .command(
                "POST",
                &format!("{base}/custom-handlers/set-mode"),
                br#"{"mode":"autoAccept"}"#,
            )
            .await
            .unwrap();
        assert_eq!(driver.command("POST",&script,br#"{"script":"navigator.registerProtocolHandler('web+test','/handler?value=%s');try{navigator.registerProtocolHandler('javascript','/handler?value=%s')}catch(e){return e.name}","args":[]}"#).await.unwrap(),ScriptValue::from(json!("SecurityError")));
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn native_element_routes_select_shadow_roots_and_preserve_errors() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let base = format!(
            "/session/{}",
            created.get("sessionId").unwrap().as_string().unwrap()
        );
        let script = format!("{base}/execute/sync");
        driver.command("POST",&script,br#"{"script":"document.body.innerHTML='<div id=host></div>';document.getElementById('host').attachShadow({mode:'closed'}).innerHTML='<button id=target>Go</button>';document.querySelectorAll=()=>[];Element.prototype.querySelectorAll=()=>[]","args":[]}"#).await.unwrap();
        let result = driver
            .command(
                "POST",
                &format!("{base}/oryn/elements"),
                br##"{"selectors":["#host","#target"]}"##,
            )
            .await
            .unwrap();
        let ScriptValue::Array(elements) = result else {
            panic!("expected native element references")
        };
        assert_eq!(elements.len(), 1);
        assert!(
            elements[0]
                .get(oryn_native::host::webdriver_script::ELEMENT_KEY)
                .is_some()
        );
        assert_eq!(
            driver
                .command(
                    "POST",
                    &format!("{base}/element"),
                    br##"{"using":"css selector","value":"#missing"}"##
                )
                .await
                .unwrap_err()
                .code,
            "no such element"
        );
        assert_eq!(
            driver
                .command(
                    "POST",
                    &format!("{base}/elements"),
                    br#"{"using":"css selector","value":"["}"#
                )
                .await
                .unwrap_err()
                .code,
            "invalid selector"
        );
        assert_eq!(
            driver
                .command(
                    "POST",
                    &format!("{base}/oryn/elements"),
                    br#"{"selectors":["body","button"]}"#
                )
                .await
                .unwrap_err()
                .code,
            "no such shadow root"
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn element_send_keys_validates_before_focus_and_preserves_existing_selection() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let base = format!(
            "/session/{}",
            created.get("sessionId").unwrap().as_string().unwrap()
        );
        let script = format!("{base}/execute/sync");
        driver.command("POST",&script,br#"{"script":"document.body.innerHTML='<input id=target value=ab><input id=other>';document.getElementById('other').focus();window.focused=0;document.getElementById('target').addEventListener('focus',()=>window.focused++)","args":[]}"#).await.unwrap();
        let element = driver
            .command(
                "POST",
                &format!("{base}/element"),
                br##"{"using":"css selector","value":"#target"}"##,
            )
            .await
            .unwrap();
        let id = element
            .get(oryn_native::host::webdriver_script::ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        let path = format!("{base}/element/{id}/value");
        let oversized = serde_json::to_vec(&json!({"text":"x".repeat(3000)})).unwrap();
        assert_eq!(
            driver
                .command("POST", &path, &oversized)
                .await
                .unwrap_err()
                .code,
            "invalid argument"
        );
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return [document.activeElement.id,window.focused]","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::from(json!(["other", 0]))
        );
        driver
            .command("POST", &path, br#"{"text":"c"}"#)
            .await
            .unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return document.getElementById('target').value","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::from(json!("abc"))
        );
        driver.command("POST",&script,br#"{"script":"document.getElementById('target').setSelectionRange(0,1)","args":[]}"#).await.unwrap();
        driver
            .command("POST", &path, br#"{"text":"z"}"#)
            .await
            .unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return document.getElementById('target').value","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::from(json!("zbc"))
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn native_frame_routes_select_execute_and_recover_after_detachment() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let base = format!(
            "/session/{}",
            created.get("sessionId").unwrap().as_string().unwrap()
        );
        let script = format!("{base}/execute/sync");
        driver.command("POST",&script,br#"{"script":"document.body.innerHTML='<iframe id=f srcdoc=\"<body>child</body>\"></iframe>'","args":[]}"#).await.unwrap();
        driver
            .command("POST", &format!("{base}/frame"), br#"{"id":0}"#)
            .await
            .unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return document.body.textContent","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::from(json!("child"))
        );
        let frame = driver
            .command("POST", &script, br#"{"script":"return window","args":[]}"#)
            .await
            .unwrap();
        assert!(
            frame
                .get(oryn_native::host::webdriver_script::FRAME_KEY)
                .unwrap()
                .as_string()
                .unwrap()
                .starts_with("frame-")
        );
        driver
            .command("POST", &format!("{base}/frame/parent"), b"{}")
            .await
            .unwrap();
        let element = driver
            .command(
                "POST",
                &script,
                br#"{"script":"return document.getElementById('f')","args":[]}"#,
            )
            .await
            .unwrap();
        let body = format!("{{\"id\":{}}}", element.to_json_text().unwrap());
        driver
            .command("POST", &format!("{base}/frame"), body.as_bytes())
            .await
            .unwrap();
        driver
            .command(
                "POST",
                &script,
                br#"{"script":"frameElement.remove()","args":[]}"#,
            )
            .await
            .unwrap_err();
        driver
            .command("POST", &format!("{base}/frame"), br#"{"id":null}"#)
            .await
            .unwrap();
        assert_eq!(
            driver
                .command("POST", &format!("{base}/frame"), br#"{"id":false}"#)
                .await
                .unwrap_err()
                .code,
            "invalid argument"
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn controlled_navigation_routes_replace_documents_and_history_without_scoped_frame() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let base = format!(
            "/session/{}",
            created.get("sessionId").unwrap().as_string().unwrap()
        );
        let first = "data:text/html,<title>first</title>";
        let second = "data:text/html,<title>second</title>";
        for target in [first, second] {
            let body = serde_json::to_vec(&json!({"url":target})).unwrap();
            assert_eq!(
                driver
                    .command("POST", &format!("{base}/url"), &body)
                    .await
                    .unwrap(),
                ScriptValue::Null
            );
        }
        driver
            .command("POST", &format!("{base}/back"), b"{}")
            .await
            .unwrap();
        assert_eq!(
            driver
                .command("GET", &format!("{base}/title"), b"")
                .await
                .unwrap(),
            ScriptValue::from(json!("first"))
        );
        driver
            .command("POST", &format!("{base}/forward"), b"{}")
            .await
            .unwrap();
        assert_eq!(
            driver
                .command("GET", &format!("{base}/title"), b"")
                .await
                .unwrap(),
            ScriptValue::from(json!("second"))
        );
        driver
            .command("POST", &format!("{base}/refresh"), b"{}")
            .await
            .unwrap();
        assert_eq!(
            driver
                .command("GET", &format!("{base}/title"), b"")
                .await
                .unwrap(),
            ScriptValue::from(json!("second"))
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn modal_prompt_returns_control_and_resumes_same_native_script_after_accept() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let base = format!(
            "/session/{}",
            created.get("sessionId").unwrap().as_string().unwrap()
        );
        let script = format!("{base}/execute/sync");
        let result=driver.command("POST",&script,br#"{"script":"window.answer=prompt('Name','initial');window.finished=true;return 42","args":[]}"#).await.unwrap();
        assert_eq!(result, ScriptValue::Null);
        assert_eq!(
            driver
                .command("GET", &format!("{base}/alert/text"), b"")
                .await
                .unwrap(),
            ScriptValue::from(json!("Name"))
        );
        driver
            .command("POST", &format!("{base}/alert/text"), br#"{"text":"Ada"}"#)
            .await
            .unwrap();
        driver
            .command("POST", &format!("{base}/alert/accept"), b"{}")
            .await
            .unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return [window.answer,window.finished]","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::from(json!(["Ada", true]))
        );
        assert_eq!(
            driver
                .command("GET", &format!("{base}/alert/text"), b"")
                .await
                .unwrap_err()
                .code,
            "no such alert"
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn prompt_routes_preserve_utf16_text_default_and_replacement() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let base = format!(
            "/session/{}",
            created.get("sessionId").unwrap().as_string().unwrap()
        );
        let script = format!("{base}/execute/sync");
        driver
            .command(
                "POST",
                &script,
                br#"{"script":"window.answer=prompt('\ud800','\udfff')","args":[]}"#,
            )
            .await
            .unwrap();
        assert_eq!(
            driver
                .command("GET", &format!("{base}/alert/text"), b"")
                .await
                .unwrap(),
            ScriptValue::String(vec![0xd800])
        );
        driver
            .command("POST", &format!("{base}/alert/accept"), b"{}")
            .await
            .unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return window.answer","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::String(vec![0xdfff])
        );
        driver
            .command(
                "POST",
                &script,
                br#"{"script":"window.answer=prompt('replacement')","args":[]}"#,
            )
            .await
            .unwrap();
        driver
            .command(
                "POST",
                &format!("{base}/alert/text"),
                br#"{"text":"\ud801x\udfff"}"#,
            )
            .await
            .unwrap();
        driver
            .command("POST", &format!("{base}/alert/accept"), b"{}")
            .await
            .unwrap();
        assert_eq!(
            driver
                .command(
                    "POST",
                    &script,
                    br#"{"script":"return window.answer","args":[]}"#
                )
                .await
                .unwrap(),
            ScriptValue::String(vec![0xd801, 120, 0xdfff])
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn default_prompt_handler_dismisses_and_returns_alert_data_before_title() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let base = format!(
            "/session/{}",
            created.get("sessionId").unwrap().as_string().unwrap()
        );
        driver
            .command(
                "POST",
                &format!("{base}/execute/sync"),
                br#"{"script":"alert('pending');document.title='resumed'","args":[]}"#,
            )
            .await
            .unwrap();
        let error = driver
            .command("GET", &format!("{base}/title"), b"")
            .await
            .unwrap_err();
        assert_eq!(error.code, "unexpected alert open");
        assert_eq!(error.response()["value"]["data"], json!({"text":"pending"}));
        assert_eq!(
            driver
                .command("GET", &format!("{base}/title"), b"")
                .await
                .unwrap(),
            ScriptValue::from(json!("resumed"))
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn session_delete_revokes_async_script_without_a_timeout_or_callback() {
        let driver =
            Arc::new(Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap());
        let created = driver
            .command(
                "POST",
                "/session",
                br#"{"capabilities":{"alwaysMatch":{"timeouts":{"script":null}}}}"#,
            )
            .await
            .unwrap();
        let base = format!(
            "/session/{}",
            created.get("sessionId").unwrap().as_string().unwrap()
        );
        let executing = driver
            .start_command(
                "POST".into(),
                format!("{base}/execute/async"),
                br#"{"script":"void 0","args":[]}"#.to_vec(),
            )
            .unwrap();
        // Wait for admission without needing the session lock held by execution.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while driver.scripts.active_count() == 0 && tokio::time::Instant::now() < deadline {
            tokio::task::yield_now().await;
        }
        assert!(driver.scripts.active_count() > 0);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            driver.command("DELETE", &base, b""),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            executing.await.unwrap().unwrap_err().code,
            "invalid session id"
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn command_byte_admission_recovers_without_leaking_command_slots() {
        let driver =
            Arc::new(Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap());
        let held = driver
            .command_bytes
            .clone()
            .acquire_many_owned(64 * 1024 * 1024)
            .await
            .unwrap();
        let result = driver.start_command("GET".into(), "/status".into(), Vec::new());
        assert_eq!(
            result.err().unwrap().message,
            "driver command byte capacity exhausted"
        );
        assert_eq!(driver.command_capacity.available_permits(), 4);
        drop(held);
        let response = driver
            .start_command("GET".into(), "/status".into(), Vec::new())
            .unwrap();
        assert_eq!(
            response.await.unwrap().unwrap().get("ready"),
            Some(&ScriptValue::Bool(true))
        );
        driver.shutdown().await.unwrap();
        assert_eq!(driver.command_bytes.available_permits(), 64 * 1024 * 1024);
    }

    async fn drain_commands(driver: &Driver) {
        let mut tasks = std::mem::take(&mut *driver.commands.lock().unwrap());
        while let Some(result) = tasks.join_next().await {
            driver.record_completion(result);
        }
    }

    #[tokio::test]
    async fn abandoned_creation_response_retires_the_new_native_session() {
        let driver =
            Arc::new(Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap());
        let response = driver
            .start_command(
                "POST".into(),
                "/session".into(),
                br#"{"capabilities":{}}"#.to_vec(),
            )
            .unwrap();
        drop(response);
        drain_commands(&driver).await;
        assert!(driver.sessions.accepting_sessions());
        assert!(driver.task_failure.lock().unwrap().is_none());
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn disconnect_does_not_cancel_an_admitted_existing_session_command() {
        let driver =
            Arc::new(Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap());
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let path = format!(
            "/session/{}/timeouts",
            created.get("sessionId").unwrap().as_string().unwrap()
        );
        let response = driver
            .start_command("POST".into(), path.clone(), br#"{"script":17}"#.to_vec())
            .unwrap();
        drop(response);
        drain_commands(&driver).await;
        assert_eq!(
            driver
                .command("GET", &path, b"")
                .await
                .unwrap()
                .get("script"),
            Some(&ScriptValue::from(json!(17)))
        );
        driver.shutdown().await.unwrap();
        assert!(
            driver
                .start_command("GET".into(), "/status".into(), vec![])
                .is_err()
        );
    }

    #[tokio::test]
    async fn dispatch_creates_queries_and_deletes_a_native_session() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let id = created.get("sessionId").unwrap().as_string().unwrap();
        let base = format!("/session/{id}");
        assert_eq!(
            driver
                .command("GET", &format!("{base}/url"), b"")
                .await
                .unwrap(),
            ScriptValue::from(json!("about:blank"))
        );
        assert_eq!(
            driver
                .command("POST", &format!("{base}/timeouts"), br#"{"script":null}"#)
                .await
                .unwrap(),
            ScriptValue::Null
        );
        assert_eq!(
            driver
                .command("GET", &format!("{base}/timeouts"), b"")
                .await
                .unwrap()
                .get("script"),
            Some(&ScriptValue::Null)
        );
        assert_eq!(
            driver
                .command("POST", &format!("{base}/window"), br#"{"handle":42}"#)
                .await
                .unwrap_err()
                .code,
            "invalid argument"
        );
        assert_eq!(
            driver
                .command("POST", &format!("{base}/actions"), br#"{"actions":[]}"#)
                .await
                .unwrap(),
            ScriptValue::Null
        );
        driver.command("DELETE", &base, b"").await.unwrap();
        assert_eq!(
            driver
                .command("GET", &format!("{base}/window"), b"")
                .await
                .unwrap_err()
                .code,
            "invalid session id"
        );
        driver.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn action_http_routes_retain_pressed_keys_and_release_them() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        let created = driver
            .command("POST", "/session", br#"{"capabilities":{}}"#)
            .await
            .unwrap();
        let base = format!(
            "/session/{}",
            created.get("sessionId").unwrap().as_string().unwrap()
        );
        driver.command("POST",&format!("{base}/execute/sync"),br#"{"script":"globalThis.keys=[];document.addEventListener('keydown',e=>{keys.push([e.type,e.key,e.repeat,e.isTrusted]);e.preventDefault()});document.addEventListener('keyup',e=>keys.push([e.type,e.key,e.repeat,e.isTrusted]));return null","args":[]}"#).await.unwrap();
        let down=br#"{"actions":[{"type":"key","id":"keyboard","actions":[{"type":"keyDown","value":"a"}]}]}"#;
        driver
            .command("POST", &format!("{base}/actions"), down)
            .await
            .unwrap();
        driver
            .command("POST", &format!("{base}/actions"), down)
            .await
            .unwrap();
        assert_eq!(
            driver
                .command("DELETE", &format!("{base}/actions"), b"")
                .await
                .unwrap(),
            ScriptValue::Null
        );
        let result = driver
            .command(
                "POST",
                &format!("{base}/execute/sync"),
                br#"{"script":"return keys","args":[]}"#,
            )
            .await
            .unwrap();
        assert_eq!(
            result,
            ScriptValue::from(json!([
                ["keydown", "a", false, true],
                ["keydown", "a", true, true],
                ["keyup", "a", false, true]
            ]))
        );
        assert_eq!(driver.command("POST",&format!("{base}/actions"),br#"{"actions":[{"type":"key","id":"bad","actions":[{"type":"pointerDown","button":0}]}]}"#).await.unwrap_err().code,"invalid argument");
        driver.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn routing_errors_precede_session_lookup() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        assert_eq!(
            driver
                .command("POST", "/status", b"{}")
                .await
                .unwrap_err()
                .code,
            "unknown method"
        );
        assert_eq!(
            driver
                .command("PUT", "/session/absent/window", b"{}")
                .await
                .unwrap_err()
                .code,
            "unknown method"
        );
        assert_eq!(
            driver
                .command("GET", "/session/absent/window", b"")
                .await
                .unwrap_err()
                .code,
            "invalid session id"
        );
        assert_eq!(
            driver
                .command("GET", "/session/absent/unknown", b"")
                .await
                .unwrap_err()
                .code,
            "unknown command"
        );
        driver.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn malformed_capabilities_never_consume_session_capacity() {
        let driver = Driver::new(Browser::new(), ContextOptions::loopback_test(), 1).unwrap();
        for body in [
            b"null".as_slice(),
            b"{}",
            b"{",
            br#"{"capabilities":{"firstMatch":[]}}"#,
        ] {
            assert_eq!(
                driver
                    .command("POST", "/session", body)
                    .await
                    .unwrap_err()
                    .code,
                "invalid argument"
            );
        }
        assert_eq!(
            driver
                .command("GET", "/status", b"")
                .await
                .unwrap()
                .get("ready"),
            Some(&ScriptValue::Bool(true))
        );
        driver.shutdown().await.unwrap();
    }
}
