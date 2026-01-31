#!/usr/bin/env node

/**
 * Test that reads diagnostic logs from chrome.storage
 */

const puppeteer = require('puppeteer');
const path = require('path');

async function main() {
  let browser;

  try {
    console.log('\n=== Diagnostic Logs Test ===\n');

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
    const extensionId = swTarget.url().split('/')[2];
    console.log('✓ Extension ID:', extensionId);

    // Open wizard
    const wizardTarget = await browser.waitForTarget(t => t.url().includes('first_run_wizard.html'));
    const wizardPage = await wizardTarget.page();

    // Complete wizard
    console.log('\nCompleting wizard...');
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

    // Wait for model dropdown
    await wizardPage.waitForSelector('.model-dropdown', { timeout: 5000 });
    await new Promise(resolve => setTimeout(resolve, 500));

    await wizardPage.select('.model-dropdown', 'Gemma-2B-it-q4f16_1');
    await wizardPage.click('#btn-next');
    await new Promise(resolve => setTimeout(resolve, 3000));

    console.log('✓ Wizard completed\n');

    // Wait a bit for config to load
    await new Promise(resolve => setTimeout(resolve, 5000));

    // Read diagnostic logs via message
    console.log('=== Reading Diagnostic Logs ===\n');
    const worker = await swTarget.worker();
    const diagnosticResponse = await worker.evaluate(() => {
      return new Promise((resolve) => {
        chrome.runtime.sendMessage({ type: 'get_diagnostic_logs' }, response => {
          resolve(response);
        });
      });
    });

    if (diagnosticResponse && diagnosticResponse.logs) {
      console.log(`Found ${diagnosticResponse.logs.length} diagnostic log entries:\n`);
      diagnosticResponse.logs.forEach((entry, i) => {
        const time = new Date(entry.timestamp).toLocaleTimeString();
        const data = Object.keys(entry.data).length > 0 ? JSON.stringify(entry.data) : '';
        console.log(`${i + 1}. [${time}] ${entry.message} ${data}`);
      });
    } else {
      console.log('✗ No diagnostic logs found');
    }

    // Keep browser open for inspection
    console.log('\n\nBrowser will stay open for 30 seconds for inspection...');
    await new Promise(resolve => setTimeout(resolve, 30000));

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
