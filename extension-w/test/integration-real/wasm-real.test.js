/**
 * Real WASM Module Integration Tests
 * Tests the actual WASM module in a browser environment using Puppeteer
 *
 * These tests load the real oryn_core WASM module and verify it works correctly.
 * Unlike the mock-based integration tests, these test the actual Rust→WASM compilation.
 */

const puppeteer = require('puppeteer');
const path = require('path');
const fs = require('fs');

const WASM_DIR = path.resolve(__dirname, '../../wasm');
const WASM_FILE = path.join(WASM_DIR, 'oryn_core_bg.wasm');
const WASM_JS = path.join(WASM_DIR, 'oryn_core.js');

// Check if WASM is built before running tests
beforeAll(() => {
  if (!fs.existsSync(WASM_FILE)) {
    throw new Error(
      'WASM module not found. Build it first:\n' +
      '  cd ../.. && ./scripts/build-wasm.sh'
    );
  }
});

describe('Real WASM Module Integration', () => {
  let browser;
  let page;

  beforeAll(async () => {
    browser = await puppeteer.launch({
      headless: 'new',
      args: ['--no-sandbox', '--disable-setuid-sandbox']
    });
  }, 30000);

  afterAll(async () => {
    if (browser) {
      await browser.close();
    }
  });

  beforeEach(async () => {
    page = await browser.newPage();

    // Expose helper to create test page
    await page.setContent(`
      <!DOCTYPE html>
      <html>
      <head>
        <title>WASM Test</title>
      </head>
      <body>
        <div id="output"></div>
      </body>
      </html>
    `);

    // Add WASM files to page
    await page.addScriptTag({
      path: WASM_JS,
      type: 'module'
    });
  });

  afterEach(async () => {
    if (page) {
      await page.close();
    }
  });

  describe('Module Loading', () => {
    test('should load WASM module', async () => {
      const result = await page.evaluate(async () => {
        try {
          const module = await import('/wasm/oryn_core.js');
          return { success: true, hasInit: typeof module.default === 'function' };
        } catch (e) {
          return { success: false, error: e.message };
        }
      });

      expect(result.success).toBe(true);
    }, 10000);

    test('should initialize WASM module', async () => {
      const result = await page.evaluate(async () => {
        try {
          const module = await import('/wasm/oryn_core.js');
          const init = module.default;
          await init();
          return { success: true };
        } catch (e) {
          return { success: false, error: e.message };
        }
      });

      expect(result.success).toBe(true);
    }, 10000);

    test('should create OrynCore instance', async () => {
      const result = await page.evaluate(async () => {
        try {
          const module = await import('/wasm/oryn_core.js');
          await module.default();
          const { OrynCore } = module;
          const core = new OrynCore();
          return {
            success: true,
            hasMethods: typeof core.processCommand === 'function' &&
                       typeof core.updateScan === 'function'
          };
        } catch (e) {
          return { success: false, error: e.message };
        }
      });

      expect(result.success).toBe(true);
      expect(result.hasMethods).toBe(true);
    }, 10000);

    test('should get version information', async () => {
      const result = await page.evaluate(async () => {
        try {
          const module = await import('/wasm/oryn_core.js');
          await module.default();
          const { OrynCore } = module;
          const version = OrynCore.getVersion();
          return { success: true, version };
        } catch (e) {
          return { success: false, error: e.message };
        }
      });

      expect(result.success).toBe(true);
      expect(result.version).toMatch(/^\d+\.\d+\.\d+$/);
    }, 10000);
  });

  describe('Scan Management', () => {
    test('should update scan context', async () => {
      const result = await page.evaluate(async () => {
        try {
          const module = await import('/wasm/oryn_core.js');
          await module.default();
          const { OrynCore } = module;
          const core = new OrynCore();

          const scan = {
            page: {
              url: 'https://example.com',
              title: 'Test',
              viewport: { width: 1920, height: 1080 },
              scroll: { x: 0, y: 0 }
            },
            elements: [{
              id: 1,
              selector: '#submit',
              element_type: 'button',
              text: 'Submit',
              attributes: {},
              rect: { x: 0, y: 0, width: 100, height: 30 },
              label: null,
              placeholder: null,
              value: null,
              checked: null,
              href: null
            }],
            stats: { total: 1, scanned: 1 },
            patterns: null,
            changes: null,
            available_intents: null
          };

          core.updateScan(JSON.stringify(scan));
          return { success: true };
        } catch (e) {
          return { success: false, error: e.message };
        }
      });

      expect(result.success).toBe(true);
    }, 10000);

    test('should reject invalid scan JSON', async () => {
      const result = await page.evaluate(async () => {
        try {
          const module = await import('/wasm/oryn_core.js');
          await module.default();
          const { OrynCore } = module;
          const core = new OrynCore();

          try {
            core.updateScan('invalid json');
            return { success: false, shouldHaveThrown: true };
          } catch (e) {
            return { success: true, errorMessage: e.message };
          }
        } catch (e) {
          return { success: false, error: e.message };
        }
      });

      expect(result.success).toBe(true);
      expect(result.errorMessage).toBeDefined();
    }, 10000);
  });

  describe('Command Processing', () => {
    test('should process observe command', async () => {
      const result = await page.evaluate(async () => {
        try {
          const module = await import('/wasm/oryn_core.js');
          await module.default();
          const { OrynCore } = module;
          const core = new OrynCore();

          const scan = {
            page: { url: 'https://example.com', title: 'Test', viewport: { width: 1920, height: 1080 }, scroll: { x: 0, y: 0 } },
            elements: [{ id: 1, selector: '#submit', element_type: 'button', text: 'Submit', attributes: {}, rect: { x: 0, y: 0, width: 100, height: 30 } }],
            stats: { total: 1, scanned: 1 }
          };

          core.updateScan(JSON.stringify(scan));
          const resultStr = core.processCommand('observe');
          const parsed = JSON.parse(resultStr);

          return { success: true, result: parsed };
        } catch (e) {
          return { success: false, error: e.message };
        }
      });

      expect(result.success).toBe(true);
      expect(result.result).toHaveProperty('Resolved');
    }, 10000);

    test('should process goto command', async () => {
      const result = await page.evaluate(async () => {
        try {
          const module = await import('/wasm/oryn_core.js');
          await module.default();
          const { OrynCore } = module;
          const core = new OrynCore();

          const scan = {
            page: { url: 'https://example.com', title: 'Test', viewport: { width: 1920, height: 1080 }, scroll: { x: 0, y: 0 } },
            elements: [],
            stats: { total: 0, scanned: 0 }
          };

          core.updateScan(JSON.stringify(scan));
          const resultStr = core.processCommand('goto "https://test.com"');
          const parsed = JSON.parse(resultStr);

          return { success: true, result: parsed };
        } catch (e) {
          return { success: false, error: e.message };
        }
      });

      expect(result.success).toBe(true);
      expect(result.result).toHaveProperty('Resolved');
      expect(result.result.Resolved).toHaveProperty('Browser');
    }, 10000);

    test('should require scan before processing', async () => {
      const result = await page.evaluate(async () => {
        try {
          const module = await import('/wasm/oryn_core.js');
          await module.default();
          const { OrynCore } = module;
          const core = new OrynCore();

          try {
            core.processCommand('observe');
            return { success: false, shouldHaveThrown: true };
          } catch (e) {
            return { success: true, errorMessage: e.message };
          }
        } catch (e) {
          return { success: false, error: e.message };
        }
      });

      expect(result.success).toBe(true);
      expect(result.errorMessage).toMatch(/scan/i);
    }, 10000);

    test('should handle invalid commands', async () => {
      const result = await page.evaluate(async () => {
        try {
          const module = await import('/wasm/oryn_core.js');
          await module.default();
          const { OrynCore } = module;
          const core = new OrynCore();

          const scan = {
            page: { url: 'https://example.com', title: 'Test', viewport: { width: 1920, height: 1080 }, scroll: { x: 0, y: 0 } },
            elements: [],
            stats: { total: 0, scanned: 0 }
          };

          core.updateScan(JSON.stringify(scan));

          try {
            core.processCommand('invalid command');
            return { success: false, shouldHaveThrown: true };
          } catch (e) {
            return { success: true, errorMessage: e.message };
          }
        } catch (e) {
          return { success: false, error: e.message };
        }
      });

      expect(result.success).toBe(true);
    }, 10000);
  });

  describe('Performance', () => {
    test('should process commands quickly', async () => {
      const result = await page.evaluate(async () => {
        try {
          const module = await import('/wasm/oryn_core.js');
          await module.default();
          const { OrynCore } = module;
          const core = new OrynCore();

          const scan = {
            page: { url: 'https://example.com', title: 'Test', viewport: { width: 1920, height: 1080 }, scroll: { x: 0, y: 0 } },
            elements: [],
            stats: { total: 0, scanned: 0 }
          };

          core.updateScan(JSON.stringify(scan));

          const start = performance.now();
          for (let i = 0; i < 100; i++) {
            core.processCommand('observe');
          }
          const duration = performance.now() - start;

          return { success: true, duration, avgTime: duration / 100 };
        } catch (e) {
          return { success: false, error: e.message };
        }
      });

      expect(result.success).toBe(true);
      expect(result.avgTime).toBeLessThan(5); // Less than 5ms per command
    }, 15000);
  });
});
