//! One WebDriver session owns one native browser context and its window handles.
//! The transport must serialize commands for each session; handles are references,
//! never authorization credentials for another session or native context.
use super::{CommandError, timeouts::Timeouts};
use oryn_common::v3::{PageId, PageStatus};
use oryn_native::host::webdriver_script::{
    AutomationRealmSnapshot, FRAME_KEY, FrameCommand, FrameResponse, ScriptContext, ScriptValue,
    WINDOW_KEY, WebDriverFrameTarget, WindowHandleBinding,
};
use oryn_native::{Browser, BrowserContext, ContextOptions, PageHandle};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub struct Session {
    #[cfg(test)]
    lookup_wait_observer: Option<tokio::sync::oneshot::Sender<()>>,
    context: Option<BrowserContext>,
    current: Option<PageHandle>,
    handles: BTreeMap<String, PageId>,
    next_handle: u64,
    handle_namespace: String,
    element_input_namespace: String,
    selected_frame: Option<AutomationRealmSnapshot>,
    next_frame_handle: u64,
    frame_references: BTreeMap<String, (PageId, String)>,
    pub capabilities: Map<String, Value>,
    pub timeouts: Timeouts,
    pub prompt_policy: super::prompts::PromptPolicy,
}

fn no_window() -> CommandError {
    CommandError {
        code: "no such window",
        message: "window is no longer open in this session".into(),
    }
}

fn runtime_error(error: oryn_native::RuntimeError) -> CommandError {
    let code = match &error {
        oryn_native::RuntimeError::Upload(_) => "invalid argument",
        oryn_native::RuntimeError::Closed
        | oryn_native::RuntimeError::PageTaskYield
        | oryn_native::RuntimeError::PageNotFound(_) => "no such window",
        _ => "unknown error",
    };
    CommandError {
        code,
        message: error.to_string(),
    }
}

async fn close_context(context: BrowserContext) -> Result<(), CommandError> {
    let mut errors = Vec::new();
    let graceful = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        for page in context.pages() {
            if page.status != PageStatus::Closed
                && let Err(error) = context.close_page(page.page.page_id).await
            {
                let already_closed = matches!(error, oryn_native::RuntimeError::PageNotFound(_))
                    && context.pages().iter().any(|entry| {
                        entry.page.page_id == page.page.page_id
                            && entry.status == PageStatus::Closed
                    });
                if !already_closed {
                    errors.push(error.to_string());
                }
            }
        }
    })
    .await;
    if graceful.is_err() {
        errors.push("graceful session close exceeded its deadline".into());
    }
    // Logical close is not process retirement. This also drains workers belonging
    // to pages already marked closed/crashed and detached blocking requests.
    if let Err(error) = context.retire_workers().await {
        errors.push(error.to_string());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(CommandError {
            code: "unknown error",
            message: format!("native session teardown failed: {}", errors.join("; ")),
        })
    }
}

impl Session {
    pub(super) fn timing_identity(&self, webdriver_session_id: &str) -> Value {
        serde_json::json!({"webdriver_session_id":webdriver_session_id,
            "context_id":self.context.as_ref().map(BrowserContext::id),
            "page_id":self.current.as_ref().map(PageHandle::id)})
    }
    pub async fn create(
        browser: &Browser,
        options: ContextOptions,
        capabilities: Map<String, Value>,
    ) -> Result<Self, CommandError> {
        let mut creation_timing =
            oryn_native::native_timing::Scope::start("native_session_create", Value::Null);
        // Validate settings before creating a context or starting any page.
        let timeouts = capabilities
            .get("timeouts")
            .map(|value| Timeouts::default().updated(value))
            .transpose()?
            .unwrap_or_default();
        let prompt_policy = capabilities
            .get("unhandledPromptBehavior")
            .map(super::prompts::PromptPolicy::parse)
            .transpose()?
            .unwrap_or_default();
        let mut entropy = [0u8; 24];
        getrandom::fill(&mut entropy).map_err(|_| {
            CommandError::session_not_created("window identity entropy unavailable")
        })?;
        let handle_namespace = entropy
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        getrandom::fill(&mut entropy)
            .map_err(|_| CommandError::session_not_created("element input entropy unavailable"))?;
        let element_input_namespace = entropy
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut context_timing =
            oryn_native::native_timing::Scope::start("native_context_create", Value::Null);
        let context = browser.new_audited_context_with_isolated_auxiliaries(options);
        let page = context.new_page();
        let identity = serde_json::json!({"context_id":context.id(),"page_id":page.id()});
        context_timing.set_identity(identity.clone());
        context_timing.finish(true);
        creation_timing.set_identity(identity.clone());
        oryn_native::native_timing::point("native_context_created", identity.clone());
        let mut page_timing =
            oryn_native::native_timing::Scope::start("native_initial_page", identity.clone());
        if let Err(error) = page.goto("about:blank").await {
            let cleanup = close_context(context).await;
            return Err(CommandError {
                code: "session not created",
                message: match cleanup {
                    Ok(()) => format!("initial page failed: {error}"),
                    Err(cleanup) => format!("initial page failed: {error}; {}", cleanup.message),
                },
            });
        }
        page_timing.finish(true);
        let mut session = Self {
            #[cfg(test)]
            lookup_wait_observer: None,
            context: Some(context),
            current: Some(page),
            handles: BTreeMap::new(),
            next_handle: 1,
            handle_namespace,
            element_input_namespace,
            selected_frame: None,
            next_frame_handle: 1,
            frame_references: BTreeMap::new(),
            capabilities,
            timeouts,
            prompt_policy,
        };
        session.window_handles()?;
        creation_timing.finish(true);
        oryn_native::native_timing::point("native_session_created", identity);
        Ok(session)
    }

    pub(super) fn context(&self) -> Result<&BrowserContext, CommandError> {
        self.context.as_ref().ok_or_else(|| CommandError {
            code: "invalid session id",
            message: "session is deleted".into(),
        })
    }

    pub fn current_page(&self) -> Result<PageHandle, CommandError> {
        let context = self.context()?;
        let page = self.current.as_ref().ok_or_else(no_window)?;
        if !context.pages().iter().any(|item| {
            item.page.page_id == page.page_id()
                && matches!(
                    item.status,
                    PageStatus::Open | PageStatus::AttentionRequired
                )
        }) {
            return Err(no_window());
        }
        Ok(page.clone())
    }

    pub fn window_handles(&mut self) -> Result<Vec<String>, CommandError> {
        let pages = self.context()?.pages();
        let live = pages
            .iter()
            .filter(|page| {
                matches!(
                    page.status,
                    PageStatus::Open | PageStatus::AttentionRequired
                )
            })
            .map(|page| page.page.page_id)
            .collect::<Vec<_>>();
        // Closed handles are never reused. Popups created by script are imported
        // from this context's authoritative registry, never from global IDs.
        self.handles.retain(|_, id| live.contains(id));
        for id in &live {
            if !self.handles.values().any(|existing| existing == id) {
                let handle = format!("window-{}-{}", self.handle_namespace, self.next_handle);
                self.next_handle = self.next_handle.checked_add(1).ok_or_else(|| {
                    CommandError::session_not_created("window handle space exhausted")
                })?;
                self.handles.insert(handle, *id);
            }
        }
        Ok(self.handles.keys().cloned().collect())
    }

    pub fn current_window(&mut self) -> Result<String, CommandError> {
        let id = self.current_page()?.page_id();
        self.window_handles()?;
        self.handles
            .iter()
            .find(|(_, value)| **value == id)
            .map(|(handle, _)| handle.clone())
            .ok_or_else(no_window)
    }

    pub fn switch_window(&mut self, handle: &str) -> Result<(), CommandError> {
        self.window_handles()?;
        let id = *self.handles.get(handle).ok_or_else(no_window)?;
        let page = self.context()?.activate_page(id).map_err(runtime_error)?;
        self.current = Some(page);
        self.selected_frame = None;
        Ok(())
    }

    async fn rollback_new_window(
        &mut self,
        page: PageId,
        previous: PageId,
        mut failure: CommandError,
    ) -> CommandError {
        let Some(context) = self.context.clone() else {
            return failure;
        };
        let cleanup =
            tokio::time::timeout(std::time::Duration::from_secs(5), context.close_page(page)).await;
        let cleanup_error = match cleanup {
            Ok(Ok(())) => None,
            Ok(Err(error)) => {
                let closed = context
                    .pages()
                    .iter()
                    .any(|entry| entry.page.page_id == page && entry.status == PageStatus::Closed);
                if closed && matches!(error, oryn_native::RuntimeError::PageNotFound(_)) {
                    None
                } else {
                    Some(error.to_string())
                }
            }
            Err(_) => Some("new-window cleanup exceeded its deadline".into()),
        };
        if let Some(error) = cleanup_error {
            // A failed rollback cannot leave an unreported worker owned by an
            // otherwise usable session. Revoke the context and drain all workers.
            failure
                .message
                .push_str(&format!("; new-window cleanup failed: {error}"));
            if let Err(error) = self.delete().await {
                failure
                    .message
                    .push_str(&format!("; session retirement failed: {}", error.message));
            }
            return failure;
        }
        self.handles.retain(|_, id| *id != page);
        match context.activate_page(previous) {
            Ok(previous) => self.current = Some(previous),
            Err(error) => {
                // Script may have closed the old current window while the new
                // one initialized. Never keep a stale current handle silently.
                self.current = None;
                failure
                    .message
                    .push_str(&format!("; previous window restore failed: {error}"));
            }
        }
        failure
    }

    pub async fn new_window(&mut self) -> Result<String, CommandError> {
        let current = self.current_page()?;
        let context = self.context()?.clone();
        let page = context.new_page();
        if let Err(error) = page.goto("about:blank").await {
            return Err(self
                .rollback_new_window(page.page_id(), current.page_id(), runtime_error(error))
                .await);
        }
        // New Window does not select its result as the current browsing context.
        if let Err(error) = context.activate_page(current.page_id()) {
            return Err(self
                .rollback_new_window(page.page_id(), current.page_id(), runtime_error(error))
                .await);
        }
        let result = self.window_handles().and_then(|_| {
            self.handles
                .iter()
                .find(|(_, id)| **id == page.page_id())
                .map(|(handle, _)| handle.clone())
                .ok_or_else(no_window)
        });
        match result {
            Ok(handle) => Ok(handle),
            Err(error) => Err(self
                .rollback_new_window(page.page_id(), current.page_id(), error)
                .await),
        }
    }

    pub async fn close_window(&mut self) -> Result<Vec<String>, CommandError> {
        let page = self.current_page()?;
        self.context()?
            .close_page(page.page_id())
            .await
            .map_err(runtime_error)?;
        self.current = None;
        self.selected_frame = None;
        let handles = self.window_handles()?;
        if handles.is_empty() {
            self.delete().await?;
        }
        Ok(handles)
    }

    pub fn reset_frame_for_navigation(&mut self) {
        self.selected_frame = None;
    }

    pub async fn set_permission(&mut self, value: &Value) -> Result<(), CommandError> {
        let descriptor = serde_json::from_value(
            value
                .get("descriptor")
                .cloned()
                .ok_or_else(|| CommandError::invalid("permission descriptor required"))?,
        )
        .map_err(|_| CommandError::invalid("invalid permission descriptor"))?;
        let permission = serde_json::from_value(
            value
                .get("state")
                .cloned()
                .ok_or_else(|| CommandError::invalid("permission state required"))?,
        )
        .map_err(|_| CommandError::invalid("invalid permission state"))?;
        self.script_context().await?;
        let (origin, default_embedded) = if let Some(origin) = value.get("origin") {
            let origin = origin
                .as_str()
                .ok_or_else(|| CommandError::invalid("permission origin must be a string"))?
                .to_owned();
            (origin.clone(), origin)
        } else {
            let page = self.current_page()?;
            let snapshot = if let Some(frame) = &self.selected_frame {
                frame.clone()
            } else {
                let FrameResponse::Inventory(frames) = page
                    .webdriver_frames(FrameCommand::Inventory)
                    .await
                    .map_err(super::script::error)?
                else {
                    return Err(no_window());
                };
                frames
                    .into_iter()
                    .find(|frame| frame.live && frame.parent_realm.is_none())
                    .ok_or_else(no_window)?
            };
            self.context()?
                .webdriver_permission_origins(page.page_id(), &snapshot)
                .map_err(runtime_error)?
        };
        let embedded = value
            .get("embeddedOrigin")
            .map(|value| {
                value.as_str().ok_or_else(|| {
                    CommandError::invalid("embedded permission origin must be a string")
                })
            })
            .transpose()?
            .unwrap_or(&default_embedded);
        self.context()?
            .webdriver_set_permission_partitioned(&origin, embedded, descriptor, permission)
            .map_err(|error| match error {
                oryn_native::RuntimeError::Closed => runtime_error(error),
                error => CommandError::invalid(error.to_string()),
            })
    }
    pub fn set_geolocation(&mut self, value: &Value) -> Result<(), CommandError> {
        use oryn_native::platform_control_contract::{Coordinates, PositionError};
        if value.get("coordinates").is_some() && value.get("error").is_some() {
            return Err(CommandError::invalid(
                "coordinates and error are mutually exclusive",
            ));
        }
        let position = if let Some(error) = value.get("error") {
            if error.get("type").and_then(Value::as_str) != Some("positionUnavailable") {
                return Err(CommandError::invalid("unsupported location error override"));
            }
            Some(Err(PositionError {
                code: 2,
                message: "Position unavailable by native automation override".into(),
            }))
        } else {
            match value.get("coordinates") {
                Some(Value::Null) | None => None,
                Some(coordinates) => {
                    let coordinates: Coordinates = serde_json::from_value(coordinates.clone())
                        .map_err(|_| CommandError::invalid("invalid coordinates"))?;
                    coordinates.validate().map_err(CommandError::invalid)?;
                    Some(Ok(coordinates))
                }
            }
        };
        let contexts = value
            .get("contexts")
            .and_then(Value::as_array)
            .filter(|values| !values.is_empty() && values.len() <= 64)
            .ok_or_else(|| CommandError::invalid("nonempty location contexts required"))?;
        let mut pages = std::collections::BTreeSet::new();
        for value in contexts {
            let handle = value
                .as_str()
                .ok_or_else(|| CommandError::invalid("location context must be a native handle"))?;
            if let Some((page, frame)) = self.frame_references.get(handle) {
                if !self
                    .context()?
                    .automation_frame_live(*page, frame)
                    .map_err(runtime_error)?
                {
                    return Err(no_window());
                }
                return Err(CommandError::invalid(
                    "geolocation override context must be top level",
                ));
            }
            let page = *self.handles.get(handle).ok_or_else(no_window)?;
            if !self
                .context()?
                .pages()
                .iter()
                .any(|entry| entry.page.page_id == page && entry.status != PageStatus::Closed)
            {
                return Err(no_window());
            }
            pages.insert(page);
        }
        self.context()?
            .webdriver_set_geolocation_many(&pages, position)
            .map_err(runtime_error)
    }
    pub fn set_registration_mode(&mut self, value: &Value) -> Result<(), CommandError> {
        let mode = serde_json::from_value(
            value
                .get("mode")
                .cloned()
                .ok_or_else(|| CommandError::invalid("registration mode required"))?,
        )
        .map_err(|_| CommandError::invalid("invalid registration mode"))?;
        self.context()?
            .webdriver_set_registration_mode(self.current_page()?.page_id(), mode)
            .map_err(runtime_error)
    }
    pub fn set_window_hidden(&mut self, hidden: bool) -> Result<(), CommandError> {
        self.context()?
            .webdriver_set_visibility(self.current_page()?.page_id(), hidden)
            .map_err(runtime_error)
    }

    pub async fn cookies(&mut self, delete: bool) -> Result<Vec<Value>, CommandError> {
        // Refresh the exact selected lifetime before deriving its URL. Request
        // JSON supplies no cookie origin or parent cookie-store authority.
        self.script_context().await?;
        let page = self.current_page()?;
        let snapshot = if let Some(frame) = &self.selected_frame {
            frame.clone()
        } else {
            let FrameResponse::Inventory(frames) = page
                .webdriver_frames(FrameCommand::Inventory)
                .await
                .map_err(super::script::error)?
            else {
                return Err(no_window());
            };
            frames
                .into_iter()
                .find(|frame| frame.live && frame.parent_realm.is_none())
                .ok_or_else(no_window)?
        };
        self.context
            .as_ref()
            .ok_or_else(no_window)?
            .webdriver_cookies(page.page_id(), &snapshot, delete)
            .map_err(runtime_error)
    }

    pub fn element_input_source(&self, keyboard: bool) -> String {
        format!(
            "native-element-{}-{}",
            self.element_input_namespace,
            if keyboard { "key" } else { "pointer" }
        )
    }

    pub async fn upload_element_files(
        &mut self,
        reference: String,
        text: &[u16],
    ) -> Result<ScriptValue, CommandError> {
        let text = String::from_utf16(text)
            .map_err(|_| CommandError::invalid("file paths must be valid Unicode"))?;
        if text.len() > 1024 * 1024 {
            return Err(CommandError::invalid("file path payload limit"));
        }
        let paths = text
            .split('\n')
            .map(std::path::PathBuf::from)
            .collect::<Vec<_>>();
        self.script_context().await?;
        let page = self.current_page()?;
        let snapshot = if let Some(frame) = &self.selected_frame {
            frame.clone()
        } else {
            let FrameResponse::Inventory(frames) = page
                .webdriver_frames(FrameCommand::Inventory)
                .await
                .map_err(super::script::error)?
            else {
                return Err(no_window());
            };
            frames
                .into_iter()
                .find(|frame| frame.live && frame.parent_realm.is_none())
                .ok_or_else(no_window)?
        };
        let files = self
            .context
            .as_ref()
            .ok_or_else(no_window)?
            .webdriver_upload_files_in_context(page.page_id(), &snapshot, &paths)
            .map_err(runtime_error)?;
        self.element_command(
            oryn_native::host::webdriver_script::ElementCommand::SetFiles { reference, files },
        )
        .await
    }

    pub async fn find_elements(
        &mut self,
        root: Option<String>,
        selectors: Vec<String>,
        cancellation: std::sync::Arc<super::registry::SessionCancellation>,
    ) -> Result<ScriptValue, CommandError> {
        let page = self.current_page()?;
        let start = tokio::time::Instant::now();
        let deadline = self
            .timeouts
            .implicit
            .map(|milliseconds| {
                start
                    .checked_add(std::time::Duration::from_millis(milliseconds))
                    .ok_or_else(|| CommandError::invalid("implicit timeout overflow"))
            })
            .transpose()?;
        let response = tokio::select! {biased;_=cancellation.cancelled()=>return Err(CommandError{code:"invalid session id",message:"session retired during element lookup".into()}),response=page.webdriver_frames(FrameCommand::Inventory)=>response.map_err(super::script::error)?};
        let FrameResponse::Inventory(inventory) = response else {
            return Err(CommandError {
                code: "unknown error",
                message: "native frame inventory mismatch".into(),
            });
        };
        let current = match &self.selected_frame {
            Some(selected) => inventory
                .into_iter()
                .find(|entry| entry.same_document_as(selected)),
            None => inventory
                .into_iter()
                .find(|entry| entry.live && entry.parent_realm.is_none()),
        }
        .ok_or_else(|| CommandError {
            code: "no such frame",
            message: "selected lookup document retired".into(),
        })?;
        loop {
            let command = oryn_native::host::webdriver_script::ElementCommand::Find {
                root: root.clone(),
                selectors: selectors.clone(),
            };
            let response = tokio::select! {biased;_=cancellation.cancelled()=>return Err(CommandError{code:"invalid session id",message:"session retired during element lookup".into()}),response=page.webdriver_frames(FrameCommand::Element{current:Some(current.clone()),command})=>response.map_err(super::script::error)?};
            let FrameResponse::Value(value) = response else {
                return Err(CommandError {
                    code: "unknown error",
                    message: "native lookup response mismatch".into(),
                });
            };
            let ScriptValue::Array(values) = &value else {
                return Err(CommandError {
                    code: "unknown error",
                    message: "native lookup returned non-array".into(),
                });
            };
            if !values.is_empty()
                || deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
            {
                return self.finalize_script_result(value);
            }
            #[cfg(test)]
            if let Some(observer) = self.lookup_wait_observer.take() {
                let _ = observer.send(());
            }
            let delay = deadline
                .map(|deadline| {
                    deadline
                        .saturating_duration_since(tokio::time::Instant::now())
                        .min(std::time::Duration::from_millis(25))
                })
                .unwrap_or(std::time::Duration::from_millis(25));
            tokio::select! {biased;_=cancellation.cancelled()=>return Err(CommandError{code:"invalid session id",message:"session retired during element lookup".into()}),_=tokio::time::sleep(delay)=>{}}
        }
    }

    pub async fn element_command(
        &mut self,
        command: oryn_native::host::webdriver_script::ElementCommand,
    ) -> Result<ScriptValue, CommandError> {
        let page = self.current_page()?;
        let response = page
            .webdriver_frames(FrameCommand::Element {
                current: self.selected_frame.clone(),
                command,
            })
            .await
            .map_err(super::script::error)?;
        let FrameResponse::Value(value) = response else {
            return Err(CommandError {
                code: "unknown error",
                message: "native element response mismatch".into(),
            });
        };
        self.finalize_script_result(value)
    }

    pub async fn switch_frame(&mut self, target: WebDriverFrameTarget) -> Result<(), CommandError> {
        let response = self
            .current_page()?
            .webdriver_frames(FrameCommand::Select {
                current: self.selected_frame.clone(),
                target,
            })
            .await
            .map_err(super::script::error)?;
        let FrameResponse::Selected(selected) = response else {
            return Err(CommandError {
                code: "unknown error",
                message: "native frame response mismatch".into(),
            });
        };
        self.selected_frame = if selected.parent_realm.is_none() {
            None
        } else {
            Some(selected)
        };
        Ok(())
    }

    /// Resolve only this session's live handles. Author JSON cannot supply PageIds.
    /// Work on a clone so a rejected nested reference never partially authorizes input.
    pub fn authorize_script_arguments(
        &mut self,
        value: &ScriptValue,
    ) -> Result<ScriptValue, CommandError> {
        self.window_handles()?;
        fn visit(session: &Session, value: &ScriptValue) -> Result<ScriptValue, CommandError> {
            Ok(match value {
                ScriptValue::WindowPage(_) | ScriptValue::WindowRealm { .. } => {
                    return Err(CommandError::invalid(
                        "native window identity is not an author argument",
                    ));
                }
                ScriptValue::Object(entries) => {
                    if let Some(handle) = value.get(WINDOW_KEY) {
                        let handle = handle.as_string().map_err(super::script::error)?;
                        ScriptValue::WindowPage(
                            *session.handles.get(&handle).ok_or_else(no_window)?,
                        )
                    } else if let Some(handle) = value.get(FRAME_KEY) {
                        let handle = handle.as_string().map_err(super::script::error)?;
                        let (page, frame) = session
                            .frame_references
                            .get(&handle)
                            .ok_or_else(no_window)?;
                        if !session
                            .context()?
                            .automation_frame_live(*page, frame)
                            .map_err(runtime_error)?
                        {
                            return Err(no_window());
                        }
                        ScriptValue::WindowRealm {
                            page: *page,
                            frame: frame.clone(),
                        }
                    } else {
                        ScriptValue::Object(
                            entries
                                .iter()
                                .map(|(key, value)| Ok((key.clone(), visit(session, value)?)))
                                .collect::<Result<_, CommandError>>()?,
                        )
                    }
                }
                ScriptValue::Array(values) => ScriptValue::Array(
                    values
                        .iter()
                        .map(|value| visit(session, value))
                        .collect::<Result<_, _>>()?,
                ),
                other => other.clone(),
            })
        }
        visit(self, value)
    }

    fn frame_reference_handle(
        &mut self,
        page: PageId,
        frame: &str,
    ) -> Result<String, CommandError> {
        if !self
            .context()?
            .automation_frame_live(page, frame)
            .map_err(runtime_error)?
        {
            return Err(no_window());
        }
        if let Some((handle, _)) = self
            .frame_references
            .iter()
            .find(|(_, target)| target.0 == page && target.1 == frame)
        {
            return Ok(handle.clone());
        }
        let handle = format!("frame-{}-{}", self.handle_namespace, self.next_frame_handle);
        self.next_frame_handle =
            self.next_frame_handle
                .checked_add(1)
                .ok_or_else(|| CommandError {
                    code: "unknown error",
                    message: "frame handle space exhausted".into(),
                })?;
        self.frame_references
            .insert(handle.clone(), (page, frame.into()));
        Ok(handle)
    }

    /// Import only native references still owned by this session's context.
    pub fn finalize_script_result(
        &mut self,
        value: ScriptValue,
    ) -> Result<ScriptValue, CommandError> {
        self.window_handles()?;
        fn visit(session: &mut Session, value: ScriptValue) -> Result<ScriptValue, CommandError> {
            Ok(match value {
                ScriptValue::WindowPage(id) => {
                    let handle = session
                        .handles
                        .iter()
                        .find(|(_, page)| **page == id)
                        .map(|(handle, _)| handle.clone())
                        .ok_or_else(no_window)?;
                    ScriptValue::Object(vec![(
                        WINDOW_KEY.encode_utf16().collect(),
                        ScriptValue::String(handle.encode_utf16().collect()),
                    )])
                }
                ScriptValue::WindowRealm { page, frame } => {
                    let handle = session.frame_reference_handle(page, &frame)?;
                    ScriptValue::Object(vec![(
                        FRAME_KEY.encode_utf16().collect(),
                        ScriptValue::String(handle.encode_utf16().collect()),
                    )])
                }
                ScriptValue::Object(entries) => ScriptValue::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| Ok((key, visit(session, value)?)))
                        .collect::<Result<_, CommandError>>()?,
                ),
                ScriptValue::Array(values) => ScriptValue::Array(
                    values
                        .into_iter()
                        .map(|value| visit(session, value))
                        .collect::<Result<_, _>>()?,
                ),
                other => other,
            })
        }
        visit(self, value)
    }

    /// Bind script/action context in the native command that uses it. Native
    /// frame results carry connection identities and are assigned session
    /// handles by finalize_script_result; they need no preliminary inventory.
    pub async fn execution_context(&mut self) -> Result<ScriptContext, CommandError> {
        #[cfg(unix)]
        {
            self.current_page()?;
            Ok(ScriptContext {
                selected: self.selected_frame.clone(),
                handles: Vec::new(),
                top_window_handle: Some(self.current_window()?),
            })
        }
        #[cfg(not(unix))]
        self.script_context().await
    }

    pub async fn script_context(&mut self) -> Result<ScriptContext, CommandError> {
        let page = self.current_page()?;
        let page_id = page.page_id();
        let window = self.current_window()?;
        let response = page
            .webdriver_frames(FrameCommand::Inventory)
            .await
            .map_err(super::script::error)?;
        let FrameResponse::Inventory(inventory) = response else {
            return Err(CommandError {
                code: "unknown error",
                message: "native frame inventory mismatch".into(),
            });
        };
        let root = inventory
            .iter()
            .find(|entry| entry.live && entry.parent_realm.is_none())
            .ok_or_else(no_window)?;
        if let Some(selected) = &self.selected_frame
            && !inventory
                .iter()
                .any(|entry| entry.same_document_as(selected))
        {
            return Err(CommandError {
                code: "no such frame",
                message: "selected frame document is no longer current".into(),
            });
        }
        let mut handles = vec![WindowHandleBinding {
            realm: root.realm,
            document_generation: root.document_generation,
            stream_epoch: root.stream_epoch,
            handle: window,
            top_level: true,
        }];
        for frame in inventory
            .iter()
            .filter(|entry| entry.live && entry.parent_realm.is_some())
        {
            let token = frame.frame_token.as_deref().ok_or_else(|| CommandError {
                code: "unknown error",
                message: "native frame inventory omitted connection identity".into(),
            })?;
            let handle = self.frame_reference_handle(page_id, token)?;
            handles.push(WindowHandleBinding {
                realm: frame.realm,
                document_generation: frame.document_generation,
                stream_epoch: frame.stream_epoch,
                handle,
                top_level: false,
            });
        }
        Ok(ScriptContext {
            selected: self.selected_frame.clone(),
            handles,
            top_window_handle: None,
        })
    }

    pub fn set_timeouts(&mut self, value: &Value) -> Result<(), CommandError> {
        self.context()?;
        let next = self.timeouts.updated(value)?;
        self.timeouts = next;
        self.capabilities.insert("timeouts".into(), next.to_json());
        Ok(())
    }

    pub async fn delete(&mut self) -> Result<(), CommandError> {
        // Revoke command access before teardown starts, including on failure.
        let Some(context) = self.context.take() else {
            return Ok(());
        };
        self.current = None;
        self.handles.clear();
        self.selected_frame = None;
        self.frame_references.clear();
        close_context(context).await
    }
}

impl Session {
    pub async fn crash_qualification_worker(&self) -> Result<serde_json::Value, CommandError> {
        let page = self.current_page()?;
        #[cfg(unix)]
        {
            self.context()?
                .terminate_qualification_worker(page.page_id())
                .await
                .map_err(runtime_error)
        }
        #[cfg(not(unix))]
        {
            let _ = page;
            Err(CommandError {
                code: "unsupported operation",
                message: "native worker crash qualification requires Unix process ownership".into(),
            })
        }
    }
    pub fn queued_navigation_evidence(&self) -> Result<serde_json::Value, CommandError> {
        let context = self.context()?;
        let mut pages = Vec::new();
        // Include closed pages: their retained terminal history is still part
        // of this session, even after their public window handle was removed.
        for page in context.pages() {
            let id = page.page.page_id;
            let snapshot = context
                .queued_navigation_status(id)
                .map_err(runtime_error)?;
            pages.push(serde_json::json!({"page_id":id,"window":self.handles.iter().find(|(_,value)|**value==id).map(|(handle,_)|handle),"snapshot":snapshot}));
        }
        Ok(serde_json::json!({"schema_version":1,"pages":pages}))
    }
    pub fn execution_placement(&self, nonce: &str) -> Result<serde_json::Value, CommandError> {
        self.context()?
            .execution_placement_snapshot(nonce)
            .map_err(|error| CommandError {
                code: "unknown error",
                message: error.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn upload_and_closed_lifetime_errors_keep_webdriver_status() {
        assert_eq!(
            super::runtime_error(oryn_native::RuntimeError::Upload(
                "not a regular file".into()
            ))
            .code,
            "invalid argument"
        );
        assert_eq!(
            super::runtime_error(oryn_native::RuntimeError::Closed).code,
            "no such window"
        );
        assert_eq!(
            super::runtime_error(oryn_native::RuntimeError::PageTaskYield).code,
            "no such window"
        );
    }

    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn native_window_results_resolve_only_live_session_registry_entries() {
        let browser = Browser::new();
        let mut first = Session::create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        let mut other = Session::create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        let own = first.current_page().unwrap().page_id();
        let foreign = other.current_page().unwrap().page_id();
        let handle = first.current_window().unwrap();
        let foreign_handle = other.current_window().unwrap();
        assert_ne!(handle, foreign_handle);
        let result = first
            .finalize_script_result(ScriptValue::Array(vec![ScriptValue::WindowPage(own)]))
            .unwrap();
        let expected = ScriptValue::Array(vec![ScriptValue::Object(vec![(
            WINDOW_KEY.encode_utf16().collect(),
            ScriptValue::String(handle.encode_utf16().collect()),
        )])]);
        assert_eq!(result, expected);
        assert_eq!(
            first.authorize_script_arguments(&result).unwrap(),
            ScriptValue::Array(vec![ScriptValue::WindowPage(own)])
        );
        assert_eq!(
            first
                .finalize_script_result(ScriptValue::WindowPage(foreign))
                .unwrap_err()
                .code,
            "no such window"
        );
        let foreign_argument = ScriptValue::Object(vec![(
            WINDOW_KEY.encode_utf16().collect(),
            ScriptValue::String(foreign_handle.encode_utf16().collect()),
        )]);
        assert_eq!(
            first
                .authorize_script_arguments(&foreign_argument)
                .unwrap_err()
                .code,
            "no such window"
        );
        assert_eq!(
            first
                .authorize_script_arguments(&ScriptValue::WindowPage(own))
                .unwrap_err()
                .code,
            "invalid argument"
        );
        assert_eq!(
            first
                .authorize_script_arguments(&ScriptValue::WindowRealm {
                    page: own,
                    frame: "0".repeat(64)
                })
                .unwrap_err()
                .code,
            "invalid argument"
        );
        assert_eq!(
            first
                .finalize_script_result(ScriptValue::WindowRealm {
                    page: foreign,
                    frame: "0".repeat(64)
                })
                .unwrap_err()
                .code,
            "no such window"
        );
        let forged_frame = ScriptValue::Object(vec![(
            FRAME_KEY.encode_utf16().collect(),
            ScriptValue::String("frame-forged".encode_utf16().collect()),
        )]);
        assert_eq!(
            first
                .authorize_script_arguments(&forged_frame)
                .unwrap_err()
                .code,
            "no such window"
        );
        let child_handle = first.new_window().await.unwrap();
        first.switch_window(&child_handle).unwrap();
        let child = first.current_page().unwrap().page_id();
        first.close_window().await.unwrap();
        assert_eq!(
            first
                .finalize_script_result(ScriptValue::WindowPage(child))
                .unwrap_err()
                .code,
            "no such window"
        );
        first.delete().await.unwrap();
        other.delete().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execution_context_binds_windows_and_rejects_retired_selected_frame() {
        use oryn_native::host::webdriver_script::ScriptRequest;
        let browser = Browser::new();
        let mut session = Session::create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        let page = session.current_page().unwrap();
        page.load_html(
            "https://example.test/",
            "<iframe srcdoc='<p>child</p>'></iframe>",
        )
        .await
        .unwrap();
        let request = |body: &str| ScriptRequest {
            body: body.encode_utf16().collect(),
            args: Vec::new(),
            asynchronous: false,
            timeout_ms: Some(1000),
        };
        let context = session.execution_context().await.unwrap();
        let value = page
            .execute_webdriver_script_in_context(
                request("return [window,document.querySelector('iframe').contentWindow]"),
                context,
            )
            .await
            .unwrap();
        let result = session.finalize_script_result(value.clone()).unwrap();
        assert_eq!(session.authorize_script_arguments(&result).unwrap(), value);
        let FrameResponse::Inventory(inventory) = page
            .webdriver_frames(FrameCommand::Inventory)
            .await
            .unwrap()
        else {
            panic!("inventory");
        };
        session.selected_frame = Some(
            inventory
                .into_iter()
                .find(|entry| entry.live && entry.parent_realm.is_some())
                .unwrap(),
        );
        let context = session.execution_context().await.unwrap();
        let child = page
            .execute_webdriver_script_in_context(request("return window"), context)
            .await
            .unwrap();
        let ScriptValue::Array(windows) = value else {
            panic!("window results")
        };
        assert_eq!(child, windows[1]);
        page.evaluate("document.querySelector('iframe').remove();'removed'")
            .await
            .unwrap();
        // Context assembly does not perform a frame RPC. The native operation
        // rejects the selected retired document when it actually starts.
        let context = session.execution_context().await.unwrap();
        let error = page
            .execute_webdriver_script_in_context(request("return 42"), context)
            .await
            .unwrap_err();
        assert_eq!(error.code, "no such frame");
        assert_eq!(
            session
                .authorize_script_arguments(&result)
                .unwrap_err()
                .code,
            "no such window"
        );
        session.delete().await.unwrap();
    }

    #[tokio::test]
    async fn frame_result_and_local_inventory_share_connection_handle() {
        let browser = Browser::new();
        let mut session = Session::create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        let page = session.current_page().unwrap();
        page.load_html(
            "https://example.test/",
            "<iframe srcdoc='<p>child</p>'></iframe>",
        )
        .await
        .unwrap();
        let bindings = session.script_context().await.unwrap();
        let local = bindings
            .handles
            .iter()
            .find(|binding| !binding.top_level)
            .unwrap()
            .handle
            .clone();
        let FrameResponse::Inventory(inventory) = page
            .webdriver_frames(FrameCommand::Inventory)
            .await
            .unwrap()
        else {
            panic!("frame inventory")
        };
        let frame = inventory
            .iter()
            .find(|entry| entry.live && entry.parent_realm.is_some())
            .unwrap()
            .frame_token
            .clone()
            .unwrap();
        let native = ScriptValue::WindowRealm {
            page: page.page_id(),
            frame,
        };
        let result = session.finalize_script_result(native.clone()).unwrap();
        assert_eq!(result.get(FRAME_KEY).unwrap().as_string().unwrap(), local);
        assert_eq!(session.authorize_script_arguments(&result).unwrap(), native);
        page.evaluate("document.querySelector('iframe').remove(); 'removed'")
            .await
            .unwrap();
        // Native inventory checkpoints retire detached frame connections.
        session.script_context().await.unwrap();
        assert_eq!(
            session
                .authorize_script_arguments(&result)
                .unwrap_err()
                .code,
            "no such window"
        );
        session.delete().await.unwrap();
    }

    #[tokio::test]
    async fn windows_share_session_ownership_without_implicit_selection() {
        let browser = Browser::new();
        let mut session = Session::create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        let first = session.current_window().unwrap();
        let second = session.new_window().await.unwrap();
        assert_ne!(first, second);
        assert_eq!(session.current_window().unwrap(), first);
        session.switch_window(&second).unwrap();
        assert_eq!(session.current_window().unwrap(), second);
        assert_eq!(session.close_window().await.unwrap(), vec![first.clone()]);
        assert_eq!(session.current_page().err().unwrap().code, "no such window");
        assert_eq!(
            session.switch_window(&second).unwrap_err().code,
            "no such window"
        );
        session.switch_window(&first).unwrap();
        assert!(session.close_window().await.unwrap().is_empty());
        assert_eq!(
            session.window_handles().unwrap_err().code,
            "invalid session id"
        );
        session.delete().await.unwrap();
    }

    #[tokio::test]
    async fn unpublished_window_rollback_restores_previous_without_losing_failure() {
        let browser = Browser::new();
        let mut session = Session::create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        let previous = session.current_page().unwrap().page_id();
        let original = session.current_window().unwrap();
        let context = session.context().unwrap().clone();
        let orphan = context.new_page();
        orphan.goto("about:blank").await.unwrap();
        let failure = session
            .rollback_new_window(
                orphan.page_id(),
                previous,
                CommandError::invalid("creation failure"),
            )
            .await;
        assert_eq!(failure.message, "creation failure");
        assert_eq!(session.current_window().unwrap(), original);
        assert_eq!(session.window_handles().unwrap(), vec![original]);
        assert!(
            context
                .pages()
                .iter()
                .any(|entry| entry.page.page_id == orphan.page_id()
                    && entry.status == PageStatus::Closed)
        );
        session.delete().await.unwrap();
    }

    #[tokio::test]
    async fn implicit_find_waits_for_native_timers_and_preserves_document_identity() {
        let browser = Browser::new();
        let mut session = Session::create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        let page = session.current_page().unwrap();
        page.load_html("https://lookup.test/", "<body></body>")
            .await
            .unwrap();
        session.timeouts.implicit = Some(500);
        page.evaluate("setTimeout(()=>{const e=document.createElement('div');e.id='late';document.body.append(e)},30)").await.unwrap();
        let result = session
            .find_elements(
                None,
                vec!["#late".into()],
                std::sync::Arc::new(super::super::registry::SessionCancellation::default()),
            )
            .await
            .unwrap();
        assert!(matches!(result,ScriptValue::Array(values) if values.len()==1));
        assert_eq!(
            session
                .find_elements(
                    None,
                    vec!["[".into()],
                    std::sync::Arc::new(super::super::registry::SessionCancellation::default())
                )
                .await
                .unwrap_err()
                .code,
            "invalid selector"
        );
        // Trigger replacement only after lookup has captured the old document
        // and observed an empty result. A wall-clock timer can fire before the
        // lookup is admitted, which legitimately selects the new document.
        let (observed, waiting) = tokio::sync::oneshot::channel();
        session.lookup_wait_observer = Some(observed);
        let lookup = session.find_elements(
            None,
            vec!["#replacement".into()],
            std::sync::Arc::new(super::super::registry::SessionCancellation::default()),
        );
        let replacement = async {
            waiting.await.unwrap();
            page.evaluate("document.open();document.write('<body><div id=replacement></div>');document.close()")
                .await.unwrap();
        };
        let (result, ()) = tokio::join!(lookup, replacement);
        assert_eq!(result.unwrap_err().code, "no such frame");
        session.delete().await.unwrap();
    }
    #[tokio::test]
    async fn implicit_find_unbounded_timeout_still_observes_session_revocation() {
        let browser = Browser::new();
        let mut session = Session::create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        session.timeouts.implicit = None;
        let cancellation =
            std::sync::Arc::new(super::super::registry::SessionCancellation::default());
        cancellation.cancel();
        assert_eq!(
            session
                .find_elements(None, vec!["#missing".into()], cancellation)
                .await
                .unwrap_err()
                .code,
            "invalid session id"
        );
        session.delete().await.unwrap();
    }

    #[tokio::test]
    async fn rollback_reports_previous_window_loss_and_clears_current() {
        let browser = Browser::new();
        let mut session = Session::create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        let previous = session.current_page().unwrap().page_id();
        let context = session.context().unwrap().clone();
        let orphan = context.new_page();
        orphan.goto("about:blank").await.unwrap();
        context.close_page(previous).await.unwrap();
        let failure = session
            .rollback_new_window(
                orphan.page_id(),
                previous,
                CommandError::invalid("creation failure"),
            )
            .await;
        assert!(failure.message.contains("creation failure"));
        assert!(failure.message.contains("previous window restore failed"));
        assert!(session.current.is_none());
        session.delete().await.unwrap();
    }

    #[tokio::test]
    async fn timeout_updates_and_deletion_preserve_native_session_boundaries() {
        let browser = Browser::new();
        let mut a = Session::create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        let mut b = Session::create(&browser, ContextOptions::loopback_test(), Map::new())
            .await
            .unwrap();
        a.set_timeouts(&json!({"script":null,"implicit":5}))
            .unwrap();
        let before = a.timeouts;
        assert!(
            a.set_timeouts(&json!({"implicit":10,"pageLoad":false}))
                .is_err()
        );
        assert_eq!(a.timeouts, before);
        assert_eq!(b.timeouts, Timeouts::default());
        let retained = a.current_page().unwrap();
        a.delete().await.unwrap();
        assert!(retained.evaluate("1").await.is_err());
        assert_eq!(a.current_page().err().unwrap().code, "invalid session id");
        assert!(b.current_page().is_ok());
        b.delete().await.unwrap();
    }
}
