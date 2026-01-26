use crate::resolution::association::{
    AssociationResult, find_associated_control, is_actionable_label,
};
use crate::resolution::command_meta::CommandMeta;
use crate::resolution::context::ResolutionContext;
use crate::resolution::inference::get_inference_rules;
use crate::resolution::requirement::{ContainerType, TargetRequirement};
use crate::resolution::result::ResolutionError;

use crate::backend::Backend;
use oryn_common::protocol::{
    ExecuteRequest, ScanResult, ScannerAction, ScannerData, ScannerProtocolResponse,
};
use oryn_common::resolver::{self, Target};
use oryn_parser::ast;

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
    fn find_by_text_content(ctx: &ResolutionContext, s: &str) -> Option<u32> {
        ctx.elements()
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
    }

    /// Find an element by text or selector (fallback when semantic resolution fails).
    fn find_element_by_text_or_selector(ctx: &ResolutionContext, s: &str) -> Option<u32> {
        Self::find_by_selector_match(ctx, s).or_else(|| Self::find_by_text_content(ctx, s))
    }

    /// Resolve a command, returning a command with all targets as Target::Id.
    pub async fn resolve<B: Backend + ?Sized>(
        cmd: ast::Command,
        scan: &ScanResult,
        backend: &mut B,
    ) -> Result<ast::Command, ResolutionError> {
        let ctx = ResolutionContext::new(scan);

        Self::resolve_command(cmd, &ctx, backend).await
    }

    async fn resolve_command<B: Backend + ?Sized>(
        cmd: ast::Command,
        ctx: &ResolutionContext<'_>,
        backend: &mut B,
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
                    backend,
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
                        Self::resolve_target(&Target::Infer, &meta.requirement, true, ctx, backend)
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
                // If target is a keyword, pass directly to scanner action
                if Self::is_dismiss_fallback_keyword(&cmd.target) {
                    return Ok(ast::Command::Dismiss(cmd));
                }

                // Otherwise, try to resolve as a text target (e.g., specific element text)
                let text_target = Target::Text(cmd.target.clone());
                match Self::resolve_target(&text_target, &meta.requirement, true, ctx, backend)
                    .await
                {
                    Ok(id) => Ok(make_click_cmd(id)),
                    Err(e) => Err(e),
                }
            }

            ast::Command::AcceptCookies => {
                // Try to infer cookie accept button
                match Self::resolve_target(&Target::Infer, &meta.requirement, true, ctx, backend)
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
                if validate_requirement(id32, requirement, ctx) {
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
                                // Special case: labels can trigger actions on their associated controls
                                // (browser handles focus/toggle when clicking a label)
                                if matches!(
                                    requirement,
                                    TargetRequirement::Clickable | TargetRequirement::Checkable
                                ) && is_actionable_label(id, ctx)
                                {
                                    return Ok(id);
                                }

                                // Fallback: Return the element anyway for Clickable/Checkable
                                // Many UI patterns rely on event bubbling (clicking text inside a button/label)
                                // This preserves backward compatibility with the old resolver behavior
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
                    None => Err(ResolutionError {
                        target: format!("{:?}", target),
                        reason: format!("No element matches target: {}", s),
                        attempted: vec![
                            "semantic_resolution".into(),
                            "selector_fallback".into(),
                            "text_fallback".into(),
                        ],
                    }),
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
                        el = Oryn.ShadowUtils.querySelectorWithShadow(document.body, selectors[i]);
                        if (el) break;
                    }} catch (e) {{}}
                }}
                if (!el) return {{ found: false }};
                var id = Oryn.State.inverseMap.get(el);
                // Verify id still exists in elementMap (may have been cleared by scan)
                if (id !== undefined && Oryn.State.elementMap.has(id)) return {{ found: true, id: id }};
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
                    el = Oryn.ShadowUtils.querySelectorWithShadow(document.body, "{}");
                }} catch (e) {{}}
                if (!el) return {{ found: false }};
                var id = Oryn.State.inverseMap.get(el);
                // Verify id still exists in elementMap (may have been cleared by scan)
                if (id !== undefined && Oryn.State.elementMap.has(id)) return {{ found: true, id: id }};
                id = Oryn.State.nextId++;
                Oryn.State.inverseMap.set(el, id);
                Oryn.State.elementMap.set(id, el);
                return {{ found: true, id: id }};
                "###,
                escaped
            )
        };

        let req = ScannerAction::Execute(ExecuteRequest {
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

/// Validate that an element satisfies a requirement.
///
/// This is pub(crate) so it can be used by the association module.
pub(crate) fn validate_requirement(
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
