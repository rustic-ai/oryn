use crate::resolution::command_meta::CommandMeta;
use crate::resolution::context::ResolutionContext;
use crate::resolution::inference::get_inference_rules;
use crate::resolution::requirement::{ContainerType, TargetRequirement};
use crate::resolution::result::ResolutionError;

use crate::backend::Backend;
use crate::command::{Command, Target};
use crate::protocol::{
    ExecuteRequest, ScanResult, ScannerData, ScannerProtocolResponse, ScannerRequest,
};
use crate::resolver;

use async_recursion::async_recursion;
use std::collections::HashMap;

pub struct ResolutionEngine;

impl ResolutionEngine {
    /// Resolve a command, returning a command with all targets as Target::Id.
    pub async fn resolve<B: Backend>(
        cmd: Command,
        scan: &ScanResult,
        backend: &mut B,
    ) -> Result<Command, ResolutionError> {
        let ctx = ResolutionContext::new(scan);

        Self::resolve_command(cmd, &ctx, backend).await
    }

    async fn resolve_command<B: Backend>(
        cmd: Command,
        ctx: &ResolutionContext<'_>,
        backend: &mut B,
    ) -> Result<Command, ResolutionError> {
        let meta = CommandMeta::for_command(&cmd);

        // Helper macro to reduce boilerplate for simple target resolution
        macro_rules! resolve {
            ($target:expr) => {
                Self::resolve_target(
                    $target,
                    &meta.requirement,
                    meta.allows_inference,
                    ctx,
                    backend,
                )
                .await?
            };
        }

        match &cmd {
            Command::Click(target, opts) => Ok(Command::Click(
                Target::Id(resolve!(target) as usize),
                opts.clone(),
            )),

            Command::Type(target, text, opts) => Ok(Command::Type(
                Target::Id(resolve!(target) as usize),
                text.clone(),
                opts.clone(),
            )),

            Command::Submit(target) => Ok(Command::Submit(Target::Id(resolve!(target) as usize))),
            Command::Check(target) => Ok(Command::Check(Target::Id(resolve!(target) as usize))),
            Command::Uncheck(target) => Ok(Command::Uncheck(Target::Id(resolve!(target) as usize))),
            Command::Clear(target) => Ok(Command::Clear(Target::Id(resolve!(target) as usize))),
            Command::Focus(target) => Ok(Command::Focus(Target::Id(resolve!(target) as usize))),
            Command::Hover(target) => Ok(Command::Hover(Target::Id(resolve!(target) as usize))),

            Command::Select(target, value) => Ok(Command::Select(
                Target::Id(resolve!(target) as usize),
                value.clone(),
            )),

            Command::Dismiss(target, opts) => {
                Self::resolve_action_to_click(target, &meta, ctx, backend, || {
                    Self::is_dismiss_fallback_keyword(target)
                        .then(|| Command::Dismiss(target.clone(), opts.clone()))
                })
                .await
            }

            Command::Accept(target, opts) => {
                Self::resolve_action_to_click(target, &meta, ctx, backend, || {
                    Self::is_accept_fallback_keyword(target)
                        .then(|| Command::Accept(target.clone(), opts.clone()))
                })
                .await
            }

            _ => Ok(cmd),
        }
    }

    /// Resolve an action command (like Dismiss/Accept) that transforms to Click on success.
    async fn resolve_action_to_click<B, F>(
        target: &Target,
        meta: &CommandMeta,
        ctx: &ResolutionContext<'_>,
        backend: &mut B,
        fallback: F,
    ) -> Result<Command, ResolutionError>
    where
        B: Backend + ?Sized,
        F: FnOnce() -> Option<Command>,
    {
        match Self::resolve_target(
            target,
            &meta.requirement,
            meta.allows_inference,
            ctx,
            backend,
        )
        .await
        {
            Ok(id) => Ok(Command::Click(Target::Id(id as usize), HashMap::new())),
            Err(e) => fallback().ok_or(e),
        }
    }

    fn is_dismiss_fallback_keyword(target: &Target) -> bool {
        matches!(target, Target::Text(s) if matches!(s.to_lowercase().as_str(), "popups" | "modals"))
    }

    fn is_accept_fallback_keyword(target: &Target) -> bool {
        matches!(target, Target::Text(s) if s.to_lowercase() == "cookies")
    }

    /// Resolve a single target to an element ID.
    #[async_recursion]
    async fn resolve_target<B>(
        target: &Target,
        requirement: &TargetRequirement,
        allows_inference: bool,
        ctx: &ResolutionContext<'_>,
        backend: &mut B,
    ) -> Result<u32, ResolutionError>
    where
        B: Backend + ?Sized,
    {
        match target {
            // Already resolved
            Target::Id(id) => {
                let id32 = *id as u32;
                if Self::validate_requirement(id32, requirement, ctx) {
                    Ok(id32)
                } else {
                    Err(ResolutionError {
                        target: format!("{:?}", target),
                        reason: format!("Element {} doesn't satisfy {:?}", id, requirement),
                        attempted: vec![],
                    })
                }
            }

            // Inference requested
            Target::Infer => {
                if allows_inference {
                    Self::infer_target(requirement, ctx, backend).await
                } else {
                    Err(ResolutionError {
                        target: "Infer".into(),
                        reason: "This command doesn't support target inference".into(),
                        attempted: vec![],
                    })
                }
            }

            // CSS/XPath selector - needs browser query
            Target::Selector(selector) => {
                Self::resolve_selector(selector, requirement, backend).await
            }

            // Semantic targets - use existing resolver
            Target::Text(s) | Target::Role(s) => {
                let strategy = requirement.to_strategy();
                match resolver::resolve_target(target, &ctx.to_resolver_context(), strategy) {
                    Ok(Target::Id(id)) => Ok(id as u32),
                    _ => {
                        // Fallback: If semantic resolution failed, try matching against selectors
                        // This handles cases like `click "modal"` or `#id` passed as text
                        // We do a loose match on selector
                        if let Some(id) = ctx
                            .elements()
                            .find(|e| e.selector == *s || e.selector.contains(s))
                            .map(|e| e.id)
                        {
                            Ok(id)
                        } else {
                            // Secondary fallback: Try to find element by text content directly in ctx
                            if let Some(id) = ctx
                                .elements()
                                .find(|e| {
                                    e.text
                                        .as_ref()
                                        .map(|t| t == s || t.contains(s))
                                        .unwrap_or(false)
                                        || e.label
                                            .as_ref()
                                            .map(|l| l == s || l.contains(s))
                                            .unwrap_or(false)
                                        || e.placeholder
                                            .as_ref()
                                            .map(|p| p == s || p.contains(s))
                                            .unwrap_or(false)
                                })
                                .map(|e| e.id)
                            {
                                Ok(id)
                            } else {
                                Err(ResolutionError {
                                    target: format!("{:?}", target),
                                    reason: format!("No element matches target: {}", s),
                                    attempted: vec![
                                        "semantic_resolution".into(),
                                        "selector_fallback".into(),
                                        "text_fallback".into(),
                                    ],
                                })
                            }
                        }
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
    #[async_recursion]
    async fn infer_target<B>(
        requirement: &TargetRequirement,
        ctx: &ResolutionContext<'_>,
        backend: &mut B,
    ) -> Result<u32, ResolutionError>
    where
        B: Backend + ?Sized,
    {
        let rules = get_inference_rules(requirement);
        let mut attempted = vec![];

        let mut rules_sorted = rules;
        rules_sorted.sort_by_key(|r| -r.priority);

        for rule in rules_sorted {
            attempted.push(rule.name.to_string());

            if let Some(inferred_target) = (rule.infer)(ctx) {
                // Try to resolve the inferred target
                // Note: We're passing 'false' for allows_inference to prevent infinite recursion
                match Self::resolve_target(&inferred_target, requirement, false, ctx, backend).await
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

    /// Resolve a CSS selector by querying the browser.
    async fn resolve_selector<B: Backend + ?Sized>(
        selector: &str,
        _requirement: &TargetRequirement,
        backend: &mut B,
    ) -> Result<u32, ResolutionError> {
        let escaped = selector.replace('\\', "\\\\").replace('"', "\\\"");

        // Only try advanced fallback strategies if the selector is a simple word
        // (no special CSS characters like [ ] > + ~)
        let is_simple = !selector
            .chars()
            .any(|c| matches!(c, '[' | ']' | '>' | '+' | '~' | ' ' | ':'));

        let script = if is_simple {
            format!(
                r###"
                var selectors = [
                    '{}',
                    '#{}',
                    '[name="{}"]',
                    '[placeholder*="{}"]',
                    '[aria-label*="{}"]'
                ];
                var el = null;
                for (var i = 0; i < selectors.length; i++) {{
                    try {{
                        el = document.querySelector(selectors[i]);
                        if (el) break;
                    }} catch (e) {{}}
                }}
                if (!el) return {{ found: false }};
                var id = Oryn.State.inverseMap.get(el);
                if (id !== undefined) return {{ found: true, id: id }};
                id = Oryn.State.nextId++;
                Oryn.State.inverseMap.set(el, id);
                Oryn.State.elementMap.set(id, el);
                return {{ found: true, id: id }};
                "###,
                escaped, escaped, escaped, escaped, escaped
            )
        } else {
            format!(
                r###" 
                var el = null;
                try {{
                    el = document.querySelector("{}");
                }} catch (e) {{}}
                if (!el) return {{ found: false }};
                var id = Oryn.State.inverseMap.get(el);
                if (id !== undefined) return {{ found: true, id: id }};
                id = Oryn.State.nextId++;
                Oryn.State.inverseMap.set(el, id);
                Oryn.State.elementMap.set(id, el);
                return {{ found: true, id: id }};
                "###,
                escaped
            )
        };

        let req = ScannerRequest::Execute(ExecuteRequest {
            script,
            args: vec![],
        });

        match backend.execute_scanner(req).await {
            Ok(resp) => Self::parse_selector_response(&resp, selector),
            Err(e) => Err(ResolutionError {
                target: selector.into(),
                reason: e.to_string(),
                attempted: vec!["browser_query".into()],
            }),
        }
    }

    fn validate_requirement(
        id: u32,
        requirement: &TargetRequirement,
        ctx: &ResolutionContext,
    ) -> bool {
        let Some(elem) = ctx.get_element(id) else {
            return false;
        };

        let attr_is = |key: &str, value: &str| elem.attributes.get(key).is_some_and(|v| v == value);

        let attr_matches = |key: &str, values: &[&str]| {
            elem.attributes
                .get(key)
                .is_some_and(|v| values.contains(&v.as_str()))
        };

        let is_type = |t: &str| elem.element_type == t;

        match requirement {
            TargetRequirement::Any => true,

            TargetRequirement::Typeable => {
                matches!(elem.element_type.as_str(), "input" | "textarea" | "select")
                    || attr_is("contenteditable", "true")
            }

            TargetRequirement::Clickable => {
                matches!(elem.element_type.as_str(), "button" | "a")
                    || attr_is("role", "button")
                    || (is_type("input") && attr_matches("type", &["submit", "button", "reset"]))
            }

            TargetRequirement::Checkable => {
                attr_matches("type", &["checkbox", "radio"])
                    || attr_matches("role", &["checkbox", "radio", "switch"])
            }

            TargetRequirement::Submittable => {
                is_type("form")
                    || (is_type("button") && !attr_matches("type", &["button", "reset"]))
                    || (is_type("input") && attr_is("type", "submit"))
            }

            TargetRequirement::Container(ct) => match ct {
                ContainerType::Form => is_type("form"),
                ContainerType::Modal | ContainerType::Dialog => {
                    is_type("dialog") || attr_matches("role", &["dialog", "alertdialog"])
                }
                ContainerType::Any => true,
            },

            TargetRequirement::Selectable => is_type("select") || attr_is("role", "listbox"),

            TargetRequirement::Dismissable | TargetRequirement::Acceptable => {
                matches!(elem.element_type.as_str(), "button" | "a" | "input")
                    || attr_matches("role", &["button", "link"])
            }
        }
    }

    fn parse_selector_response(
        resp: &ScannerProtocolResponse,
        selector: &str,
    ) -> Result<u32, ResolutionError> {
        match resp {
            ScannerProtocolResponse::Ok { data, .. } => {
                if let ScannerData::Value(result) = data.as_ref()
                    && let Some(inner) = result.get("result")
                    && let Some(obj) = inner.as_object()
                    && obj.get("found").and_then(|v| v.as_bool()) == Some(true)
                    && let Some(id) = obj.get("id").and_then(|v| v.as_u64())
                {
                    return Ok(id as u32);
                }
                Err(ResolutionError {
                    target: selector.into(),
                    reason: "Element not found".into(),
                    attempted: vec!["browser_query".into()],
                })
            }
            ScannerProtocolResponse::Error { message, .. } => Err(ResolutionError {
                target: selector.into(),
                reason: message.clone(),
                attempted: vec!["browser_query".into()],
            }),
        }
    }
}
