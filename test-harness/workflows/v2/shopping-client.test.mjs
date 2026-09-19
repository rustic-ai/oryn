import test from 'node:test';
import assert from 'node:assert/strict';
import {hydrateShop, paymentMessageAccepted} from './shopping-client.mjs';

test('repeated BFCache parks abort requests and resume with read-only view refresh', () => {
  const listeners = () => {
    const callbacks = new Map();
    return {
      addEventListener(name, callback, options) {
        const entries = callbacks.get(name) || [];
        entries.push({callback, once: options?.once}); callbacks.set(name, entries);
      },
      dispatch(name, event) {
        for (const entry of [...(callbacks.get(name) || [])]) {
          if (entry.once) callbacks.set(name, callbacks.get(name).filter(value => value !== entry));
          entry.callback(event);
        }
      }
    };
  };
  const app = {dataset: {base: '/case'}, contains: () => true};
  const document = {...listeners(), querySelector: () => app};
  const window = {...listeners(), location: new URL('https://shop.test/case/cart')};
  const requests = [];
  hydrateShop(document, window, (url, options) => {
    requests.push({url: String(url), options});
    return new Promise(() => {});
  });
  const link = {href: 'https://shop.test/case/checkout'};
  document.dispatch('click', {target: {closest: () => link}, preventDefault() {}});
  assert.equal(requests.length, 1);
  assert.equal(app.dataset.state, 'loading');
  window.dispatch('pagehide', {persisted: true});
  assert.equal(requests[0].options.signal.aborted, true);
  window.dispatch('pageshow', {persisted: false});
  assert.equal(requests.length, 1);
  window.dispatch('pageshow', {persisted: true});
  assert.equal(requests.length, 2);
  assert.equal(requests[1].url, window.location.href);
  assert.equal(requests[1].options.method, undefined);
  assert.equal(requests[1].options.body, undefined);
  window.dispatch('pagehide', {persisted: true});
  assert.equal(requests[1].options.signal.aborted, true);
  window.dispatch('pageshow', {persisted: true});
  assert.equal(requests.length, 3);
  app.dataset.state = 'ready';
  window.dispatch('pageshow', {persisted: true});
  assert.equal(requests.length, 3);
});

test('framed payment requires exact origin, window, quote and receipt shape', () => {
  const source = {};
  const frame = {contentWindow: source};
  const status = {dataset: {origin: 'http://127.0.0.1:4101', quote: 'quote-a'}};
  const event = {source, origin: status.dataset.origin,
    data: {type: 'controlled-payment', quote: 'quote-a', receipt: 'a'.repeat(32)}};
  assert.equal(paymentMessageAccepted(event, frame, status), true);
  for (const changed of [{...event, source: {}}, {...event, origin: 'http://127.0.0.1:4102'},
    {...event, data: {...event.data, quote: 'foreign'}}, {...event, data: {...event.data, receipt: 'forged'}}]) {
    assert.equal(paymentMessageAccepted(changed, frame, status), false);
  }
  assert.equal(paymentMessageAccepted(event, null, status), false);
});
