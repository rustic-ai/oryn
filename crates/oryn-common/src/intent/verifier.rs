use crate::command::Target;
use crate::intent::definition::{Condition, TargetKind};
use crate::protocol::ScanResult;
use crate::resolver::{ResolutionStrategy, ResolverContext, resolve_target};
use async_recursion::async_recursion;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum VerificationError {
    #[error("Condition failed: {0}")]
    Failed(String),
    #[error("Evaluation error: {0}")]
    Error(String),
    #[error("Resolution error: {0}")]
    Resolution(#[from] crate::resolver::ResolverError),
}

pub struct VerifierContext<'a> {
    pub scan_result: &'a ScanResult,
    pub variables: Option<&'a HashMap<String, Value>>,
}

impl<'a> VerifierContext<'a> {
    pub fn new(scan_result: &'a ScanResult) -> Self {
        Self {
            scan_result,
            variables: None,
        }
    }

    pub fn with_variables(
        scan_result: &'a ScanResult,
        variables: &'a HashMap<String, Value>,
    ) -> Self {
        Self {
            scan_result,
            variables: Some(variables),
        }
    }

    pub fn resolve_target_exists(
        &self,
        target_spec: &crate::intent::definition::TargetSpec,
    ) -> Result<Option<Target>, VerificationError> {
        let ctx = ResolverContext::new(self.scan_result);
        let target = Self::convert_target_spec(target_spec)?;

        match resolve_target(&target, &ctx, ResolutionStrategy::Best) {
            Ok(t) => Ok(Some(t)),
            Err(_) => {
                if let Some(fallback) = &target_spec.fallback {
                    self.resolve_target_exists(fallback)
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn convert_target_spec(
        spec: &crate::intent::definition::TargetSpec,
    ) -> Result<Target, VerificationError> {
        Ok(match &spec.kind {
            TargetKind::Pattern { pattern } => Target::Text(pattern.clone()),
            TargetKind::Role { role } => Target::Role(role.clone()),
            // Note: MatchType::Regex is intentionally not implemented - the resolver's
            // scoring handles exact/contains matching well. See definition.rs for details.
            TargetKind::Text { text, .. } => Target::Text(text.clone()),
            TargetKind::Selector { selector } => Target::Selector(selector.clone()),
            TargetKind::Id { id } => Target::Id(*id as usize),
            TargetKind::Near { near, anchor } => Target::Near {
                target: Box::new(Self::convert_target_spec(near)?),
                anchor: Box::new(Self::convert_target_spec(anchor)?),
            },
            TargetKind::Inside { inside, container } => Target::Inside {
                target: Box::new(Self::convert_target_spec(inside)?),
                container: Box::new(Self::convert_target_spec(container)?),
            },
            TargetKind::After { after, anchor } => Target::After {
                target: Box::new(Self::convert_target_spec(after)?),
                anchor: Box::new(Self::convert_target_spec(anchor)?),
            },
            TargetKind::Before { before, anchor } => Target::Before {
                target: Box::new(Self::convert_target_spec(before)?),
                anchor: Box::new(Self::convert_target_spec(anchor)?),
            },
            TargetKind::Contains { contains, content } => Target::Contains {
                target: Box::new(Self::convert_target_spec(contains)?),
                content: Box::new(Self::convert_target_spec(content)?),
            },
        })
    }
}

pub struct Verifier;

impl Default for Verifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Verifier {
    pub fn new() -> Self {
        Self
    }
    #[async_recursion]
    #[allow(clippy::only_used_in_recursion)]
    pub async fn verify(
        &self,
        condition: &Condition,
        context: &VerifierContext,
    ) -> Result<bool, VerificationError> {
        match condition {
            Condition::All(conditions) => {
                for cond in conditions {
                    if !self.verify(cond, context).await? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Condition::Any(conditions) => {
                for cond in conditions {
                    if self.verify(cond, context).await? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Condition::PatternExists(pattern_name) => {
                if let Some(patterns) = &context.scan_result.patterns {
                    let exists = match pattern_name.as_str() {
                        "login" => patterns.login.is_some(),
                        "search" => patterns.search.is_some(),
                        "pagination" => patterns.pagination.is_some(),
                        "modal" => patterns.modal.is_some(),
                        "cookie_banner" => patterns.cookie_banner.is_some(),
                        _ => false,
                    };
                    Ok(exists)
                } else {
                    Ok(false)
                }
            }
            Condition::PatternGone(pattern_name) => {
                if let Some(patterns) = &context.scan_result.patterns {
                    let exists = match pattern_name.as_str() {
                        "login" => patterns.login.is_some(),
                        "search" => patterns.search.is_some(),
                        "pagination" => patterns.pagination.is_some(),
                        "modal" => patterns.modal.is_some(),
                        "cookie_banner" => patterns.cookie_banner.is_some(),
                        _ => false,
                    };
                    Ok(!exists)
                } else {
                    Ok(true)
                }
            }
            Condition::Visible(target) => Ok(context.resolve_target_exists(target)?.is_some()),
            Condition::Hidden(target) => Ok(context.resolve_target_exists(target)?.is_none()),
            Condition::UrlContains(substrings) => {
                let url = &context.scan_result.page.url;
                Ok(substrings.iter().any(|s| url.contains(s)))
            }
            Condition::UrlMatches(regex) => {
                match Regex::new(regex) {
                    Ok(re) => Ok(re.is_match(&context.scan_result.page.url)),
                    Err(_) => {
                        // Fallback to contains for invalid regex
                        Ok(context.scan_result.page.url.contains(regex))
                    }
                }
            }
            Condition::TextContains { text, within } => {
                if let Some(target_spec) = within {
                    if let Some(Target::Id(id)) = context.resolve_target_exists(target_spec)? {
                        if let Some(el) = context
                            .scan_result
                            .elements
                            .iter()
                            .find(|e| e.id as usize == id)
                        {
                            Ok(el.text.as_deref().unwrap_or("").contains(text)
                                || el.label.as_deref().unwrap_or("").contains(text))
                        } else {
                            Ok(false)
                        }
                    } else {
                        Ok(false)
                    }
                } else {
                    // Check whole page context (e.g. any element)
                    Ok(context.scan_result.elements.iter().any(|e| {
                        e.text.as_deref().unwrap_or("").contains(text)
                            || e.label.as_deref().unwrap_or("").contains(text)
                    }))
                }
            }
            Condition::Count { selector, min, max } => {
                // Basic selector counting
                let count = context
                    .scan_result
                    .elements
                    .iter()
                    .filter(|e| e.selector.contains(selector) || e.element_type == *selector)
                    .count();

                if let Some(min) = min
                    && count < *min
                {
                    return Ok(false);
                }
                if let Some(max) = max
                    && count > *max
                {
                    return Ok(false);
                }
                Ok(true)
            }
            Condition::Expression(expr) => {
                // Handle variable references like "$reject"
                if let Some(var_name) = expr.strip_prefix('$') {
                    if let Some(vars) = context.variables
                        && let Some(value) = vars.get(var_name)
                    {
                        return Ok(is_truthy(value));
                    }
                    // Variable not found = falsy
                    Ok(false)
                } else {
                    // Literal expression: "true" or "1" are truthy
                    Ok(expr == "true" || expr == "1")
                }
            }
        }
    }
}

/// Determines if a JSON value is "truthy" for condition evaluation.
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty() && s != "false" && s != "0",
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Null => false,
    }
}
