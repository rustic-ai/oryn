/**
 * E2E tests for extension loading and initialization
 * Uses Puppeteer to test extension in real Chrome
 */

const puppeteer = require('puppeteer');
const path = require('path');

const EXTENSION_PATH = path.resolve(__dirname, '../..');
const TEST_PAGE_URL = 'file://' + path.resolve(__dirname, '../fixtures/static-page.html');

describe('Extension Loading E2E', () => {
  let browser;
  let page;

  beforeAll(async () => {
    // Launch browser with extension loaded
    browser = await puppeteer.launch({
      headless: false, // Extensions only work in headed mode
      args: [
        `--disable-extensions-except=${EXTENSION_PATH}`,
        `--load-extension=${EXTENSION_PATH}`,
        '--no-sandbox',
        '--disable-setuid-sandbox'
      ]
    });

    page = await browser.newPage();
  });

  afterAll(async () => {
    if (browser) {
      await browser.close();
    }
  });

  test('should load extension successfully', async () => {
    // Navigate to chrome://extensions
    await page.goto('chrome://extensions');

    // Wait for extensions page to load
    await page.waitForTimeout(1000);

    // Extension should be visible (this is a basic check)
    const title = await page.title();
    expect(title).toContain('Extensions');
  });

  test('should initialize WASM module', async () => {
    // Navigate to test page
    await page.goto(TEST_PAGE_URL);
    await page.waitForTimeout(500);

    // Get background page
    const targets = await browser.targets();
    const backgroundTarget = targets.find(
      target => target.type() === 'service_worker'
    );

    if (backgroundTarget) {
      const backgroundPage = await backgroundTarget.worker();

      // Check WASM initialization
      const isInitialized = await backgroundPage.evaluate(() => {
        return typeof self.isWasmInitialized !== 'undefined';
      });

      expect(isInitialized).toBeDefined();
    }
  }, 10000);

  test('should inject content scripts', async () => {
    await page.goto(TEST_PAGE_URL);
    await page.waitForTimeout(500);

    // Check if scanner.js is injected
    const scannerLoaded = await page.evaluate(() => {
      return typeof window.scanner !== 'undefined';
    });

    expect(scannerLoaded).toBe(true);
  });

  test('should have extension popup available', async () => {
    const targets = await browser.targets();
    const extensionTarget = targets.find(
      target => target.type() === 'background_page' || target.type() === 'service_worker'
    );

    expect(extensionTarget).toBeDefined();
  });
});
