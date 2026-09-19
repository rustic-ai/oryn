import test from 'node:test';
import assert from 'node:assert/strict';

async function mount(t, replay = false) {
  const captureListeners = {}, storage = new Map(), navigations = [];
  const listeners = {}, lifecycle = {}, pending = [], buttons = [{disabled: false}];
  let message;
  const children = [];
  if (replay) storage.set('oryn-controlled-write:/saas/case/', JSON.stringify({
    url: 'https://app.test/saas/case/update', body: 'version=1&title=Approved'}));
  const root = {dataset: {}, setAttribute() {}, contains: value => value === form,
    querySelector: selector => selector === '#version' ? {textContent: '1'} :
      selector === '#record' && replay ? {} : children.find(node => '#'+node.id === selector),
    querySelectorAll: () => buttons, append: (...nodes) => { children.push(...nodes); message = nodes.at(-1); }};
  class Form { constructor() { this.action = 'https://app.test/saas/case/update'; this.method = 'post'; } }
  const form = new Form();
  const replacements = {
    location: {href: 'https://app.test/saas/case/records', origin: 'https://app.test', pathname: '/saas/case/records', replace: url => navigations.push(url)},
    document: {querySelector: () => root, addEventListener: (name, callback, capture) => { (capture ? captureListeners : listeners)[name] = callback; },
      createElement: () => ({dataset: {}, events: {}, setAttribute() {},
        addEventListener(name, callback) { this.events[name] = callback; }})},
    addEventListener: (name, callback) => { lifecycle[name] = callback; },
    sessionStorage: {getItem: key => storage.get(key) ?? null, setItem: (key,value) => storage.set(key,value)},
    MutationObserver: class { observe() {} },
    HTMLFormElement: Form,
    FormData: class extends Array { constructor() { super(['csrf', 'fixture-csrf'], ['title', 'Approved'], ['version', '1']); } },
    fetch: (url, options) => new Promise((resolve, reject) => pending.push({url, options, resolve, reject})),
  };
  for (const [name, value] of Object.entries(replacements)) {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, name);
    Object.defineProperty(globalThis, name, {value, writable: true, configurable: true});
    t.after(() => descriptor ? Object.defineProperty(globalThis, name, descriptor) : delete globalThis[name]);
  }
  await import(`./saas-client.mjs?test=${encodeURIComponent(t.name)}`);
  return {root, buttons, pending, lifecycle, navigations, storage, get message() { return message; },
    submit: () => {const event = {target: form, preventDefault() {}}; captureListeners.submit?.(event); return listeners.submit(event);}};
}

test('controlled replay shares mutation admission and retires on pagehide', async t => {
  const ui = await mount(t, true);
  const button = ui.root.querySelector('#repeat-save');
  const status = ui.root.querySelector('#duplicate-status');
  const replay = button.events.click();
  await button.events.click();
  await ui.submit();
  assert.equal(ui.pending.length, 1);
  assert.equal(ui.pending[0].options.body.toString(), 'version=1&title=Approved');
  assert.equal(JSON.parse(ui.storage.get('oryn-controlled-write:/saas/case/')).body,
    'version=1&title=Approved');
  ui.lifecycle.pagehide();
  assert.equal(ui.pending[0].options.signal.aborted, true);
  ui.pending[0].resolve({status: 409, text: async () => 'duplicate'});
  await replay;
  assert.equal(status.dataset.count, undefined);
  await button.events.click();
  assert.equal(ui.pending.length, 1);
});

test('ambiguous controlled replay failure requires a read before further writes', async t => {
  const ui = await mount(t, true);
  const button = ui.root.querySelector('#repeat-save');
  const replay = button.events.click();
  ui.pending[0].reject(new Error('connection lost'));
  await replay;
  assert.equal(ui.root.dataset.state, 'error');
  assert.equal(button.disabled, true);
  await button.events.click(); await ui.submit();
  assert.equal(ui.pending.length, 1);
});

test('restored authenticated page reacquires server state without replaying a save', async t => {
  const ui = await mount(t);
  ui.lifecycle.pageshow({persisted: false});
  assert.deepEqual(ui.navigations, []);
  const saving = ui.submit();
  ui.lifecycle.pagehide();
  ui.lifecycle.pageshow({persisted: true});
  assert.deepEqual(ui.navigations, ['https://app.test/saas/case/records']);
  await ui.submit();
  assert.equal(ui.pending.length, 1);
  ui.pending[0].resolve({text: async () => 'stale save response'});
  await saving;
  assert.equal(ui.message, undefined);
});

test('one in-flight mutation preserves form fields and blocks duplicate submits', async t => {
  const ui = await mount(t);
  const first = ui.submit();
  await ui.submit();
  assert.equal(ui.pending.length, 1);
  assert.deepEqual([...ui.pending[0].options.body], [['csrf', 'fixture-csrf'], ['title', 'Approved'], ['version', '1']]);
  assert.equal(ui.root.dataset.state, 'saving');
  assert.equal(ui.buttons[0].disabled, true);
  ui.pending[0].reject(new Error('Connection lost after possible commit'));
  await first;
  assert.equal(ui.root.dataset.state, 'error');
  assert.match(ui.message.textContent, /Reload the record/);
  await ui.submit();
  assert.equal(ui.pending.length, 1);
});

test('navigation aborts the request and late completion cannot change the old page', async t => {
  const ui = await mount(t);
  const saving = ui.submit();
  ui.lifecycle.pagehide();
  assert.equal(ui.pending[0].options.signal.aborted, true);
  ui.pending[0].resolve({text: async () => '<h1>Old response</h1>'});
  await saving;
  assert.equal(ui.message, undefined);
  assert.equal(ui.root.dataset.state, 'saving');
  await ui.submit();
  assert.equal(ui.pending.length, 1);
});

test('foreign response does not replace UI or authorize an automatic retry', async t => {
  const ui = await mount(t);
  const saving = ui.submit();
  ui.pending[0].resolve({url: 'https://foreign.test/saas/case/records',
    text: async () => '<h1>Foreign</h1>', headers: {get: () => 'text/html'}});
  await saving;
  assert.equal(ui.root.dataset.state, 'error');
  assert.equal(ui.buttons[0].disabled, true);
  await ui.submit();
  assert.equal(ui.pending.length, 1);
});
