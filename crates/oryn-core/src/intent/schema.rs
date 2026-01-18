use crate::intent::definition::{IntentDefinition, Step};
use std::collections::HashSet;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Intent name cannot be empty")]
    EmptyName,
    #[error("Intent must have at least one step")]
    NoSteps,
    #[error("Duplicate parameter name: {0}")]
    DuplicateParameter(String),
    #[error("Invalid step: {0}")]
    InvalidStep(String),
}

pub trait Validatable {
    fn validate(&self) -> Result<(), ValidationError>;
}

impl Validatable for IntentDefinition {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError::EmptyName);
        }

        if self.steps.is_empty() {
            return Err(ValidationError::NoSteps);
        }

        let mut param_names = HashSet::new();
        for param in &self.parameters {
            if !param_names.insert(&param.name) {
                return Err(ValidationError::DuplicateParameter(param.name.clone()));
            }
        }

        for step in &self.steps {
            step.validate()?;
        }

        Ok(())
    }
}

impl Validatable for Step {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Step::Branch(wrapper) => {
                if wrapper.branch.then_steps.is_empty() && wrapper.branch.else_steps.is_empty() {
                    return Err(ValidationError::InvalidStep(
                        "Branch must have at least one 'then' or 'else' step".into(),
                    ));
                }
                for s in &wrapper.branch.then_steps {
                    s.validate()?;
                }
                for s in &wrapper.branch.else_steps {
                    s.validate()?;
                }
            }
            Step::Loop(wrapper) => {
                if wrapper.loop_.steps.is_empty() {
                    return Err(ValidationError::InvalidStep(
                        "Loop must have body steps".into(),
                    ));
                }
                for s in &wrapper.loop_.steps {
                    s.validate()?;
                }
            }
            Step::Try(wrapper) => {
                if wrapper.try_.steps.is_empty() {
                    return Err(ValidationError::InvalidStep(
                        "Try block must have steps".into(),
                    ));
                }
                for s in &wrapper.try_.steps {
                    s.validate()?;
                }
                for s in &wrapper.try_.catch {
                    s.validate()?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
