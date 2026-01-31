#!/usr/bin/env node
/**
 * Test extension-w with Puppeteer
 * Captures console logs from background service worker and wizard page
 */

const puppeteer = require('puppeteer');
const path = require('path');
const fs = require('fs');

const PROJECT_ROOT = path.join(__dirname, '..');
const EXT_DIR = path.join(PROJECT_ROOT, 'extension-w');
const LOG_FILE = path.join(PROJECT_ROOT, 'extension-w-test.log');

// Color logging
const colors = {
  reset: '\x1b[0m',
  red: '\x1b[31m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  cyan: '\x1b[36m',
};

function log(color, prefix, msg) {
  console.log(`${color}[${prefix}]${colors.reset} ${msg}`);
}

const logInfo = (msg) => log(colors.blue, 'INFO', msg);
const logPass = (msg) => log(colors.green, 'PASS', msg);
const logFail = (msg) => log(colors.red, 'FAIL', msg);
const logWarn = (msg) => log(colors.yellow, 'WARN', msg);

// Store all logs
const allLogs = [];

function saveLog(source, type, message) {
  const entry = `[${new Date().toISOString()}] [${source}] [${type}] ${message}`;
  allLogs.push(entry);
  console.log(entry);
}

async function testExtension() {
  console.log('\n' + colors.cyan + '╔════════════════════════════════════════════════════════════════╗' + colors.reset);
  console.log(colors.cyan + '║           Testing Oryn-W Extension with Puppeteer              ║' + colors.reset);
  console.log(colors.cyan + '╚════════════════════════════════════════════════════════════════╝' + colors.reset + '\n');

  // Check extension directory
  if (!fs.existsSync(EXT_DIR)) {
    logFail(`Extension directory not found: ${EXT_DIR}`);
    logInfo('Please run: ./scripts/build-extension-w.sh');
    process.exit(1);
  }

  logPass(`Extension directory found: ${EXT_DIR}`);

  let browser;
  let serviceWorkerTarget;

  try {
    // Launch browser with extension
    logInfo('Launching Chromium with extension...');

    browser = await puppeteer.launch({
      headless: false, // Must be false to load extensions
      args: [
        `--disable-extensions-except=${EXT_DIR}`,
        `--load-extension=${EXT_DIR}`,
        '--no-first-run',
        '--no-default-browser-check',
        '--disable-blink-features=AutomationControlled',
      ],
      devtools: true, // Auto-open DevTools
    });

    logPass('Browser launched');

    // Wait a bit for extension to load
    await new Promise(resolve => setTimeout(resolve, 2000));

    // Get all targets (pages, service workers, etc.)
    const targets = await browser.targets();

    // Find the service worker
    serviceWorkerTarget = targets.find(
      target => target.type() === 'service_worker' && target.url().includes('extension-w')
    );

    if (serviceWorkerTarget) {
      logPass('Found service worker: ' + serviceWorkerTarget.url());

      // Listen to service worker console
      const worker = await serviceWorkerTarget.worker();
      if (worker) {
        worker.on('console', msg => {
          const text = msg.text();
          saveLog('ServiceWorker', msg.type(), text);
        });

        worker.on('pageerror', err => {
          saveLog('ServiceWorker', 'ERROR', err.toString());
        });

        logPass('Listening to service worker console');
      }
    } else {
      logWarn('Service worker not found yet');
    }

    // Find the wizard tab (first-run wizard should auto-open)
    logInfo('Waiting for first-run wizard to open...');

    let wizardPage;
    for (let i = 0; i < 20; i++) {
      const pages = await browser.pages();
      wizardPage = pages.find(p => p.url().includes('first_run_wizard.html'));

      if (wizardPage) {
        logPass('Found wizard page: ' + wizardPage.url());
        break;
      }

      await new Promise(resolve => setTimeout(resolve, 500));
    }

    if (!wizardPage) {
      // Wizard didn't auto-open, try to open manually
      logWarn('Wizard did not auto-open, opening manually...');
      const pages = await browser.pages();
      wizardPage = pages[0];
      const wizardUrl = `chrome-extension://${serviceWorkerTarget ? new URL(serviceWorkerTarget.url()).host : ''}/ui/first_run_wizard.html`;

      logInfo(`Navigating to: ${wizardUrl}`);
      // Can't navigate to extension URLs directly, so just wait
      logWarn('Please open the wizard manually from chrome://extensions');
    }

    if (wizardPage) {
      // Listen to wizard console
      wizardPage.on('console', msg => {
        saveLog('Wizard', msg.type(), msg.text());
      });

      wizardPage.on('pageerror', err => {
        saveLog('Wizard', 'ERROR', err.toString());
      });

      logPass('Listening to wizard console');

      // Take screenshot of wizard
      const screenshotPath = path.join(PROJECT_ROOT, 'wizard-screenshot.png');
      await wizardPage.screenshot({ path: screenshotPath, fullPage: true });
      logInfo(`Screenshot saved: ${screenshotPath}`);
    }

    // Keep browser open and collect logs
    logInfo('Browser is running. Complete the wizard flow manually.');
    logInfo('Press Ctrl+C to stop and save logs.');

    // Wait for interrupt
    await new Promise(() => {}); // Never resolves, will be interrupted

  } catch (error) {
    if (error.message.includes('SIGINT')) {
      logInfo('Stopping browser...');
    } else {
      logFail('Error: ' + error.message);
      console.error(error);
    }
  } finally {
    // Save logs to file
    if (allLogs.length > 0) {
      fs.writeFileSync(LOG_FILE, allLogs.join('\n'));
      logPass(`Logs saved to: ${LOG_FILE}`);

      // Print summary
      console.log('\n' + colors.cyan + '═══ Log Summary ═══' + colors.reset);

      const errors = allLogs.filter(l => l.includes('[error]') || l.includes('[ERROR]'));
      const llmLogs = allLogs.filter(l => l.includes('LLM Manager') || l.includes('WebLLM'));
      const wizardLogs = allLogs.filter(l => l.includes('Wizard'));

      console.log(`Total logs: ${allLogs.length}`);
      console.log(`Errors: ${errors.length}`);
      console.log(`LLM-related: ${llmLogs.length}`);
      console.log(`Wizard-related: ${wizardLogs.length}`);

      if (errors.length > 0) {
        console.log('\n' + colors.red + '═══ ERRORS ═══' + colors.reset);
        errors.forEach(e => console.log(e));
      }
    }

    if (browser) {
      await browser.close();
    }
  }
}

// Handle interrupts gracefully
process.on('SIGINT', async () => {
  logInfo('\nReceived SIGINT, saving logs and exiting...');
  process.exit(0);
});

// Run test
testExtension().catch(error => {
  logFail('Fatal error: ' + error.message);
  console.error(error);
  process.exit(1);
});
