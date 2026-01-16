/**
 * Mutation and Persistence Tests for Lemmascope Scanner
 */

const puppeteer = require('puppeteer');
const fs = require('fs');
const path = require('path');

const SCANNER_JS = fs.readFileSync(path.resolve(__dirname, '../src/scanner.js'), 'utf8');

const runCommand = async (page, cmd) => {
    return page.evaluate(async (command) => {
        return await window.Lemmascope.process(command);
    }, cmd);
};

describe('Scanner Mutation and Persistence', () => {
    let browser;
    let page;

    beforeAll(async () => {
        browser = await puppeteer.launch({
            headless: 'new',
            args: ['--no-sandbox', '--disable-setuid-sandbox']
        });
    });

    afterAll(async () => {
        if (browser) await browser.close();
    });

    beforeEach(async () => {
        page = await browser.newPage();
        await page.setContent(`
            <html>
                <body>
                    <div id="container">
                        <button id="btn-1">Original Button</button>
                        <input id="input-1" value="Initial Value">
                    </div>
                    <div id="log"></div>
                </body>
            </html>
        `);
        await page.evaluate(SCANNER_JS);
    });

    afterEach(async () => {
        if (page) await page.close();
    });

    test('IDs persist across scans without navigation', async () => {
        // Initial scan
        const scan1 = await runCommand(page, { cmd: 'scan' });
        const btn1 = scan1.elements.find((e) => e.attributes.id === 'btn-1');
        const id1 = btn1.id;

        // Second scan
        const scan2 = await runCommand(page, { cmd: 'scan' });
        const btn2 = scan2.elements.find((e) => e.attributes.id === 'btn-1');
        const id2 = btn2.id;

        expect(id1).toBe(id2);
        expect(id1).toBeGreaterThan(0);
    });

    test('detects appeared elements', async () => {
        // First scan to establish baseline
        await runCommand(page, { cmd: 'scan', monitor_changes: true });

        // Add an element
        await page.evaluate(() => {
            const newBtn = document.createElement('button');
            newBtn.id = 'btn-new';
            newBtn.innerText = 'New Button';
            document.getElementById('container').appendChild(newBtn);
        });

        // Second scan with monitoring
        const result = await runCommand(page, { cmd: 'scan', monitor_changes: true });

        expect(result.changes).toBeDefined();
        const appeared = result.changes.find((c) => c.change_type === 'appeared');
        expect(appeared).toBeDefined();

        const newBtnData = result.elements.find((e) => e.id === appeared.id);
        expect(newBtnData.attributes.id).toBe('btn-new');
    });

    test('detects disappeared elements', async () => {
        // Baseline
        const scan1 = await runCommand(page, { cmd: 'scan', monitor_changes: true });
        const btn1 = scan1.elements.find((e) => e.attributes.id === 'btn-1');

        // Remove element
        await page.evaluate(() => {
            document.getElementById('btn-1').remove();
        });

        const result = await runCommand(page, { cmd: 'scan', monitor_changes: true });

        expect(result.changes).toBeDefined();
        const disappeared = result.changes.find((c) => c.change_type === 'disappeared' && c.id === btn1.id);
        expect(disappeared).toBeDefined();
    });

    test('detects text changes', async () => {
        // Baseline
        const scan1 = await runCommand(page, { cmd: 'scan', monitor_changes: true });
        const btn1 = scan1.elements.find((e) => e.attributes.id === 'btn-1');

        // Change text
        await page.evaluate(() => {
            document.getElementById('btn-1').innerText = 'Updated Button';
        });

        const result = await runCommand(page, { cmd: 'scan', monitor_changes: true });

        expect(result.changes).toBeDefined();
        const textChange = result.changes.find((c) => c.change_type === 'text_changed' && c.id === btn1.id);
        expect(textChange).toBeDefined();
        expect(textChange.old_value).toBe('Original Button');
        expect(textChange.new_value).toBe('Updated Button');
    });

    test('detects state changes (value/focused)', async () => {
        // Baseline
        const scan1 = await runCommand(page, { cmd: 'scan', monitor_changes: true });
        const input1 = scan1.elements.find((e) => e.attributes.id === 'input-1');

        // Change value and focus
        await page.evaluate(() => {
            const input = document.getElementById('input-1');
            input.value = 'New Value';
            input.focus();
        });

        const result = await runCommand(page, { cmd: 'scan', monitor_changes: true });

        expect(result.changes).toBeDefined();

        // Note: property state change is reported as state_changed: key:value
        const focusedChange = result.changes.find(
            (c) => c.change_type === 'state_changed' && c.id === input1.id && c.new_value === 'focused:true'
        );
        expect(focusedChange).toBeDefined();
    });
});
