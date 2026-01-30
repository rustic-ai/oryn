//! Core Resolution Engine
//!
//! This module contains the main resolution logic that is backend-independent.
//! It uses the SelectorResolver trait to abstract the single backend-dependent operation
//! (CSS/XPath selector queries).
//!
//! All semantic resolution (text matching, label association, inference rules) is
//! backend-independent and operates purely on scan data structures.

use super::result::ResolutionError;
use super::{
    find_associated_control, get_inference_rules, validate_requirement, AssociationResult,
    CommandMeta, ResolutionContext, SelectorResolver, TargetRequirement,
};
use crate::ast;
use oryn_common::protocol::ScanResult;
use oryn_common::resolver::{self, Target};

use async_recursion::async_recursion;

pub struct ResolutionEngine;

impl ResolutionEngine {
    /// Find an element by matching its selector against the search string.
    fn find_by_selector_match(ctx: &ResolutionContext, s: &str) -> Option<u32> {
        ctx.elements()
            .find(|e| e.selector == s || e.selector.contains(s))
            .map(|e| e.id)
    }

    /// Find an element by matching against text content, label, or placeholder.
    /// Prioritizes exact matches over partial matches.
    fn find_by_text_content(ctx: &ResolutionContext, s: &str) -> Option<u32> {
        fn matches_field(field: &Option<String>, target: &str, exact: bool) -> bool {
            field.as_ref().is_some_and(|v| {
                if exact {
                    v == target
                } else {
                    v.contains(target)
                }
            })
        }

        fn matches_element(e: &oryn_common::protocol::Element, s: &str, exact: bool) -> bool {
            matches_field(&e.text, s, exact)
                || matches_field(&e.label, s, exact)
                || matches_field(&e.placeholder, s, exact)
        }

        // First, try exact match; fallback to partial match (contains)
        ctx.elements()
            .find(|e| matches_element(e, s, true))
            .or_else(|| ctx.elements().find(|e| matches_element(e, s, false)))
            .map(|e| e.id)
    }

    /// Find an element by text or selector (fallback when semantic resolution fails).
    fn find_element_by_text_or_selector(ctx: &ResolutionContext, s: &str) -> Option<u32> {
        Self::find_by_selector_match(ctx, s).or_else(|| Self::find_by_text_content(ctx, s))
    }

    /// Resolve a command, returning a command with all targets as Target::Id.
    pub async fn resolve<S: SelectorResolver + Send>(
        cmd: ast::Command,
        scan: &ScanResult,
        selector_resolver: &mut S,
    ) -> Result<ast::Command, ResolutionError> {
        let ctx = ResolutionContext::new(scan);

        Self::resolve_command(cmd, &ctx, selector_resolver).await
    }

    async fn resolve_command<S: SelectorResolver + Send>(
        cmd: ast::Command,
        ctx: &ResolutionContext<'_>,
        selector_resolver: &mut S,
    ) -> Result<ast::Command, ResolutionError> {
        let meta = CommandMeta::for_command(&cmd);

        fn make_id_target(id: u32) -> ast::Target {
            ast::Target {
                atomic: ast::TargetAtomic::Id(id as usize),
                relation: None,
            }
        }

        fn make_click_cmd(id: u32) -> ast::Command {
            ast::Command::Click(ast::ClickCmd {
                target: make_id_target(id),
                double: false,
                right: false,
                middle: false,
                force: false,
                ctrl: false,
                shift: false,
                alt: false,
                timeout: None,
            })
        }

        macro_rules! resolve_target_to_id {
            ($target:expr) => {{
                let resolver_target = $target.to_resolver_target();
                Self::resolve_target(
                    &resolver_target,
                    &meta.requirement,
                    meta.allows_inference,
                    ctx,
                    selector_resolver,
                )
                .await?
            }};
        }

        match cmd {
            ast::Command::Click(mut cmd) => {
                cmd.target = make_id_target(resolve_target_to_id!(&cmd.target));
                Ok(ast::Command::Click(cmd))
            }

            ast::Command::Type(mut cmd) => {
                cmd.target = make_id_target(resolve_target_to_id!(&cmd.target));
                Ok(ast::Command::Type(cmd))
            }

            ast::Command::Submit(mut cmd) => {
                let id = match &cmd.target {
                    Some(target) => resolve_target_to_id!(target),
                    None => {
                        Self::resolve_target(
                            &Target::Infer,
                            &meta.requirement,
                            true,
                            ctx,
                            selector_resolver,
                        )
                        .await?
                    }
                };
                cmd.target = Some(make_id_target(id));
                Ok(ast::Command::Submit(cmd))
            }

            ast::Command::Check(mut cmd) => {
                cmd.target = make_id_target(resolve_target_to_id!(&cmd.target));
                Ok(ast::Command::Check(cmd))
            }

            ast::Command::Uncheck(mut cmd) => {
                cmd.target = make_id_target(resolve_target_to_id!(&cmd.target));
                Ok(ast::Command::Uncheck(cmd))
            }

            ast::Command::Clear(mut cmd) => {
                cmd.target = make_id_target(resolve_target_to_id!(&cmd.target));
                Ok(ast::Command::Clear(cmd))
            }

            ast::Command::Focus(mut cmd) => {
                cmd.target = make_id_target(resolve_target_to_id!(&cmd.target));
                Ok(ast::Command::Focus(cmd))
            }

            ast::Command::Hover(mut cmd) => {
                cmd.target = make_id_target(resolve_target_to_id!(&cmd.target));
                Ok(ast::Command::Hover(cmd))
            }

            ast::Command::Select(mut cmd) => {
                cmd.target = make_id_target(resolve_target_to_id!(&cmd.target));
                Ok(ast::Command::Select(cmd))
            }

            ast::Command::Dismiss(cmd) => {
                // Keywords are passed directly to the scanner action
                if Self::is_dismiss_fallback_keyword(&cmd.target) {
                    return Ok(ast::Command::Dismiss(cmd));
                }

                // Resolve as a text target and convert to a click
                let text_target = Target::Text(cmd.target.clone());
                Self::resolve_target(
                    &text_target,
                    &meta.requirement,
                    true,
                    ctx,
                    selector_resolver,
                )
                .await
                .map(make_click_cmd)
            }

            ast::Command::AcceptCookies => {
                // Try to infer cookie accept button
                match Self::resolve_target(
                    &Target::Infer,
                    &meta.requirement,
                    true,
                    ctx,
                    selector_resolver,
                )
                .await
                {
                    Ok(id) => Ok(make_click_cmd(id)),
                    Err(_) => {
                        // Keep as AcceptCookies for backend to handle
                        Ok(ast::Command::AcceptCookies)
                    }
                }
            }

            // Commands that don't need resolution
            other => Ok(other),
        }
    }

    fn is_dismiss_fallback_keyword(target: &str) -> bool {
        matches!(
            target.to_lowercase().as_str(),
            "modal" | "modals" | "popup" | "popups" | "banner" | "banners" | "cookies"
        )
    }

    /// Resolve a single target to an element ID.
    #[cfg_attr(not(target_arch = "wasm32"), async_recursion)]
    #[cfg_attr(target_arch = "wasm32", async_recursion(?Send))]
    async fn resolve_target<S>(
        target: &Target,
        requirement: &TargetRequirement,
        allows_inference: bool,
        ctx: &ResolutionContext<'_>,
        selector_resolver: &mut S,
    ) -> Result<u32, ResolutionError>
    where
        S: SelectorResolver + Send,
    {
        match target {
            // Already resolved
            Target::Id(id) => {
                let id32 = *id as u32;
                // TRUST THE ID: If the user/agent specified an ID, assume it's actionable.
                // The scanner has already filtered for "referenceable" elements.
                Ok(id32)
            }

            // Inference requested
            Target::Infer => {
                if allows_inference {
                    Self::infer_target(requirement, ctx, selector_resolver).await
                } else {
                    Err(ResolutionError {
                        target: "Infer".into(),
                        reason: "This command doesn't support target inference".into(),
                        attempted: vec![],
                    })
                }
            }

            // CSS/XPath selector - use trait abstraction
            Target::Selector(selector) => {
                Self::resolve_selector(selector, requirement, selector_resolver).await
            }

            // Semantic targets - use existing resolver
            Target::Text(s) | Target::Role(s) => {
                let strategy = requirement.to_strategy();
                let resolved_id =
                    match resolver::resolve_target(target, &ctx.to_resolver_context(), strategy) {
                        Ok(Target::Id(id)) => Some(id as u32),
                        _ => Self::find_element_by_text_or_selector(ctx, s),
                    };

                match resolved_id {
                    Some(id) => {
                        // Check if the resolved element satisfies the requirement
                        if validate_requirement(id, requirement, ctx) {
                            return Ok(id);
                        }

                        // Element doesn't satisfy requirement - try finding associated control
                        match find_associated_control(id, requirement, ctx) {
                            AssociationResult::Found(control_id) => Ok(control_id),
                            AssociationResult::NoAssociation => {
                                // For Clickable/Checkable, return the element anyway:
                                // - Labels trigger actions on associated controls (browser handles focus/toggle)
                                // - Event bubbling means clicking text inside a button still works
                                // This preserves backward compatibility with the old resolver
                                if matches!(
                                    requirement,
                                    TargetRequirement::Clickable | TargetRequirement::Checkable
                                ) {
                                    return Ok(id);
                                }

                                Err(ResolutionError {
                                    target: format!("{:?}", target),
                                    reason: format!(
                                        "Element '{}' (id={}) doesn't satisfy {:?} and has no associated control",
                                        s, id, requirement
                                    ),
                                    attempted: vec![
                                        "semantic_resolution".into(),
                                        "requirement_validation".into(),
                                        "label_association".into(),
                                    ],
                                })
                            }
                        }
                    }
                    None => {
                        // Check if user tried to use element ID with quotes (common mistake)
                        let mut reason = format!("No element matches text {:?}", s);
                        let mut hint = None;

                        if s.parse::<u32>().is_ok() {
                            // Text is numeric - user likely meant to use ID without quotes
                            hint = Some(format!(
                                "Syntax error: Element IDs should not use quotes.\n\
                                 \n\
                                 If you meant element ID {}: use `click {}` (without quotes)\n\
                                 If you meant to search for text \"{}\": the text wasn't found on page\n\
                                 \n\
                                 Remember: Numbers are IDs (no quotes). Text uses quotes.\n\
                                 \n\
                                 Run 'observe' to see available elements with their [IDs].",
                                s, s, s
                            ));
                            reason = format!("No element matches text \"{}\" (numeric)", s);
                        }

                        let mut error = ResolutionError {
                            target: format!("{:?}", target),
                            reason,
                            attempted: vec![
                                "semantic_resolution".into(),
                                "selector_fallback".into(),
                                "text_fallback".into(),
                            ],
                        };

                        if let Some(h) = hint {
                            error.reason.push_str("\n\n");
                            error.reason.push_str(&h);
                        }

                        Err(error)
                    }
                }
            }

            // Relational targets - use existing resolver (already recursive)
            Target::Near { .. }
            | Target::Inside { .. }
            | Target::After { .. }
            | Target::Before { .. }
            | Target::Contains { .. } => {
                let strategy = requirement.to_strategy();
                match resolver::resolve_target(target, &ctx.to_resolver_context(), strategy) {
                    Ok(Target::Id(id)) => Ok(id as u32),
                    Ok(_) => Err(ResolutionError {
                        target: format!("{:?}", target),
                        reason: "Resolution didn't produce an ID".into(),
                        attempted: vec!["relational_resolution".into()],
                    }),
                    Err(e) => Err(ResolutionError {
                        target: format!("{:?}", target),
                        reason: e.to_string(),
                        attempted: vec!["relational_resolution".into()],
                    }),
                }
            }
        }
    }

    /// Infer a target using inference rules.
    #[cfg_attr(not(target_arch = "wasm32"), async_recursion)]
    #[cfg_attr(target_arch = "wasm32", async_recursion(?Send))]
    async fn infer_target<S>(
        requirement: &TargetRequirement,
        ctx: &ResolutionContext<'_>,
        selector_resolver: &mut S,
    ) -> Result<u32, ResolutionError>
    where
        S: SelectorResolver + Send,
    {
        let mut rules = get_inference_rules(requirement);
        rules.sort_by_key(|r| -r.priority);
        let mut attempted = vec![];

        for rule in rules {
            attempted.push(rule.name.to_string());

            if let Some(inferred_target) = (rule.infer)(ctx) {
                // Try to resolve the inferred target
                // Note: We're passing 'false' for allows_inference to prevent infinite recursion
                match Self::resolve_target(
                    &inferred_target,
                    requirement,
                    false,
                    ctx,
                    selector_resolver,
                )
                .await
                {
                    Ok(id) => return Ok(id),
                    Err(_) => continue, // Try next rule
                }
            }
        }

        Err(ResolutionError {
            target: "Infer".into(),
            reason: format!("No inference rule satisfied {:?}", requirement),
            attempted,
        })
    }

    /// Resolve a CSS selector using the provided selector resolver.
    async fn resolve_selector<S: SelectorResolver>(
        selector: &str,
        _requirement: &TargetRequirement,
        selector_resolver: &mut S,
    ) -> Result<u32, ResolutionError> {
        match selector_resolver.resolve_selector(selector).await {
            Ok(Some(id)) => Ok(id),
            Ok(None) => Err(ResolutionError {
                target: selector.into(),
                reason: "Element not found".into(),
                attempted: vec!["selector_resolution".into()],
            }),
            Err(e) => Err(ResolutionError {
                target: selector.into(),
                reason: e.to_string(),
                attempted: vec!["selector_resolution".into()],
            }),
        }
    }
}
