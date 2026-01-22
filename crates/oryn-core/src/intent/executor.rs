use crate::backend::{Backend, BackendError};
use crate::command::{Command, Target};
use crate::intent::definition::{
    ActionStep, ActionType, Condition, FlowDefinition, IntentDefinition, PageAction, PageDef, Step,
    TargetKind, TargetSpec,
};
use crate::intent::registry::IntentRegistry;
use crate::intent::verifier::{Verifier, VerifierContext};
use crate::protocol::{
    ChangeType, PageChanges, ScanRequest, ScanResult, ScannerData, ScannerProtocolResponse,
    ScannerRequest,
};
use crate::resolver::{ResolutionStrategy, ResolverContext, resolve_target};
use crate::translator;
use async_recursion::async_recursion;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    #[error("Intent not found: {0}")]
    IntentNotFound(String),
    #[error("Missing parameter: {0}")]
    MissingParameter(String),
    #[error("Invalid parameter type for {0}")]
    InvalidParameterType(String),
    #[error("Backend error: {0}")]
    Backend(#[from] BackendError),
    #[error("Translation error: {0}")]
    Translation(#[from] translator::TranslationError),
    #[error("Resolution error: {0}")]
    Resolution(#[from] crate::resolver::ResolverError),
    #[error("Verification error: {0}")]
    Verification(#[from] crate::intent::verifier::VerificationError),
    #[error("Step execution failed: {0}")]
    StepFailed(String),
    #[error("Intent failed: {0}")]
    IntentFailed(String),
    #[error("Flow page not found: {0}")]
    FlowPageNotFound(String),
    #[error("Flow URL pattern timeout: expected pattern '{0}' but current URL is '{1}'")]
    FlowUrlPatternTimeout(String, String),
    #[error("Invalid URL pattern regex: {0}")]
    InvalidUrlPattern(String),
}

pub struct IntentResult {
    pub status: IntentStatus,
    pub data: Option<Value>,
    pub logs: Vec<String>,
    pub checkpoint: Option<String>,
    pub hints: Vec<String>,
    pub changes: Option<PageChanges>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IntentStatus {
    Success,
    PartialSuccess { completed: usize, total: usize },
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointState {
    pub name: String,
    pub variables: HashMap<String, Value>,
    pub step_index: usize,
}

pub struct IntentExecutor<'a, B: Backend + ?Sized> {
    backend: &'a mut B,
    registry: &'a IntentRegistry,
    verifier: &'a Verifier,
    variables: HashMap<String, Value>,
    logs: Vec<String>,
    last_scan: Option<ScanResult>,
    initial_scan: Option<ScanResult>,
    checkpoints: Vec<CheckpointState>,
    last_checkpoint: Option<String>,
}

impl<'a, B: Backend + ?Sized> IntentExecutor<'a, B> {
    pub fn new(backend: &'a mut B, registry: &'a IntentRegistry, verifier: &'a Verifier) -> Self {
        Self {
            backend,
            registry,
            verifier,
            variables: HashMap::new(),
            logs: Vec::new(),
            last_scan: None,
            initial_scan: None,
            checkpoints: Vec::new(),
            last_checkpoint: None,
        }
    }

    pub async fn execute(
        &mut self,
        intent_name: &str,
        params: HashMap<String, Value>,
    ) -> Result<IntentResult, ExecutorError> {
        self.logs.push(format!("Executing intent: {}", intent_name));

        // 1. RESOLVE
        let intent = self
            .registry
            .get(intent_name)
            .ok_or_else(|| ExecutorError::IntentNotFound(intent_name.to_string()))?
            .clone(); // Clone to avoid borrow issues

        // 2. PARSE & BIND PARAMETERS
        self.bind_parameters(&intent.parameters, &params)?;

        // 3. PLAN (Scan initial state)
        if !self.backend.is_ready().await {
            self.backend.launch().await?;
        }
        self.perform_scan().await?;
        self.initial_scan = self.last_scan.clone();

        // 4. EXECUTE - check for flow vs steps
        if let Some(flow) = &intent.flow {
            return self.execute_flow(&intent, flow.clone()).await;
        }

        let mut steps_completed = 0;
        let total_steps = intent.steps.len();

        for step in &intent.steps {
            match self
                .execute_step_with_retry(step, &intent.options.retry)
                .await
            {
                Ok(_) => steps_completed += 1,
                Err(e) => {
                    // Return PartialSuccess if some steps completed
                    if steps_completed > 0 {
                        return Ok(IntentResult {
                            status: IntentStatus::PartialSuccess {
                                completed: steps_completed,
                                total: total_steps,
                            },
                            data: None,
                            logs: self.logs.clone(),
                            checkpoint: self.last_checkpoint.clone(),
                            hints: vec![format!("Failed at step {}: {}", steps_completed + 1, e)],
                            changes: self.calculate_changes(),
                        });
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        // 5. VERIFY
        if let Some(success_cond) = &intent.success {
            // Need fresh scan for final verification
            self.perform_scan().await?;
            if let Some(scan) = &self.last_scan {
                let ctx = VerifierContext::with_variables(scan, &self.variables);
                let passed = self
                    .verifier
                    .verify(&Condition::All(success_cond.conditions.clone()), &ctx)
                    .await?;
                if !passed {
                    // Verification failed after steps executed
                    return Ok(IntentResult {
                        status: IntentStatus::PartialSuccess {
                            completed: steps_completed,
                            total: total_steps,
                        },
                        data: None,
                        logs: self.logs.clone(),
                        checkpoint: self.last_checkpoint.clone(),
                        hints: vec!["Steps completed but verification failed".to_string()],
                        changes: self.calculate_changes(),
                    });
                }
            }
        } else {
            // If no verification needed, maybe refresh scan for final state diff?
            self.perform_scan().await?;
        }

        // 6. RESPOND
        Ok(IntentResult {
            status: IntentStatus::Success,
            data: self
                .extract_result_data(intent.success.as_ref().and_then(|s| s.extract.as_ref())),
            logs: self.logs.clone(),
            checkpoint: self.last_checkpoint.clone(),
            hints: vec![],
            changes: self.calculate_changes(),
        })
    }

    pub async fn execute_with_resume(
        &mut self,
        intent_name: &str,
        params: HashMap<String, Value>,
        resume_from: Option<&str>,
    ) -> Result<IntentResult, ExecutorError> {
        self.logs.push(format!(
            "Executing intent (resume={:?}): {}",
            resume_from, intent_name
        ));

        // 1. RESOLVE
        let intent = self
            .registry
            .get(intent_name)
            .ok_or_else(|| ExecutorError::IntentNotFound(intent_name.to_string()))?
            .clone();

        // 2. PARSE & BIND PARAMETERS
        self.bind_parameters(&intent.parameters, &params)?;

        // 3. PLAN (Scan initial state)
        if !self.backend.is_ready().await {
            self.backend.launch().await?;
        }
        self.perform_scan().await?;
        self.initial_scan = self.last_scan.clone();

        // 4. EXECUTE (with resume logic)
        let start_index = if let Some(checkpoint_name) = resume_from {
            // ... (keep logic) ...
            let mut idx = 0;
            let mut found = false;
            for (i, step) in intent.steps.iter().enumerate() {
                if let Step::Checkpoint(wrapper) = step
                    && wrapper.checkpoint == checkpoint_name
                {
                    idx = i + 1; // Start AFTER the checkpoint
                    found = true;
                    break;
                }
            }

            if !found {
                return Err(ExecutorError::StepFailed(format!(
                    "Checkpoint '{}' not found in intent",
                    checkpoint_name
                )));
            }
            self.logs.push(format!("Resuming from step index {}", idx));
            idx
        } else {
            0
        };

        // Execute steps starting from start_index
        for (_, step) in intent.steps.iter().enumerate().skip(start_index) {
            // Apply retry logic if configured
            // For now, use intent-level retry config for all steps
            // Or ideally, step-level retry. Definition needs update for Step-level retry?
            // The plan mentioned "Enhanced Retry Logic" using IntentOptions.

            self.execute_step_with_retry(step, &intent.options.retry)
                .await?;
        }

        // 6. RESPOND (Verify omitted for brevity in resume flow? Or strictly verify?)
        // Let's verify success condition
        if let Some(success_cond) = &intent.success {
            self.perform_scan().await?;
            if let Some(scan) = &self.last_scan {
                let ctx = VerifierContext::with_variables(scan, &self.variables);
                let passed = self
                    .verifier
                    .verify(&Condition::All(success_cond.conditions.clone()), &ctx)
                    .await?;
                if !passed {
                    return Err(ExecutorError::IntentFailed(
                        "Success conditions not met".into(),
                    ));
                }
            }
        } else {
            self.perform_scan().await?;
        }

        Ok(IntentResult {
            status: IntentStatus::Success,
            data: self
                .extract_result_data(intent.success.as_ref().and_then(|s| s.extract.as_ref())),
            logs: self.logs.clone(),
            checkpoint: self.last_checkpoint.clone(),
            hints: vec![],
            changes: self.calculate_changes(),
        })
    }

    #[async_recursion]
    async fn execute_step_with_retry(
        &mut self,
        step: &Step,
        config: &crate::intent::definition::RetryConfig,
    ) -> Result<(), ExecutorError> {
        let mut attempts = 0;
        let max_attempts = config.max_attempts.max(1);

        loop {
            attempts += 1;
            match self.execute_step(step).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    if attempts >= max_attempts {
                        // Check for per-step error handlers
                        if let Step::Action(crate::intent::definition::ActionStep {
                            on_error: Some(error_steps),
                            ..
                        }) = step
                        {
                            self.logs.push(format!(
                                "Step failed after {} attempts. Executing on_error handler. Error: {}",
                                attempts, e
                            ));

                            for handler_step in error_steps {
                                self.execute_step_with_retry(handler_step, config).await?;
                            }

                            self.logs.push(
                                "on_error handler completed successfully. Recovered.".to_string(),
                            );
                            return Ok(());
                        }
                        return Err(e);
                    }
                    // Simple check: is it retryable? Most executor errors are transient (selector not found, etc)
                    // unless it's strictly logic error.
                    // For now, retry everything except IntentNotFound/Param errors?
                    // Let's assume most step failures are retryable.

                    let delay = (config.delay_ms as f64
                        * config.backoff_multiplier.powi((attempts - 1) as i32))
                        as u64;
                    self.logs.push(format!(
                        "Step failed (attempt {}/{}). Retrying in {}ms. Error: {}",
                        attempts, max_attempts, delay, e
                    ));
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;

                    // Refresh DOM before retry
                    let _ = self.perform_scan().await;
                }
            }
        }
    }

    fn extract_result_data(&self, rule: Option<&Value>) -> Option<Value> {
        let rule = rule?;
        Some(self.resolve_extraction_value(rule))
    }

    fn resolve_extraction_value(&self, val: &Value) -> Value {
        match val {
            Value::String(s) => {
                if let Some(v) = self.resolve_variable_value(s) {
                    v
                } else {
                    val.clone()
                }
            }
            Value::Object(map) => {
                let mut new_map = serde_json::Map::new();
                for (k, v) in map {
                    new_map.insert(k.clone(), self.resolve_extraction_value(v));
                }
                Value::Object(new_map)
            }
            Value::Array(arr) => Value::Array(
                arr.iter()
                    .map(|v| self.resolve_extraction_value(v))
                    .collect(),
            ),
            _ => val.clone(),
        }
    }

    pub async fn perform_scan(&mut self) -> Result<(), ExecutorError> {
        let scan_req = ScannerRequest::Scan(ScanRequest::default());
        let resp = self.backend.execute_scanner(scan_req).await?;

        if let ScannerProtocolResponse::Ok { data, .. } = resp
            && let ScannerData::Scan(mut result) = *data
        {
            // Calculate available intents
            result.available_intents = Some(self.calculate_available_intents(&result));
            self.last_scan = Some(*result);
        }
        Ok(())
    }

    fn calculate_available_intents(
        &self,
        scan: &ScanResult,
    ) -> Vec<crate::protocol::IntentAvailability> {
        let mut availability = Vec::new();

        for intent in self.registry.list() {
            let mut status = crate::protocol::AvailabilityStatus::Ready;
            let mut reason = None;

            // 1. Check URL RegEx (Any match is sufficient)
            if !intent.triggers.urls.is_empty() {
                let mut url_matched = false;
                for url_pattern in &intent.triggers.urls {
                    match regex::Regex::new(url_pattern) {
                        Ok(re) => {
                            if re.is_match(&scan.page.url) {
                                url_matched = true;
                                break;
                            }
                        }
                        Err(_) => {
                            reason = Some(format!("Invalid URL regex: {}", url_pattern));
                            // Continue checking others? Or fail hard?
                            // Let's mark as unavailable if any regex is invalid to be safe/noisy?
                            // Or just ignore invalid ones?
                            // Let's record reason and fail this specific pattern match.
                        }
                    }
                }

                if !url_matched && reason.is_none() {
                    status = crate::protocol::AvailabilityStatus::NavigateRequired;
                } else if !url_matched {
                    status = crate::protocol::AvailabilityStatus::Unavailable;
                }
            }

            // 2. Check Patterns
            // Only check patterns if URL is fine (Ready so far)
            if status == crate::protocol::AvailabilityStatus::Ready {
                if let Some(patterns) = &scan.patterns {
                    for required_pattern in &intent.triggers.patterns {
                        let found = match required_pattern.as_str() {
                            "login_form" => patterns.login.is_some(),
                            "search_box" => patterns.search.is_some(),
                            "pagination" => patterns.pagination.is_some(),
                            "modal" => patterns.modal.is_some(),
                            "cookie_banner" => patterns.cookie_banner.is_some(),
                            _ => false, // Unknown pattern
                        };

                        if !found {
                            status = crate::protocol::AvailabilityStatus::MissingPattern;
                            reason = Some(format!("Missing pattern: {}", required_pattern));
                            break;
                        }
                    }
                } else if !intent.triggers.patterns.is_empty() {
                    // No patterns detected but intent requires some
                    status = crate::protocol::AvailabilityStatus::MissingPattern;
                    reason = Some("No patterns detected on page".to_string());
                }
            }

            availability.push(crate::protocol::IntentAvailability {
                name: intent.name.clone(),
                status,
                parameters: intent.parameters.iter().map(|p| p.name.clone()).collect(),
                trigger_reason: reason,
            });
        }
        availability
    }

    fn bind_parameters(
        &mut self,
        defs: &[crate::intent::definition::ParameterDef],
        params: &HashMap<String, Value>,
    ) -> Result<(), ExecutorError> {
        for def in defs {
            if let Some(value) = params.get(&def.name).or(def.default.as_ref()) {
                self.variables.insert(def.name.clone(), value.clone());
            } else if def.required {
                return Err(ExecutorError::MissingParameter(def.name.clone()));
            }
        }
        Ok(())
    }

    #[async_recursion]
    async fn execute_step(&mut self, step: &Step) -> Result<(), ExecutorError> {
        match step {
            Step::Action(action) => self.execute_action(action).await,
            Step::Branch(wrapper) => {
                let condition_met = self.evaluate_condition(&wrapper.branch.condition).await?;
                if condition_met {
                    for s in &wrapper.branch.then_steps {
                        self.execute_step(s).await?;
                    }
                } else {
                    for s in &wrapper.branch.else_steps {
                        self.execute_step(s).await?;
                    }
                }
                Ok(())
            }
            Step::Loop(wrapper) => {
                // Determine what to loop over
                let items = if let Some(val) = self.variables.get(&wrapper.loop_.over) {
                    // Loop over array variable
                    val.as_array().cloned().unwrap_or_default()
                } else if let Ok(n) = wrapper.loop_.over.parse::<usize>() {
                    // Treat as count 0..N
                    (0..n).map(|i| json!(i)).collect()
                } else {
                    // Fallback: see if it looks like "start..end"
                    if let Some((start, end)) = wrapper.loop_.over.split_once("..") {
                        if let (Ok(s), Ok(e)) = (start.parse::<usize>(), end.parse::<usize>()) {
                            (s..e).map(|i| json!(i)).collect()
                        } else {
                            // Fallback: run 'max' times
                            (0..wrapper.loop_.max).map(|i| json!(i)).collect()
                        }
                    } else {
                        // Fallback: run 'max' times
                        (0..wrapper.loop_.max).map(|i| json!(i)).collect()
                    }
                };

                let limit = wrapper.loop_.max;
                for item in items.iter().take(limit) {
                    self.variables
                        .insert(wrapper.loop_.as_var.clone(), item.clone());
                    for s in &wrapper.loop_.steps {
                        self.execute_step(s).await?;
                    }
                }
                Ok(())
            }
            Step::Try(wrapper) => {
                let mut success = true;
                for s in &wrapper.try_.steps {
                    if let Err(e) = self.execute_step(s).await {
                        self.logs.push(format!("Try block step failed: {}", e));
                        success = false;
                        break;
                    }
                }

                if !success {
                    for s in &wrapper.try_.catch {
                        self.execute_step(s).await?;
                    }
                }
                Ok(())
            }
            Step::Checkpoint(wrapper) => {
                self.last_checkpoint = Some(wrapper.checkpoint.clone());
                // In a real system, we might persist state to disk here.
                // For now, we just track it in memory.
                self.checkpoints.push(CheckpointState {
                    name: wrapper.checkpoint.clone(),
                    variables: self.variables.clone(),
                    step_index: 0, // We don't have index here easily without passing it.
                                   // But for "resume_from" logic above, we rely on name matching.
                });
                self.logs
                    .push(format!("Checkpoint reached: {}", wrapper.checkpoint));
                Ok(())
            }
        }
    }

    async fn execute_action(&mut self, step: &ActionStep) -> Result<(), ExecutorError> {
        self.logs.push(format!("Action: {:?}", step.action));

        let target = match &step.target {
            Some(spec) => Some(self.resolve_target_spec(spec).await?),
            None => None,
        };

        match step.action {
            ActionType::Click => {
                if let Some(t) = target {
                    let cmd = Command::Click(t, self.convert_options(&step.options));
                    let req = translator::translate(&cmd)?;
                    self.backend.execute_scanner(req).await?;
                }
            }
            ActionType::Type => {
                if let Some(t) = target {
                    let text = self.resolve_variable(step.options.get("text"));
                    let cmd = Command::Type(t, text, self.convert_options(&step.options));
                    let req = translator::translate(&cmd)?;
                    self.backend.execute_scanner(req).await?;
                }
            }
            ActionType::Wait => {
                let cond_str = step
                    .options
                    .get("condition")
                    .and_then(|v| v.as_str())
                    .unwrap_or("visible");

                let wait_cond = match cond_str {
                    "visible" => {
                        if let Some(t) = target {
                            crate::command::WaitCondition::Visible(t)
                        } else {
                            return Err(ExecutorError::MissingParameter(
                                "target for wait visible".into(),
                            ));
                        }
                    }
                    "hidden" => {
                        if let Some(t) = target {
                            crate::command::WaitCondition::Hidden(t)
                        } else {
                            return Err(ExecutorError::MissingParameter(
                                "target for wait hidden".into(),
                            ));
                        }
                    }
                    "load" => crate::command::WaitCondition::Load,
                    "idle" => crate::command::WaitCondition::Idle,
                    "url" => {
                        if let Some(p) = step.options.get("pattern").and_then(|v| v.as_str()) {
                            crate::command::WaitCondition::Url(p.to_string())
                        } else {
                            return Err(ExecutorError::MissingParameter(
                                "pattern for wait url".into(),
                            ));
                        }
                    }
                    _ => {
                        return Err(ExecutorError::InvalidParameterType(format!(
                            "Unknown wait condition: {}",
                            cond_str
                        )));
                    }
                };

                let cmd = Command::Wait(wait_cond, self.convert_options(&step.options));
                let req = translator::translate(&cmd)?;
                self.backend.execute_scanner(req).await?;
            }
            ActionType::FillForm => {
                // Resolve the data parameter, which can be a variable reference or inline object
                let data_json = if let Some(v) = step.options.get("data") {
                    if let Some(s) = v.as_str() {
                        if let Some(res) = self.resolve_variable_value(s) {
                            res
                        } else {
                            v.clone()
                        }
                    } else {
                        v.clone()
                    }
                } else {
                    json!({})
                };

                if let Some(obj) = data_json.as_object() {
                    for (key, val) in obj {
                        let val_str = val.as_str().unwrap_or_default().to_string();
                        let mut found_via_scan = false;

                        if let Some(scan) = &self.last_scan {
                            // Use scoring-based matching to find the best form field
                            if let Some(el) = find_best_form_field(&scan.elements, key) {
                                let t = Target::Id(el.id as usize);
                                let cmd = Command::Type(t, val_str.clone(), HashMap::new());
                                if let Ok(req) = translator::translate(&cmd)
                                    && self.backend.execute_scanner(req).await.is_ok()
                                {
                                    found_via_scan = true;
                                }
                            }
                        }

                        if !found_via_scan {
                            // Fallback to selector strategy for hidden/unscanned fields
                            let selector = format!(
                                "input[name='{}'], input[id='{}'], textarea[name='{}']",
                                key, key, key
                            );
                            let spec = TargetSpec {
                                kind: TargetKind::Selector { selector },
                                fallback: None,
                            };
                            if let Ok(t) = self.resolve_target_spec(&spec).await {
                                let cmd = Command::Type(t, val_str, HashMap::new());
                                let req = translator::translate(&cmd)?;
                                self.backend.execute_scanner(req).await?;
                            } else {
                                self.logs
                                    .push(format!("Could not find field for key: {}", key));
                            }
                        }
                    }
                }
            }
            ActionType::Select => {
                if let Some(t) = target {
                    // Option value/text/index
                    let value = self.resolve_variable(step.options.get("value"));
                    // If no value, maybe index?
                    // For now, Command::Select takes Target and String value.
                    // If empty, it might mean "select the target itself" if it's an option?
                    // But usually Select(Target, Value).
                    let cmd = Command::Select(t, value);
                    let req = translator::translate(&cmd)?;
                    self.backend.execute_scanner(req).await?;
                }
            }
            ActionType::Check => {
                if let Some(t) = target {
                    let cmd = Command::Check(t);
                    let req = translator::translate(&cmd)?;
                    self.backend.execute_scanner(req).await?;
                }
            }
            ActionType::Uncheck => {
                if let Some(t) = target {
                    let cmd = Command::Uncheck(t);
                    let req = translator::translate(&cmd)?;
                    self.backend.execute_scanner(req).await?;
                }
            }
            ActionType::Clear => {
                if let Some(t) = target {
                    let cmd = Command::Clear(t);
                    let req = translator::translate(&cmd)?;
                    self.backend.execute_scanner(req).await?;
                }
            }
            ActionType::Scroll => {
                // Scroll to target OR scroll based on options (up/down/etc)
                let options = self.convert_options(&step.options);
                let cmd = Command::Scroll(target, options);
                let req = translator::translate(&cmd)?;
                self.backend.execute_scanner(req).await?;
            }
            ActionType::Execute => {
                // Run raw script
                if let Some(script) = step.options.get("script").and_then(|v| v.as_str()) {
                    let _ = self.backend.execute_script(script).await?;
                }
            }
            ActionType::Intent => {
                // Intent Composition
                if let Some(sub_intent_name) = step.options.get("name").and_then(|v| v.as_str()) {
                    self.logs
                        .push(format!("Executing Sub-Intent: {}", sub_intent_name));
                    // Pass parameters from options
                    let _params = step.options.clone();

                    // We need to resolve parameters that reference variables?
                    // convert_options resolves string values.
                    // But executor.execute takes HashMap<String, Value>.
                    // We need to map `options` back to `Value` but resolved.
                    let mut resolved_params = HashMap::new();
                    for (k, v) in step.options.iter() {
                        if k == "name" {
                            continue;
                        }
                        if let Some(s) = v.as_str() {
                            // If it's a variable reference, resolve it to Value
                            if let Some(val) = self.resolve_variable_value(s) {
                                resolved_params.insert(k.clone(), val);
                            } else {
                                // Literal string
                                resolved_params.insert(k.clone(), v.clone());
                            }
                        } else {
                            // Non-string value, keep as is
                            resolved_params.insert(k.clone(), v.clone());
                        }
                    }

                    // We need *recur* here.
                    // Since we act on `&mut self`, we can't easily recurse with `self.execute`.
                    // `execute` takes `&mut self`.
                    // BUT `execute` resets state (logs, variables?).
                    // Wait, `execute` uses self.variables.
                    // A sub-intent should have its OWN scope or inherit?
                    // Usually sub-intents have isolated scope passing params.
                    // But `self` holds `backend`, `registry`.
                    // If we reuse `self`, we pollute `self.variables`?

                    // Better: Create a sub-executor.
                    // We need to temporarily borrow backend/registry/verifier.
                    // Problem: `self` borrows them.

                    // Option 1: `execute_sub_intent` method that creates a new executor on the stack
                    // borrowing necessary fields from `self` (careful with mutable borrow of backend).
                    // We *can* borrow `self.backend` mutably if we don't borrow `self` elsewhere.

                    // Let's delegate to a helper that takes explicit args.
                    let sub_result = {
                        // Create temporary scope
                        // We need to construct a new Executor.
                        let mut sub_executor = IntentExecutor::new(
                            self.backend, // We re-borrow backend? `self.backend`
                            self.registry,
                            self.verifier,
                        );
                        // Execute sub-intent
                        sub_executor.execute(sub_intent_name, resolved_params).await
                    };

                    match sub_result {
                        Ok(res) => {
                            self.logs.push(format!(
                                "Sub-Intent '{}' finished: {:?}",
                                sub_intent_name, res.status
                            ));
                            // Append logs?
                            self.logs.extend(res.logs);
                            if let IntentStatus::Failed(e) = res.status {
                                return Err(ExecutorError::IntentFailed(format!(
                                    "Sub-intent failed: {}",
                                    e
                                )));
                            }
                            if let IntentStatus::PartialSuccess { .. } = res.status {
                                // Treat partial success of sub-intent as failure of this step??
                                // Or just bubble?
                                // Simplest: If not full success, fail this step.
                                return Err(ExecutorError::IntentFailed(
                                    "Sub-intent partial success".into(),
                                ));
                            }
                        }
                        Err(e) => {
                            return Err(ExecutorError::StepFailed(format!(
                                "Sub-intent execution error: {}",
                                e
                            )));
                        }
                    }
                } else {
                    return Err(ExecutorError::MissingParameter(
                        "name for intent action".into(),
                    ));
                }
            }
            ActionType::Navigate => {
                let url = step
                    .options
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(|s| self.resolve_variable(Some(&Value::String(s.to_string()))))
                    .ok_or_else(|| ExecutorError::MissingParameter("url for navigate".into()))?;
                self.logs.push(format!("Navigating to: {}", url));
                self.backend.navigate(&url).await?;
            }
            ActionType::GoBack => {
                self.logs.push("Navigating back".to_string());
                self.backend.go_back().await?;
            }
            ActionType::GoForward => {
                self.logs.push("Navigating forward".to_string());
                self.backend.go_forward().await?;
            }
            ActionType::Refresh => {
                self.logs.push("Refreshing page".to_string());
                self.backend.refresh().await?;
            }
        }
        Ok(())
    }

    async fn resolve_target_spec(&mut self, spec: &TargetSpec) -> Result<Target, ExecutorError> {
        // Refresh scan if needed
        self.perform_scan().await?;

        // Use last_scan
        let ctx = if let Some(scan) = &self.last_scan {
            ResolverContext::new(scan)
        } else {
            ResolverContext::empty()
        };

        let target_tree = Self::build_target(spec);

        match resolve_target(&target_tree, &ctx, ResolutionStrategy::Best) {
            Ok(t) => Ok(t),
            Err(e) => {
                if let Some(fallback) = &spec.fallback {
                    self.logs
                        .push(format!("Resolution failed ({}), trying fallback...", e));
                    // Recursive call for fallback
                    Box::pin(self.resolve_target_spec(fallback)).await
                } else {
                    Err(ExecutorError::Resolution(e))
                }
            }
        }
    }

    fn build_target(spec: &TargetSpec) -> Target {
        match &spec.kind {
            TargetKind::Pattern { pattern } => Target::Text(pattern.clone()),
            TargetKind::Role { role } => Target::Role(role.clone()),
            TargetKind::Text { text, .. } => Target::Text(text.clone()),
            TargetKind::Selector { selector } => Target::Selector(selector.clone()),
            TargetKind::Id { id } => Target::Id(*id as usize),
            TargetKind::Near { near, anchor } => Target::Near {
                target: Box::new(Self::build_target(near)),
                anchor: Box::new(Self::build_target(anchor)),
            },
            TargetKind::Inside { inside, container } => Target::Inside {
                target: Box::new(Self::build_target(inside)),
                container: Box::new(Self::build_target(container)),
            },
            TargetKind::After { after, anchor } => Target::After {
                target: Box::new(Self::build_target(after)),
                anchor: Box::new(Self::build_target(anchor)),
            },
            TargetKind::Before { before, anchor } => Target::Before {
                target: Box::new(Self::build_target(before)),
                anchor: Box::new(Self::build_target(anchor)),
            },
            TargetKind::Contains { contains, content } => Target::Contains {
                target: Box::new(Self::build_target(contains)),
                content: Box::new(Self::build_target(content)),
            },
        }
    }

    async fn evaluate_condition(&mut self, cond: &Condition) -> Result<bool, ExecutorError> {
        // Condition might rely on latest state
        self.perform_scan().await?;

        if let Some(scan) = &self.last_scan {
            let ctx = VerifierContext::with_variables(scan, &self.variables);
            self.verifier
                .verify(cond, &ctx)
                .await
                .map_err(ExecutorError::Verification)
        } else {
            // No scan available?
            Ok(false)
        }
    }

    fn resolve_variable(&self, val: Option<&Value>) -> String {
        let Some(Value::String(s)) = val else {
            return String::new();
        };

        if let Some(var_name) = s.strip_prefix('$')
            && let Some(v) = self.variables.get(var_name)
        {
            return v.as_str().unwrap_or_default().to_string();
        }
        s.clone()
    }

    fn resolve_variable_value(&self, s: &str) -> Option<Value> {
        if let Some(var_name) = s.strip_prefix('$') {
            self.variables.get(var_name).cloned()
        } else {
            None
        }
    }

    fn convert_options(&self, options: &HashMap<String, Value>) -> HashMap<String, String> {
        options
            .iter()
            .map(|(k, v)| {
                let value = if v.is_string() {
                    self.resolve_variable(Some(v))
                } else {
                    v.to_string()
                };
                (k.clone(), value)
            })
            .collect()
    }

    fn calculate_changes(&self) -> Option<PageChanges> {
        let Some(initial) = &self.initial_scan else {
            return None;
        };
        let Some(final_scan) = &self.last_scan else {
            return None;
        };

        let mut changes = PageChanges::default();

        // URL Change
        if initial.page.url != final_scan.page.url {
            changes.url = Some(final_scan.page.url.clone());
        }

        // Title Change
        if initial.page.title != final_scan.page.title {
            changes.title = Some(final_scan.page.title.clone());
        }

        // Removed Elements (Simplistic diff by ID or selector? IDs might change on reload)
        // Better: Semantically removed/added?
        // Protocol might provide `changes` in `ScanResult` if `monitor_changes` was on.
        // But here we want a high-level summary.

        // Let's rely on `final_scan.changes` if available (if monitor_changes was enabled).
        // If not, maybe just return what we have.
        // For MVP, just URL/Title is good first step.
        // We can create a naive diff of elements count?

        if let Some(diffs) = &final_scan.changes {
            for diff in diffs {
                match diff.change_type {
                    ChangeType::Appeared => {
                        // Find element to get selector/desc
                        if let Some(el) = final_scan.elements.iter().find(|e| e.id == diff.id) {
                            changes.added.push(el.selector.clone());
                        }
                    }
                    ChangeType::Disappeared => {
                        // We can't find it in final scan.
                        // Maybe find in initial?
                        if let Some(el) = initial.elements.iter().find(|e| e.id == diff.id) {
                            changes.removed.push(el.selector.clone());
                        }
                    }
                    _ => {}
                }
            }
        }

        if changes.url.is_none()
            && changes.title.is_none()
            && changes.added.is_empty()
            && changes.removed.is_empty()
        {
            None
        } else {
            Some(changes)
        }
    }

    // ==========================================================================
    // Multi-Page Flow Execution
    // ==========================================================================

    /// Execute a multi-page flow definition.
    #[async_recursion]
    async fn execute_flow(
        &mut self,
        intent: &IntentDefinition,
        flow: FlowDefinition,
    ) -> Result<IntentResult, ExecutorError> {
        self.logs
            .push("Starting multi-page flow execution".to_string());

        // Determine start page (use explicit start or first page)
        let start_page_name = flow
            .start
            .clone()
            .or_else(|| flow.pages.first().map(|p| p.name.clone()))
            .unwrap_or_default();

        let mut current_page_name = Some(start_page_name);
        let mut extracted_data: HashMap<String, Value> = HashMap::new();
        let mut pages_completed = 0;
        let total_pages = flow.pages.len();

        // Page execution loop
        while let Some(page_name) = current_page_name.take() {
            let page = flow
                .pages
                .iter()
                .find(|p| p.name == page_name)
                .ok_or_else(|| ExecutorError::FlowPageNotFound(page_name.clone()))?
                .clone();

            self.logs
                .push(format!("Flow: executing page '{}'", page.name));

            match self.execute_page(&page, intent).await {
                Ok(page_data) => {
                    pages_completed += 1;

                    // Merge extracted data from page
                    if let Some(obj) = page_data.as_ref().and_then(|d| d.as_object()) {
                        extracted_data.extend(obj.iter().map(|(k, v)| (k.clone(), v.clone())));
                    }

                    // Determine next page from transition
                    current_page_name = page
                        .next
                        .as_ref()
                        .and_then(|t| t.target_page())
                        .map(String::from);
                    if current_page_name.is_none() {
                        self.logs.push("Flow: completed successfully".to_string());
                    }
                }
                Err(e) => {
                    // Check for page-level error handler
                    if let Some(error_page) = &page.on_error {
                        self.logs.push(format!(
                            "Flow: page '{}' failed, transitioning to error handler '{}'",
                            page.name, error_page
                        ));
                        current_page_name = Some(error_page.clone());
                    } else if pages_completed > 0 {
                        return Ok(IntentResult {
                            status: IntentStatus::PartialSuccess {
                                completed: pages_completed,
                                total: total_pages,
                            },
                            data: Some(json!(extracted_data)),
                            logs: self.logs.clone(),
                            checkpoint: self.last_checkpoint.clone(),
                            hints: vec![format!("Flow failed at page '{}': {}", page.name, e)],
                            changes: self.calculate_changes(),
                        });
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        // Verify success conditions if specified
        if let Some(success_cond) = &intent.success {
            self.perform_scan().await?;
            if let Some(scan) = &self.last_scan {
                let ctx = VerifierContext::with_variables(scan, &self.variables);
                let passed = self
                    .verifier
                    .verify(&Condition::All(success_cond.conditions.clone()), &ctx)
                    .await?;
                if !passed {
                    return Ok(IntentResult {
                        status: IntentStatus::PartialSuccess {
                            completed: pages_completed,
                            total: total_pages,
                        },
                        data: Some(json!(extracted_data)),
                        logs: self.logs.clone(),
                        checkpoint: self.last_checkpoint.clone(),
                        hints: vec!["Flow pages completed but verification failed".to_string()],
                        changes: self.calculate_changes(),
                    });
                }
            }
        }

        Ok(IntentResult {
            status: IntentStatus::Success,
            data: (!extracted_data.is_empty()).then(|| json!(extracted_data)),
            logs: self.logs.clone(),
            checkpoint: self.last_checkpoint.clone(),
            hints: vec![],
            changes: self.calculate_changes(),
        })
    }

    /// Execute a single page within a flow.
    #[async_recursion]
    async fn execute_page(
        &mut self,
        page: &PageDef,
        intent: &IntentDefinition,
    ) -> Result<Option<Value>, ExecutorError> {
        self.wait_for_url_pattern(&page.url_pattern, intent.options.timeout)
            .await?;

        for action in &page.intents {
            match action {
                PageAction::IntentRef(intent_name) => {
                    self.logs.push(format!(
                        "Page '{}': executing intent '{}'",
                        page.name, intent_name
                    ));

                    let sub_result = {
                        let mut sub_executor =
                            IntentExecutor::new(self.backend, self.registry, self.verifier);
                        sub_executor.variables = self.variables.clone();
                        sub_executor
                            .execute(intent_name, self.variables.clone())
                            .await
                    };

                    let res = sub_result?;
                    self.logs.extend(res.logs);

                    match res.status {
                        IntentStatus::Success => {}
                        IntentStatus::Failed(e) => {
                            return Err(ExecutorError::IntentFailed(format!(
                                "Sub-intent '{intent_name}' failed: {e}"
                            )));
                        }
                        IntentStatus::PartialSuccess { .. } => {
                            return Err(ExecutorError::IntentFailed(format!(
                                "Sub-intent '{intent_name}' only partially succeeded"
                            )));
                        }
                    }
                }
                PageAction::Inline { steps } => {
                    self.logs
                        .push(format!("Page '{}': executing inline steps", page.name));
                    for step in steps {
                        self.execute_step_with_retry(step, &intent.options.retry)
                            .await?;
                    }
                }
            }
        }

        // Extract data if specified
        let Some(extract_rules) = &page.extract else {
            return Ok(None);
        };

        self.perform_scan().await?;

        let Some(scan) = &self.last_scan else {
            return Ok(None);
        };

        let extracted_data: HashMap<String, Value> = extract_rules
            .iter()
            .filter_map(|(key, rule)| {
                let selector = rule.get("selector")?.as_str()?;
                let element = scan.elements.iter().find(|e| e.selector == selector)?;
                let text = element.text.clone().unwrap_or_default();
                Some((key.clone(), json!(text)))
            })
            .collect();

        Ok((!extracted_data.is_empty()).then(|| json!(extracted_data)))
    }

    /// Wait for the current URL to match a pattern.
    async fn wait_for_url_pattern(
        &mut self,
        pattern: &str,
        timeout_ms: u64,
    ) -> Result<(), ExecutorError> {
        let regex = Regex::new(pattern)
            .map_err(|e| ExecutorError::InvalidUrlPattern(format!("{pattern}: {e}")))?;

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);
        let poll_interval = std::time::Duration::from_millis(500);

        loop {
            self.perform_scan().await?;

            if let Some(scan) = &self.last_scan
                && regex.is_match(&scan.page.url)
            {
                self.logs.push(format!(
                    "URL matched pattern '{pattern}': {}",
                    scan.page.url
                ));
                return Ok(());
            }

            if start.elapsed() >= timeout {
                let current_url = self
                    .last_scan
                    .as_ref()
                    .map_or("unknown".to_string(), |s| s.page.url.clone());
                return Err(ExecutorError::FlowUrlPatternTimeout(
                    pattern.to_string(),
                    current_url,
                ));
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}

/// Normalizes text for comparison by lowercasing and removing extra whitespace.
fn normalize_text(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Calculates a match score for a form field based on the key.
/// Higher scores indicate better matches.
fn score_form_field(element: &crate::protocol::Element, key: &str) -> u32 {
    // Only consider form fields
    if !matches!(
        element.element_type.as_str(),
        "input" | "textarea" | "select"
    ) {
        return 0;
    }

    let key_normalized = normalize_text(key);
    let mut score = 0u32;

    // Exact name match: 100 points
    if let Some(name) = element.attributes.get("name")
        && normalize_text(name) == key_normalized
    {
        return 100;
    }

    // Exact id match: 100 points
    if let Some(id) = element.attributes.get("id")
        && normalize_text(id) == key_normalized
    {
        return 100;
    }

    // Label matching
    if let Some(label) = &element.label {
        let label_normalized = normalize_text(label);
        if label_normalized == key_normalized {
            score = score.max(90); // Exact label: 90 points
        } else if label_normalized.contains(&key_normalized) {
            score = score.max(45); // Label contains: 45 points
        }
    }

    // Placeholder matching
    if let Some(placeholder) = &element.placeholder {
        let ph_normalized = normalize_text(placeholder);
        if ph_normalized == key_normalized {
            score = score.max(80); // Exact placeholder: 80 points
        } else if ph_normalized.contains(&key_normalized) {
            score = score.max(40); // Placeholder contains: 40 points
        }
    }

    // ARIA label matching
    if let Some(aria) = element.attributes.get("aria-label") {
        let aria_normalized = normalize_text(aria);
        if aria_normalized == key_normalized {
            score = score.max(85); // Exact ARIA label: 85 points
        } else if aria_normalized.contains(&key_normalized) {
            score = score.max(42); // ARIA label contains: 42 points
        }
    }

    // Semantic type matching: 75 points
    let semantic_score = semantic_field_score(element, &key_normalized);
    if semantic_score > 0 {
        score = score.max(semantic_score);
    }

    score
}

/// Returns a score based on semantic matching between the key and input type/autocomplete.
fn semantic_field_score(element: &crate::protocol::Element, key_normalized: &str) -> u32 {
    let input_type = element
        .attributes
        .get("type")
        .map(|s| s.as_str())
        .unwrap_or("text");
    let autocomplete = element
        .attributes
        .get("autocomplete")
        .map(|s| normalize_text(s));

    match key_normalized {
        "email" | "e-mail" | "email address" => {
            if input_type == "email" || autocomplete.as_deref() == Some("email") {
                return 75;
            }
        }
        "password" | "pass" | "pwd" => {
            if input_type == "password"
                || autocomplete.as_deref() == Some("current-password")
                || autocomplete.as_deref() == Some("new-password")
            {
                return 75;
            }
        }
        "phone" | "telephone" | "phone number" | "tel" => {
            if input_type == "tel" || autocomplete.as_deref() == Some("tel") {
                return 75;
            }
        }
        "username" | "user" | "login" => {
            if autocomplete.as_deref() == Some("username") {
                return 75;
            }
        }
        "name" | "full name" | "your name" => {
            if autocomplete.as_deref() == Some("name") {
                return 75;
            }
        }
        "first name" | "firstname" | "given name" => {
            if autocomplete.as_deref() == Some("given-name") {
                return 75;
            }
        }
        "last name" | "lastname" | "surname" | "family name" => {
            if autocomplete.as_deref() == Some("family-name") {
                return 75;
            }
        }
        _ => {}
    }

    0
}

/// Finds the best matching form field for a given key from scan results.
fn find_best_form_field<'a>(
    elements: &'a [crate::protocol::Element],
    key: &str,
) -> Option<&'a crate::protocol::Element> {
    elements
        .iter()
        .filter_map(|el| {
            let score = score_form_field(el, key);
            (score > 0).then_some((el, score))
        })
        .max_by_key(|(_, score)| *score)
        .map(|(el, _)| el)
}
