/**
 * Real WASM Module Tests with Extension Loaded
 * Tests the actual WASM module running inside the browser extension
 *
 * This is different from wasm-real.test.js which tests WASM in isolation.
 * Here we test WASM as it actually runs in the extension's service worker.
 *
 * @jest-environment node
 */

const puppeteer = require('puppeteer');
const path = require('path');
const fs = require('fs');

const EXTENSION_DIR = path.resolve(__dirname, '../..');
const WASM_FILE = path.join(EXTENSION_DIR, 'wasm/oryn_core_bg.wasm');

// Check if WASM is built and extension files exist
beforeAll(() => {
  if (!fs.existsSync(WASM_FILE)) {
    throw new Error(
      'WASM module not found. Build it first:\n' +
      '  cd ../.. && ./scripts/build-wasm.sh'
    );
  }

  const requiredFiles = ['manifest.json', 'background.js', 'scanner.js'];
  for (const file of requiredFiles) {
    if (!fs.existsSync(path.join(EXTENSION_DIR, file))) {
      throw new Error(`Required extension file not found: ${file}`);
    }
  }
});

describe('Extension WASM Integration', () => {
  let browser;
  let backgroundPage;

  beforeAll(async () => {
    // Launch Chromium with extension loaded
    browser = await puppeteer.launch({
      headless: false, // Extensions don't work in headless mode
      args: [
        `--disable-extensions-except=${EXTENSION_DIR}`,
        `--load-extension=${EXTENSION_DIR}`,
        '--no-sandbox',
        '--disable-setuid-sandbox',
        '--disable-dev-shm-usage'
      ]
    });

    // Wait for extension to load
    await new Promise(resolve => setTimeout(resolve, 2000));

    // Get background service worker page
    const targets = await browser.targets();
    const backgroundTarget = targets.find(
      target => target.type() === 'service_worker' &&
                target.url().includes('chrome-extension://')
    );

    if (!backgroundTarget) {
      throw new Error('Extension background service worker not found. Extension may have failed to load.');
    }

    backgroundPage = await backgroundTarget.worker();
  }, 30000);

  afterAll(async () => {
    if (browser) {
      await browser.close();
    }
  });

  describe('WASM Initialization in Extension', () => {
    test('should initialize WASM module in background script', async () => {
      const result = await backgroundPage.evaluate(() => {
        return {
          isInitialized: typeof globalThis.isWasmInitialized !== 'undefined' ? globalThis.isWasmInitialized : null,
          hasOrynCore: typeof globalThis.orynCore !== 'undefined',
          hasInit: typeof globalThis.wasmInit !== 'undefined'
        };
      });

      // The extension should have initialized WASM
      expect(result.isInitialized).toBe(true);
      expect(result.hasOrynCore).toBe(true);
    }, 10000);

    test('should have OrynCore instance available', async () => {
      const result = await backgroundPage.evaluate(() => {
        if (!globalThis.orynCore) {
          return { available: false };
        }

        return {
          available: true,
          hasMethods: typeof globalThis.orynCore.processCommand === 'function' &&
                     typeof globalThis.orynCore.updateScan === 'function'
        };
      });

      expect(result.available).toBe(true);
      expect(result.hasMethods).toBe(true);
    }, 10000);

    test('should get version from WASM module', async () => {
      const result = await backgroundPage.evaluate(() => {
        try {
          // Access the OrynCore class from the imported module
          if (!globalThis.OrynCoreClass) {
            return { success: false, error: 'OrynCore class not found' };
          }
          const version = globalThis.OrynCoreClass.getVersion();
          return { success: true, version };
        } catch (e) {
          return { success: false, error: e.message };
        }
      });

      expect(result.success).toBe(true);
      if (result.version) {
        expect(result.version).toMatch(/^\d+\.\d+\.\d+$/);
      }
    }, 10000);
  });

  describe('Extension Message Flow with WASM', () => {
    let testPage;

    beforeEach(async () => {
      // Create a test page for the extension to interact with
      testPage = await browser.newPage();
      await testPage.goto('https://example.com');
      await testPage.waitForTimeout(1000); // Wait for content scripts
    });

    afterEach(async () => {
      if (testPage) {
        await testPage.close();
      }
    });

    test('should handle get_status message', async () => {
      // Test status directly from background context
      const result = await backgroundPage.evaluate(() => {
        // Access the variables that get_status would return
        return {
          wasmInitialized: globalThis.isWasmInitialized,
          hasOrynCore: globalThis.orynCore !== null && globalThis.orynCore !== undefined
        };
      });

      expect(result).toHaveProperty('wasmInitialized');
      expect(result.wasmInitialized).toBe(true);
      expect(result.hasOrynCore).toBe(true);
    }, 10000);

    test('should process observe command through extension', async () => {
      // First ensure we have a scan loaded
      await backgroundPage.evaluate(() => {
        const mockScan = {
          page: { url: 'https://example.com', title: 'Test', viewport: { width: 1920, height: 1080 }, scroll: { x: 0, y: 0 } },
          elements: [{ id: 1, selector: '#test', type: 'button', text: 'Test', attributes: {}, rect: { x: 0, y: 0, width: 100, height: 30 } }],
          stats: { total: 1, scanned: 1 }
        };
        if (globalThis.orynCore) {
          globalThis.orynCore.updateScan(JSON.stringify(mockScan));
        }
      });

      // Now test the command from background context
      const result = await backgroundPage.evaluate(async () => {
        try {
          if (!globalThis.orynCore) {
            return { success: false, error: 'orynCore not available' };
          }
          const resultStr = globalThis.orynCore.processCommand('observe');
          const parsed = JSON.parse(resultStr);
          return { success: true, response: parsed };
        } catch (e) {
          return { success: false, error: e.message, stack: e.stack };
        }
      });

      // The command should succeed with scan loaded
      expect(result).toBeDefined();
      expect(result.success).toBe(true);
    }, 10000);

    test('should handle scan_complete message', async () => {
      const mockScan = {
        page: {
          url: 'https://example.com',
          title: 'Example',
          viewport: { width: 1920, height: 1080 },
          scroll: { x: 0, y: 0 }
        },
        elements: [{
          id: 1,
          selector: '#test',
          type: 'button',
          text: 'Test',
          attributes: {},
          rect: { x: 0, y: 0, width: 100, height: 30 }
        }],
        stats: { total: 1, scanned: 1 }
      };

      // Test scan update directly
      const result = await backgroundPage.evaluate((scan) => {
        try {
          if (!globalThis.orynCore) {
            return { success: false, error: 'orynCore not available' };
          }
          globalThis.orynCore.updateScan(JSON.stringify(scan));
          return { success: true, ok: true };
        } catch (e) {
          return { success: false, error: e.message };
        }
      }, mockScan);

      expect(result.success).toBe(true);
      expect(result.ok).toBe(true);
    }, 10000);
  });

  describe('WASM Command Processing in Extension', () => {
    test('should process commands through background script', async () => {
      const result = await backgroundPage.evaluate(async () => {
        try {
          // First update scan
          const mockScan = {
            page: { url: 'https://example.com', title: 'Test', viewport: { width: 1920, height: 1080 }, scroll: { x: 0, y: 0 } },
            elements: [{ id: 1, selector: '#test', type: 'button', text: 'Test', attributes: {}, rect: { x: 0, y: 0, width: 100, height: 30 } }],
            stats: { total: 1, scanned: 1 }
          };

          if (globalThis.orynCore) {
            globalThis.orynCore.updateScan(JSON.stringify(mockScan));

            // Process command
            const resultStr = globalThis.orynCore.processCommand('observe');
            const parsed = JSON.parse(resultStr);

            return { success: true, result: parsed };
          } else {
            return { success: false, error: 'orynCore not available' };
          }
        } catch (e) {
          return { success: false, error: e.message, stack: e.stack };
        }
      });

      if (!result.success) {
        console.error('Command processing failed:', result.error);
        if (result.stack) console.error('Stack:', result.stack);
      }

      expect(result.success).toBe(true);
      if (result.success) {
        expect(result.result).toHaveProperty('Resolved');
      }
    }, 10000);

    test('should handle different command types', async () => {
      // First ensure scan is loaded
      await backgroundPage.evaluate(() => {
        const mockScan = {
          page: { url: 'https://example.com', title: 'Test', viewport: { width: 1920, height: 1080 }, scroll: { x: 0, y: 0 } },
          elements: [{ id: 1, selector: '#test', type: 'button', text: 'Test', attributes: {}, rect: { x: 0, y: 0, width: 100, height: 30 } }],
          stats: { total: 1, scanned: 1 }
        };
        if (globalThis.orynCore) {
          globalThis.orynCore.updateScan(JSON.stringify(mockScan));
        }
      });

      const commands = [
        'observe',
        'goto "https://test.com"',
        'click "Test"'
      ];

      for (const cmd of commands) {
        const result = await backgroundPage.evaluate(async (command) => {
          try {
            if (!globalThis.orynCore) {
              return { success: false, error: 'orynCore not available' };
            }

            const resultStr = globalThis.orynCore.processCommand(command);
            const parsed = JSON.parse(resultStr);

            return { success: true, result: parsed, command };
          } catch (e) {
            return { success: false, error: e.message, stack: e.stack, command };
          }
        }, cmd);

        if (!result.success) {
          console.error(`Command "${cmd}" failed:`, result.error);
          if (result.stack) console.error('Stack:', result.stack);
        }

        expect(result.success).toBe(true);
        expect(result.result).toHaveProperty('Resolved');
      }
    }, 15000);
  });

  describe('Performance in Extension Context', () => {
    test('should process commands quickly in extension', async () => {
      const result = await backgroundPage.evaluate(() => {
        try {
          if (!globalThis.orynCore) {
            return { success: false, error: 'orynCore not available' };
          }

          // Ensure scan is loaded
          const mockScan = {
            page: { url: 'https://example.com', title: 'Test', viewport: { width: 1920, height: 1080 }, scroll: { x: 0, y: 0 } },
            elements: [],
            stats: { total: 0, scanned: 0 }
          };
          globalThis.orynCore.updateScan(JSON.stringify(mockScan));

          // Measure performance
          const start = performance.now();
          for (let i = 0; i < 100; i++) {
            globalThis.orynCore.processCommand('observe');
          }
          const duration = performance.now() - start;

          return {
            success: true,
            duration,
            avgTime: duration / 100,
            commandsPerSecond: 100 / (duration / 1000)
          };
        } catch (e) {
          return { success: false, error: e.message };
        }
      });

      expect(result.success).toBe(true);
      expect(result.avgTime).toBeLessThan(10); // Less than 10ms per command in extension
      console.log(`Performance: ${result.commandsPerSecond.toFixed(0)} commands/second`);
    }, 15000);
  });

  describe('Extension Console Logs', () => {
    test('should not have WASM initialization errors', async () => {
      const logs = [];

      backgroundPage.on('console', msg => {
        logs.push({ type: msg.type(), text: msg.text() });
      });

      // Trigger some activity
      await backgroundPage.evaluate(() => {
        console.log('[Test] Checking for errors');
      });

      await new Promise(resolve => setTimeout(resolve, 1000));

      // Check for error logs
      const errors = logs.filter(log => log.type === 'error');
      const wasmErrors = errors.filter(log =>
        log.text.toLowerCase().includes('wasm') ||
        log.text.toLowerCase().includes('oryn')
      );

      if (wasmErrors.length > 0) {
        console.log('WASM-related errors found:', wasmErrors);
      }

      // We expect no WASM-related errors
      expect(wasmErrors.length).toBe(0);
    }, 10000);
  });
});
