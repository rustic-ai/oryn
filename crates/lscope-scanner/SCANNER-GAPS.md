# Scanner Protocol Implementation Gaps

This document tracks remaining gaps between the scanner implementation (`src/scanner.js`) and the specification (`docs/SPEC-SCANNER-PROTOCOL.md`).

**Last Updated:** January 2025
**Current Coverage:** ~85%

---

## High Priority Gaps

### 1. Missing Command Parameters

| Command | Parameter | Description | Spec Reference |
|---------|-----------|-------------|----------------|
| `scan` | `near` | Filter elements by proximity to text | Section 3.1 |
| `hover` | `offset` | Click offset from element center (for consistency with `click`) | - |
| `wait_for` | `poll_interval` | Custom polling interval (hardcoded to 100ms) | - |

### 2. Missing Response Fields

#### Click Command
- `navigation`: Boolean indicating if click triggered navigation
- `dom_changes`: Object describing elements added/removed/modified

#### All Action Commands (type, clear, check, select, scroll, focus, hover, submit)
- `selector`: CSS selector of target element (click has this, others don't)
- `timing`: Execution timing information (only scan has this)

#### Select Command
- `index`: The index that was selected (when selecting by index)

#### Submit Command
- `form_selector`: Selector of the submitted form
- `form_id`: ID of the submitted form element

### 3. Scan Response Completeness

| Field | Description | Status |
|-------|-------------|--------|
| `settings_applied.max_elements` | Echo back applied settings | Missing |
| `settings_applied.include_hidden` | Echo back applied settings | Missing |
| `settings_applied.include_iframes` | Echo back applied settings | Missing |
| `stats.limit_reached` | Boolean if max_elements was hit | Missing |

---

## Medium Priority Gaps

### 4. Element Attributes Coverage

Currently serialized: `href`, `src`, `placeholder`, `name`, `id`, `autocomplete`, `aria-label`

Missing attributes that would be useful:
- `aria-hidden` - Accessibility state
- `aria-disabled` - Alternative disabled state
- `title` - Tooltip text
- `class` - Often contains styling/state info
- `tabindex` - Focus order
- `data-testid` / `data-*` - Common in modern SPAs

### 5. Element State/Modifiers

Currently in state: `visible`, `disabled`, `focused`, `checked`, `unchecked`, `required`, `readonly`, `value`

Missing modifiers from spec Section 4.3:
- `hidden` - Explicit flag when element is hidden but `include_hidden=true`
- `primary` - Primary/prominent action button flag

### 6. Error Code Usage

| Error Code | Spec Section | Implementation |
|------------|--------------|----------------|
| `ELEMENT_NOT_FOUND` | 2.3 | ✓ Used |
| `ELEMENT_STALE` | 2.3 | ✓ Used |
| `ELEMENT_NOT_VISIBLE` | 2.3 | ✓ Used |
| `ELEMENT_DISABLED` | 2.3 | ✓ Used |
| `ELEMENT_NOT_INTERACTABLE` | 2.3 | ✓ Used |
| `SELECTOR_INVALID` | 2.3 | ✓ Used |
| `TIMEOUT` | 2.3 | ✓ Used |
| `NAVIGATION_ERROR` | 2.3 | ✗ Never thrown |
| `SCRIPT_ERROR` | 2.3 | ✓ Used |
| `UNKNOWN_COMMAND` | 2.3 | ✓ Used |

### 7. Response Data Structure

Spec Section 2.2 shows response data should be nested under a `data` field:
```json
{
  "ok": true,
  "data": { ... command-specific response ... },
  "timing": { ... }
}
```

Current implementation flattens response fields at top level. This is a design decision but differs from spec.

---

## Lower Priority Gaps

### 8. Behavioral Edge Cases

#### Script Execution Scope
The `execute` command uses `Function` constructor which limits script access:
- Scripts cannot access `window`, `document`, `console` directly
- Only `args` parameter is available in scope
- May need wrapper to provide DOM access

#### Multi-Select Support
- `get_value` returns array for multi-select elements
- `select` command doesn't support selecting multiple options
- Need `select_multiple` or array value support

#### Double-Click Event Sequence
Current implementation fires: mousedown → mouseup → click → mousedown → mouseup → click → dblclick

Native browser behavior may differ. Consider matching native sequence more closely.

#### Element Staleness Detection
- No page navigation detection
- Doesn't invalidate map on history/pushstate changes
- Relies only on `isConnected` check

### 9. Selector Generation

Current `generateSelector()` uses `:nth-of-type` which can be fragile:
- Doesn't attempt class-based selectors
- Doesn't use data-testid or other stable attributes
- Falls back to positional selectors that break with DOM changes

Consider improving selector stability by trying:
1. ID (current)
2. data-testid or similar test attributes
3. Unique class combinations
4. aria-label based selectors
5. nth-of-type as last resort

### 10. Iframe Context Information

Current `iframe_context` includes: `iframe_id`, `src`

Could also include:
- `parent_frame_id` for nested iframes
- `frame_rect` - iframe position in main document
- `frame_origin` - origin information

---

## Implementation Recommendations

### Quick Wins (< 1 hour each)
1. Add `selector` field to all action command responses
2. Add `timing` to all command responses
3. Echo back settings in scan response
4. Add `hidden` and `primary` to element state
5. Add missing attributes to serialization

### Medium Effort (1-4 hours each)
1. Implement `near` parameter for proximity filtering
2. Add navigation detection for click response
3. Improve selector generation algorithm
4. Add multi-select support to `select` command

### Larger Efforts (4+ hours)
1. DOM mutation tracking for `dom_changes` in click response
2. Page navigation detection and map invalidation
3. Full response data structure refactor to match spec nesting

---

## Testing Recommendations

Add tests for:
1. Pattern detection edge cases (forms without submit buttons, etc.)
2. Iframe scanning with same-origin content
3. Error code coverage (each error code should have a test)
4. Element staleness scenarios
5. Multi-select elements
6. Visibility edge cases (opacity, visibility, display combinations)

---

## Version History

| Version | Changes |
|---------|---------|
| 1.0.0 | Initial implementation |
| 1.1.0 | Added pattern detection, iframe support, enhanced role detection |
