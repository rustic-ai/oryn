# Feature Gap Analysis

> **Generated**: 2026-01-15
> **Overall Implementation Status**: ~60-65% Complete

This document identifies gaps between documented features and actual implementation in Lemmascope.

---

## Executive Summary

Lemmascope has solid foundational architecture with working binaries for all three modes (Headless, Embedded, Remote). However, critical pieces are missing that prevent agents from using the full feature set described in documentation.

**Key Blockers:**
1. Missing semantic target resolution layer
2. Translator only handles 8 of 20+ parsed commands
3. Build errors in backend.rs and translator.rs

---

## Critical Gap: Semantic Target Resolution

### The Problem

The Intent Language is designed for natural agent interaction:
```
click "Sign In"
type "Email" "user@example.com"
select "Size" "Large"
```

**Current Reality**: Only numeric ID targeting works:
```
click 3
type 5 "user@example.com"
select 7 "Large"
```

### Root Cause

The parser correctly creates semantic targets:
```rust
Target::Text("Sign In")
Target::Role("email")
Target::Near(box Target::Text("Password"))
```

But the translator **only accepts** `Target::Id(n)` and rejects all others.

### Missing Component

SPEC-UNIFIED.md (section 3.1) describes a "Semantic Resolver" layer that should:
1. Take semantic targets from parsed commands
2. Query the scanner's element inventory
3. Resolve to concrete numeric IDs
4. Handle ambiguity (multiple matches)

**This layer does not exist in the codebase.**

### Impact

- Agents cannot use natural language targeting
- Defeats the core value proposition of intent-based automation
- Forces agents to: `observe` → parse response → extract IDs → issue commands with IDs

---

## Gap: Untranslated Commands

### Translator Coverage

| Status | Commands |
|--------|----------|
| ✅ Translated | `observe`, `click`, `type`, `submit`, `scroll`, `wait`, `storage`, `execute` |
| ❌ Parsed but not translated | See table below |

### Missing Command Translations

| Command | Parser Support | Translator Support | Notes |
|---------|---------------|-------------------|-------|
| `goto` | ✅ | ❌ | Navigation - critical |
| `html` | ✅ | ❌ | Content extraction |
| `text` | ✅ | ❌ | Content extraction |
| `title` | ✅ | ❌ | Page info |
| `url` | ✅ | ❌ | Page info |
| `refresh` | ✅ | ❌ | Navigation |
| `back` | ✅ | ❌ | Navigation |
| `forward` | ✅ | ❌ | Navigation |
| `press` | ✅ | ❌ | Keyboard input |
| `hover` | ✅ | ❌ | Mouse interaction |
| `focus` | ✅ | ❌ | Element focus |
| `clear` | ✅ | ❌ | Input clearing |
| `check` | ✅ | ❌ | Checkbox/radio |
| `uncheck` | ✅ | ❌ | Checkbox |
| `select` | ✅ | ⚠️ Partial | Missing option resolution |
| `extract` | ✅ | ❌ | Data extraction |
| `cookies` | ✅ | ❌ | Cookie management |
| `tabs` | ✅ | ❌ | Tab/window management |
| `screenshot` | ✅ | ❌ | Via backend trait only |
| `pdf` | ✅ | ❌ | Headless-specific |
| `login` | ✅ | ❌ | High-level goal |
| `search` | ✅ | ❌ | High-level goal |
| `dismiss` | ✅ | ❌ | Modal/banner handling |
| `accept` | ✅ | ❌ | Dialog handling |
| `scroll_until` | ✅ | ❌ | Conditional scrolling |

### Translator Bottleneck Code

Location: `lscope-core/src/translator.rs:159`
```rust
_ => Err(TranslationError::Unsupported(format!("{:?}", command))),
```

This catch-all silently rejects most commands.

---

## Gap: Cross-Mode Feature Parity

### Screenshot Support

| Mode | Status | Code Location |
|------|--------|---------------|
| Headless | ✅ Full implementation | Uses chromiumoxide |
| Embedded | ❌ Returns `NotSupported` | backend.rs:109 |
| Remote | ❌ Returns `NotSupported` | "Not implemented yet" |

### PDF Generation

| Mode | Status |
|------|--------|
| Headless | ✅ Works via CDP |
| Embedded | ❌ Not available |
| Remote | ❌ Not available |

### Network Interception

| Mode | Status |
|------|--------|
| Headless | ⚠️ Available but not exposed via commands |
| Embedded | ❌ Not available |
| Remote | ❌ Not available |

---

## Gap: Scanner Integration

### Pattern Detection

The scanner can detect UI patterns (login forms, search boxes, etc.) but:
- Pattern data not included in `ScanResult` response type
- No command to query detected patterns
- No automatic actions based on patterns

### Change Tracking

Documentation shows change tracking in observations:
```json
{
  "changes": [
    {"id": 5, "change": "appeared"},
    {"id": 12, "change": "text_changed", "old": "0", "new": "3"}
  ]
}
```

**Not implemented** - `ScanResult` in protocol.rs lacks a `changes` field.

---

## Current Build Errors

### backend.rs (lscope-e)

```
Line 35:  BackendError::ConnectionFailed - variant doesn't exist
Line 90:  Type mismatch: expected Vec<Value>, found Option<_>
Line 107: BackendError::NotConnected - variant doesn't exist
Line 111: BackendError::ConnectionFailed - variant doesn't exist
Line 120: BackendError::ConnectionFailed - variant doesn't exist
Line 133: BackendError::NotSupported - variant doesn't exist
```

**Cause**: `BackendError` enum in lscope-core doesn't define these variants.

### translator.rs

```
Line 106: Type annotation needed - cannot infer type parameter T for Option
Line 68:  Clippy: match_single_binding - can simplify match expression
```

### parser.rs (Warnings)

```
Line 247, 248: while_let_loop - can be rewritten as while let
Line 534, 572: collapsible_if - nested if can be collapsed
```

### server.rs (Warnings)

```
Line 9:  Unused import: warn
Line 17: Dead code: response_tx field never read
```

---

## Gap: Documentation vs Reality

### USE-CASES.md Scenarios

The five documented use cases cannot fully execute:

| Use Case | Blockers |
|----------|----------|
| Research Assistant | `text` extraction not translated |
| E-Commerce | `select` option resolution incomplete |
| Travel Booking | Multi-step flow not orchestrated |
| Account Management | `cookies`, `storage` inspection incomplete |
| Content Publishing | `type` with file paths not supported |

### PRODUCT-INTRO.md Claims

| Claim | Reality |
|-------|---------|
| "click 'Sign In'" works | ❌ Only `click 3` works |
| Pattern detection returns patterns | ❌ Not in response format |
| Change tracking between observations | ❌ Not implemented |
| Automatic cookie banner dismissal | ❌ Not implemented |

---

## Prioritized Remediation Plan

### P0 - Critical (Blocks Core Functionality)

1. **Fix build errors in backend.rs**
   - Add missing `BackendError` variants to lscope-core
   - Fix type mismatch on line 90

2. **Fix translator.rs type error**
   - Add type annotation at line 106

3. **Implement Semantic Target Resolution**
   - Create resolver module in lscope-core
   - Query scanner inventory for text/role/relation matches
   - Return best match or error on ambiguity

### P1 - High (Enables Documented Features)

4. **Expand translator coverage**
   - Priority commands: `goto`, `check`, `uncheck`, `press`, `hover`, `focus`, `clear`
   - Content commands: `text`, `html`, `title`, `url`
   - Navigation: `refresh`, `back`, `forward`

5. **Complete `select` translation**
   - Resolve option text to option value/index

### P2 - Medium (Feature Parity)

6. **Screenshot in Embedded/Remote modes**
7. **Add `changes` field to ScanResult**
8. **Add `patterns` field to ScanResult**

### P3 - Low (Nice to Have)

9. **Cookie management commands**
10. **Tab/window management**
11. **PDF in non-headless modes** (may not be feasible)
12. **High-level goals** (`login`, `search`)

---

## Appendix: File Locations

| Component | Path |
|-----------|------|
| Parser | `lscope-core/src/parser.rs` |
| Translator | `lscope-core/src/translator.rs` |
| Protocol types | `lscope-core/src/protocol.rs` |
| Backend trait | `lscope-core/src/backend.rs` |
| Scanner | `scanner/scanner.js` |
| Headless backend | `lscope-h/src/backend.rs` |
| Embedded backend | `lscope-e/src/backend.rs` |
| Remote backend | `lscope-r/src/backend.rs` |
