# Design Document: Recursive Resolution System (Final)

## Executive Summary

This document describes a refactoring of Oryn's target resolution and command execution pipeline. The goal is to transform Oryn from a "fail-fast" system that requires explicit targets into an **intent-satisfying** system that can infer, expand, and recursively resolve commands to achieve the user's goal.

**Key insight**: Instead of requiring users to specify exact targets, Oryn should accept high-level intents and recursively resolve them into executable actions.

**Implementation**: Single sprint with internal phases. No intermediate deliverables required.

---

## Table of Contents

1. [Problem Statement](#problem-statement)
2. [Current Architecture](#current-architecture)
3. [Proposed Architecture](#proposed-architecture)
4. [Core Concepts](#core-concepts)
5. [Data Structures](#data-structures)
6. [Resolution Algorithm](#resolution-algorithm)
7. [Examples](#examples)
8. [Implementation Plan](#implementation-plan)
9. [File Changes Summary](#file-changes-summary)

---

## Problem Statement

### Current Limitations

1. **Rigid Target Requirements**: Commands like `submit` require an explicit target, but users expect `submit` alone to work (find the form, submit it).

2. **CSS Selector Hack**: The executor has a `resolve_selectors_to_ids()` function that breaks the clean pipeline by executing JavaScript mid-resolution.

3. **No Target Inference**: If a user types `type "email" "test@example.com"`, we resolve "email" to an element. But `submit` without a target should infer "submit the current/only form."

4. **Flat Resolution**: Current resolution is one-level deep. We can't express "find the submit button inside the login form containing the email field I just typed into."

5. **No Backtracking**: If resolution fails one way, we don't try alternatives.

### Desired Behavior

```oil
# Current: Fails with "Expected target"
submit

# Desired: Finds the form, finds its submit button, clicks it
submit
  → infer: find active/only form
    → find submit button within form
      → click

# Current: Handled by executor hack
type css(input[role="textbox"]) "hello"

# Desired: Integrated into resolution pipeline
type css(input[role="textbox"]) "hello"
  → resolve css selector via sub-resolution
    → type "hello" into resolved element
```

---

## Current Architecture

```
┌─────────┐    ┌──────────┐    ┌────────────┐    ┌───────────┐
│ Parser  │───▶│ Resolver │───▶│ Translator │───▶│ Executor  │
└─────────┘    └──────────┘    └────────────┘    └───────────┘
     │              │                │                 │
     │         Semantic         Requires          Executes
     │         Resolution       Target::Id        ScannerRequest
     │              │                │                 │
     ▼              ▼                ▼                 ▼
 Command      Target::Id        ScannerRequest    Backend
 with Target  (text/role)       (or fail)
              Selector passes
              through (!)
```

### Pain Points in Current Code

| Location | Issue |
|----------|-------|
| `translator.rs:21-29` | `extract_id()` rejects all non-ID targets |
| `executor.rs:413-457` | `resolve_selectors_to_ids()` hack breaks pipeline |
| `resolver.rs:134` | CSS selectors pass through unresolved |
| `parser.rs` | Commands require explicit targets |

### What Already Works Well

| Component | Status |
|-----------|--------|
| `Target` enum with Text, Role, Relational | ✅ Well-designed |
| Scoring-based resolution | ✅ Flexible |
| `ResolutionStrategy` (PreferInput, etc.) | ✅ Command-aware |
| `DetectedPatterns` (login, search, modal) | ✅ Ready for inference |
| Relational resolution (Inside, Near, etc.) | ✅ Already recursive |

---

## Proposed Architecture

```
┌─────────┐    ┌─────────────────────────────────────────┐    ┌───────────┐
│ Parser  │───▶│          Resolution Engine              │───▶│ Executor  │
└─────────┘    │  ┌─────────────────────────────────┐   │    └───────────┘
     │         │  │       Unified Resolver          │   │         │
     │         │  │  ┌──────────┐  ┌─────────────┐ │   │         │
     │         │  │  │ Infer    │  │ Resolve     │ │   │         │
     │         │  │  │ Missing  │──│ Target      │ │   │         │
     │         │  │  │ Targets  │  │             │ │   │         │
     │         │  │  └──────────┘  └──────┬──────┘ │   │         │
     │         │  │       ▲              │        │   │         │
     │         │  │       │    ┌─────────▼──────┐ │   │         │
     │         │  │       └────│ Sub-Resolution │ │   │         │
     │         │  │            │ (CSS, expand)  │ │   │         │
     │         │  │            └────────────────┘ │   │         │
     │         │  └─────────────────────────────────┘   │         │
     │         │                                        │         │
     │         │  Context: ScanResult + Patterns +     │         │
     │         │           Focus + History + Backend   │         │
     │         └────────────────────────────────────────┘         │
     │                           │                                │
     ▼                           ▼                                ▼
 Command                   ResolvedCommand                 ScannerRequest
 (targets optional)        (all Target::Id)                (executed)
```

### Key Changes

1. **Parser** allows optional targets for commands that support inference
2. **Resolution Engine** is a new unified module that handles all resolution
3. **Unified Resolver** combines semantic resolution + CSS resolution + inference
4. **Translator** remains but becomes trivial (all targets are already `Target::Id`)
5. **Rich Context** includes patterns, focus state, and backend access for sub-resolution

---

## Core Concepts

### 1. Target Requirement (Merged with Resolution Strategy)

Commands declare what they **need**. This merges the existing `ResolutionStrategy` with new requirement types:

```rust
// crates/oryn-core/src/resolution/requirement.rs

/// What kind of element a command requires.
#[derive(Debug, Clone, PartialEq)]
pub enum TargetRequirement {
    /// Any interactive element (default)
    Any,

    /// Must be typeable: input, textarea, contenteditable
    Typeable,

    /// Must be clickable: button, link, interactive element
    Clickable,

    /// Must be checkable: checkbox, radio
    Checkable,

    /// Must be submittable: form or submit button
    Submittable,

    /// Must be a container of specific type
    Container(ContainerType),

    /// Must be selectable: select element
    Selectable,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContainerType {
    Form,
    Modal,
    Dialog,
    Any,
}

impl TargetRequirement {
    /// Convert to legacy ResolutionStrategy for scoring
    pub fn to_strategy(&self) -> ResolutionStrategy {
        match self {
            Self::Typeable => ResolutionStrategy::PreferInput,
            Self::Clickable => ResolutionStrategy::PreferClickable,
            Self::Checkable => ResolutionStrategy::PreferCheckable,
            _ => ResolutionStrategy::Best,
        }
    }
}
```

### 2. Resolution Result (Simplified)

Resolution uses an iterative approach instead of continuations:

```rust
// crates/oryn-core/src/resolution/result.rs

/// Result of a resolution attempt.
#[derive(Debug)]
pub enum Resolution {
    /// Fully resolved to an element ID
    Resolved(u32),

    /// Needs a sub-resolution step (e.g., CSS selector query)
    /// The resolver will handle this and retry
    NeedsBrowserQuery(String),  // CSS selector to query

    /// Resolution failed with reason
    Failed(ResolutionError),
}

#[derive(Debug, Clone)]
pub struct ResolutionError {
    pub target: String,
    pub reason: String,
    pub attempted: Vec<String>,  // Strategies tried
}
```

### 3. Resolution Context (Enhanced)

```rust
// crates/oryn-core/src/resolution/context.rs

use crate::protocol::{ScanResult, DetectedPatterns, Element};
use crate::backend::Backend;

/// All context available for resolution decisions.
pub struct ResolutionContext<'a> {
    /// Latest scan result
    scan: &'a ScanResult,

    /// Currently focused element (if known)
    focused: Option<u32>,

    /// Current scope (if resolving within a container)
    scope: Option<u32>,

    /// Recent command history (for context)
    history: Vec<RecentCommand>,

    /// Backend for dynamic queries (CSS selector resolution)
    backend: Option<&'a dyn Backend>,
}

#[derive(Debug, Clone)]
pub struct RecentCommand {
    pub command_type: String,
    pub target_id: Option<u32>,
}

impl<'a> ResolutionContext<'a> {
    pub fn new(scan: &'a ScanResult) -> Self {
        Self {
            scan,
            focused: None,
            scope: None,
            history: vec![],
            backend: None,
        }
    }

    pub fn with_backend(mut self, backend: &'a dyn Backend) -> Self {
        self.backend = Some(backend);
        self
    }

    pub fn with_focus(mut self, focused: u32) -> Self {
        self.focused = Some(focused);
        self
    }

    /// Create a scoped context for resolution within a container.
    pub fn scoped_to(&self, container_id: u32) -> ResolutionContext<'a> {
        ResolutionContext {
            scan: self.scan,
            focused: self.focused,
            scope: Some(container_id),
            history: self.history.clone(),
            backend: self.backend,
        }
    }

    /// Get elements, optionally filtered by scope.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        let scope = self.scope;
        let scope_rect = scope.and_then(|id| {
            self.scan.elements.iter()
                .find(|e| e.id == id)
                .map(|e| e.rect.clone())
        });

        self.scan.elements.iter().filter(move |e| {
            match &scope_rect {
                Some(rect) => is_inside(&e.rect, rect),
                None => true,
            }
        })
    }

    /// Get detected patterns.
    pub fn patterns(&self) -> Option<&DetectedPatterns> {
        self.scan.patterns.as_ref()
    }

    /// Get the focused element.
    pub fn focused(&self) -> Option<u32> {
        self.focused
    }

    /// Get element by ID.
    pub fn get_element(&self, id: u32) -> Option<&Element> {
        self.scan.elements.iter().find(|e| e.id == id)
    }
}
```

### 4. Inference Rules

When a target is missing, the system uses inference rules:

```rust
// crates/oryn-core/src/resolution/inference.rs

use super::{ResolutionContext, TargetRequirement};
use crate::command::Target;

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
        _ => vec![],
    }
}

fn submittable_rules() -> Vec<InferenceRule> {
    vec![
        // Rule 1: Use detected login pattern submit button
        InferenceRule {
            name: "login_pattern_submit",
            requirement: TargetRequirement::Submittable,
            priority: 100,
            infer: |ctx| {
                ctx.patterns()
                    .and_then(|p| p.login.as_ref())
                    .and_then(|l| l.submit)
                    .map(|id| Target::Id(id as usize))
            },
        },
        // Rule 2: Use detected search pattern submit button
        InferenceRule {
            name: "search_pattern_submit",
            requirement: TargetRequirement::Submittable,
            priority: 95,
            infer: |ctx| {
                ctx.patterns()
                    .and_then(|p| p.search.as_ref())
                    .and_then(|s| s.submit)
                    .map(|id| Target::Id(id as usize))
            },
        },
        // Rule 3: Find the only form and its submit button
        InferenceRule {
            name: "single_form_submit",
            requirement: TargetRequirement::Submittable,
            priority: 80,
            infer: |ctx| {
                let forms: Vec<_> = ctx.elements()
                    .filter(|e| e.element_type == "form")
                    .collect();

                if forms.len() == 1 {
                    // Find submit button inside this form
                    let form_id = forms[0].id;
                    let scoped = ctx.scoped_to(form_id);
                    scoped.elements()
                        .find(|e| {
                            e.element_type == "button" &&
                            e.attributes.get("type").map(|t| t == "submit").unwrap_or(false)
                        })
                        .map(|e| Target::Id(e.id as usize))
                }
                else {
                    None
                }
            },
        },
        // Rule 4: Find any submit button on page
        InferenceRule {
            name: "any_submit_button",
            requirement: TargetRequirement::Submittable,
            priority: 60,
            infer: |ctx| {
                ctx.elements()
                    .find(|e| {
                        (e.element_type == "button" || e.element_type == "input") &&
                        e.attributes.get("type").map(|t| t == "submit").unwrap_or(false)
                    })
                    .map(|e| Target::Id(e.id as usize))
            },
        },
    ]
}

fn form_container_rules() -> Vec<InferenceRule> {
    vec![
        // Rule 1: Form containing focused element
        InferenceRule {
            name: "form_with_focus",
            requirement: TargetRequirement::Container(ContainerType::Form),
            priority: 100,
            infer: |ctx| {
                let focused_id = ctx.focused()?;
                let focused_elem = ctx.get_element(focused_id)?;

                // Find form containing this element
                ctx.elements()
                    .filter(|e| e.element_type == "form")
                    .find(|form| is_inside(&focused_elem.rect, &form.rect))
                    .map(|e| Target::Id(e.id as usize))
            },
        },
        // Rule 2: Single form on page
        InferenceRule {
            name: "single_form",
            requirement: TargetRequirement::Container(ContainerType::Form),
            priority: 80,
            infer: |ctx| {
                let forms: Vec<_> = ctx.elements()
                    .filter(|e| e.element_type == "form")
                    .collect();

                if forms.len() == 1 {
                    Some(Target::Id(forms[0].id as usize))
                } else {
                    None
                }
            },
        },
        // Rule 3: Login form pattern
        InferenceRule {
            name: "login_form_pattern",
            requirement: TargetRequirement::Container(ContainerType::Form),
            priority: 90,
            infer: |ctx| {
                // If login pattern detected, find form containing password field
                let login = ctx.patterns()?.login.as_ref()?;
                let password_elem = ctx.get_element(login.password)?;

                ctx.elements()
                    .filter(|e| e.element_type == "form")
                    .find(|form| is_inside(&password_elem.rect, &form.rect))
                    .map(|e| Target::Id(e.id as usize))
            },
        },
    ]
}

fn is_inside(inner: &crate::protocol::Rect, outer: &crate::protocol::Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.width <= outer.x + outer.width
        && inner.y + inner.height <= outer.y + outer.height
}
```

---

## Data Structures

### Command Metadata

Commands declare their target requirements:

```rust
// crates/oryn-core/src/resolution/command_meta.rs

use super::TargetRequirement;
use crate::command::Command;

/// Metadata about a command's resolution requirements.
pub struct CommandMeta {
    /// What kind of target this command needs
    pub requirement: TargetRequirement,
    /// Whether the target can be inferred if missing
    pub allows_inference: bool,
}

impl CommandMeta {
    pub fn for_command(cmd: &Command) -> Self {
        match cmd {
            Command::Click(_, _) => Self {
                requirement: TargetRequirement::Clickable,
                allows_inference: false,
            },
            Command::Type(_, _, _) => Self {
                requirement: TargetRequirement::Typeable,
                allows_inference: false,
            },
            Command::Submit(_) => Self {
                requirement: TargetRequirement::Submittable,
                allows_inference: true,  // Can infer form's submit button
            },
            Command::Check(_) | Command::Uncheck(_) => Self {
                requirement: TargetRequirement::Checkable,
                allows_inference: false,
            },
            Command::Select(_, _) => Self {
                requirement: TargetRequirement::Selectable,
                allows_inference: false,
            },
            Command::Clear(_) | Command::Focus(_) | Command::Hover(_) => Self {
                requirement: TargetRequirement::Any,
                allows_inference: false,
            },
            _ => Self {
                requirement: TargetRequirement::Any,
                allows_inference: false,
            },
        }
    }
}
```

### Parser Changes

Allow optional targets for inference-capable commands:

```rust
// In parser.rs, the Submit command handling changes:

// Before:
// submit target -> Command::Submit(target)

// After:
// submit -> Command::Submit(Target::Infer)  // New variant
// submit target -> Command::Submit(target)
```

Add new Target variant:

```rust
// crates/oryn-core/src/command.rs

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Target {
    // ... existing variants ...

    /// Placeholder indicating target should be inferred
    Infer,
}
```

---

## Resolution Algorithm

### Main Resolution Engine

```rust
// crates/oryn-core/src/resolution/engine.rs

use crate::command::{Command, Target};
use crate::protocol::ScanResult;
use crate::backend::Backend;
use super::{
    ResolutionContext, Resolution, ResolutionError,
    TargetRequirement, CommandMeta,
    inference::get_inference_rules,
};

pub struct ResolutionEngine;

impl ResolutionEngine {
    /// Resolve a command, returning a command with all targets as Target::Id.
    pub async fn resolve<B: Backend + ?Sized>(
        cmd: Command,
        scan: &ScanResult,
        backend: &mut B,
    ) -> Result<Command, ResolutionError> {
        let ctx = ResolutionContext::new(scan)
            .with_backend(backend);

        Self::resolve_command(cmd, &ctx, backend).await
    }

    async fn resolve_command<B: Backend + ?Sized>(
        cmd: Command,
        ctx: &ResolutionContext<'_>,
        backend: &mut B,
    ) -> Result<Command, ResolutionError> {
        let meta = CommandMeta::for_command(&cmd);

        match &cmd {
            Command::Click(target, opts) => {
                let resolved = Self::resolve_target(
                    target, &meta.requirement, meta.allows_inference, ctx, backend
                ).await?;
                Ok(Command::Click(Target::Id(resolved as usize), opts.clone()))
            }

            Command::Type(target, text, opts) => {
                let resolved = Self::resolve_target(
                    target, &meta.requirement, meta.allows_inference, ctx, backend
                ).await?;
                Ok(Command::Type(Target::Id(resolved as usize), text.clone(), opts.clone()))
            }

            Command::Submit(target) => {
                let resolved = Self::resolve_target(
                    target, &meta.requirement, meta.allows_inference, ctx, backend
                ).await?;
                Ok(Command::Submit(Target::Id(resolved as usize)))
            }

            Command::Check(target) => {
                let resolved = Self::resolve_target(
                    target, &meta.requirement, meta.allows_inference, ctx, backend
                ).await?;
                Ok(Command::Check(Target::Id(resolved as usize)))
            }

            Command::Uncheck(target) => {
                let resolved = Self::resolve_target(
                    target, &meta.requirement, meta.allows_inference, ctx, backend
                ).await?;
                Ok(Command::Uncheck(Target::Id(resolved as usize)))
            }

            Command::Clear(target) => {
                let resolved = Self::resolve_target(
                    target, &meta.requirement, meta.allows_inference, ctx, backend
                ).await?;
                Ok(Command::Clear(Target::Id(resolved as usize)))
            }

            Command::Focus(target) => {
                let resolved = Self::resolve_target(
                    target, &meta.requirement, meta.allows_inference, ctx, backend
                ).await?;
                Ok(Command::Focus(Target::Id(resolved as usize)))
            }

            Command::Hover(target) => {
                let resolved = Self::resolve_target(
                    target, &meta.requirement, meta.allows_inference, ctx, backend
                ).await?;
                Ok(Command::Hover(Target::Id(resolved as usize)))
            }

            Command::Select(target, value) => {
                let resolved = Self::resolve_target(
                    target, &meta.requirement, meta.allows_inference, ctx, backend
                ).await?;
                Ok(Command::Select(Target::Id(resolved as usize), value.clone()))
            }

            // Commands without targets pass through
            _ => Ok(cmd),
        }
    }

    /// Resolve a single target to an element ID.
    async fn resolve_target<B: Backend + ?Sized>(
        target: &Target,
        requirement: &TargetRequirement,
        allows_inference: bool,
        ctx: &ResolutionContext<'_>,
        backend: &mut B,
    ) -> Result<u32, ResolutionError> {
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
            Target::Text(_) | Target::Role(_) => {
                let strategy = requirement.to_strategy();
                match crate::resolver::resolve_target(target, &ctx.to_resolver_context(), strategy) {
                    Ok(Target::Id(id)) => Ok(id as u32),
                    Ok(_) => Err(ResolutionError {
                        target: format!("{:?}", target),
                        reason: "Resolution didn't produce an ID".into(),
                        attempted: vec!["semantic_resolution".into()],
                    }),
                    Err(e) => Err(ResolutionError {
                        target: format!("{:?}", target),
                        reason: e.to_string(),
                        attempted: vec!["semantic_resolution".into()],
                    }),
                }
            }

            // Relational targets - use existing resolver (already recursive)
            Target::Near { .. } | Target::Inside { .. } |
            Target::After { .. } | Target::Before { .. } |
            Target::Contains { .. } => {
                let strategy = requirement.to_strategy();
                match crate::resolver::resolve_target(target, &ctx.to_resolver_context(), strategy) {
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
    async fn infer_target<B: Backend + ?Sized>(
        requirement: &TargetRequirement,
        ctx: &ResolutionContext<'_>,
        backend: &mut B,
    ) -> Result<u32, ResolutionError> {
        let rules = get_inference_rules(requirement);
        let mut attempted = vec![];

        for rule in rules.iter().sorted_by_key(|r| -r.priority) {
            attempted.push(rule.name.to_string());

            if let Some(inferred_target) = (rule.infer)(ctx) {
                // Try to resolve the inferred target
                match Self::resolve_target(&inferred_target, requirement, false, ctx, backend).await {
                    Ok(id) => return Ok(id),
                    Err(_) => continue,  // Try next rule
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
        requirement: &TargetRequirement,
        backend: &mut B,
    ) -> Result<u32, ResolutionError> {
        // Same logic as current resolve_selector in executor.rs
        // but integrated into the resolution pipeline
        let escaped = selector.replace('\\', "\\\\").replace('\'', "\\'");

        let script = format!(
            r#"
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
            "#,
            escaped, escaped, escaped, escaped, escaped
        );

        let req = crate::protocol::ScannerRequest::Execute(
            crate::protocol::ExecuteRequest { script, args: vec![] }
        );

        match backend.execute_scanner(req).await {
            Ok(resp) => {
                // Parse response to extract ID
                // (Same parsing logic as current resolve_selector)
                Self::parse_selector_response(&resp, selector)
            }
            Err(e) => Err(ResolutionError {
                target: selector.into(),
                reason: e.to_string(),
                attempted: vec!["browser_query".into()],
            }),
        }
    }

    fn validate_requirement(id: u32, requirement: &TargetRequirement, ctx: &ResolutionContext) -> bool {
        let elem = match ctx.get_element(id) {
            Some(e) => e,
            None => return false,
        };

        match requirement {
            TargetRequirement::Any => true,

            TargetRequirement::Typeable => {
                matches!(elem.element_type.as_str(), "input" | "textarea" | "select")
                    || elem.attributes.get("contenteditable").map(|v| v == "true").unwrap_or(false)
            }

            TargetRequirement::Clickable => {
                matches!(elem.element_type.as_str(), "button" | "a")
                    || elem.attributes.get("role").map(|r| r == "button").unwrap_or(false)
                    || (elem.element_type == "input" &&
                        elem.attributes.get("type").map(|t|
                            matches!(t.as_str(), "submit" | "button" | "reset")
                        ).unwrap_or(false))
            }

            TargetRequirement::Checkable => {
                elem.attributes.get("type")
                    .map(|t| matches!(t.as_str(), "checkbox" | "radio"))
                    .unwrap_or(false)
                    || elem.attributes.get("role")
                        .map(|r| matches!(r.as_str(), "checkbox" | "radio" | "switch"))
                        .unwrap_or(false)
            }

            TargetRequirement::Submittable => {
                elem.element_type == "form"
                    || (elem.element_type == "button" &&
                        elem.attributes.get("type").map(|t| t == "submit").unwrap_or(true))
                    || (elem.element_type == "input" &&
                        elem.attributes.get("type").map(|t| t == "submit").unwrap_or(false))
            }

            TargetRequirement::Container(ct) => {
                match ct {
                    ContainerType::Form => elem.element_type == "form",
                    ContainerType::Modal | ContainerType::Dialog => {
                        elem.attributes.get("role")
                            .map(|r| matches!(r.as_str(), "dialog" | "alertdialog"))
                            .unwrap_or(false)
                            || elem.element_type == "dialog"
                    }
                    ContainerType::Any => true,
                }
            }

            TargetRequirement::Selectable => {
                elem.element_type == "select"
                    || elem.attributes.get("role").map(|r| r == "listbox").unwrap_or(false)
            }
        }
    }

    fn parse_selector_response(
        resp: &crate::protocol::ScannerProtocolResponse,
        selector: &str,
    ) -> Result<u32, ResolutionError> {
        use crate::protocol::{ScannerProtocolResponse, ScannerData};

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
            ScannerProtocolResponse::Error { message, .. } => {
                Err(ResolutionError {
                    target: selector.into(),
                    reason: message.clone(),
                    attempted: vec!["browser_query".into()],
                })
            }
        }
    }
}
```

---

## Examples

### Example 1: `submit` (No Target - Inference)

```
Input: submit

Resolution Flow:
1. Parser produces: Command::Submit(Target::Infer)
2. ResolutionEngine.resolve_command() called
3. CommandMeta shows allows_inference=true for Submit
4. resolve_target() sees Target::Infer, calls infer_target()
5. get_inference_rules(Submittable) returns rules
6. Rule "login_pattern_submit" checks ctx.patterns().login.submit
   → Found: element 12
7. validate_requirement(12, Submittable) → true
8. Returns: Command::Submit(Target::Id(12))

Output: Command::Submit(Target::Id(12))
```

### Example 2: `type css(input[name="email"]) "test@example.com"`

```
Input: type css(input[name="email"]) "test@example.com"

Resolution Flow:
1. Parser produces: Command::Type(Target::Selector("input[name='email']"), "test@...", {})
2. resolve_target() sees Target::Selector
3. resolve_selector() executes JavaScript query
4. Browser returns element ID 7
5. validate_requirement(7, Typeable) → true (it's an input)
6. Returns: Command::Type(Target::Id(7), "test@example.com", {})

Output: Command::Type(Target::Id(7), "test@example.com", {})
```

### Example 3: `type "Email" "test@example.com"` with Backtracking

```
Input: type "Email" "test@example.com"

Resolution Flow:
1. Parser produces: Command::Type(Target::Text("Email"), "test@...", {})
2. resolve_target() with requirement=Typeable
3. Calls existing resolver with strategy=PreferInput
4. Existing resolver scores elements:
   - Label "Email" (element 4): text match 100pts, but NOT typeable → -30 bonus
   - Input labeled "Email" (element 5): label match 90pts + input bonus +50 = 140pts
5. Element 5 wins with highest score
6. validate_requirement(5, Typeable) → true
7. Returns: Command::Type(Target::Id(5), "test@example.com", {})

Output: Command::Type(Target::Id(5), "test@example.com", {})
```

---

## Implementation Plan

### Single Sprint Structure

```
Week 1: Foundation + Inference
Week 2: CSS Integration + Parser Changes
Week 3: Integration + Testing
```

### Phase 1: Foundation (Days 1-5)

**Create new module structure:**

```
crates/oryn-core/src/resolution/
├── mod.rs           # Module exports
├── requirement.rs   # TargetRequirement enum
├── context.rs       # ResolutionContext
├── result.rs        # Resolution enum
├── inference.rs     # InferenceRule and implementations
├── command_meta.rs  # CommandMeta
└── engine.rs        # ResolutionEngine
```

**Deliverables:**
- [x] `TargetRequirement` enum with validation
- [x] `ResolutionContext` with scoping
- [x] Basic inference rules for `Submittable`
- [x] Unit tests for requirement validation

### Phase 2: CSS Integration (Days 6-8)

**Move CSS resolution from executor to resolution engine:**

- [x] Move `resolve_selector()` logic to `resolution/engine.rs`
- [x] Remove `resolve_selectors_to_ids()` from executor
- [x] Update executor to use `ResolutionEngine::resolve()`

### Phase 3: Parser Changes (Days 9-10)

**Allow optional targets:**

- [x] Add `Target::Infer` variant
- [x] Update parser for `submit` without target
- [ ] Update parser for `scroll` without target

### Phase 4: Integration (Days 11-13)

**Wire everything together:**

- [x] Update `CommandExecutor` to use `ResolutionEngine`
- [x] Ensure translator receives only `Target::Id`
- [x] Verify all existing tests pass

### Phase 5: Additional Inference Rules (Days 14-15)

**Expand inference capabilities:**

- [x] Add inference rules for modal dismiss/accept
- [x] Add inference rules for search submit
- [x] Add form container inference

### Phase 6: Testing (Days 16-18)

- [x] Integration tests for inference scenarios
- [ ] E2E tests with `submit` without target
- [ ] E2E tests with CSS selectors
- [ ] Performance verification

---

## File Changes Summary

### New Files

| File | Purpose |
|------|---------|
| `src/resolution/mod.rs` | Module exports |
| `src/resolution/requirement.rs` | `TargetRequirement` enum |
| `src/resolution/context.rs` | `ResolutionContext` struct |
| `src/resolution/result.rs` | `Resolution` enum |
| `src/resolution/inference.rs` | Inference rules |
| `src/resolution/command_meta.rs` | Command metadata |
| `src/resolution/engine.rs` | Main resolution engine |

### Modified Files

| File | Changes |
|------|---------|
| `src/command.rs` | Add `Target::Infer` variant |
| `src/parser.rs` | Allow optional targets for submit, scroll |
| `src/executor.rs` | Remove `resolve_selectors_to_ids()`, use `ResolutionEngine` |
| `src/lib.rs` | Export `resolution` module |

### Files to Keep Unchanged

| File | Reason |
|------|--------|
| `src/resolver.rs` | Existing semantic resolution still used |
| `src/translator.rs` | Keep as thin mapping layer |
| `src/protocol.rs` | No changes needed |

---

## Scanner Changes Required

### Focus Tracking

The scanner needs to report the currently focused element. Add to `ScanResult`:

```rust
// In protocol.rs ScanResult
#[serde(default, skip_serializing_if = "Option::is_none")]
pub focused_element: Option<u32>,
```

Scanner JavaScript needs to detect `document.activeElement` and report its ID.

---

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Inference chooses wrong element | Inference rules have priority ordering; highest-confidence rules run first |
| CSS resolution adds latency | Same latency as current hack; no change |
| Backward compatibility | `Target::Infer` is new; existing scripts with explicit targets work unchanged |
| Complex inference chains | Limit inference depth to 2 levels; fail fast with clear errors |

---

## Success Criteria

1. **`submit` works without target** when a form is present
2. **CSS selectors** work without executor hack
3. **All existing E2E tests pass** unchanged
4. **New tests** cover inference scenarios
5. **No performance regression** in resolution time

---

## Open Questions (Resolved)

| Question | Resolution |
|----------|------------|
| Depth limits for recursion | Max 2 levels for inference; relational targets already unlimited |
| Ambiguity handling | Use existing scoring system; highest score wins |
| Debugging | Resolution errors include `attempted` list showing what was tried |
| Learning from success | Out of scope for this sprint; can add later |

---

## Conclusion

This design refactors the resolution pipeline to be intent-aware while preserving backward compatibility. The key changes are:

1. **New `resolution/` module** with unified resolution logic
2. **Target inference** for commands that support it (starting with `submit`)
3. **CSS selector resolution** moved from executor hack to proper pipeline
4. **Enhanced context** with patterns, focus, and scoping

The implementation is scoped to a single sprint with clear phases and deliverables.
