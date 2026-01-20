# E2E Test Gaps Analysis

## Overview

This document captures the remaining gaps preventing all E2E test scripts from passing. These gaps represent areas where the scanner, resolver, or translator need enhancement to support advanced testing scenarios.

**Analysis Date**: 2026-01-20
**Passing Tests**: 5/9 (56%)
**Failing Tests**: 4/9 (44%)

---

## Current Test Status

| Test Script | Status | Category | Primary Issue |
|-------------|--------|----------|---------------|
| 01_static.oil | PASS | Basic | - |
| 02_forms.oil | PASS | Basic | - |
| 03_ecommerce.oil | PASS | Basic | - |
| 04_interactivity.oil | PASS | Basic | - |
| 05_dynamic.oil | PASS | Basic | - |
| 06_edge_cases.oil | **FAIL** | Advanced | Shadow DOM scanning |
| 07_intents_builtin.oil | **FAIL** | Advanced | Element disambiguation |
| 08_multipage_flows.oil | **FAIL** | Advanced | Text anchor resolution |
| 09_target_resolution.oil | **FAIL** | Advanced | CSS selector for type |

---

## Gap 1: Shadow DOM Scanning

### Affected Test
- `test-harness/scripts/06_edge_cases.oil`

### Error Message
```
Error: Resolution error: No element matches target: Open Action (hint: run 'observe' first)
```

### Description
Elements inside Shadow DOM (even open shadow roots) are not being scanned by the scanner. The scanner's `scanPage()` function only traverses the light DOM, missing elements attached to shadow roots.

### Root Cause
In `crates/oryn-scanner/src/scanner.js`, the `collectElements()` function uses standard DOM traversal which doesn't descend into shadow roots:
```javascript
// Current: Only traverses light DOM
document.querySelectorAll('button, input, select, ...')
```

### Suggested Fix
Recursively traverse shadow roots during element collection:
```javascript
function collectElements(root = document) {
    const elements = [];
    // Collect from current root
    root.querySelectorAll('button, input, ...').forEach(el => elements.push(el));
    // Recursively traverse shadow roots
    root.querySelectorAll('*').forEach(el => {
        if (el.shadowRoot) {
            elements.push(...collectElements(el.shadowRoot));
        }
    });
    return elements;
}
```

### Priority
**Medium** - Shadow DOM is increasingly common in modern web components.

---

## Gap 2: Text Element Anchors for "near" Resolution

### Affected Test
- `test-harness/scripts/08_multipage_flows.oil`

### Error Message
```
Error: Resolution error: No element matches target: Wireless Mouse (hint: run 'observe' first)
```

### Description
The scanner only returns interactive elements (buttons, inputs, links, selects) in scan results. Non-interactive text elements (h3, span, strong, p, label text) are not included in the scan context. This causes "near" resolution to fail when the anchor is a text-only element.

### Example
```oil
click "+" near "Wireless Mouse"  # Fails because "Wireless Mouse" is in an <strong> tag
```

The resolver needs "Wireless Mouse" in the scan context to calculate proximity, but since `<strong>` is not an interactive element, it's not scanned.

### Root Cause
In `crates/oryn-scanner/src/scanner.js`:
```javascript
// Only these elements are collected
const INTERACTIVE_SELECTORS = 'button, input, select, textarea, a, [role="button"], ...';
```

### Suggested Fix
Add a separate scan mode or always include text anchors:

**Option A: Include labeled text elements**
```javascript
// Add text elements that commonly serve as labels/anchors
const ANCHOR_SELECTORS = 'h1, h2, h3, h4, h5, h6, label, strong, [data-label]';
```

**Option B: On-demand text lookup**
When "near" resolution is requested, perform a secondary lookup for the anchor text in the DOM without requiring it to be pre-scanned.

### Priority
**High** - This is a common pattern in intent-based testing (click button near label).

---

## Gap 3: Element Disambiguation

### Affected Test
- `test-harness/scripts/07_intents_builtin.oil`

### Error Messages
```
Error: Element 7 not found
```
(Element index mismatch after page state changes)

### Description
Multiple issues with element targeting:

1. **Ambiguous label matching**: When multiple elements share the same label (e.g., an input with `aria-label="Search"` and a button with text "Search"), the resolver picks the first match regardless of element type.

2. **Element index instability**: After page state changes (e.g., dismissing a cookie banner), element indices shift, causing subsequent commands to target wrong elements.

### Example
```oil
type "Search" "wireless mouse"  # Types into input (correct)
click "Search"                  # Clicks input again (wrong - should click button)
```

### Root Cause
The resolver in `crates/oryn-core/src/resolver.rs` returns the first matching element without considering:
- The command context (type vs click)
- Element roles (input vs button)
- Recent interactions

### Suggested Fix
**Option A: Command-aware resolution**
```rust
// In resolver, consider command type
fn resolve_for_command(target: &Target, command: &Command, ctx: &ResolverContext) -> Target {
    match command {
        Command::Type(_) => prefer_input_elements(candidates),
        Command::Click(_) => prefer_clickable_elements(candidates),
        _ => first_match(candidates),
    }
}
```

**Option B: Role-based disambiguation**
When label matches multiple elements, prefer:
- For `type`: input, textarea, [contenteditable]
- For `click`: button, a, [role="button"]
- For `check`: checkbox, switch

### Priority
**High** - This affects real-world form interactions.

---

## Gap 4: CSS Selector Support for Type Command

### Affected Test
- `test-harness/scripts/09_target_resolution.oil`

### Error Message
```
Error: Translation error: Invalid target for command: Type requires a resolved numeric ID target
```

### Description
The `type` command (and other commands) cannot accept CSS selectors directly. The translator requires targets to be resolved to numeric element IDs before translation.

### Example
```oil
type css(#search-input) "query"  # Fails - CSS selector not supported
type "Search" "query"            # Works - resolved to ID during execution
```

### Root Cause
In `crates/oryn-core/src/translator.rs`:
```rust
Command::Type(target, text, options) => {
    // Only accepts Target::Id(n), not Target::Selector
    let id = match target {
        Target::Id(id) => id,
        _ => return Err("Type requires a resolved numeric ID target"),
    };
}
```

### Suggested Fix
**Option A: Resolve CSS selectors in executor**
Before translation, resolve CSS selectors to IDs:
```rust
// In executor, before translation
Command::Type(Target::Selector(sel), text, opts) => {
    let id = resolve_selector_to_id(&sel, scan_context)?;
    Command::Type(Target::Id(id), text, opts)
}
```

**Option B: Support selectors in scanner**
Modify the scanner's `type` handler to accept selectors:
```javascript
Executor.type = async (params) => {
    let element;
    if (params.id) {
        element = STATE.elementMap.get(params.id);
    } else if (params.selector) {
        element = document.querySelector(params.selector);
    }
    // ... rest of implementation
}
```

### Priority
**Medium** - CSS selectors are a fallback for when semantic targeting fails.

---

## Implementation Recommendations

### Phase 1: High Priority (Most Impact)
1. **Gap 2**: Text anchor resolution - enables natural "near" patterns
2. **Gap 3**: Element disambiguation - improves form interaction reliability

### Phase 2: Medium Priority
3. **Gap 1**: Shadow DOM scanning - modern web component support
4. **Gap 4**: CSS selector support - provides escape hatch for edge cases

### Files to Modify

| Gap | Primary Files |
|-----|---------------|
| Gap 1 | `crates/oryn-scanner/src/scanner.js` |
| Gap 2 | `crates/oryn-scanner/src/scanner.js`, `crates/oryn-core/src/resolver.rs` |
| Gap 3 | `crates/oryn-core/src/resolver.rs`, `crates/oryn-core/src/executor.rs` |
| Gap 4 | `crates/oryn-core/src/translator.rs`, `crates/oryn-core/src/executor.rs` |

---

## Test Scripts Reference

All test scripts are located in `test-harness/scripts/`:

| Script | Purpose |
|--------|---------|
| 01_static.oil | Static page navigation and content verification |
| 02_forms.oil | Form filling and submission |
| 03_ecommerce.oil | E-commerce flow (catalog, cart, checkout) |
| 04_interactivity.oil | Modals, toasts, SPA navigation |
| 05_dynamic.oil | Infinite scroll, live search |
| 06_edge_cases.oil | Shadow DOM, iframes, edge cases |
| 07_intents_builtin.oil | Built-in intent testing |
| 08_multipage_flows.oil | Multi-page checkout flow |
| 09_target_resolution.oil | Advanced targeting and CSS selectors |
