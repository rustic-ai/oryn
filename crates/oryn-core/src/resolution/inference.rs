use super::{ContainerType, ResolutionContext, TargetRequirement, is_inside};
use crate::command::Target;
use crate::protocol::Element;

/// An inference rule that can produce a target from context.
pub struct InferenceRule {
    pub name: &'static str,
    pub requirement: TargetRequirement,
    pub priority: i32,
    pub infer: fn(&ResolutionContext) -> Option<Target>,
}

/// Get inference rules for a requirement type.
pub fn get_inference_rules(requirement: &TargetRequirement) -> Vec<InferenceRule> {
    match requirement {
        TargetRequirement::Submittable => submittable_rules(),
        TargetRequirement::Container(ContainerType::Form) => form_container_rules(),
        TargetRequirement::Dismissable => dismissable_rules(),
        TargetRequirement::Acceptable => acceptable_rules(),
        _ => vec![],
    }
}

/// Convert an element ID to a Target.
fn id_to_target(id: u32) -> Target {
    Target::Id(id as usize)
}

/// Check if element has a specific attribute value.
fn attr_is(elem: &Element, key: &str, value: &str) -> bool {
    elem.attributes.get(key).is_some_and(|v| v == value)
}

/// Check if element is a submit button (button or input with type=submit).
fn is_submit_button(elem: &Element) -> bool {
    (elem.element_type == "button" || elem.element_type == "input")
        && attr_is(elem, "type", "submit")
}

/// Check if element is a modal/dialog.
fn is_modal(elem: &Element) -> bool {
    elem.element_type == "dialog"
        || elem
            .attributes
            .get("role")
            .is_some_and(|r| r == "dialog" || r == "alertdialog")
}

/// Check if element looks like a close button.
fn is_close_button(elem: &Element) -> bool {
    elem.attributes
        .get("aria-label")
        .is_some_and(|l| l.to_lowercase().contains("close"))
        || elem.text.as_ref().is_some_and(|t| {
            let lower = t.to_lowercase();
            lower == "x" || lower == "close"
        })
}

/// Check if element looks like an accept button.
fn is_accept_button(elem: &Element) -> bool {
    if elem.element_type != "button" {
        return false;
    }
    elem.text.as_ref().is_some_and(|t| {
        let lower = t.to_lowercase();
        lower == "accept" || lower.contains("allow all") || lower.contains("agree")
    })
}

/// Find the single element matching a predicate, or None if zero or multiple match.
fn find_single<'a>(
    ctx: &'a ResolutionContext,
    predicate: impl Fn(&Element) -> bool,
) -> Option<&'a Element> {
    let matches: Vec<_> = ctx.elements().filter(|e| predicate(e)).collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}

fn submittable_rules() -> Vec<InferenceRule> {
    vec![
        InferenceRule {
            name: "login_pattern_submit",
            requirement: TargetRequirement::Submittable,
            priority: 100,
            infer: |ctx| ctx.patterns()?.login.as_ref()?.submit.map(id_to_target),
        },
        InferenceRule {
            name: "search_pattern_submit",
            requirement: TargetRequirement::Submittable,
            priority: 95,
            infer: |ctx| ctx.patterns()?.search.as_ref()?.submit.map(id_to_target),
        },
        InferenceRule {
            name: "single_form_submit",
            requirement: TargetRequirement::Submittable,
            priority: 80,
            infer: |ctx| {
                let form = find_single(ctx, |e| e.element_type == "form")?;
                let scoped = ctx.scoped_to(form.id);
                scoped
                    .elements()
                    .find(|e| is_submit_button(e))
                    .map(|e| id_to_target(e.id))
            },
        },
        InferenceRule {
            name: "any_submit_button",
            requirement: TargetRequirement::Submittable,
            priority: 60,
            infer: |ctx| {
                ctx.elements()
                    .find(|e| is_submit_button(e))
                    .map(|e| id_to_target(e.id))
            },
        },
    ]
}

fn form_container_rules() -> Vec<InferenceRule> {
    vec![
        InferenceRule {
            name: "form_with_focus",
            requirement: TargetRequirement::Container(ContainerType::Form),
            priority: 100,
            infer: |ctx| {
                let focused_elem = ctx.get_element(ctx.focused()?)?;
                ctx.elements()
                    .find(|e| e.element_type == "form" && is_inside(&focused_elem.rect, &e.rect))
                    .map(|e| id_to_target(e.id))
            },
        },
        InferenceRule {
            name: "single_form",
            requirement: TargetRequirement::Container(ContainerType::Form),
            priority: 80,
            infer: |ctx| find_single(ctx, |e| e.element_type == "form").map(|e| id_to_target(e.id)),
        },
        InferenceRule {
            name: "login_form_pattern",
            requirement: TargetRequirement::Container(ContainerType::Form),
            priority: 90,
            infer: |ctx| {
                let login = ctx.patterns()?.login.as_ref()?;
                let password_elem = ctx.get_element(login.password)?;
                ctx.elements()
                    .find(|e| e.element_type == "form" && is_inside(&password_elem.rect, &e.rect))
                    .map(|e| id_to_target(e.id))
            },
        },
    ]
}

fn dismissable_rules() -> Vec<InferenceRule> {
    vec![
        InferenceRule {
            name: "modal_pattern_close",
            requirement: TargetRequirement::Dismissable,
            priority: 100,
            infer: |ctx| ctx.patterns()?.modal.as_ref()?.close.map(id_to_target),
        },
        InferenceRule {
            name: "cookie_banner_reject",
            requirement: TargetRequirement::Dismissable,
            priority: 95,
            infer: |ctx| {
                ctx.patterns()?
                    .cookie_banner
                    .as_ref()?
                    .reject
                    .map(id_to_target)
            },
        },
        InferenceRule {
            name: "any_modal_close",
            requirement: TargetRequirement::Dismissable,
            priority: 80,
            infer: |ctx| {
                let modal = find_single(ctx, is_modal)?;
                let scoped = ctx.scoped_to(modal.id);
                scoped
                    .elements()
                    .find(|e| is_close_button(e))
                    .map(|e| id_to_target(e.id))
            },
        },
    ]
}

fn acceptable_rules() -> Vec<InferenceRule> {
    vec![
        InferenceRule {
            name: "cookie_banner_accept",
            requirement: TargetRequirement::Acceptable,
            priority: 100,
            infer: |ctx| {
                ctx.patterns()?
                    .cookie_banner
                    .as_ref()?
                    .accept
                    .map(id_to_target)
            },
        },
        InferenceRule {
            name: "modal_pattern_confirm",
            requirement: TargetRequirement::Acceptable,
            priority: 95,
            infer: |ctx| ctx.patterns()?.modal.as_ref()?.confirm.map(id_to_target),
        },
        InferenceRule {
            name: "any_accept_button",
            requirement: TargetRequirement::Acceptable,
            priority: 60,
            infer: |ctx| {
                ctx.elements()
                    .find(|e| is_accept_button(e))
                    .map(|e| id_to_target(e.id))
            },
        },
    ]
}
