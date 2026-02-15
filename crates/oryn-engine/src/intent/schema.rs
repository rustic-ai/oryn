use oryn_common::intent::definition::{FlowDefinition, IntentDefinition, PageTransition, Step};
use std::collections::HashSet;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Intent name cannot be empty")]
    EmptyName,
    #[error("Intent must have at least one step or a flow")]
    NoStepsOrFlow,
    #[error("Intent cannot have both steps and flow defined (mutually exclusive)")]
    StepsAndFlowMutuallyExclusive,
    #[error("Duplicate parameter name: {0}")]
    DuplicateParameter(String),
    #[error("Invalid step: {0}")]
    InvalidStep(String),
    #[error("Flow must have at least one page")]
    FlowNoPages,
    #[error("Duplicate page name in flow: {0}")]
    DuplicatePageName(String),
    #[error("Invalid page transition: page '{0}' not found")]
    InvalidPageTransition(String),
    #[error("Invalid start page: '{0}' not found")]
    InvalidStartPage(String),
    #[error("Page '{0}' has invalid error handler: page '{1}' not found")]
    InvalidErrorHandler(String, String),
}

pub trait Validatable {
    fn validate(&self) -> Result<(), ValidationError>;
}

impl Validatable for IntentDefinition {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError::EmptyName);
        }

        // Check mutual exclusivity of steps and flow
        let has_steps = !self.steps.is_empty();
        let has_flow = self.flow.is_some();

        if has_steps && has_flow {
            return Err(ValidationError::StepsAndFlowMutuallyExclusive);
        }

        if !has_steps && !has_flow {
            return Err(ValidationError::NoStepsOrFlow);
        }

        let mut param_names = HashSet::new();
        for param in &self.parameters {
            if !param_names.insert(&param.name) {
                return Err(ValidationError::DuplicateParameter(param.name.clone()));
            }
        }

        // Validate steps if present
        for step in &self.steps {
            step.validate()?;
        }

        // Validate flow if present
        if let Some(flow) = &self.flow {
            flow.validate()?;
        }

        Ok(())
    }
}

impl Validatable for FlowDefinition {
    fn validate(&self) -> Result<(), ValidationError> {
        // Flow must have at least one page
        if self.pages.is_empty() {
            return Err(ValidationError::FlowNoPages);
        }

        // Collect all page names for validation
        let mut page_names = HashSet::new();
        for page in &self.pages {
            if !page_names.insert(&page.name) {
                return Err(ValidationError::DuplicatePageName(page.name.clone()));
            }
        }

        // Validate start page if specified
        if let Some(start) = &self.start
            && !page_names.contains(start)
        {
            return Err(ValidationError::InvalidStartPage(start.clone()));
        }

        // Validate page transitions
        for page in &self.pages {
            // Validate next transition
            if let Some(PageTransition::Page { page: target }) = &page.next
                && !page_names.contains(target)
            {
                return Err(ValidationError::InvalidPageTransition(target.clone()));
            }

            // Validate on_error handler
            if let Some(error_page) = &page.on_error
                && !page_names.contains(error_page)
            {
                return Err(ValidationError::InvalidErrorHandler(
                    page.name.clone(),
                    error_page.clone(),
                ));
            }

            // Validate inline steps in intents
            for intent in &page.intents {
                if let oryn_common::intent::definition::PageAction::Inline { steps } = intent {
                    for step in steps {
                        step.validate()?;
                    }
                }
            }
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
