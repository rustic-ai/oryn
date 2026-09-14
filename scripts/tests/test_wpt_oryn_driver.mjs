import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

function adapter() {
  const context = vm.createContext({test_driver_internal:{}, legacyCalls:0});
  vm.runInContext('window=globalThis; __oryn_testdriver_actions=()=>{legacyCalls++}', context);
  vm.runInContext(readFileSync(new URL('../wpt-testdriver-oryn.js', import.meta.url), 'utf8'), context);
  return context;
}

test('approximate native action shim cannot establish WPT automation success', async () => {
  const context = adapter();
  await assert.rejects(context.test_driver_internal.action_sequence([]), /not implemented/);
  assert.equal(context.legacyCalls,0);
  assert.deepEqual(Array.from(context.__oryn_testdriver_log, e => e.outcome), ['started','unsupported']);
});

test('unknown native accessibility IDs fail instead of reporting completion', async () => {
  const context = adapter();
  await assert.rejects(context.test_driver_internal.get_accessibility_properties_for_accessibility_node('old'), /stale/);
  assert.deepEqual(Array.from(context.__oryn_testdriver_log, e => e.outcome), ['started','failed']);
});

test('versioned actions route to the origin realm or explicit context', async () => {
  const context = adapter();
  const calls = [];
  const child = {__oryn_testdriver_actions_version:1,
    __oryn_testdriver_actions:actions => {calls.push(actions); return Promise.resolve();}};
  const origin = {nodeType:1,ownerDocument:{defaultView:child}};
  const actions = [{type:'pointer',id:'mouse',actions:[{type:'pointerMove',origin,x:0,y:0}]}];
  await context.test_driver_internal.action_sequence(actions);
  await context.test_driver_internal.action_sequence([],child);
  assert.equal(calls[0],actions);
  assert.equal(calls.length,2);
  assert.equal(context.legacyCalls,0);
  assert.deepEqual(Array.from(context.__oryn_testdriver_log,e=>e.outcome),
    ['started','completed','started','completed']);
});

test('mixed-frame origins reject before dispatch into either realm', async () => {
  const context = adapter();
  const actions = [{type:'pointer',id:'mouse',actions:[{},{}].map(ownerDocument=>
    ({type:'pointerMove',origin:{nodeType:1,ownerDocument},x:0,y:0}))}];
  await assert.rejects(context.test_driver_internal.action_sequence(actions),/mixed-frame/);
  assert.equal(context.legacyCalls,0);
  assert.deepEqual(Array.from(context.__oryn_testdriver_log,e=>e.outcome),['started','unsupported']);
});
