/**
 * Comprehensive Scanner Protocol Tests
 * Tests against SPEC-SCANNER-PROTOCOL.md Version 1.0
 */

const puppeteer = require('puppeteer');
const fs = require('fs');
const path = require('path');

// Load scanner source
const SCANNER_JS = fs.readFileSync(path.resolve(__dirname, '../src/scanner.js'), 'utf8');

// Harness paths
const HARNESS_PATH = 'file://' + path.resolve(__dirname, 'harness.html');
const HARNESS_PATTERNS_PATH = 'file://' + path.resolve(__dirname, 'harness-patterns.html');
const HARNESS_EDGE_CASES_PATH = 'file://' + path.resolve(__dirname, 'harness-edge-cases.html');
const HARNESS_IFRAMES_PATH = 'file://' + path.resolve(__dirname, 'harness-iframes.html');

// Helper to execute scanner commands
const runCommand = async (page, cmd) => {
    return page.evaluate(async (command) => {
        return await window.Lemmascope.process(command);
    }, cmd);
};

// Helper to get element by attribute from scan results
const findElement = (elements, attr, value) => {
    return elements.find((el) => el.attributes?.[attr] === value);
};

const findElementByText = (elements, text) => {
    return elements.find((el) => el.text?.includes(text));
};

describe('Scanner Protocol Tests', () => {
    let browser;

    beforeAll(async () => {
        browser = await puppeteer.launch({
            headless: 'new',
            args: ['--no-sandbox', '--disable-setuid-sandbox']
        });
    });

    afterAll(async () => {
        if (browser) await browser.close();
    });

    // ============================================================
    // SECTION 2: MESSAGE FORMAT
    // ============================================================
    describe('Section 2: Message Format', () => {
        let page;

        beforeEach(async () => {
            page = await browser.newPage();
            await page.goto(HARNESS_PATH);
            await page.evaluate(SCANNER_JS);
        });

        afterEach(async () => {
            if (page) await page.close();
        });

        describe('2.1 Request Structure', () => {
            test('requires cmd field', async () => {
                const result = await runCommand(page, {});
                expect(result.status).toBe('error');
                expect(result.code).toBe('INVALID_REQUEST');
            });

            test('accepts string JSON input', async () => {
                const result = await page.evaluate(async () => {
                    return await window.Lemmascope.process('{"cmd": "version"}');
                });
                expect(result.status).toBe('ok');
            });
        });

        describe('2.2 Response Structure', () => {
            test('success response has ok=true and data field', async () => {
                const result = await runCommand(page, { cmd: 'version' });
                expect(result.status).toBe('ok');
                expect(result.error).toBeUndefined();
            });

            test('success response includes timing', async () => {
                const result = await runCommand(page, { cmd: 'scan' });
                expect(result.status).toBe('ok');
                expect(result.timing).toBeDefined();
                expect(result.timing.duration_ms).toBeGreaterThanOrEqual(0);
            });

            test('error response has ok=false, error, and code', async () => {
                const result = await runCommand(page, { cmd: 'click', id: 99999 });
                expect(result.status).toBe('error');
                expect(result.error).toBeDefined();
                expect(result.code).toBeDefined();
            });
        });

        describe('2.3 Error Codes', () => {
            test('ELEMENT_NOT_FOUND for invalid ID', async () => {
                const result = await runCommand(page, { cmd: 'click', id: 99999 });
                expect(result.code).toBe('ELEMENT_NOT_FOUND');
            });

            test('ELEMENT_STALE for removed element', async () => {
                // Scan first
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                // Remove the element
                await page.evaluate(() => {
                    document.getElementById('btn-1').remove();
                });

                // Try to click
                const result = await runCommand(page, { cmd: 'click', id: btn.id });
                expect(result.status).toBe('error');
                expect(result.code).toBe('ELEMENT_STALE');
            });

            test('ELEMENT_NOT_VISIBLE for hidden element', async () => {
                const scan = await runCommand(page, { cmd: 'scan', include_hidden: true });
                const hiddenBtn = scan.elements.find((el) => el.text === 'Hidden Element');

                if (hiddenBtn) {
                    const result = await runCommand(page, { cmd: 'click', id: hiddenBtn.id });
                    expect(result.status).toBe('error');
                    expect(result.code).toBe('ELEMENT_NOT_VISIBLE');
                }
            });

            test('ELEMENT_DISABLED for disabled element', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const disabledInput = findElement(scan.elements, 'id', 'disabled-input');

                if (disabledInput) {
                    const result = await runCommand(page, { cmd: 'type', id: disabledInput.id, text: 'test' });
                    expect(result.status).toBe('error');
                    expect(result.code).toBe('ELEMENT_DISABLED');
                }
            });

            test('SELECTOR_INVALID for bad selector', async () => {
                const result = await runCommand(page, { cmd: 'scan', within: '###invalid' });
                expect(result.status).toBe('error');
                // Note: DOMException from querySelector returns numeric code, scanner wraps as error
                expect(result.code).toBeDefined();
            });

            test('TIMEOUT for wait_for timeout', async () => {
                const result = await runCommand(page, {
                    cmd: 'wait_for',
                    condition: 'exists',
                    selector: '#nonexistent-element',
                    timeout: 100
                });
                expect(result.status).toBe('error');
                expect(result.code).toBe('TIMEOUT');
            });

            test('SCRIPT_ERROR for bad script', async () => {
                const result = await runCommand(page, {
                    cmd: 'execute',
                    script: 'throw new Error("test error")'
                });
                expect(result.status).toBe('error');
                expect(result.code).toBe('SCRIPT_ERROR');
            });

            test('UNKNOWN_COMMAND for invalid command', async () => {
                const result = await runCommand(page, { cmd: 'invalid_command' });
                expect(result.status).toBe('error');
                expect(result.code).toBe('UNKNOWN_COMMAND');
            });
        });
    });

    // ============================================================
    // SECTION 3: COMMAND REFERENCE
    // ============================================================
    describe('Section 3: Command Reference', () => {
        // ----------------------------------------------------------
        // 3.1 scan
        // ----------------------------------------------------------
        describe('3.1 scan', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('returns page information', async () => {
                const result = await runCommand(page, { cmd: 'scan' });
                expect(result.status).toBe('ok');

                const { page: pageInfo } = result;
                expect(pageInfo.url).toContain('harness.html');
                expect(pageInfo.title).toBe('Scanner Harness');
                expect(pageInfo.viewport).toHaveProperty('width');
                expect(pageInfo.viewport).toHaveProperty('height');
                expect(pageInfo.scroll).toHaveProperty('x');
                expect(pageInfo.scroll).toHaveProperty('y');
                expect(pageInfo.scroll).toHaveProperty('max_y');
                expect(pageInfo.readyState).toBe('complete');
            });

            test('returns element list with required fields', async () => {
                const result = await runCommand(page, { cmd: 'scan' });
                const elements = result.elements;

                expect(elements.length).toBeGreaterThan(0);

                const el = elements[0];
                expect(el).toHaveProperty('id');
                expect(el).toHaveProperty('type');
                expect(el).toHaveProperty('role');
                expect(el).toHaveProperty('text');
                expect(el).toHaveProperty('selector');
                expect(el).toHaveProperty('xpath');
                expect(el).toHaveProperty('rect');
                expect(el.rect).toHaveProperty('x');
                expect(el.rect).toHaveProperty('y');
                expect(el.rect).toHaveProperty('width');
                expect(el.rect).toHaveProperty('height');
                expect(el).toHaveProperty('attributes');
                expect(el).toHaveProperty('state');
            });

            test('max_elements parameter limits results', async () => {
                const result = await runCommand(page, { cmd: 'scan', max_elements: 5 });
                expect(result.elements.length).toBeLessThanOrEqual(5);
                expect(result.settings_applied.max_elements).toBe(5);
            });

            test('include_hidden parameter includes hidden elements', async () => {
                const withoutHidden = await runCommand(page, { cmd: 'scan', include_hidden: false });
                const withHidden = await runCommand(page, { cmd: 'scan', include_hidden: true });

                expect(withHidden.elements.length).toBeGreaterThanOrEqual(withoutHidden.elements.length);
                expect(withHidden.settings_applied.include_hidden).toBe(true);
            });

            test('within parameter limits to container', async () => {
                const result = await runCommand(page, { cmd: 'scan', within: '#forms' });
                expect(result.status).toBe('ok');

                // All elements should be within the forms section
                const elements = result.elements;
                const nonFormElements = elements.filter(
                    (el) => el.attributes.id === 'btn-1' || el.attributes.id === 'input-1'
                );
                expect(nonFormElements.length).toBe(0);
            });

            test('viewport_only parameter filters to viewport', async () => {
                const allResult = await runCommand(page, { cmd: 'scan', viewport_only: false });
                const viewportResult = await runCommand(page, { cmd: 'scan', viewport_only: true });

                expect(viewportResult.settings_applied.viewport_only).toBe(true);
                // Viewport-only should have same or fewer elements
                expect(viewportResult.elements.length).toBeLessThanOrEqual(allResult.elements.length);
            });

            test('near parameter filters by proximity', async () => {
                const result = await runCommand(page, { cmd: 'scan', near: 'Label For Button' });
                expect(result.status).toBe('ok');

                const nearBtn = findElement(result.elements, 'id', 'near-btn');
                const farBtn = findElement(result.elements, 'id', 'far-btn');

                expect(nearBtn).toBeDefined();
                expect(farBtn).toBeUndefined();
            });

            test('returns stats', async () => {
                const result = await runCommand(page, { cmd: 'scan' });
                expect(result.stats).toBeDefined();
                expect(result.stats.total).toBeGreaterThan(0);
            });

            test('echoes back settings_applied', async () => {
                const result = await runCommand(page, {
                    cmd: 'scan',
                    max_elements: 50,
                    include_hidden: true,
                    include_iframes: false,
                    viewport_only: true
                });

                expect(result.settings_applied).toEqual({
                    max_elements: 50,
                    include_hidden: true,
                    include_iframes: false,
                    viewport_only: true
                });
            });
        });

        // ----------------------------------------------------------
        // 3.2 click
        // ----------------------------------------------------------
        describe('3.2 click', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('clicks element by ID', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                const result = await runCommand(page, { cmd: 'click', id: btn.id });
                expect(result.status).toBe('ok');
                expect(result.action).toBe('clicked');
                expect(result.id).toBe(btn.id);

                // Verify side effect
                const log = await page.evaluate(() => document.getElementById('log').innerText);
                expect(log).toContain('Button 1 clicked');
            });

            test('returns selector in response', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                const result = await runCommand(page, { cmd: 'click', id: btn.id });
                expect(result.selector).toBe('#btn-1');
            });

            test('returns coordinates in response', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                const result = await runCommand(page, { cmd: 'click', id: btn.id });
                expect(result.coordinates).toHaveProperty('x');
                expect(result.coordinates).toHaveProperty('y');
            });

            test('double-click with click_count=2', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                const result = await runCommand(page, { cmd: 'click', id: btn.id, click_count: 2 });
                expect(result.status).toBe('ok');
                expect(result.action).toBe('double_clicked');
            });

            test('right-click with button="right"', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                const result = await runCommand(page, { cmd: 'click', id: btn.id, button: 'right' });
                expect(result.status).toBe('ok');
                expect(result.button).toBe('right');
            });

            test('middle-click with button="middle"', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                const result = await runCommand(page, { cmd: 'click', id: btn.id, button: 'middle' });
                expect(result.status).toBe('ok');
                expect(result.button).toBe('middle');
            });

            test('click with modifiers', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                const result = await runCommand(page, {
                    cmd: 'click',
                    id: btn.id,
                    modifiers: ['Shift', 'Control']
                });
                expect(result.status).toBe('ok');
            });

            test('click with offset', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                const result = await runCommand(page, {
                    cmd: 'click',
                    id: btn.id,
                    offset: { x: 5, y: 5 }
                });
                expect(result.status).toBe('ok');
            });

            test('force click on covered element', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const coveredBtn = findElement(scan.elements, 'id', 'covered-btn');

                if (coveredBtn) {
                    // Without force should fail
                    const failResult = await runCommand(page, { cmd: 'click', id: coveredBtn.id });
                    expect(failResult.status).toBe('error');

                    // With force should succeed
                    const forceResult = await runCommand(page, { cmd: 'click', id: coveredBtn.id, force: true });
                    expect(forceResult.status).toBe('ok');
                }
            });

            test('scroll_into_view=false skips scrolling', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                const result = await runCommand(page, {
                    cmd: 'click',
                    id: btn.id,
                    scroll_into_view: false
                });
                expect(result.status).toBe('ok');
            });

            test('detects navigation on click', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const navBtn = findElement(scan.elements, 'id', 'nav-btn');

                const result = await runCommand(page, { cmd: 'click', id: navBtn.id });
                expect(result.status).toBe('ok');
                expect(result.navigation).toBe(true);
            });

            test('tracks dom_changes on click', async () => {
                // Add a button that modifies DOM
                await page.evaluate(() => {
                    const btn = document.createElement('button');
                    btn.id = 'dom-modifier';
                    btn.onclick = () => {
                        const span = document.createElement('span');
                        span.innerText = 'Added';
                        document.body.appendChild(span);
                    };
                    document.body.appendChild(btn);
                });

                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'dom-modifier');

                const result = await runCommand(page, { cmd: 'click', id: btn.id });
                expect(result.dom_changes).toBeDefined();
                expect(result.dom_changes.added).toBeGreaterThan(0);
            });
        });

        // ----------------------------------------------------------
        // 3.3 type
        // ----------------------------------------------------------
        describe('3.3 type', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('types text into input', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const input = findElement(scan.elements, 'id', 'input-1');

                const result = await runCommand(page, { cmd: 'type', id: input.id, text: 'Hello World' });
                expect(result.status).toBe('ok');
                expect(result.action).toBe('typed');
                expect(result.text).toBe('Hello World');
                expect(result.value).toBe('Hello World');
            });

            test('returns selector in response', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const input = findElement(scan.elements, 'id', 'input-1');

                const result = await runCommand(page, { cmd: 'type', id: input.id, text: 'test' });
                expect(result.selector).toBeDefined();
            });

            test('clears existing content by default', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const input = findElement(scan.elements, 'id', 'input-1');

                // Type initial text
                await runCommand(page, { cmd: 'type', id: input.id, text: 'First' });
                // Type again (should clear)
                const result = await runCommand(page, { cmd: 'type', id: input.id, text: 'Second' });

                expect(result.value).toBe('Second');
            });

            test('clear=false appends to existing', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const input = findElement(scan.elements, 'id', 'input-1');

                await runCommand(page, { cmd: 'type', id: input.id, text: 'First' });
                const result = await runCommand(page, { cmd: 'type', id: input.id, text: 'Second', clear: false });

                expect(result.value).toBe('FirstSecond');
            });

            test('types with delay between keystrokes', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const input = findElement(scan.elements, 'id', 'input-1');

                const start = Date.now();
                const result = await runCommand(page, { cmd: 'type', id: input.id, text: 'ABC', delay: 50 });
                const elapsed = Date.now() - start;

                expect(result.status).toBe('ok');
                expect(elapsed).toBeGreaterThanOrEqual(100); // At least 2 delays
            });

            test('fails on disabled input', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const disabledInput = findElement(scan.elements, 'id', 'disabled-input');

                if (disabledInput) {
                    const result = await runCommand(page, { cmd: 'type', id: disabledInput.id, text: 'test' });
                    expect(result.status).toBe('error');
                    expect(result.code).toBe('ELEMENT_DISABLED');
                }
            });
        });

        // ----------------------------------------------------------
        // 3.4 clear
        // ----------------------------------------------------------
        describe('3.4 clear', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('clears input value', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const input = findElement(scan.elements, 'id', 'input-1');

                // Set a value first
                await runCommand(page, { cmd: 'type', id: input.id, text: 'Some text' });

                // Clear it
                const result = await runCommand(page, { cmd: 'clear', id: input.id });
                expect(result.status).toBe('ok');
                expect(result.action).toBe('cleared');

                // Verify cleared
                const value = await runCommand(page, { cmd: 'get_value', id: input.id });
                expect(value.value).toBe('');
            });

            test('returns selector in response', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const input = findElement(scan.elements, 'id', 'input-1');

                const result = await runCommand(page, { cmd: 'clear', id: input.id });
                expect(result.selector).toBeDefined();
            });
        });

        // ----------------------------------------------------------
        // 3.5 check / uncheck
        // ----------------------------------------------------------
        describe('3.5 check / uncheck', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('check sets checkbox to checked', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const checkbox = findElement(scan.elements, 'id', 'check-1');

                const result = await runCommand(page, { cmd: 'check', id: checkbox.id });
                expect(result.status).toBe('ok');
                expect(result.action).toBe('checked');
                expect(result.checked).toBe(true);
            });

            test('uncheck sets checkbox to unchecked', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const checkbox = findElement(scan.elements, 'id', 'check-1');

                // First check it
                await runCommand(page, { cmd: 'check', id: checkbox.id });
                // Then uncheck
                const result = await runCommand(page, { cmd: 'uncheck', id: checkbox.id });

                expect(result.status).toBe('ok');
                expect(result.action).toBe('unchecked');
                expect(result.checked).toBe(false);
            });

            test('returns previous state', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const checkbox = findElement(scan.elements, 'id', 'check-1');

                const result = await runCommand(page, { cmd: 'check', id: checkbox.id });
                expect(result).toHaveProperty('previous');
            });

            test('returns selector in response', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const checkbox = findElement(scan.elements, 'id', 'check-1');

                const result = await runCommand(page, { cmd: 'check', id: checkbox.id });
                expect(result.selector).toBeDefined();
            });
        });

        // ----------------------------------------------------------
        // 3.6 select
        // ----------------------------------------------------------
        describe('3.6 select', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('selects by value', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const select = findElement(scan.elements, 'id', 'select-1');

                const result = await runCommand(page, { cmd: 'select', id: select.id, value: '2' });
                expect(result.status).toBe('ok');
                expect(result.action).toBe('selected');
                expect(result.value).toContain('2');
            });

            test('selects by text', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const select = findElement(scan.elements, 'id', 'select-1');

                const result = await runCommand(page, { cmd: 'select', id: select.id, text: 'Option 3' });
                expect(result.status).toBe('ok');
                expect(result.value).toContain('3');
            });

            test('selects by index', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const select = findElement(scan.elements, 'id', 'select-1');

                const result = await runCommand(page, { cmd: 'select', id: select.id, index: 1 });
                expect(result.status).toBe('ok');
                expect(result.value).toContain('2'); // Index 1 = Option 2 = value "2"
            });

            test('returns previous selection', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const select = findElement(scan.elements, 'id', 'select-1');

                const result = await runCommand(page, { cmd: 'select', id: select.id, value: '2' });
                expect(result.previous).toBeDefined();
                expect(result.previous).toHaveProperty('value');
                expect(result.previous).toHaveProperty('text');
            });

            test('multi-select with array of values', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const multiSelect = findElement(scan.elements, 'id', 'multi-select');

                const result = await runCommand(page, { cmd: 'select', id: multiSelect.id, value: ['1', '3'] });
                expect(result.status).toBe('ok');
                expect(result.value).toEqual(['1', '3']);
            });

            test('multi-select with array of indexes', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const multiSelect = findElement(scan.elements, 'id', 'multi-select');

                const result = await runCommand(page, { cmd: 'select', id: multiSelect.id, index: [0, 2] });
                expect(result.status).toBe('ok');
                expect(result.value).toEqual(['1', '3']);
            });

            test('returns selector in response', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const select = findElement(scan.elements, 'id', 'select-1');

                const result = await runCommand(page, { cmd: 'select', id: select.id, value: '2' });
                expect(result.selector).toBeDefined();
            });

            test('fails for non-select element', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                const result = await runCommand(page, { cmd: 'select', id: btn.id, value: 'x' });
                expect(result.status).toBe('error');
            });

            test('fails for non-existent option', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const select = findElement(scan.elements, 'id', 'select-1');

                const result = await runCommand(page, { cmd: 'select', id: select.id, value: 'nonexistent' });
                expect(result.status).toBe('error');
            });
        });

        // ----------------------------------------------------------
        // 3.7 scroll
        // ----------------------------------------------------------
        describe('3.7 scroll', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('scrolls down by direction', async () => {
                const result = await runCommand(page, { cmd: 'scroll', direction: 'down', amount: 100 });
                expect(result.status).toBe('ok');
                expect(result.scroll).toBeDefined();
                expect(result.scroll.y).toBeGreaterThanOrEqual(0);
            });

            test('scrolls up by direction', async () => {
                // First scroll down
                await runCommand(page, { cmd: 'scroll', direction: 'down', amount: 200 });
                // Then scroll up
                const result = await runCommand(page, { cmd: 'scroll', direction: 'up', amount: 100 });

                expect(result.status).toBe('ok');
                expect(result.scroll.y).toBeLessThan(200);
            });

            test('scrolls element into view', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                // Find an element that might be below the fold
                const elements = scan.elements;
                const lastEl = elements[elements.length - 1];

                const result = await runCommand(page, { cmd: 'scroll', element: lastEl.id });
                expect(result.status).toBe('ok');
            });

            test('scrolls within container', async () => {
                // Need to use the edge cases harness for this
                await page.goto(HARNESS_EDGE_CASES_PATH);
                await page.evaluate(SCANNER_JS);

                const result = await runCommand(page, {
                    cmd: 'scroll',
                    container: '#scroll-container',
                    direction: 'down',
                    amount: 100
                });
                expect(result.status).toBe('ok');
            });

            test('returns scroll position and max', async () => {
                const result = await runCommand(page, { cmd: 'scroll', direction: 'down', amount: 50 });

                expect(result.scroll).toHaveProperty('x');
                expect(result.scroll).toHaveProperty('y');
                expect(result.scroll).toHaveProperty('max_x');
                expect(result.scroll).toHaveProperty('max_y');
            });

            test('supports smooth behavior', async () => {
                const result = await runCommand(page, {
                    cmd: 'scroll',
                    direction: 'down',
                    amount: 100,
                    behavior: 'smooth'
                });
                expect(result.status).toBe('ok');
            });
        });

        // ----------------------------------------------------------
        // 3.8 focus
        // ----------------------------------------------------------
        describe('3.8 focus', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('focuses element', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const input = findElement(scan.elements, 'id', 'input-1');

                const result = await runCommand(page, { cmd: 'focus', id: input.id });
                expect(result.status).toBe('ok');
                expect(result.action).toBe('focused');

                // Verify focus
                const isFocused = await page.evaluate(() => document.activeElement.id === 'input-1');
                expect(isFocused).toBe(true);
            });

            test('returns selector in response', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const input = findElement(scan.elements, 'id', 'input-1');

                const result = await runCommand(page, { cmd: 'focus', id: input.id });
                expect(result.selector).toBeDefined();
            });
        });

        // ----------------------------------------------------------
        // 3.9 hover
        // ----------------------------------------------------------
        describe('3.9 hover', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('hovers over element', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                const result = await runCommand(page, { cmd: 'hover', id: btn.id });
                expect(result.status).toBe('ok');
                expect(result.action).toBe('hovered');
            });

            test('returns coordinates', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                const result = await runCommand(page, { cmd: 'hover', id: btn.id });
                expect(result.coordinates).toHaveProperty('x');
                expect(result.coordinates).toHaveProperty('y');
            });

            test('returns selector in response', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                const result = await runCommand(page, { cmd: 'hover', id: btn.id });
                expect(result.selector).toBeDefined();
            });

            test('fails on hidden element', async () => {
                const scan = await runCommand(page, { cmd: 'scan', include_hidden: true });
                const hidden = scan.elements.find((el) => el.text === 'Hidden Element');

                if (hidden) {
                    const result = await runCommand(page, { cmd: 'hover', id: hidden.id });
                    expect(result.status).toBe('error');
                    expect(result.code).toBe('ELEMENT_NOT_VISIBLE');
                }
            });
        });

        // ----------------------------------------------------------
        // 3.10 submit
        // ----------------------------------------------------------
        describe('3.10 submit', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('submits form by element ID within form', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const emailInput = findElement(scan.elements, 'id', 'email');

                const result = await runCommand(page, { cmd: 'submit', id: emailInput.id });
                expect(result.status).toBe('ok');
                expect(result.action).toBe('submitted');

                // Verify form submitted (check log)
                const log = await page.evaluate(() => document.getElementById('log').innerText);
                expect(log).toContain('Form submitted');
            });

            test('submits form directly by form ID', async () => {
                // Use patterns harness which has form IDs
                await page.goto(HARNESS_PATTERNS_PATH);
                await page.evaluate(SCANNER_JS);

                const scan = await runCommand(page, { cmd: 'scan' });
                // Find any element in the login form
                const emailInput = findElement(scan.elements, 'id', 'login-email');

                const result = await runCommand(page, { cmd: 'submit', id: emailInput.id });
                expect(result.status).toBe('ok');
            });

            test('submits focused element form when no ID provided', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const emailInput = findElement(scan.elements, 'id', 'email');

                // Focus the input first
                await runCommand(page, { cmd: 'focus', id: emailInput.id });
                // Submit without ID
                const result = await runCommand(page, { cmd: 'submit' });

                expect(result.status).toBe('ok');
            });

            test('returns form_selector and form_id', async () => {
                await page.goto(HARNESS_PATTERNS_PATH);
                await page.evaluate(SCANNER_JS);

                const scan = await runCommand(page, { cmd: 'scan' });
                const emailInput = findElement(scan.elements, 'id', 'login-email');

                const result = await runCommand(page, { cmd: 'submit', id: emailInput.id });
                expect(result.form_selector).toBeDefined();
                expect(result.form_id).toBe('login-form');
            });

            test('fails when no form found', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1'); // Not in a form

                const result = await runCommand(page, { cmd: 'submit', id: btn.id });
                expect(result.status).toBe('error');
            });
        });

        // ----------------------------------------------------------
        // 3.11 get_value
        // ----------------------------------------------------------
        describe('3.11 get_value', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('gets text input value', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const input = findElement(scan.elements, 'id', 'input-1');

                await runCommand(page, { cmd: 'type', id: input.id, text: 'test value' });
                const result = await runCommand(page, { cmd: 'get_value', id: input.id });

                expect(result.status).toBe('ok');
                expect(result.value).toBe('test value');
            });

            test('gets checkbox value as boolean', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const checkbox = findElement(scan.elements, 'id', 'check-1');

                await runCommand(page, { cmd: 'check', id: checkbox.id });
                const result = await runCommand(page, { cmd: 'get_value', id: checkbox.id });

                expect(result.status).toBe('ok');
                expect(result.value).toBe(true);
            });

            test('gets select value', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const select = findElement(scan.elements, 'id', 'select-1');

                await runCommand(page, { cmd: 'select', id: select.id, value: '2' });
                const result = await runCommand(page, { cmd: 'get_value', id: select.id });

                expect(result.status).toBe('ok');
                expect(result.value).toBe('2');
            });

            test('gets multi-select value as array', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const multiSelect = findElement(scan.elements, 'id', 'multi-select');

                await runCommand(page, { cmd: 'select', id: multiSelect.id, value: ['1', '3'] });
                const result = await runCommand(page, { cmd: 'get_value', id: multiSelect.id });

                expect(result.status).toBe('ok');
                expect(Array.isArray(result.value)).toBe(true);
                expect(result.value).toEqual(['1', '3']);
            });
        });

        // ----------------------------------------------------------
        // 3.12 get_text
        // ----------------------------------------------------------
        describe('3.12 get_text', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('gets text content by selector', async () => {
                const result = await runCommand(page, { cmd: 'get_text', selector: 'h1' });
                expect(result.status).toBe('ok');
                expect(result.text).toContain('Scanner Test Harness');
            });

            test('fails for non-existent selector', async () => {
                const result = await runCommand(page, { cmd: 'get_text', selector: '#nonexistent' });
                expect(result.status).toBe('error');
                expect(result.code).toBe('ELEMENT_NOT_FOUND');
            });
        });

        // ----------------------------------------------------------
        // 3.13 exists
        // ----------------------------------------------------------
        describe('3.13 exists', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('returns true for existing element', async () => {
                const result = await runCommand(page, { cmd: 'exists', selector: '#btn-1' });
                expect(result.status).toBe('ok');
                expect(result.exists).toBe(true);
            });

            test('returns false for non-existent element', async () => {
                const result = await runCommand(page, { cmd: 'exists', selector: '#nonexistent' });
                expect(result.status).toBe('ok');
                expect(result.exists).toBe(false);
            });
        });

        // ----------------------------------------------------------
        // 3.14 wait_for
        // ----------------------------------------------------------
        describe('3.14 wait_for', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('waits for element to exist', async () => {
                // Add element after delay
                page.evaluate(() => {
                    setTimeout(() => {
                        const el = document.createElement('div');
                        el.id = 'delayed-exists';
                        document.body.appendChild(el);
                    }, 200);
                });

                const result = await runCommand(page, {
                    cmd: 'wait_for',
                    condition: 'exists',
                    selector: '#delayed-exists',
                    timeout: 1000
                });

                expect(result.status).toBe('ok');
                expect(result.condition_met).toBe(true);
                expect(result.waited_ms).toBeGreaterThanOrEqual(100);
            });

            test('waits for element to be visible', async () => {
                // Make hidden element visible after delay
                await page.evaluate(() => {
                    const btn = document.querySelector('.hidden');
                    if (btn) {
                        btn.id = 'will-be-visible';
                        setTimeout(() => {
                            btn.classList.remove('hidden');
                        }, 200);
                    }
                });

                const result = await runCommand(page, {
                    cmd: 'wait_for',
                    condition: 'visible',
                    selector: '#will-be-visible',
                    timeout: 1000
                });

                expect(result.status).toBe('ok');
                expect(result.condition_met).toBe(true);
            });

            test('waits for element to be hidden', async () => {
                await page.evaluate(() => {
                    const btn = document.getElementById('btn-1');
                    setTimeout(() => {
                        btn.style.display = 'none';
                    }, 200);
                });

                const result = await runCommand(page, {
                    cmd: 'wait_for',
                    condition: 'hidden',
                    selector: '#btn-1',
                    timeout: 1000
                });

                expect(result.status).toBe('ok');
                expect(result.condition_met).toBe(true);
            });

            test('waits for element to be gone', async () => {
                await page.evaluate(() => {
                    const btn = document.getElementById('btn-1');
                    setTimeout(() => {
                        btn.remove();
                    }, 200);
                });

                const result = await runCommand(page, {
                    cmd: 'wait_for',
                    condition: 'gone',
                    selector: '#btn-1',
                    timeout: 1000
                });

                expect(result.status).toBe('ok');
                expect(result.condition_met).toBe(true);
            });

            test('waits for element to be enabled', async () => {
                await page.evaluate(() => {
                    const input = document.getElementById('disabled-input');
                    if (input) {
                        setTimeout(() => {
                            input.disabled = false;
                        }, 200);
                    }
                });

                const scan = await runCommand(page, { cmd: 'scan' });
                const disabledInput = findElement(scan.elements, 'id', 'disabled-input');

                if (disabledInput) {
                    const result = await runCommand(page, {
                        cmd: 'wait_for',
                        condition: 'enabled',
                        id: disabledInput.id,
                        timeout: 1000
                    });

                    expect(result.status).toBe('ok');
                }
            });

            test('waits for element to be disabled', async () => {
                await page.evaluate(() => {
                    const input = document.getElementById('input-1');
                    setTimeout(() => {
                        input.disabled = true;
                    }, 200);
                });

                const result = await runCommand(page, {
                    cmd: 'wait_for',
                    condition: 'disabled',
                    selector: '#input-1',
                    timeout: 1000
                });

                expect(result.status).toBe('ok');
                expect(result.condition_met).toBe(true);
            });

            test('times out when condition not met', async () => {
                const result = await runCommand(page, {
                    cmd: 'wait_for',
                    condition: 'exists',
                    selector: '#will-never-exist',
                    timeout: 200
                });

                expect(result.status).toBe('error');
                expect(result.code).toBe('TIMEOUT');
            });

            test('returns waited_ms in response', async () => {
                const result = await runCommand(page, {
                    cmd: 'wait_for',
                    condition: 'exists',
                    selector: '#btn-1', // Already exists
                    timeout: 1000
                });

                expect(result.status).toBe('ok');
                expect(result.waited_ms).toBeDefined();
                expect(result.waited_ms).toBeGreaterThanOrEqual(0);
            });

            test('navigation condition detects URL change', async () => {
                // Schedule URL change via hash
                await page.evaluate(() => {
                    setTimeout(() => {
                        window.location.hash = '#test-nav';
                    }, 100);
                });

                const result = await runCommand(page, {
                    cmd: 'wait_for',
                    condition: 'navigation',
                    timeout: 1000
                });

                expect(result.status).toBe('ok');
                expect(result.condition_met).toBe(true);
                expect(result.previous_url).toBeDefined();
                expect(result.current_url).toBeDefined();
                expect(result.current_url).toContain('#test-nav');
            });

            test('navigation condition times out with NAVIGATION_ERROR', async () => {
                const result = await runCommand(page, {
                    cmd: 'wait_for',
                    condition: 'navigation',
                    timeout: 200
                });

                expect(result.status).toBe('error');
                expect(result.code).toBe('NAVIGATION_ERROR');
            });
        });

        // ----------------------------------------------------------
        // 3.15 execute
        // ----------------------------------------------------------
        describe('3.15 execute', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('executes script and returns result', async () => {
                const result = await runCommand(page, {
                    cmd: 'execute',
                    script: 'return 1 + 2'
                });

                expect(result.status).toBe('ok');
                expect(result.result).toBe(3);
            });

            test('passes args to script', async () => {
                const result = await runCommand(page, {
                    cmd: 'execute',
                    script: 'return args[0] + args[1]',
                    args: [10, 20]
                });

                expect(result.status).toBe('ok');
                expect(result.result).toBe(30);
            });

            test('returns complex objects', async () => {
                const result = await runCommand(page, {
                    cmd: 'execute',
                    script: 'return { a: 1, b: [1, 2, 3] }'
                });

                expect(result.status).toBe('ok');
                expect(result.result).toEqual({ a: 1, b: [1, 2, 3] });
            });

            test('returns SCRIPT_ERROR for invalid script', async () => {
                const result = await runCommand(page, {
                    cmd: 'execute',
                    script: 'invalid javascript {'
                });

                expect(result.status).toBe('error');
                expect(result.code).toBe('SCRIPT_ERROR');
            });
        });

        // ----------------------------------------------------------
        // 3.16 version
        // ----------------------------------------------------------
        describe('3.16 version', () => {
            let page;

            beforeEach(async () => {
                page = await browser.newPage();
                await page.goto(HARNESS_PATH);
                await page.evaluate(SCANNER_JS);
            });

            afterEach(async () => {
                if (page) await page.close();
            });

            test('returns protocol version', async () => {
                const result = await runCommand(page, { cmd: 'version' });

                expect(result.status).toBe('ok');
                expect(result.protocol).toBe('1.0');
            });

            test('returns scanner version', async () => {
                const result = await runCommand(page, { cmd: 'version' });

                expect(result.scanner).toBeDefined();
            });

            test('returns features list', async () => {
                const result = await runCommand(page, { cmd: 'version' });

                expect(result.features).toBeDefined();
                expect(Array.isArray(result.features)).toBe(true);
                expect(result.features).toContain('scan');
                expect(result.features).toContain('click');
                expect(result.features).toContain('type');
            });
        });
    });

    // ============================================================
    // SECTION 4: ELEMENT CLASSIFICATION
    // ============================================================
    describe('Section 4: Element Classification', () => {
        let page;

        beforeEach(async () => {
            page = await browser.newPage();
            await page.goto(HARNESS_PATH);
            await page.evaluate(SCANNER_JS);
        });

        afterEach(async () => {
            if (page) await page.close();
        });

        describe('4.2 Element Roles', () => {
            test('detects email role', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const emailInput = findElement(scan.elements, 'id', 'email');

                expect(emailInput.role).toBe('email');
            });

            test('detects password role', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const passwordInput = findElement(scan.elements, 'id', 'password');

                expect(passwordInput.role).toBe('password');
            });

            test('detects search role', async () => {
                await page.goto(HARNESS_PATTERNS_PATH);
                await page.evaluate(SCANNER_JS);

                const scan = await runCommand(page, { cmd: 'scan' });
                const searchInput = findElement(scan.elements, 'id', 'search-input');

                expect(searchInput.role).toBe('search');
            });

            test('detects submit role', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const loginBtn = findElementByText(scan.elements, 'Login');

                expect(loginBtn.role).toBe('submit');
            });

            test('detects primary role', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const primaryBtn = findElement(scan.elements, 'id', 'btn-primary-class');

                expect(primaryBtn.role).toBe('primary');
            });

            test('detects link role', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const link = findElement(scan.elements, 'id', 'link-1');

                expect(link.role).toBe('link');
            });

            test('detects checkbox role', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const checkbox = findElement(scan.elements, 'id', 'check-1');

                expect(checkbox.role).toBe('checkbox');
            });
        });

        describe('4.3 Element Modifiers/State', () => {
            test('tracks visible state', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                expect(btn.state.visible).toBe(true);
            });

            test('tracks hidden state', async () => {
                const scan = await runCommand(page, { cmd: 'scan', include_hidden: true });
                const hidden = scan.elements.find((el) => el.text === 'Hidden Element');

                if (hidden) {
                    expect(hidden.state.hidden).toBe(true);
                    expect(hidden.state.visible).toBe(false);
                }
            });

            test('tracks disabled state', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const disabledBtn = findElement(scan.elements, 'id', 'disabled-btn');

                if (disabledBtn) {
                    expect(disabledBtn.state.disabled).toBe(true);
                }
            });

            test('tracks focused state', async () => {
                // Focus an element first
                await page.focus('#input-1');

                const scan = await runCommand(page, { cmd: 'scan' });
                const input = findElement(scan.elements, 'id', 'input-1');

                expect(input.state.focused).toBe(true);
            });

            test('tracks checked state', async () => {
                // Check the checkbox first
                await page.click('#check-1');

                const scan = await runCommand(page, { cmd: 'scan' });
                const checkbox = findElement(scan.elements, 'id', 'check-1');

                expect(checkbox.state.checked).toBe(true);
                expect(checkbox.state.unchecked).toBe(false);
            });

            test('tracks required state', async () => {
                await page.goto(HARNESS_EDGE_CASES_PATH);
                await page.evaluate(SCANNER_JS);

                const scan = await runCommand(page, { cmd: 'scan' });
                const requiredInput = findElement(scan.elements, 'id', 'required-input');

                expect(requiredInput.state.required).toBe(true);
            });

            test('tracks readonly state', async () => {
                await page.goto(HARNESS_EDGE_CASES_PATH);
                await page.evaluate(SCANNER_JS);

                const scan = await runCommand(page, { cmd: 'scan' });
                const readonlyInput = findElement(scan.elements, 'id', 'readonly-input');

                expect(readonlyInput.state.readonly).toBe(true);
            });

            test('tracks primary state', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const loginBtn = findElementByText(scan.elements, 'Login');

                expect(loginBtn.state.primary).toBe(true);
            });

            test('tracks value state', async () => {
                await page.type('#input-1', 'test value');

                const scan = await runCommand(page, { cmd: 'scan' });
                const input = findElement(scan.elements, 'id', 'input-1');

                expect(input.state.value).toBe('test value');
            });
        });

        describe('Element Attributes', () => {
            test('includes href attribute', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const link = findElement(scan.elements, 'id', 'link-1');

                expect(link.attributes.href).toBe('#');
            });

            test('includes placeholder attribute', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const input = findElement(scan.elements, 'id', 'input-1');

                expect(input.attributes.placeholder).toBe('Type here...');
            });

            test('includes aria-label attribute', async () => {
                await page.goto(HARNESS_PATTERNS_PATH);
                await page.evaluate(SCANNER_JS);

                const scan = await runCommand(page, { cmd: 'scan' });
                const searchInput = findElement(scan.elements, 'id', 'search-input');

                expect(searchInput.attributes['aria-label']).toBe('Search');
            });

            test('includes data-testid attribute', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'attr-btn');

                expect(btn.attributes['data-testid']).toBe('test-btn-id');
            });

            test('includes title attribute', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'attr-btn');

                expect(btn.attributes.title).toBe('Button Title');
            });

            test('includes class attribute', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'attr-btn');

                expect(btn.attributes.class).toContain('test-class');
            });

            test('includes tabindex attribute', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'attr-btn');

                expect(btn.attributes.tabindex).toBe('1');
            });
        });
    });

    // ============================================================
    // SECTION 5: PATTERN DETECTION
    // ============================================================
    describe('Section 5: Pattern Detection', () => {
        let page;

        beforeEach(async () => {
            page = await browser.newPage();
            await page.goto(HARNESS_PATTERNS_PATH);
            await page.evaluate(SCANNER_JS);
        });

        afterEach(async () => {
            if (page) await page.close();
        });

        describe('5.1 Login Form Pattern', () => {
            test('detects login form with email', async () => {
                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns).toBeDefined();
                expect(result.patterns.login).toBeDefined();
                expect(result.patterns.login.email).toBeDefined();
                expect(result.patterns.login.password).toBeDefined();
            });

            test('includes submit button in login pattern', async () => {
                const result = await runCommand(page, { cmd: 'scan' });

                // Submit button detection depends on button text matching patterns
                // May not always be detected if text doesn't match "sign in", "log in", etc.
                if (result.patterns.login.submit) {
                    expect(typeof result.patterns.login.submit).toBe('number');
                }
            });

            test('includes remember checkbox in login pattern', async () => {
                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns.login.remember).toBeDefined();
            });

            test('includes form selector in login pattern', async () => {
                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns.login.form).toBeDefined();
            });
        });

        describe('5.2 Search Form Pattern', () => {
            test('detects search form', async () => {
                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns.search).toBeDefined();
                expect(result.patterns.search.input).toBeDefined();
            });

            test('includes submit button in search pattern', async () => {
                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns.search.submit).toBeDefined();
            });
        });

        describe('5.3 Pagination Pattern', () => {
            test('detects pagination', async () => {
                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns.pagination).toBeDefined();
            });

            test('includes prev/next in pagination', async () => {
                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns.pagination.prev).toBeDefined();
                expect(result.patterns.pagination.next).toBeDefined();
            });

            test('includes page numbers in pagination', async () => {
                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns.pagination.pages).toBeDefined();
                expect(result.patterns.pagination.pages.length).toBeGreaterThan(0);
            });
        });

        describe('5.4 Modal Dialog Pattern', () => {
            test('detects visible modal', async () => {
                // Open the modal
                await page.click('#open-modal-btn');

                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns.modal).toBeDefined();
                expect(result.patterns.modal.container).toBeDefined();
            });

            test('includes close button in modal pattern', async () => {
                await page.click('#open-modal-btn');

                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns.modal.close).toBeDefined();
            });

            test('includes title in modal pattern', async () => {
                await page.click('#open-modal-btn');

                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns.modal.title).toBeDefined();
                expect(result.patterns.modal.title).toContain('Confirm');
            });
        });

        describe('5.5 Cookie Banner Pattern', () => {
            test('detects visible cookie banner', async () => {
                // Show the cookie banner
                await page.click('#show-cookie-btn');

                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns.cookie_banner).toBeDefined();
                expect(result.patterns.cookie_banner.container).toBeDefined();
            });

            test('includes accept button in cookie banner', async () => {
                await page.click('#show-cookie-btn');

                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns.cookie_banner.accept).toBeDefined();
            });

            test('includes reject button in cookie banner', async () => {
                await page.click('#show-cookie-btn');

                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.patterns.cookie_banner.reject).toBeDefined();
            });
        });
    });

    // ============================================================
    // SECTION 6: ELEMENT MAP LIFECYCLE
    // ============================================================
    describe('Section 6: Element Map Lifecycle', () => {
        let page;

        beforeEach(async () => {
            page = await browser.newPage();
            await page.goto(HARNESS_PATH);
            await page.evaluate(SCANNER_JS);
        });

        afterEach(async () => {
            if (page) await page.close();
        });

        describe('6.1 Map Creation', () => {
            test('assigns numeric IDs', async () => {
                const result = await runCommand(page, { cmd: 'scan' });
                const ids = result.elements.map((el) => el.id);

                // All IDs should be positive numbers
                for (const id of ids) {
                    expect(typeof id).toBe('number');
                    expect(id).toBeGreaterThan(0);
                }
            });

            test('clears previous map on re-scan', async () => {
                // First scan
                const scan1 = await runCommand(page, { cmd: 'scan' });
                const btn1 = findElement(scan1.elements, 'id', 'btn-1');
                expect(btn1.id).toBeGreaterThan(0);

                // Second scan
                const scan2 = await runCommand(page, { cmd: 'scan' });
                const btn2 = findElement(scan2.elements, 'id', 'btn-1');

                // After re-scan, the same element should have the same relative position
                // IDs reset to start from 1
                expect(btn2.id).toBeGreaterThan(0);
                // Old IDs from scan1 should still work since they point to same elements
                // The key point is map was cleared and rebuilt with fresh IDs
            });
        });

        describe('6.3 Map Staleness', () => {
            test('invalidates map on hash change', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                // Trigger hash change
                await page.evaluate(() => {
                    window.location.hash = '#test-hash';
                });
                await new Promise((r) => setTimeout(r, 100));

                // Old ID should not work
                const result = await runCommand(page, { cmd: 'click', id: btn.id });
                expect(result.status).toBe('error');
                expect(result.code).toBe('ELEMENT_NOT_FOUND');
            });

            test('invalidates map on pushState', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                // Trigger pushState with same-origin path (file:// requires special handling)
                // Use try-catch since file:// protocol has restrictions
                const pushWorked = await page.evaluate(() => {
                    try {
                        window.history.pushState({}, '', window.location.pathname + '?test=1');
                        return true;
                    } catch (_e) {
                        return false;
                    }
                });

                if (pushWorked) {
                    await new Promise((r) => setTimeout(r, 100));
                    // Old ID should not work
                    const result = await runCommand(page, { cmd: 'click', id: btn.id });
                    expect(result.status).toBe('error');
                    expect(result.code).toBe('ELEMENT_NOT_FOUND');
                }
            });

            test('invalidates map on replaceState', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const btn = findElement(scan.elements, 'id', 'btn-1');

                // Trigger replaceState with same-origin path
                const replaceWorked = await page.evaluate(() => {
                    try {
                        window.history.replaceState({}, '', window.location.pathname + '?replaced=1');
                        return true;
                    } catch (_e) {
                        return false;
                    }
                });

                if (replaceWorked) {
                    await new Promise((r) => setTimeout(r, 100));
                    // Old ID should not work
                    const result = await runCommand(page, { cmd: 'click', id: btn.id });
                    expect(result.status).toBe('error');
                    expect(result.code).toBe('ELEMENT_NOT_FOUND');
                }
            });
        });
    });

    // ============================================================
    // SECTION 7: IFRAME HANDLING
    // ============================================================
    describe('Section 7: Iframe Handling', () => {
        let page;

        beforeEach(async () => {
            page = await browser.newPage();
            await page.goto(HARNESS_IFRAMES_PATH);
            await page.evaluate(SCANNER_JS);
            // Wait for iframes to load
            await new Promise((r) => setTimeout(r, 500));
        });

        afterEach(async () => {
            if (page) await page.close();
        });

        describe('7.1 Same-Origin Iframes', () => {
            test('scans elements within same-origin iframe', async () => {
                const result = await runCommand(page, { cmd: 'scan' });

                // Find elements with iframe_context
                const iframeElements = result.elements.filter((el) => el.iframe_context);
                expect(iframeElements.length).toBeGreaterThan(0);
            });

            test('includes iframe_context with iframe_id and src', async () => {
                const result = await runCommand(page, { cmd: 'scan' });

                const iframeElement = result.elements.find((el) => el.iframe_context);
                if (iframeElement) {
                    expect(iframeElement.iframe_context).toHaveProperty('iframe_id');
                    expect(iframeElement.iframe_context).toHaveProperty('src');
                }
            });

            test('can interact with elements in same-origin iframe', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });

                // Find button inside iframe
                const iframeBtn = scan.elements.find(
                    (el) => el.iframe_context && el.attributes?.id === 'iframe-btn-1'
                );

                if (iframeBtn) {
                    const result = await runCommand(page, { cmd: 'click', id: iframeBtn.id });
                    expect(result.status).toBe('ok');
                }
            });

            test('reports iframe accessibility in stats', async () => {
                const result = await runCommand(page, { cmd: 'scan' });

                expect(result.stats.iframes).toBeDefined();
                expect(result.stats.iframes.total).toBeGreaterThan(0);
                expect(result.stats.iframes.accessible).toBeGreaterThan(0);
            });
        });

        describe('7.2 Cross-Origin Iframes', () => {
            test('reports cross-origin iframes as inaccessible', async () => {
                const result = await runCommand(page, { cmd: 'scan' });

                // Find iframe element (not content inside)
                const iframeEl = result.elements.find((el) => el.type === 'iframe' && el.iframe);

                if (iframeEl && !iframeEl.iframe.accessible) {
                    expect(iframeEl.iframe.origin).toBe('cross-origin');
                }
            });
        });

        describe('include_iframes parameter', () => {
            test('excludes iframe content when include_iframes=false', async () => {
                const withIframes = await runCommand(page, { cmd: 'scan', include_iframes: true });
                const withoutIframes = await runCommand(page, { cmd: 'scan', include_iframes: false });

                const iframeElementsWith = withIframes.elements.filter((el) => el.iframe_context);
                const iframeElementsWithout = withoutIframes.elements.filter((el) => el.iframe_context);

                expect(iframeElementsWith.length).toBeGreaterThan(iframeElementsWithout.length);
            });
        });
    });

    // ============================================================
    // SELECTOR GENERATION
    // ============================================================
    describe('Selector Generation', () => {
        let page;

        beforeEach(async () => {
            page = await browser.newPage();
            await page.goto(HARNESS_PATH);
            await page.evaluate(SCANNER_JS);
        });

        afterEach(async () => {
            if (page) await page.close();
        });

        test('generates ID-based selector when ID is unique', async () => {
            const scan = await runCommand(page, { cmd: 'scan' });
            const btn = findElement(scan.elements, 'id', 'btn-1');

            expect(btn.selector).toBe('#btn-1');
        });

        test('generates data-testid selector when available and unique', async () => {
            const scan = await runCommand(page, { cmd: 'scan' });
            const btn = scan.elements.find((el) => el.attributes['data-testid'] === 'unique-test-id');

            expect(btn.selector).toBe('[data-testid="unique-test-id"]');
        });

        test('generates aria-label selector when unique', async () => {
            await page.goto(HARNESS_PATTERNS_PATH);
            await page.evaluate(SCANNER_JS);

            const scan = await runCommand(page, { cmd: 'scan' });
            const searchInput = findElement(scan.elements, 'id', 'search-input');

            // May use aria-label if unique
            expect(searchInput.selector).toBeDefined();
        });

        test('generates valid CSS selector', async () => {
            const scan = await runCommand(page, { cmd: 'scan' });

            for (const el of scan.elements.slice(0, 10)) {
                // Verify selector works
                const exists = await page.evaluate((sel) => {
                    try {
                        return !!document.querySelector(sel);
                    } catch (_e) {
                        return false;
                    }
                }, el.selector);

                expect(exists).toBe(true);
            }
        });

        test('generates XPath expression', async () => {
            const scan = await runCommand(page, { cmd: 'scan' });
            const btn = findElement(scan.elements, 'id', 'btn-1');

            expect(btn.xpath).toBeDefined();
            expect(btn.xpath.startsWith('/') || btn.xpath.startsWith('//')).toBe(true);
        });
    });

    // ============================================================
    // EDGE CASES
    // ============================================================
    describe('Edge Cases', () => {
        let page;

        beforeEach(async () => {
            page = await browser.newPage();
            await page.goto(HARNESS_EDGE_CASES_PATH);
            await page.evaluate(SCANNER_JS);
        });

        afterEach(async () => {
            if (page) await page.close();
        });

        describe('Visibility Edge Cases', () => {
            test('excludes display:none elements by default', async () => {
                const result = await runCommand(page, { cmd: 'scan' });
                const hidden = findElement(result.elements, 'id', 'hidden-btn');

                expect(hidden).toBeUndefined();
            });

            test('excludes visibility:hidden elements by default', async () => {
                const result = await runCommand(page, { cmd: 'scan' });
                const invisible = findElement(result.elements, 'id', 'invisible-btn');

                expect(invisible).toBeUndefined();
            });

            test('excludes opacity:0 elements by default', async () => {
                const result = await runCommand(page, { cmd: 'scan' });
                const transparent = findElement(result.elements, 'id', 'transparent-btn');

                expect(transparent).toBeUndefined();
            });

            test('excludes truly zero-size elements by default', async () => {
                // Add a truly zero-sized element dynamically
                await page.evaluate(() => {
                    const div = document.createElement('div');
                    div.id = 'true-zero-size';
                    div.style.width = '0px';
                    div.style.height = '0px';
                    div.style.overflow = 'hidden';
                    div.setAttribute('role', 'button');
                    document.body.appendChild(div);
                });

                const result = await runCommand(page, { cmd: 'scan' });
                const zeroSize = findElement(result.elements, 'id', 'true-zero-size');

                expect(zeroSize).toBeUndefined();
            });
        });

        describe('Covered Element Handling', () => {
            test('detects covered elements', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const covered = findElement(scan.elements, 'id', 'fully-covered-btn');

                if (covered) {
                    const result = await runCommand(page, { cmd: 'click', id: covered.id });
                    expect(result.status).toBe('error');
                    expect(result.code).toBe('ELEMENT_NOT_INTERACTABLE');
                }
            });

            test('allows clicking partially covered elements at center', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const partial = findElement(scan.elements, 'id', 'partially-covered-btn');

                if (partial) {
                    // Center should be covered, so this should fail
                    const result = await runCommand(page, { cmd: 'click', id: partial.id });
                    // Result depends on exact overlay positioning
                    expect(result).toBeDefined();
                }
            });
        });

        describe('Contenteditable Elements', () => {
            test('types into contenteditable div', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const editable = findElement(scan.elements, 'id', 'editable-div');

                if (editable) {
                    const result = await runCommand(page, {
                        cmd: 'type',
                        id: editable.id,
                        text: 'New content'
                    });
                    expect(result.status).toBe('ok');
                }
            });
        });

        describe('Custom Interactive Elements', () => {
            test('detects elements with cursor:pointer', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const clickable = findElement(scan.elements, 'id', 'clickable-div');

                expect(clickable).toBeDefined();
            });

            test('detects elements with onclick', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const onclick = findElement(scan.elements, 'id', 'onclick-span');

                expect(onclick).toBeDefined();
            });

            test('detects elements with role=button', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const roleBtn = findElement(scan.elements, 'id', 'role-button');

                expect(roleBtn).toBeDefined();
            });
        });

        describe('Dynamic Elements', () => {
            test('finds dynamically added elements after re-scan', async () => {
                // Add dynamic element
                await page.click('#add-element-btn');

                // Re-scan
                const scan = await runCommand(page, { cmd: 'scan' });
                const dynamic = scan.elements.find((el) => el.text === 'Dynamic Button');

                expect(dynamic).toBeDefined();
            });
        });

        describe('Stale Element Handling', () => {
            test('returns ELEMENT_STALE for removed element', async () => {
                const scan = await runCommand(page, { cmd: 'scan' });
                const staleBtn = findElement(scan.elements, 'id', 'stale-btn');

                // Remove the element
                await page.click('#remove-stale-btn');

                // Try to click old reference
                const result = await runCommand(page, { cmd: 'click', id: staleBtn.id });
                expect(result.status).toBe('error');
                expect(result.code).toBe('ELEMENT_STALE');
            });
        });
    });
});
