#!/usr/bin/env node

/**
 * IMPROVED DEBUG TEST: Monitor service worker console for offscreen status
 */

const puppeteer = require('puppeteer');
const path = require('path');

async function main() {
  let browser;

  try {
    console.log('\n=== Offscreen Status Monitor Test ===\n');

    // Launch browser with extension
    const extensionPath = path.join(__dirname, 'extension-w');
    browser = await puppeteer.launch({
      headless: false,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    console.log('✓ Browser launched');

    // Wait for extension to load
    await new Promise(resolve => setTimeout(resolve, 2000));

    // Find extension service worker
    const swTarget = await browser.waitForTarget(t => t.type() === 'service_worker');
    console.log('✓ Extension ID:', swTarget.url().split('/')[2]);

    // Connect to service worker for console logs
    const swWorker = await swTarget.worker();
    console.log('✓ Connected to service worker\n');

    const offscreenLogs = [];

    // Listen to service worker console
    swWorker.on('console', msg => {
      const text = msg.text();
      console.log(`[SW ${msg.type()}]:`, text);

      if (text.includes('[Offscreen Status]')) {
        offscreenLogs.push(text);
      }
    });

    console.log('=== Listening to service worker console ===\n');

    // Open wizard
    console.log('Opening wizard...');
    const wizardTarget = await browser.waitForTarget(t => t.url().includes('first_run_wizard.html'));
    const wizardPage = await wizardTarget.page();

    // Complete wizard
    console.log('Completing wizard...');
    await wizardPage.waitForSelector('#step-1.active', { timeout: 5000 });
    await wizardPage.waitForFunction(() => !document.getElementById('btn-next').disabled, { timeout: 15000 });
    await wizardPage.click('#btn-next');
    await new Promise(resolve => setTimeout(resolve, 2000));

    // Select WebLLM
    await wizardPage.evaluate(() => {
      const options = Array.from(document.querySelectorAll('.adapter-option'));
      const webllm = options.find(el => el.textContent.includes('WebLLM'));
      if (webllm) webllm.click();
    });

    // Wait for model dropdown to appear
    await wizardPage.waitForSelector('.model-dropdown', { timeout: 5000 });
    await new Promise(resolve => setTimeout(resolve, 500));

    await wizardPage.select('.model-dropdown', 'Gemma-2B-it-q4f16_1');
    await wizardPage.click('#btn-next');
    await new Promise(resolve => setTimeout(resolve, 3000));

    console.log('✓ Wizard completed\n');

    // Wait for config to load and offscreen to initialize
    console.log('=== Waiting 10 seconds for offscreen initialization ===\n');
    await new Promise(resolve => setTimeout(resolve, 10000));

    console.log('\n=== Offscreen Status Log Summary ===');
    if (offscreenLogs.length === 0) {
      console.log('✗ No offscreen status messages received!');
    } else {
      console.log(`✓ Received ${offscreenLogs.length} offscreen status messages:`);
      offscreenLogs.forEach((log, i) => {
        console.log(`  ${i + 1}. ${log}`);
      });
    }

    // Keep browser open for inspection
    console.log('\nBrowser will stay open for 60 seconds for inspection...');
    await new Promise(resolve => setTimeout(resolve, 60000));

  } catch (error) {
    console.error('Test failed:', error);
    throw error;
  } finally {
    if (browser) {
      await browser.close();
      console.log('\nBrowser closed');
    }
  }
}

main().catch(error => {
  console.error('Fatal error:', error);
  process.exit(1);
});
