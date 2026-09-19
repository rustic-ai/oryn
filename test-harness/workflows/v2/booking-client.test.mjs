import test from 'node:test';
import assert from 'node:assert/strict';

// Supply only the UI surface. The actual module handles async responses and
// controls whether a normal form submission can navigate to the backend.
class Control {
  constructor(value = '') { this.value = value; this.dataset = {}; this.listeners = {}; }
  addEventListener(type, callback) { this.listeners[type] = callback; }
  fire(type) { return this.listeners[type]?.({preventDefault() {}}); }
  replaceChildren() { this.value = ''; }
  append() {}
}

async function mount(t) {
  const form = new Control();
  form.elements = {local: new Control('2026-11-01T01:30'),
    zone: new Control('America/New_York'), appointment_option: new Control('video')};
  const status = new Control(), occurrence = new Control(), review = new Control(), refresh = new Control();
  const controls = {'#appointment-search': form, '#selection-status': status,
    '#occurrence': occurrence, '#review-appointment': review, '#refresh-availability': refresh};
  const pending = [], lifecycle = {};
  const globals = {document: {querySelector: selector => controls[selector]},
    Option: class {}, location: {href: ''},
    addEventListener: (name,callback) => { lifecycle[name] = callback; },
    fetch: (url,options) => new Promise((resolve, reject) => pending.push({resolve, reject,options}))};
  for (const [name, value] of Object.entries(globals)) {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, name);
    Object.defineProperty(globalThis, name, {value, configurable: true, writable: true});
    t.after(() => descriptor ? Object.defineProperty(globalThis, name, descriptor) : delete globalThis[name]);
  }
  await import(`./booking-client.mjs?test=${encodeURIComponent(t.name)}`);
  return {form, status, occurrence, review, refresh, pending, lifecycle, location: globals.location};
}
const tick = () => new Promise(resolve => setImmediate(resolve));
const available = {ok: true, json: async () => ({slots: [
  {slot: 'early', utc: '2026-11-01T05:30:00.000Z', available: true},
]})};
function choose(ui) { ui.occurrence.value = '2026-11-01T05:30:00.000Z'; ui.occurrence.fire('change'); }

test('restoration invalidates old slots and waits for new availability', async t => {
  const ui = await mount(t);
  ui.lifecycle.pagehide();
  assert.equal(ui.pending[0].options.signal.aborted, true);
  ui.lifecycle.pageshow({persisted: true});
  ui.pending[0].resolve(available); await tick(); choose(ui);
  assert.equal(ui.review.disabled, true);
  ui.pending[1].resolve({ok: true,json: async () => ({slots: []})}); await tick();
  choose(ui); ui.form.fire('submit');
  assert.equal(ui.location.href, '');
  ui.lifecycle.pagehide();
  assert.equal(ui.pending[1].options.signal.aborted, true);
  ui.lifecycle.pageshow({persisted: true});
  assert.equal(ui.pending.length, 3);
});

test('refresh retires cached availability and failure cannot permit a booking', async t => {
  const ui = await mount(t);
  ui.pending[0].resolve(available); await tick(); choose(ui);
  assert.equal(ui.review.disabled, false);
  const refreshing = ui.refresh.fire('click');
  ui.form.elements.local.fire('input'); choose(ui);
  assert.equal(ui.review.disabled, true);
  ui.form.fire('submit'); assert.equal(ui.location.href, '');
  ui.pending[1].reject(new Error('Network unavailable')); await refreshing;
  assert.equal(ui.status.dataset.state, 'error');
  choose(ui); ui.form.fire('submit'); assert.equal(ui.location.href, '');
});

test('late response from an older refresh cannot replace current unavailable slots', async t => {
  const ui = await mount(t);
  const refreshing = ui.refresh.fire('click');
  ui.pending[1].resolve({ok: true, json: async () => ({slots: []})}); await refreshing;
  ui.pending[0].resolve(available); await tick(); choose(ui);
  assert.equal(ui.review.disabled, true);
  ui.form.fire('submit'); assert.equal(ui.location.href, '');
});

test('successful submit carries client instant, wall time, zone and selected option', async t => {
  const ui = await mount(t);
  ui.pending[0].resolve(available); await tick(); choose(ui);
  ui.form.elements.appointment_option.value = 'phone';
  ui.form.fire('submit');
  const query = new URL(ui.location.href, 'https://fixture.test/booking/case/availability').searchParams;
  assert.equal(query.get('slot'), 'early');
  assert.equal(query.get('client_utc'), '2026-11-01T05:30:00.000Z');
  assert.equal(query.get('client_local'), '2026-11-01T01:30');
  assert.equal(query.get('zone'), 'America/New_York');
  assert.equal(query.get('appointment_option'), 'phone');
});
