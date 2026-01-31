#!/usr/bin/env node
/**
 * Test extension-w wizard with Puppeteer
 * Automates the wizard flow and captures all console logs
 */

const puppeteer = require('puppeteer');
const path = require('path');

const PROJECT_ROOT = __dirname;
const EXT_DIR = path.join(PROJECT_ROOT, 'extension-w');

// Color helpers
const colors = {
  reset: '\x1b[0m',
  red: '\x1b[31m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  cyan: '\x1b[36m',
  magenta: '\x1b[35m',
};

function log(color, prefix, msg) {
  const timestamp = new Date().toISOString().split('T')[1].slice(0, -1);
  console.log(`${colors.cyan}[${timestamp}]${colors.reset} ${color}[${prefix}]${colors.reset} ${msg}`);
}

const logInfo = (msg) => log(colors.blue, 'INFO', msg);
const logPass = (msg) => log(colors.green, 'PASS', msg);
const logFail = (msg) => log(colors.red, 'FAIL', msg);
const logWarn = (msg) => log(colors.yellow, 'WARN', msg);
const logDebug = (msg) => log(colors.magenta, 'DEBUG', msg);

// Store logs
const serviceLogs = [];
const wizardLogs = [];
const errors = [];

async function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

async function testWizard() {
  console.log('\n' + colors.cyan + '╔════════════════════════════════════════════════════════════════╗' + colors.reset);
  console.log(colors.cyan + '║        Testing Oryn-W First-Run Wizard (Automated)            ║' + colors.reset);
  console.log(colors.cyan + '╚════════════════════════════════════════════════════════════════╝' + colors.reset + '\n');

  let browser;

  try {
    logInfo('Launching browser with extension...');

    browser = await puppeteer.launch({
      headless: false,
      args: [
        `--disable-extensions-except=${EXT_DIR}`,
        `--load-extension=${EXT_DIR}`,
        '--no-first-run',
        '--no-default-browser-check',
        '--window-size=1400,900',
      ],
      devtools: false, // We'll handle logging programmatically
    });

    logPass('Browser launched');

    // Wait for extension to load
    await sleep(3000);

    // Get all targets
    let targets = await browser.targets();
    logInfo(`Found ${targets.length} targets: ${targets.map(t => t.type()).join(', ')}`);

    // Find service worker (try multiple times)
    let serviceWorkerTarget = null;
    let extensionId = null;

    for (let attempt = 0; attempt < 10; attempt++) {
      targets = await browser.targets();
      serviceWorkerTarget = targets.find(
        t => t.type() === 'service_worker' && t.url().includes('chrome-extension://')
      );

      if (serviceWorkerTarget) {
        const url = serviceWorkerTarget.url();
        extensionId = url.split('/')[2];
        logPass(`Found service worker: ${extensionId}`);

        try {
          // Try to listen to service worker console
          const worker = await serviceWorkerTarget.worker();
          if (worker) {
            worker.on('console', msg => {
              const text = msg.text();
              serviceLogs.push(text);
              if (text.includes('ERROR') || text.includes('error') || text.includes('Failed')) {
                logFail(`SW: ${text}`);
                errors.push(`Service Worker: ${text}`);
              } else if (text.includes('LLM Manager') || text.includes('setActiveAdapter')) {
                logDebug(`SW: ${text}`);
              }
            });

            logPass('Listening to service worker console');
          }
        } catch (err) {
          logWarn(`Could not attach to service worker: ${err.message}`);
        }
        break;
      }

      await sleep(1000);
    }

    if (!serviceWorkerTarget) {
      logWarn('Service worker not found - will try to find extension ID from pages');
      // Try to find extension ID from any extension page
      const extPages = targets.filter(t => t.url().startsWith('chrome-extension://'));
      if (extPages.length > 0) {
        extensionId = extPages[0].url().split('/')[2];
        logInfo(`Found extension ID from page: ${extensionId}`);
      }
    }

    // Wait for wizard to open (auto-opened by extension)
    logInfo('Waiting for wizard to open...');
    let wizardPage = null;

    for (let i = 0; i < 20; i++) {
      const pages = await browser.pages();
      wizardPage = pages.find(p => p.url().includes('first_run_wizard.html'));
      if (wizardPage) {
        logPass('Wizard auto-opened');
        if (!extensionId) {
          extensionId = wizardPage.url().split('/')[2];
          logInfo(`Extracted extension ID: ${extensionId}`);
        }
        break;
      }
      await sleep(500);
    }

    if (!wizardPage) {
      if (extensionId) {
        logWarn('Wizard did not auto-open, opening manually...');
        wizardPage = await browser.newPage();
        await wizardPage.goto(`chrome-extension://${extensionId}/ui/first_run_wizard.html`);
      } else {
        logFail('Could not find extension ID to open wizard');
        const allPages = await browser.pages();
        logInfo(`Available pages: ${allPages.map(p => p.url()).join(', ')}`);
        throw new Error('Cannot find wizard and no extension ID available');
      }
    }

    logPass('Wizard page ready');

    // Listen to wizard console
    wizardPage.on('console', msg => {
      const text = msg.text();
      wizardLogs.push(text);
      if (text.includes('ERROR') || text.includes('error') || text.includes('Failed')) {
        logFail(`Wizard: ${text}`);
        errors.push(`Wizard: ${text}`);
      } else if (text.includes('Wizard')) {
        logDebug(`Wizard: ${text}`);
      }
    });

    wizardPage.on('pageerror', err => {
      const text = err.toString();
      logFail(`Wizard Error: ${text}`);
      errors.push(`Wizard PageError: ${text}`);
    });

    // Wait for page to load
    await sleep(2000);

    // Take initial screenshot
    await wizardPage.screenshot({ path: 'wizard-step1.png', fullPage: true });
    logInfo('Screenshot: wizard-step1.png');

    // ===== STEP 1: Hardware Check =====
    logInfo('Step 1: Waiting for hardware check to complete...');
    await wizardPage.waitForSelector('#hw-results .hw-item', { timeout: 10000 });
    logPass('Hardware check completed');

    // Check for errors in hardware check
    const hwErrors = await wizardPage.$$('.hw-item.unavailable');
    if (hwErrors.length > 0) {
      logWarn(`Found ${hwErrors.length} unavailable hardware items`);
    }

    await sleep(1000);

    // Click Next to go to Step 2
    logInfo('Clicking Next to go to Step 2...');
    await wizardPage.click('#btn-next');
    await sleep(2000);

    await wizardPage.screenshot({ path: 'wizard-step2.png', fullPage: true });
    logInfo('Screenshot: wizard-step2.png');

    // ===== STEP 2: Choose Model =====
    logInfo('Step 2: Selecting WebLLM adapter...');

    // Wait for adapter options to appear
    await wizardPage.waitForSelector('.adapter-option', { timeout: 5000 });

    // Find and click WebLLM option
    const adapterOptions = await wizardPage.$$('.adapter-option');
    logInfo(`Found ${adapterOptions.length} adapter options`);

    let webllmOption = null;
    for (const option of adapterOptions) {
      const text = await wizardPage.evaluate(el => el.textContent, option);
      if (text.includes('WebLLM')) {
        webllmOption = option;
        break;
      }
    }

    if (!webllmOption) {
      logFail('WebLLM option not found!');
      throw new Error('WebLLM adapter not available');
    }

    logInfo('Clicking WebLLM adapter...');
    await webllmOption.click();
    await sleep(1000);

    // Check if model dropdown appeared
    const modelDropdown = await wizardPage.$('.model-dropdown');
    if (modelDropdown) {
      logPass('Model dropdown appeared');

      // Select a model
      logInfo('Selecting model from dropdown...');
      await wizardPage.select('.model-dropdown', 'Gemma-2B-it-q4f16_1');
      await sleep(500);

      const selectedModel = await wizardPage.$eval('.model-dropdown', el => el.value);
      logPass(`Selected model: ${selectedModel}`);
    } else {
      logWarn('Model dropdown did not appear');
    }

    await wizardPage.screenshot({ path: 'wizard-step2-selected.png', fullPage: true });
    logInfo('Screenshot: wizard-step2-selected.png');

    // Click Next to go to Step 3 (saves config + triggers download)
    logInfo('Clicking Next to go to Step 3...');
    await wizardPage.click('#btn-next');
    await sleep(3000);

    // Check if we're on step 3
    const step3Visible = await wizardPage.$eval('#step-3', el =>
      el.classList.contains('active')
    );

    if (step3Visible) {
      logPass('Advanced to Step 3');
      await wizardPage.screenshot({ path: 'wizard-step3.png', fullPage: true });
      logInfo('Screenshot: wizard-step3.png');

      // Get the summary text
      const summaryText = await wizardPage.$eval('#config-summary', el => el.textContent);
      logInfo(`Configuration summary: ${summaryText.trim().substring(0, 100)}...`);

      // Check if download section is visible (should be for WebLLM)
      const downloadVisible = await wizardPage.$eval('#download-section', el =>
        el.style.display !== 'none'
      );
      if (downloadVisible) {
        logPass('Download progress section is visible');
      } else {
        logWarn('Download progress section is NOT visible');
      }

      // Check if "Open Oryn" button is disabled during download
      const btnDisabled = await wizardPage.$eval('#btn-next', el => el.disabled);
      const btnText = await wizardPage.$eval('#btn-next', el => el.textContent);
      logInfo(`Open Oryn button: text="${btnText}", disabled=${btnDisabled}`);
      if (btnDisabled) {
        logPass('"Open Oryn" button is correctly disabled during download');
      } else {
        logWarn('"Open Oryn" button should be disabled during download');
      }

      // Monitor download progress for a bit
      logInfo('Monitoring download progress for 15 seconds...');
      for (let i = 0; i < 5; i++) {
        await sleep(3000);
        const percentText = await wizardPage.$eval('#download-percentage', el => el.textContent).catch(() => 'N/A');
        const statusText = await wizardPage.$eval('#download-status-text', el => el.textContent).catch(() => 'N/A');
        const errorVisible = await wizardPage.$eval('#download-error', el => el.style.display !== 'none').catch(() => false);
        logInfo(`Progress: ${percentText} | Status: ${statusText} | Error: ${errorVisible}`);

        if (errorVisible) {
          const errorText = await wizardPage.$eval('#download-error', el => el.textContent).catch(() => '');
          logWarn(`Download error displayed: ${errorText.trim().substring(0, 100)}`);
          break;
        }
      }

      await wizardPage.screenshot({ path: 'wizard-step3-progress.png', fullPage: true });
      logInfo('Screenshot: wizard-step3-progress.png');
    } else {
      logWarn('Did not advance to Step 3');
      await wizardPage.screenshot({ path: 'wizard-stuck.png', fullPage: true });
      logInfo('Screenshot: wizard-stuck.png');
    }

    // Print summary
    console.log('\n' + colors.cyan + '═══ Test Summary ═══' + colors.reset);
    console.log(`Service Worker Logs: ${serviceLogs.length}`);
    console.log(`Wizard Logs: ${wizardLogs.length}`);
    console.log(`Errors: ${errors.length}`);

    if (errors.length > 0) {
      console.log('\n' + colors.red + '═══ ERRORS ═══' + colors.reset);
      errors.forEach(e => console.log(`  ${e}`));
    }

    // Print key logs
    console.log('\n' + colors.cyan + '═══ Key Service Worker Logs ═══' + colors.reset);
    serviceLogs
      .filter(l => l.includes('LLM Manager') || l.includes('setActiveAdapter') || l.includes('Deferred'))
      .slice(-15)
      .forEach(l => console.log(`  ${l}`));

    console.log('\n' + colors.cyan + '═══ Key Wizard Logs ═══' + colors.reset);
    wizardLogs
      .filter(l => l.includes('Wizard') || l.includes('download') || l.includes('Status'))
      .slice(-20)
      .forEach(l => console.log(`  ${l}`));

    if (step3Visible && errors.length === 0) {
      logPass('\n✓ Wizard flow completed successfully!');
    } else if (step3Visible) {
      logWarn('\n⚠ Wizard reached Step 3 but had some errors - check logs');
    } else {
      logFail('\n✗ Wizard flow failed - did not reach Step 3');
    }

    // Keep browser open for inspection
    logInfo('\nBrowser will stay open for 30 seconds for inspection...');
    logInfo('Press Ctrl+C to close early');
    await sleep(30000);

  } catch (error) {
    logFail(`Test failed: ${error.message}`);
    console.error(error.stack);

    // Take error screenshot
    if (browser) {
      const pages = await browser.pages();
      if (pages.length > 0) {
        await pages[pages.length - 1].screenshot({ path: 'wizard-error.png', fullPage: true });
        logInfo('Error screenshot: wizard-error.png');
      }
    }
  } finally {
    if (browser) {
      await browser.close();
      logInfo('Browser closed');
    }
  }
}

// Run test
testWizard().catch(err => {
  logFail('Fatal error: ' + err.message);
  console.error(err);
  process.exit(1);
});
