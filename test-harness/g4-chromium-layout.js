#!/usr/bin/env node
'use strict';

// Chromium is used only as differential evidence. Native Oryn never imports,
// launches, or speaks CDP in its production path.
const childProcess = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const sleep = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const chrome = [
    process.env.CHROME_BIN,
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/Applications/Chromium.app/Contents/MacOS/Chromium',
    '/usr/bin/google-chrome',
    '/usr/bin/chromium',
].filter(Boolean).find(candidate => fs.existsSync(candidate));
if (!chrome) throw new Error('Chromium executable not found; set CHROME_BIN');

async function waitFor(operation, timeoutMs = 15_000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        const value = await operation();
        if (value) return value;
        await sleep(25);
    }
    throw new Error('timed out waiting for Chromium');
}

async function main() {
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'oryn-g4-chromium-'));
    const version = childProcess.execFileSync(chrome, ['--version'], {encoding: 'utf8'}).trim();
    const processHandle = childProcess.spawn(chrome, [
        '--headless=new', '--disable-background-networking', '--disable-component-update',
        '--disable-default-apps', '--disable-sync', '--no-first-run',
        '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=0',
        `--user-data-dir=${profile}`, 'about:blank',
    ], {stdio: ['ignore', 'ignore', 'pipe']});
    try {
        const portFile = path.join(profile, 'DevToolsActivePort');
        const port = Number(await waitFor(() => fs.existsSync(portFile)
            && fs.readFileSync(portFile, 'utf8').split(/\r?\n/)[0]));
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
        await call('Runtime.enable');
        await call('Emulation.setDeviceMetricsOverride', {width: 400, height: 300, deviceScaleFactor: 1, mobile: false});
        const html = `<!doctype html><style>
          body{margin:0}.block{width:200px;padding:10px;border:2px solid}
          .flex{display:flex;width:300px;gap:10px}.flex>button{width:80px;height:30px}
          .grid{display:grid;grid-template-columns:1fr 1fr;width:300px;gap:8px}.grid>span{height:20px}
          .text{width:90px;font-size:16px}.move{width:40px;height:20px;transform:translate(12px,8px)}
          .spacer{height:500px}.fixed{position:fixed;left:5px;top:7px;width:20px;height:20px}
        </style><div id=block class=block>Block</div>
        <div class=flex><button id=f1>One</button><button id=f2>Two</button></div>
        <div class=grid><span id=g1>A</span><span id=g2>B</span><span id=g3>C</span></div>
        <p id=text class=text>Oryn deterministic inline wrapping fixture text</p>
        <div id=move class=move></div><div class=spacer></div><div id=fixed class=fixed></div>`;
        await call('Page.enable');
        await call('Page.navigate', {url: `data:text/html,${encodeURIComponent(html)}`});
        await waitFor(async () => {
            const response = await call('Runtime.evaluate', {expression: "document.readyState==='complete'", returnByValue: true});
            return response.result.value;
        });
        const expression = `(() => {
          const r=id=>document.querySelector(id).getBoundingClientRect();
          const before=r('#fixed');scrollTo(0,400);const after=r('#fixed');
          return {block_border_width:r('#block').width,
            flex_second_right_of_first:r('#f2').x>r('#f1').x,
            grid_second_right_of_first:r('#g2').x>r('#g1').x,
            grid_third_below_first:r('#g3').y>r('#g1').y,
            text_has_fragments:document.querySelector('#text').getClientRects().length>0,
            transform_moves_x:r('#move').x>=12,
            fixed_stable_after_scroll:Math.abs(before.y-after.y)<0.01,
            scroll_clamped:scrollY>0};
        })()`;
        const response = await call('Runtime.evaluate', {expression, returnByValue: true});
        console.log(JSON.stringify({schema_version: 1, domain: 'chromium', chromium_version: version, relations: response.result.value, outcome: {status: 'passed'}}, null, 2));
        socket.close();
    } finally {
        if (processHandle.exitCode === null) processHandle.kill('SIGTERM');
        await Promise.race([new Promise(resolve => processHandle.once('exit', resolve)), sleep(2_000)]);
        if (processHandle.exitCode === null) processHandle.kill('SIGKILL');
        fs.rmSync(profile, {recursive: true, force: true});
    }
}

main().catch(error => {
    console.error(error.stack || String(error));
    process.exitCode = 1;
});
