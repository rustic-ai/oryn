#!/usr/bin/env node

/**
 * Comprehensive test: Wizard → Config Load → Offscreen Init → Download
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

function log(color, level, msg) {
  console.log(`${colors.cyan}[${timestamp()}]${colors.reset} ${color}[${level}]${colors.reset} ${msg}`);
}

const logInfo = (msg) => log(colors.blue, 'INFO', msg);
const logPass = (msg) => log(colors.green, 'PASS', msg);
const logFail = (msg) => log(colors.red, 'FAIL', msg);
const logWarn = (msg) => log(colors.yellow, 'WARN', msg);
const logDebug = (msg) => log(colors.magenta, 'DEBUG', msg);

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

async function main() {
  console.log(colors.cyan + '╔════════════════════════════════════════════════════════════════╗' + colors.reset);
  console.log(colors.cyan + '║     Oryn-W: Complete Download Test (Wizard → Download)        ║' + colors.reset);
  console.log(colors.cyan + '╚════════════════════════════════════════════════════════════════╝' + colors.reset);
  console.log();

  let browser;
  const logs = {
    service: [],
    wizard: [],
    offscreen: [],
    sidepanel: [],
    errors: []
  };

  try {
    // ===== PHASE 1: Launch Browser =====
    logInfo('Phase 1: Launching browser with extension...');
    const extensionPath = path.resolve(__dirname, 'extension-w');

    browser = await puppeteer.launch({
      headless: false,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
        '--no-sandbox',
        '--disable-setuid-sandbox',
        '--enable-features=WebGPU',
      ],
      defaultViewport: null,
    });

    logPass('Browser launched');
    await sleep(3000);

    // Find service worker
    const targets = await browser.targets();
    const serviceWorkerTarget = targets.find(t => t.type() === 'service_worker');
    if (!serviceWorkerTarget) throw new Error('Service worker not found');

    const extensionId = serviceWorkerTarget.url().split('/')[2];
    logPass(`Service worker found: ${extensionId}`);

    // Monitor service worker
    const swPage = await serviceWorkerTarget.page();
    if (swPage) {
      swPage.on('console', msg => {
        const text = msg.text();
        logs.service.push(text);
        if (text.includes('Error') || text.includes('error')) {
          logs.errors.push(`[SW] ${text}`);
        }
      });
    }

    // ===== PHASE 2: Complete Wizard =====
    logInfo('\nPhase 2: Completing wizard...');
    await sleep(1000);

    const wizardTarget = (await browser.targets()).find(t => t.url().includes('first_run_wizard.html'));
    if (!wizardTarget) throw new Error('Wizard did not open');

    const wizardPage = await wizardTarget.page();
    wizardPage.on('console', msg => {
      const text = msg.text();
      logs.wizard.push(text);
      if (text.includes('Error') || text.includes('error')) {
        logs.errors.push(`[Wizard] ${text}`);
      }
    });

    logPass('Wizard opened');

    // Wait for Step 1 to be active
    await wizardPage.waitForSelector('#step-1.active', { timeout: 5000 });
    logInfo('Step 1 active, waiting for hardware check...');

    // Wait for Next button to be enabled (hardware check complete)
    await wizardPage.waitForFunction(() => {
      const btn = document.getElementById('btn-next');
      return btn && !btn.disabled;
    }, { timeout: 15000 });

    logPass('Hardware check complete');
    await wizardPage.click('#btn-next');
    await sleep(2000);

    // Step 2: Select adapter
    logInfo('Step 2: Selecting WebLLM...');
    await wizardPage.waitForSelector('.adapter-option', { timeout: 5000 });

    // Click WebLLM
    await wizardPage.evaluate(() => {
      const options = Array.from(document.querySelectorAll('.adapter-option'));
      const webllm = options.find(el => el.textContent.includes('WebLLM'));
      if (webllm) webllm.click();
    });

    await sleep(1000);

    // Select model
    const modelDropdown = await wizardPage.$('.model-dropdown');
    if (!modelDropdown) throw new Error('Model dropdown not found');

    await wizardPage.select('.model-dropdown', 'Gemma-2B-it-q4f16_1');
    logPass('Selected Gemma-2B-it-q4f16_1');
    await sleep(500);

    // Click Download & Continue
    await wizardPage.click('#btn-next');
    logInfo('Clicked "Download & Continue", waiting for Step 3...');
    await sleep(3000);

    // Verify Step 3
    const step3Active = await wizardPage.evaluate(() => {
      return document.getElementById('step-3')?.classList.contains('active');
    });

    if (!step3Active) throw new Error('Step 3 not active after clicking button');
    logPass('✓ Wizard completed (Step 3 active)');

    // ===== PHASE 3: Verify Config Saved =====
    logInfo('\nPhase 3: Verifying config persistence...');

    const configSaved = await wizardPage.evaluate(async () => {
      const result = await chrome.storage.sync.get(['llmConfig']);
      return result.llmConfig;
    });

    if (!configSaved) throw new Error('Config not saved to chrome.storage');
    if (configSaved.selectedAdapter !== 'webllm') throw new Error(`Wrong adapter: ${configSaved.selectedAdapter}`);
    if (configSaved.selectedModel !== 'Gemma-2B-it-q4f16_1') throw new Error(`Wrong model: ${configSaved.selectedModel}`);

    logPass(`✓ Config saved: ${configSaved.selectedAdapter} / ${configSaved.selectedModel}`);

    // ===== PHASE 4: Check if Offscreen Created on Startup =====
    logInfo('\nPhase 4: Checking for offscreen document...');
    await sleep(2000);

    let allTargets = await browser.targets();
    let offscreenTarget = allTargets.find(t => t.url().includes('offscreen.html'));

    if (offscreenTarget) {
      logPass('✓ Offscreen document exists (created during config load)');
      const offscreenPage = await offscreenTarget.page();
      if (offscreenPage) {
        offscreenPage.on('console', msg => {
          const text = msg.text();
          logs.offscreen.push(text);
          console.log(`    ${colors.magenta}[Offscreen]${colors.reset} ${text}`);
        });
        logPass('Listening to offscreen console');
      }
    } else {
      logWarn('⚠ Offscreen not created yet (will be created on first LLM call)');
    }

    // ===== PHASE 5: Open Sidepanel =====
    logInfo('\nPhase 5: Opening sidepanel...');

    // Navigate to a test page
    const testPage = await browser.newPage();
    await testPage.goto('https://example.com');
    await sleep(1000);
    logPass('Test page loaded');

    // Manually open sidepanel - user will need to click the extension icon
    // OR we can try to trigger it programmatically via service worker
    logInfo('Attempting to open sidepanel via service worker...');

    // Send message to service worker to open sidepanel
    await testPage.evaluate(() => {
      // This won't work from regular page, skip it
      console.log('[Test] Skipping automatic sidepanel open');
    });

    logWarn('⚠ Please manually click the Oryn-W extension icon to open sidepanel');
    logInfo('Waiting 10 seconds for you to open sidepanel...');
    await sleep(10000);

    // Find sidepanel target
    allTargets = await browser.targets();
    const sidepanelTarget = allTargets.find(t => t.url().includes('sidepanel.html'));

    if (!sidepanelTarget) throw new Error('Sidepanel not found');

    const sidepanelPage = await sidepanelTarget.page();
    sidepanelPage.on('console', msg => {
      const text = msg.text();
      logs.sidepanel.push(text);
      if (text.includes('Download progress') || text.includes('Initializing')) {
        logPass(`[Sidepanel] ${text}`);
      }
    });

    logPass('Monitoring sidepanel console');

    // ===== PHASE 6: Trigger LLM Call =====
    logInfo('\nPhase 6: Triggering LLM initialization via test prompt...');

    // Type a test task in sidepanel
    await sidepanelPage.waitForSelector('#task-input', { timeout: 5000 });
    await sidepanelPage.type('#task-input', 'Say hello');
    await sleep(500);

    // Click execute button
    const executeBtn = await sidepanelPage.$('#execute-agent');
    if (!executeBtn) throw new Error('Execute button not found');

    logInfo('Clicking execute button to trigger LLM...');
    await executeBtn.click();

    logInfo('Waiting 5 seconds for offscreen initialization...');
    await sleep(5000);

    // ===== PHASE 7: Verify Offscreen Created =====
    logInfo('\nPhase 7: Verifying offscreen document creation...');

    allTargets = await browser.targets();
    offscreenTarget = allTargets.find(t => t.url().includes('offscreen.html'));

    if (!offscreenTarget) {
      logFail('✗ Offscreen document NOT created after LLM call');
      throw new Error('Offscreen not created - download will not work');
    }

    logPass('✓ Offscreen document exists');

    // Connect to offscreen if not already
    const offscreenPage = await offscreenTarget.page();
    if (offscreenPage && !logs.offscreen.length) {
      offscreenPage.on('console', msg => {
        const text = msg.text();
        logs.offscreen.push(text);
        console.log(`    ${colors.magenta}[Offscreen]${colors.reset} ${text}`);
      });
    }

    // ===== PHASE 8: Monitor for Download =====
    logInfo('\nPhase 8: Monitoring for WebLLM initialization and download...');
    logInfo('Waiting 20 seconds for download to start...');

    await sleep(20000);

    // ===== FINAL ANALYSIS =====
    console.log('\n' + colors.cyan + '═══════════════════════════════════════════════════════' + colors.reset);
    console.log(colors.cyan + '                    TEST RESULTS                        ' + colors.reset);
    console.log(colors.cyan + '═══════════════════════════════════════════════════════' + colors.reset);

    // Check what happened
    const hasOffscreenInit = logs.offscreen.some(l =>
      l.includes('LLM Manager') || l.includes('Initializing') || l.includes('initialize')
    );

    const hasWebLLMInit = logs.offscreen.some(l =>
      l.includes('WebLLM') && l.includes('Initializing')
    );

    const hasDownloadProgress = logs.offscreen.some(l =>
      l.includes('Download progress') || l.includes('download')
    );

    const hasError = logs.errors.length > 0;

    console.log();
    logInfo('Phase Results:');
    console.log(`  1. Browser Launch:        ${colors.green}✓ PASS${colors.reset}`);
    console.log(`  2. Wizard Completion:     ${colors.green}✓ PASS${colors.reset}`);
    console.log(`  3. Config Saved:          ${colors.green}✓ PASS${colors.reset}`);
    console.log(`  4. Offscreen Created:     ${offscreenTarget ? colors.green + '✓ PASS' : colors.red + '✗ FAIL'}${colors.reset}`);
    console.log(`  5. Sidepanel Opened:      ${colors.green}✓ PASS${colors.reset}`);
    console.log(`  6. LLM Call Triggered:    ${colors.green}✓ PASS${colors.reset}`);
    console.log(`  7. Offscreen Init:        ${hasOffscreenInit ? colors.green + '✓ PASS' : colors.red + '✗ FAIL'}${colors.reset}`);
    console.log(`  8. WebLLM Init:           ${hasWebLLMInit ? colors.green + '✓ PASS' : colors.red + '✗ FAIL'}${colors.reset}`);
    console.log(`  9. Download Started:      ${hasDownloadProgress ? colors.green + '✓ PASS' : colors.red + '✗ FAIL'}${colors.reset}`);
    console.log();

    console.log(colors.cyan + '───────────────────────────────────────────────────────' + colors.reset);
    console.log(`Service Worker Logs:  ${logs.service.length}`);
    console.log(`Wizard Logs:          ${logs.wizard.length}`);
    console.log(`Offscreen Logs:       ${logs.offscreen.length}`);
    console.log(`Sidepanel Logs:       ${logs.sidepanel.length}`);
    console.log(`Errors:               ${logs.errors.length}`);

    if (logs.errors.length > 0) {
      console.log('\n' + colors.red + '═══ ERRORS ═══' + colors.reset);
      logs.errors.slice(-10).forEach(e => console.log(`  ${e}`));
    }

    console.log('\n' + colors.cyan + '═══ Key Logs ═══' + colors.reset);

    console.log('\n' + colors.blue + 'Service Worker:' + colors.reset);
    logs.service.filter(l =>
      l.includes('LLM') || l.includes('Config') || l.includes('Offscreen')
    ).slice(-10).forEach(l => console.log(`  ${l}`));

    console.log('\n' + colors.magenta + 'Offscreen:' + colors.reset);
    if (logs.offscreen.length > 0) {
      logs.offscreen.slice(-20).forEach(l => console.log(`  ${l}`));
    } else {
      console.log(`  ${colors.yellow}(no logs - offscreen may not have initialized)${colors.reset}`);
    }

    console.log('\n' + colors.cyan + '═══ VERDICT ═══' + colors.reset);

    if (hasDownloadProgress) {
      logPass('✓✓✓ DOWNLOAD IS WORKING! ✓✓✓');
    } else if (hasWebLLMInit) {
      logWarn('⚠ WebLLM initialized but download not detected yet');
      logInfo('Download may be starting - check offscreen console manually');
    } else if (hasOffscreenInit) {
      logFail('✗ Offscreen initialized but WebLLM did not initialize');
      logInfo('Dynamic import may have failed - check offscreen console');
    } else if (offscreenTarget) {
      logFail('✗ Offscreen exists but did not initialize');
      logInfo('Message passing may be broken');
    } else {
      logFail('✗ Offscreen document was never created');
      logInfo('Config loading or proxy routing is broken');
    }

    console.log();
    logInfo('Browser will stay open for 60 seconds for manual inspection...');
    logInfo('Open DevTools on offscreen document (chrome://inspect) to see initialization');
    await sleep(60000);

  } catch (error) {
    logFail(`Test failed: ${error.message}`);
    console.error(error.stack);
    if (browser) {
      logInfo('Browser will stay open for 60 seconds for debugging...');
      await sleep(60000);
    }
  } finally {
    if (browser) {
      await browser.close();
      logInfo('Browser closed');
    }
  }
}

main().catch(console.error);
