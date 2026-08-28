#!/usr/bin/env node
'use strict';

// Chromium is deliberately confined to the differential harness. Native Oryn
// never imports, launches, or speaks CDP in its production execution path.

const childProcess = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const sleep = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));

async function waitFor(description, operation, timeoutMs = 15_000) {
    const deadline = Date.now() + timeoutMs;
    let lastError;
    while (Date.now() < deadline) {
        try {
            const result = await operation();
            if (result) return result;
        } catch (error) {
            lastError = error;
        }
        await sleep(50);
    }
    throw new Error(`timed out waiting for ${description}${lastError ? `: ${lastError.message}` : ''}`);
}

function chromiumExecutable() {
    const candidates = [
        process.env.CHROME_BIN,
        '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
        '/Applications/Chromium.app/Contents/MacOS/Chromium',
        '/usr/bin/google-chrome',
        '/usr/bin/chromium',
        '/usr/bin/chromium-browser',
    ].filter(Boolean);
    const executable = candidates.find(candidate => fs.existsSync(candidate));
    if (!executable) throw new Error('Chromium executable not found; set CHROME_BIN');
    return executable;
}

class CdpTarget {
    constructor(target) {
        this.target = target;
        this.nextId = 1;
        this.pending = new Map();
        this.diagnostics = [];
    }

    async connect() {
        try {
            this.socket = new WebSocket(this.target.webSocketDebuggerUrl);
            this.socket.addEventListener('message', event => {
                const message = JSON.parse(String(event.data));
                if (message.id) {
                    const pending = this.pending.get(message.id);
                    if (!pending) return;
                    this.pending.delete(message.id);
                    if (message.error) pending.reject(new Error(message.error.message));
                    else pending.resolve(message.result);
                    return;
                }
                if (message.method === 'Runtime.exceptionThrown') {
                    this.diagnostics.push({kind: 'exception', detail: message.params.exceptionDetails.text});
                }
                if (message.method === 'Runtime.consoleAPICalled' && message.params.type === 'error') {
                    this.diagnostics.push({kind: 'console_error', detail: message.params.args.map(item => item.value ?? item.description ?? '').join(' ')});
                }
            });
            await new Promise((resolve, reject) => {
                this.socket.addEventListener('open', resolve, {once: true});
                this.socket.addEventListener('error', () => reject(new Error('CDP WebSocket failed')), {once: true});
            });
            await this.call('Runtime.enable');
            await this.call('Page.enable');
            return this;
        } catch (error) {
            await this.close();
            throw error;
        }
    }

    call(method, params = {}, timeoutMs = 15_000) {
        const id = this.nextId++;
        const promise = new Promise((resolve, reject) => {
            const timer = setTimeout(() => {
                this.pending.delete(id);
                reject(new Error(`CDP ${method} timed out after ${timeoutMs}ms`));
            }, timeoutMs);
            this.pending.set(id, {
                resolve: value => {
                    clearTimeout(timer);
                    resolve(value);
                },
                reject: error => {
                    clearTimeout(timer);
                    reject(error);
                },
            });
        });
        try {
            this.socket.send(JSON.stringify({id, method, params}));
        } catch (error) {
            this.pending.get(id)?.reject(error);
            this.pending.delete(id);
        }
        return promise;
    }

    async evaluate(expression) {
        const response = await this.call('Runtime.evaluate', {
            expression,
            awaitPromise: true,
            returnByValue: true,
            userGesture: true,
        });
        if (response.exceptionDetails) {
            throw new Error(response.exceptionDetails.exception?.description || response.exceptionDetails.text);
        }
        return response.result.value;
    }

    async wait(expression, description, timeoutMs) {
        return waitFor(description, () => this.evaluate(expression), timeoutMs);
    }

    async close() {
        if (this.socket?.readyState === WebSocket.OPEN) this.socket.close();
    }
}

class ChromiumHarness {
    async start() {
        this.profile = fs.mkdtempSync(path.join(os.tmpdir(), 'oryn-g3-chromium-'));
        this.executable = chromiumExecutable();
        this.version = childProcess.execFileSync(this.executable, ['--version'], {encoding: 'utf8'}).trim();
        this.process = childProcess.spawn(this.executable, [
            '--headless=new',
            '--disable-background-networking',
            '--disable-background-timer-throttling',
            '--disable-backgrounding-occluded-windows',
            '--disable-component-update',
            '--disable-default-apps',
            '--disable-features=Translate,MediaRouter',
            '--disable-renderer-backgrounding',
            '--disable-sync',
            '--metrics-recording-only',
            '--no-first-run',
            '--no-default-browser-check',
            '--remote-debugging-address=127.0.0.1',
            '--remote-debugging-port=0',
            `--user-data-dir=${this.profile}`,
            'about:blank',
        ], {stdio: ['ignore', 'ignore', 'pipe']});
        let stderr = '';
        this.process.stderr.on('data', chunk => { stderr += String(chunk); });
        const portFile = path.join(this.profile, 'DevToolsActivePort');
        this.port = Number(await waitFor('Chromium DevTools endpoint', () => {
            if (!fs.existsSync(portFile)) {
                if (this.process.exitCode !== null) throw new Error(`Chromium exited: ${stderr}`);
                return false;
            }
            return fs.readFileSync(portFile, 'utf8').split(/\r?\n/)[0];
        }));
        return this;
    }

    async json(route, options = {}) {
        const response = await fetch(`http://127.0.0.1:${this.port}${route}`, options);
        if (!response.ok) throw new Error(`DevTools ${route} returned ${response.status}`);
        return response.json();
    }

    async newPage(url) {
        let lastError;
        for (let attempt = 1; attempt <= 3; attempt += 1) {
            const target = await this.json(`/json/new?${encodeURIComponent(url)}`, {method: 'PUT'});
            try {
                const page = await new CdpTarget(target).connect();
                await page.wait("document.readyState === 'complete'", `load ${url}`);
                return page;
            } catch (error) {
                lastError = error;
                await fetch(`http://127.0.0.1:${this.port}/json/close/${target.id}`).catch(() => {});
                if (attempt < 3) await sleep(250);
            }
        }
        throw new Error(`failed to create a responsive Chromium page for ${url}: ${lastError?.message ?? 'unknown error'}`);
    }

    async targets() {
        return (await this.json('/json/list')).filter(target => target.type === 'page');
    }

    async waitForPopup(parent, fragment) {
        const target = await waitFor(`popup URL containing ${fragment}`, async () => {
            const targets = await this.targets();
            return targets.find(candidate => candidate.id !== parent.target.id && candidate.url.includes(fragment));
        });
        const page = await new CdpTarget(target).connect();
        await page.wait("document.readyState === 'complete'", `popup ${fragment}`);
        return page;
    }

    async closePage(page) {
        try {
            const response = await fetch(
                `http://127.0.0.1:${this.port}/json/close/${page.target.id}`,
            );
            if (!response.ok) {
                throw new Error(`DevTools close returned ${response.status}`);
            }
        } finally {
            await page.close();
        }
    }

    async stop() {
        if (this.process && this.process.exitCode === null) {
            this.process.kill('SIGTERM');
            await Promise.race([
                new Promise(resolve => this.process.once('exit', resolve)),
                sleep(2_000),
            ]);
            if (this.process.exitCode === null) this.process.kill('SIGKILL');
        }
        if (this.profile?.startsWith(os.tmpdir() + path.sep)) {
            fs.rmSync(this.profile, {recursive: true, force: true});
        }
    }
}

const clickText = text => `(() => { const element=[...document.querySelectorAll('button,a')].find(item=>item.textContent.trim()===${JSON.stringify(text)}); if(!element)return false;element.click();return true; })()`;
const outputIs = value => `document.querySelector('output')?.textContent.trim() === ${JSON.stringify(value)}`;
const setControl = (label, value, eventType) => `(() => { const element=document.querySelector('[aria-label=${JSON.stringify(label)}]');if(!element)return false;element.value=${JSON.stringify(value)};element.dispatchEvent(new Event(${JSON.stringify(eventType)},{bubbles:true}));return true; })()`;
const recordPass = (harness, caseId) => console.log(JSON.stringify({case: caseId, domain: 'chromium', status: 'passed', chromium_version: harness.version}));

async function assertClean(page, label) {
    if (page.diagnostics.length) throw new Error(`${label} emitted diagnostics: ${JSON.stringify(page.diagnostics)}`);
}

async function runFrameworks(harness, baseUrl) {
    for (const framework of ['react', 'next', 'vue', 'nuxt', 'angular', 'svelte', 'sveltekit', 'preact']) {
        const page = await harness.newPage(`${baseUrl}/g3/frameworks/${framework}/`);
        await page.wait("document.body?.innerText.includes('Items: 0')", `${framework} hydration`);
        // SSR text can be visible before a framework has attached its event
        // listeners. Retry the public control until its observable output
        // changes; each click and check is a bounded CDP task and no private
        // framework state is inspected.
        await waitFor(`${framework} hydrated action`, async () => {
            if (await page.evaluate(outputIs('Items: 1'))) return true;
            if (!await page.evaluate(clickText('Add item'))) {
                throw new Error(`${framework} Add item button not found`);
            }
            return page.evaluate(outputIs('Items: 1'));
        });
        if (framework === 'react') {
            await page.wait("document.querySelector('#g3-modern-surface') !== null", 'React enhanced surface');
            if (!await page.evaluate(setControl('Search', 'oryn', 'input'))) throw new Error('React Search control not found');
            if (!await page.evaluate(setControl('Category', 'tools', 'change'))) throw new Error('React Category control not found');
            if (!await page.evaluate(setControl('Delivery date', '2026-08-27', 'input'))) throw new Error('React date control not found');
            for (const action of ['Attach fixture file', 'Load chunk', 'Open modal']) {
                if (!await page.evaluate(clickText(action))) throw new Error(`React ${action} control not found`);
            }
            await page.wait("document.querySelector('[role=dialog]')?.textContent.includes('Portal modal')", 'React portal modal');
            if (!await page.evaluate(clickText('Close modal'))) throw new Error('React close-modal control not found');
            for (const action of ['Next page', 'Buy']) {
                if (!await page.evaluate(clickText(action))) throw new Error(`React ${action} control not found`);
            }
            await page.wait(
                "document.querySelector('#g3-surface')?.textContent.trim() === 'Surface: oryn|tools|2026-08-27|2|1|fixture.txt:7|loaded|closed'",
                'React modern-app surface',
            );
            const structuralSurface = await page.evaluate("document.querySelectorAll('#heavy-ssr article').length === 12 && document.querySelectorAll('#g3-modern-surface table tbody tr').length === 1 && document.querySelector('g3-badge')?.textContent === 'Custom element ready' && document.querySelector('a[download=\"oryn-report.json\"]') !== null");
            if (!structuralSurface) throw new Error('React SSR/table/custom-element/download surface failed');
        }
        await assertClean(page, framework);
        recordPass(harness, `framework-${framework}`);
        await harness.closePage(page);
    }
}

async function runOAuth(harness, baseUrl) {
    {
        const parent = await harness.newPage(`${baseUrl}/g3/auth/rp/popup`);
        await parent.evaluate(clickText('Sign in with IdP'));
        const child = await harness.waitForPopup(parent, '/g3/auth/idp/authorize');
        await child.evaluate(clickText('Approve'));
        await parent.wait(outputIs('Authenticated with PKCE'), 'PKCE callback');
        await assertClean(parent, 'OAuth popup parent');
        recordPass(harness, 'oauth-popup-code-pkce');
        await harness.closePage(parent);
    }
    {
        const page = await harness.newPage(`${baseUrl}/g3/auth/rp/redirect`);
        await page.evaluate(clickText('Start redirect sign in'));
        await page.wait("location.pathname.endsWith('/g3/auth/idp/redirect')", 'IdP redirect');
        await page.evaluate(clickText('Approve redirect'));
        await page.wait(outputIs('Redirect authenticated'), 'redirect callback');
        await assertClean(page, 'OAuth redirect');
        recordPass(harness, 'oauth-redirect');
        await harness.closePage(page);
    }
    {
        const parent = await harness.newPage(`${baseUrl}/g3/auth/idp/session`);
        await parent.call('Page.navigate', {url: `${baseUrl}/g3/auth/rp/sso`});
        await parent.wait("document.readyState === 'complete'", 'SSO relying party');
        await parent.evaluate(clickText('Continue with SSO'));
        // The existing-session IdP approves and closes in a microtask, so the
        // popup may disappear before target discovery. The RP postcondition is
        // the observable contract for this flow.
        await parent.wait(outputIs('SSO authenticated'), 'existing-session SSO');
        await assertClean(parent, 'existing-session SSO');
        recordPass(harness, 'oauth-existing-session-sso');
        await harness.closePage(parent);
    }
    {
        const parent = await harness.newPage(`${baseUrl}/g3/auth/rp/mfa`);
        await parent.evaluate(clickText('Start MFA'));
        const child = await harness.waitForPopup(parent, '/g3/auth/idp/mfa');
        await child.evaluate("document.querySelector('input').value='246810';document.querySelector('input').dispatchEvent(new Event('input',{bubbles:true}));true");
        await child.evaluate(clickText('Verify code'));
        await parent.wait(outputIs('MFA authenticated'), 'MFA handoff');
        await assertClean(parent, 'MFA handoff');
        recordPass(harness, 'oauth-mfa-handoff-return');
        await harness.closePage(parent);
    }
    {
        const parent = await harness.newPage(`${baseUrl}/g3/auth/rp/recovery`);
        await parent.evaluate(clickText('Open popup'));
        const child = await harness.waitForPopup(parent, '/g3/auth/idp/authorize');
        await child.evaluate(clickText('Cancel'));
        await waitFor('popup target closure', async () => !(await harness.targets()).some(target => target.id === child.target.id));
        await parent.evaluate(clickText('Check popup'));
        await parent.wait(outputIs('Popup closed; opener recovered'), 'popup close recovery');
        await assertClean(parent, 'popup close recovery');
        await harness.closePage(parent);

        const denial = await harness.newPage(`${baseUrl}/g3/auth/rp/recovery`);
        await denial.evaluate("window.open=()=>null;true");
        await denial.evaluate(clickText('Open popup'));
        await denial.wait(outputIs('Popup denied'), 'popup denial recovery');
        await assertClean(denial, 'popup denial recovery');
        recordPass(harness, 'oauth-popup-denial-close-recovery');
        await harness.closePage(denial);
    }
}

async function main() {
    const [mode, baseUrl] = process.argv.slice(2);
    if (!['frameworks', 'oauth'].includes(mode) || !baseUrl) {
        throw new Error('usage: g3-chromium.js frameworks|oauth http://127.0.0.1:PORT');
    }
    const harness = await new ChromiumHarness().start();
    try {
        if (mode === 'frameworks') await runFrameworks(harness, baseUrl.replace(/\/$/, ''));
        else await runOAuth(harness, baseUrl.replace(/\/$/, ''));
    } finally {
        await harness.stop();
    }
}

main().catch(error => {
    console.error(error.stack || String(error));
    process.exitCode = 1;
});
