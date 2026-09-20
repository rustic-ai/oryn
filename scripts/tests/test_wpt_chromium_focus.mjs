import test from 'node:test';
import assert from 'node:assert/strict';
import {createFocusBarrier,shutdownFocusSession} from '../wpt-chromium-focus.mjs';

async function fixture(options={}) {
  const calls=[];let source,controller;
  const event=(method,params={})=>controller.event({method,params});
  const call=async(method,params={})=>{
    calls.push({method,params});
    if(options.intercept){const replacement=await options.intercept(method,params,event);if(replacement!==undefined)return replacement}
    if(method==='Page.addScriptToEvaluateOnNewDocument'){source=params.source;return {identifier:'native-registration'}}
    if(method==='Debugger.getScriptSource')return {scriptSource:options.badSource?'debugger;':source};
    if(method==='Debugger.evaluateOnCallFrame')return {result:{value:{url:options.finalUrl||'https://fixture.test/test.html',focused:options.focus!==false}}};
    if(method==='Debugger.resume')event('Debugger.resumed');
    return {};
  };
  controller=createFocusBarrier({call,targetId:'owned-target',timeout:options.timeout||60,
    activate:async target=>{calls.push({method:'activate',params:{target}});await options.onActivate?.(event)}});
  await controller.install('root');
  const context={id:7,uniqueId:'native-generation',auxData:{isDefault:true,frameId:'root'}};
  const commit=(loader='loader')=>{
    event('Page.frameNavigated',{frame:{id:'root',loaderId:loader,url:options.finalUrl||'https://fixture.test/test.html'}});
    event('Runtime.executionContextCreated',{context:{...context,...options.context}});
    event('Debugger.scriptParsed',{scriptId:'script',executionContextId:7,url:source.split('sourceURL=')[1],hasSourceURL:true});
  };
  controller.beginNavigation(options.requestedUrl||'https://fixture.test/test.html');
  if(options.redirect){event('Network.requestWillBeSent',{type:'Document',frameId:'root',loaderId:'loader',request:{url:'https://fixture.test/test.html'}});event('Network.requestWillBeSent',{type:'Document',frameId:'root',loaderId:'loader',request:{url:options.finalUrl.split('#')[0],urlFragment:options.finalUrl.includes('#')?'#'+options.finalUrl.split('#')[1]:undefined},redirectResponse:{url:'https://fixture.test/test.html'}})}
  commit(options.loader||'loader');
  const pause=()=>event('Debugger.paused',{reason:'other',callFrames:[{callFrameId:'frame',location:{scriptId:'script'}}]});
  return {controller,calls,event,pause,commit,reply:()=>controller.navigationReply({loaderId:'loader'}),context};
}

test('native setup admission precedes resume and preserves exact registration/context',async()=>{
  const f=await fixture();f.reply();f.pause();await f.controller.ready();await f.controller.idle();
  assert.equal(f.controller.admitted(7,'native-generation'),true);
  assert.equal(f.controller.admitted(7,'foreign-generation'),false);
  assert.equal(f.calls.find(c=>c.method==='activate').params.target,'owned-target');
  const methods=f.calls.map(c=>c.method);
  assert.ok(methods.indexOf('Debugger.getScriptSource')<methods.indexOf('activate'));
  assert.ok(methods.indexOf('activate')<methods.indexOf('Debugger.resume'));
  const registration=f.calls.find(c=>c.method==='Page.addScriptToEvaluateOnNewDocument').params;
  assert.equal(registration.runImmediately,false);assert.equal(registration.worldName,undefined);
  assert.match(registration.source,/^if\(window===top\)\{debugger;\}/);
  await f.controller.stop();
});

for(const [name,options] of [
  ['foreign frame',{context:{auxData:{isDefault:true,frameId:'foreign'}}}],
  ['isolated world',{context:{auxData:{isDefault:false,frameId:'root'}}}],
  ['missing unique identity',{context:{uniqueId:undefined}}],
  ['source mismatch',{badSource:true}],
  ['native focus false',{focus:false,timeout:15}],
])test(`focus admission rejects ${name} without resume`,async()=>{
  const f=await fixture(options);f.reply();f.pause();await assert.rejects(f.controller.ready());
  await assert.rejects(f.controller.idle());assert.equal(f.calls.some(c=>c.method==='Debugger.resume'),false);await f.controller.stop();
});

test('requested loader mismatch rejects before admission',async()=>{
  const f=await fixture({loader:'foreign-loader'});assert.throws(f.reply,/loader mismatch/);await f.controller.stop();
});

for(const [name,method,params] of [
  ['context retirement','Runtime.executionContextDestroyed',{executionContextId:7}],
  ['root replacement','Page.frameNavigated',{frame:{id:'root',loaderId:'replacement',url:'https://fixture.test/test.html'}}],
  ['detach','Page.frameDetached',{frameId:'root'}],
  ['unexpected resume','Debugger.resumed',{}],
  ['same-document change','Page.navigatedWithinDocument',{frameId:'root',url:'https://fixture.test/test.html#changed'}],
  ['second pause','Debugger.paused',{reason:'other',callFrames:[]}],
])test(`focus admission rejects ${name} during activation`,async()=>{
  const f=await fixture({onActivate:event=>event(method,params)});f.reply();f.pause();
  await assert.rejects(f.controller.ready());assert.equal(f.calls.some(c=>c.method==='Debugger.resume'),false);await f.controller.stop();
});

test('deadline does not release a late paused command or revive admission',async()=>{
  let finish;
  const f=await fixture({timeout:15,intercept:async method=>{
    if(method==='Debugger.getScriptSource')return await new Promise(resolve=>{finish=resolve});
  }});
  f.reply();f.pause();await assert.rejects(f.controller.ready(),/deadline/);
  finish({scriptSource:'late'});await new Promise(resolve=>setTimeout(resolve,0));
  await f.controller.stop();assert.equal(f.calls.some(c=>c.method==='Debugger.resume'),false);
});

test('same-document navigation retains admission; replacement requires new native setup',async()=>{
  const f=await fixture();f.reply();f.pause();await f.controller.ready();await f.controller.idle();
  f.event('Page.frameStartedNavigating',{frameId:'root',navigationType:'sameDocument'});
  f.event('Page.navigatedWithinDocument',{frameId:'root',url:'https://fixture.test/test.html#next'});
  assert.equal(f.controller.admitted(7,'native-generation'),true);
  f.event('Page.frameNavigated',{frame:{id:'root',loaderId:'next',url:'https://fixture.test/test.html'}});
  assert.equal(f.controller.admitted(7,'native-generation'),false);
  f.pause();await f.controller.idle();assert.equal(f.controller.admitted(7,'native-generation'),true);await f.controller.stop();
});

test('child-session lifecycle cannot replace main-root authority',async()=>{
  const f=await fixture();f.reply();f.pause();await f.controller.ready();await f.controller.idle();
  f.controller.event({sessionId:'foreign',method:'Runtime.executionContextsCleared'});
  assert.equal(f.controller.admitted(7,'native-generation'),true);await f.controller.stop();
});

test('fixture debugger pauses resume without refocusing or new admission',async()=>{
  const f=await fixture();f.reply();f.pause();await f.controller.ready();await f.controller.idle();
  const activations=f.calls.filter(c=>c.method==='activate').length;
  f.event('Debugger.scriptParsed',{scriptId:'author',executionContextId:7,url:'https://fixture.test/author.js'});
  f.event('Debugger.paused',{reason:'other',callFrames:[{callFrameId:'authorframe',location:{scriptId:'author'}}]});
  await f.controller.idle();assert.equal(f.calls.filter(c=>c.method==='activate').length,activations);await f.controller.stop();
});

test('cleanup settles barrier and terminates browser before detaching either socket',async()=>{
  const calls=[];
  await shutdownFocusSession({cancel(){calls.push('cancel')},barrier:{async stop(){calls.push('barrier')}},
    async stopBrowser(){calls.push('browser')},detach(){calls.push('detach')},async release(){calls.push('profile')}});
  assert.deepEqual(calls,['cancel','barrier','browser','detach','profile']);
});

test('cleanup failures still attempt Chrome termination and retain live-debugger ownership',async()=>{
  const calls=[];
  await assert.rejects(shutdownFocusSession({cancel(){throw Error('cancel')},barrier:{async stop(){throw Error('barrier')}},
    async stopBrowser(){calls.push('browser');throw Error('still alive')},detach(){calls.push('detach')},release(){calls.push('profile')}}),AggregateError);
  assert.deepEqual(calls,['browser']);
});


test('initial redirects require native request chain and matching committed loader',async()=>{
  const good=await fixture({redirect:true,finalUrl:'https://other.test/final.html'});good.reply();good.pause();await good.controller.ready();await good.controller.idle();await good.controller.stop();
  const bad=await fixture({finalUrl:'https://other.test/final.html'});bad.reply();bad.pause();await assert.rejects(bad.controller.ready(),/redirect chain/);assert.equal(bad.calls.some(c=>c.method==='Debugger.resume'),false);await bad.controller.stop();
});

test('ready deadline latches failure before a delayed first pause',async()=>{
  const f=await fixture({timeout:10});f.reply();await assert.rejects(f.controller.ready(),/deadline/);
  f.pause();await assert.rejects(f.controller.idle(),/deadline/);
  assert.equal(f.calls.some(c=>c.method==='Debugger.resume'),false);await f.controller.stop();
});

test('resumed then author pause before prior resume reply is serialized without refocus',async()=>{
  let first=true;
  const f=await fixture({intercept:async(method,params,event)=>{
    if(method==='Debugger.resume'&&first){first=false;event('Debugger.resumed');
      event('Debugger.paused',{reason:'other',callFrames:[{callFrameId:'author-frame',location:{scriptId:'author'}}]});
      await Promise.resolve();return {};
    }
  }});
  f.reply();f.pause();await f.controller.ready();await f.controller.idle();
  assert.equal(f.calls.filter(c=>c.method==='Debugger.resume').length,2);
  assert.equal(f.calls.filter(c=>c.method==='activate').length,1);await f.controller.stop();
});

test('successful completion removes only its exact setup and rechecks retirement',async()=>{
  const f=await fixture();f.reply();f.pause();await f.controller.ready();await f.controller.idle();
  await f.controller.finish(7,'native-generation');
  assert.deepEqual(f.calls.find(c=>c.method==='Page.removeScriptToEvaluateOnNewDocument').params,{identifier:'native-registration'});await f.controller.stop();
  const stale=await fixture({intercept:async(method,params,event)=>{
    if(method==='Page.removeScriptToEvaluateOnNewDocument')event('Runtime.executionContextDestroyed',{executionContextId:7});
  }});stale.reply();stale.pause();await stale.controller.ready();await stale.controller.idle();
  await assert.rejects(stale.controller.finish(7,'native-generation'),/identity changed/);await stale.controller.stop();
});


test('stopping retained pending work prevents its late continuation from resuming',async()=>{
  let finish;
  const f=await fixture({intercept:async method=>{if(method==='Debugger.getScriptSource')return await new Promise(resolve=>{finish=resolve})}});
  f.reply();f.pause();await Promise.resolve();await Promise.resolve();
  const stopped=f.controller.stop();finish({scriptSource:'late'});await stopped;
  assert.equal(f.calls.some(c=>c.method==='Debugger.resume'),false);
});


test('redirect transport chains omit fragments while native location preserves final fragment',async()=>{
  const f=await fixture({redirect:true,requestedUrl:'https://fixture.test/test.html#start',finalUrl:'https://other.test/final.html#end'});
  f.reply();f.pause();await f.controller.ready();await f.controller.idle();assert.equal(f.controller.admitted(7,'native-generation'),true);await f.controller.stop();
});
