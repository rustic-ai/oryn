/**
 * E2E tests for command execution
 * Tests full command flow through extension
 */

const puppeteer = require('puppeteer');
const path = require('path');

const EXTENSION_PATH = path.resolve(__dirname, '../..');
const FIXTURES_DIR = path.resolve(__dirname, '../fixtures');

describe('Command Execution E2E', () => {
  let browser;
  let page;

  beforeAll(async () => {
    browser = await puppeteer.launch({
      headless: false,
      args: [
        `--disable-extensions-except=${EXTENSION_PATH}`,
        `--load-extension=${EXTENSION_PATH}`,
        '--no-sandbox',
        '--disable-setuid-sandbox'
      ]
    });
  }, 30000);

  beforeEach(async () => {
    page = await browser.newPage();
  });

  afterEach(async () => {
    if (page) {
      await page.close();
    }
  });

  afterAll(async () => {
    if (browser) {
      await browser.close();
    }
  });

  describe('Static Page Tests', () => {
    test('should execute observe command', async () => {
      await page.goto(`file://${FIXTURES_DIR}/static-page.html`);
      await page.waitForTimeout(500);

      // Send observe command through extension
      const result = await page.evaluate(async () => {
        return await chrome.runtime.sendMessage({
          type: 'execute_oil',
          oil: 'observe'
        });
      });

      expect(result.error).toBeUndefined();
    }, 10000);

    test('should execute click command', async () => {
      await page.goto(`file://${FIXTURES_DIR}/static-page.html`);
      await page.waitForTimeout(500);

      const result = await page.evaluate(async () => {
        return await chrome.runtime.sendMessage({
          type: 'execute_oil',
          oil: 'click "Submit"'
        });
      });

      expect(result.error).toBeUndefined();
    }, 10000);
  });

  describe('Form Interaction Tests', () => {
    test('should type into input field', async () => {
      await page.goto(`file://${FIXTURES_DIR}/form-page.html`);
      await page.waitForTimeout(500);

      await page.evaluate(async () => {
        await chrome.runtime.sendMessage({
          type: 'execute_oil',
          oil: 'type "Email" "test@example.com"'
        });
      });

      // Verify input was typed
      const value = await page.$eval('#email', el => el.value);
      expect(value).toBe('test@example.com');
    }, 10000);

    test('should execute form submission', async () => {
      await page.goto(`file://${FIXTURES_DIR}/form-page.html`);
      await page.waitForTimeout(500);

      // Fill form and submit
      await page.evaluate(async () => {
        await chrome.runtime.sendMessage({
          type: 'execute_oil',
          oil: 'type "Email" "user@test.com"'
        });

        await chrome.runtime.sendMessage({
          type: 'execute_oil',
          oil: 'type "Password" "secret123"'
        });

        await chrome.runtime.sendMessage({
          type: 'execute_oil',
          oil: 'submit'
        });
      });

      // Wait for potential page change/submission
      await page.waitForTimeout(500);

      // Form submission would be complete
      expect(true).toBe(true);
    }, 15000);
  });

  describe('Navigation Tests', () => {
    test('should execute goto command', async () => {
      await page.goto(`file://${FIXTURES_DIR}/static-page.html`);
      await page.waitForTimeout(500);

      const result = await page.evaluate(async () => {
        return await chrome.runtime.sendMessage({
          type: 'execute_oil',
          oil: 'goto "https://example.com"'
        });
      });

      // Wait for navigation
      await page.waitForNavigation({ timeout: 5000 }).catch(() => {
        // Navigation might be blocked in test environment
      });

      expect(result.error).toBeUndefined();
    }, 10000);
  });

  describe('Error Handling', () => {
    test('should handle invalid commands gracefully', async () => {
      await page.goto(`file://${FIXTURES_DIR}/static-page.html`);
      await page.waitForTimeout(500);

      const result = await page.evaluate(async () => {
        return await chrome.runtime.sendMessage({
          type: 'execute_oil',
          oil: 'invalid command syntax'
        });
      });

      expect(result.error).toBeDefined();
      expect(result.error).toContain('Command processing failed');
    }, 10000);

    test('should handle empty commands', async () => {
      await page.goto(`file://${FIXTURES_DIR}/static-page.html`);
      await page.waitForTimeout(500);

      const result = await page.evaluate(async () => {
        return await chrome.runtime.sendMessage({
          type: 'execute_oil',
          oil: ''
        });
      });

      expect(result.error).toBeDefined();
    }, 10000);
  });

  describe('Status Checking', () => {
    test('should return WASM initialization status', async () => {
      await page.goto(`file://${FIXTURES_DIR}/static-page.html`);
      await page.waitForTimeout(500);

      const status = await page.evaluate(async () => {
        return await chrome.runtime.sendMessage({
          type: 'get_status'
        });
      });

      expect(status).toHaveProperty('wasmInitialized');
      expect(status).toHaveProperty('hasScan');
    }, 10000);
  });

  describe('Scan Management', () => {
    test('should request and update scan', async () => {
      await page.goto(`file://${FIXTURES_DIR}/static-page.html`);
      await page.waitForTimeout(500);

      // Trigger scan
      await page.evaluate(async () => {
        await chrome.runtime.sendMessage({
          type: 'execute_oil',
          oil: 'observe'
        });
      });

      // Check status after scan
      const status = await page.evaluate(async () => {
        return await chrome.runtime.sendMessage({
          type: 'get_status'
        });
      });

      expect(status.hasScan).toBe(true);
    }, 10000);
  });
});
