//! Semantic Target Resolution
//!
//! Maps AST `Target`s to concrete Element IDs using a `ResolverContext`.

use crate::ast::{Target, TargetAtomic, TargetRelation, RelationKind};
use oryn_common::protocol::{Element, ScanResult}; // Use protocol elements directly
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

    #[error("Relational resolution failed: {0}")]
    RelationalError(String),

    #[error("Stale context: scan result is too old or missing")]
    StaleContext,
}

/// Strategy for handling multiple matches.
#[derive(Debug, Clone, Copy, Default)]
pub enum ResolutionStrategy {
    #[default]
    First,
    Unique,
    Best,
    PreferInput,
    PreferClickable,
    PreferCheckable,
}

/// Context for resolving semantic targets.
#[derive(Debug, Clone)]
pub struct ResolverContext {
    elements: Vec<Element>,
}

impl ResolverContext {
    pub fn new(scan_result: &ScanResult) -> Self {
        Self {
            elements: scan_result.elements.clone(),
        }
    }

    pub fn empty() -> Self {
        Self { elements: vec![] }
    }

    pub fn elements(&self) -> &[Element] {
        &self.elements
    }

    pub fn get_element(&self, id: u32) -> Option<&Element> {
        self.elements.iter().find(|e| e.id == id)
    }
}

pub fn resolve_target(
    target: &Target,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<Target, ResolverError> {
    // Recursive resolution
    // Currently, Target is { atomic, relation }
    // If relation exists, we resolve the relation (which includes another target)
    // Legacy resolver handled relations as: Near { target, anchor }
    // AST is: atomic (target) -> relation -> anchor (target)
    // E.g. "input near 'Email'" -> atomic=input, relation=Near(anchor='Email')
    
    if let Some(relation) = &target.relation {
        resolve_relation(&target.atomic, &relation.kind, &relation.target, ctx, strategy)
    } else {
        resolve_atomic(&target.atomic, ctx, strategy)
    }
}

fn resolve_atomic(
    atomic: &TargetAtomic,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<Target, ResolverError> {
    match atomic {
        TargetAtomic::Id(id) => {
            if ctx.get_element(*id as u32).is_some() {
                 Ok(Target {
                     atomic: TargetAtomic::Id(*id),
                     relation: None,
                 })
            } else {
                Err(ResolverError::NoMatch(format!("Element ID {} not found", id)))
            }
        }
        TargetAtomic::Text(text) => {
            let id = resolve_by_text(text, ctx, strategy)?;
            Ok(Target { atomic: TargetAtomic::Id(id as usize), relation: None })
        }
        TargetAtomic::Role(role) => {
            let id = resolve_by_role(role, ctx, strategy)?;
            Ok(Target { atomic: TargetAtomic::Id(id as usize), relation: None })
        }
        TargetAtomic::Selector { kind, value } => {
            // Selectors are generally passed through if not supported by resolver, 
            // but here we are resolving to ID. 
            // If we can't resolve selector locally (requires browser), we might return as is?
            // BUT: Action enum requires IDs for many things.
            // If the pipeline is Parse -> Resolve -> Action, resolved Action needs ID.
            // If we can't resolve, we fail?
            // "Selectors... pass through (scanner handles it)" said legacy resolver.
            // Legacy resolver returned Target::Selector for selectors.
            // So we should return Target with Selector atomic if we can't resolve.
            Ok(Target {
                atomic: TargetAtomic::Selector { kind: kind.clone(), value: value.clone() },
                relation: None,
            })
        }
    }
}

fn resolve_relation(
    target_atomic: &TargetAtomic,
    kind: &RelationKind,
    anchor_target: &Target,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<Target, ResolverError> {
    // Resolve anchor first
    let anchor_resolved = resolve_target(anchor_target, ctx, ResolutionStrategy::First)?;
    
    // Anchor must be an ID
    let anchor_id = match anchor_resolved.atomic {
        TargetAtomic::Id(id) => id as u32,
        _ => return Err(ResolverError::RelationalError("Anchor could not be resolved to ID".into())),
    };

    let anchor_elem = ctx.get_element(anchor_id).ok_or_else(|| {
        ResolverError::NoMatch(format!("Anchor element {} not found", anchor_id))
    })?;

    // Find candidates for the target
    // We create a temporary Target for atomic part to reuse get_matching_candidates
    let temp_target = Target { atomic: target_atomic.clone(), relation: None };
    let candidates = get_matching_candidates(&temp_target, ctx)?;

    if candidates.is_empty() {
        return Err(ResolverError::NoMatch(format!("{:?} relation", target_atomic)));
    }

    let mut scored_matches: Vec<(u32, i32)> = Vec::new();

    for id in candidates {
        let elem = ctx.get_element(id).unwrap();
        let matches = match kind {
            RelationKind::Near => {
                // Distance based
                let dist = distance_center(&elem.rect, &anchor_elem.rect);
                let score = (10000.0 / (dist + 1.0)) as i32;
                Some(score)
            }
            RelationKind::Inside => {
                // Target inside Anchor
                if is_inside(&elem.rect, &anchor_elem.rect) {
                    Some(100)
                } else {
                    None
                }
            }
            RelationKind::Contains => {
                // Target contains Anchor
                if is_inside(&anchor_elem.rect, &elem.rect) {
                    Some(100)
                } else {
                    None
                }
            }
            RelationKind::After => {
                // Target after Anchor
                 if is_after(&elem.rect, &anchor_elem.rect) {
                     let dist = distance_l1(&elem.rect, &anchor_elem.rect);
                     Some((10000.0 / (dist + 1.0)) as i32)
                 } else {
                     None
                 }
            }
            RelationKind::Before => {
                // Target before Anchor
                if is_before(&elem.rect, &anchor_elem.rect) {
                    let dist = distance_l1(&elem.rect, &anchor_elem.rect);
                    Some((10000.0 / (dist + 1.0)) as i32)
                } else {
                    None
                }
            }
        };

        if let Some(score) = matches {
            scored_matches.push((id, score));
        }
    }

    let selected_id = select_match(&scored_matches, "Relation", strategy, Some(ctx))?;
    Ok(Target { atomic: TargetAtomic::Id(selected_id as usize), relation: None })
}

// ... Implement logic for text/role scoring, similar to legacy ...

fn resolve_by_text(
    text: &str,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<u32, ResolverError> {
    let normalized = normalize_text(text);
    let mut matches = Vec::new();

    for elem in ctx.elements() {
        let mut score = 0;
        
        // Port scoring logic from legacy
        if let Some(t) = &elem.text {
            if normalize_text(t) == normalized { score = 100; }
            else if normalize_text(t).contains(&normalized) { score = 50; }
        }
        // ... (Label, Placeholder, Value, attributes) ...
         if let Some(ref label) = elem.label {
            if normalize_text(label) == normalized {
                score = score.max(90);
            } else if normalize_text(label).contains(&normalized) {
                score = score.max(45);
            }
        }
         // HTML name attribute match (for form inputs)
        if let Some(name_attr) = elem.attributes.get("name") {
            if normalize_text(name_attr) == normalized {
                score = score.max(86);
            } else if normalize_text(name_attr).contains(&normalized) {
                score = score.max(43);
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
        
        if score > 0 {
            matches.push((elem.id, score));
        }
    }

    select_match(&matches, text, strategy, Some(ctx))
}

fn resolve_by_role(
    role: &str,
    ctx: &ResolverContext,
    strategy: ResolutionStrategy,
) -> Result<u32, ResolverError> {
    let normalized = role.to_lowercase();
    let mut matches = Vec::new();

    for elem in ctx.elements() {
        let mut score = 0;
        if elem.element_type.to_lowercase() == normalized { score = 80; }
        if let Some(r) = &elem.role { if r.to_lowercase() == normalized { score = 100; } }
        // ... Check type attribute, aria-role ...
        
        if score > 0 {
            matches.push((elem.id, score));
        }
    }
    
    select_match(&matches, role, strategy, Some(ctx))
}

// Helpers

fn normalize_text(s: &str) -> String {
    s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn select_match(
    matches: &[(u32, i32)],
    desc: &str,
    strategy: ResolutionStrategy,
    ctx: Option<&ResolverContext>,
) -> Result<u32, ResolverError> {
    if matches.is_empty() { return Err(ResolverError::NoMatch(desc.into())); }
    
    let mut sorted = matches.to_vec();
    
    if let Some(context) = ctx {
        for (id, score) in sorted.iter_mut() {
             if let Some(elem) = context.get_element(*id) {
                 // apply strategy bonuses
                 match strategy {
                     ResolutionStrategy::PreferInput => {
                         if matches!(elem.element_type.as_str(), "input" | "textarea" | "select") { *score += 50; }
                     }
                     // ...
                     _ => {}
                 }
             }
        }
    }

    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    match strategy {
        ResolutionStrategy::Unique => {
             if sorted.len() > 1 && sorted[0].1 == sorted[1].1 {
                 Err(ResolverError::AmbiguousMatch {
                     target: desc.into(),
                     count: sorted.len(),
                     candidates: sorted.iter().map(|k| k.0).collect(),
                 })
             } else {
                 Ok(sorted[0].0)
             }
        }
        _ => Ok(sorted[0].0),
    }
}

fn get_matching_candidates(target: &Target, ctx: &ResolverContext) -> Result<Vec<u32>, ResolverError> {
    match &target.atomic {
        TargetAtomic::Id(id) => Ok(vec![*id as u32]),
        TargetAtomic::Text(text) => {
             let normalized = normalize_text(text);
             Ok(ctx.elements.iter().filter(|e| 
                 e.text.as_ref().map(|t| normalize_text(t).contains(&normalized)).unwrap_or(false)
                 // or label, etc.
             ).map(|e| e.id).collect())
        }
        TargetAtomic::Role(role) => {
            let normalized = role.to_lowercase();
            Ok(ctx.elements.iter().filter(|e|
                e.element_type == normalized || e.role.as_ref().map(|r| r == &normalized).unwrap_or(false)
            ).map(|e| e.id).collect())
        }
        _ => Ok(vec![])
    }
}

fn distance_center(r1: &oryn_common::protocol::Rect, r2: &oryn_common::protocol::Rect) -> f32 {
    let c1 = (r1.x + r1.width/2.0, r1.y + r1.height/2.0);
    let c2 = (r2.x + r2.width/2.0, r2.y + r2.height/2.0);
    ((c1.0 - c2.0).powi(2) + (c1.1 - c2.1).powi(2)).sqrt()
}

fn distance_l1(r1: &oryn_common::protocol::Rect, r2: &oryn_common::protocol::Rect) -> f32 {
    (r1.x - r2.x).abs() + (r1.y - r2.y).abs()
}

fn is_inside(inner: &oryn_common::protocol::Rect, outer: &oryn_common::protocol::Rect) -> bool {
    inner.x >= outer.x && inner.y >= outer.y 
    && inner.x + inner.width <= outer.x + outer.width 
    && inner.y + inner.height <= outer.y + outer.height
}

fn is_after(target: &oryn_common::protocol::Rect, anchor: &oryn_common::protocol::Rect) -> bool {
    let anchor_bottom = anchor.y + anchor.height;
    target.y >= anchor_bottom || (target.y >= anchor.y && target.x > anchor.x + anchor.width)
}

fn is_before(target: &oryn_common::protocol::Rect, anchor: &oryn_common::protocol::Rect) -> bool {
    let target_bottom = target.y + target.height;
    target_bottom <= anchor.y || (target.y <= anchor.y + anchor.height && target.x + target.width < anchor.x)
}
