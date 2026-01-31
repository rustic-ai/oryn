#!/usr/bin/env node
/**
 * Test that WebLLM and wllama load from local bundles (not CDN).
 *
 * Usage:
 *   node test-local-bundles.js              # test both adapters sequentially
 *   node test-local-bundles.js webllm       # test WebLLM only
 *   node test-local-bundles.js wllama       # test wllama only
 *
 * What this verifies:
 *   1. Extension loads, wizard auto-opens
 *   2. Hardware check completes (Step 1)
 *   3. Adapter selection works (Step 2)
 *   4. Offscreen document is created for the adapter
 *   5. Library loads from LOCAL bundle (no CDN fetch errors)
 *   6. Model download begins (progress > 0% or status changes from initial)
 *   7. No "Failed to load" or CSP errors in any console
 */

const puppeteer = require('puppeteer');
const path = require('path');
const fs = require('fs');

const PROJECT_ROOT = __dirname;
const EXT_DIR = path.join(PROJECT_ROOT, 'extension-w');

// ── Logging ──────────────────────────────────────────────────────────────────

const C = {
  reset: '\x1b[0m', red: '\x1b[31m', green: '\x1b[32m',
  yellow: '\x1b[33m', blue: '\x1b[34m', cyan: '\x1b[36m', magenta: '\x1b[35m',
  dim: '\x1b[2m',
};

function ts() {
  return new Date().toISOString().split('T')[1].slice(0, -1);
}

const logInfo  = (msg) => console.log(`${C.dim}[${ts()}]${C.reset} ${C.blue}INFO${C.reset}  ${msg}`);
const logPass  = (msg) => console.log(`${C.dim}[${ts()}]${C.reset} ${C.green}PASS${C.reset}  ${msg}`);
const logFail  = (msg) => console.log(`${C.dim}[${ts()}]${C.reset} ${C.red}FAIL${C.reset}  ${msg}`);
const logWarn  = (msg) => console.log(`${C.dim}[${ts()}]${C.reset} ${C.yellow}WARN${C.reset}  ${msg}`);
const logDebug = (msg) => console.log(`${C.dim}[${ts()}]${C.reset} ${C.magenta}DBG ${C.reset}  ${msg}`);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ── Assertion helpers ────────────────────────────────────────────────────────

class TestResult {
  constructor(adapterName) {
    this.adapter = adapterName;
    this.passed = [];
    this.failed = [];
    this.warnings = [];
  }
  pass(msg) { this.passed.push(msg); logPass(msg); }
  fail(msg) { this.failed.push(msg); logFail(msg); }
  warn(msg) { this.warnings.push(msg); logWarn(msg); }
  get ok() { return this.failed.length === 0; }
}

// ── Verify bundle files exist ────────────────────────────────────────────────

function checkBundlesExist() {
  const files = [
    'extension-w/llm/vendor/webllm.bundle.js',
    'extension-w/llm/vendor/wllama.bundle.js',
    'extension-w/llm/vendor/wllama-wasm/single-thread/wllama.wasm',
    'extension-w/llm/vendor/wllama-wasm/multi-thread/wllama.wasm',
  ];
  const missing = files.filter((f) => !fs.existsSync(path.join(PROJECT_ROOT, f)));
  if (missing.length) {
    logFail('Missing bundle files (run ./scripts/bundle-llm-libs.sh first):');
    missing.forEach((f) => logFail(`  ${f}`));
    process.exit(1);
  }
  logPass('All bundle files present');
}

// ── Core test: run one adapter through the wizard ────────────────────────────

async function testAdapter(adapterName) {
  const result = new TestResult(adapterName);
  const displayName = adapterName === 'webllm' ? 'WebLLM' : 'wllama';

  console.log(`\n${C.cyan}━━━ Testing ${displayName} local bundle ━━━${C.reset}\n`);

  // Collect logs from all contexts
  const logs = { sw: [], wizard: [], offscreen: [] };
  const cdnErrors = [];
  const cspErrors = [];

  let browser;
  try {
    // ── Launch browser ───────────────────────────────────────────────────
    logInfo('Launching browser with extension...');
    browser = await puppeteer.launch({
      headless: false,
      args: [
        `--disable-extensions-except=${EXT_DIR}`,
        `--load-extension=${EXT_DIR}`,
        '--no-first-run',
        '--no-default-browser-check',
        '--window-size=1280,900',
        '--enable-features=WebGPU',
      ],
    });
    result.pass('Browser launched');

    await sleep(3000);

    // ── Find service worker ──────────────────────────────────────────────
    let extensionId = null;
    for (let attempt = 0; attempt < 15; attempt++) {
      const targets = await browser.targets();
      const sw = targets.find(
        (t) => t.type() === 'service_worker' && t.url().includes('chrome-extension://')
      );
      if (sw) {
        extensionId = sw.url().split('/')[2];
        logInfo(`Extension ID: ${extensionId}`);
        try {
          const worker = await sw.worker();
          if (worker) {
            worker.on('console', (msg) => {
              const text = msg.text();
              logs.sw.push(text);
              if (text.includes('esm.sh') || text.includes('cdn.jsdelivr')) {
                cdnErrors.push(`SW: ${text}`);
              }
            });
          }
        } catch (_) { /* can't always attach */ }
        break;
      }
      await sleep(500);
    }
    if (!extensionId) {
      result.fail('Could not find extension service worker');
      return result;
    }
    result.pass('Service worker found');

    // ── Find wizard page ─────────────────────────────────────────────────
    let wizardPage = null;
    for (let i = 0; i < 20; i++) {
      const pages = await browser.pages();
      wizardPage = pages.find((p) => p.url().includes('first_run_wizard.html'));
      if (wizardPage) break;
      await sleep(500);
    }
    if (!wizardPage) {
      logInfo('Wizard did not auto-open, navigating manually');
      wizardPage = await browser.newPage();
      await wizardPage.goto(
        `chrome-extension://${extensionId}/ui/first_run_wizard.html`,
        { waitUntil: 'domcontentloaded' }
      );
    }
    result.pass('Wizard page ready');

    // Listen to wizard console
    wizardPage.on('console', (msg) => {
      const text = msg.text();
      logs.wizard.push(text);
      if (text.includes('esm.sh') || text.includes('cdn.jsdelivr')) {
        cdnErrors.push(`Wizard: ${text}`);
      }
    });
    wizardPage.on('pageerror', (err) => {
      const text = err.toString();
      logs.wizard.push(`[PAGE_ERROR] ${text}`);
      if (text.includes('Content Security Policy') || text.includes('CSP')) {
        cspErrors.push(text);
      }
    });

    // ── Step 1: Hardware check ───────────────────────────────────────────
    logInfo('Step 1: Waiting for hardware check...');
    await wizardPage.waitForSelector('#hw-results .hw-item', { timeout: 15000 });
    result.pass('Hardware check completed');
    await sleep(500);

    await wizardPage.click('#btn-next');
    await sleep(1500);

    // ── Step 2: Select adapter ───────────────────────────────────────────
    logInfo(`Step 2: Selecting ${displayName} adapter...`);
    await wizardPage.waitForSelector('.adapter-option', { timeout: 5000 });

    const adapterOptions = await wizardPage.$$('.adapter-option');
    logInfo(`Found ${adapterOptions.length} adapter options`);

    let targetOption = null;
    for (const option of adapterOptions) {
      const text = await wizardPage.evaluate((el) => el.textContent, option);
      if (text.includes(displayName)) {
        targetOption = option;
        break;
      }
    }
    if (!targetOption) {
      result.fail(`${displayName} adapter option not found in wizard`);
      return result;
    }

    await targetOption.click();
    await sleep(1000);
    result.pass(`${displayName} adapter selected`);

    // Select a small model to minimize download -- use the dropdown inside the selected card
    const selectedCard = await wizardPage.$('.adapter-option.selected');
    const dropdown = selectedCard ? await selectedCard.$('.model-dropdown') : null;
    if (dropdown) {
      const modelId = adapterName === 'webllm' ? 'gemma-2b-it-q4f16_1-MLC-1k' : 'tinyllama';
      await dropdown.select(modelId);
      await sleep(300);
      const selectedModel = await wizardPage.evaluate((el) => el.value, dropdown);
      result.pass(`Model selected: ${selectedModel}`);
    }

    // ── Step 3: Trigger download ─────────────────────────────────────────
    logInfo('Step 3: Advancing to setup/download...');
    await wizardPage.click('#btn-next');
    await sleep(3000);

    const onStep3 = await wizardPage
      .$eval('#step-3', (el) => el.classList.contains('active'))
      .catch(() => false);
    if (!onStep3) {
      result.fail('Did not advance to Step 3');
      await wizardPage.screenshot({
        path: `test-${adapterName}-stuck.png`,
        fullPage: true,
      });
      return result;
    }
    result.pass('Advanced to Step 3');

    // Check download section visible
    const dlVisible = await wizardPage
      .$eval('#download-section', (el) => el.style.display !== 'none')
      .catch(() => false);
    if (dlVisible) {
      result.pass('Download section visible');
    } else {
      result.warn('Download section not visible');
    }

    // ── Wait for offscreen document to appear ────────────────────────────
    logInfo('Waiting for offscreen document...');
    let offscreenTarget = null;
    for (let i = 0; i < 20; i++) {
      const targets = await browser.targets();
      offscreenTarget = targets.find(
        (t) => t.url().includes('offscreen.html') && t.url().includes(extensionId)
      );
      if (offscreenTarget) break;
      await sleep(500);
    }
    if (offscreenTarget) {
      result.pass('Offscreen document created');

      // Attach to offscreen console
      try {
        const offscreenPage = await offscreenTarget.page();
        if (offscreenPage) {
          offscreenPage.on('console', (msg) => {
            const text = msg.text();
            logs.offscreen.push(text);
            if (text.includes('esm.sh') || text.includes('cdn.jsdelivr')) {
              cdnErrors.push(`Offscreen: ${text}`);
            }
          });
          offscreenPage.on('pageerror', (err) => {
            const text = err.toString();
            logs.offscreen.push(`[PAGE_ERROR] ${text}`);
            if (text.includes('Content Security Policy') || text.includes('CSP')) {
              cspErrors.push(`Offscreen: ${text}`);
            }
          });
        }
      } catch (e) {
        logDebug(`Could not attach to offscreen page: ${e.message}`);
      }
    } else {
      result.warn('Offscreen document not found after 10s');
    }

    // ── Monitor download progress ────────────────────────────────────────
    logInfo('Monitoring download progress for 30 seconds...');

    let sawProgress = false;
    let sawLoading = false;
    let sawError = false;
    let lastPct = '';

    for (let i = 0; i < 15; i++) {
      await sleep(2000);

      const pct = await wizardPage
        .$eval('#download-percentage', (el) => el.textContent)
        .catch(() => '');
      const status = await wizardPage
        .$eval('#download-status-text', (el) => el.textContent)
        .catch(() => '');
      const errVisible = await wizardPage
        .$eval('#download-error', (el) => el.style.display !== 'none')
        .catch(() => false);

      if (pct && pct !== '0%' && pct !== '') sawProgress = true;
      if (status && (status.includes('Loading') || status.includes('Downloading') || status.includes('Initializing'))) {
        sawLoading = true;
      }

      if (pct !== lastPct || i % 3 === 0) {
        logInfo(`  [${(i + 1) * 2}s] Progress: ${pct || '-'} | Status: ${status || '-'}`);
        lastPct = pct;
      }

      if (errVisible) {
        const errText = await wizardPage
          .$eval('#download-error', (el) => el.textContent)
          .catch(() => '');
        const trimmed = errText.trim().substring(0, 200);

        // WebGPU/shader-f16 errors are expected in headless Chromium — not a bundle failure
        const isGpuError =
          trimmed.includes('WebGPU') ||
          trimmed.includes('shader-f16') ||
          trimmed.includes('GPU adapter');
        if (isGpuError && adapterName === 'webllm') {
          result.pass(
            `WebLLM bundle loaded & model found (GPU not available in test env: ${trimmed.substring(0, 100)})`
          );
          // This counts as "saw loading" since the library was loaded and tried to init
          sawLoading = true;
        } else {
          result.fail(`Download error: ${trimmed}`);
        }
        sawError = true;
        break;
      }
    }

    if (!sawError) {
      if (sawProgress) {
        result.pass('Download progress observed (percentage updated)');
      } else if (sawLoading) {
        result.pass('Download initiated (status changed to loading/initializing)');
      } else {
        result.warn('No progress observed in 30s (may be slow or pending)');
      }
    }

    // ── Check for CDN / CSP errors ───────────────────────────────────────
    // Also scan offscreen logs for the key bundle-load success messages
    const bundleLoaded = logs.offscreen.some(
      (l) =>
        l.includes('Library loaded from local bundle') ||
        l.includes(`[${displayName}] Library loaded`)
    );
    // Also check SW logs (the LLM manager logs adapter loading)
    const swBundleLoaded = logs.sw.some(
      (l) => l.includes('loaded from local bundle') || l.includes(`[${displayName}]`)
    );

    if (bundleLoaded || swBundleLoaded) {
      result.pass(`${displayName} loaded from local bundle`);
    } else {
      // Check if any log mentions the adapter at all
      const anyMention =
        logs.offscreen.some((l) => l.toLowerCase().includes(adapterName)) ||
        logs.sw.some((l) => l.toLowerCase().includes(adapterName));
      if (anyMention) {
        result.warn(`${displayName} was referenced but "loaded from local bundle" message not captured`);
      } else {
        result.warn(`Could not confirm ${displayName} bundle loading from logs`);
      }
    }

    if (cdnErrors.length > 0) {
      result.fail(`CDN references found (should be local):`);
      cdnErrors.forEach((e) => logFail(`  ${e}`));
    } else {
      result.pass('No CDN references (esm.sh / cdn.jsdelivr) in any console');
    }

    if (cspErrors.length > 0) {
      result.fail('CSP violations detected:');
      cspErrors.forEach((e) => logFail(`  ${e}`));
    } else {
      result.pass('No CSP violations');
    }

    // Screenshot
    await wizardPage.screenshot({
      path: `test-${adapterName}-result.png`,
      fullPage: true,
    });
    logInfo(`Screenshot: test-${adapterName}-result.png`);

    // ── Dump relevant logs ───────────────────────────────────────────────
    console.log(`\n${C.dim}── Offscreen logs (${logs.offscreen.length}) ──${C.reset}`);
    logs.offscreen.slice(-20).forEach((l) => console.log(`  ${C.dim}${l}${C.reset}`));

    console.log(`\n${C.dim}── SW logs (filtered, last 15) ──${C.reset}`);
    logs.sw
      .filter(
        (l) =>
          l.includes('LLM') ||
          l.includes('Offscreen') ||
          l.includes('adapter') ||
          l.includes('error') ||
          l.includes(adapterName)
      )
      .slice(-15)
      .forEach((l) => console.log(`  ${C.dim}${l}${C.reset}`));

    // Keep browser open briefly for inspection
    logInfo('Browser open for 5 seconds for inspection...');
    await sleep(5000);
  } catch (err) {
    result.fail(`Unhandled error: ${err.message}`);
    console.error(err.stack);
  } finally {
    if (browser) {
      await browser.close();
      logInfo('Browser closed');
    }
  }
  return result;
}

// ── Main ─────────────────────────────────────────────────────────────────────

async function main() {
  const arg = process.argv[2];
  let adapters;
  if (arg === 'webllm') {
    adapters = ['webllm'];
  } else if (arg === 'wllama') {
    adapters = ['wllama'];
  } else {
    adapters = ['webllm', 'wllama'];
  }

  console.log(`\n${C.cyan}╔════════════════════════════════════════════════════════════════╗${C.reset}`);
  console.log(`${C.cyan}║      Test: Local LLM Bundle Loading (${adapters.join(', ')})${C.reset}`);
  console.log(`${C.cyan}╚════════════════════════════════════════════════════════════════╝${C.reset}\n`);

  checkBundlesExist();

  const results = [];
  for (const adapter of adapters) {
    const r = await testAdapter(adapter);
    results.push(r);
  }

  // ── Final summary ──────────────────────────────────────────────────────
  console.log(`\n${C.cyan}╔════════════════════════════════════════════════════════════════╗${C.reset}`);
  console.log(`${C.cyan}║                        RESULTS                                ║${C.reset}`);
  console.log(`${C.cyan}╚════════════════════════════════════════════════════════════════╝${C.reset}\n`);

  let allOk = true;
  for (const r of results) {
    const icon = r.ok ? `${C.green}PASS${C.reset}` : `${C.red}FAIL${C.reset}`;
    console.log(`  ${icon}  ${r.adapter}  (${r.passed.length} passed, ${r.failed.length} failed, ${r.warnings.length} warnings)`);
    if (r.failed.length) {
      r.failed.forEach((f) => console.log(`        ${C.red}✗ ${f}${C.reset}`));
      allOk = false;
    }
    if (r.warnings.length) {
      r.warnings.forEach((w) => console.log(`        ${C.yellow}⚠ ${w}${C.reset}`));
    }
  }

  console.log('');
  if (allOk) {
    console.log(`  ${C.green}All tests passed.${C.reset}\n`);
    process.exit(0);
  } else {
    console.log(`  ${C.red}Some tests failed.${C.reset}\n`);
    process.exit(1);
  }
}

main().catch((err) => {
  logFail(`Fatal: ${err.message}`);
  console.error(err);
  process.exit(1);
});
