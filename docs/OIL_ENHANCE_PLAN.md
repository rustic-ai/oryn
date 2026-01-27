# Oryn Improvement Implementation Plan

## Overview

**Timeline**: 2-3 weeks (aggressive)
**Scope**: Phase 0 (Quick Wins) + Phase 1 (P0 Priorities)
**Testing**: Full test suite + E2E quick mode (oryn-h) between phases

This plan focuses on maximum impact improvements with minimum risk, leveraging the finding that **scanner.js already provides rich data** that the Rust formatter currently discards.

---

## Phase 0: Quick Wins (Week 1: Days 1-3)

**Goal**: Surface existing scanner data through better formatting - zero scanner changes.

**Complexity**: Low | **Impact**: High | **Risk**: Very Low

### 0.1 Action Confirmation with Context

**Files**:
- `crates/oryn-common/src/protocol.rs` - Extend ActionResult
- `crates/oryn-engine/src/formatter/mod.rs` - Format rich responses

**Current**: Actions return `"Action Result: success=true, msg=..."` (line 87-89 in formatter)

**Scanner provides** (scanner.js lines 1055-1064, 1175-1182):
- `navigation: bool` - Did action cause page navigation?
- `dom_changes: {added, removed, attributes}` - Mutation counts
- `value: string` - Final value after typing
- `coordinates: {x, y}` - Click position

**Implementation**:

1. **Extend ActionResult** in `protocol.rs` after line 540:
```rust
pub struct ActionResult {
    pub success: bool,
    pub message: Option<String>,
    pub navigation: Option<bool>,  // Already exists
    pub dom_changes: Option<DomChanges>,  // NEW
    pub value: Option<String>,  // NEW
    pub coordinates: Option<Coordinates>,  // NEW
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomChanges {
    pub added: usize,
    pub removed: usize,
    pub attributes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coordinates {
    pub x: i32,
    pub y: i32,
}
```

2. **Update formatter** (replace lines 87-89 in `formatter/mod.rs`):
```rust
ScannerData::Action(a) => {
    let mut output = format!("ok {}\n",
        a.message.as_deref().unwrap_or("action"));

    if let Some(true) = a.navigation {
        output.push_str("\n# navigation detected\n");
    }

    if let Some(changes) = &a.dom_changes {
        if changes.added > 0 || changes.removed > 0 {
            output.push_str(&format!("\n# changes: +{} -{} elements\n",
                changes.added, changes.removed));
        }
    }

    if let Some(value) = &a.value {
        output.push_str(&format!("\n# value: {:?}\n", value));
    }

    output
}
```

**Expected Output**:
```
type 1 "user@example.com"

ok type
# value: "user@example.com"
```

```
click "Login"

ok click
# navigation detected
# changes: +15 -5 elements
```

**Testing**: Run existing E2E tests - should see richer output, same behavior.

---

### 0.2 Show Input Values in Observations

**Files**: `crates/oryn-engine/src/formatter/mod.rs`

**Current**: Element values not displayed (lines 21-56)

**Scanner provides**: Element.value in serialization (already in protocol.rs line 491)

**Implementation**: In `format_response` Scan branch (around line 52):

```rust
let label = el.text.clone().or(el.label.clone()).unwrap_or_default();

// NEW: Append value if present
let value_suffix = if let Some(ref val) = el.value {
    if !val.is_empty() {
        let display_val = mask_sensitive(val, &el.element_type, &[]);
        format!(" = {:?}", display_val)
    } else {
        String::new()
    }
} else if el.element_type == "checkbox" || el.element_type == "radio" {
    // Show checked state as value
    if el.state.checked {
        " = checked".to_string()
    } else {
        String::new()
    }
} else {
    String::new()
};

output.push_str(&format!(
    "[{}] {} {:?}{}{}\n",
    el.id, type_str, label, flags_str, value_suffix
));
```

**Expected Output**:
```
observe

@ example.com "Login"
[1] input/email "Email" {required} = "user@example.com"
[2] input/password "Password" {required} = "••••••••"
[3] checkbox "Remember me" = checked
[4] button "Sign in"
```

**Testing**: Scan forms with filled inputs, verify values display correctly and passwords masked.

---

### 0.3 Include Position Data (--full flag)

**Files**:
- `crates/oryn-core/src/oil.pest` - Add --full flag to observe_cmd
- `crates/oryn-core/src/parser.rs` - Parse --full flag
- `crates/oryn-common/src/protocol.rs` - Add full_mode to ScanRequest
- `crates/oryn-engine/src/formatter/mod.rs` - Display position when enabled

**Current**: Element.rect exists (protocol.rs line 497) but not displayed

**Implementation**:

1. Add to ScanRequest in protocol.rs (around line 115):
```rust
pub struct ScanRequest {
    // ... existing fields ...
    #[serde(default)]
    pub full_mode: bool,  // NEW
}
```

2. Parse --full in parser.rs
3. Format with position in formatter.rs:
```rust
// In format_response for Scan
if scan.full_mode {  // Access from request context
    output.push_str(&format!(
        "[{}] {} {:?} @ ({:.0},{:.0}) {}x{}{}{}\n",
        el.id, type_str, label,
        el.rect.x, el.rect.y, el.rect.width, el.rect.height,
        flags_str, value_suffix
    ));
} else {
    // Current compact format
}
```

**Expected Output**:
```
observe --full

@ example.com "Login"
[1] input/email "Email" @ (100,50) 300x40 {required} = "user@example.com"
[2] button "Submit" @ (100,100) 100x30
```

**Testing**: Compare positions with browser viewport, verify off-screen elements.

---

## Phase 0 Testing Checkpoint

**Before proceeding to Phase 1**:
- [ ] Run `./scripts/run-tests.sh` - all tests pass
- [ ] Run `./scripts/run-e2e-tests.sh --quick` - oryn-h E2E passes
- [ ] Manual: Verify sensitive field masking works
- [ ] Manual: Check action confirmations on test-harness pages
- [ ] Manual: Verify value display on forms

---

## Phase 1: P0 Priorities (Week 1-2: Days 4-10)

**Goal**: Implement high-impact improvements requiring scanner + formatter changes.

**Complexity**: Medium | **Impact**: Very High | **Risk**: Medium

### 1.1 Diff-Mode Observations (FAST-TRACKED)

**Priority**: P0 | **Complexity**: Medium | **Impact**: Very High

**Files**:
- `crates/oryn-scanner/src/scanner.js` - Enable comprehensive change tracking
- `crates/oryn-common/src/protocol.rs` - Ensure ElementChange/ChangeType complete
- `crates/oryn-engine/src/formatter/mod.rs` - Format change deltas
- `crates/oryn-core/src/oil.pest` - Add --diff flag to observe_cmd

**Architecture**:
```
observe --diff
   ↓
ScanRequest { monitor_changes: true }
   ↓
Scanner: diffElements() + cache → find changes
   ↓
ScanResult { changes: Some(vec![...]) }
   ↓
Formatter: Display +/-/~ notation
```

**Key Insight**: Protocol already has `changes: Option<Vec<ElementChange>>` (protocol.rs line 331), scanner has `diffElements()` (scanner.js lines 824-860), just need to wire it up!

**Implementation**:

**1.1.1 Verify ChangeType enum** in protocol.rs (around line 447-456):
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    Appeared,
    Disappeared,
    TextChanged,
    StateChanged,
    PositionChanged,
}
```

**1.1.2 Enhance scanner.js** (in scan function around line 760-820):
```javascript
// After collecting elements, if monitor_changes enabled
if (monitorChanges) {
    const changes = [];
    const currentIds = new Set();

    // Check all current elements
    for (const serialized of allSerializedElements) {
        currentIds.add(serialized.id);
        const cached = STATE.cache.get(serialized.id);

        if (!cached) {
            // New element appeared
            changes.push({
                id: serialized.id,
                change_type: 'appeared',
                new_value: serialized.text || serialized.label || null
            });
        } else {
            // Use existing diffElements function
            const textChanged = cached.text !== serialized.text;
            const stateChanged = JSON.stringify(cached.state) !== JSON.stringify(serialized.state);

            if (textChanged) {
                changes.push({
                    id: serialized.id,
                    change_type: 'text_changed',
                    old_value: cached.text,
                    new_value: serialized.text
                });
            }

            if (stateChanged) {
                changes.push({
                    id: serialized.id,
                    change_type: 'state_changed',
                    old_value: JSON.stringify(cached.state),
                    new_value: JSON.stringify(serialized.state)
                });
            }
        }
    }

    // Check for disappeared elements
    for (const [id, cached] of STATE.cache.entries()) {
        if (!currentIds.has(id)) {
            changes.push({
                id: id,
                change_type: 'disappeared',
                old_value: cached.text || cached.label || null
            });
        }
    }

    result.changes = changes;

    // Update cache with current state
    for (const serialized of allSerializedElements) {
        STATE.cache.set(serialized.id, serialized);
    }
}
```

**1.1.3 Format changes** in formatter.rs (in Scan branch around line 82):
```rust
// After patterns section
if let Some(changes) = &scan.changes {
    if !changes.is_empty() {
        output.push_str("\n# changes\n");
        for change in changes {
            match change.change_type {
                ChangeType::Appeared => {
                    output.push_str(&format!("+ [{}] appeared: {:?}\n",
                        change.id,
                        change.new_value.as_deref().unwrap_or("")));
                }
                ChangeType::Disappeared => {
                    output.push_str(&format!("- [{}] disappeared: {:?}\n",
                        change.id,
                        change.old_value.as_deref().unwrap_or("")));
                }
                ChangeType::TextChanged => {
                    output.push_str(&format!("~ [{}] text: {:?} → {:?}\n",
                        change.id,
                        change.old_value.as_deref().unwrap_or(""),
                        change.new_value.as_deref().unwrap_or("")));
                }
                ChangeType::StateChanged => {
                    output.push_str(&format!("~ [{}] state changed\n", change.id));
                }
                ChangeType::PositionChanged => {
                    output.push_str(&format!("~ [{}] moved\n", change.id));
                }
            }
        }
    }
}
```

**1.1.4 Wire --diff flag** through parser and translator to set `monitor_changes: true`

**Expected Output**:
```
click "Add to Cart"

ok click
# changes: +2 -0 elements

observe --diff

@ example.com "Products"

# changes
+ [51] appeared: "Added to cart"
~ [12] text: "Add to Cart" → "Added ✓"
~ [12] state changed
- [8] disappeared: "Sale Badge"

# elements
[1] ...
```

**Testing**:
- E2E: Click action that shows modal → verify modal elements appear
- E2E: Form validation → verify error messages appear
- E2E: Product add-to-cart → verify cart badge updates
- Unit: diffElements function with various element states

**CRITICAL**: After changes, run `./scripts/sync-scanner.sh` to sync to extension variants!

---

### 1.2 Smart Error Hints

**Priority**: P0 | **Complexity**: Medium | **Impact**: High

**Files**:
- `crates/oryn-common/src/resolver.rs` - Add fuzzy matching
- `crates/oryn-engine/src/executor.rs` - Enrich error messages
- `crates/oryn-scanner/src/scanner.js` - Add position context to errors

**Current**: Errors minimal (executor.rs lines 78-84)

**Implementation**:

**1.2.1 Add fuzzy matching** to resolver.rs:
```rust
// New public function
pub fn find_similar_elements(
    target_text: &str,
    elements: &[Element],
    limit: usize,
) -> Vec<(u32, String, f32)> {
    let mut scores = vec![];
    let normalized_target = normalize_text(target_text);

    for elem in elements {
        // Check text field
        if let Some(ref text) = elem.text {
            let score = string_similarity(&normalized_target, &normalize_text(text));
            if score > 0.5 {
                scores.push((elem.id, text.clone(), score));
            }
        }

        // Also check label
        if let Some(ref label) = elem.label {
            let score = string_similarity(&normalized_target, &normalize_text(label));
            if score > 0.5 {
                scores.push((elem.id, label.clone(), score));
            }
        }
    }

    scores.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    scores.truncate(limit);
    scores
}

fn string_similarity(a: &str, b: &str) -> f32 {
    // Longest Common Subsequence ratio
    let lcs_len = lcs_length(a.as_bytes(), b.as_bytes());
    (2.0 * lcs_len as f32) / (a.len() + b.len()) as f32
}

fn lcs_length(a: &[u8], b: &[u8]) -> usize {
    let m = a.len();
    let n = b.len();
    let mut dp = vec![vec![0; n + 1]; m + 1];

    for i in 1..=m {
        for j in 1..=n {
            if a[i - 1] == b[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }
    dp[m][n]
}
```

**1.2.2 Enhance error checking** in executor.rs (replace `check_scanner_error` around line 78):
```rust
fn check_scanner_error(
    &self,
    resp: &ScannerProtocolResponse,
    original_target: Option<&str>,
) -> Result<(), ExecutorError> {
    if let ScannerProtocolResponse::Error { code, message, details, .. } = resp {
        let enriched = match code.as_str() {
            "ELEMENT_NOT_FOUND" => {
                if let (Some(target), Some(scan)) = (original_target, &self.last_scan) {
                    let similar = find_similar_elements(target, &scan.elements, 3);
                    if !similar.is_empty() {
                        format!("{}\n\nSimilar elements:\n{}",
                            message,
                            similar.iter().map(|(id, text, score)|
                                format!("  [{}] {:?} ({:.0}% match)", id, text, score * 100.0)
                            ).collect::<Vec<_>>().join("\n")
                        )
                    } else {
                        message.clone()
                    }
                } else {
                    message.clone()
                }
            }
            "ELEMENT_NOT_VISIBLE" => {
                // Extract position from details if available
                if let Some(details) = details {
                    if let Some(rect) = details.get("rect") {
                        format!("{}\n\nElement position: {}\n\nHint: Try 'scroll' or add '--force' flag",
                            message, rect)
                    } else {
                        message.clone()
                    }
                } else {
                    message.clone()
                }
            }
            _ => message.clone()
        };
        Err(ExecutorError::Scanner(enriched))
    } else {
        Ok(())
    }
}
```

**1.2.3 Scanner visibility error enhancement** (scanner.js around line 994):
```javascript
if (!params.force && !Utils.isVisible(el)) {
    const rect = el.getBoundingClientRect();
    throw {
        msg: `Element ${params.id} is not visible`,
        code: 'ELEMENT_NOT_VISIBLE',
        details: {
            rect: {
                x: Math.round(rect.x),
                y: Math.round(rect.y),
                width: Math.round(rect.width),
                height: Math.round(rect.height)
            },
            viewport: {
                width: window.innerWidth,
                height: window.innerHeight
            }
        }
    };
}
```

**Expected Output**:
```
click "Add to Crat"

error: element not found

Similar elements:
  [12] "Add to Cart" (90% match)
  [14] "Add to Wishlist" (60% match)

Hint: Try 'click 12' or run 'observe' to refresh
```

**Testing**:
- Unit: String similarity with various typos
- E2E: Deliberate typo in element text
- E2E: Click off-screen element without --force

**CRITICAL**: After changes, run `./scripts/sync-scanner.sh`!

---

### 1.3 Pattern Confidence Scores

**Priority**: P1 | **Complexity**: Low | **Impact**: Medium

**Files**:
- `crates/oryn-common/src/protocol.rs` - Add confidence field to patterns
- `crates/oryn-scanner/src/scanner.js` - Calculate confidence scores
- `crates/oryn-engine/src/formatter/mod.rs` - Display confidence

**Implementation**:

**1.3.1 Add confidence** to LoginPattern in protocol.rs (around line 372):
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginPattern {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<u32>,
    pub password: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remember: Option<u32>,
    #[serde(default)]
    pub confidence: f32,  // NEW: 0.0-1.0
}
```

**1.3.2 Calculate confidence** in scanner.js Patterns module:
```javascript
// In detectLoginForm function
const confidence = calculateLoginConfidence({
    hasPassword: !!password,
    hasEmail: !!email,
    hasUsername: !!username,
    hasSubmit: !!submit,
    isInForm: /* check if in <form> tag */
});

function calculateLoginConfidence(signals) {
    let score = 0.5; // Base for having password
    if (signals.hasEmail || signals.hasUsername) score += 0.2;
    if (signals.hasSubmit) score += 0.15;
    if (signals.isInForm) score += 0.15;
    return Math.min(score, 1.0);
}
```

**1.3.3 Format confidence** in formatter.rs (around line 60):
```rust
if let Some(login) = &patterns.login {
    output.push_str(&format!("- Login Form ({:.0}% confidence)\n",
        login.confidence * 100.0));
    if login.confidence < 0.7 {
        output.push_str("  Note: Unusual structure, verify before use\n");
    }
}
```

**Testing**: Test on various login forms (standard vs. non-standard layouts)

---

## Phase 1 Testing Checkpoint

**Before declaring completion**:
- [ ] Run `./scripts/run-tests.sh` - all tests pass
- [ ] Run `./scripts/run-e2e-tests.sh --quick` - oryn-h E2E passes
- [ ] Manual: Verify diff-mode on product page, dashboard
- [ ] Manual: Test fuzzy matching with typos
- [ ] Manual: Check pattern confidence on test-harness login page
- [ ] Verify scanner.js synced: `./scripts/check-scanner-sync.sh`

---

## Critical Files Summary

| File | Changes | Phase |
|------|---------|-------|
| `crates/oryn-common/src/protocol.rs` | Extend ActionResult, add DomChanges/Coordinates, add confidence to patterns | 0, 1 |
| `crates/oryn-engine/src/formatter/mod.rs` | Format action context, values, changes, confidence | 0, 1 |
| `crates/oryn-scanner/src/scanner.js` | Enhance change tracking, error context, pattern confidence | 1 |
| `crates/oryn-common/src/resolver.rs` | Add fuzzy matching for smart hints | 1 |
| `crates/oryn-engine/src/executor.rs` | Enrich error messages with context | 1 |
| `crates/oryn-core/src/oil.pest` | Add --full, --diff flags | 0, 1 |
| `crates/oryn-core/src/parser.rs` | Parse new flags | 0, 1 |

---

## Verification & Testing

### After Phase 0
```bash
# Format, lint, test
./scripts/run-tests.sh

# Quick E2E (oryn-h only)
./scripts/run-e2e-tests.sh --quick

# Manual validation
# 1. Navigate to test-harness login page
# 2. Run: observe
# 3. Verify values shown: [1] input/email "Email" = "..."
# 4. Run: type 1 "test@example.com"
# 5. Verify output shows: # value: "test@example.com"
```

### After Phase 1
```bash
# Full test suite
./scripts/run-tests.sh

# Quick E2E
./scripts/run-e2e-tests.sh --quick

# Verify scanner sync
./scripts/check-scanner-sync.sh

# Manual validation
# 1. Navigate to test-harness product page
# 2. Run: observe --diff
# 3. Run: click "Add to Cart"
# 4. Run: observe --diff
# 5. Verify changes shown: + [51] appeared: "Added to cart"

# Test fuzzy matching
# 1. Run: click "Submt" (typo)
# 2. Verify output suggests: [3] "Submit" (90% match)
```

---

## Success Criteria

### Phase 0 Complete When:
- [x] Action responses show navigation status
- [x] Action responses show DOM change counts
- [x] Type actions show final input value
- [x] Input elements display current values
- [x] Checkboxes show "= checked"
- [x] Passwords masked with "••••••••"
- [x] observe --full shows position data
- [x] All existing tests pass
- [x] No behavioral changes, only richer output

### Phase 1 Complete When:
- [x] observe --diff shows element changes (+/-/~)
- [x] Diff mode reduces observe output by 70%+
- [x] Typos in element names suggest corrections
- [x] Visibility errors include position hints
- [x] Pattern detection shows confidence scores
- [x] All tests pass including E2E
- [x] scanner.js synchronized across variants

---

## Risk Mitigation

### Low Risk (Phase 0)
- Pure formatting changes
- No scanner.js modifications
- Easy to rollback

### Medium Risk (Phase 1)
- **Diff-mode**: Cache management complexity
  - Mitigation: Thorough E2E testing, monitor for stale state
- **Fuzzy matching**: Performance on large element lists
  - Mitigation: Limit to 3 suggestions, early cutoff at 50% threshold
- **Scanner changes**: Must sync across all variants
  - Mitigation: Use sync script, verify with check script

---

## Timeline Breakdown

**Week 1**:
- Days 1-2: Phase 0.1 + 0.2 (Action context, value display)
- Day 3: Phase 0.3 (Position data) + Testing checkpoint
- Days 4-5: Phase 1.1 (Diff-mode) - scanner changes

**Week 2**:
- Days 1-2: Phase 1.1 continued (formatter, testing)
- Days 3-4: Phase 1.2 (Smart hints)
- Day 5: Phase 1.3 (Pattern confidence) + Testing checkpoint

**Week 3** (buffer):
- Days 1-2: Bug fixes, polish, edge cases
- Days 3-5: Documentation, additional testing

---

## Post-Implementation

### Documentation Updates Needed
- Update OIL command reference with --full, --diff flags
- Document new response formats with examples
- Add troubleshooting guide for error hints

### Future Phases (Deferred)
- Phase 2: Observation improvements (2.2 interactability only, no hierarchical grouping)
- Phase 3: Protocol enhancements (batch commands, implicit waits)
- Phase 4: New capabilities (optional)

These can be tackled in a follow-up project once Phase 0-1 are proven stable in production.
