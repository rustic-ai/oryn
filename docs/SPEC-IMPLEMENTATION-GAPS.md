# Spec vs Implementation Gap Analysis

## Overview

This document identifies gaps between the **SPEC-INTENT-ENGINE.md** specification and the current oryn-core implementation.

**Analysis Date**: 2026-01-17
**Spec Version**: 1.0
**Implementation**: oryn-core crate

---

## Summary

| Category | Spec Sections | Implementation Status |
|----------|---------------|----------------------|
| Intent Tiers | §2 | ❌ Not Implemented |
| Intent Definition Format | §3 | ❌ Not Implemented |
| Built-in Intent Specifications | §4 | ⚠️ Partial (commands exist, not full intents) |
| Intent Execution Model | §5 | ❌ Not Implemented |
| Pattern-Intent Mapping | §6 | ⚠️ Partial (patterns exist, no mapping) |
| Agent-Defined Intents | §7 | ❌ Not Implemented |
| Intent Learning | §8 | ❌ Not Implemented |
| Site-Specific Packs | §9 | ❌ Not Implemented |
| Response Format | §10 | ⚠️ Partial (basic responses) |
| Configuration | §11 | ❌ Not Implemented |
| Extensibility | §12 | ❌ Not Implemented |

---

## Critical Gaps (High Priority)

### 1. Intent Tiers System (Spec §2)

**Spec Requires:**
- Tier 1: Built-in intents compiled into binary (login, logout, search, accept_cookies, dismiss_popups, scroll_to, fill_form, submit_form)
- Tier 2: Loaded intents from YAML/JSON files with priority ordering
- Tier 3: Discovered intents created by learning during operation

**Current State:**
- ❌ No tier system exists
- ❌ No intent registry
- ⚠️ Commands like `Login`, `Search` exist but are not full intent implementations

**Files Affected:**
- New: `crates/oryn-core/src/intent/mod.rs`
- New: `crates/oryn-core/src/intent/registry.rs`
- New: `crates/oryn-core/src/intent/builtin/`

---

### 2. Intent Definition Format (Spec §3)

**Spec Requires:**
```yaml
intent: <name>
version: <semver>
triggers:
  patterns: [<pattern names>]
  keywords: [<text triggers>]
  urls: [<URL patterns>]
parameters:
  - name: <name>
    type: string|number|boolean|object|array
    required: <boolean>
    default: <value>
steps:
  - action: click|type|select|wait|...
    target: <target specification>
success:
  conditions: [<success indicators>]
failure:
  recovery: [<recovery steps>]
options:
  timeout: <duration>
  retry: <configuration>
```

**Current State:**
- ❌ No YAML/JSON intent loading
- ❌ No intent schema validation
- ❌ No parameter binding system
- ❌ No success/failure conditions
- ❌ No step-based execution

**Files Affected:**
- New: `crates/oryn-core/src/intent/definition.rs`
- New: `crates/oryn-core/src/intent/loader.rs`
- New: `crates/oryn-core/src/intent/schema.rs`

---

### 3. Built-in Intent Specifications (Spec §4)

**Spec Requires 8 Built-in Intents:**

| Intent | Spec Status | Implementation |
|--------|-------------|----------------|
| `login <user> <pass>` | §4.1 | ⚠️ Command exists (`command.rs:37`), no verification/fallback |
| `logout` | Appendix A | ❌ Not implemented |
| `search <query>` | §4.2 | ⚠️ Command exists (`command.rs:38`), no clear/wait logic |
| `accept_cookies` | §4.3 | ⚠️ `Accept` command exists, no banner detection |
| `dismiss_popups` | §4.4 | ⚠️ `Dismiss` command exists, no iteration/rescan |
| `scroll_to <target>` | Appendix A | ⚠️ `Scroll` exists, no specific scroll_to |
| `fill_form <data>` | §4.5 | ❌ Not implemented |
| `submit_form` | §4.6 | ⚠️ `Submit` exists, no form identification |

**Detailed Gaps per Intent:**

#### login (§4.1)
- ❌ No heuristic fallback (find email/password/submit by patterns)
- ❌ No verification (URL change, login form removal)
- ❌ No `--no-submit` and `--wait` options
- ❌ No success/failure response format

#### search (§4.2)
- ❌ No `--submit` option (enter/click/auto)
- ❌ No clear existing content step
- ❌ No wait for results

#### accept_cookies (§4.3)
- ❌ No `--reject` option
- ❌ No cookie banner pattern detection
- ❌ No verification of banner dismissal

#### dismiss_popups (§4.4)
- ❌ No `--all` and `--type` options
- ❌ No iterative popup scanning (max 5 iterations)
- ❌ No categorization (modal, overlay, toast, banner)

#### fill_form (§4.5)
- ❌ Entire intent not implemented
- ❌ No form field matching by name/id/label
- ❌ No `--pattern` and `--partial` options

---

### 4. Intent Execution Model (Spec §5)

**Spec Requires 6-Stage Pipeline:**
```
PARSE → RESOLVE → PLAN → EXECUTE → VERIFY → RESPOND
```

**Current State:**
- ⚠️ Parser exists (`parser.rs`) - handles PARSE
- ⚠️ Resolver exists (`resolver.rs`) - handles target RESOLVE
- ❌ No PLAN stage (step expansion, condition evaluation)
- ⚠️ Translator exists (`translator.rs`) - partial EXECUTE
- ❌ No VERIFY stage (success/failure conditions)
- ⚠️ Basic responses - no standard RESPOND format

**Missing Components:**

#### Target Resolution (§5.2)
- ⚠️ Pattern reference resolution exists
- ⚠️ Role matching exists
- ⚠️ Text matching exists
- ⚠️ Selector matching exists
- ❌ No fallback chain traversal as specified

#### Error Handling (§5.3)
- ⚠️ Error mapping exists (`error_mapping.rs`)
- ❌ No step-level retry configuration
- ❌ No `on_error` handlers
- ❌ No rescan_and_retry behavior

#### Checkpointing (§5.4)
- ❌ Not implemented
- ❌ No checkpoint markers in execution
- ❌ No `--resume` option for long intents

---

### 5. Pattern-Intent Mapping (Spec §6)

**Spec Requires:**
- Intent availability evaluation based on detected patterns
- `# available intents` section in observe output
- Intent suggestions based on page context

**Current State:**
- ⚠️ Patterns detected (`protocol.rs:157-182`): LoginPattern, SearchPattern, PaginationPattern, ModalPattern, CookieBannerPattern
- ❌ No mapping from patterns → available intents
- ❌ No intent suggestions in responses
- ❌ No `intents --list` or availability queries

**Files Affected:**
- Modify: `crates/oryn-core/src/protocol.rs`
- New: `crates/oryn-core/src/intent/availability.rs`

---

## Medium Priority Gaps

### 6. Agent-Defined Intents (Spec §7)

**Spec Requires:**
```
define <name>:
  description: "..."
  steps:
    - click "Button"
    - wait visible toast
```

**Current State:**
- ❌ No `define` command
- ❌ No session intent storage
- ❌ No `undefine`, `intents --session`, `export` commands
- ❌ No simplified step syntax parser

---

### 7. Intent Learning (Spec §8)

**Spec Requires:**
- Intent Learner observing command sequences
- Pattern recognition across sessions
- Intent proposals with confidence scores
- Refinement interface

**Current State:**
- ❌ Not implemented (entire subsystem)

---

### 8. Site-Specific Intent Packs (Spec §9)

**Spec Requires:**
```
intent-packs/
├── github.com/
│   ├── pack.yaml
│   ├── patterns.yaml
│   └── intents/
```

**Current State:**
- ❌ No pack loading system
- ❌ No pack metadata (version, domains, auto_load)
- ❌ No `packs`, `pack load/unload/install` commands
- ❌ No URL-based auto-loading

---

### 9. Response Format (Spec §10)

**Spec Requires:**
```
ok <intent> [<summary>]

# actions
<action log>

# changes
<change notation>

# result
<extracted data>
```

**Current State:**
- ⚠️ `ScannerProtocolResponse` exists with Ok/Error variants
- ❌ No action log in responses
- ❌ No changes notation (+ added, - removed, ~ modified)
- ❌ No checkpoint reporting on failure
- ❌ No `partial` response type

**Files Affected:**
- Modify: `crates/oryn-core/src/protocol.rs`
- Modify: `crates/oryn-core/src/formatter.rs`

---

### 10. Configuration System (Spec §11)

**Spec Requires:**
```yaml
intent_engine:
  resolution:
    tier_priority: [user, pack, core, builtin]
  execution:
    default_timeout: 30s
    max_retries: 3
  verification:
    verify_success: true
  learning:
    enabled: true
```

**Current State:**
- ❌ No configuration file loading
- ❌ No `config show/set/reset` commands
- ❌ No per-intent option overrides

---

### 11. Extensibility (Spec §12)

**Spec Requires:**
- Custom step actions (pack-defined)
- Custom conditions
- Hooks (before_intent, after_navigate, on_error)

**Current State:**
- ❌ Not implemented

---

## Low Priority Gaps

### 12. Future Features (Spec §14)

These are explicitly marked as "Future Directions" in the spec:
- Goal-level commands (natural language goals)
- Multi-page flows
- Collaborative learning
- Intent composition

---

## What IS Implemented

### Parser (`parser.rs` - 1,014 lines)
✅ Tokenization with quotes, flags, parentheses
✅ Forgiving command syntax (goto/navigate/visit)
✅ Target parsing (ID, text, role, selector, relational)
✅ Option extraction

### Resolver (`resolver.rs` - 1,556 lines)
✅ Text matching with scoring (exact: 100, partial: 50, etc.)
✅ Role matching with input type awareness
✅ Relational resolution (near, inside, after, before, contains)
✅ Strategy modes (First, Unique, Best)

### Translator (`translator.rs` - 328 lines)
✅ Command → ScannerRequest translation
✅ Option parsing (force, double, append, delay)
✅ JavaScript execution for storage/text operations

### Error Mapping (`error_mapping.rs` - 290 lines)
✅ Protocol error code mapping
✅ Recovery hints for each error type
✅ Element ID extraction from details

### Protocol (`protocol.rs` - 423 lines)
✅ Request/Response serialization
✅ Pattern detection structures
✅ Element with full metadata

### Commands (`command.rs` - 172 lines)
✅ 30+ commands defined
✅ Sophisticated Target enum
✅ WaitCondition and ExtractSource

---

## Implementation Roadmap

### Phase 1: Intent Infrastructure
1. Create intent registry with tier system
2. Implement YAML intent loader with schema validation
3. Add intent executor with step-based execution

### Phase 2: Built-in Intents
1. Implement full `login` intent with verification
2. Implement `fill_form` intent
3. Implement `accept_cookies` with banner detection
4. Add remaining built-in intents

### Phase 3: Response Enhancement
1. Add action logging to responses
2. Implement changes notation
3. Add partial success response type

### Phase 4: Configuration & Packs
1. Add configuration file loading
2. Implement pack loading system
3. Add URL-based auto-loading

### Phase 5: Learning (Optional)
1. Command sequence observation
2. Pattern recognition
3. Intent proposal system

---

## Appendix: File Reference

| Spec Section | Implementation File | Status |
|-------------|---------------------|--------|
| §2 Intent Tiers | *Not exists* | ❌ |
| §3 Definition Format | *Not exists* | ❌ |
| §4 Built-in Intents | `command.rs` (partial) | ⚠️ |
| §5 Execution Model | `parser.rs`, `resolver.rs`, `translator.rs` | ⚠️ |
| §6 Pattern Mapping | `protocol.rs` (patterns only) | ⚠️ |
| §7 Agent-Defined | *Not exists* | ❌ |
| §8 Learning | *Not exists* | ❌ |
| §9 Packs | *Not exists* | ❌ |
| §10 Response Format | `protocol.rs`, `formatter.rs` | ⚠️ |
| §11 Configuration | *Not exists* | ❌ |
| §12 Extensibility | *Not exists* | ❌ |
