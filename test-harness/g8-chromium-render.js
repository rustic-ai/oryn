#!/usr/bin/env node
'use strict';

// Differential harness only. Native Oryn never imports, launches, or speaks CDP.
const childProcess = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const output = process.argv[2];
if (!output) throw new Error('output PNG path is required');
const sleep = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const chrome = [process.env.CHROME_BIN,
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/Applications/Chromium.app/Contents/MacOS/Chromium',
    '/usr/bin/google-chrome', '/usr/bin/chromium']
    .filter(Boolean).find(candidate => fs.existsSync(candidate));
if (!chrome) throw new Error('Chromium executable not found; set CHROME_BIN');

async function waitFor(operation, timeoutMs = 15000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        const value = await operation();
        if (value) return value;
        await sleep(25);
    }
    throw new Error('timed out waiting for Chromium');
}

async function main() {
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'oryn-g8-chromium-'));
    const version = childProcess.execFileSync(chrome, ['--version'], {encoding: 'utf8'}).trim();
    const processHandle = childProcess.spawn(chrome, [
        '--headless=new', '--disable-background-networking', '--disable-component-update',
        '--disable-default-apps', '--disable-sync', '--no-first-run',
        '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=0',
        `--user-data-dir=${profile}`, 'about:blank'], {stdio: ['ignore', 'ignore', 'pipe']});
    try {
        const portFile = path.join(profile, 'DevToolsActivePort');
        const port = Number(await waitFor(() => fs.existsSync(portFile) && fs.readFileSync(portFile, 'utf8').split(/\r?\n/)[0]));
        const target = await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, {method: 'PUT'}).then(response => response.json());
        const socket = new WebSocket(target.webSocketDebuggerUrl);
        await new Promise((resolve, reject) => {
            socket.addEventListener('open', resolve, {once: true});
            socket.addEventListener('error', reject, {once: true});
        });
        let nextId = 1;
        const pending = new Map();
        socket.addEventListener('message', event => {
            const message = JSON.parse(String(event.data));
            const waiter = pending.get(message.id);
            if (!waiter) return;
            pending.delete(message.id);
            message.error ? waiter.reject(new Error(message.error.message)) : waiter.resolve(message.result);
        });
        const call = (method, params = {}) => new Promise((resolve, reject) => {
            const id = nextId++;
            pending.set(id, {resolve, reject});
            socket.send(JSON.stringify({id, method, params}));
        });
        await call('Page.enable');
        await call('Runtime.enable');
        await call('Emulation.setDeviceMetricsOverride', {width: 320, height: 200, deviceScaleFactor: 1, mobile: false});
        const html = `<!doctype html><style>
          html,body{margin:0;background-color:#fafafa;color:#102030}
          #card{width:220px;height:72px;padding:8px;border:3px solid #204080;background-color:#dceeff;font-size:18px}
          img{display:block;width:16px;height:16px}
        </style><div id=card>Oryn CPU paint
        <img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="></div>`;
        await call('Page.navigate', {url: `data:text/html,${encodeURIComponent(html)}`});
        await waitFor(async () => (await call('Runtime.evaluate', {expression: "document.readyState==='complete'", returnByValue: true})).result.value);
        const shot = await call('Page.captureScreenshot', {format: 'png', fromSurface: true});
        fs.writeFileSync(output, Buffer.from(shot.data, 'base64'));
        console.log(JSON.stringify({schema_version: 1, domain: 'chromium', chromium_version: version, width: 320, height: 200, outcome: {status: 'passed'}}, null, 2));
        socket.close();
    } finally {
        if (processHandle.exitCode === null) processHandle.kill('SIGTERM');
        await Promise.race([new Promise(resolve => processHandle.once('exit', resolve)), sleep(2000)]);
        if (processHandle.exitCode === null) processHandle.kill('SIGKILL');
        fs.rmSync(profile, {recursive: true, force: true});
    }
}

main().catch(error => { console.error(error.stack || String(error)); process.exitCode = 1; });
