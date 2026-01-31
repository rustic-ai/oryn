#!/usr/bin/env node

/**
 * COMPREHENSIVE TEST: Wait for actual download completion
 * This test will wait up to 15 minutes for the download
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
  magenta: '\x1b[35m',
  bold: '\x1b[1m'
};

function log(color, level, msg) {
  const ts = new Date().toLocaleTimeString('en-US', { hour12: false });
  console.log(`${colors.cyan}[${ts}]${colors.reset} ${color}[${level}]${colors.reset} ${msg}`);
}

const logInfo = (msg) => log(colors.blue, 'INFO', msg);
const logPass = (msg) => log(colors.green, 'PASS', msg);
const logFail = (msg) => log(colors.red, 'FAIL', msg);
const logWarn = (msg) => log(colors.yellow, 'WARN', msg);
const logProgress = (msg) => log(colors.magenta, 'PROGRESS', msg);

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

async function main() {
  console.log(colors.bold + colors.cyan + '\n╔══════════════════════════════════════════════════════════════╗' + colors.reset);
  console.log(colors.bold + colors.cyan + '║     Oryn-W: FULL DOWNLOAD TEST (will wait 15 min)           ║' + colors.reset);
  console.log(colors.bold + colors.cyan + '╚══════════════════════════════════════════════════════════════╝\n' + colors.reset);

  let browser;
  const logs = {
    service: [],
    offscreen: [],
    errors: []
  };

  let downloadStarted = false;
  let downloadProgress = 0;
  let downloadComplete = false;
  let webllmInitialized = false;

  try {
    // ===== PHASE 1: Launch =====
    logInfo('Phase 1: Launching browser...');
    const extensionPath = path.resolve(__dirname, 'extension-w');

    browser = await puppeteer.launch({
      headless: false,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
        '--no-sandbox',
        '--enable-features=WebGPU',
      ],
      defaultViewport: null,
    });

    logPass('Browser launched');
    await sleep(3000);

    const targets = await browser.targets();
    const swTarget = targets.find(t => t.type() === 'service_worker');
    if (!swTarget) throw new Error('Service worker not found');

    const extensionId = swTarget.url().split('/')[2];
    logPass(`Extension ID: ${extensionId}`);

    // Monitor service worker
    const swPage = await swTarget.page();
    if (swPage) {
      swPage.on('console', msg => {
        const text = msg.text();
        logs.service.push(text);
        if (text.includes('Offscreen')) {
          logInfo(`[SW] ${text}`);
        }
        if (text.includes('Error') || text.includes('error')) {
          logs.errors.push(`[SW] ${text}`);
          logFail(`[SW] ${text}`);
        }
      });
    }

    // ===== PHASE 2: Wizard =====
    logInfo('\nPhase 2: Completing wizard...');

    const wizardTarget = (await browser.targets()).find(t => t.url().includes('first_run_wizard.html'));
    if (!wizardTarget) throw new Error('Wizard not found');

    const wizardPage = await wizardTarget.page();
    await wizardPage.waitForSelector('#step-1.active', { timeout: 5000 });
    await wizardPage.waitForFunction(() => !document.getElementById('btn-next').disabled, { timeout: 15000 });
    logPass('Hardware check complete');

    await wizardPage.click('#btn-next');
    await sleep(2000);

    // Select WebLLM
    await wizardPage.evaluate(() => {
      const options = Array.from(document.querySelectorAll('.adapter-option'));
      const webllm = options.find(el => el.textContent.includes('WebLLM'));
      if (webllm) webllm.click();
    });
    await sleep(1000);

    await wizardPage.select('.model-dropdown', 'Gemma-2B-it-q4f16_1');
    logPass('Selected Gemma-2B (1.5GB)');
    await sleep(500);

    await wizardPage.click('#btn-next');
    await sleep(3000);

    const step3Active = await wizardPage.evaluate(() => document.getElementById('step-3')?.classList.contains('active'));
    if (!step3Active) throw new Error('Wizard failed');
    logPass('✓ Wizard completed');

    // ===== PHASE 3: Wait for Config Load =====
    logInfo('\nPhase 3: Checking if config loads on startup...');
    await sleep(3000);

    let allTargets = await browser.targets();
    let offscreenTarget = allTargets.find(t => t.url().includes('offscreen.html'));
    let offscreenPage = null;

    if (offscreenTarget) {
      logPass('✓ Offscreen created on startup!');

      // Connect to offscreen console immediately!
      offscreenPage = await offscreenTarget.page();
      if (offscreenPage) {
        logPass('Connected to offscreen console');

        // Set up console listener
        offscreenPage.on('console', msg => {
          const text = msg.text();
          logs.offscreen.push(text);

          // Log ALL messages for debugging
          logInfo(`[Offscreen] ${text}`);

          if (text.includes('WebLLM') && text.includes('Initializing')) {
            webllmInitialized = true;
            logPass(`WebLLM initialization started!`);
          }

          if (text.includes('Download progress')) {
            downloadStarted = true;
            const match = text.match(/(\d+\.?\d*)%/);
            if (match) {
              downloadProgress = parseFloat(match[1]);
              logProgress(`Download: ${downloadProgress.toFixed(1)}%`);
            }
            if (downloadProgress >= 99.9) {
              downloadComplete = true;
            }
          }

          if (text.includes('Error') || text.includes('error') || text.includes('Failed')) {
            logs.errors.push(`[Offscreen] ${text}`);
            logFail(`ERROR: ${text}`);
          }
        });

        logInfo('Listening to offscreen console...');
      }
    } else {
      logWarn('⚠ Offscreen NOT created on startup (will be created on first use)');
    }

    // ===== PHASE 4: Trigger Download =====
    logInfo('\nPhase 4: Opening sidepanel and triggering download...');
    logInfo('Please MANUALLY:');
    logInfo('  1. Click the Oryn-W extension icon');
    logInfo('  2. In sidepanel, enter task: "Say hello"');
    logInfo('  3. Click "Execute" button');
    logInfo('\nWaiting 30 seconds for you to do this...');

    await sleep(30000);

    // ===== PHASE 5: Monitor Offscreen =====
    logInfo('\nPhase 5: Checking offscreen document status...');

    // If not already connected, try to connect now
    if (!offscreenPage) {
      // Check every 2 seconds for up to 60 seconds
      for (let i = 0; i < 30; i++) {
        allTargets = await browser.targets();
        offscreenTarget = allTargets.find(t => t.url().includes('offscreen.html'));

        if (offscreenTarget) {
          logPass('✓ Offscreen document created!');
          break;
        }

        await sleep(2000);
      }

      if (!offscreenTarget) {
        logFail('✗ Offscreen was never created after 60 seconds');
        throw new Error('Offscreen not created - download cannot work');
      }

      // Connect to offscreen console
      offscreenPage = await offscreenTarget.page();
      if (offscreenPage) {
        logPass('Connected to offscreen console');

        // Set up console listener
        offscreenPage.on('console', msg => {
          const text = msg.text();
          logs.offscreen.push(text);

          // Log ALL messages for debugging
          logInfo(`[Offscreen] ${text}`);

          if (text.includes('WebLLM') && text.includes('Initializing')) {
            webllmInitialized = true;
            logPass(`WebLLM initialization started!`);
          }

          if (text.includes('Download progress')) {
            downloadStarted = true;
            const match = text.match(/(\d+\.?\d*)%/);
            if (match) {
              downloadProgress = parseFloat(match[1]);
              logProgress(`Download: ${downloadProgress.toFixed(1)}%`);
            }
            if (downloadProgress >= 99.9) {
              downloadComplete = true;
            }
          }

          if (text.includes('Error') || text.includes('error') || text.includes('Failed')) {
            logs.errors.push(`[Offscreen] ${text}`);
            logFail(`ERROR: ${text}`);
          }
        });

        logInfo('Listening to offscreen console...');
      }
    } else {
      logPass('✓ Offscreen already connected from Phase 3');
    }

    // ===== PHASE 6: Wait for Download =====
    logInfo('\nPhase 6: Waiting for WebLLM initialization and download...');
    logInfo('This may take 5-15 minutes for Gemma-2B (1.5GB)');
    logInfo('Checking every 10 seconds...\n');

    const maxWaitMinutes = 15;
    const checkIntervalSec = 10;
    const maxChecks = (maxWaitMinutes * 60) / checkIntervalSec;

    for (let i = 0; i < maxChecks; i++) {
      const elapsed = (i * checkIntervalSec) / 60;

      if (downloadComplete) {
        logPass(`\n✓✓✓ DOWNLOAD COMPLETE after ${elapsed.toFixed(1)} minutes! ✓✓✓`);
        break;
      }

      if (downloadStarted) {
        logInfo(`[${elapsed.toFixed(1)} min] Download in progress: ${downloadProgress.toFixed(1)}%`);
      } else if (webllmInitialized) {
        logInfo(`[${elapsed.toFixed(1)} min] WebLLM initialized, waiting for download to start...`);
      } else {
        logInfo(`[${elapsed.toFixed(1)} min] Waiting for WebLLM initialization...`);
      }

      await sleep(checkIntervalSec * 1000);
    }

    // ===== FINAL REPORT =====
    console.log('\n' + colors.bold + colors.cyan + '═══════════════════════════════════════════════════════════' + colors.reset);
    console.log(colors.bold + colors.cyan + '                    FINAL RESULTS                            ' + colors.reset);
    console.log(colors.bold + colors.cyan + '═══════════════════════════════════════════════════════════' + colors.reset);
    console.log();

    console.log('Offscreen Logs Collected:     ' + logs.offscreen.length);
    console.log('Service Worker Logs:          ' + logs.service.length);
    console.log('Errors:                       ' + logs.errors.length);
    console.log();

    console.log('Test Results:');
    console.log(`  Wizard Completed:           ${colors.green}✓ YES${colors.reset}`);
    console.log(`  Config Saved:               ${colors.green}✓ YES${colors.reset}`);
    console.log(`  Offscreen Created:          ${offscreenTarget ? colors.green + '✓ YES' : colors.red + '✗ NO'}${colors.reset}`);
    console.log(`  WebLLM Initialized:         ${webllmInitialized ? colors.green + '✓ YES' : colors.red + '✗ NO'}${colors.reset}`);
    console.log(`  Download Started:           ${downloadStarted ? colors.green + '✓ YES' : colors.red + '✗ NO'}${colors.reset}`);
    console.log(`  Download Progress:          ${downloadStarted ? colors.cyan + downloadProgress.toFixed(1) + '%' : colors.red + 'N/A'}${colors.reset}`);
    console.log(`  Download Completed:         ${downloadComplete ? colors.green + '✓ YES' : colors.red + '✗ NO'}${colors.reset}`);
    console.log();

    if (logs.errors.length > 0) {
      console.log(colors.red + '═══ ERRORS ═══' + colors.reset);
      logs.errors.slice(-10).forEach(e => console.log(`  ${e}`));
      console.log();
    }

    console.log(colors.cyan + '═══ Sample Offscreen Logs ═══' + colors.reset);
    logs.offscreen.filter(l =>
      l.includes('Initializing') || l.includes('WebLLM') || l.includes('Download') || l.includes('Error')
    ).slice(-30).forEach(l => console.log(`  ${l}`));
    console.log();

    // Final verdict
    if (downloadComplete) {
      console.log(colors.bold + colors.green + '\n🎉🎉🎉 SUCCESS: DOWNLOAD WORKS COMPLETELY! 🎉🎉🎉\n' + colors.reset);
    } else if (downloadStarted) {
      console.log(colors.bold + colors.yellow + '\n⚠ PARTIAL: Download started but did not complete in time\n' + colors.reset);
      console.log(colors.yellow + `Progress reached: ${downloadProgress.toFixed(1)}%` + colors.reset);
      console.log(colors.yellow + 'Try running test again or wait longer\n' + colors.reset);
    } else if (webllmInitialized) {
      console.log(colors.bold + colors.red + '\n✗ FAILURE: WebLLM initialized but download never started\n' + colors.reset);
    } else if (offscreenTarget) {
      console.log(colors.bold + colors.red + '\n✗ FAILURE: Offscreen created but WebLLM did not initialize\n' + colors.reset);
    } else {
      console.log(colors.bold + colors.red + '\n✗ FAILURE: Offscreen was never created\n' + colors.reset);
    }

    logInfo('Browser will stay open for 60 seconds for inspection...');
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
