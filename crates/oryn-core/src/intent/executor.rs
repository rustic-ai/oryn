use crate::backend::{Backend, BackendError};
use crate::command::{Command, Target};
use crate::intent::definition::{ActionStep, ActionType, Condition, Step, TargetKind, TargetSpec};
use crate::intent::registry::IntentRegistry;
use crate::intent::verifier::Verifier;
use crate::protocol::{ScanRequest, ScannerData, ScannerProtocolResponse, ScannerRequest};
use crate::resolver::{ResolutionStrategy, ResolverContext, resolve_target};
use crate::translator;
use async_recursion::async_recursion;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
}

impl<'a, B: Backend> IntentExecutor<'a, B> {
    pub fn new(backend: &'a mut B, registry: &'a IntentRegistry, verifier: &'a Verifier) -> Self {
        Self {
            backend,
            registry,
            verifier,
            variables: HashMap::new(),
            logs: Vec::new(),
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
        // Ensure we have a fresh scan for resolution
        if !self.backend.is_ready().await {
            self.backend.launch().await?;
        }

        // 4. EXECUTE
        for step in &intent.steps {
            self.execute_step(step).await?;
        }

        // 5. VERIFY
        if let Some(_success_cond) = &intent.success {
            // Verify success conditions
            // self.verifier.verify(&success_cond.conditions)...
        }

        // 6. RESPOND
        Ok(IntentResult {
            success: true,
            data: None,
            logs: self.logs.clone(),
        })
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
            Step::Loop(_wrapper) => {
                // TODO: Implement loop logic - evaluate 'over' expression and iterate
                Ok(())
            }
            Step::Try(wrapper) => {
                for s in &wrapper.try_.steps {
                    self.execute_step(s).await?;
                }
                // TODO: Implement catch logic on error
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
                // Translate to Command::Wait
            }
            ActionType::FillForm => {
                // Custom logic
            }
            _ => {
                // Implement other actions
            }
        }
        Ok(())
    }

    async fn resolve_target_spec(&mut self, spec: &TargetSpec) -> Result<Target, ExecutorError> {
        let scan_req = ScannerRequest::Scan(ScanRequest::default());
        let resp = self.backend.execute_scanner(scan_req).await?;

        let ctx = match resp {
            ScannerProtocolResponse::Ok { data, .. } => match *data {
                ScannerData::Scan(result) => ResolverContext::new(&result),
                _ => ResolverContext::empty(),
            },
            _ => ResolverContext::empty(),
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

    async fn evaluate_condition(&self, cond: &Condition) -> Result<bool, ExecutorError> {
        self.verifier
            .verify(cond)
            .await
            .map_err(ExecutorError::Verification)
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
