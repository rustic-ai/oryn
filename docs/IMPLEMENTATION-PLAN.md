# Oryn Intent Engine Implementation Plan

## Overview

Implement the Intent Engine as specified in SPEC-INTENT-ENGINE.md, transforming high-level agent commands into sequences of atomic scanner operations with verification, error handling, and pattern-based availability.

## Current State

> **Last Updated:** 2026-01-17

**Core Infrastructure** (in `crates/oryn-core/src/`):
- `parser.rs` (1,014 lines) - Tokenizer, command parsing, target parsing
- `resolver.rs` (1,556 lines) - Target resolution with scoring
- `translator.rs` (328 lines) - Command → ScannerRequest
- `protocol.rs` (423 lines) - Request/Response, Pattern structs
- `error_mapping.rs` (290 lines) - Error codes and hints
- `command.rs` (172 lines) - Command enum with 30+ commands

**Intent Engine** (in `crates/oryn-core/src/intent/`):
- `definition.rs` (250 lines) - All core types ✅
- `registry.rs` (128 lines) - Tier-based registry ✅
- `executor.rs` (252 lines) - 6-stage pipeline (partial) ⚠️
- `verifier.rs` (46 lines) - Condition checking (stubs) ⚠️
- `mapper.rs` (56 lines) - Pattern-to-intent mapping ✅
- `loader.rs` (49 lines) - YAML loading ✅
- `builtin/` (8 intents) - All definitions created ✅

---

## Implementation Status

| Phase | Description | Status | Completion |
|-------|-------------|--------|------------|
| Phase 1 | Intent Infrastructure | ✅ Complete | 100% |
| Phase 2 | Built-in Intents | ✅ Complete | 95% |
| Phase 3 | Execution Pipeline | ⚠️ Partial | 60% |
| Phase 4 | Pattern-Intent Mapping | ⚠️ Partial | 80% |
| Phase 5 | YAML Loading | ✅ Complete | 100% |
| Tests | Unit & Integration | ⚠️ Minimal | 10% |

### Detailed Status

#### Phase 1: Intent Infrastructure ✅
| File | Lines | Status |
|------|-------|--------|
| `intent/mod.rs` | 7 | ✅ |
| `intent/definition.rs` | 250 | ✅ All types implemented |
| `intent/registry.rs` | 128 | ✅ Tier priority works |

#### Phase 2: Built-in Intents ✅
| Intent | Lines | Status |
|--------|-------|--------|
| `login.rs` | 125 | ✅ |
| `search.rs` | 93 | ✅ |
| `accept_cookies.rs` | 80 | ✅ |
| `dismiss_popups.rs` | 84 | ✅ |
| `fill_form.rs` | 49 | ⚠️ Minimal implementation |
| `submit_form.rs` | 65 | ✅ |
| `scroll_to.rs` | 34 | ✅ |
| `logout.rs` | 104 | ✅ |

#### Phase 3: Execution Pipeline ⚠️
| Component | Status | Notes |
|-----------|--------|-------|
| PARSE/BIND | ✅ | `bind_parameters()` implemented |
| RESOLVE | ✅ | Registry lookup works |
| PLAN | ⚠️ | Basic scan only |
| EXECUTE - Action | ✅ | Click/Type work |
| EXECUTE - Branch | ✅ | Condition branching works |
| EXECUTE - Loop | ❌ | TODO stub |
| EXECUTE - Try | ❌ | TODO stub |
| VERIFY | ⚠️ | Only All/Any conditions work |
| RESPOND | ✅ | Returns IntentResult |

**Verifier gaps:** `PatternExists`, `Visible`, `Hidden`, `UrlContains`, `UrlMatches`, `TextContains`, `Count`, `Expression` all return placeholder values - need runtime state access.

#### Phase 4: Pattern-Intent Mapping ⚠️
| Component | Status |
|-----------|--------|
| `mapper.rs` | ✅ Maps 5 pattern types |
| `formatter.rs` integration | ❌ Missing `# available intents` section |

#### Phase 5: YAML Loading ✅
| Component | Status |
|-----------|--------|
| `loader.rs` | ✅ Loads from directory |
| `serde_yaml` dependency | ✅ Added |
| `glob` dependency | ✅ Added |

#### Tests ⚠️
| Area | Tests |
|------|-------|
| `registry.rs` | 1 (priority) |
| `definition.rs` | 0 |
| `executor.rs` | 0 |
| `verifier.rs` | 0 |

### Remaining Work

1. **Verifier implementation** - Add runtime state (ScanResult, URL) to evaluate conditions
2. **Loop/Try steps** - Implement iteration and error recovery in executor
3. **Formatter integration** - Add `# available intents` to observe output
4. **fill_form.rs** - Expand field matching logic
5. **Tests** - Add unit tests for executor, verifier, definition serde

---

## Module Structure

```
crates/oryn-core/src/
├── intent/
│   ├── mod.rs              # Module exports
│   ├── definition.rs       # IntentDefinition, Step, Parameter types
│   ├── registry.rs         # IntentRegistry with tier system
│   ├── executor.rs         # 6-stage pipeline execution
│   ├── verifier.rs         # Success/failure condition checking
│   ├── mapper.rs           # Pattern-to-intent availability
│   ├── loader.rs           # YAML loading (Phase 5)
│   └── builtin/
│       ├── mod.rs
│       ├── login.rs
│       ├── search.rs
│       ├── accept_cookies.rs
│       ├── dismiss_popups.rs
│       ├── fill_form.rs
│       ├── submit_form.rs
│       ├── scroll_to.rs
│       └── logout.rs
└── lib.rs                  # Add `pub mod intent;`
```

---

## Implementation Phases

### Phase 1: Intent Infrastructure

**Files to create:**
1. `intent/mod.rs` - Module exports
2. `intent/definition.rs` - Core types:
   - `IntentTier` (BuiltIn, Loaded, Discovered)
   - `IntentDefinition` (name, version, triggers, parameters, steps, success/failure, options)
   - `Step` enum (Action, Branch, Loop, Try, Checkpoint)
   - `ActionStep`, `ActionType`, `TargetSpec`
   - `ParameterDef`, `ParamType`
   - `IntentOptions` (timeout, retries, checkpoint)
3. `intent/registry.rs` - `IntentRegistry`:
   - `register_builtin()`, `get()`, `available_for_patterns()`
   - Tier-based priority ordering

### Phase 2: Built-in Intents

**Files to create** (`intent/builtin/`):

| Intent | Key Features |
|--------|--------------|
| `login.rs` | Find email/password/submit via pattern or heuristics, type credentials, click submit, verify URL change and form removal |
| `search.rs` | Clear existing, type query, submit via Enter or button, wait for results |
| `accept_cookies.rs` | Detect cookie banner pattern, click accept/reject, verify dismissal |
| `dismiss_popups.rs` | Iterate through modal/overlay/toast patterns, click close buttons, rescan (max 5 iterations) |
| `fill_form.rs` | Match data keys to fields by name/id/label, set values by field type |
| `submit_form.rs` | Find submit button in form, click, wait for navigation or response |
| `scroll_to.rs` | Scroll target element into viewport |
| `logout.rs` | Find logout link/button, click, verify session ended |

Each built-in provides `fn definition() -> IntentDefinition` returning full intent spec.

### Phase 3: Execution Pipeline

**File:** `intent/executor.rs`

Implement 6-stage pipeline:
1. **PARSE** - Validate intent name, bind parameters
2. **RESOLVE** - Look up definition from registry, check triggers satisfied
3. **PLAN** - Scan page, resolve `TargetSpec` → `Target::Id` using existing resolver
4. **EXECUTE** - Run steps sequentially:
   - Action steps → translate to Command → ScannerRequest → Backend
   - Branch steps → evaluate condition, choose path
   - Loop steps → iterate with max bound
   - Try steps → execute with fallback on error
5. **VERIFY** - Check success/failure conditions against final state
6. **RESPOND** - Build `IntentResult` with action log and changes

**File:** `intent/verifier.rs`

Implement condition checking:
- `PatternExists`, `PatternGone`
- `Visible`, `Hidden`
- `UrlContains`, `UrlMatches`
- `TextContains`
- `All`, `Any` (compound conditions)

### Phase 4: Pattern-Intent Mapping

**File:** `intent/mapper.rs`

Map detected patterns to intent availability:
- `LoginPattern` → `login` ready
- `SearchPattern` → `search` ready
- `CookieBannerPattern` → `accept_cookies` ready
- `ModalPattern` → `dismiss_popups` ready

**Modify:** `formatter.rs` to include `# available intents` section in observe output.

### Phase 5: YAML Loading (Optional)

**File:** `intent/loader.rs`

- Parse YAML intent definitions with serde_yaml
- Schema validation
- Load from directories for intent packs

---

## Key Data Structures

```rust
// intent/definition.rs

pub enum IntentTier { BuiltIn, Loaded, Discovered }

pub struct IntentDefinition {
    pub name: String,
    pub version: String,
    pub tier: IntentTier,
    pub triggers: IntentTriggers,
    pub parameters: Vec<ParameterDef>,
    pub steps: Vec<Step>,
    pub success: Option<SuccessCondition>,
    pub failure: Option<FailureCondition>,
    pub options: IntentOptions,
}

pub enum Step {
    Action(ActionStep),
    Branch { condition: Condition, then_steps: Vec<Step>, else_steps: Vec<Step> },
    Loop { over: String, as_var: String, steps: Vec<Step>, max: usize },
    Try { steps: Vec<Step>, catch: Vec<Step> },
    Checkpoint(String),
}

pub struct ActionStep {
    pub action: ActionType,
    pub target: Option<TargetSpec>,
    pub options: HashMap<String, Value>,
}

// intent/executor.rs

pub struct IntentResult {
    pub status: IntentStatus,  // Success, Failed, Partial
    pub actions: Vec<ActionLogEntry>,
    pub changes: Vec<ElementChange>,
    pub extracted: Option<Value>,
    pub checkpoint: Option<String>,
    pub hint: Option<String>,
}
```

---

## Integration Points

1. **Resolver** - Use `resolver::resolve_target()` for TargetSpec → Target::Id
2. **Translator** - Use `translator::translate()` for Command → ScannerRequest
3. **Backend** - Use `backend.execute_scanner()` for atomic operations
4. **Protocol** - Use `DetectedPatterns` for intent availability

---

## Dependencies to Add

```toml
# crates/oryn-core/Cargo.toml
serde_yaml = "0.9"
glob = "0.3"
```

---

## Verification

### Unit Tests
- `definition.rs` - Serde round-trip, defaults
- `registry.rs` - Tier priority, pattern matching
- `executor.rs` - Parameter expansion, step execution
- `verifier.rs` - All condition types

### Integration Tests
- Login with mock backend (success and failure paths)
- Form filling with field matching
- Cookie banner dismissal

### End-to-End
```bash
# Run existing tests
cargo test -p oryn-core

# Run with weston-headless (if available)
cargo test -p oryn-e --features headless
```

---

## Files to Modify

| File | Change | Status |
|------|--------|--------|
| `lib.rs` | Add `pub mod intent;` | ✅ Done |
| `formatter.rs` | Add intent response formatting, available intents section | ❌ Pending |
| `Cargo.toml` | Add `serde_yaml`, `glob` dependencies | ✅ Done |

## Files Created

| File | Purpose | Lines | Status |
|------|---------|-------|--------|
| `intent/mod.rs` | Module structure | 7 | ✅ |
| `intent/definition.rs` | Core types | 250 | ✅ |
| `intent/registry.rs` | Intent registry | 128 | ✅ |
| `intent/executor.rs` | 6-stage pipeline | 252 | ⚠️ Partial |
| `intent/verifier.rs` | Condition checking | 46 | ⚠️ Stubs |
| `intent/mapper.rs` | Pattern-intent mapping | 56 | ✅ |
| `intent/loader.rs` | YAML loading | 49 | ✅ |
| `intent/builtin/mod.rs` | Built-in exports | 22 | ✅ |
| `intent/builtin/login.rs` | Login intent | 125 | ✅ |
| `intent/builtin/search.rs` | Search intent | 93 | ✅ |
| `intent/builtin/accept_cookies.rs` | Cookie banner | 80 | ✅ |
| `intent/builtin/dismiss_popups.rs` | Popup dismissal | 84 | ✅ |
| `intent/builtin/fill_form.rs` | Form filling | 49 | ⚠️ Minimal |
| `intent/builtin/submit_form.rs` | Form submission | 65 | ✅ |
| `intent/builtin/scroll_to.rs` | Scroll to element | 34 | ✅ |
| `intent/builtin/logout.rs` | Logout | 104 | ✅ |

**Total:** 1,444 lines across 16 files
