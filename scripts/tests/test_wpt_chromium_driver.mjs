import test from 'node:test';
import assert from 'node:assert/strict';
import {dispatchDriver, settleAutomation} from '../wpt-chromium-driver.mjs';

test('completion waits for input and page reply, including newly queued work', async () => {
  const log=[];
  let finish;
  let queue=new Promise(resolve=>{finish=()=>{log.push('first');resolve()}});
  let reads=0;
  const result=settleAutomation(()=>queue,async()=>{
    if (++reads===1) queue=Promise.resolve().then(()=>log.push('second'));
    return [...log];
  },()=>true,Date.now()+1000);
  await Promise.resolve();assert.equal(reads,0);
  finish();
  assert.deepEqual(await result,['first','second']);assert.equal(reads,2);
});

test('completion drain rejects retirement, disappearance and hanging input', async () => {
  await assert.rejects(settleAutomation(()=>Promise.resolve(),async()=>({}),()=>false,Date.now()+1000),/context_retired/);
  const queue=Promise.resolve();
  await assert.rejects(settleAutomation(()=>queue,async()=>null,()=>true,Date.now()+1000),/disappeared/);
  await assert.rejects(settleAutomation(()=>new Promise(()=>{}),async()=>({}),()=>true,Date.now()+10),/drain_timeout/);
  let live=true;
  await assert.rejects(settleAutomation(()=>queue,async()=>{live=false;return {}},()=>live,Date.now()+1000),/context_retired/);
});

test('body send_keys preserves focus and sends native Tab metadata', async () => {
  for (const body of [true,false]) {
    const calls=[];
    await dispatchDriver(async (method,params) => {
      calls.push([method,params]);
      if(method==='Runtime.evaluate')return {result:{objectId:'target'}};
      if(method==='Runtime.callFunctionOn')return {result:{value:body}};
      return {};
    },1,{operation:'send_keys',id:1,args:{keys:'\uE004'}},()=>true);
    assert.equal(calls.some(([method])=>method==='DOM.focus'),!body);
    const keys=calls.filter(([method])=>method==='Input.dispatchKeyEvent').map(([,p])=>p);
    assert.deepEqual(keys.map(p=>[p.type,p.key,p.code,p.windowsVirtualKeyCode]),
      [['keyDown','Tab','Tab',9],['keyUp','Tab','Tab',9]]);
  }
});

test('unverified keyboard target fails before input', async () => {
  const calls=[];
  await assert.rejects(dispatchDriver(async (method,params)=>{
    calls.push(method);
    return method==='Runtime.evaluate'?{result:{objectId:'target'}}:{};
  },1,{operation:'send_keys',id:1,args:{keys:'\uE004'}},()=>true),/target_state_unverified/);
  assert.ok(!calls.includes('Input.dispatchKeyEvent'));
});

test('send_keys modifiers reject before any partial input', async () => {
  const calls = [];
  await assert.rejects(dispatchDriver(async (...args) => calls.push(args), 1,
    {operation:'send_keys', id:1, args:{keys:'a\uE008b'}}, () => true), /unsupported/);
  assert.deepEqual(calls, []);
});

test('unsupported action variant produces no partial input', async () => {
  const calls = [];
  await assert.rejects(dispatchDriver(async (...args) => calls.push(args), 1,
    {operation:'action_sequence', id:1, args:{actions:[{id:'touch',type:'pointer',
      parameters:{pointerType:'touch'},actions:[]}]}}, () => true), /unsupported/);
  assert.deepEqual(calls, []);
});

test('successive clicks increment counts and movement resets them', async () => {
  const events=[];
  const actions=[];
  for(const x of [5,5,10,10,10]) actions.push({type:'pointerMove',x,y:5},
    {type:'pointerDown',button:0},{type:'pointerUp',button:0});
  await dispatchDriver(async (method,params)=>{
    if(method==='Input.dispatchMouseEvent')events.push(params);
    return {};
  },1,{id:1,operation:'action_sequence',args:{actions:[{type:'pointer',id:'mouse',actions}]}},()=>true,{});
  assert.deepEqual(events.filter(e=>e.type==='mouseReleased').map(e=>e.clickCount),[1,2,1,2,3]);
});

test('stale execution cannot dispatch input', async () => {
  const calls = [];
  await assert.rejects(dispatchDriver(async (...args) => calls.push(args), 1,
    {operation:'click',id:1}, () => false), /stale_context/);
  assert.deepEqual(calls, []);
});

test('click uses native input and releases remote object', async () => {
  const calls = [];
  await dispatchDriver(async (method, params) => {
    calls.push([method, params]);
    if (method === 'Runtime.evaluate') return {result:{objectId:'element'}};
    if (method === 'DOM.getContentQuads') return {quads:[[0,0,20,0,20,20,0,20]]};
    if (method === 'Page.getLayoutMetrics') return {cssLayoutViewport:{clientWidth:800,clientHeight:600}};
    return {};
  }, 1, {operation:'click',id:1}, () => true);
  assert.deepEqual(calls.filter(([m]) => m === 'Input.dispatchMouseEvent').map(([,p]) => p.type),
    ['mouseMoved','mousePressed','mouseReleased']);
  assert.equal(calls.at(-1)[0], 'Runtime.releaseObject');
});

test('element origins use clipped viewport center and propagate modifiers', async () => {
  const calls = [];
  await dispatchDriver(async (method, params) => {
    calls.push([method,params]);
    if (method === 'Runtime.evaluate') return {result:{objectId:'origin'}};
    if (method === 'DOM.getContentQuads') return {quads:[[-100,-25,100,-25,100,50,-100,50]]};
    if (method === 'Page.getLayoutMetrics') return {cssLayoutViewport:{clientWidth:800,clientHeight:600}};
    return {};
  }, 1, {id:1,operation:'action_sequence',args:{actions:[
    {id:'key',type:'key',actions:[{type:'keyDown',value:'\uE008'},{type:'pause'},{type:'pause'}]},
    {id:'pointer',type:'pointer',actions:[{type:'pointerMove',origin:{element:'1:0'},x:0,y:0},{type:'pointerDown',button:0},{type:'pointerUp',button:0}]}]}}, () => true);
  const events = calls.filter(([m]) => m === 'Input.dispatchMouseEvent').map(([,p]) => p);
  assert.equal(events[0].x, 50);
  assert.equal(events[0].y, 25);
  assert.ok(events.every(e => e.modifiers === 8));
});

test('accessibility fields match pinned WPT serialization, stale IDs reject', async () => {
  const state = {};
  const call = async method => method === 'Runtime.evaluate' ? {result:{objectId:'ax'}} :
    method === 'Accessibility.getPartialAXTree' ? {nodes:[{nodeId:'node',parentId:'parent',childIds:['child'],
      role:{value:'checkbox'},name:{value:'Choice'},properties:[{name:'checked',value:{value:'true'}},{name:'internal',value:{value:'omit'}}]}]} : {};
  const result = await dispatchDriver(call, 1, {id:1,operation:'get_accessibility_properties_for_element'}, () => true, state);
  assert.deepEqual(result,{accessibilityId:'node',parent:'parent',children:['child'],role:'checkbox',label:'Choice',checked:'true'});
  await assert.rejects(dispatchDriver(call, 1,{id:2,operation:'get_accessibility_properties_for_accessibility_node',args:{id:'node'}},()=>true,{}), /stale_accessibility_node/);
});

test('pointer and keyboard ticks finish with all input released', async () => {
  const calls = [];
  await dispatchDriver(async (method, params) => { calls.push([method, params]); return {}; }, 1,
    {operation:'action_sequence',id:1,args:{actions:[{id:'mouse',type:'pointer',actions:[
      {type:'pointerMove',x:10,y:20},{type:'pointerDown',button:0},{type:'pointerUp',button:0}]}]}}, () => true);
  assert.deepEqual(calls.map(([,p]) => p.type), ['mouseMoved','mousePressed','mouseReleased']);
});
