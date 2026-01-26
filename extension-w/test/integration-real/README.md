# Real WASM Integration Tests

These tests verify the **actual WASM module** (not mocks) works correctly in a browser environment.

## Why Separate Tests?

The mock-based integration tests (`test/integration/`) are fast and run in Node.js, but they use a JavaScript mock (`wasm-mock.js`) that simulates WASM behavior. These real WASM tests verify:

- ✅ The actual Rust code compiles to WASM correctly
- ✅ The WASM module loads in a browser
- ✅ The wasm-bindgen JavaScript glue code works
- ✅ OrynCore class methods work as expected
- ✅ Command processing returns correct results
- ✅ Performance is acceptable

## Prerequisites

Build the WASM module first:

```bash
cd /home/rohit/work/dragonscale/oryn
./scripts/build-wasm.sh
```

This creates:
- `extension-w/wasm/oryn_core_bg.wasm` (WASM binary)
- `extension-w/wasm/oryn_core.js` (JavaScript bindings)

## Running the Tests

### Option 1: Extension-Based Automated Tests (Recommended)

Run tests with the extension actually loaded in Chromium:

```bash
npm run test:integration:real
```

This uses Puppeteer to:
- Launch Chromium with the extension loaded
- Access the background service worker
- Test WASM in its real execution context
- Verify message passing through the extension
- Measure performance in the actual environment

**Note:** Requires `headless: false` because Chrome extensions don't work in headless mode. A browser window will open and close automatically.

### Option 2: Browser-Based Visual Test Page

Open the test page in a browser:

```bash
# From extension-w directory
cd /home/rohit/work/dragonscale/oryn/extension-w

# Open in Chromium
chromium test/integration-real/wasm-test-page.html

# Or use the launch script
chromium-browser test/integration-real/wasm-test-page.html

# Or just open the file
xdg-open test/integration-real/wasm-test-page.html
```

**What you'll see:**
- Green checkmarks ✓ for passing tests
- Red X marks ✗ for failing tests
- Test summary at the bottom
- Detailed error messages for failures

**Use case:** Quick visual check that WASM module works in isolation.

## Test Files

This directory contains two types of WASM tests:

### extension-wasm.test.js (Recommended)
Tests WASM module **within the extension context**:
- Loads the actual browser extension
- Tests WASM in background service worker
- Verifies message passing through extension
- Tests real-world integration
- Automated with Puppeteer

**Run:** `npm run test:integration:real`

### wasm-test-page.html
Tests WASM module **in isolation**:
- Loads WASM module directly in a webpage
- No extension context required
- Visual feedback with color-coded results
- Quick manual verification

**Run:** `chromium test/integration-real/wasm-test-page.html`

### wasm-real.test.js (Legacy)
Isolated Puppeteer tests for WASM only (no extension). Mostly superseded by extension-wasm.test.js.

## What Gets Tested

### Module Loading (4 tests)
- ✅ WASM module loads
- ✅ WASM initializes
- ✅ OrynCore instance can be created
- ✅ Version information is accessible

### Scan Management (2 tests)
- ✅ Scan context can be updated
- ✅ Invalid scan JSON is rejected

### Command Processing (4 tests)
- ✅ `observe` command works
- ✅ `goto "url"` command works
- ✅ `click "text"` command works
- ✅ Invalid commands throw errors

### Error Handling (1 test)
- ✅ Processing without scan throws appropriate error

### Performance (1 test)
- ✅ 100 commands complete in <500ms (avg <5ms each)

**Total: 12 real WASM tests**

## Interpreting Results

### All Tests Pass ✅
The WASM module is working correctly and ready for use in the extension.

### Some Tests Fail ❌

**Common Failures:**

1. **"WASM module not found"**
   - Build the WASM module: `./scripts/build-wasm.sh`

2. **"Failed to parse scan JSON"**
   - Check scan structure in test
   - Verify WASM validation logic

3. **"Unknown command"**
   - Parser may have changed
   - Update test or fix parser

4. **Performance test fails**
   - WASM may be slow in debug mode
   - Rebuild with `--release` flag

## Comparison: Mock vs Real WASM Tests

| Aspect | Mock Tests | Real WASM Tests |
|--------|-----------|-----------------|
| **Location** | `test/integration/` | `test/integration-real/` |
| **WASM Module** | JavaScript mock | Actual Rust WASM |
| **Environment** | Node.js + Jest | Browser (Chromium) |
| **Speed** | ~0.6s (41 tests) | ~2-3s (12 tests) |
| **What's Tested** | JavaScript logic | Rust→WASM compilation |
| **Dependencies** | None | WASM build required |
| **CI/CD** | ✅ Fast, automated | ⚠️ Slower, requires build |

## When to Run Each

### Mock Tests (Always)
```bash
npm run test:integration
```
- ✅ During development (fast feedback)
- ✅ In CI/CD pipelines (no WASM build needed)
- ✅ When testing JavaScript logic
- ✅ Before committing code

### Real WASM Tests (Periodic)
```bash
npm run test:integration:real
# OR open wasm-test-page.html in browser
```
- ✅ After building WASM module
- ✅ Before releasing extension
- ✅ When Rust code changes
- ✅ To verify actual behavior

### Both (Comprehensive)
```bash
npm run test:all:with-wasm
```
- ✅ Before major releases
- ✅ When refactoring Rust code
- ✅ To verify end-to-end integration

## Troubleshooting

### CORS Errors

**Problem:** "CORS policy blocked the fetch"

**Solution:** Open with a local server:
```bash
cd extension-w
python3 -m http.server 8000
# Open http://localhost:8000/test/integration-real/wasm-test-page.html
```

### WASM Module Won't Load

**Problem:** "Failed to load WASM"

**Check:**
1. WASM file exists: `ls -lh wasm/oryn_core_bg.wasm`
2. WASM file is valid: `file wasm/oryn_core_bg.wasm` (should say "WebAssembly")
3. Build succeeded: `./scripts/build-wasm.sh` (check for errors)

### Tests Timeout

**Problem:** Tests hang or timeout

**Solutions:**
1. Increase Jest timeout: `jest.setTimeout(30000)` in test file
2. Check browser console for errors
3. Verify WASM file isn't corrupted: Re-build with `./scripts/build-wasm.sh`

### Different Results Than Mock

**Problem:** Real WASM behaves differently than mock

**This is expected!** The mock simulates WASM behavior but may differ. Real WASM tests are the source of truth. If real WASM tests fail, the Rust code needs fixing, not the mock.

## Adding New Tests

### Browser Test Page

Edit `wasm-test-page.html` and add a new test:

```javascript
// Test 11: Your New Test
try {
    // Your test code here
    const result = core.processCommand('your command');
    const success = /* your validation */;
    log('Your Test Name', success, 'Details');
} catch (e) {
    log('Your Test Name', false, e.message);
}
```

### Puppeteer Test

Edit `wasm-real.test.js` and add a new test:

```javascript
test('should do something', async () => {
    const result = await page.evaluate(async () => {
        const module = await import('/wasm/oryn_core.js');
        await module.default();
        // Your test code
    });

    expect(result.success).toBe(true);
}, 10000);
```

## Future Improvements

1. **Automated Puppeteer tests** - Make Puppeteer tests work reliably
2. **CI Integration** - Run real WASM tests in GitHub Actions
3. **Performance benchmarks** - Track WASM performance over time
4. **Size monitoring** - Alert if WASM size grows too much
5. **Browser compatibility** - Test in Firefox, Safari (WASM may behave differently)

## Summary

- **Mock tests** = Fast, test JavaScript logic
- **Real WASM tests** = Slower, test actual Rust WASM module
- **Both are valuable** and test different things
- **Run mock tests frequently**, real WASM tests periodically
- **Browser test page** is easiest way to run real WASM tests

---

**Quick Command:**
```bash
chromium test/integration-real/wasm-test-page.html
```
