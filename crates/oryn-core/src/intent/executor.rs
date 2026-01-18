use crate::backend::{Backend, BackendError};
use crate::command::{Command, Target};
use crate::intent::definition::{ActionStep, ActionType, Condition, Step, TargetKind, TargetSpec};
use crate::intent::registry::IntentRegistry;
use crate::intent::verifier::{Verifier, VerifierContext};
use crate::protocol::{
    ScanRequest, ScanResult, ScannerData, ScannerProtocolResponse, ScannerRequest,
};
use crate::resolver::{ResolutionStrategy, ResolverContext, resolve_target};
use crate::translator;
use async_recursion::async_recursion;
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentResult {
    pub success: bool,
    pub data: Option<Value>,
    pub logs: Vec<String>,
}

pub struct IntentExecutor<'a, B: Backend> {
    backend: &'a mut B,
    registry: &'a IntentRegistry,
    verifier: &'a Verifier,
    variables: HashMap<String, Value>,
    logs: Vec<String>,
    last_scan: Option<ScanResult>,
}

impl<'a, B: Backend> IntentExecutor<'a, B> {
    pub fn new(backend: &'a mut B, registry: &'a IntentRegistry, verifier: &'a Verifier) -> Self {
        Self {
            backend,
            registry,
            verifier,
            variables: HashMap::new(),
            logs: Vec::new(),
            last_scan: None,
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

        // 4. EXECUTE
        for step in &intent.steps {
            self.execute_step(step).await?;
        }

        // 5. VERIFY
        if let Some(success_cond) = &intent.success {
            // Need fresh scan for final verification
            self.perform_scan().await?;
            if let Some(scan) = &self.last_scan {
                let ctx = VerifierContext::new(scan);
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
        }

        // 6. RESPOND
        Ok(IntentResult {
            success: true,
            data: None, // TODO: Implement extraction
            logs: self.logs.clone(),
        })
    }

    async fn perform_scan(&mut self) -> Result<(), ExecutorError> {
        let scan_req = ScannerRequest::Scan(ScanRequest::default());
        let resp = self.backend.execute_scanner(scan_req).await?;

        if let ScannerProtocolResponse::Ok { data, .. } = resp
            && let ScannerData::Scan(result) = *data
        {
            self.last_scan = Some(result);
        }
        Ok(())
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
                } else {
                    // Treat 'over' as a numeric range "0..N" or just run max times if logic requires
                    // For now, simple range loop support if 'over' matches "start..end"
                    // Or if it's not a var, maybe it's a fixed list?
                    // Fallback: run 'max' times
                    (0..wrapper.loop_.max).map(|i| json!(i)).collect()
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
            Step::Checkpoint(_) => Ok(()),
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
                // Note: data_var was resolved but we parse data differently below
                let _ = self.resolve_variable(step.options.get("data"));
                // This resolves to a string value of the variable name if passed as "$data"
                // But we need the actual object.
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
                            // Strategy: Find best matching element in scan result
                            // 1. Exact match on name or id
                            // 2. Exact match on label
                            // 3. Contains match on label

                            let best_match = scan.elements.iter().find(|e| {
                                let name_match = e.attributes.get("name").map(|s| s.as_str())
                                    == Some(key)
                                    || e.attributes.get("id").map(|s| s.as_str()) == Some(key);
                                if name_match {
                                    return true;
                                }

                                // Only consider form fields
                                if matches!(
                                    e.element_type.as_str(),
                                    "input" | "textarea" | "select"
                                ) {
                                    if let Some(label) = &e.label {
                                        if label.eq_ignore_ascii_case(key) {
                                            return true;
                                        }
                                    }
                                    if let Some(aria) = e.attributes.get("aria-label") {
                                        if aria.eq_ignore_ascii_case(key) {
                                            return true;
                                        }
                                    }
                                }
                                false
                            });

                            if let Some(el) = best_match {
                                let t = Target::Id(el.id as usize);
                                let cmd = Command::Type(t, val_str.clone(), HashMap::new());
                                if let Ok(req) = translator::translate(&cmd) {
                                    if let Ok(_) = self.backend.execute_scanner(req).await {
                                        found_via_scan = true;
                                    }
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
            _ => {
                // Implement other actions like Wait, Scroll
                // Wait is essentially verify with retry
            }
        }
        Ok(())
    }

    async fn resolve_target_spec(&mut self, spec: &TargetSpec) -> Result<Target, ExecutorError> {
        // Refresh scan if needed?
        // For now, always refresh to point to latest DOM state
        self.perform_scan().await?;

        // Use last_scan
        let ctx = if let Some(scan) = &self.last_scan {
            ResolverContext::new(scan)
        } else {
            ResolverContext::empty()
        };

        self.convert_target_spec(spec, &ctx)
    }

    fn convert_target_spec(
        &self,
        spec: &TargetSpec,
        ctx: &ResolverContext,
    ) -> Result<Target, ExecutorError> {
        let target = match &spec.kind {
            TargetKind::Pattern { pattern } => Target::Text(pattern.clone()),
            TargetKind::Role { role } => Target::Role(role.clone()),
            TargetKind::Text { text, .. } => Target::Text(text.clone()),
            TargetKind::Selector { selector } => Target::Selector(selector.clone()),
            TargetKind::Id { id } => Target::Id(*id as usize),
        };

        Ok(resolve_target(&target, ctx, ResolutionStrategy::Best)?)
    }

    async fn evaluate_condition(&mut self, cond: &Condition) -> Result<bool, ExecutorError> {
        // Condition might rely on latest state
        self.perform_scan().await?;

        if let Some(scan) = &self.last_scan {
            let ctx = VerifierContext::new(scan);
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
}
