# Spec vs Implementation Gap Analysis

## Overview

This document identifies gaps between the **SPEC-INTENT-ENGINE.md** specification and the current oryn-core implementation.

**Analysis Date**: 2026-01-18
**Last Updated**: 2026-01-18
**Spec Version**: 1.0
**Implementation**: oryn-core crate

---

## Summary

| Category | Spec Sections | Implementation Status |
|----------|---------------|----------------------|
| Intent Tiers | §2 | ✅ Implemented |
| Intent Definition Format | §3 | ✅ Implemented (YAML + code) |
| Built-in Intent Specifications | §4 | ✅ All 8 intents implemented |
| Intent Execution Model | §5 | ✅ Full 6-stage pipeline |
| Pattern-Intent Mapping | §6 | ✅ Implemented |
| Agent-Defined Intents | §7 | ✅ Implemented |
| Site-Specific Packs | §8 | ✅ Implemented |
| Response Format | §9 | ✅ Implemented (including PartialSuccess) |
| Configuration | §10 | ✅ Implemented |
| Security | §11 | ✅ Implemented |
| Future Features | §12 | ❌ Not Implemented (by design) |

---

## ✅ Fully Implemented Components

### 1. Intent Tiers System (Spec §2)

**Implementation**: `crates/oryn-core/src/intent/registry.rs`

| Spec Requirement | Implementation |
|------------------|----------------|
| Tier 1: Built-in intents | ✅ `IntentTier::BuiltIn` - 8 intents in `intent/builtin/` |
| Tier 2: Loaded intents | ✅ `IntentTier::Loaded` - priority over built-in |
| Tier 3: Discovered intents | ✅ `IntentTier::Discovered` - lowest priority |
| Priority ordering | ✅ Loaded > BuiltIn > Discovered |
| Pattern-to-intent mapping | ✅ Secondary index in registry |

---

### 2. Intent Definition Format (Spec §3)

**Implementation**: `crates/oryn-core/src/intent/definition.rs`

| Spec Requirement | Implementation |
|------------------|----------------|
| Intent metadata (name, version, description) | ✅ `IntentDefinition` struct |
| Triggers (patterns, keywords, urls) | ✅ `IntentTriggers` struct |
| Parameters with types and defaults | ✅ `ParameterDef` with `ParamType` enum |
| Step definitions | ✅ `Step` enum with Action/Branch/Loop/Try/Checkpoint |
| Success/failure conditions | ✅ `SuccessCondition`, `FailureCondition` |
| Intent options (timeout, retry) | ✅ `IntentOptions` with `RetryConfig` |
| YAML file loading | ✅ `IntentLoader` in `intent/loader.rs` |
| Schema validation | ✅ `Validatable` trait in `intent/schema.rs` |

**YAML Loading**: Intents can be loaded from YAML files:
```yaml
# intents/core/login.yaml
intent: login
version: 1.0.0
steps:
  - action: type
    target: { role: email }
```

**Implementation**:
- `IntentLoader::load_from_dir()` loads all `*.yaml` files from a directory
- `Validatable` trait validates intent definitions before registration
- Built-in intents still defined programmatically in `intent/builtin/*.rs`

---

### 3. Built-in Intent Specifications (Spec §4)

**Implementation**: `crates/oryn-core/src/intent/builtin/`

| Intent | File | Spec Compliance |
|--------|------|-----------------|
| `login` | `login.rs` | ✅ Username/password, fallback chain, verification |
| `logout` | `logout.rs` | ✅ User menu detection, try/catch fallback |
| `search` | `search.rs` | ✅ Query input, enter key submission |
| `accept_cookies` | `accept_cookies.rs` | ✅ Banner detection, accept/reject branch |
| `dismiss_popups` | `dismiss_popups.rs` | ✅ Loop with max iterations, try/catch |
| `fill_form` | `fill_form.rs` | ✅ Object data parameter, form field matching |
| `submit_form` | `submit_form.rs` | ✅ Pattern-based submit, wait for navigation |
| `scroll_to` | `scroll_to.rs` | ✅ Target-based scrolling |

---

### 4. Intent Execution Model (Spec §5)

**Implementation**: `crates/oryn-core/src/intent/executor.rs`

| Pipeline Stage | Implementation |
|----------------|----------------|
| 1. PARSE | ✅ Parameter extraction and validation |
| 2. RESOLVE | ✅ Registry lookup, trigger evaluation |
| 3. PLAN | ✅ Initial scan, target resolution |
| 4. EXECUTE | ✅ Step execution with retry, variable binding |
| 5. VERIFY | ✅ Success/failure condition evaluation |
| 6. RESPOND | ✅ `IntentResult` with status, data, logs, changes |

**Target Resolution** (`resolver.rs`):
- ✅ Pattern reference resolution
- ✅ Role matching with scoring
- ✅ Text matching (exact/contains)
- ✅ Selector matching
- ✅ Fallback chain traversal
- ⚠️ Relational targets (Near, Inside, After, Before) - structs exist, partial implementation

**Error Handling**:
- ✅ Exponential backoff retry (configurable attempts, delay, multiplier)
- ✅ Error mapping with recovery hints
- ⚠️ No per-step `on_error` handlers from definition

**Checkpointing**:
- ✅ Checkpoint step type
- ✅ Resume from checkpoint via `execute_with_resume()`
- ✅ Checkpoint state preserved across resume

---

## ✅ Recently Implemented Components

### 5. Pattern-Intent Mapping (Spec §6) ✅ COMPLETE

**Spec Requirement**: When scanner detects patterns, show available intents.

**Implementation**:

**Protocol Types** (`protocol.rs:220-240`):
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

**Executor Integration** (`executor.rs:332-405`):
- `calculate_available_intents()` evaluates each registered intent
- Checks URL patterns against current page URL
- Verifies required patterns are detected on the page
- Returns availability status with reasons

**Formatter Output** (`formatter/mod.rs:180-220`):
```
Available Intents:
- 🟢 login (username, password)
- 🟢 fill_form (data)
- 🟠 search [NavigateRequired]
- 🔴 checkout [Missing pattern: cart]
```

Status icons: 🟢 Ready, 🟠 NavigateRequired, 🔴 MissingPattern, ⚫ Unavailable

---

### 6. Agent-Defined Intents (Spec §7) ✅ COMPLETE

**Spec Requirement**: Agents define intents during sessions using DSL.

**Implementation**:

**Commands** (`command.rs:179-182`):
```rust
Define(String),         // Define new intent from DSL
Undefine(String),       // Remove session intent
Export(String, String), // Export intent to YAML file
Intents(IntentFilter),  // List intents (All or Session)
```

**Parser Support** (`parser.rs:195-198`):
- `define <name>:` - Parses multi-line DSL definition
- `undefine <name>` - Removes session-defined intent
- `export <name> <path>` - Exports intent to YAML file
- `intents` / `intents --session` - Lists available intents

**Supporting Infrastructure**:
- `SessionIntentManager` stores session intents (`intent/session.rs`)
- `DefineParser` parses simplified DSL (`intent/define_parser.rs`)

**Usage**:
```
define add_to_wishlist:
  description: "Add current product to wishlist"
  steps:
    - click "Add to Wishlist"
    - wait visible { text_contains: "added" }

undefine add_to_wishlist

export add_to_wishlist ./my-intents/wishlist.yaml
```

---

### 7. Site-Specific Intent Packs (Spec §8) ✅ COMPLETE

**Spec Requirement**: Load domain-specific patterns and intents.

```
intent-packs/
├── github.com/
│   ├── pack.yaml
│   ├── patterns.yaml
│   └── intents/
│       ├── star_repo.yaml
│       └── create_issue.yaml
```

**Current State**:

| Spec Feature | Status | Notes |
|--------------|--------|-------|
| Pack metadata loading | ✅ | `pack.yaml` parsed |
| Pack trust levels | ✅ | Full, Verified, Sandboxed, Untrusted |
| Intent YAML loading | ✅ | `IntentLoader::load_from_dir()` |
| Schema validation | ✅ | `Validatable` trait |
| `packs` list command | ✅ | `Command::Packs` |
| `pack load <name>` | ✅ | `Command::PackLoad(String)` |
| `pack unload <name>` | ✅ | `Command::PackUnload(String)` |
| Auto-load by URL | ✅ | Wired in `repl.rs:199-216` |
| `pack install --source` | ❌ | No community repo support (future) |
| `pack update` | ❌ | No update mechanism (future) |
| Custom actions (JS) | ❌ | Not supported (future) |

**Auto-Load Implementation** (`repl.rs:199-216`):
```rust
Command::GoTo(url) => {
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
- `auto_load: bool` defaults to `true`
- `pack_paths` defaults to `~/.oryn/packs` and `./packs`

---

### 8. Response Format (Spec §9) ✅ COMPLETE

**Spec Requirement**: Structured result with status, data, logs, and changes.

**Implementation** (`executor.rs:39-53`):
```rust
pub struct IntentResult {
    pub status: IntentStatus,
    pub data: Option<Value>,
    pub logs: Vec<String>,
    pub checkpoint: Option<String>,
    pub hints: Vec<String>,
    pub changes: Option<PageChanges>,
}

pub enum IntentStatus {
    Success,
    PartialSuccess { completed: usize, total: usize },
    Failed(String),
}
```

**Features**:
- ✅ `IntentResult` struct with all required fields
- ✅ Execution logs captured at each step
- ✅ Page changes (before/after scan diff) calculated
- ✅ `PartialSuccess` status with step progress tracking
- ✅ Hints for recovery on partial/failed execution
- ✅ Last checkpoint preserved for resume capability

---

### 9. Configuration System (Spec §10)

**Spec Requirement**: YAML configuration for engine settings.

**Current State**:
- ✅ `IntentEngineConfig` struct exists (`config/schema.rs`)
- ✅ Default values for timeout, retries, delay
- ✅ `LearningConfig` with enable flag and thresholds
- ✅ Per-intent option overrides supported (`--timeout`, `--retry`)

Configuration is handled via struct defaults and per-intent command options. No additional implementation needed for core functionality.

---

## ❌ Not Implemented Components

### 10. Future Features (Spec §12)

These are explicitly marked as future directions in the spec:

| Feature | Description | Status |
|---------|-------------|--------|
| Goal-Level Commands | Natural language goals planned automatically | ❌ |
| Multi-Page Flows | Intents spanning multiple navigations | ❌ |
| Collaborative Learning | Share intents across users | ❌ |
| Intent Composition | Build complex intents from simpler ones | ❌ |

**Goal-Level Commands** (§12.1):
```
goal: "Purchase the cheapest flight to NYC next Friday"

# plan
1. search "flights to NYC"
2. filter by date "next Friday"
3. sort by price
4. select cheapest option
5. checkout

Execute? [yes / modify / cancel]
```

**Multi-Page Flows** (§12.2):
```yaml
intent: complete_purchase
type: flow

pages:
  - name: cart
    url_pattern: "*/cart*"
    intents: [verify_cart, apply_coupon]
    next: checkout
  - name: checkout
    url_pattern: "*/checkout*"
    intents: [fill_shipping, fill_payment]
```

**Intent Composition** (§12.4):
```yaml
intent: setup_new_account
compose:
  - intent: signup
    params: { email: $email, password: $password }
  - intent: verify_email
  - intent: complete_profile
  - intent: configure_notifications
```

---

## Implementation Roadmap

### Phase 1: Core Extensibility ✅ COMPLETE

1. **YAML Intent Loading** ✅
   - `IntentLoader` with schema validation implemented
   - Files: `intent/loader.rs`, `intent/schema.rs`

2. **Pack Auto-Load on Navigation** ✅
   - Fully wired in REPL navigation handler
   - Files: `pack/manager.rs:124`, `repl.rs:199-216`

### Phase 2: Agent Experience ✅ COMPLETE

3. **Pattern-Intent Mapping Output** ✅
   - `IntentAvailability` in protocol and formatter
   - Files: `protocol.rs:220-240`, `formatter/mod.rs:180-220`

4. **Agent-Defined Intent Commands** ✅
   - `define`, `undefine`, `export` commands wired
   - Files: `command.rs:179-182`, `parser.rs:195-198`

5. **PartialSuccess Status** ✅
   - Step progress tracking in executor
   - Files: `executor.rs:48-53`, `executor.rs:114-142`

### Phase 3: Advanced (Future)

6. **Goal-Level Commands** ❌
   - Requires LLM integration for planning

7. **Multi-Page Flows** ❌
   - State machine for cross-page intents

8. **Intent Composition** ❌
   - Compose complex intents from simpler ones
   - Note: Partial support via `intent` action step

---

## File Reference

| Component | Files | Status |
|-----------|-------|--------|
| Registry | `intent/registry.rs` | ✅ Complete |
| Definition | `intent/definition.rs` | ✅ Complete |
| Executor | `intent/executor.rs` | ✅ Complete |
| Verifier | `intent/verifier.rs` | ✅ Complete |
| Resolver | `resolver.rs` | ✅ Complete |
| Built-in Intents | `intent/builtin/*.rs` | ✅ Complete |
| Session Manager | `intent/session.rs` | ✅ Complete |
| Define Parser | `intent/define_parser.rs` | ✅ Complete |
| Intent Loader | `intent/loader.rs` | ✅ Complete |
| Schema Validation | `intent/schema.rs` | ✅ Complete |
| Pack Manager | `pack/manager.rs`, `repl.rs` | ✅ Complete |
| Pack Loader | `pack/loader.rs` | ✅ Complete |
| Config Schema | `config/schema.rs` | ✅ Complete |
| Formatter | `formatter/mod.rs` | ✅ Complete |
| Command Parser | `command.rs` | ✅ Complete |
| Protocol | `protocol.rs` | ✅ Complete |

---

## Appendix: Spec Section Cross-Reference

| Spec § | Title | Implementation |
|--------|-------|----------------|
| 1 | Overview | ✅ Architecture implemented |
| 2 | Intent Tiers | ✅ `IntentTier` enum, registry priority |
| 3 | Definition Format | ✅ Rust structs + YAML loading |
| 4 | Built-in Intents | ✅ All 8 intents |
| 5 | Execution Model | ✅ 6-stage pipeline |
| 6 | Pattern-Intent Mapping | ✅ `IntentAvailability` in output |
| 7 | Agent-Defined Intents | ✅ `define`, `undefine`, `export` commands |
| 8 | Site-Specific Packs | ✅ Complete (including auto-load) |
| 9 | Response Format | ✅ Including PartialSuccess status |
| 10 | Configuration | ✅ Complete |
| 11 | Security | ✅ Pack trust levels |
| 12 | Future Directions | ❌ Not implemented (by design) |
