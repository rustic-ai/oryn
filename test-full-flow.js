#!/usr/bin/env node

/**
 * Complete E2E test: Wizard + First Use + Download
 */

const puppeteer = require('puppeteer');
const path = require('path');

const colors = {
  reset: '\x1b[0m',
  cyan: '\x1b[36m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  red: '\x1b[31m',
  blue: '\x1b[34m',
  magenta: '\x1b[35m'
};

function timestamp() {
  return new Date().toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit', fractionalSecondDigits: 3 });
}

function logInfo(msg) {
  console.log(`${colors.cyan}[${timestamp()}]${colors.reset} ${colors.blue}[INFO]${colors.reset} ${msg}`);
}

function logPass(msg) {
  console.log(`${colors.cyan}[${timestamp()}]${colors.reset} ${colors.green}[PASS]${colors.reset} ${msg}`);
}

function logFail(msg) {
  console.log(`${colors.cyan}[${timestamp()}]${colors.reset} ${colors.red}[FAIL]${colors.reset} ${msg}`);
}

function logWarn(msg) {
  console.log(`${colors.cyan}[${timestamp()}]${colors.reset} ${colors.yellow}[WARN]${colors.reset} ${msg}`);
}

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

async function main() {
  console.log(colors.cyan + '╔════════════════════════════════════════════════════════════════╗' + colors.reset);
  console.log(colors.cyan + '║     Testing Oryn-W: Full Flow (Wizard + Download)             ║' + colors.reset);
  console.log(colors.cyan + '╚════════════════════════════════════════════════════════════════╝' + colors.reset);
  console.log();

  let browser;
  let serviceLogs = [];
  let offscreenLogs = [];
  let errors = [];

  try {
    // Launch browser with extension
    logInfo('Launching browser with extension...');
    const extensionPath = path.resolve(__dirname, 'extension-w');

    browser = await puppeteer.launch({
      headless: false,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
        '--no-sandbox',
        '--disable-setuid-sandbox',
        '--disable-dev-shm-usage',
        '--enable-features=WebGPU',
      ],
      defaultViewport: null,
    });

    logPass('Browser launched');

    await sleep(3000);

    // Find service worker
    const targets = await browser.targets();
    logInfo(`Found ${targets.length} targets: ${targets.map(t => t.type()).join(', ')}`);

    const serviceWorkerTarget = targets.find(
      target => target.type() === 'service_worker'
    );

    if (!serviceWorkerTarget) {
      throw new Error('Service worker not found!');
    }

    const extensionId = serviceWorkerTarget.url().split('/')[2];
    logPass(`Found service worker: ${extensionId}`);

    // Listen to service worker console
    const swPage = await serviceWorkerTarget.page();
    if (swPage) {
      swPage.on('console', msg => {
        const text = msg.text();
        serviceLogs.push(text);
        if (text.includes('error') || text.includes('Error') || text.includes('FAIL')) {
          errors.push(`Service Worker: ${text}`);
        }
      });
      logPass('Listening to service worker console');
    }

    // ===== PART 1: WIZARD =====
    logInfo('Waiting for wizard to open...');
    await sleep(1000);

    const wizardTargets = await browser.targets();
    const wizardTarget = wizardTargets.find(
      t => t.url().includes('first_run_wizard.html')
    );

    if (!wizardTarget) {
      throw new Error('Wizard did not auto-open!');
    }

    const wizardPage = await wizardTarget.page();
    logPass('Wizard auto-opened');

    wizardPage.on('console', msg => {
      const text = msg.text();
      if (text.includes('error') || text.includes('Error') || text.includes('FAIL')) {
        errors.push(`Wizard: ${text}`);
        logFail(`Wizard: ${text}`);
      }
    });

    await wizardPage.waitForSelector('#step-1.active', { timeout: 5000 });
    logPass('Wizard page ready');

    // Complete Step 1
    await sleep(2000);
    await wizardPage.waitForSelector('.hardware-status.available, .hardware-status.unavailable', { timeout: 10000 });
    logPass('Hardware check completed');

    await sleep(1000);
    await wizardPage.click('#btn-next');
    await sleep(2000);

    // Complete Step 2
    logInfo('Step 2: Selecting WebLLM adapter...');
    await wizardPage.waitForSelector('.adapter-option', { timeout: 5000 });

    const adapterOptions = await wizardPage.$$('.adapter-option');
    let webllmOption = null;
    for (const option of adapterOptions) {
      const text = await wizardPage.evaluate(el => el.textContent, option);
      if (text.includes('WebLLM')) {
        webllmOption = option;
        break;
      }
    }

    if (!webllmOption) throw new Error('WebLLM option not found');

    await webllmOption.click();
    await sleep(1000);

    const modelDropdown = await wizardPage.$('.model-dropdown');
    if (!modelDropdown) throw new Error('Model dropdown did not appear');

    await wizardPage.select('.model-dropdown', 'Gemma-2B-it-q4f16_1');
    await sleep(500);
    logPass('Selected Gemma-2B model');

    // Click Download & Continue
    await wizardPage.click('#btn-next');
    await sleep(3000);

    // Verify Step 3
    const step3Visible = await wizardPage.$eval('#step-3', el => el.classList.contains('active'));
    if (!step3Visible) throw new Error('Did not advance to Step 3');
    logPass('Wizard completed (Step 3 visible)');

    // ===== PART 2: FIRST USE (TRIGGER DOWNLOAD) =====
    logInfo('\n===== Testing First Use (Download) =====');

    // Open a regular page
    const testPage = await browser.newPage();
    await testPage.goto('https://example.com');
    await sleep(1000);
    logPass('Opened test page');

    // Try to monitor offscreen document
    logInfo('Checking for offscreen document...');
    await sleep(2000);

    const allTargets = await browser.targets();
    logInfo(`Current targets: ${allTargets.map(t => `${t.type()}:${t.url()}`).join(', ')}`);

    const offscreenTarget = allTargets.find(t => t.url().includes('offscreen.html'));

    if (offscreenTarget) {
      logPass('Offscreen document found!');
      const offscreenPage = await offscreenTarget.page();
      if (offscreenPage) {
        offscreenPage.on('console', msg => {
          const text = msg.text();
          offscreenLogs.push(text);
          console.log(`  ${colors.magenta}[Offscreen]${colors.reset} ${text}`);
          if (text.includes('Download progress') || text.includes('Initializing')) {
            logPass(`Download: ${text}`);
          }
        });
        logPass('Listening to offscreen console');
      }
    } else {
      logWarn('Offscreen document not found yet (will be created on first LLM call)');
    }

    // Simulate LLM usage via background message
    logInfo('Simulating LLM usage to trigger download...');

    const response = await testPage.evaluate(async () => {
      try {
        // First check status
        const status = await chrome.runtime.sendMessage({ type: 'llm_status' });
        console.log('LLM Status:', status);

        // Try to trigger initialization by calling prompt
        // This should create offscreen and start download
        return { status, message: 'Status retrieved' };
      } catch (error) {
        return { error: error.message };
      }
    });

    logInfo(`LLM Status response: ${JSON.stringify(response)}`);

    // Wait to see if offscreen initializes
    logInfo('Waiting 10 seconds for offscreen initialization...');
    await sleep(10000);

    // Check for offscreen again
    const targetsAfter = await browser.targets();
    const offscreenAfter = targetsAfter.find(t => t.url().includes('offscreen.html'));

    if (offscreenAfter) {
      logPass('✓ Offscreen document was created!');

      // Try to connect to it
      const offscreenPage = await offscreenAfter.page();
      if (offscreenPage) {
        logPass('Connected to offscreen page');

        // Check console for initialization logs
        const hasInitLogs = offscreenLogs.some(l =>
          l.includes('LLM Manager') || l.includes('Initializing') || l.includes('WebLLM')
        );

        if (hasInitLogs) {
          logPass('✓ Offscreen is initializing LLM!');
        } else {
          logWarn('Offscreen exists but no initialization logs yet');
        }
      }
    } else {
      logFail('✗ Offscreen document was NOT created');
      logWarn('Download was NOT triggered - offscreen initialization did not happen');
    }

    // Print summary
    console.log('\n' + colors.cyan + '═══ Test Summary ═══' + colors.reset);
    console.log(`Service Worker Logs: ${serviceLogs.length}`);
    console.log(`Offscreen Logs: ${offscreenLogs.length}`);
    console.log(`Errors: ${errors.length}`);

    if (errors.length > 0) {
      console.log('\n' + colors.red + '═══ ERRORS ═══' + colors.reset);
      errors.forEach(e => console.log(`  ${e}`));
    }

    console.log('\n' + colors.cyan + '═══ Key Service Worker Logs ═══' + colors.reset);
    serviceLogs
      .filter(l => l.includes('LLM') || l.includes('Offscreen') || l.includes('Config'))
      .slice(-20)
      .forEach(l => console.log(`  ${l}`));

    console.log('\n' + colors.cyan + '═══ Offscreen Logs ═══' + colors.reset);
    if (offscreenLogs.length > 0) {
      offscreenLogs.forEach(l => console.log(`  ${l}`));
    } else {
      console.log('  (no offscreen logs - offscreen may not have been created)');
    }

    // Final verdict
    const wizardWorked = step3Visible && errors.filter(e => e.includes('Wizard')).length === 0;
    const offscreenCreated = offscreenAfter !== undefined;

    console.log('\n' + colors.cyan + '═══ Final Verdict ═══' + colors.reset);
    if (wizardWorked) {
      logPass('✓ Wizard flow: WORKING');
    } else {
      logFail('✗ Wizard flow: FAILED');
    }

    if (offscreenCreated) {
      logPass('✓ Offscreen document: CREATED');
    } else {
      logFail('✗ Offscreen document: NOT CREATED (download will not work)');
    }

    // Keep open for inspection
    logInfo('\nBrowser will stay open for 60 seconds for manual inspection...');
    logInfo('You can manually test task execution in sidepanel');
    await sleep(60000);

  } catch (error) {
    logFail(`Test failed: ${error.message}`);
    console.error(error.stack);
  } finally {
    if (browser) {
      await browser.close();
      logInfo('Browser closed');
    }
  }
}

main().catch(console.error);
