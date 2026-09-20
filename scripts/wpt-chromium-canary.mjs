// Isolated Chromium reference: reads the same report adapter over CDP, without DOM edits.
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { dispatchDriver, settleAutomation } from './wpt-chromium-driver.mjs';
import { createFocusBarrier, shutdownFocusSession } from './wpt-chromium-focus.mjs';
import { createDiagnostics } from './wpt-chromium-diagnostics.mjs';

const [binary, url, deadlineText = '30000'] = process.argv.slice(2);
const diagnostics = createDiagnostics(url);
let diagnosticEmissions = 0;
function emitFailure(error, stage='uncaught') {
  if (diagnosticEmissions >= 2) return;
  diagnosticEmissions++;
  try {
    diagnostics.failure(error);
    process.stderr.write('ORYN_CHROMIUM_DIAGNOSTIC ' + diagnostics.serialize(error, stage) + '\n');
  } catch {
    try { process.stderr.write('ORYN_CHROMIUM_DIAGNOSTIC_UNAVAILABLE\n'); } catch { /* retain original failure */ }
  }
}
// Observe startup failures without changing Node's default nonzero termination.
process.once('uncaughtExceptionMonitor', error => emitFailure(error, 'uncaught'));
if (!binary || !url) throw new Error('usage: script CHROMIUM URL TIMEOUT_MS');
// Separate headless browser processes still compete for real focus on macOS.
// Acquire before launching Chrome, and retain through its complete shutdown.
diagnostics.phase('focus_lease');
const focusLease = spawn(process.env.ORYN_WPT_PYTHON || 'python3',
  [fileURLToPath(new URL('./wpt-reference-lock.py', import.meta.url))],
  {stdio:['pipe','pipe','inherit']});
focusLease.once('spawn', () => diagnostics.browser('focus_lease_spawned', {pid:focusLease.pid}));
focusLease.once('exit', (code, signal) => diagnostics.browser('focus_lease_exit', {pid:focusLease.pid, code, signal}));
await new Promise((resolve,reject) => {
  const timer = setTimeout(() => {
    focusLease.kill('SIGTERM');reject(new Error('Chromium focus lease timeout'));
  },300000);
  focusLease.once('error',error=>{clearTimeout(timer);reject(error)});
  focusLease.once('exit',()=>{clearTimeout(timer);reject(new Error('Chromium focus lease exited'))});
  focusLease.stdout.once('data',data=>{
    clearTimeout(timer);
    if(data.toString().trim()==='ready')resolve();
    else reject(new Error('Chromium focus lease invalid acknowledgement'));
  });
});
diagnostics.phase('browser_startup');
const profile = await mkdtemp(join(tmpdir(), 'oryn-wpt-chromium-'));
const child = spawn(binary, ['--headless=new', '--remote-debugging-port=0',
  `--user-data-dir=${profile}`, '--no-first-run', '--no-default-browser-check',
  '--disable-background-networking', '--ignore-certificate-errors',
  // ChromeDriver's desktop default avoids OS Keychain prompts in this disposable profile.
  ...(process.platform === 'darwin' ? ['--use-mock-keychain'] : []), 'about:blank'],
  { stdio: ['ignore', 'ignore', 'pipe'] });
// Drain continuously; only bounded, explicitly selected native error codes survive.
child.stderr.on('data', chunk => {
  try { diagnostics.browserStderr(chunk); } catch { /* diagnostics cannot fail the run */ }
});
child.stderr.once('end', () => {
  try { diagnostics.browserStderrEnd(); } catch { /* preserve the original outcome */ }
});
child.stderr.once('error', error => diagnostics.browser('stderr_stream_error', {error_code:error.code}));
child.once('spawn', () => diagnostics.browser('spawned', {pid:child.pid}));
const exited = new Promise(resolve => child.once('exit', (code, signal) => {
  diagnostics.browser('exit', {pid:child.pid, code, signal});
  resolve();
}));
let spawnError;
child.on('error', error => { spawnError = error; diagnostics.browser('spawn_error', {error_code:error.code}); });
let socket, browserSocket, focusBarrier, executionError, shuttingDown=false;
const pending=new Map();
try {
  let port;
  const startupDeadline = Date.now() + 15000;
  while (Date.now() < startupDeadline) {
    if (spawnError) throw spawnError;
    if (child.exitCode !== null) throw new Error('Chromium exited during startup');
    try { port = Number((await readFile(join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]); }
    catch { /* startup has not published its endpoint yet */ }
    if (port) break;
    await delay(50);
  }
  if (!port) throw new Error('Chromium startup timeout');
  diagnostics.phase('cdp_connection');
  const endpoint = await fetch(`http://127.0.0.1:${port}/json/new?about:blank`,
    { method: 'PUT', signal: AbortSignal.timeout(5000) }).then(response => response.json());
  socket = new WebSocket(endpoint.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener('open', resolve, { once: true });
    socket.addEventListener('error', reject, { once: true });
  });
  const browserEndpoint=await fetch(`http://127.0.0.1:${port}/json/version`,{signal:AbortSignal.timeout(5000)}).then(response=>response.json());
  browserSocket=new WebSocket(browserEndpoint.webSocketDebuggerUrl);
  await new Promise((resolve,reject)=>{browserSocket.addEventListener('open',resolve,{once:true});browserSocket.addEventListener('error',reject,{once:true})});
  browserSocket.addEventListener('message',event=>{
    const response=JSON.parse(event.data),waiter=pending.get(response.id);
    if(!waiter||waiter.endpoint!==browserSocket)return;
    pending.delete(response.id);clearTimeout(waiter.timer);diagnostics.response(response.id,!!response.error);
    response.error?waiter.reject(new Error(JSON.stringify(response.error))):waiter.resolve(response.result);
  });
  let nextId = 0;
  const contexts = new Map();
  const driverStates = new Map();
  const bridgeErrors = [];
  const contextKey = (sessionId, id) => `${sessionId || ''}:${id}`;
  const automationEvents = [];
  let automationOverflow = false;
  function recordOperation(event) {
    if (automationEvents.length >= 4096) automationOverflow = true;
    else automationEvents.push(event);
  }
  let operationId = 0;
  let automationQueue = Promise.resolve();
  let entryFocus;
  let entryContext;
  let creationFocus;
  socket.addEventListener('message', event => {
    const response = JSON.parse(event.data);
    diagnostics.event(response);
    focusBarrier?.event(response);
    const sessionId = response.sessionId;
    if (response.method === 'Runtime.bindingCalled' && !sessionId &&
        response.params.name === '__oryn_reference_focus') {
      try {
        const observation = JSON.parse(response.params.payload);
        if (observation.phase === 'document_created' && creationFocus === undefined) creationFocus = observation.focused === true;
        if (observation.phase === 'report_ready' && entryFocus === undefined) {
          const current=contexts.get(contextKey(sessionId,response.params.executionContextId));
          if(!focusBarrier?.admitted(response.params.executionContextId,current))throw Error('report without native pre-author focus admission');
          entryFocus = observation.focused === true;
          const key = contextKey(sessionId, response.params.executionContextId);
          entryContext = {key, generation:contexts.get(key), id:response.params.executionContextId};
        }
      } catch { if (bridgeErrors.length < 64) bridgeErrors.push('invalid_reference_focus_observation'); }
    }
    if (response.method === 'Target.attachedToTarget') {
      const childSession = response.params.sessionId;
      (async () => {
        if (!['iframe','page'].includes(response.params.targetInfo.type)) {
          await call('Runtime.runIfWaitingForDebugger', {}, childSession);
          return;
        }
        await call('Runtime.enable', {}, childSession);
        await call('Runtime.addBinding', {name:'__oryn_driver_request'}, childSession);
        await call('Target.setAutoAttach', {autoAttach:true, waitForDebuggerOnStart:true, flatten:true}, childSession);
        await call('Runtime.runIfWaitingForDebugger', {}, childSession);
      })().catch(async () => {
        if (bridgeErrors.length < 64) bridgeErrors.push('child_bridge_initialization_failed');
        await call('Runtime.runIfWaitingForDebugger', {}, childSession).catch(() => {});
      });
    } else if (response.method === 'Target.detachedFromTarget') {
      const prefix = `${response.params.sessionId}:`;
      for (const key of contexts.keys()) if (key.startsWith(prefix)) { contexts.delete(key); driverStates.delete(key); }
    }
    if (response.method === 'Runtime.executionContextCreated') {
      const key = contextKey(sessionId,response.params.context.id);
      contexts.set(key, response.params.context.uniqueId);
      driverStates.set(key, {});
    } else if (response.method === 'Runtime.executionContextDestroyed') {
      const key = contextKey(sessionId,response.params.executionContextId);
      contexts.delete(key);
      driverStates.delete(key);
    } else if (response.method === 'Runtime.executionContextsCleared') {
      const prefix = `${sessionId || ''}:`;
      for (const key of contexts.keys()) if (key.startsWith(prefix)) { contexts.delete(key); driverStates.delete(key); }
    } else if (response.method === 'Runtime.bindingCalled' && response.params.name === '__oryn_driver_request') {
      const contextId = response.params.executionContextId;
      const key = contextKey(sessionId,contextId);
      const generation = contexts.get(key);
      const alive = () => generation !== undefined && contexts.get(key) === generation;
      const scopedCall = (method,params) => call(method,params,sessionId);
      automationQueue = automationQueue.then(async () => {
        if (!alive()) return;
        let request;
        try { request = JSON.parse(response.params.payload); } catch { return; }
        if (!Number.isSafeInteger(request.id) || typeof request.token !== 'string') return;
        const hostId = ++operationId;
        recordOperation({id:hostId, operation:request.operation, outcome:'started', backend:'chromium', context_id:contextId});
        let result = null, error = null;
        try { result = await dispatchDriver(scopedCall, contextId, request, alive, driverStates.get(key)); }
        catch (failure) { error = failure.message === 'unsupported' ? 'unsupported' : 'automation_failed'; }
        recordOperation({id:hostId, operation:request.operation,
          outcome:!alive() ? 'failed' : error === 'unsupported' ? 'unsupported' : error ? 'failed' : 'completed', backend:'chromium', context_id:contextId});
        if (alive()) await scopedCall('Runtime.evaluate', {contextId,
          expression: `globalThis.__oryn_driver_reply?.(${JSON.stringify(request.token)},${request.id},${JSON.stringify(result)},${JSON.stringify(error)})`});
      }).catch(() => {});
    }
    const waiter = pending.get(response.id);
    if (!waiter || waiter.endpoint!==socket) return;
    pending.delete(response.id);
    diagnostics.response(response.id, !!response.error);
    clearTimeout(waiter.timer);
    if (response.error) waiter.reject(new Error(JSON.stringify(response.error)));
    else waiter.resolve(response.result);
  });
  function call(method, params = {}, sessionId, endpoint=socket) {
    if(shuttingDown)return Promise.reject(new Error('Chromium bridge shutting down'));
    return new Promise((resolve, reject) => {
      const id = ++nextId;
      const diagnosticRequest = diagnostics.request(id, method, sessionId);
      const timer = setTimeout(() => { pending.delete(id); diagnostics.timeout(id, 10000, diagnosticRequest); reject(new Error(`CDP timeout: ${method}`)); }, 10000);
      pending.set(id, { resolve, reject, timer, endpoint });
      endpoint.send(JSON.stringify({ id, method, params, ...(sessionId ? {sessionId} : {}) }));
    });
  }
  diagnostics.phase('cdp_setup');
  await call('Runtime.enable');
  await call('Page.enable');
  await call('Network.enable');
  await call('Runtime.addBinding', {name:'__oryn_driver_request'});
  await call('Runtime.addBinding', {name:'__oryn_reference_focus'});
  focusBarrier=createFocusBarrier({call,activate:targetId=>call('Target.activateTarget',{targetId},undefined,browserSocket),targetId:endpoint.id,
    observe:(kind,data)=>{if(kind==='preauthor_focus_observed'&&creationFocus===undefined)creationFocus=data.focused;diagnostics.phase(kind)}});
  await focusBarrier.install((await call('Page.getFrameTree')).frameTree.frame.id);
  await call('Target.setAutoAttach', {autoAttach:true, waitForDebuggerOnStart:true, flatten:true});
  const version = await call('Browser.getVersion');
  // /json/new creates a background target. Focus must be established before
  // navigation: parser-time focus tests otherwise silently miss native events.
  // Do not use focus emulation; tests must still be able to move focus away.
  diagnostics.phase('initial_focus');
  await call('Page.bringToFront');
  const focusProbe = await call('Runtime.evaluate', {
    expression: 'document.hasFocus()', returnByValue: true });
  if (focusProbe.result?.value !== true) throw new Error('Chromium initial focus preflight failed');
  diagnostics.phase('navigate');
  focusBarrier.beginNavigation(url);
  focusBarrier.navigationReply(await call('Page.navigate', { url }));
  await focusBarrier.ready();
  diagnostics.phase('completion');
  const deadline = Date.now() + Number(deadlineText);
  let completion;
  while (Date.now() < deadline) {
    await focusBarrier.idle();
    const value = await call('Runtime.evaluate', {
      expression: 'globalThis.__oryn_wpt_completion || null', returnByValue: true });
    if (value.result?.value) { completion = value.result.value; break; }
    await delay(25);
  }
  if (!completion) throw new Error('Chromium completion timeout');
  await focusBarrier.idle();
  diagnostics.completion(completion);
  if (entryFocus !== true) throw new Error(`Chromium document-entry focus unverified (${entryFocus === undefined ? 'missing observation' : 'unfocused'})`);
  diagnostics.phase('settle_automation');
  completion = await settleAutomation(() => automationQueue, async () => {
    const value = await call('Runtime.evaluate', {contextId:entryContext.id,
      expression:'globalThis.__oryn_wpt_completion || null', returnByValue:true});
    if (value.exceptionDetails) throw new Error('reference_completion_read_failed');
    return value.result?.value;
  }, () => entryContext?.generation !== undefined &&
    contexts.get(entryContext.key) === entryContext.generation, deadline);
  await focusBarrier.idle();
  if(!focusBarrier.admitted(entryContext.id,entryContext.generation))throw Error('completion native focus identity retired');
  await focusBarrier.finish(entryContext.id,entryContext.generation);
  process.stdout.write(JSON.stringify({ completion, version, automation_events:automationEvents,
    initial_focus_verified:true,
    preauthor_focus_verified:true,
    entry_focus_verified:true,
    entry_focus_phase:'report_ready',
    creation_focus_observed:creationFocus ?? null,
    focus_isolation:'host_serialized',
    automation_overflow:automationOverflow, automation_bridge_errors:bridgeErrors }) + '\n');
} catch (error) {
  executionError = error;
  emitFailure(error, 'failure');
  throw error;
} finally {
  diagnostics.phase('cleanup');
  try {
    await shutdownFocusSession({barrier:focusBarrier,
      cancel(){shuttingDown=true;for(const waiter of pending.values()){clearTimeout(waiter.timer);waiter.reject(new Error('Chromium bridge shutting down'))}pending.clear()},
      async stopBrowser(){
        if(child.exitCode===null&&child.signalCode===null)child.kill('SIGTERM');
        await Promise.race([exited,delay(5000)]);
        if(child.exitCode===null&&child.signalCode===null){child.kill('SIGKILL');await Promise.race([exited,delay(5000)])}
        if(child.exitCode===null&&child.signalCode===null)throw Error('owned Chromium did not terminate');
      },
      detach(){socket?.close();browserSocket?.close()},
      async release(){focusLease.stdin.end();await rm(profile,{recursive:true,force:true})},
    });
  }catch(cleanupError){emitFailure(cleanupError,'cleanup');throw cleanupError}
  if (executionError) emitFailure(executionError, 'cleanup');
}
