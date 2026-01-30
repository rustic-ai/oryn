# Fix: Pagination Pattern Type Mismatch

## Problem

The extension was failing with a parsing error when pagination patterns were detected on pages:

```
Failed to parse scan: invalid type: map, expected u32 at line 1 column 155718
```

## Root Cause

The scanner's pagination detection was sending an array of objects instead of an array of IDs:

**Scanner output** (crates/oryn-scanner/src/scanner.js:2093):
```javascript
// BUG: Sends full objects
if (pageNumbers.length > 0) result.pages = pageNumbers.sort((a, b) => a.page - b.page);
// Result: [{page: 1, id: 123}, {page: 2, id: 456}]
```

**Rust expected type** (crates/oryn-common/src/protocol.rs):
```rust
pub struct PaginationPattern {
    pub prev: Option<u32>,
    pub next: Option<u32>,
    pub pages: Vec<u32>,  // ← Expects just the IDs!
}
```

When the scanner detected pagination (e.g., page number buttons), it would:
1. Build `pageNumbers` array as `[{page: 1, id: 123}, {page: 2, id: 456}]`
2. Sort by page number
3. **Send the full objects** instead of just extracting the IDs

Since Rust's `PaginationPattern.pages` expects `Vec<u32>` (array of numbers), receiving an object caused the deserialization error.

## The Fix

Extract just the IDs from the sorted array:

**scanner.js:2093**:
```javascript
// FIXED: Extract just the IDs
if (pageNumbers.length > 0) result.pages = pageNumbers.sort((a, b) => a.page - b.page).map(p => p.id);
// Result: [123, 456]
```

Now the scanner sends:
- `[123, 456]` (array of IDs) ✅
- Not `[{page: 1, id: 123}, {page: 2, id: 456}]` (array of objects) ❌

## Why This Matters

Pattern detection is critical for the Ralph Agent's task planning. When pagination is detected, the agent can:
- Navigate through product listings
- Scrape data across multiple pages
- Understand multi-page workflows

The type mismatch prevented WASM from parsing scan results, causing all agent iterations to fail with parsing errors instead of executing commands.

## Files Modified

**`crates/oryn-scanner/src/scanner.js`** (line 2093)
- Changed from sending full objects to extracting just IDs
- Synced to `extension/scanner.js` and `extension-w/scanner.js`

## Related Issues

This is the second type mismatch fixed in this session:

1. **Negative scroll values**: `max_x` and `max_y` could be negative when content < viewport, but Rust expected `u32` (unsigned). Fixed by wrapping in `Math.max(0, ...)`.

2. **Pagination pattern objects** (this fix): Sending objects instead of IDs. Fixed by adding `.map(p => p.id)`.

Both issues stem from JavaScript's flexible typing vs Rust's strict type system. The scanner must always match the exact types expected by the Rust structs.

## Testing

1. **Reload the extension** in chrome://extensions
2. Navigate to a page with pagination (e.g., Amazon product listings, Google search)
3. Open sidepanel and run "scan"
4. Verify no parsing errors in console
5. Test agent mode with pagination-heavy tasks

---

**Status:** ✅ Fixed
**Date:** 2026-01-29
**Impact:** Agent can now scan pages with pagination without parsing errors
