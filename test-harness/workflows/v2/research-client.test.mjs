import test from 'node:test';
import assert from 'node:assert/strict';
import {hydrateResearch} from './research-client.mjs';

function surface() {
  const events = {}, eventOptions = {};
  const app = {dataset: {base: '/research/abc'}};
  const form = {elements: {q: {value: 'energy'}}, addEventListener() {}, reportValidity: () => true};
  const results = {innerHTML: 'SSR sources', addEventListener() {}};
  const status = {textContent: ''};
  const elements = {'#research-app': app, '#search-form': form, '#results': results, '#search-status': status};
  const routes = [];
  const window = {location: {href: 'http://fixture/research/abc/start', origin: 'http://fixture'},
    history: {pushState: (_, __, url) => routes.push(String(url))},
    addEventListener: (name, callback, options) => { events[name] = callback; eventOptions[name] = options; }};
  return {document: {querySelector: name => elements[name]}, window, app, results, routes, events, eventOptions};
}

test('interrupted restored search reacquires current route without pushing history', async () => {
  const ui = surface(), requests = [];
  ui.window.location.href = 'http://fixture/research/abc/search?q=restored&page=2';
  const client = hydrateResearch(ui.document, ui.window, async () => ({
    searchRoute: (_, query, page) => ({url: `${query}:${page}`, api: `${query}:${page}`}),
    fetchResults: (api, options) => new Promise(resolve => requests.push({api, options, resolve}))
  }));
  const old = client.navigate('interrupted', '3');
  await Promise.resolve();
  assert.notEqual(ui.eventOptions.pagehide?.once, true);
  ui.events.pagehide();
  assert.equal(requests[0].options.signal.aborted, true);
  const restore = ui.events.pageshow({persisted: true});
  await Promise.resolve();
  assert.equal(requests[1].api, 'restored:2');
  requests[0].resolve('old facts'); await old;
  assert.equal(ui.results.innerHTML, 'SSR sources');
  ui.events.pagehide();
  assert.equal(requests[1].options.signal.aborted, true);
  const secondRestore = ui.events.pageshow({persisted: true});
  await Promise.resolve();
  requests[1].resolve('first restore facts'); await restore;
  requests[2].resolve('current facts'); await secondRestore;
  assert.equal(ui.results.innerHTML, 'current facts');
  assert.equal(ui.app.dataset.page, '2');
  assert.deepEqual(ui.routes, []);
  ui.events.pageshow({persisted: true});
  assert.equal(requests.length, 3);
});

test('late response cannot overwrite a newer navigation', async () => {
  const ui = surface();
  const requests = [];
  const client = hydrateResearch(ui.document, ui.window, async () => ({
    searchRoute: (_, query) => ({url: query, api: query}),
    fetchResults: (api, options) => new Promise(resolve => requests.push({api, options, resolve}))
  }));
  const first = client.navigate('old', '1');
  await Promise.resolve();
  const second = client.navigate('current', '2');
  await Promise.resolve();
  assert.equal(requests[0].options.signal.aborted, true);
  requests[1].resolve('current facts');
  await second;
  requests[0].resolve('retired facts');
  await first;
  assert.equal(ui.results.innerHTML, 'current facts');
  assert.deepEqual(ui.routes, ['current']);
  assert.equal(ui.app.dataset.page, '2');
});

test('page retirement revokes pending response publication', async () => {
  const ui = surface();
  let finish;
  const client = hydrateResearch(ui.document, ui.window, async () => ({
    searchRoute: () => ({url: 'next', api: 'next'}),
    fetchResults: () => new Promise(resolve => { finish = resolve; })
  }));
  const pending = client.navigate('energy', '1');
  await Promise.resolve();
  ui.events.pagehide();
  finish('late sources');
  await pending;
  assert.equal(ui.results.innerHTML, 'SSR sources');
  assert.deepEqual(ui.routes, []);
});

test('chunk failure is visible and does not replace existing sources', async () => {
  const ui = surface();
  const client = hydrateResearch(ui.document, ui.window, async () => { throw new Error('chunk unavailable'); });
  await client.navigate('energy', '1');
  assert.equal(ui.app.dataset.state, 'error');
  assert.equal(ui.results.innerHTML, 'SSR sources');
});
