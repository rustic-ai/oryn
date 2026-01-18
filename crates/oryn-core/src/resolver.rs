//! Semantic Target Resolution
//!
//! This module bridges the gap between semantic targets (text, role, relational)
//! and numeric element IDs required by the scanner protocol.
//!
//! The parser creates rich semantic targets like `Target::Text("Sign In")` or
//! `Target::Role("email")`, but the scanner only accepts numeric IDs. The resolver
//! uses the latest scan result to map semantic targets to concrete element IDs.

use crate::command::Target;
use crate::protocol::{Element, ScanResult};
use thiserror::Error;

/// Errors that can occur during target resolution.
#[derive(Error, Debug, Clone)]
pub enum ResolverError {
    #[error("No element matches target: {0}")]
    NoMatch(String),

    #[error("Ambiguous target '{target}' matches {count} elements: {candidates:?}")]
    AmbiguousMatch {
        target: String,
        count: usize,
        candidates: Vec<u32>,
    },

    #[error("Stale context: scan result is too old or missing")]
    StaleContext,

    #[error("Relational resolution failed: {0}")]
    RelationalError(String),
}

/// Strategy for handling multiple matches.
#[derive(Debug, Clone, Copy, Default)]
pub enum ResolutionStrategy {
    /// Return the first match (by position/visibility).
    #[default]
    First,
    /// Error if multiple elements match.
    Unique,
    /// Return the best match by scoring.
    Best,
}

/// Context for resolving semantic targets.
///
/// Built from a `ScanResult` and used to resolve targets until the next scan.
#[derive(Debug, Clone)]
pub struct ResolverContext {
    elements: Vec<Element>,
    url: String,
}

impl ResolverContext {
    /// Create a new resolver context from a scan result.
    pub fn new(scan_result: &ScanResult) -> Self {
        Self {
            elements: scan_result.elements.clone(),
            url: scan_result.page.url.clone(),
        }
    }

    /// Create an empty context (for testing or when no scan has been performed).
    pub fn empty() -> Self {
        Self {
            elements: vec![],
            url: String::new(),
        }
    }

    /// Check if the context has any elements.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Get the number of elements in the context.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Get an element by ID.
    pub fn get_element(&self, id: u32) -> Option<&Element> {
        self.elements.iter().find(|e| e.id == id)
    }

    /// Get all elements.
    pub fn elements(&self) -> &[Element] {
        &self.elements
    }

    /// Get the URL context.
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// Resolve a semantic target to a concrete `Target::Id`.
///
/// This is the main entry point for target resolution. It recursively resolves
/// relational targets and returns a `Target::Id` if successful.
pub fn resolve_target(
    target: &Target,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<Target, ResolverError> {
    match target {
        // Already resolved - pass through
        Target::Id(id) => {
            // Verify the ID exists in context
            if ctx.get_element(*id as u32).is_some() {
                Ok(Target::Id(*id))
            } else {
                Err(ResolverError::NoMatch(format!(
                    "Element ID {} not found",
                    id
                )))
            }
        }

        // Text matching: element.text, label, placeholder, value
        Target::Text(text) => resolve_by_text(text, ctx, strategy),

        // Role matching: element.role
        Target::Role(role) => resolve_by_role(role, ctx, strategy),

        // CSS/XPath selector: pass through (scanner handles it)
        Target::Selector(_) => Ok(target.clone()),

        // Relational: resolve recursively
        Target::Near { target, anchor } => resolve_near(target, anchor, ctx, strategy),
        Target::Inside { target, container } => resolve_inside(target, container, ctx, strategy),
        Target::After { target, anchor } => resolve_after(target, anchor, ctx, strategy),
        Target::Before { target, anchor } => resolve_before(target, anchor, ctx, strategy),
        Target::Contains { target, content } => resolve_contains(target, content, ctx, strategy),
    }
}

/// Resolve a text-based target.
fn resolve_by_text(
    text: &str,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<Target, ResolverError> {
    let normalized = normalize_text(text);
    let mut matches: Vec<(u32, i32)> = vec![]; // (id, score)

    for elem in ctx.elements() {
        let mut score = 0;

        // Exact text match (highest priority)
        if let Some(ref elem_text) = elem.text {
            if normalize_text(elem_text) == normalized {
                score = 100;
            } else if normalize_text(elem_text).contains(&normalized) {
                score = 50;
            }
        }

        // Label match
        if let Some(ref label) = elem.label {
            if normalize_text(label) == normalized {
                score = score.max(90);
            } else if normalize_text(label).contains(&normalized) {
                score = score.max(45);
            }
        }

        // Placeholder match
        if let Some(ref placeholder) = elem.placeholder {
            if normalize_text(placeholder) == normalized {
                score = score.max(80);
            } else if normalize_text(placeholder).contains(&normalized) {
                score = score.max(40);
            }
        }

        // Value match (for inputs)
        if let Some(ref value) = elem.value
            && normalize_text(value) == normalized
        {
            score = score.max(70);
        }

        // aria-label attribute
        if let Some(aria_label) = elem.attributes.get("aria-label") {
            if normalize_text(aria_label) == normalized {
                score = score.max(85);
            } else if normalize_text(aria_label).contains(&normalized) {
                score = score.max(42);
            }
        }

        // title attribute
        if let Some(title) = elem.attributes.get("title")
            && normalize_text(title) == normalized
        {
            score = score.max(75);
        }

        if score > 0 {
            matches.push((elem.id, score));
        }
    }

    select_match(&matches, text, strategy)
}

/// Resolve a role-based target.
fn resolve_by_role(
    role: &str,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<Target, ResolverError> {
    let normalized_role = role.to_lowercase();
    let mut matches: Vec<(u32, i32)> = vec![];

    for elem in ctx.elements() {
        let mut score = 0;

        // Direct role match
        if let Some(ref elem_role) = elem.role
            && elem_role.to_lowercase() == normalized_role
        {
            score = 100;
        }

        // Element type match (button, input, etc.)
        if elem.element_type.to_lowercase() == normalized_role {
            score = score.max(80);
        }

        // Input type matching (email, password, etc.)
        if let Some(input_type) = elem.attributes.get("type")
            && input_type.to_lowercase() == normalized_role
        {
            score = score.max(90);
        }

        // Autocomplete attribute matching
        if let Some(autocomplete) = elem.attributes.get("autocomplete")
            && autocomplete.to_lowercase() == normalized_role
        {
            score = score.max(85);
        }

        // ARIA role
        if let Some(aria_role) = elem.attributes.get("role")
            && aria_role.to_lowercase() == normalized_role
        {
            score = score.max(95);
        }

        // Special role mappings
        if normalized_role == "submit"
            && (elem.element_type == "button"
                || elem
                    .attributes
                    .get("type")
                    .map(|t| t == "submit")
                    .unwrap_or(false))
        {
            score = score.max(85);
        }

        // Penalize disabled elements
        if elem.state.disabled && score > 0 {
            score -= 20;
        }

        if score > 0 {
            matches.push((elem.id, score));
        }
    }

    select_match(&matches, role, strategy)
}

/// Resolve `target near anchor`.
fn resolve_near(
    target: &Target,
    anchor: &Target,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<Target, ResolverError> {
    // First resolve the anchor
    let anchor_resolved = resolve_target(anchor, ctx, ResolutionStrategy::First)?;
    let anchor_id = match anchor_resolved {
        Target::Id(id) => id,
        _ => {
            return Err(ResolverError::RelationalError(
                "Anchor must resolve to ID".into(),
            ));
        }
    };

    let anchor_elem = ctx
        .get_element(anchor_id as u32)
        .ok_or_else(|| ResolverError::NoMatch(format!("Anchor element {} not found", anchor_id)))?;

    // Get candidates matching the target pattern
    let candidates = get_matching_candidates(target, ctx)?;

    if candidates.is_empty() {
        return Err(ResolverError::NoMatch(format!(
            "{:?} near {:?}",
            target, anchor
        )));
    }

    // Score by distance from anchor
    let anchor_center = (
        anchor_elem.rect.x + anchor_elem.rect.width / 2.0,
        anchor_elem.rect.y + anchor_elem.rect.height / 2.0,
    );

    let mut scored: Vec<(u32, i32)> = candidates
        .iter()
        .filter_map(|&id| {
            ctx.get_element(id).map(|elem| {
                let center = (
                    elem.rect.x + elem.rect.width / 2.0,
                    elem.rect.y + elem.rect.height / 2.0,
                );
                let distance = ((center.0 - anchor_center.0).powi(2)
                    + (center.1 - anchor_center.1).powi(2))
                .sqrt();
                // Invert distance to score (closer = higher)
                let score = (10000.0 / (distance + 1.0)) as i32;
                (id, score)
            })
        })
        .collect();

    scored.sort_by(|a, b| b.1.cmp(&a.1));

    select_match(
        &scored,
        &format!("{:?} near {:?}", target, anchor),
        strategy,
    )
}

/// Resolve `target inside container`.
fn resolve_inside(
    target: &Target,
    container: &Target,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<Target, ResolverError> {
    // Resolve container first
    let container_resolved = resolve_target(container, ctx, ResolutionStrategy::First)?;
    let container_id = match container_resolved {
        Target::Id(id) => id,
        _ => {
            return Err(ResolverError::RelationalError(
                "Container must resolve to ID".into(),
            ));
        }
    };

    let container_elem = ctx.get_element(container_id as u32).ok_or_else(|| {
        ResolverError::NoMatch(format!("Container element {} not found", container_id))
    })?;

    // Get candidates matching target
    let candidates = get_matching_candidates(target, ctx)?;

    // Filter to those inside container (by bounding box)
    let inside: Vec<(u32, i32)> = candidates
        .iter()
        .filter_map(|&id| {
            ctx.get_element(id).and_then(|elem| {
                if is_inside(&elem.rect, &container_elem.rect) {
                    Some((id, 100))
                } else {
                    None
                }
            })
        })
        .collect();

    if inside.is_empty() {
        return Err(ResolverError::NoMatch(format!(
            "{:?} inside {:?}",
            target, container
        )));
    }

    select_match(
        &inside,
        &format!("{:?} inside {:?}", target, container),
        strategy,
    )
}

/// Resolve `target after anchor`.
fn resolve_after(
    target: &Target,
    anchor: &Target,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<Target, ResolverError> {
    let anchor_resolved = resolve_target(anchor, ctx, ResolutionStrategy::First)?;
    let anchor_id = match anchor_resolved {
        Target::Id(id) => id,
        _ => {
            return Err(ResolverError::RelationalError(
                "Anchor must resolve to ID".into(),
            ));
        }
    };

    let anchor_elem = ctx
        .get_element(anchor_id as u32)
        .ok_or_else(|| ResolverError::NoMatch(format!("Anchor element {} not found", anchor_id)))?;

    let candidates = get_matching_candidates(target, ctx)?;

    // Elements that come after (below or to the right)
    let after: Vec<(u32, i32)> = candidates
        .iter()
        .filter_map(|&id| {
            ctx.get_element(id).and_then(|elem| {
                // After = starts at or below anchor's bottom, or to the right on same row
                let anchor_bottom = anchor_elem.rect.y + anchor_elem.rect.height;
                let is_after = elem.rect.y >= anchor_bottom
                    || (elem.rect.y >= anchor_elem.rect.y
                        && elem.rect.x > anchor_elem.rect.x + anchor_elem.rect.width);
                if is_after {
                    // Score by proximity (closer = better)
                    let dist = (elem.rect.y - anchor_bottom).abs()
                        + (elem.rect.x - anchor_elem.rect.x).abs();
                    Some((id, (10000.0 / (dist + 1.0)) as i32))
                } else {
                    None
                }
            })
        })
        .collect();

    if after.is_empty() {
        return Err(ResolverError::NoMatch(format!(
            "{:?} after {:?}",
            target, anchor
        )));
    }

    select_match(
        &after,
        &format!("{:?} after {:?}", target, anchor),
        strategy,
    )
}

/// Resolve `target before anchor`.
fn resolve_before(
    target: &Target,
    anchor: &Target,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<Target, ResolverError> {
    let anchor_resolved = resolve_target(anchor, ctx, ResolutionStrategy::First)?;
    let anchor_id = match anchor_resolved {
        Target::Id(id) => id,
        _ => {
            return Err(ResolverError::RelationalError(
                "Anchor must resolve to ID".into(),
            ));
        }
    };

    let anchor_elem = ctx
        .get_element(anchor_id as u32)
        .ok_or_else(|| ResolverError::NoMatch(format!("Anchor element {} not found", anchor_id)))?;

    let candidates = get_matching_candidates(target, ctx)?;

    // Elements that come before (above or to the left)
    let before: Vec<(u32, i32)> = candidates
        .iter()
        .filter_map(|&id| {
            ctx.get_element(id).and_then(|elem| {
                let elem_bottom = elem.rect.y + elem.rect.height;
                let is_before = elem_bottom <= anchor_elem.rect.y
                    || (elem.rect.y <= anchor_elem.rect.y + anchor_elem.rect.height
                        && elem.rect.x + elem.rect.width < anchor_elem.rect.x);
                if is_before {
                    let dist = (anchor_elem.rect.y - elem_bottom).abs()
                        + (anchor_elem.rect.x - elem.rect.x).abs();
                    Some((id, (10000.0 / (dist + 1.0)) as i32))
                } else {
                    None
                }
            })
        })
        .collect();

    if before.is_empty() {
        return Err(ResolverError::NoMatch(format!(
            "{:?} before {:?}",
            target, anchor
        )));
    }

    select_match(
        &before,
        &format!("{:?} before {:?}", target, anchor),
        strategy,
    )
}

/// Resolve `target contains content`.
fn resolve_contains(
    target: &Target,
    content: &Target,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<Target, ResolverError> {
    // Resolve content first
    let content_resolved = resolve_target(content, ctx, ResolutionStrategy::First)?;
    let content_id = match content_resolved {
        Target::Id(id) => id,
        _ => {
            return Err(ResolverError::RelationalError(
                "Content must resolve to ID".into(),
            ));
        }
    };

    let content_elem = ctx.get_element(content_id as u32).ok_or_else(|| {
        ResolverError::NoMatch(format!("Content element {} not found", content_id))
    })?;

    let candidates = get_matching_candidates(target, ctx)?;

    // Find targets that contain the content element
    let containing: Vec<(u32, i32)> = candidates
        .iter()
        .filter_map(|&id| {
            ctx.get_element(id).and_then(|elem| {
                if is_inside(&content_elem.rect, &elem.rect) {
                    Some((id, 100))
                } else {
                    None
                }
            })
        })
        .collect();

    if containing.is_empty() {
        return Err(ResolverError::NoMatch(format!(
            "{:?} contains {:?}",
            target, content
        )));
    }

    select_match(
        &containing,
        &format!("{:?} contains {:?}", target, content),
        strategy,
    )
}

// Helper functions

/// Normalize text for comparison (lowercase, trim, collapse whitespace).
fn normalize_text(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Check if inner rect is inside outer rect.
fn is_inside(inner: &crate::protocol::Rect, outer: &crate::protocol::Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.width <= outer.x + outer.width
        && inner.y + inner.height <= outer.y + outer.height
}

/// Get element IDs matching a target pattern (without full resolution).
fn get_matching_candidates(
    target: &Target,
    ctx: &ResolverContext,
) -> Result<Vec<u32>, ResolverError> {
    match target {
        Target::Id(id) => Ok(vec![*id as u32]),
        Target::Text(text) => {
            let normalized = normalize_text(text);
            Ok(ctx
                .elements()
                .iter()
                .filter(|e| {
                    e.text
                        .as_ref()
                        .map(|t| normalize_text(t).contains(&normalized))
                        .unwrap_or(false)
                        || e.label
                            .as_ref()
                            .map(|l| normalize_text(l).contains(&normalized))
                            .unwrap_or(false)
                        || e.placeholder
                            .as_ref()
                            .map(|p| normalize_text(p).contains(&normalized))
                            .unwrap_or(false)
                })
                .map(|e| e.id)
                .collect())
        }
        Target::Role(role) => {
            let normalized = role.to_lowercase();
            Ok(ctx
                .elements()
                .iter()
                .filter(|e| {
                    e.role
                        .as_ref()
                        .map(|r| r.to_lowercase() == normalized)
                        .unwrap_or(false)
                        || e.element_type.to_lowercase() == normalized
                        || e.attributes
                            .get("type")
                            .map(|t| t.to_lowercase() == normalized)
                            .unwrap_or(false)
                })
                .map(|e| e.id)
                .collect())
        }
        Target::Selector(_) => {
            // Can't resolve selector without browser - return all elements
            Ok(ctx.elements().iter().map(|e| e.id).collect())
        }
        // For relational targets, get candidates from the inner target
        Target::Near { target, .. }
        | Target::Inside { target, .. }
        | Target::After { target, .. }
        | Target::Before { target, .. }
        | Target::Contains { target, .. } => get_matching_candidates(target, ctx),
    }
}

/// Select the best match based on strategy.
fn select_match(
    matches: &[(u32, i32)],
    target_desc: &str,
    strategy: ResolutionStrategy,
) -> Result<Target, ResolverError> {
    if matches.is_empty() {
        return Err(ResolverError::NoMatch(target_desc.to_string()));
    }

    // Sort by score descending
    let mut sorted = matches.to_vec();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    match strategy {
        ResolutionStrategy::First | ResolutionStrategy::Best => {
            Ok(Target::Id(sorted[0].0 as usize))
        }
        ResolutionStrategy::Unique => {
            if sorted.len() > 1 && sorted[0].1 == sorted[1].1 {
                // Top scores are tied - ambiguous
                let candidates: Vec<u32> = sorted
                    .iter()
                    .filter(|m| m.1 == sorted[0].1)
                    .map(|m| m.0)
                    .collect();
                Err(ResolverError::AmbiguousMatch {
                    target: target_desc.to_string(),
                    count: candidates.len(),
                    candidates,
                })
            } else {
                Ok(Target::Id(sorted[0].0 as usize))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        Element, ElementState, PageInfo, Rect, ScanResult, ScanStats, ScrollInfo, ViewportInfo,
    };
    use std::collections::HashMap;

    fn make_element(
        id: u32,
        text: Option<&str>,
        role: Option<&str>,
        element_type: &str,
    ) -> Element {
        Element {
            id,
            element_type: element_type.to_string(),
            role: role.map(|s| s.to_string()),
            text: text.map(|s| s.to_string()),
            label: None,
            value: None,
            placeholder: None,
            selector: format!("#elem-{}", id),
            xpath: None,
            rect: Rect {
                x: (id * 100) as f32,
                y: (id * 50) as f32,
                width: 100.0,
                height: 30.0,
            },
            attributes: HashMap::new(),
            state: ElementState::default(),
            children: vec![],
        }
    }

    fn make_context(elements: Vec<Element>) -> ResolverContext {
        let scan_result = ScanResult {
            page: PageInfo {
                url: "https://example.com".to_string(),
                title: "Test".to_string(),
                viewport: ViewportInfo::default(),
                scroll: ScrollInfo::default(),
            },
            elements,
            stats: ScanStats {
                total: 3,
                scanned: 3,
            },
            patterns: None,
            changes: None,
            available_intents: None,
        };
        ResolverContext::new(&scan_result)
    }

    #[test]
    fn test_resolve_id_passthrough() {
        let ctx = make_context(vec![make_element(1, Some("Button"), None, "button")]);
        let result = resolve_target(&Target::Id(1), &ctx, ResolutionStrategy::First);
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_id_not_found() {
        let ctx = make_context(vec![make_element(1, Some("Button"), None, "button")]);
        let result = resolve_target(&Target::Id(99), &ctx, ResolutionStrategy::First);
        assert!(matches!(result, Err(ResolverError::NoMatch(_))));
    }

    #[test]
    fn test_resolve_text_exact() {
        let ctx = make_context(vec![
            make_element(1, Some("Sign In"), None, "button"),
            make_element(2, Some("Sign Up"), None, "button"),
        ]);
        let result = resolve_target(
            &Target::Text("Sign In".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_text_case_insensitive() {
        let ctx = make_context(vec![make_element(1, Some("SIGN IN"), None, "button")]);
        let result = resolve_target(
            &Target::Text("sign in".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_role() {
        let mut elem = make_element(1, None, Some("textbox"), "input");
        elem.attributes
            .insert("type".to_string(), "email".to_string());
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Role("email".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_no_match() {
        let ctx = make_context(vec![make_element(1, Some("Login"), None, "button")]);
        let result = resolve_target(
            &Target::Text("Nonexistent".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Err(ResolverError::NoMatch(_))));
    }

    #[test]
    fn test_selector_passthrough() {
        let ctx = make_context(vec![]);
        let result = resolve_target(
            &Target::Selector("#my-id".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Selector(_))));
    }

    // ============================================================
    // Helper for positioned elements (relational tests)
    // ============================================================

    #[allow(clippy::too_many_arguments)]
    fn make_element_at(
        id: u32,
        text: Option<&str>,
        role: Option<&str>,
        element_type: &str,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Element {
        Element {
            id,
            element_type: element_type.to_string(),
            role: role.map(|s| s.to_string()),
            text: text.map(|s| s.to_string()),
            label: None,
            value: None,
            placeholder: None,
            selector: format!("#elem-{}", id),
            xpath: None,
            rect: Rect {
                x,
                y,
                width,
                height,
            },
            attributes: HashMap::new(),
            state: ElementState::default(),
            children: vec![],
        }
    }

    // ============================================================
    // Text Resolution Field Tests
    // ============================================================

    #[test]
    fn test_resolve_text_by_label() {
        let mut elem = make_element(1, None, None, "input");
        elem.label = Some("Email Address".to_string());
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Text("Email Address".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_text_by_label_contains() {
        let mut elem = make_element(1, None, None, "input");
        elem.label = Some("Enter your email address here".to_string());
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Text("email".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_text_by_placeholder() {
        let mut elem = make_element(1, None, None, "input");
        elem.placeholder = Some("Enter password".to_string());
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Text("Enter password".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_text_by_placeholder_contains() {
        let mut elem = make_element(1, None, None, "input");
        elem.placeholder = Some("Search for products...".to_string());
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Text("products".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_text_by_value() {
        let mut elem = make_element(1, None, None, "input");
        elem.value = Some("john@example.com".to_string());
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Text("john@example.com".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_text_by_aria_label() {
        let mut elem = make_element(1, None, None, "button");
        elem.attributes
            .insert("aria-label".to_string(), "Close dialog".to_string());
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Text("Close dialog".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_text_by_aria_label_contains() {
        let mut elem = make_element(1, None, None, "button");
        elem.attributes.insert(
            "aria-label".to_string(),
            "Close this dialog window".to_string(),
        );
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Text("dialog".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_text_by_title() {
        let mut elem = make_element(1, None, None, "a");
        elem.attributes
            .insert("title".to_string(), "Visit our homepage".to_string());
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Text("Visit our homepage".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_text_scoring_prefers_exact_over_contains() {
        // Element 1 has "Sign" in text (contains match)
        // Element 2 has exact "Sign In" text (exact match)
        let elem1 = make_element(1, Some("Sign Up Now"), None, "button");
        let elem2 = make_element(2, Some("Sign In"), None, "button");
        let ctx = make_context(vec![elem1, elem2]);
        let result = resolve_target(
            &Target::Text("Sign In".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        // Should prefer exact match (element 2)
        assert!(matches!(result, Ok(Target::Id(2))));
    }

    // ============================================================
    // Role Resolution Tests
    // ============================================================

    #[test]
    fn test_resolve_role_by_aria_role() {
        let mut elem = make_element(1, None, None, "div");
        elem.attributes
            .insert("role".to_string(), "button".to_string());
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Role("button".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_role_by_element_type() {
        let elem = make_element(1, Some("Click me"), None, "button");
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Role("button".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_role_by_autocomplete() {
        let mut elem = make_element(1, None, None, "input");
        elem.attributes
            .insert("autocomplete".to_string(), "email".to_string());
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Role("email".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_role_submit_button() {
        let mut elem = make_element(1, Some("Submit"), None, "button");
        elem.attributes
            .insert("type".to_string(), "submit".to_string());
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Role("submit".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_role_disabled_penalty() {
        // Element 1 is disabled, Element 2 is not
        let mut elem1 = make_element(1, None, Some("textbox"), "input");
        elem1
            .attributes
            .insert("type".to_string(), "email".to_string());
        elem1.state.disabled = true;

        let mut elem2 = make_element(2, None, Some("textbox"), "input");
        elem2
            .attributes
            .insert("type".to_string(), "email".to_string());
        elem2.state.disabled = false;

        let ctx = make_context(vec![elem1, elem2]);
        let result = resolve_target(
            &Target::Role("email".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        // Should prefer enabled element (element 2) due to disabled penalty
        assert!(matches!(result, Ok(Target::Id(2))));
    }

    #[test]
    fn test_resolve_role_case_insensitive() {
        let elem = make_element(1, Some("Click"), None, "button");
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Role("BUTTON".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    // ============================================================
    // Resolution Strategy Tests
    // ============================================================

    #[test]
    fn test_strategy_unique_success() {
        let ctx = make_context(vec![
            make_element(1, Some("Submit"), None, "button"),
            make_element(2, Some("Cancel"), None, "button"),
        ]);
        let result = resolve_target(
            &Target::Text("Submit".into()),
            &ctx,
            ResolutionStrategy::Unique,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_strategy_unique_ambiguous_error() {
        // Both elements have same text - should error
        let ctx = make_context(vec![
            make_element(1, Some("Submit"), None, "button"),
            make_element(2, Some("Submit"), None, "button"),
        ]);
        let result = resolve_target(
            &Target::Text("Submit".into()),
            &ctx,
            ResolutionStrategy::Unique,
        );
        assert!(matches!(
            result,
            Err(ResolverError::AmbiguousMatch { count: 2, .. })
        ));
    }

    #[test]
    fn test_strategy_unique_different_scores_success() {
        // Element 1 has exact match (100 pts), Element 2 has contains match (50 pts)
        let ctx = make_context(vec![
            make_element(1, Some("Submit"), None, "button"),
            make_element(2, Some("Submit Form"), None, "button"),
        ]);
        let result = resolve_target(
            &Target::Text("Submit".into()),
            &ctx,
            ResolutionStrategy::Unique,
        );
        // Should succeed because scores are different
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_strategy_best_returns_highest_score() {
        let ctx = make_context(vec![
            make_element(1, Some("Click Submit button"), None, "button"), // contains: 50
            make_element(2, Some("Submit"), None, "button"),              // exact: 100
        ]);
        let result = resolve_target(
            &Target::Text("Submit".into()),
            &ctx,
            ResolutionStrategy::Best,
        );
        assert!(matches!(result, Ok(Target::Id(2))));
    }

    // ============================================================
    // Relational Resolution: resolve_near
    // ============================================================

    #[test]
    fn test_resolve_near_basic() {
        // Layout: [Anchor at 0,0] [Target at 150,0] [Farther at 300,0]
        let anchor = make_element_at(1, Some("Username"), None, "label", 0.0, 0.0, 100.0, 30.0);
        let target = make_element_at(2, None, None, "input", 150.0, 0.0, 100.0, 30.0);
        let farther = make_element_at(3, None, None, "input", 300.0, 0.0, 100.0, 30.0);

        let ctx = make_context(vec![anchor, target, farther]);
        let result = resolve_target(
            &Target::Near {
                target: Box::new(Target::Role("input".into())),
                anchor: Box::new(Target::Text("Username".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        // Should select element 2 (closest input to Username label)
        assert!(matches!(result, Ok(Target::Id(2))));
    }

    #[test]
    fn test_resolve_near_selects_closest() {
        // Layout: [Anchor] .... [Close target] .......... [Far target]
        let anchor = make_element_at(1, Some("Email"), None, "label", 0.0, 0.0, 80.0, 30.0);
        let close = make_element_at(2, None, None, "input", 100.0, 0.0, 100.0, 30.0);
        let far = make_element_at(3, None, None, "input", 500.0, 0.0, 100.0, 30.0);

        let ctx = make_context(vec![anchor, far, close]); // Order shouldn't matter
        let result = resolve_target(
            &Target::Near {
                target: Box::new(Target::Role("input".into())),
                anchor: Box::new(Target::Text("Email".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(2))));
    }

    #[test]
    fn test_resolve_near_anchor_not_found() {
        let target = make_element_at(1, None, None, "input", 100.0, 0.0, 100.0, 30.0);
        let ctx = make_context(vec![target]);
        let result = resolve_target(
            &Target::Near {
                target: Box::new(Target::Role("input".into())),
                anchor: Box::new(Target::Text("Nonexistent".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Err(ResolverError::NoMatch(_))));
    }

    #[test]
    fn test_resolve_near_target_not_found() {
        let anchor = make_element_at(1, Some("Label"), None, "label", 0.0, 0.0, 80.0, 30.0);
        let ctx = make_context(vec![anchor]);
        let result = resolve_target(
            &Target::Near {
                target: Box::new(Target::Role("input".into())),
                anchor: Box::new(Target::Text("Label".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Err(ResolverError::NoMatch(_))));
    }

    #[test]
    fn test_resolve_near_vertical_proximity() {
        // Anchor above, two inputs below at different distances
        let anchor = make_element_at(1, Some("Form"), None, "label", 100.0, 0.0, 80.0, 30.0);
        let close = make_element_at(2, None, None, "input", 100.0, 50.0, 100.0, 30.0);
        let far = make_element_at(3, None, None, "input", 100.0, 200.0, 100.0, 30.0);

        let ctx = make_context(vec![anchor, far, close]);
        let result = resolve_target(
            &Target::Near {
                target: Box::new(Target::Role("input".into())),
                anchor: Box::new(Target::Text("Form".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(2))));
    }

    // ============================================================
    // Relational Resolution: resolve_inside
    // ============================================================

    #[test]
    fn test_resolve_inside_basic() {
        // Container at (0,0) 500x500, button inside at (50,50) 100x30
        let container = make_element_at(1, Some("Form"), None, "form", 0.0, 0.0, 500.0, 500.0);
        let inside_btn =
            make_element_at(2, Some("Submit"), None, "button", 50.0, 50.0, 100.0, 30.0);
        let outside_btn =
            make_element_at(3, Some("Other"), None, "button", 600.0, 50.0, 100.0, 30.0);

        let ctx = make_context(vec![container, inside_btn, outside_btn]);
        let result = resolve_target(
            &Target::Inside {
                target: Box::new(Target::Role("button".into())),
                container: Box::new(Target::Text("Form".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(2))));
    }

    #[test]
    fn test_resolve_inside_partial_overlap_excluded() {
        // Container at (0,0) 200x200, button partially outside at (150,50) 100x30
        let container = make_element_at(1, Some("Box"), None, "div", 0.0, 0.0, 200.0, 200.0);
        let partial = make_element_at(2, Some("Button"), None, "button", 150.0, 50.0, 100.0, 30.0);

        let ctx = make_context(vec![container, partial]);
        let result = resolve_target(
            &Target::Inside {
                target: Box::new(Target::Role("button".into())),
                container: Box::new(Target::Text("Box".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        // Partial overlap should be excluded
        assert!(matches!(result, Err(ResolverError::NoMatch(_))));
    }

    #[test]
    fn test_resolve_inside_container_not_found() {
        let btn = make_element_at(1, Some("Submit"), None, "button", 50.0, 50.0, 100.0, 30.0);
        let ctx = make_context(vec![btn]);
        let result = resolve_target(
            &Target::Inside {
                target: Box::new(Target::Role("button".into())),
                container: Box::new(Target::Text("Nonexistent".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Err(ResolverError::NoMatch(_))));
    }

    #[test]
    fn test_resolve_inside_multiple_elements() {
        // Container with two buttons inside
        let container = make_element_at(1, Some("Form"), None, "form", 0.0, 0.0, 500.0, 500.0);
        let btn1 = make_element_at(2, Some("Submit"), None, "button", 50.0, 50.0, 100.0, 30.0);
        let btn2 = make_element_at(3, Some("Cancel"), None, "button", 50.0, 100.0, 100.0, 30.0);

        let ctx = make_context(vec![container, btn1, btn2]);
        let result = resolve_target(
            &Target::Inside {
                target: Box::new(Target::Text("Cancel".into())),
                container: Box::new(Target::Text("Form".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(3))));
    }

    // ============================================================
    // Relational Resolution: resolve_after
    // ============================================================

    #[test]
    fn test_resolve_after_below() {
        // Anchor at top, target below
        let anchor = make_element_at(1, Some("Name"), None, "label", 0.0, 0.0, 100.0, 30.0);
        let below = make_element_at(2, None, None, "input", 0.0, 50.0, 200.0, 30.0);

        let ctx = make_context(vec![anchor, below]);
        let result = resolve_target(
            &Target::After {
                target: Box::new(Target::Role("input".into())),
                anchor: Box::new(Target::Text("Name".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(2))));
    }

    #[test]
    fn test_resolve_after_right_same_row() {
        // Anchor on left, target to the right on same row
        let anchor = make_element_at(1, Some("Label"), None, "label", 0.0, 0.0, 80.0, 30.0);
        let right = make_element_at(2, None, None, "input", 100.0, 0.0, 150.0, 30.0);

        let ctx = make_context(vec![anchor, right]);
        let result = resolve_target(
            &Target::After {
                target: Box::new(Target::Role("input".into())),
                anchor: Box::new(Target::Text("Label".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(2))));
    }

    #[test]
    fn test_resolve_after_selects_closest() {
        let anchor = make_element_at(1, Some("Start"), None, "label", 0.0, 0.0, 80.0, 30.0);
        let close = make_element_at(2, None, None, "input", 0.0, 40.0, 100.0, 30.0);
        let far = make_element_at(3, None, None, "input", 0.0, 200.0, 100.0, 30.0);

        let ctx = make_context(vec![anchor, far, close]);
        let result = resolve_target(
            &Target::After {
                target: Box::new(Target::Role("input".into())),
                anchor: Box::new(Target::Text("Start".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(2))));
    }

    #[test]
    fn test_resolve_after_no_elements_after() {
        // Element is above/before anchor
        let anchor = make_element_at(1, Some("End"), None, "label", 0.0, 100.0, 80.0, 30.0);
        let before = make_element_at(2, None, None, "input", 0.0, 0.0, 100.0, 30.0);

        let ctx = make_context(vec![anchor, before]);
        let result = resolve_target(
            &Target::After {
                target: Box::new(Target::Role("input".into())),
                anchor: Box::new(Target::Text("End".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Err(ResolverError::NoMatch(_))));
    }

    // ============================================================
    // Relational Resolution: resolve_before
    // ============================================================

    #[test]
    fn test_resolve_before_above() {
        let above = make_element_at(1, Some("Title"), None, "label", 0.0, 0.0, 100.0, 30.0);
        let anchor = make_element_at(2, Some("Input"), None, "input", 0.0, 50.0, 200.0, 30.0);

        let ctx = make_context(vec![above, anchor]);
        let result = resolve_target(
            &Target::Before {
                target: Box::new(Target::Text("Title".into())),
                anchor: Box::new(Target::Role("input".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_before_left_same_row() {
        let left = make_element_at(1, Some("Label"), None, "label", 0.0, 0.0, 80.0, 30.0);
        let anchor = make_element_at(2, None, None, "input", 100.0, 0.0, 150.0, 30.0);

        let ctx = make_context(vec![left, anchor]);
        let result = resolve_target(
            &Target::Before {
                target: Box::new(Target::Text("Label".into())),
                anchor: Box::new(Target::Role("input".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_before_selects_closest() {
        let far = make_element_at(1, Some("Far"), None, "label", 0.0, 0.0, 80.0, 30.0);
        let close = make_element_at(2, Some("Close"), None, "label", 0.0, 150.0, 80.0, 30.0);
        let anchor = make_element_at(3, None, None, "input", 0.0, 200.0, 100.0, 30.0);

        let ctx = make_context(vec![far, close, anchor]);
        let result = resolve_target(
            &Target::Before {
                target: Box::new(Target::Role("label".into())),
                anchor: Box::new(Target::Role("input".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        // Should select closest label (element 2)
        assert!(matches!(result, Ok(Target::Id(2))));
    }

    #[test]
    fn test_resolve_before_no_elements_before() {
        let anchor = make_element_at(1, Some("First"), None, "label", 0.0, 0.0, 80.0, 30.0);
        let after = make_element_at(2, None, None, "input", 0.0, 100.0, 100.0, 30.0);

        let ctx = make_context(vec![anchor, after]);
        let result = resolve_target(
            &Target::Before {
                target: Box::new(Target::Role("input".into())),
                anchor: Box::new(Target::Text("First".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Err(ResolverError::NoMatch(_))));
    }

    // ============================================================
    // Relational Resolution: resolve_contains
    // ============================================================

    #[test]
    fn test_resolve_contains_basic() {
        // Outer div contains inner button
        let outer = make_element_at(1, Some("Card"), None, "div", 0.0, 0.0, 300.0, 200.0);
        let inner = make_element_at(2, Some("Action"), None, "button", 50.0, 50.0, 80.0, 30.0);

        let ctx = make_context(vec![outer, inner]);
        let result = resolve_target(
            &Target::Contains {
                target: Box::new(Target::Text("Card".into())),
                content: Box::new(Target::Text("Action".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }

    #[test]
    fn test_resolve_contains_content_not_found() {
        let outer = make_element_at(1, Some("Card"), None, "div", 0.0, 0.0, 300.0, 200.0);
        let ctx = make_context(vec![outer]);
        let result = resolve_target(
            &Target::Contains {
                target: Box::new(Target::Text("Card".into())),
                content: Box::new(Target::Text("Nonexistent".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Err(ResolverError::NoMatch(_))));
    }

    #[test]
    fn test_resolve_contains_no_container() {
        // Content exists but no target contains it
        let content = make_element_at(1, Some("Button"), None, "button", 50.0, 50.0, 80.0, 30.0);
        let non_container =
            make_element_at(2, Some("Other"), None, "div", 500.0, 500.0, 100.0, 100.0);

        let ctx = make_context(vec![content, non_container]);
        let result = resolve_target(
            &Target::Contains {
                target: Box::new(Target::Text("Other".into())),
                content: Box::new(Target::Text("Button".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Err(ResolverError::NoMatch(_))));
    }

    #[test]
    fn test_resolve_contains_multiple_containers() {
        // Nested containers: outer > middle > inner
        let outer = make_element_at(1, Some("Outer"), None, "div", 0.0, 0.0, 400.0, 400.0);
        let middle = make_element_at(2, Some("Middle"), None, "div", 50.0, 50.0, 300.0, 300.0);
        let inner = make_element_at(3, Some("Inner"), None, "button", 100.0, 100.0, 80.0, 30.0);

        let ctx = make_context(vec![outer, middle, inner]);
        // Ask for div that contains Inner - both outer and middle qualify
        let result = resolve_target(
            &Target::Contains {
                target: Box::new(Target::Role("div".into())),
                content: Box::new(Target::Text("Inner".into())),
            },
            &ctx,
            ResolutionStrategy::First,
        );
        // First match should be returned (element 1 in this case, sorted by score)
        assert!(matches!(result, Ok(Target::Id(1))) || matches!(result, Ok(Target::Id(2))));
    }

    // ============================================================
    // Edge Cases
    // ============================================================

    #[test]
    fn test_empty_context() {
        let ctx = ResolverContext::empty();
        assert!(ctx.is_empty());
        assert_eq!(ctx.len(), 0);

        let result = resolve_target(
            &Target::Text("anything".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Err(ResolverError::NoMatch(_))));
    }

    #[test]
    fn test_normalize_text_whitespace() {
        // Text with extra whitespace should still match
        let elem = make_element(1, Some("  Sign   In  "), None, "button");
        let ctx = make_context(vec![elem]);
        let result = resolve_target(
            &Target::Text("Sign In".into()),
            &ctx,
            ResolutionStrategy::First,
        );
        assert!(matches!(result, Ok(Target::Id(1))));
    }
}
