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
    let mut best_match: Option<(&crate::protocol::Element, u32)> = None;

    for element in elements {
        let score = score_form_field(element, key);
        if score > 0 {
            match &best_match {
                Some((_, best_score)) if score > *best_score => {
                    best_match = Some((element, score));
                }
                None => {
                    best_match = Some((element, score));
                }
                _ => {}
            }
        }
    }

    best_match.map(|(el, _)| el)
}
