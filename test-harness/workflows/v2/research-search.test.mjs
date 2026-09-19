import test from 'node:test';
import assert from 'node:assert/strict';
import {fetchResults, searchRoute} from './research-search.mjs';

test('query delimiters stay data and the request stays on the current case', () => {
  const {url, api} = searchRoute('/research/abc', 'a&other=secret', '2', 'http://127.0.0.1:4100');
  assert.equal(url.searchParams.get('q'), 'a&other=secret');
  assert.equal(url.searchParams.has('other'), false);
  assert.equal(api.pathname, '/research/abc/api');
  assert.equal(api.origin, url.origin);
});

test('failed responses cannot be rendered as successful sources', async () => {
  await assert.rejects(fetchResults('http://fixture/api', {fetcher: async () => ({ok: false, status: 503})}), /503/);
});

test('abort and credentials are forwarded to the native fetch implementation', async () => {
  const controller = new AbortController();
  const text = await fetchResults('http://fixture/api', {signal: controller.signal,
    fetcher: async (url, options) => {
      assert.equal(options.signal, controller.signal);
      assert.equal(options.credentials, 'same-origin');
      return {ok: true, text: async () => 'controlled sources'};
    }});
  assert.equal(text, 'controlled sources');
});
