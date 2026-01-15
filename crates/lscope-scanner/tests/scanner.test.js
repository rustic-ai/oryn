const puppeteer = require('puppeteer');
const fs = require('fs');
const path = require('path');

// Load scanner source
const SCANNER_JS = fs.readFileSync(path.resolve(__dirname, '../src/scanner.js'), 'utf8');
const HARNESS_PATH = 'file://' + path.resolve(__dirname, 'harness.html');

describe('Universal Scanner', () => {
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
        await page.goto(HARNESS_PATH);
        await page.evaluate(SCANNER_JS);
    });

    afterEach(async () => {
        if (page) await page.close();
    });

    test('verifies harness loads', async () => {
        const title = await page.title();
        expect(title).toBe('Scanner Harness');
    });

    test('scans the page and finds elements', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });

        expect(result.ok).toBe(true);
        expect(result.elements.length).toBeGreaterThan(8);

        // Check for specific known elements
        const emailInput = result.elements.find((el) => el.attributes.id === 'email');
        expect(emailInput).toBeDefined();
        expect(emailInput.role).toBe('email'); // Role detection

        const loginBtn = result.elements.find((el) => el.text === 'Login');
        expect(loginBtn).toBeDefined();
        expect(loginBtn.role).toBe('submit'); // type="submit" button gets 'submit' role
    });

    test('filters hidden elements by default', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });

        const hiddenDiv = result.elements.find((el) => el.text === 'Hidden Element');
        expect(hiddenDiv).toBeUndefined();
    });

    test('includes hidden elements when requested', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan', include_hidden: true });
        });

        const hiddenDiv = result.elements.find((el) => el.text === 'Hidden Element');
        expect(hiddenDiv).toBeDefined();
    });

    test('clicks a button', async () => {
        // 1. Scan to get ID
        const scan = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });
        const btn = scan.elements.find((el) => el.attributes.id === 'btn-1');

        // 2. Click
        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'click', id: id });
        }, btn.id);

        expect(result.ok).toBe(true);

        // 3. Verify side effect (log updated)
        const logContent = await page.evaluate(() => document.getElementById('log').innerText);
        expect(logContent).toContain('Button 1 clicked');
    });

    test('types into an input', async () => {
        // 1. Scan
        const scan = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });
        const input = scan.elements.find((el) => el.attributes.id === 'input-1');

        // 2. Type
        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'type', id: id, text: 'Hello World' });
        }, input.id);

        expect(result.ok).toBe(true);

        // 3. Verify value extraction
        const valueResult = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'get_value', id: id });
        }, input.id);
        expect(valueResult.value).toBe('Hello World');

        // 4. Verify side effect (onchange log)
        const logContent = await page.evaluate(() => document.getElementById('log').innerText);
        // Note: type() dispatches change event
        expect(logContent).toContain('Input changed: Hello World');
    });

    test('checks a checkbox', async () => {
        const scan = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });
        const checkbox = scan.elements.find((el) => el.attributes.id === 'check-1');

        await page.evaluate(async (id) => {
            await window.Lemmascope.process({ cmd: 'check', id: id });
        }, checkbox.id);

        const after = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });
        const checkedEl = after.elements.find((el) => el.id === checkbox.id);

        expect(checkedEl.state.checked).toBe(true);
    });

    test('selects an option', async () => {
        const scan = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });
        const select = scan.elements.find((el) => el.attributes.id === 'select-1');

        await page.evaluate(async (id) => {
            await window.Lemmascope.process({ cmd: 'select', id: id, value: '2' });
        }, select.id);

        const valueResult = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'get_value', id: id });
        }, select.id);

        expect(valueResult.value).toBe('2');
    });

    test('waits for condition', async () => {
        // Create delayed element
        await page.evaluate(() => {
            setTimeout(() => {
                const el = document.createElement('div');
                el.id = 'delayed-div';
                el.innerText = 'Appeared!';
                document.body.appendChild(el);
            }, 500);
        });

        const start = Date.now();
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({
                cmd: 'wait_for',
                condition: 'exists',
                selector: '#delayed-div',
                timeout: 1000
            });
        });

        expect(result.ok).toBe(true);
        expect(Date.now() - start).toBeGreaterThanOrEqual(400); // approx
    });
});
