# Scanner.js Sync - Testing Checklist

Use this checklist to verify the synced scanner.js works correctly across all deployment modes.

## Pre-Testing

- [x] Files synced (all 3 identical)
- [x] Automation scripts created
- [x] Documentation written
- [ ] Changes committed to git

## Core Functionality Tests

### Oryn-H (Headless Chromium)

```bash
./scripts/run-e2e-tests.sh --quick
```

- [ ] All E2E tests pass
- [ ] No regressions in test results
- [ ] Timing similar to previous runs
- [ ] Scanner injection works
- [ ] All commands execute correctly

**If tests fail:** Check SCANNER-SYNC-SUMMARY.md for rollback instructions

### Oryn-E (Embedded WebKit)

```bash
./scripts/run-e2e-tests.sh oryn-e-debian
```

- [ ] Tests pass on WebKit
- [ ] No browser-specific issues
- [ ] Shadow DOM features work (if test suite includes them)

### Oryn-R (Remote Extension)

**Manual test:**

1. Load extension:
   - Open chrome://extensions
   - Enable Developer mode
   - Load unpacked: `extension/`
   - Extension loads without errors

2. Connect to server:
   - Start oryn-r server
   - Extension connects via WebSocket
   - Connection indicator shows "connected"

3. Basic commands:
   ```
   goto https://example.com
   observe
   click "More information"
   observe
   ```

4. Form interaction:
   ```
   goto https://httpbin.org/forms/post
   type "custname" "Test User"
   type "comments" "Testing scanner"
   click "Submit order"
   ```

**Checklist:**
- [ ] Extension loads without errors
- [ ] WebSocket connection works
- [ ] `observe` returns elements
- [ ] `click` executes correctly
- [ ] `type` works in forms
- [ ] Navigation commands work
- [ ] Pattern detection works (login, search)

### Oryn-W (WASM Extension)

**Build and load:**

```bash
./scripts/build-extension-w.sh
```

1. Load extension:
   - Open chrome://extensions
   - Load unpacked: `extension-w/`
   - Check console for WASM initialization

2. Test popup:
   - Click extension icon
   - Enter command: `observe`
   - Check for response
   - Try: `click "Example"`

3. Test sidepanel:
   - Open sidepanel
   - Check WASM status: "Ready"
   - View command logs
   - Execute commands

**Checklist:**
- [ ] WASM initializes successfully
- [ ] Extension loads without errors
- [ ] Popup UI works
- [ ] Sidepanel works
- [ ] Commands execute via WASM engine
- [ ] `observe` returns elements
- [ ] `click` and `type` work
- [ ] No performance degradation

## Shadow DOM Specific Tests

Test on sites with Shadow DOM (if available):

**Sites to try:**
- Salesforce (heavy shadow DOM)
- YouTube player controls
- Polymer/LitElement demo sites
- Storybook component libraries

**Commands:**
```
observe
# Should find elements inside shadow roots

click "Button Text In Shadow"
# Should locate across shadow boundaries

type "Input In Shadow" "test"
# Should work in shadow DOM inputs
```

**Checklist:**
- [ ] Elements in shadow DOM detected in `observe`
- [ ] Can click elements in shadow roots
- [ ] Can type in inputs inside shadow DOM
- [ ] Selectors work across shadow boundaries
- [ ] Pattern detection works with shadow DOM

## Regression Tests

Run full E2E suite:

```bash
./scripts/run-e2e-tests.sh
```

**Expected:** All tests pass on all backends

- [ ] 01_static.oil - ✓ Pass
- [ ] 02_forms.oil - ✓ Pass
- [ ] 03_ecommerce.oil - ✓ Pass
- [ ] 04_interactivity.oil - ✓ Pass
- [ ] 05_dynamic.oil - ✓ Pass
- [ ] 06_edge_cases.oil - ✓ Pass

## Performance Tests

### Load Time
- [ ] Scanner injection time similar (< 100ms)
- [ ] WASM load time acceptable (< 200ms)

### Execution Time
- [ ] `observe` completes in < 500ms (typical page)
- [ ] `click` responds immediately
- [ ] No noticeable lag in commands

### Memory Usage
- [ ] No memory leaks (run multiple scans)
- [ ] Memory usage stable after multiple operations
- [ ] WASM memory footprint acceptable (< 10MB)

## Compatibility Tests

### Browser Versions
- [ ] Chrome 120+ (extension, extension-w)
- [ ] Edge 120+ (extension, extension-w)
- [ ] Chromium 120+ (oryn-h)
- [ ] WebKit (oryn-e)

### Modern Web Features
- [ ] Web Components work
- [ ] Shadow DOM elements accessible
- [ ] Custom elements detected
- [ ] SPA navigation handled correctly

## Bug Fix Verification

New bug fixes in this sync:

1. **WeakMap reset on navigation:**
   ```
   goto https://example.com
   observe
   goto https://another-site.com
   observe
   # Should not have stale element IDs
   ```
   - [ ] No stale elements after navigation
   - [ ] IDs reset correctly
   - [ ] No memory leaks

2. **Element map update fix:**
   ```
   observe
   click 1
   observe
   # Element map should be consistent
   ```
   - [ ] No "element not found" errors
   - [ ] Map stays synchronized

## Documentation Verification

- [ ] SCANNER-SYNC.md is clear
- [ ] CLAUDE.md updated correctly
- [ ] Scripts are documented
- [ ] Workflow is understandable

## Final Checks

- [ ] All tests passing
- [ ] No console errors
- [ ] Extensions load correctly
- [ ] No performance regressions
- [ ] Ready to commit

## If Tests Fail

1. **Check which deployment mode failed**
2. **Review error logs**
3. **Compare with previous behavior**
4. **Check SCANNER-SYNC-SUMMARY.md for rollback**
5. **File issues with specific failure details**

## Success Criteria

All checkboxes above should be checked before considering sync complete.

Priority order:
1. ✅ Core functionality (observe, click, type)
2. ✅ Regression tests (E2E suite)
3. ✅ Extension loading
4. ⚠️ Shadow DOM (nice to have, test if available)
5. ⚠️ Performance (should be similar, not critical)

---

**Testing Status:** Not started

**Last Updated:** 2026-01-26

**Tester:** _______________

**Date:** _______________
