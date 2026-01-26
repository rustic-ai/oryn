use super::{ResolutionContext, TargetRequirement};
use super::requirement::ContainerType;

/// Check if an element satisfies a requirement.
pub fn validate_requirement(
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
