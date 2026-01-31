#!/usr/bin/env node

/**
 * DEBUG TEST: Check if we can connect to offscreen and see its state
 */

const puppeteer = require('puppeteer');
const path = require('path');

async function main() {
  let browser;

  try {
    console.log('\n=== Offscreen Debug Test ===\n');

    // Launch browser with extension
    const extensionPath = path.join(__dirname, 'extension-w');
    browser = await puppeteer.launch({
      headless: false,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
        '--auto-open-devtools-for-tabs',
      ],
    });

    console.log('✓ Browser launched');

    // Wait for extension to load
    await new Promise(resolve => setTimeout(resolve, 3000));

    // Find extension service worker
    const targets = await browser.targets();
    const swTarget = targets.find(t => t.type() === 'service_worker');
    if (!swTarget) throw new Error('Service worker not found');

    console.log('✓ Extension ID:', swTarget.url().split('/')[2]);

    // Open wizard
    const wizardTarget = await browser.waitForTarget(t => t.url().includes('first_run_wizard.html'));
    const wizardPage = await wizardTarget.page();

    // Complete wizard
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
    await new Promise(resolve => setTimeout(resolve, 1000));

    await wizardPage.select('.model-dropdown', 'Gemma-2B-it-q4f16_1');
    await wizardPage.click('#btn-next');
    await new Promise(resolve => setTimeout(resolve, 3000));

    console.log('✓ Wizard completed');

    // Check for offscreen
    await new Promise(resolve => setTimeout(resolve, 3000));
    const allTargets = await browser.targets();

    console.log('\n=== All Targets ===');
    allTargets.forEach((t, i) => {
      console.log(`${i}: type=${t.type()}, url=${t.url()}`);
    });

    const offscreenTarget = allTargets.find(t => t.url().includes('offscreen.html'));

    if (!offscreenTarget) {
      console.log('\n✗ No offscreen document found!');
      return;
    }

    console.log('\n✓ Offscreen target found:', offscreenTarget.url());
    console.log('  Type:', offscreenTarget.type());

    // Try to get the page
    console.log('\n=== Trying to connect to offscreen page ===');
    try {
      const offscreenPage = await offscreenTarget.page();
      console.log('✓ Got offscreen page object');

      if (!offscreenPage) {
        console.log('✗ offscreenPage is null!');
        return;
      }

      // Try to evaluate in the page
      console.log('\n=== Evaluating code in offscreen context ===');
      const hasWindow = await offscreenPage.evaluate(() => typeof window !== 'undefined');
      console.log('Has window:', hasWindow);

      const hasChrome = await offscreenPage.evaluate(() => typeof chrome !== 'undefined');
      console.log('Has chrome:', hasChrome);

      const scriptTags = await offscreenPage.evaluate(() => {
        return Array.from(document.querySelectorAll('script')).map(s => ({
          src: s.src,
          type: s.type
        }));
      });
      console.log('Script tags:', JSON.stringify(scriptTags, null, 2));

      const errors = await offscreenPage.evaluate(() => {
        return window.__errors || [];
      });
      console.log('Window errors:', errors);

      // Set up console listener
      console.log('\n=== Setting up console listener ===');
      offscreenPage.on('console', msg => {
        console.log(`[OFFSCREEN CONSOLE ${msg.type()}]:`, msg.text());
      });

      offscreenPage.on('pageerror', error => {
        console.log('[OFFSCREEN PAGE ERROR]:', error.message);
      });

      // Wait and see if any logs appear
      console.log('\n=== Waiting 10 seconds for console logs ===');
      await new Promise(resolve => setTimeout(resolve, 10000));

      console.log('\n=== Test complete ===');

    } catch (error) {
      console.log('✗ Error getting offscreen page:', error.message);
      console.log('Error stack:', error.stack);
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
      console.log('Browser closed');
    }
  }
}

main().catch(error => {
  console.error('Fatal error:', error);
  process.exit(1);
});
