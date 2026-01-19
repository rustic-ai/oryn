# Implementation Priorities

## Overview

This document analyzes the gaps between the Intent Engine specification and current implementation, categorizing them by functional importance.

**Analysis Date**: 2026-01-18
**Last Updated**: 2026-01-18
**Related Document**: [SPEC-IMPLEMENTATION-GAPS.md](./SPEC-IMPLEMENTATION-GAPS.md)

---

## Current State Summary

The intent engine is **feature complete** for Phase 1 and Phase 2:

### Core Infrastructure ✅
- ✅ 6-stage execution pipeline (resolve → parse → plan → execute → verify → respond)
- ✅ All 8 built-in intents (login, logout, search, accept_cookies, dismiss_popups, fill_form, submit_form, scroll_to)
- ✅ Target resolution with fallback chains (pattern → role → text → selector)
- ✅ Relational targets (Near, Inside, After, Before, Contains) in resolver
- ✅ Per-step on_error handlers (YAML only)
- ✅ Retry logic with exponential backoff
- ✅ Checkpointing and resume support
- ✅ Session intent manager and define parser
- ✅ Pack manager with trust levels
- ✅ Learning observer and pattern recognizer

### Extensibility ✅
- ✅ YAML intent loading from directories (`IntentLoader`)
- ✅ Schema validation for intent definitions (`Validatable` trait)
- ✅ `define`, `undefine`, `export` commands wired to parser

### Agent Experience ✅
- ✅ Available intents output in scan results (`IntentAvailability`)
- ✅ Availability status indicators (Ready, NavigateRequired, MissingPattern, Unavailable)
- ✅ `PartialSuccess` status with step progress tracking
- ✅ Formatted output with intent availability icons

**Status**: All Phase 1 and Phase 2 features are complete

---

## Gap Analysis by Priority

### 🔴 Core Functional Gaps

These gaps block or significantly limit key use cases.

#### 1. YAML Intent Loading ✅ COMPLETE

**Spec Reference**: §3, §8

**Status**: ✅ Fully implemented

**Implementation**:
- `crates/oryn-core/src/intent/loader.rs` - `IntentLoader::load_from_dir()` loads YAML files using glob patterns
- `crates/oryn-core/src/intent/schema.rs` - `Validatable` trait with validation for `IntentDefinition` and `Step`
- Schema validation integrated into loader with proper error handling

**Key Code**:
```rust
// crates/oryn-core/src/intent/loader.rs
impl IntentLoader {
    pub fn load_from_dir(path: &Path, registry: &mut IntentRegistry) -> Result<usize, LoaderError>;
}

// crates/oryn-core/src/intent/schema.rs
pub trait Validatable {
    fn validate(&self) -> Result<(), ValidationError>;
}
```

---

#### 2. Pack Auto-Load on Navigation ✅ COMPLETE

**Spec Reference**: §8.5

**Status**: ✅ Fully implemented

**Implementation** (`repl.rs:199-216`):
```rust
Command::GoTo(url) => {
    // Auto-load pack if configured
    if state.config.packs.auto_load {
        if let Some(pack_name) = state.pack_manager.should_auto_load(url) {
            if let Err(e) = state.pack_manager.load_pack_by_name(&pack_name).await {
                eprintln!("Warning: Failed to auto-load pack for {}: {}", pack_name, e);
            } else {
                println!("Auto-loaded pack: {}", pack_name);
            }
        }
    }
    // ... navigate ...
}
```

**Configuration** (`config/schema.rs:58-77`):
- `auto_load: bool` - defaults to `true`
- `pack_paths: Vec<PathBuf>` - defaults to `~/.oryn/packs` and `./packs`

**Flow**:
1. User runs `goto https://github.com/...`
2. REPL checks `config.packs.auto_load` (default: true)
3. Calls `pack_manager.should_auto_load(url)` to find matching pack
4. If found and not loaded, calls `load_pack_by_name()` to load it
5. Proceeds with navigation

---

### 🟡 Important for Agent Experience

These gaps affect agent usability but have workarounds.

#### 3. `define` Command Wiring ✅ COMPLETE

**Spec Reference**: §7

**Status**: ✅ Fully implemented

**Implementation**:
- `Command::Define(String)` - Define a new intent (`command.rs:179`)
- `Command::Undefine(String)` - Remove a session intent (`command.rs:180`)
- `Command::Export(String, String)` - Export intent to YAML (`command.rs:181`)
- Parser handles `define`, `undefine`, `export` commands (`parser.rs:195-197`)

**Parser Syntax** (now supported):
```
define checkout:
  description: "Complete checkout flow"
  steps:
    - click "Proceed to Checkout"
    - fill_form $shipping_data
    - click "Place Order"

undefine checkout

export checkout ./my-intents/checkout.yaml
```

**Supporting Infrastructure**:
- `SessionIntentManager` stores session intents (`intent/session.rs`)
- `DefineParser` parses DSL syntax (`intent/define_parser.rs`)

---

#### 4. Available Intents in Output ✅ COMPLETE

**Spec Reference**: §6

**Status**: ✅ Fully implemented

**Implementation**:

**Protocol Types** (`protocol.rs:224-240`):
```rust
pub struct IntentAvailability {
    pub name: String,
    pub status: AvailabilityStatus,
    pub parameters: Vec<String>,
    pub trigger_reason: Option<String>,
}

pub enum AvailabilityStatus {
    Ready,              // Can execute now
    NavigateRequired,   // Need to navigate first
    MissingPattern,     // Required pattern not detected
    Unavailable,        // Cannot execute
}
```

**Executor Integration** (`executor.rs:353-426`):
- `calculate_available_intents()` evaluates each intent against current page
- Checks URL patterns and required patterns from intent triggers
- Returns availability list with status and reasons

**Formatter Output** (`formatter/mod.rs:180-247`):
```
@ example.com/login "Sign In"

[1] input/email "Email" {required}
[2] input/password "Password" {required}
[3] button/submit "Sign In"

Patterns:
- login_form

Available Intents:
- 🟢 login (username, password)
- 🟢 fill_form (data)
- 🟠 search [NavigateRequired]
- 🔴 checkout [Missing pattern: cart]
```

Status icons: 🟢 Ready, 🟠 NavigateRequired, 🔴 MissingPattern, ⚫ Unavailable

---

#### 5. PartialSuccess Status ✅ COMPLETE

**Spec Reference**: §9.3

**Status**: ✅ Fully implemented

**Implementation** (`executor.rs:39-53`):
```rust
pub enum IntentStatus {
    Success,
    PartialSuccess { completed: usize, total: usize },
    Failed(String),
}

pub struct IntentResult {
    pub status: IntentStatus,
    pub data: Option<Value>,
    pub logs: Vec<String>,
    pub checkpoint: Option<String>,
    pub hints: Vec<String>,
    pub changes: Option<PageChanges>,
}
```

**Executor Logic** (`executor.rs:114-142`):
- Tracks `steps_completed` counter during execution
- On step failure with `steps_completed > 0`, returns `PartialSuccess`
- Includes hints about which step failed and why
- Preserves last checkpoint for resume capability

**Example Output**:
```
Intent: checkout
Status: PartialSuccess (3/5 steps completed)
Checkpoint: payment_entered
Hint: Failed at step 4: Payment validation timeout
```

---

### 🔵 Advanced / Future

These are complex features that aren't core to the primary use case.

#### 6. Goal-Level Commands

**Spec Reference**: §12.1

**Current State**: Not implemented (explicitly future in spec).

**Complexity**: High (requires planning/reasoning layer)

**Justification to Defer**: Requires LLM integration for planning.

---

#### 7. Multi-Page Flows

**Spec Reference**: §12.2

**Current State**: Not implemented (explicitly future in spec).

**Complexity**: Medium

**Justification to Defer**: Can chain intents manually. State machine adds complexity.

---

#### 8. Intent Composition ✅ EFFECTIVELY COMPLETE

**Spec Reference**: §12.4

**Current State**: Fully functional via `define` + `action: intent` steps.

**What Works**:
- Multi-step intent definitions with `define` command
- Calling intents from within intents using `action: intent`
- Parameter passing with variable resolution (`$var_name`)
- Combined with `branch`, `loop`, `try` for complex flows
- Export to YAML for reuse

**Example**:
```yaml
define checkout:
  steps:
    - action: intent
      name: fill_form
      data: $shipping
    - click "Continue"
    - action: intent
      name: fill_form
      data: $payment
    - action: intent
      name: submit_form
```

**Spec's `compose:` Syntax**: The spec proposes a dedicated `compose:` keyword, but this is purely syntactic sugar. The current `steps` + `action: intent` approach achieves identical functionality.

---

## Recommended Implementation Order

### Phase 1: Core Extensibility ✅ COMPLETE
**Goal**: Enable users to define custom intents without recompilation.

| Task | Priority | Effort | Status |
|------|----------|--------|--------|
| YAML Intent Loader | High | Medium | ✅ Complete |
| Pack Auto-Load Wiring | High | Low | ✅ Complete |

### Phase 2: Agent Experience ✅ COMPLETE
**Goal**: Improve agent discoverability and feedback.

| Task | Priority | Effort | Status |
|------|----------|--------|--------|
| Wire `define` command | Medium | Medium | ✅ Complete |
| Available intents output | Medium | Medium | ✅ Complete |
| PartialSuccess status | Medium | Low | ✅ Complete |

### Phase 3: Advanced (Future)
**Goal**: Advanced automation capabilities.

| Task | Priority | Effort | Status |
|------|----------|--------|--------|
| Goal-level commands | Low | High | ❌ Not started |
| Multi-page flows | Low | Medium | ❌ Not started |
| Intent composition | Low | Low | ✅ Effectively complete (via `define` + `action: intent`) |

---

## Decision Matrix

Use this to prioritize based on your deployment model:

| Deployment Model | Remaining Gaps | Status |
|------------------|----------------|--------|
| **Fixed product with built-in intents** | None | ✅ Ready |
| **Platform for user-defined automation** | None | ✅ Ready |
| **AI agent for web automation** | None | ✅ Ready |
| **Site-specific automation packs** | None | ✅ Ready |

---

## Appendix: Implementation Summary

| Component | Files | Status |
|-----------|-------|--------|
| YAML Intent Loader | `intent/loader.rs`, `intent/schema.rs` | ✅ Complete |
| Define Commands | `command.rs:179-181`, `parser.rs:195-197` | ✅ Complete |
| Available Intents | `protocol.rs:224-240`, `executor.rs:353-426`, `formatter/mod.rs:180-247` | ✅ Complete |
| PartialSuccess | `executor.rs:39-53`, `executor.rs:114-142` | ✅ Complete |
| Pack Auto-Load | `pack/manager.rs:124`, `repl.rs:199-216` | ✅ Complete |
| Relational Targets | `resolver.rs:280-563` | ✅ Complete |
| Per-step on_error | `executor.rs:296-315` (YAML only) | ✅ Complete |

---

## Related Documents

- [SPEC-INTENT-ENGINE.md](./SPEC-INTENT-ENGINE.md) - Full specification
- [SPEC-IMPLEMENTATION-GAPS.md](./SPEC-IMPLEMENTATION-GAPS.md) - Detailed gap analysis
