use crate::command::Target;
use crate::intent::definition::{Condition, TargetKind};
use crate::protocol::ScanResult;
use crate::resolver::{ResolutionStrategy, ResolverContext, resolve_target};
use async_recursion::async_recursion;

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
}

impl<'a> VerifierContext<'a> {
    pub fn new(scan_result: &'a ScanResult) -> Self {
        Self { scan_result }
    }

    pub fn resolve_target_exists(
        &self,
        target_spec: &crate::intent::definition::TargetSpec,
    ) -> Result<Option<Target>, VerificationError> {
        let ctx = ResolverContext::new(self.scan_result);
        let target = self.convert_target_spec(target_spec, &ctx)?;

        match resolve_target(&target, &ctx, ResolutionStrategy::Best) {
            Ok(t) => Ok(Some(t)),
            Err(_) => Ok(None),
        }
    }

    fn convert_target_spec(
        &self,
        spec: &crate::intent::definition::TargetSpec,
        _ctx: &ResolverContext,
    ) -> Result<Target, VerificationError> {
        Ok(match &spec.kind {
            TargetKind::Pattern { pattern } => Target::Text(pattern.clone()),
            TargetKind::Role { role } => Target::Role(role.clone()),
            TargetKind::Text { text, .. } => Target::Text(text.clone()), // TODO: Handle match_type
            TargetKind::Selector { selector } => Target::Selector(selector.clone()),
            TargetKind::Id { id } => Target::Id(*id as usize),
        })
    }
}

pub struct Verifier;

impl Verifier {
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
            Condition::Visible(target) => Ok(context.resolve_target_exists(target)?.is_some()),
            Condition::Hidden(target) => Ok(context.resolve_target_exists(target)?.is_none()),
            Condition::UrlContains(substrings) => {
                let url = &context.scan_result.page.url;
                Ok(substrings.iter().any(|s| url.contains(s)))
            }
            Condition::UrlMatches(regex) => {
                // Simple strict equality for now, or use `regex` crate if needed
                // Implementation plan says "UrlMatches"
                // Assuming regex check or wildcard. For MVP, equality or simple contains.
                // Let's implement basics.
                Ok(context.scan_result.page.url.contains(regex))
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
            Condition::Expression(_) => Ok(false), // Placeholder
        }
    }
}
