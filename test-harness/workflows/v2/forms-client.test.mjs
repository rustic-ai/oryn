import test from 'node:test';
import assert from 'node:assert/strict';
import {hydrateApplication} from './forms-client.mjs';

function control(extra = {}) {
  return {events: {}, addEventListener(name, listener) { this.events[name] = listener; },
    reportValidity: () => true, ...extra};
}
function surface() {
  const email = control({value: 'zoe@example.test'});
  const form = control({dataset: {}, elements: {case_id: {value: 'a'.repeat(32)}}});
  const identity = control({hidden: false, querySelectorAll: () => [email]});
  const details = control({hidden: true});
  const next = control(), previous = control(), picker = control(), status = {textContent: ''};
  const window = control();
  const elements = {'#application': form, '#identity-step': identity, '#details-step': details,
    '#email': email, '#validation-status': status, '#next-step': next,
    '#previous-step': previous, '#role-picker': picker};
  return {document: {querySelector: selector => elements[selector], defaultView: window}, window, email, form, identity, details, next, previous, status};
}

test('parking or returning to identity retires validation without discarding form values', async () => {
  for (const retire of [ui => ui.window.events.pagehide(), ui => ui.previous.events.click()]) {
    const ui = surface();
    let finish, signal;
    hydrateApplication(ui.document, (url, options) => {
      signal = options.signal;
      return new Promise(resolve => { finish = resolve; });
    });
    const pending = ui.next.events.click();
    retire(ui);
    assert.equal(signal.aborted, true);
    finish({ok: true, json: async () => ({valid: true})});
    await pending;
    assert.equal(ui.form.dataset.state, 'editing');
    assert.equal(ui.identity.hidden, false);
    assert.equal(ui.details.hidden, true);
    assert.equal(ui.email.value, 'zoe@example.test');
    let prevented = false;
    ui.form.events.submit({preventDefault: () => { prevented = true; }});
    assert.equal(prevented, true);
    const fresh = ui.next.events.click();
    finish({ok: true, json: async () => ({valid: true})});
    await fresh;
    assert.equal(ui.form.dataset.state, 'review');
  }
});

test('changing email retires pending validation and prevents submit', async () => {
  const ui = surface();
  let finish;
  hydrateApplication(ui.document, () => new Promise(resolve => { finish = resolve; }));
  const pending = ui.next.events.click();
  ui.email.value = 'blocked@example.test';
  ui.email.events.input();
  finish({ok: true, json: async () => ({valid: true})});
  await pending;
  assert.equal(ui.details.hidden, true);
  let prevented = false;
  ui.form.events.submit({preventDefault: () => { prevented = true; }});
  assert.equal(prevented, true);
});

test('only actual eligible response opens review and allows native submission', async () => {
  const ui = surface();
  hydrateApplication(ui.document, async () => ({ok: true, json: async () => ({valid: true})}));
  await ui.next.events.click();
  assert.equal(ui.details.hidden, false);
  assert.equal(ui.form.dataset.state, 'review');
  let prevented = false;
  ui.form.events.submit({preventDefault: () => { prevented = true; }});
  assert.equal(prevented, false);
});

test('failed or rejected validation cannot manufacture a receipt', async () => {
  for (const response of [{ok: false}, {ok: true, json: async () => ({valid: false})}]) {
    const ui = surface();
    hydrateApplication(ui.document, async () => response);
    await ui.next.events.click();
    assert.equal(ui.details.hidden, true);
    assert.notEqual(ui.form.dataset.state, 'review');
  }
});
