// Historical adapter unit fixture: native authority is explicitly supplied by
// the test VM mocks. Production event/default coverage lives in native Rust tests.
import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFileSync} from 'node:fs';
const source=readFileSync(new URL('../../crates/oryn-native/src/native_actions.js',import.meta.url),'utf8');
test('element click uses the native pointer sequence and rejects interception',async()=>{
  const r=realm();
  await r.context.__oryn_testdriver_click(r.element);
  const click=r.events.find(e=>e.type==='click');
  assert.equal(click.pointerType,'mouse');assert.equal(click.pointerId,1);
  assert.equal(click.detail,1);assert.equal(click.buttons,0);
  r.setHit({nodeType:1});
  const count=r.events.length;
  await assert.rejects(r.context.__oryn_testdriver_click(r.element),/intercepted/);
  assert.equal(r.events.length,count);
});
function realm() {
  const events=[],handlers={},dragCalls=[];
  const doc={};
  class Node {contains(other){return this===other;}}
  class Element extends Node {getClientRects(){return [this.nativeRect];}}
  const element=Object.assign(new Element(),{nodeType:1,isConnected:true,ownerDocument:doc,
    nativeRect:{left:0,right:100,top:0,bottom:100}});
  let hit=element,acceptDrag=false;
  doc.activeElement=doc.body=element;doc.elementFromPoint=()=>hit;
  class Input {constructor(type,init){this.type=type;Object.assign(this,init)}}
  const context=vm.createContext({document:doc,innerWidth:800,innerHeight:600,performance,setTimeout,
    Node,Element,MouseEvent:Input,PointerEvent:Input,WheelEvent:Input,addEventListener:(name,fn)=>handlers[name]=fn,
    __oryn_pointer_boundary_snapshot:(old,target)=>[old?[old]:[],target?[target]:[],old===target?target:null],
    __oryn_native_drag_input:(operation,target,x,y,flags)=>{
      dragCalls.push({operation,target,x,y,flags});
      return acceptDrag&&(operation==='begin'||operation==='advance');
    },
    __oryn_focus_from_pointer:()=>{},
    __oryn_dispatch_user_input:(target,type,passive,event)=>{events.push(event);return true},
    __oryn_testdriver_keyboard:(target,key,down)=>{
      const name=key==='\uE008'?'Shift':key;
      if(down)context.__oryn_testdriver_modifiers.add(name);else context.__oryn_testdriver_modifiers.delete(name);
      events.push({type:down?'keydown':'keyup',key});
    }});
  vm.runInContext(source,context);
  return {context,events,handlers,element,doc,dragCalls,setHit:value=>hit=value,
    enableDrag:()=>acceptDrag=true,send:context.__oryn_testdriver_actions};
}
const mouse=actions=>[{id:'mouse',type:'pointer',actions}];
test('invalid late variant rejects without partial input',async()=>{
  const r=realm();await assert.rejects(r.send(mouse([{type:'pointerDown',button:0},{type:'scroll'}])),/not implemented/);
  assert.equal(r.events.length,0);
});
test('coordinates, buttons, click count and modifiers survive ticks',async()=>{
  const r=realm();
  await r.send([{id:'keys',type:'key',actions:[{type:'keyDown',value:'\uE008'}]},
    ...mouse([{type:'pointerMove',origin:r.element,x:0,y:0},
      {type:'pointerDown',button:0},{type:'pointerUp',button:0},
      {type:'pointerDown',button:0},{type:'pointerUp',button:0}])]);
  const clicks=r.events.filter(e=>e.type==='click');
  assert.deepEqual(clicks.map(e=>[e.clientX,e.clientY,e.buttons,e.detail,e.shiftKey]),[[50,50,0,1,true],[50,50,0,2,true]]);
  assert.equal(r.events.find(e=>e.type==='mousedown').buttons,1);
  assert.equal(r.events.filter(e=>e.type==='dblclick').length,1);
});
test('tick pauses use wall time and do not add per-source durations',async()=>{
  const r=realm(),start=performance.now();
  await r.send([{id:'a',type:'none',actions:[{type:'pause',duration:30}]},
    {id:'b',type:'none',actions:[{type:'pause',duration:30}]}]);
  assert.ok(performance.now()-start>=29);
});
test('element positions are resolved at execution time',async()=>{
  const r=realm();
  r.element.nativeRect={left:-100,right:200,top:-100,bottom:100};
  await r.send(mouse([{type:'pointerMove',origin:r.element,x:5,y:10}]));
  const e=r.events.find(e=>e.type==='mousemove');assert.deepEqual([e.clientX,e.clientY],[105,60]);
});
test('boundary enter and leave events are non-composed',async()=>{
  const r=realm();
  await r.send(mouse([{type:'pointerMove',x:10,y:10}]));
  for(const type of ['mouseenter','pointerenter']) {
    const event=r.events.find(e=>e.type===type);
    assert.equal(event.composed,false);assert.equal(event.bubbles,false);assert.equal(event.cancelable,false);
  }
  assert.equal(r.events.find(e=>e.type==='mouseover').composed,true);
});
test('failure releases held input; following sequence can run',async()=>{
  const r=realm();
  await assert.rejects(r.send(mouse([{type:'pointerDown',button:0},{type:'pointerMove',x:-1,y:0}])),/out of bounds/);
  assert.ok(r.events.some(e=>e.type==='mouseup'&&e.buttons===0));
  assert.ok(!r.events.some(e=>e.type==='click'));
  await r.send(mouse([{type:'pointerMove',x:1,y:2}]));
});
test('held key is released before the next sequence',async()=>{
  const r=realm();await r.send([{id:'k',type:'key',actions:[{type:'keyDown',value:'\uE008'}]}]);
  assert.ok(r.context.__oryn_testdriver_modifiers.has('Shift'));
  await r.send([]);assert.equal(r.context.__oryn_testdriver_modifiers.size,0);
  assert.equal(r.events.at(-1).type,'keyup');
});
test('retired realm rejects pending work without dispatch into replacement',async()=>{
  const r=realm();const pending=r.send([{id:'pause',type:'none',actions:[{type:'pause',duration:20}]}]);
  r.handlers.pagehide();await assert.rejects(pending,/stale/);assert.equal(r.events.length,0);
});
test('request snapshots prevent queued mutation',async()=>{
  const r=realm(),actions=mouse([{type:'pointerMove',x:10,y:20}]);
  const pending=r.send(actions);actions[0].actions[0].x=500;
  await pending;assert.equal(r.events.find(e=>e.type==='mousemove').clientX,10);
});
test('native drag routing consumes the mouse gesture and carries physical coordinates',async()=>{
  const r=realm();r.enableDrag();
  await r.send(mouse([{type:'pointerMove',x:10,y:10},{type:'pointerDown',button:0},
    {type:'pointerMove',x:30,y:40},{type:'pointerUp',button:0}]));
  assert.deepEqual(r.dragCalls.map(call=>call.operation),['begin','advance','drop']);
  assert.deepEqual(r.dragCalls.map(call=>[call.x,call.y]),[[30,40],[30,40],[30,40]]);
  assert.equal(r.events.filter(event=>event.type==='pointercancel').length,1);
  assert.equal(r.events.filter(event=>event.type==='click'||event.type==='mouseup').length,0);
});
test('Escape routes cancellation once and suppresses subsequent drop and click',async()=>{
  const r=realm();r.enableDrag();
  await r.send([...mouse([{type:'pointerMove',x:10,y:10},{type:'pointerDown',button:0},
    {type:'pointerMove',x:30,y:40},{type:'pause'},{type:'pointerUp',button:0}]),
    {id:'keyboard',type:'key',actions:[{type:'pause'},{type:'pause'},{type:'pause'},
      {type:'keyDown',value:'\uE00C'},{type:'keyUp',value:'\uE00C'}]}]);
  assert.deepEqual(r.dragCalls.map(call=>call.operation),['begin','advance','cancel']);
  assert.equal(r.events.filter(event=>event.type==='click').length,0);
});
