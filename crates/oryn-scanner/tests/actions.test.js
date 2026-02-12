/**
 * Advanced Actions and Extraction Tests for Oryn Scanner
 */

const puppeteer = require('puppeteer');
const fs = require('fs');
const path = require('path');

const SCANNER_JS = fs.readFileSync(path.resolve(__dirname, '../src/scanner.js'), 'utf8');

const runCommand = async (page, cmd) => {
    return page.evaluate(async (command) => {
        return await window.Oryn.process(command);
    }, cmd);
};

describe('Advanced Actions and Extraction', () => {
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
        page.on('console', (msg) => console.log('BROWSER:', msg.text()));
    });

    afterEach(async () => {
        if (page) await page.close();
    });

    describe('Extraction', () => {
        beforeEach(async () => {
            await page.setContent(`
                <html>
                    <head><meta name="description" content="Test Meta"></head>
                    <body>
                        <div id="links">
                            <a href="https://google.com">Google</a>
                            <a href="https://github.com">GitHub</a>
                        </div>
                        <div id="images">
                            <img src="test.png" alt="Test Image">
                        </div>
                        <table>
                            <tr><td>Row 1 Col 1</td><td>Row 1 Col 2</td></tr>
                            <tr><td>Row 2 Col 1</td><td>Row 2 Col 2</td></tr>
                        </table>
                    </body>
                </html>
            `);
            await page.evaluate(SCANNER_JS);
            await runCommand(page, { cmd: 'scan' });
        });

        test('extracts links', async () => {
            const result = await runCommand(page, { cmd: 'extract', source: 'links' });
            expect(result.status).toBe('ok');
            expect(result.results.length).toBe(2);
            expect(result.results[0].text).toBe('Google');
            expect(result.results[0].url).toBe('https://google.com/');
            expect(result.results[0].id).toBeGreaterThan(0);
        });

        test('extracts images', async () => {
            const result = await runCommand(page, { cmd: 'extract', source: 'images' });
            expect(result.status).toBe('ok');
            expect(result.results.length).toBe(1);
            expect(result.results[0].alt).toBe('Test Image');
            expect(result.results[0].id).toBeGreaterThan(0);
        });

        test('extracts tables', async () => {
            const result = await runCommand(page, { cmd: 'extract', source: 'tables' });
            expect(result.status).toBe('ok');
            expect(result.results.length).toBe(1);
            expect(result.results[0].rows.length).toBe(2);
            expect(result.results[0].rows[0][0]).toBe('Row 1 Col 1');
        });

        test('extracts meta', async () => {
            const result = await runCommand(page, { cmd: 'extract', source: 'meta' });
            expect(result.status).toBe('ok');
            const desc = result.results.find((m) => m.name === 'description');
            expect(desc.content).toBe('Test Meta');
        });
    });

    describe('Composite Actions', () => {
        beforeEach(async () => {
            await page.setContent(`
                <html>
                    <body>
                        <!-- Fake Login Form -->
                        <div id="login-container">
                            <input type="text" id="user" name="user" placeholder="Username">
                            <input type="password" id="pass" placeholder="Password">
                            <button id="login-btn">Sign In</button>
                        </div>
                        
                        <!-- Fake Search -->
                        <form id="search-form">
                            <input type="search" id="q" name="q" placeholder="search">
                            <button type="submit">Search</button>
                        </form>

                        <!-- Cookie Banner -->
                        <div id="cookies">
                            <p>We use cookies</p>
                            <button id="accept-cookies">Accept All</button>
                        </div>
                        <script>document.addEventListener('submit', e => e.preventDefault());</script>
                    </body>
                </html>
            `);
            await page.evaluate(SCANNER_JS);
            await runCommand(page, { cmd: 'scan' });
        });

        test('executes login flow', async () => {
            const result = await runCommand(page, {
                cmd: 'login',
                username: 'testuser',
                password: 'password123'
            });
            expect(result.status).toBe('ok');
            expect(result.success).toBe(true);
            expect(result.message).toBe('login_initiated');

            const userVal = await page.$eval('#user', (el) => el.value);
            const passVal = await page.$eval('#pass', (el) => el.value);
            expect(userVal).toBe('testuser');
            expect(passVal).toBe('password123');
        });

        test('executes search flow', async () => {
            const result = await runCommand(page, {
                cmd: 'search',
                query: 'oryn'
            });
            expect(result.status).toBe('ok');
            expect(result.success).toBe(true);
            expect(result.message).toBe('search_initiated');

            const searchVal = await page.$eval('#q', (el) => el.value);
            expect(searchVal).toBe('oryn');
        });

        test('accepts cookies', async () => {
            const result = await runCommand(page, { cmd: 'accept', target: 'cookies' });
            expect(result.status).toBe('ok');
            expect(result.success).toBe(true);
            expect(result.message).toBe('accepted_cookies');
        });
    });
});
