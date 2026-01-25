//! Label-to-control association strategies for actionable element resolution.
//!
//! When users write commands like `type "Email" "test@example.com"`, the resolver may match
//! the `<label>Email</label>` element instead of the associated `<input>`. This module provides
//! strategies to find the associated form control when the resolved element doesn't satisfy
//! the command's requirement.

use crate::resolution::context::{ResolutionContext, is_inside};
use crate::resolution::engine::validate_requirement;
use crate::resolution::requirement::TargetRequirement;
use oryn_common::protocol::Rect;

/// Result of attempting to find an associated control.
#[derive(Debug)]
pub enum AssociationResult {
    /// Found an associated control with the given element ID.
    Found(u32),
    /// No association was found.
    NoAssociation,
}

/// Configuration for association lookup.
const MAX_ADJACENT_ELEMENTS: usize = 5;
const MAX_VERTICAL_GAP_PX: f32 = 50.0;

/// Attempt to find an associated form control for a label-like element.
///
/// This is used when an element is resolved but doesn't satisfy the command's requirement
/// (e.g., a label was matched but a `type` command needs an input).
///
/// Association strategies (in priority order):
/// 1. `for` attribute: `<label for="x">` → find element with `id="x"`
/// 2. Nested control: `<label>Text <input></label>` → find input inside label bounds
/// 3. Adjacent control: `<label>Text</label><input>` → find next sibling input
pub fn find_associated_control(
    elem_id: u32,
    requirement: &TargetRequirement,
    ctx: &ResolutionContext,
) -> AssociationResult {
    let Some(elem) = ctx.get_element(elem_id) else {
        return AssociationResult::NoAssociation;
    };

    // Only attempt for label-like elements
    if !is_label_like(&elem.element_type) {
        return AssociationResult::NoAssociation;
    }

    // Strategy 1: Check for `for` attribute
    if let Some(control_id) = find_by_for_attribute(elem_id, requirement, ctx) {
        return AssociationResult::Found(control_id);
    }

    // Strategy 2: Check for nested control (bounding box containment)
    if let Some(control_id) = find_nested_control(elem_id, &elem.rect, requirement, ctx) {
        return AssociationResult::Found(control_id);
    }

    // Strategy 3: Check for adjacent control (DOM order + proximity)
    if let Some(control_id) = find_adjacent_control(elem_id, &elem.rect, requirement, ctx) {
        return AssociationResult::Found(control_id);
    }

    AssociationResult::NoAssociation
}

/// Check if an element type is label-like (non-actionable text container).
fn is_label_like(element_type: &str) -> bool {
    matches!(
        element_type,
        "label" | "span" | "p" | "strong" | "b" | "em" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
    )
}

/// Find associated control via the `for` attribute.
///
/// `<label for="email">Email</label>` → find element with `id="email"`
fn find_by_for_attribute(
    elem_id: u32,
    requirement: &TargetRequirement,
    ctx: &ResolutionContext,
) -> Option<u32> {
    let elem = ctx.get_element(elem_id)?;

    // Only labels have `for` attribute
    if elem.element_type != "label" {
        return None;
    }

    // Get the `for` attribute value
    let for_id = elem.attributes.get("for")?;

    // Find element with matching id
    ctx.elements()
        .find(|e| {
            e.attributes.get("id").is_some_and(|id| id == for_id)
                && validate_requirement(e.id, requirement, ctx)
        })
        .map(|e| e.id)
}

/// Find a nested control within the label's bounding box.
///
/// `<label>Email <input></label>` → find the input inside the label
fn find_nested_control(
    label_id: u32,
    label_rect: &Rect,
    requirement: &TargetRequirement,
    ctx: &ResolutionContext,
) -> Option<u32> {
    // Find elements that:
    // 1. Are inside the label's bounding box
    // 2. Satisfy the requirement
    // Note: We don't filter by ID order because nested inputs may be scanned before
    // the label text in DOM traversal order
    ctx.elements()
        .filter(|e| {
            e.id != label_id
                && is_inside(&e.rect, label_rect)
                && validate_requirement(e.id, requirement, ctx)
        })
        .min_by_key(|e| e.id)
        .map(|e| e.id)
}

/// Find an adjacent control near the label.
///
/// `<label>Email</label><input>` → find the input near the label
/// Also handles `<input><label>Text</label>` patterns where input comes first
fn find_adjacent_control(
    label_id: u32,
    label_rect: &Rect,
    requirement: &TargetRequirement,
    ctx: &ResolutionContext,
) -> Option<u32> {
    // Get all elements that satisfy the requirement (excluding the label itself)
    let mut candidates: Vec<_> = ctx
        .elements()
        .filter(|e| e.id != label_id && validate_requirement(e.id, requirement, ctx))
        .collect();

    // Sort by proximity to the label (prefer same row, then closest)
    candidates.sort_by(|a, b| {
        let a_dist = calculate_proximity(label_rect, &a.rect);
        let b_dist = calculate_proximity(label_rect, &b.rect);
        a_dist
            .partial_cmp(&b_dist)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Check the first N candidates for proximity
    for candidate in candidates.iter().take(MAX_ADJACENT_ELEMENTS) {
        // Check vertical proximity: element should be roughly on the same line
        // or immediately above/below the label
        let label_bottom = label_rect.y + label_rect.height;
        let label_top = label_rect.y;
        let elem_bottom = candidate.rect.y + candidate.rect.height;

        // Same row: overlapping vertical ranges or close together
        let is_same_row = (candidate.rect.y <= label_bottom + MAX_VERTICAL_GAP_PX)
            && (elem_bottom >= label_top - MAX_VERTICAL_GAP_PX);

        // Below the label
        let is_below = candidate.rect.y >= label_bottom
            && candidate.rect.y <= label_bottom + MAX_VERTICAL_GAP_PX;

        // Above the label (for patterns like checkbox before label)
        let is_above = elem_bottom <= label_top && elem_bottom >= label_top - MAX_VERTICAL_GAP_PX;

        if is_same_row || is_below || is_above {
            return Some(candidate.id);
        }
    }

    None
}

/// Calculate proximity score between a label and potential control.
/// Lower score means closer/better match.
fn calculate_proximity(label_rect: &Rect, elem_rect: &Rect) -> f32 {
    let label_center_y = label_rect.y + label_rect.height / 2.0;
    let elem_center_y = elem_rect.y + elem_rect.height / 2.0;
    let label_center_x = label_rect.x + label_rect.width / 2.0;
    let elem_center_x = elem_rect.x + elem_rect.width / 2.0;

    // Vertical distance is weighted more heavily
    let dy = (label_center_y - elem_center_y).abs();
    let dx = (label_center_x - elem_center_x).abs();

    // Elements on the same row get priority
    dy * 2.0 + dx
}

/// Check if an element is a label that can trigger an action on its associated control.
///
/// This handles cases where clicking/checking a label element will trigger the browser's
/// native behavior to focus or toggle the associated form control.
/// Also handles elements that are INSIDE a clickable label.
pub fn is_actionable_label(elem_id: u32, ctx: &ResolutionContext) -> bool {
    let Some(elem) = ctx.get_element(elem_id) else {
        return false;
    };

    // Check if it's a <label> element
    if elem.element_type == "label" {
        // Check if it has a `for` attribute pointing to an existing element
        if elem.attributes.get("for").is_some_and(|for_id| {
            ctx.elements()
                .any(|e| e.attributes.get("id").is_some_and(|id| id == for_id))
        }) {
            return true;
        }

        // Check if it contains a form control (nested input pattern)
        let label_rect = &elem.rect;
        if ctx.elements().any(|e| {
            e.id != elem_id
                && is_inside(&e.rect, label_rect)
                && matches!(e.element_type.as_str(), "input" | "select" | "textarea")
        }) {
            return true;
        }
    }

    // Check if this element is INSIDE a label that contains a form control
    // (e.g., <label><strong>Text</strong><input></label>)
    let elem_rect = &elem.rect;
    if ctx.elements().any(|label| {
        label.element_type == "label"
            && label.id != elem_id
            && is_inside(elem_rect, &label.rect)
            && ctx.elements().any(|e| {
                e.id != elem_id
                    && e.id != label.id
                    && is_inside(&e.rect, &label.rect)
                    && matches!(e.element_type.as_str(), "input" | "select" | "textarea")
            })
    }) {
        return true;
    }

    // Check if this element is INSIDE a parent with role="checkbox" or role="radio"
    // (e.g., <div role="checkbox"><span>☐</span> Email notifications</div>)
    ctx.elements().any(|parent| {
        parent.id != elem_id
            && is_inside(elem_rect, &parent.rect)
            && parent
                .attributes
                .get("role")
                .is_some_and(|r| matches!(r.as_str(), "checkbox" | "radio" | "switch" | "button"))
    })
}
