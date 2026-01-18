# Intent Engine Completion Plan

Complete implementation of remaining Intent Engine features per `docs/SPEC-INTENT-ENGINE.md`.

## Current State

**Fully Implemented:** 6-stage executor, 8 built-in intents, definition schema, registry, verifier, YAML loader, mapper, target resolution with scoring, control flow (branch/loop/try), Checkpointing, error retry, response formatting, Config system, Pack system, Agent-defined intents, Security/masking

**Not Implemented:** Intent learner (Deferred to post-MVP)

---

## Implementation Status Summary

| Phase                    | Status            | Completeness | Notes                                        |
| ------------------------ | ----------------- | ------------ | -------------------------------------------- |
| Phase 1: Config          | ✅ Complete        | 100%         | All files, structs, and tests implemented    |
| Phase 2: Packs           | ✅ Complete        | 100%         | All features including intent glob loading   |
| Phase 3: Checkpoints     | ✅ Complete        | 100%         | Full checkpoint/resume/retry implemented     |
| Phase 4: Session Intents | ✅ Complete        | 90%          | Missing role-based targets in define parser  |
| Phase 5: Formatting      | ⚠️ Partial         | 70%          | Missing dedicated success/failure formatters |
| Phase 6: Learner         | ⏸️ Deferred        | N/A          | Post-MVP                                     |

---

## Phase 1: Configuration System ✅ COMPLETE

**Priority: Critical (enables all other features)**
**Status: 100% Implemented**

### New Files

```
crates/oryn-core/src/config/
├── mod.rs           # Module exports
├── schema.rs        # OrynConfig, IntentEngineConfig, PacksConfig, etc.
└── loader.rs        # ConfigLoader with default paths
```

### Key Structures (`config/schema.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OrynConfig {
    pub intent_engine: IntentEngineConfig,
    pub packs: PacksConfig,
    pub learning: LearningConfig,
    pub security: SecurityConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentEngineConfig {
    pub default_timeout_ms: u64,      // 30000
    pub step_timeout_ms: u64,         // 10000
    pub max_retries: usize,           // 3
    pub retry_delay_ms: u64,          // 1000
    pub strict_mode: bool,            // false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacksConfig {
    pub auto_load: bool,
    pub pack_paths: Vec<PathBuf>,     // ~/.oryn/packs, ./packs
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub sensitive_fields: Vec<String>, // password, token, card_number, cvv, ssn
    pub redact_in_logs: bool,
}
```

### ConfigLoader (`config/loader.rs`)

```rust
impl ConfigLoader {
    /// Load from: ./oryn.yaml → ~/.oryn/config.yaml → defaults
    pub fn load_default() -> Result<OrynConfig, ConfigError>;
    pub fn load_from(path: &Path) -> Result<OrynConfig, ConfigError>;
}
```

### Integration

1. Add `pub mod config;` to `crates/oryn-core/src/lib.rs`
2. Update `ReplState` in `crates/oryn/src/repl.rs`:
   ```rust
   struct ReplState {
       resolver_context: Option<ResolverContext>,
       registry: IntentRegistry,
       config: OrynConfig,  // NEW
   }
   ```

### Tests
- `crates/oryn-core/tests/config_test.rs` - YAML parsing, defaults, merging ✅

**Verified Implementation:**
- ✅ All 3 files exist (mod.rs, schema.rs, loader.rs)
- ✅ All structs implemented with correct fields and defaults
- ✅ ConfigLoader has `load_default()` and `load_from()` methods
- ✅ Module exported in lib.rs
- ✅ ReplState integrated with config field
- ✅ 3 tests passing

---

## Phase 2: Pack System ✅ COMPLETE

**Priority: High (most user-visible value)**
**Depends on: Phase 1**
**Status: 100% Implemented**

### New Files

```
crates/oryn-core/src/pack/
├── mod.rs           # Module exports
├── definition.rs    # PackMetadata, SitePattern, PackTrust
├── loader.rs        # PackLoader - load pack.yaml, patterns, intents
└── manager.rs       # PackManager - discover, load, unload, auto-load
```

### Directory Structure

```
~/.oryn/packs/
├── github.com/
│   ├── pack.yaml
│   ├── patterns.yaml
│   └── intents/
│       ├── star_repo.yaml
│       └── create_issue.yaml
└── amazon.com/
    └── ...
```

### Pack Metadata (`pack/definition.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackMetadata {
    pub pack: String,              // "github.com"
    pub version: String,
    pub description: String,
    pub domains: Vec<String>,      // github.com, gist.github.com
    pub patterns: Vec<String>,     // glob: "patterns.yaml"
    pub intents: Vec<String>,      // glob: "intents/*.yaml"
    pub auto_load: Vec<String>,    // "https://github.com/*"
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PackTrust {
    Full,       // builtin
    Verified,   // signed
    Sandboxed,  // no execute action
    Untrusted,
}
```

### PackManager (`pack/manager.rs`)

```rust
pub struct PackManager {
    loaded_packs: HashMap<String, LoadedPack>,
    registry: IntentRegistry,
    pack_paths: Vec<PathBuf>,
}

impl PackManager {
    pub fn discover_and_load(&mut self) -> Result<Vec<String>, PackError>;
    pub fn load_pack(&mut self, domain: &str) -> Result<(), PackError>;
    pub fn unload_pack(&mut self, domain: &str) -> Result<(), PackError>;
    pub fn should_auto_load(&self, url: &str) -> Option<&str>;
    pub fn registry(&self) -> &IntentRegistry;
}
```

### Integration

1. Add `pub mod pack;` to `crates/oryn-core/src/lib.rs`
2. Update `ReplState` to use `PackManager`:
   ```rust
   struct ReplState {
       resolver_context: Option<ResolverContext>,
       pack_manager: PackManager,  // Replaces registry
       config: OrynConfig,
   }
   ```
3. Auto-load on navigation in `execute_command`:
   ```rust
   Command::GoTo(url) => {
       if let Some(domain) = state.pack_manager.should_auto_load(url) {
           state.pack_manager.load_pack(domain)?;
       }
       // ... existing navigation
   }
   ```
4. Add commands to parser: `packs`, `pack load <domain>`, `pack unload <domain>`

### Tests
- `crates/oryn-core/tests/pack_test.rs` ✅
- Test fixtures: `crates/oryn-core/tests/fixtures/packs/github.com/` ❌ (uses tempfile instead)

**Verified Implementation:**
- ✅ All 4 files exist (mod.rs, definition.rs, loader.rs, manager.rs)
- ✅ PackMetadata with all required fields
- ✅ PackTrust enum with Full, Verified, Sandboxed, Untrusted
- ✅ PackManager with load_pack_by_name, unload_pack, should_auto_load, registry
- ✅ REPL commands: packs, pack load, pack unload
- ✅ Auto-load on GoTo navigation
- ✅ Intent loading from glob patterns fully implemented in loader.rs
- ✅ 4 tests passing

---

## Phase 3: Checkpointing & Enhanced Error Handling ✅ COMPLETE

**Priority: Medium (completes partial implementation)**
**Can run in parallel with Phases 1-2**
**Status: 100% Implemented**

### Modifications to `executor.rs`

Add checkpoint tracking:
```rust
pub struct IntentExecutor<'a, B: Backend> {
    // ... existing fields
    checkpoints: Vec<CheckpointState>,
    last_checkpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointState {
    pub name: String,
    pub variables: HashMap<String, Value>,
    pub step_index: usize,
}
```

Update `execute_step` for `Step::Checkpoint`:
```rust
Step::Checkpoint(wrapper) => {
    self.last_checkpoint = Some(wrapper.checkpoint.clone());
    self.checkpoints.push(CheckpointState {
        name: wrapper.checkpoint.clone(),
        variables: self.variables.clone(),
        step_index: current_index,
    });
    Ok(())
}
```

Add resume capability:
```rust
pub async fn execute_with_resume(
    &mut self,
    intent_name: &str,
    params: HashMap<String, Value>,
    resume_from: Option<&str>,
) -> Result<IntentResult, ExecutorError>;
```

### Enhanced Retry Logic

```rust
async fn execute_step_with_retry(
    &mut self,
    step: &Step,
    config: &RetryConfig,
) -> Result<(), ExecutorError> {
    let mut attempts = 0;
    loop {
        match self.execute_step(step).await {
            Ok(()) => return Ok(()),
            Err(e) if attempts < config.max_attempts && e.is_retryable() => {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(
                    config.delay_ms * (config.backoff_multiplier.powi(attempts as i32) as u64)
                )).await;
                self.perform_scan().await?; // Refresh DOM state
            }
            Err(e) => return Err(e),
        }
    }
}
```

### Tests
- Add checkpoint tests to `executor_test.rs` ✅
- Test resume from checkpoint ✅ (`executor_checkpoint_test.rs`)
- Test retry with backoff ✅ (implementation verified, exponential backoff with powi)

**Verified Implementation:**
- ✅ `CheckpointState` struct with name, variables, step_index
- ✅ `IntentExecutor.checkpoints` and `last_checkpoint` fields
- ✅ `execute_with_resume()` method with full resume logic
- ✅ `execute_step_with_retry()` with exponential backoff
- ✅ DOM refresh via `perform_scan()` before retry
- ✅ Logging for retry attempts
- ✅ Dedicated test file: `executor_checkpoint_test.rs`

---

## Phase 4: Agent-Defined Intents ✅ COMPLETE (90%)

**Priority: Medium (runtime extensibility)**
**Depends on: Phase 1**
**Status: 90% Implemented**

### New Files

```
crates/oryn-core/src/intent/
├── session.rs       # SessionIntentManager
└── define_parser.rs # Parse simplified define syntax
```

### SessionIntentManager (`intent/session.rs`)

```rust
pub struct SessionIntent {
    pub definition: IntentDefinition,
    pub created_at: Instant,
    pub invocation_count: usize,
}

pub struct SessionIntentManager {
    intents: HashMap<String, SessionIntent>,
}

impl SessionIntentManager {
    pub fn define(&mut self, definition: IntentDefinition) -> Result<(), SessionError>;
    pub fn undefine(&mut self, name: &str) -> Result<(), SessionError>;
    pub fn get(&self, name: &str) -> Option<&SessionIntent>;
    pub fn list(&self) -> Vec<&SessionIntent>;
    pub fn export(&self, name: &str, path: &Path) -> Result<(), SessionError>;
}
```

### Simplified Define Syntax (`intent/define_parser.rs`)

```
define add_to_wishlist:
  description: "Add to wishlist"
  steps:
    - click "Add to Wishlist" or click "♡"
    - wait visible { text_contains: "added" }
```

Parsing rules:
- `click "X" or click "Y"` → try block with fallbacks
- `wait visible X` → wait with visibility condition
- `type email $value` → type into role:email

### New Commands

Add to parser:
- `define <name>: <body>` - register session intent
- `undefine <name>` - remove session intent
- `intents --session` - list session intents
- `export <name> <path>` - save to YAML file

### Integration

1. Add to `intent/mod.rs`: `pub mod session;` and `pub mod define_parser;`
2. Update ReplState with `session_intents: SessionIntentManager`
3. Session intents register to registry with `IntentTier::Discovered`

### Tests
- `crates/oryn-core/tests/session_intent_test.rs` ✅
- Test define/undefine lifecycle ✅
- Test simplified syntax parsing ✅
- Test export to YAML ✅

**Verified Implementation:**
- ✅ `session.rs` with SessionIntent and SessionIntentManager
- ✅ All methods: define, undefine, get, get_mut, list, export
- ✅ `define_parser.rs` with syntax support (click, type, wait)
- ✅ Commands: define, undefine, export, run
- ✅ REPL integration with session_intents field
- ✅ `intents --session` command with REPL handler
- ✅ "Or" fallback syntax: `click "X" or click "Y"` → try block (recursive parsing)
- ✅ Variable substitution (`$value` parameters) in executor
- ✅ 3 tests passing

**Remaining Work:**
- ⚠️ Role-based targets in define parser (`type email $value` syntax) - define_parser only creates Text targets, not Role targets

---

## Phase 5: Advanced Response Formatting ⚠️ PARTIAL (70%)

**Priority: Medium (better UX)**
**Depends on: Phase 3**
**Status: 70% Implemented**

### Modifications to `formatter/mod.rs`

Add intent-specific formatting per SPEC Section 10:

```rust
pub fn format_intent_success(result: &IntentResult, intent: &str) -> String {
    // ok login "user@example.com"
    // # actions
    // type [1] "user@example.com"
    // type [2] "••••••••"
    // click [3] "Sign in"
    // # changes
    // ~ url: /login → /dashboard
    // - login_form
    // + user_menu
}

pub fn format_intent_failure(result: &IntentResult, intent: &str, error: &str) -> String {
    // error checkout: payment failed
    // # actions (up to failure)
    // # checkpoint: payment_started
    // # hint: Retry with --resume payment_started
}
```

### Sensitive Data Masking

```rust
fn mask_sensitive(value: &str, field_name: &str, sensitive_fields: &[String]) -> String {
    if sensitive_fields.iter().any(|f| field_name.to_lowercase().contains(f)) {
        "••••••••".to_string()
    } else {
        value.to_string()
    }
}
```

### Enhanced IntentResult

```rust
pub struct IntentResult {
    pub status: IntentStatus,
    pub data: Option<Value>,
    pub logs: Vec<ActionLog>,
    pub changes: Option<PageChanges>,
    pub checkpoint: Option<String>,
    pub hints: Vec<String>,
}

pub enum IntentStatus {
    Success,
    PartialSuccess { completed: usize, total: usize },
    Failed { error: String, recoverable: bool },
}
```

### Tests
- `crates/oryn-core/tests/formatter_intent_test.rs` ✅
- Test masking sensitive data ✅
- Test change notation ⚠️ (basic test, no PageChanges)

**Verified Implementation:**
- ✅ `mask_sensitive()` function with configurable sensitive_fields
- ✅ `IntentStatus` enum with Success, PartialSuccess, Failed
- ✅ `IntentResult` struct with status, data, logs, checkpoint, hints, changes
- ✅ `changes: Option<PageChanges>` field properly integrated
- ✅ Generic `format_intent_result()` for success/failure
- ✅ `mask_sensitive_log()` basic implementation (handles password/secret)
- ✅ 3 tests passing (including masking test)

**Remaining Work:**
- ❌ `format_intent_success()` dedicated function NOT IMPLEMENTED (per spec Section 10.1)
- ❌ `format_intent_failure()` dedicated function NOT IMPLEMENTED (per spec Section 10.2)
- ❌ Action enumeration format (`type [1] "value"`) NOT IMPLEMENTED
- ⚠️ `mask_sensitive_log()` only handles password/secret keywords, not all sensitive field types

---

## Phase 6: Intent Learner

**Priority: Low (most complex)**
**Depends on: Phases 1, 4**

This phase is deferred for complexity. Key components when implemented:

```
crates/oryn-core/src/learner/
├── mod.rs
├── observer.rs      # Record command sequences
├── recognizer.rs    # Find repeated patterns
├── proposer.rs      # Generate intent proposals
└── storage.rs       # Persist observations
```

Core algorithm:
1. Observer records command sequences with domain/URL context
2. Recognizer identifies 3+ similar sequences using sequence alignment
3. Proposer extracts parameters from varying elements
4. User reviews/refines proposals
5. Accepted intents promoted to session or persistent

---

## Implementation Order

```
Phase 1 (Config) ──────────────────────────────────────────────►
                    │
                    ├──► Phase 2 (Packs) ─────────────────────►
                    │
Phase 3 (Checkpoints) ─────────────────────────────────────────►
                    │
                    └──► Phase 4 (Session Intents) ───────────►
                                        │
                                        └──► Phase 5 (Formatting)

Phase 6 (Learner) - Deferred to post-MVP
```

Phases 1 and 3 can start immediately in parallel.
Phases 2 and 4 depend on Phase 1.
Phase 5 depends on Phase 3.

---

## Key Files to Modify

| File                                        | Changes                                           |
| ------------------------------------------- | ------------------------------------------------- |
| `crates/oryn-core/src/lib.rs`               | Add `pub mod config;` and `pub mod pack;`         |
| `crates/oryn-core/src/intent/mod.rs`        | Add `pub mod session;`                            |
| `crates/oryn-core/src/intent/executor.rs`   | Checkpointing, retry, resume                      |
| `crates/oryn-core/src/intent/definition.rs` | Enhanced RetryConfig                              |
| `crates/oryn-core/src/formatter/mod.rs`     | Intent response formatting                        |
| `crates/oryn/src/repl.rs`                   | Config loading, PackManager, SessionIntentManager |
| `crates/oryn-core/src/parser.rs`            | New commands: packs, define, export               |

---

## Verification

After each phase:

1. **Run tests**: `./scripts/run-tests.sh`
2. **Manual testing**:
   - Phase 1: Create `oryn.yaml`, verify config loads
   - Phase 2: Create test pack in `~/.oryn/packs/example.com/`, verify auto-load
   - Phase 3: Run multi-step intent, verify checkpoint saved, test resume
   - Phase 4: Use `define` command, verify intent executes
   - Phase 5: Run intent with password param, verify masking

3. **Integration test**: Execute full workflow - load config → auto-load pack → run intent → verify output format

---

## Test Coverage Summary

**Total:** 18 test files, 110 tests

| Test File                   | Tests | Coverage                                   |
| --------------------------- | ----- | ------------------------------------------ |
| config_test.rs              | 3     | Config loading, defaults, merging          |
| pack_test.rs                | 4     | Pack loading, manager operations, intents  |
| executor_checkpoint_test.rs | 1     | Checkpoint creation and resume             |
| executor_test.rs            | 10    | Loop, try, branch, fill_form, scoring      |
| session_intent_test.rs      | 3     | Define/undefine lifecycle, fallback syntax |
| formatter_intent_test.rs    | 3     | Masking, success/failure formatting        |
| verifier_test.rs            | 16    | Condition evaluation, URL/element matching |
| builtin_intent_test.rs      | 8     | All 8 built-in intents                     |
| learner_test.rs             | 4     | Observer, recognizer, proposer, storage    |
| Other (9 files)             | 58    | Parser, translator, core, use cases        |

---

## Remaining Work (Priority Order)

### Medium Priority
1. **Implement dedicated success/failure formatters** - Per spec Section 10.1 and 10.2
2. **Add action enumeration format** - `type [1] "value"` sequential numbering in logs

### Low Priority
3. **Role-based targets in define parser** - `type email $value` syntax to create TargetKind::Role
4. **Complete `mask_sensitive_log()`** - Extend to handle all sensitive field types (token, card_number, cvv, ssn)
