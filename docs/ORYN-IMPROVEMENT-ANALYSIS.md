# Oryn Improvement Analysis

Analysis of opportunities and patterns to enhance the Oryn communication protocol and Intent Language (OIL).

---

## Executive Summary

After reviewing the specifications, implementation code, and **scanner.js source**, I've identified improvements across five categories:

| Category | Priority | Impact |
|----------|----------|--------|
| Agent Experience | High | Reduces token waste, improves reliability |
| Observation Format | High | Better LLM comprehension |
| Protocol Efficiency | Medium | Lower latency, cleaner state management |
| OIL Syntax | Medium | More intuitive commands |
| Missing Capabilities | Low-Medium | Edge case handling |

**Key Finding**: The scanner.js already has excellent infrastructure for change tracking that is **underutilized**. Many "new features" just need proper surfacing through the Rust formatter.

---

## 1. Agent Experience Improvements

### 1.1 Incremental Observations (Diff Mode)

**Problem**: After every action, agents must call `observe` and process the entire element list again, even when only a few elements changed. This wastes tokens and cognitive load.

**Current Flow**:
```
click 5 → ok click 5 (minimal change info)
observe → Full 50+ element list (wasteful)
```

**Proposed Solution**: Add `observe --diff` or automatic change tracking:

```
click 5

ok click 5

# changes
+ [51] alert "Item added to cart"
~ [12] button "Add to Cart" → "Added ✓" {disabled}
- [8] badge "Sale"

# context (optional summary)
cart_count: 1 → 2
```

**Benefits**:
- 80%+ token reduction for action verification
- Agent immediately sees what changed
- No need for full re-scan after simple actions

**IMPLEMENTATION REALITY** ✅ **Already exists in scanner.js!**

The scanner has this infrastructure:

```javascript
// scanner.js:261-289 - createChangeTracker()
createChangeTracker: () => {
    const initialUrl = window.location.href;
    const domChanges = { added: 0, removed: 0, attributes: 0 };
    const observer = new MutationObserver(processMutations);
    observer.observe(document.body, { childList: true, subtree: true, attributes: true });
    return { navigationDetected, domChanges, cleanup };
}

// scanner.js:822-858 - diffElements() compares element states
diffElements: (oldData, newData) => {
    // Tracks: text_changed, state_changed, position_changed
}

// scanner.js:648 - monitor_changes parameter
const monitorChanges = params.monitor_changes === true;
```

**What's Missing**: 
1. Actions return `dom_changes: {added: 3, removed: 1}` (counts only, not element details)
2. The Rust formatter doesn't translate the rich change data into OIL response format
3. No `observe --diff` command wired up

**Fix**: 
- After click/type/etc, run a quick incremental scan and diff against cached state
- Return actual elements that changed, not just counts
- Add `observe --diff` that uses `monitor_changes=true`

---

### 1.2 Action Confirmation with Context

**Problem**: Current success responses are minimal. Agents often need to `observe` just to confirm an action worked.

**Current** (what scanner.js returns):
```javascript
// scanner.js:1053-1062
return Protocol.success({
    action: 'clicked',
    id: params.id,
    tag: el.tagName.toLowerCase(),
    selector: Utils.generateSelector(el),
    coordinates: { x, y },
    button: buttonType,
    navigation: navigationDetected,  // ← exists!
    dom_changes: domChanges          // ← exists! but just counts
});
```

**What agents see** (after Rust formatting):
```
ok click 3
```

**Gap**: The scanner returns rich data (`navigation`, `dom_changes`) but the formatter throws it away!

**Proposed** (surface what scanner already provides):
```
ok click 3

# result
[3] button "Sign in" → clicked at (450, 320)
navigation: true
dom_changes: +5 -2 elements

# page (if navigated)
@ dashboard.example.com/home "Dashboard"
```

**For type command** (scanner.js:1173-1180 already returns this):
```javascript
return Protocol.success({
    action: 'typed',
    value: el.value,      // ← final value exists!
    submitted: !!params.submit
});
```

Format as:
```
ok type 3

# result
[3] input/email = "user@example.com" {valid}
```

**Implementation**: Pure Rust formatter change - scanner data is already there.

---

### 1.3 Smart Hints for Common Failures

**Problem**: Error messages tell what failed but not always how to recover intelligently.

**Current**:
```
error click 99: element not found

# hint
Available elements: 1-15. Run 'observe' to refresh.
```

**Proposed**: Context-aware hints:
```
error click "Add to Cart": element not found

# context
Last scan: 45 seconds ago (stale?)
Similar elements found:
  [12] button "Add to Bag" (85% match)
  [14] button "Add to Wishlist" (60% match)
  
# hint
Try: click 12, or observe to refresh element map
```

**For visibility issues** (scanner.js:994-1002 already checks this):
```javascript
if (!params.force && !Utils.isVisible(el)) {
    throw { msg: `Element ${params.id} is not visible`, code: 'ELEMENT_NOT_VISIBLE' };
}
```

**Proposed enhancement** - include position context:
```
error click 5: element not visible

# context
[5] button "Submit" position: y=1850 (viewport ends at y=900)
element is below fold

# hint
Try: scroll down, or scroll 5, or click 5 --force
```

**Implementation**: 
- Scanner can include `el.getBoundingClientRect()` in error context
- Rust resolver can fuzzy-match text targets against all elements
- Track last scan timestamp in session state

---

### 1.4 Element Stability Indicators

**Problem**: Agents don't know which elements are stable vs. dynamically loaded. This causes premature interactions.

**Proposed**: Add stability hints to observations:
```
[1] input/search "Search" {stable}
[2] button "Submit" {stable}
[3] div/results "Loading..." {dynamic, loading}
[4] product "Widget" {dynamic, settled}
```

**Modifiers**:
- `{stable}` - Element unlikely to change
- `{dynamic}` - Element may update
- `{loading}` - Active loading indicator detected
- `{settled}` - Dynamic element that has stopped changing

---

## 2. Observation Format Improvements

### 2.1 Hierarchical Grouping

**Problem**: Flat element lists lose structural context. Agents see 50 elements but don't understand groupings.

**Current**:
```
[1] input "Email"
[2] input "Password"  
[3] button "Sign in"
[4] link "Forgot password"
[5] heading "Or continue with"
[6] button "Google"
[7] button "Apple"
```

**Proposed**: Optional grouped view:
```
@ login_form
  [1] input/email "Email" {required}
  [2] input/password "Password" {required}
  [3] button/submit "Sign in" {primary}
  [4] link "Forgot password"

@ social_login
  [5] heading "Or continue with"
  [6] button "Google"
  [7] button "Apple"

@ footer (collapsed)
  8 elements - use 'observe --section footer' to expand
```

**Benefits**:
- Clear form boundaries
- Collapsed sections reduce noise
- Agents understand structure

---

### 2.2 Value Display for Inputs

**Problem**: Current observations don't always show input values, making it hard to verify form state.

**Current**:
```
[3] input/email "Email" {required}
```

**Proposed**: Include current value:
```
[3] input/email "Email" {required} = "user@example.com"
[4] input/password "Password" {required} = "••••••••"
[5] select "Country" = "United States"
[6] checkbox "Remember me" = checked
```

---

### 2.3 Visibility/Interactability Quick View

**Problem**: Agents attempt actions on elements they can't interact with.

**Proposed**: Add interactability column in `observe --full`:
```
ID  Type        Text              State           Interact
[1] input       "Search"          {focused}       ✓ ready
[2] button      "Submit"          {disabled}      ✗ disabled
[3] link        "Learn more"      {hidden}        ✗ not visible
[4] button      "Delete"          {covered}       ⚠ covered by modal
[5] input       "Name"            {}              ✓ ready
```

---

### 2.4 Pattern Confidence Scores

**Problem**: Pattern detection can be wrong. Agents don't know when to trust detected patterns.

**Current**:
```
# patterns
- login_form: email=[1] password=[2] submit=[3]
```

**Proposed**:
```
# patterns
- login_form (95% confidence): email=[1] password=[2] submit=[3]
- cookie_banner (60% confidence): accept=[8]
  note: unusual structure, verify before dismissing
```

---

## 3. Protocol/Wire Format Improvements

### 3.1 Request IDs for All Modes

**Problem**: Only Remote mode uses request IDs. Subprocess mode could benefit for debugging/logging correlation.

**Current (subprocess)**:
```
→ click 5
← ok click 5
```

**Proposed (optional)**:
```
→ @42 click 5
← @42 ok click 5
```

Makes log correlation easier. Optional to maintain simplicity.

---

### 3.2 Streaming Observations for Large Pages

**Problem**: Pages with 200+ elements timeout or consume excessive memory before responding.

**Proposed**: Streaming protocol extension:
```
observe --stream

ok observe (streaming)
# page
@ example.com/products "Products"

# elements (batch 1/4)
[1] ... [50] ...
---continue---

# elements (batch 2/4)
[51] ... [100] ...
---continue---

# complete
total: 187 elements
---
```

Client can start processing before full response arrives.

---

### 3.3 Heartbeat/Keepalive for Long Operations

**Problem**: Some operations (file downloads, slow pages) can take 30+ seconds. No feedback during wait.

**Proposed**: Progress indicators:
```
goto slow-page.com

pending goto slow-page.com
# progress: navigation started
---continue---

pending goto slow-page.com  
# progress: 45% loaded (2.3MB/5.1MB)
---continue---

ok goto slow-page.com
@ slow-page.com "Heavy Page"
---
```

---

### 3.4 Batch Command Support

**Problem**: Related actions require multiple round-trips.

**Current**:
```
→ clear 1
← ok clear 1
→ type 1 "new value"
← ok type 1
→ press Tab
← ok press Tab
```

**Proposed**: Atomic batches:
```
batch
  clear 1
  type 1 "new value"
  press Tab
end

ok batch (3/3 succeeded)
# results
1. ok clear 1
2. ok type 1
3. ok press Tab
```

Stops on first failure with partial results.

---

## 4. OIL Syntax Improvements

### 4.1 Unified Target Syntax

**Problem**: Different commands use different target syntaxes inconsistently.

**Inconsistencies**:
```
click "Submit"              # Text works
type email "value"          # Role works
wait visible "Loading"      # Text works
wait exists "#modal"        # Selector only? 
extract css(".item")        # Different selector syntax
```

**Proposed**: Unified target everywhere:
```
wait exists "Loading"       # Text should work
wait exists css("#modal")   # Explicit selector
extract "Product"           # Text targeting for extract
```

---

### 4.2 Compound Conditions for Wait

**Problem**: Can't wait for multiple conditions.

**Current** (requires JavaScript):
```
wait until "document.querySelector('#ready') && !document.querySelector('.loading')"
```

**Proposed**:
```
wait visible "Results" and hidden "Loading"
wait url "/dashboard" or visible "Welcome"
```

---

### 4.3 Relative Actions

**Problem**: No way to click "the second Add button" or "the Delete next to Item 3".

**Current** (clunky):
```
observe
# manually identify: [5] is the right one
click 5
```

**Proposed**:
```
click "Delete" near "Item 3"           # Already supported
click "Add" nth 2                       # NEW: second match
click "Edit" inside "Row 3"             # Already supported
click first "Submit"                    # NEW: explicit first
click last "Previous"                   # NEW: explicit last
```

---

### 4.4 Implicit Waits After Navigation

**Problem**: Agents must remember to wait after navigation.

**Current**:
```
click "Login"
wait load
observe
```

**Proposed**: Navigation commands auto-wait by default:
```
click "Login"              # If navigation detected, waits automatically
observe
```

Or explicit opt-out:
```
click "Login" --no-wait    # Don't wait for navigation
```

---

### 4.5 Form Fill Shorthand

**Problem**: Multi-field forms require many commands.

**Current**:
```
type email "user@example.com"
type password "secret123"
check "Remember me"
click "Sign in"
```

**Proposed**: Compound form command:
```
fill email="user@example.com" password="secret123" "Remember me"=checked
submit
```

Or with the existing `fill_form` intent made more accessible:
```
fill {email: "user@example.com", password: "secret123", remember: true}
```

---

## 5. Missing Capabilities

### 5.1 Element Attribute Access

**Problem**: Can't query specific attributes of elements.

**Proposed**:
```
attr 5 href
# → "https://example.com/page"

attr 3 data-product-id  
# → "SKU-12345"

attr "Submit" disabled
# → "true"
```

---

### 5.2 Multiple Element Actions

**Problem**: Can't act on multiple elements at once (e.g., uncheck all checkboxes).

**Proposed**:
```
uncheck all "Newsletter"           # All matching checkboxes
click each "Remove" inside "Cart"  # Click all remove buttons
```

With safety limit:
```
click each "Item" --max 10         # Limit iterations
```

---

### 5.3 Clipboard Operations

**Problem**: No clipboard access for copy/paste workflows.

**Proposed**:
```
copy 5                    # Copy element's text to clipboard
copy "text to copy"       # Copy literal text
paste 3                   # Paste into input
clipboard                 # View clipboard contents
```

---

### 5.4 File Download Tracking

**Problem**: No way to know if/when file downloads complete.

**Proposed**:
```
click "Download PDF"
wait download

ok wait download
# download
filename: report.pdf
size: 2.3MB
path: /downloads/report.pdf
```

---

### 5.5 Conditional Command Execution

**Problem**: Agents must observe, reason, then act. Some simple conditions could be built-in.

**Proposed**:
```
click "Accept" if visible           # Only click if visible
dismiss popups if any               # Only run if popups exist
type email "test@test.com" unless filled   # Skip if already has value
```

---

## 6. Documentation/DX Improvements

### 6.1 Interactive Command Help

**Proposed**: In-session help:
```
help click

click - Click an element

Usage:
  click <target> [options]

Targets:
  click 5                    # By ID
  click "Sign in"           # By text
  click email               # By role
  click css(".btn")         # By selector

Options:
  --double     Double-click
  --right      Right-click
  --force      Click even if covered
  --ctrl       Hold Ctrl while clicking

Examples:
  click "Submit"
  click 3 --double
  click "Delete" near "Item 1"
```

---

### 6.2 Suggest Commands

**Proposed**: Context-aware suggestions:
```
suggest

Based on current page (login form detected):
  login "email" "password"    # Use login intent
  type 1 "email"              # Fill email manually
  click 3                     # Submit button

Common next steps:
  observe                     # Refresh element list
  screenshot login.png        # Capture state
```

---

## 7. Implementation Priority Matrix

| Improvement | Effort | Impact | Priority |
|-------------|--------|--------|----------|
| Diff-mode observations | Medium | High | P0 |
| Action confirmation with context | Low | High | P0 |
| Smart error hints | Medium | High | P0 |
| Hierarchical grouping | Medium | Medium | P1 |
| Value display for inputs | Low | Medium | P1 |
| Batch commands | Medium | Medium | P1 |
| Unified target syntax | Low | Medium | P1 |
| Compound wait conditions | Low | Medium | P2 |
| Relative actions (nth, first, last) | Low | Medium | P2 |
| Streaming observations | High | Medium | P2 |
| Element attribute access | Low | Low | P3 |
| Clipboard operations | Low | Low | P3 |

---

## Summary

The most impactful improvements focus on **reducing the observe-act-observe cycle overhead**:

1. **Diff-mode observations** - Don't rescan everything
2. **Richer action responses** - Confirm success without re-observing
3. **Smart hints** - Guide recovery without guessing

These directly address the primary pain point: agents spending tokens and time on redundant observations.

Secondary improvements focus on **cleaner syntax** and **structural clarity** that helps LLMs reason more accurately about page state.

---

## Appendix: Scanner.js Capabilities Analysis

After reviewing the 2,189-line scanner.js, here are underutilized capabilities:

### Already Implemented (Just Need Formatter Changes)

| Capability | Scanner Location | Current Usage | Opportunity |
|------------|------------------|---------------|-------------|
| DOM change tracking | `createChangeTracker()` L261 | Returns counts only | Return actual changed elements |
| Element diffing | `diffElements()` L822 | Only in `scan --monitor` | Use after actions |
| Navigation detection | Actions return `navigation: bool` | Ignored by formatter | Surface in responses |
| Element values | `state.value` in serialize | Returned but not formatted | Show in observations |
| Interactability check | `isInteractable()` L332 | Used for errors | Surface in scan results |
| Pattern confidence | Patterns module | Binary detection | Add scoring |

### Key Scanner Features to Leverage

**1. Change Tracking Infrastructure**
```javascript
// L261-289: Full MutationObserver setup
const observer = new MutationObserver(processMutations);
observer.observe(document.body, { childList: true, subtree: true, attributes: true });
```
Currently: Returns `{added: 3, removed: 1}` counts.
Could: Return actual `[{id: 51, type: 'appeared', element: {...}}]`

**2. Element State Caching**
```javascript
// L708-716: Cache for diff comparison
if (monitorChanges) {
    const cached = STATE.cache.get(id);
    if (cached) {
        const elementChanges = Scanner.diffElements(cached, serialized);
    }
    STATE.cache.set(id, serialized);
}
```
Currently: Only used with `monitor_changes=true` in scan.
Could: Always cache, enable `observe --diff`

**3. Rich Element Serialization**
```javascript
// L948-969: Already captures everything
return {
    id, type, role, text, label,
    selector, xpath,
    rect: { x, y, width, height },
    attributes,
    state: { visible, disabled, focused, checked, required, value }
};
```
Currently: Formatter strips most of this for "compact" mode.
Could: Include value in default output, position in --full

**4. Shadow DOM Support**
```javascript
// L445-635: Comprehensive shadow DOM traversal
ShadowUtils.collectElements()      // Finds elements in shadow roots
ShadowUtils.querySelectorWithShadow()  // CSS selection across shadows
ShadowUtils.findTextNodeWithShadow()   // Text search across shadows
```
This is excellent and properly implemented. No changes needed.

### Recommended Scanner Enhancements

**1. Add `scan_diff` Command**
```javascript
case 'scan_diff':
    // Compare current DOM against STATE.cache
    // Return only elements that changed since last scan
    result = Scanner.scanDiff(message);
    break;
```

**2. Enhance Action Responses**
```javascript
// After click, automatically capture what changed
const changes = Scanner.getRecentChanges(); // Check cache vs current
return Protocol.success({
    action: 'clicked',
    ...existing,
    element_changes: changes  // New: actual changed elements
});
```

**3. Add Fuzzy Text Matching for Errors**
```javascript
// When element not found by text, find similar
const similar = findSimilarElements(targetText, STATE.elementMap);
throw { 
    msg: 'Element not found', 
    code: 'ELEMENT_NOT_FOUND',
    similar: similar.slice(0, 3)  // Top 3 matches
};
```

### Formatter vs Scanner Responsibility

| Task | Current Owner | Recommended |
|------|---------------|-------------|
| DOM traversal | Scanner ✓ | Keep |
| Element serialization | Scanner ✓ | Keep |
| Change tracking | Scanner ✓ | Keep |
| Pattern detection | Scanner ✓ | Keep |
| Response formatting | Rust formatter | Keep |
| Diff presentation | Not done | Rust formatter |
| Smart hints | Partially Rust | Enhance in Rust |
| Fuzzy matching | Not done | Scanner (text) + Rust (targets) |

### Quick Wins (Formatter Only)

These require zero scanner changes:

1. **Show element values**: Scanner already returns `state.value`
2. **Show navigation status**: Actions return `navigation: true/false`
3. **Show DOM change counts**: Actions return `dom_changes`
4. **Include rect in --full**: Scanner returns `rect`
5. **Show checked state clearly**: Scanner returns `state.checked`

### Medium Effort (Scanner + Formatter)

1. **Diff mode**: Enable `scan({monitor_changes: true})` via OIL
2. **Similar element suggestions**: Add fuzzy matching in scanner errors
3. **Interactability indicator**: Include `isInteractable()` result in scan

### Larger Effort

1. **Full change tracking in actions**: Run quick diff after every action
2. **Pattern confidence scoring**: Enhance Patterns module
3. **Streaming responses**: Would need protocol changes
