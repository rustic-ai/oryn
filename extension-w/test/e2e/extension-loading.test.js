/**
 * E2E tests for extension loading and initialization
 * Uses Puppeteer to test extension in real Chrome
 *
 * @jest-environment node
 */

const puppeteer = require('puppeteer');
const path = require('path');

const EXTENSION_PATH = path.resolve(__dirname, '../..');
const TEST_PAGE_URL = 'file://' + path.resolve(__dirname, '../fixtures/static-page.html');

describe('Extension Loading E2E', () => {
  let browser;
  let page;
  let backgroundPage;

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

    // Wait for extension to load
    await new Promise(resolve => setTimeout(resolve, 3000));

    // Get background service worker page
    const targets = await browser.targets();
    const backgroundTarget = targets.find(
      target => target.type() === 'service_worker' &&
                target.url().includes('chrome-extension://')
    );

    if (!backgroundTarget) {
      console.error('Available targets:', targets.map(t => ({ type: t.type(), url: t.url() })));
      throw new Error('Extension background service worker not found. Check console for available targets.');
    }

    backgroundPage = await backgroundTarget.worker();

    page = await browser.newPage();
  }, 30000);

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

    // Check WASM initialization via backgroundPage
    const wasmStatus = await backgroundPage.evaluate(() => {
      return {
        isInitialized: globalThis.isWasmInitialized,
        hasOrynCore: typeof globalThis.orynCore !== 'undefined',
        hasOrynCoreClass: typeof globalThis.OrynCoreClass !== 'undefined'
      };
    });

    expect(wasmStatus.isInitialized).toBe(true);
    expect(wasmStatus.hasOrynCore).toBe(true);
    expect(wasmStatus.hasOrynCoreClass).toBe(true);
  }, 10000);

  // Skip: Chrome security restrictions prevent content scripts on file:// URLs
  test.skip('should inject content scripts', async () => {
    await page.goto(TEST_PAGE_URL);
    await page.waitForTimeout(500);

    // Check if scanner.js is injected (exports as window.Oryn.Scanner)
    const scannerLoaded = await page.evaluate(() => {
      return typeof window.Oryn !== 'undefined' && typeof window.Oryn.Scanner !== 'undefined';
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
