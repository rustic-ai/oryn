/**
 * E2E tests for extension loading and initialization
 * Uses Puppeteer to test extension in real Chrome
 *
 * @jest-environment node
 */

const puppeteer = require('puppeteer');
const path = require('path');
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));

const EXTENSION_PATH = path.resolve(__dirname, '../..');
const TEST_PAGE_URL = 'file://' + path.resolve(__dirname, '../fixtures/static-page.html');

describe('Extension Loading E2E', () => {
  let browser;
  let page;
  let backgroundPage;

  beforeAll(async () => {
    let lastError;

    // Chromium can exit while Puppeteer attaches to a just-created extension
    // service worker. Retry the complete browser launch so a dead CDP session is
    // never reused, while keeping the setup bounded.
    for (let attempt = 1; attempt <= 3; attempt += 1) {
      try {
        browser = await puppeteer.launch({
          headless: false, // Extensions only work in headed mode
          args: [
            `--disable-extensions-except=${EXTENSION_PATH}`,
            `--load-extension=${EXTENSION_PATH}`,
            '--no-sandbox',
            '--disable-setuid-sandbox'
          ]
        });

        const backgroundTarget = await browser.waitForTarget(
          target =>
            target.type() === 'service_worker' &&
            target.url().includes('chrome-extension://'),
          { timeout: 10000 }
        );

        backgroundPage = await backgroundTarget.worker();
        page = await browser.newPage();
        return;
      } catch (error) {
        lastError = error;
        if (browser) {
          await browser.close().catch(() => {});
        }
        browser = undefined;
        backgroundPage = undefined;
        page = undefined;
      }
    }

    throw new Error(`Unable to attach to extension service worker: ${lastError}`);
  }, 60000);

  afterAll(async () => {
    if (browser) {
      await browser.close();
    }
  });

  test('should load extension successfully', async () => {
    // Navigate to chrome://extensions
    await page.goto('chrome://extensions');

    // Wait for extensions page to load
    await delay(1000);

    // Extension should be visible (this is a basic check)
    const title = await page.title();
    expect(title).toContain('Extensions');
  });

  test('should initialize WASM module', async () => {
    // Navigate to test page
    await page.goto(TEST_PAGE_URL);
    await delay(500);

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
    await delay(500);

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
