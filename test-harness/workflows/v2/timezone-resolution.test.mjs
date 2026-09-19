import test from 'node:test';
import assert from 'node:assert/strict';
import {resolveLocalTime} from './timezone-resolution.mjs';

test('New York fold preserves both distinct instants', () => {
  assert.deepEqual(resolveLocalTime('2026-11-01T01:30', 'America/New_York'),
    ['2026-11-01T05:30:00.000Z', '2026-11-01T06:30:00.000Z']);
});
test('DST gaps cannot turn into reservations by normalization', () => {
  for (const zone of ['America/New_York', 'America/Vancouver']) {
    assert.deepEqual(resolveLocalTime('2026-03-08T02:30', zone), []);
  }
});
test('UTC and Vancouver conversion do not depend on the process timezone', () => {
  assert.deepEqual(resolveLocalTime('2026-11-01T05:30', 'UTC'), ['2026-11-01T05:30:00.000Z']);
  assert.deepEqual(resolveLocalTime('2026-10-31T22:30', 'America/Vancouver'), ['2026-11-01T05:30:00.000Z']);
});
test('invalid calendar dates and unknown zones are rejected', () => {
  for (const value of ['2026-02-30T01:30', '2026-11-01T25:00', '2026-1-1T01:00', '1999-01-01T00:00']) {
    assert.throws(() => resolveLocalTime(value, 'UTC'), RangeError);
  }
  assert.throws(() => resolveLocalTime('2026-01-01T00:00', 'Unknown/Zone'), RangeError);
});
