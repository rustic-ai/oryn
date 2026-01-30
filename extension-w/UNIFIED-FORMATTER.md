# Unified Formatter Implementation

## Problem

Previously, we had two separate formatters for scan results:

1. **Rust formatter** (`oryn-engine/src/formatter/mod.rs`) - Used by REPL/CLI
2. **JavaScript formatter** (`extension-w/sidepanel.js::formatScanResult()`) - Used by extension

This created:
- **Code duplication**: Same logic implemented twice
- **Inconsistency risk**: Changes to one formatter wouldn't apply to the other
- **Maintenance burden**: Bug fixes and improvements needed in both places
- **Different output**: Potential for diverging formatting styles

## Solution

**Use a single Rust formatter across all modes** by exposing it through WASM.

## Implementation

### Step 1: Move Formatter to Shared Crate

**Before:**
```
oryn-engine/src/formatter/mod.rs  ← Only accessible to Rust binaries
```

**After:**
```
oryn-common/src/formatter.rs      ← Accessible to all crates including WASM
```

**Changes:**
- Moved `crates/oryn-engine/src/formatter/mod.rs` → `crates/oryn-common/src/formatter.rs`
- Updated import: `use oryn_common::protocol` → `use crate::protocol`
- Added `pub mod formatter;` to `oryn-common/src/lib.rs`
- Updated `oryn-engine/src/executor.rs` to import from `oryn_common::formatter`
- Re-exported in `oryn-engine/src/lib.rs` for backward compatibility

### Step 2: Expose Formatter via WASM

**File:** `crates/oryn-core/src/wasm.rs`

Added new WASM binding:

```rust
#[wasm_bindgen(js_name = formatScan)]
pub fn format_scan(scan_json: &str) -> Result<String, JsValue> {
    let scan: ScanResult = serde_json::from_str(scan_json)?;

    let response = ScannerProtocolResponse::Ok {
        data: Box::new(ScannerData::Scan(Box::new(scan))),
        warnings: Vec::new(),
    };

    Ok(format_response(&response))
}
```

This exposes the Rust formatter to JavaScript as `OrynCore.formatScan(scanJson)`.

### Step 3: Update Extension to Use WASM Formatter

**File:** `extension-w/background.js`

Added message handler:

```javascript
if (request.type === 'format_scan') {
    const scanJson = JSON.stringify(request.scan);
    const formatted = OrynCoreClass.formatScan(scanJson);
    sendResponse({ success: true, formatted });
}
```

**File:** `extension-w/sidepanel.js`

Replaced JavaScript formatter with async WASM call:

```javascript
// OLD: Manual JavaScript formatting (40+ lines)
function formatScanResult(scan) {
    let output = `@ ${scan.page.url} "${scan.page.title}"\n`;
    // ... manual element formatting ...
    // ... manual pattern detection ...
}

// NEW: Call Rust formatter via WASM (5 lines)
async function formatScanResult(scan) {
    const response = await chrome.runtime.sendMessage({
        type: 'format_scan',
        scan: scan
    });
    return response.formatted;
}
```

## Benefits

### 1. **Single Source of Truth**
- One formatter implementation in `oryn-common/src/formatter.rs`
- Changes automatically apply to REPL, CLI, and Extension

### 2. **Guaranteed Consistency**
All modes now output identical format:
```
@ https://example.com "Page Title"
[1] input/email "Username" {required}
[2] input/password "Password"
[3] button/primary "Login"

Patterns:
- Login Form (85% confidence)
- Search Box
- Pagination
```

### 3. **Advanced Features Everywhere**
Features that were only in Rust formatter now work in extension:
- **Sensitive field masking**: Passwords/CVV shown as `••••••••`
- **Pattern confidence scores**: "Login Form (85% confidence)"
- **Low confidence warnings**: "(Note: Unusual structure, verify before use)"
- **Full mode support**: Element positions when `full_mode: true`
- **DOM changes display**: `+3 -1 elements`

### 4. **Less Code, Fewer Bugs**
- Removed 40+ lines of duplicate JavaScript formatting logic
- Pattern detection output now matches Rust exactly
- Type safety from Rust prevents formatting bugs

### 5. **Future-Proof**
When we add new features to the formatter (e.g., new patterns, better masking), they automatically work everywhere without touching JavaScript.

## Testing

**Reload the extension:**
1. Open `chrome://extensions`
2. Find "Oryn-W Extension"
3. Click refresh icon

**Verify uniform output:**
1. Navigate to any page with a form
2. Open Oryn sidepanel
3. Click "Execute" (runs scan)
4. Compare output with REPL:

```bash
./target/release/oryn-h
> scan
```

Both should produce **identical output** including:
- Element formatting
- Pattern detection messages
- Confidence scores
- Sensitive field masking

## Files Changed

### Created/Modified:
- ✅ `crates/oryn-common/src/formatter.rs` (moved from oryn-engine)
- ✅ `crates/oryn-common/src/lib.rs` (added formatter module)
- ✅ `crates/oryn-core/src/wasm.rs` (added formatScan binding)
- ✅ `extension-w/background.js` (added format_scan handler)
- ✅ `extension-w/sidepanel.js` (replaced formatScanResult with WASM call)
- ✅ `crates/oryn-engine/src/executor.rs` (updated import)
- ✅ `crates/oryn-engine/src/lib.rs` (re-exported formatter)

### Removed:
- ❌ `crates/oryn-engine/src/formatter/mod.rs` (moved to oryn-common)
- ❌ 40+ lines of duplicate JavaScript formatting code

## Architecture

```
┌─────────────────────────────────────┐
│  oryn-common/src/formatter.rs       │
│  (Single Source of Truth)           │
└─────────┬───────────────────────────┘
          │
    ┌─────┴──────────────────┐
    │                        │
    v                        v
┌─────────────┐      ┌──────────────────┐
│ REPL/CLI    │      │  Extension-W     │
│             │      │                  │
│ Direct use  │      │  WASM binding    │
│ from Rust   │      │  formatScan()    │
└─────────────┘      └──────────────────┘
     │                       │
     v                       v
  Terminal              Browser UI

  Identical Output ✓
```

---

**Status:** ✅ Complete
**Date:** 2026-01-29
**Impact:** All Oryn modes now produce uniform, consistent output
