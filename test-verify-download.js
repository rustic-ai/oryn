#!/usr/bin/env node

/**
 * Automated test with manual verification step
 * Tests: Wizard → Config → Prompts user to test download manually
 */

const puppeteer = require('puppeteer');
const path = require('path');
const readline = require('readline');

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
  const ts = new Date().toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
  console.log(`${colors.cyan}[${ts}]${colors.reset} ${color}[${level}]${colors.reset} ${msg}`);
}

const logInfo = (msg) => log(colors.blue, 'INFO', msg);
const logPass = (msg) => log(colors.green, 'PASS', msg);
const logFail = (msg) => log(colors.red, 'FAIL', msg);
const logWarn = (msg) => log(colors.yellow, 'WARN', msg);

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

async function promptUser(question) {
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout
  });

  return new Promise(resolve => {
    rl.question(colors.yellow + question + colors.reset + ' ', answer => {
      rl.close();
      resolve(answer.toLowerCase().trim());
    });
  });
}

async function main() {
  console.log(colors.bold + colors.cyan + '\n╔══════════════════════════════════════════════════════════════╗' + colors.reset);
  console.log(colors.bold + colors.cyan + '║          Oryn-W Download Verification Test                  ║' + colors.reset);
  console.log(colors.bold + colors.cyan + '╚══════════════════════════════════════════════════════════════╝\n' + colors.reset);

  let browser;

  try {
    // Launch browser
    logInfo('Launching browser with extension...');
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

    // Find service worker
    const targets = await browser.targets();
    const swTarget = targets.find(t => t.type() === 'service_worker');
    if (!swTarget) throw new Error('Service worker not found');

    const extensionId = swTarget.url().split('/')[2];
    logPass(`Extension ID: ${extensionId}`);

    // ===== AUTOMATED: Complete Wizard =====
    console.log(colors.cyan + '\n═══ Phase 1: Wizard (Automated) ═══' + colors.reset);

    const wizardTarget = (await browser.targets()).find(t => t.url().includes('first_run_wizard.html'));
    if (!wizardTarget) throw new Error('Wizard did not open');

    const wizardPage = await wizardTarget.page();
    logPass('Wizard opened');

    // Step 1
    await wizardPage.waitForSelector('#step-1.active', { timeout: 5000 });
    await wizardPage.waitForFunction(() => !document.getElementById('btn-next').disabled, { timeout: 15000 });
    logPass('Hardware check complete');

    await wizardPage.click('#btn-next');
    await sleep(2000);

    // Step 2
    await wizardPage.evaluate(() => {
      const options = Array.from(document.querySelectorAll('.adapter-option'));
      const webllm = options.find(el => el.textContent.includes('WebLLM'));
      if (webllm) webllm.click();
    });
    await sleep(1000);

    await wizardPage.select('.model-dropdown', 'Gemma-2B-it-q4f16_1');
    logPass('Selected Gemma-2B-it-q4f16_1');
    await sleep(500);

    await wizardPage.click('#btn-next');
    await sleep(3000);

    // Verify Step 3
    const step3Active = await wizardPage.evaluate(() => {
      return document.getElementById('step-3')?.classList.contains('active');
    });

    if (!step3Active) throw new Error('Wizard did not complete');
    logPass('✓ Wizard completed');

    // Verify config
    const config = await wizardPage.evaluate(async () => {
      const result = await chrome.storage.sync.get(['llmConfig']);
      return result.llmConfig;
    });

    if (!config || config.selectedAdapter !== 'webllm') throw new Error('Config not saved correctly');
    logPass(`✓ Config saved: ${config.selectedAdapter} / ${config.selectedModel}`);

    console.log(colors.green + '\n✅ AUTOMATED TESTS PASSED\n' + colors.reset);

    // ===== MANUAL: Verify Download =====
    console.log(colors.bold + colors.cyan + '═══ Phase 2: Download Verification (Manual) ═══' + colors.reset);
    console.log();
    console.log(colors.yellow + 'The automated portion is complete. Now we need to manually verify the download.\n' + colors.reset);
    console.log(colors.bold + 'INSTRUCTIONS:' + colors.reset);
    console.log('1. In the browser window, click the Oryn-W extension icon to open sidepanel');
    console.log('2. In sidepanel, select "Agent Mode" and enter task: ' + colors.cyan + '"Say hello"' + colors.reset);
    console.log('3. Click "Execute" button');
    console.log();
    console.log(colors.bold + '4. Open chrome://inspect in a new tab' + colors.reset);
    console.log('5. Look for "offscreen.html" under Oryn-W');
    console.log('6. Click "inspect" to open offscreen DevTools');
    console.log();
    console.log(colors.bold + '7. In offscreen console, look for:' + colors.reset);
    console.log('   - ' + colors.green + '[Offscreen] Starting LLM offscreen document' + colors.reset);
    console.log('   - ' + colors.green + '[Offscreen] Received message: offscreen_llm_set_adapter' + colors.reset);
    console.log('   - ' + colors.green + '[WebLLM] Initializing with model' + colors.reset);
    console.log('   - ' + colors.green + '[WebLLM] Download progress: X%' + colors.reset);
    console.log();

    const answer = await promptUser('\nDid you see the download progress in offscreen console? (yes/no):');

    if (answer === 'yes' || answer === 'y') {
      console.log(colors.bold + colors.green + '\n✅✅✅ DOWNLOAD IS WORKING! ✅✅✅\n' + colors.reset);

      const completed = await promptUser('Did the download complete to 100%? (yes/no):');
      if (completed === 'yes' || completed === 'y') {
        console.log(colors.bold + colors.green + '\n✅ FULL SUCCESS - Download completed!\n' + colors.reset);

        const taskWorked = await promptUser('Did the task execute successfully? (yes/no):');
        if (taskWorked === 'yes' || taskWorked === 'y') {
          console.log(colors.bold + colors.green + '\n🎉 COMPLETE SUCCESS - Everything is working! 🎉\n' + colors.reset);
        } else {
          console.log(colors.yellow + '\n⚠ Task execution failed after download\n' + colors.reset);
        }
      } else {
        console.log(colors.yellow + '\n⚠ Download started but did not complete\n' + colors.reset);
      }
    } else {
      console.log(colors.bold + colors.red + '\n✗ DOWNLOAD DID NOT START\n' + colors.reset);

      const offscreenExists = await promptUser('Did you see offscreen.html in chrome://inspect? (yes/no):');

      if (offscreenExists === 'yes' || offscreenExists === 'y') {
        console.log(colors.red + '✗ Offscreen exists but WebLLM did not initialize' + colors.reset);
        console.log(colors.yellow + '  → Check offscreen console for errors' + colors.reset);
        console.log(colors.yellow + '  → Dynamic import may have failed' + colors.reset);
      } else {
        console.log(colors.red + '✗ Offscreen document was never created' + colors.reset);
        console.log(colors.yellow + '  → Message passing is broken' + colors.reset);
        console.log(colors.yellow + '  → Check service worker logs for "Using offscreen"' + colors.reset);
      }
    }

    console.log('\n' + colors.cyan + 'Browser will stay open for further inspection. Press Ctrl+C to close.' + colors.reset);
    await sleep(300000); // 5 minutes

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
