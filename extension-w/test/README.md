# Extension-W Test Suite

Comprehensive testing for the Oryn-W browser extension with three layers of tests following the testing pyramid approach.

## Overview

The test suite follows a pyramid structure:

```
    /\
   /  \     E2E Tests (20%)
  /----\    - Full browser automation
 /      \   - Puppeteer-based
/--------\
|        |  Integration Tests (30%)
|        |  - WASM module integration
|________|  - Command processing flow

|        |  Unit Tests (50%)
|        |  - Individual component logic
|________|  - Chrome API mocking
```

## Test Layers

### Layer 1: Unit Tests (50%)

**Location:** `test/unit/`

**Purpose:** Test individual components in isolation without requiring a browser environment.

**Files:**
- `background.test.js` - Background script message handling, command execution
- `popup.test.js` - Popup UI interactions, status checking
- `sidepanel.test.js` - Log management, console interception

**Technologies:**
- Jest with jsdom environment
- sinon-chrome for Chrome API mocking
- Custom matchers for extension-specific assertions

**Run:**
```bash
npm run test:unit
```

### Layer 2: Integration Tests (30%)

**Location:** `test/integration/`

**Purpose:** Test interactions between components and the WASM module.

**Files:**
- `wasm.test.js` - WASM module loading, initialization, command processing
- `commands.test.js` - End-to-end command flow from parsing to action

**Technologies:**
- Jest
- Realistic WASM module mock (`helpers/wasm-mock.js`)
- Full command processing pipeline

**Run:**
```bash
npm run test:integration
```

### Layer 3: E2E Tests (20%)

**Location:** `test/e2e/`

**Purpose:** Test the complete extension in a real Chrome browser.

**Files:**
- `extension-loading.test.js` - Extension initialization, content script injection
- `command-execution.test.js` - Full command execution on real HTML pages

**Technologies:**
- Puppeteer
- Real Chrome browser (headless mode disabled for extensions)
- HTML fixtures for testing

**Run:**
```bash
npm run test:e2e
```

**Note:** E2E tests require the WASM module to be built first:
```bash
cd ../
./scripts/build-wasm.sh
cd extension-w
npm run test:e2e
```

## Test Fixtures

**Location:** `test/fixtures/`

HTML pages for E2E testing:

- `static-page.html` - Basic buttons, links, static elements
- `form-page.html` - Input fields, form submission
- `dynamic-page.html` - Dynamic content loading, modals

## Helper Utilities

**Location:** `test/helpers/`

- `chrome-mocks.js` - Mock factory functions for Chrome APIs
  - `createMockScanResult()` - Generate mock scan data
  - `createMockTab()` - Generate mock Chrome tab
  - `createMockWasmModule()` - Generate basic WASM mock
  - `mockRuntimeSendMessage()` - Mock runtime messaging
  - `mockTabsSendMessage()` - Mock tab messaging

- `wasm-mock.js` - Realistic WASM module mock for integration tests
  - Simulates actual parsing and translation
  - Validates scan structure
  - Provides realistic error messages

## Running Tests

### Run all tests
```bash
npm test
```

### Run specific test layers
```bash
npm run test:unit          # Unit tests only
npm run test:integration   # Integration tests only
npm run test:e2e           # E2E tests only (requires WASM build)
```

### Run with coverage
```bash
npm run test:unit -- --coverage
npm run test:integration -- --coverage
```

### Run specific test file
```bash
npm test test/unit/background.test.js
npm test test/integration/wasm.test.js
```

### Run in watch mode
```bash
npm test -- --watch
```

### Run with verbose output
```bash
npm test -- --verbose
```

## Coverage Thresholds

Configured in `package.json`:

```json
{
  "coverageThreshold": {
    "global": {
      "lines": 80,
      "functions": 75,
      "branches": 70,
      "statements": 80
    }
  }
}
```

## Writing New Tests

### Unit Test Template

```javascript
const { mockRuntimeSendMessage, createMockTab } = require('../helpers/chrome-mocks');

describe('Feature Name', () => {
  beforeEach(() => {
    // Setup DOM or mocks
  });

  test('should do something', () => {
    // Arrange
    const input = 'test';

    // Act
    const result = functionUnderTest(input);

    // Assert
    expect(result).toBe('expected');
  });
});
```

### Integration Test Template

```javascript
const { createMockScanResult } = require('../helpers/chrome-mocks');

describe('Integration Feature', () => {
  let OrynCore;

  beforeAll(async () => {
    const wasmModule = await import('../helpers/wasm-mock');
    OrynCore = wasmModule.OrynCore;
  });

  test('should integrate components', () => {
    const core = new OrynCore();
    const scan = createMockScanResult();

    core.updateScan(JSON.stringify(scan));
    const result = core.processCommand('observe');

    expect(JSON.parse(result)).toHaveProperty('Resolved');
  });
});
```

### E2E Test Template

```javascript
const puppeteer = require('puppeteer');
const path = require('path');

const EXTENSION_PATH = path.resolve(__dirname, '../..');

describe('E2E Feature', () => {
  let browser;
  let page;

  beforeAll(async () => {
    browser = await puppeteer.launch({
      headless: false,
      args: [`--load-extension=${EXTENSION_PATH}`]
    });
  });

  test('should work end-to-end', async () => {
    page = await browser.newPage();
    await page.goto('file://' + fixturePath);

    const result = await page.evaluate(async () => {
      return await chrome.runtime.sendMessage({
        type: 'execute_oil',
        oil: 'observe'
      });
    });

    expect(result.error).toBeUndefined();
  });
});
```

## Debugging Tests

### View test output
```bash
npm test -- --verbose
```

### Debug specific test
```bash
node --inspect-brk node_modules/.bin/jest test/unit/background.test.js
```

### Check Chrome extension in E2E
E2E tests run with `headless: false`, so you can see the browser window. Add `await page.waitForTimeout(10000)` to pause execution and inspect the extension.

### Console logs
Tests mock `console` by default. To see actual console output:
```javascript
beforeEach(() => {
  console.log = jest.fn().mockImplementation((...args) => {
    process.stdout.write(args.join(' ') + '\n');
  });
});
```

## Custom Matchers

The test suite includes custom Jest matchers:

### `toHaveBeenCalledWithMessage(type, data)`

Checks if a Chrome message handler was called with specific message type and data.

```javascript
expect(chrome.runtime.sendMessage).toHaveBeenCalledWithMessage(
  'execute_oil',
  { oil: 'observe' }
);
```

## Best Practices

1. **Prefer unit tests** - They're fast and catch most bugs
2. **Mock Chrome APIs** - Use helpers in `chrome-mocks.js`
3. **Keep E2E tests minimal** - They're slow and flaky
4. **Use descriptive test names** - `should process click command` not `test 1`
5. **Follow AAA pattern** - Arrange, Act, Assert
6. **Clean up after tests** - Use `afterEach` to reset state
7. **Avoid test interdependence** - Each test should run independently
8. **Test error cases** - Don't just test happy paths

## Continuous Integration

Tests are designed to run in CI environments:

- Unit and integration tests run in Node.js
- E2E tests require Chrome installation
- WASM module must be built before integration/E2E tests
- Use `CI=true` environment variable for E2E tests

## Troubleshooting

### "WASM module not found"
Build the WASM module first:
```bash
cd ..
./scripts/build-wasm.sh
cd extension-w
```

### "Chrome binary not found" (E2E)
Install Chrome or set `PUPPETEER_SKIP_CHROMIUM_DOWNLOAD=false`:
```bash
npm install puppeteer
```

### "Extension failed to load" (E2E)
Check that all extension files exist:
```bash
ls -la manifest.json background.js popup.html
```

### Tests hang or timeout
Increase Jest timeout:
```javascript
jest.setTimeout(10000); // 10 seconds
```

Or in specific test:
```javascript
test('slow test', async () => {
  // test code
}, 15000); // 15 second timeout
```

## Performance

- Unit tests: ~2-5 seconds for full suite
- Integration tests: ~5-10 seconds for full suite
- E2E tests: ~30-60 seconds for full suite

Total test time: ~1-2 minutes

## Resources

- [Jest Documentation](https://jestjs.io/)
- [Puppeteer Documentation](https://pptr.dev/)
- [sinon-chrome Documentation](https://github.com/acvetkov/sinon-chrome)
- [Chrome Extension Testing Guide](https://developer.chrome.com/docs/extensions/mv3/tut_testing/)
