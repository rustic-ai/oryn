// Parent-owned pre-author focus admission. No page globals, DOM changes, or focus emulation.
import {randomBytes} from 'node:crypto';
import {setTimeout as delay} from 'node:timers/promises';

export function createFocusBarrier({call, activate, targetId, timeout=5000, observe=()=>{}}) {
  const sourceURL=`oryn-native-focus-${randomBytes(24).toString('hex')}.js`;
  // Same-process subframes inherit registration but must never focus the top window.
  const source=`if(window===top){debugger;}\n//# sourceURL=${sourceURL}`;
  const contexts=new Map(), scripts=new Map(), admissions=new Map(), chains=new Map();
  let rootId, frame, epoch=0, registration, job, work, failure, stopped=false, paused=false;
  let expectedResume=false, releasing=false, queuedPause, initial=true, requestedUrl, requestedLoader, navigationIssued=false;
  let firstResolve, firstReject, navigationResolve;
  const navigation=new Promise(resolve=>{navigationResolve=resolve});
  const first=new Promise((resolve,reject)=>{firstResolve=resolve;firstReject=reject});
  first.catch(()=>{});
  function fail(error) {
    if(!failure){failure=error instanceof Error?error:new Error(String(error));firstReject(failure)}
  }
  function check(){if(stopped)throw Error('focus barrier stopped');if(failure)throw failure}
  async function bounded(work) {
    let timer;
    try{return await Promise.race([work,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('focus barrier deadline')),timeout)})])}
    finally{clearTimeout(timer)}
  }
  const fetchUrl=value=>{const url=new URL(value);url.hash='';return url.href};
  function invalidate(){epoch++;frame=undefined}
  async function admit(p) {
    check();const top=p.callFrames?.[0],script=scripts.get(top?.location?.scriptId);
    if(!script||script.url!==sourceURL) {
      // Debugger statements belonging to fixtures behave as with no debugger attached.
      // They never acquire focus authority or satisfy a document admission.
      if(p.reason!=='other'||p.hitBreakpoints?.length||!top?.callFrameId||!frame||![...admissions.values()].some(a=>a.epoch===epoch))throw Error('author pause before native focus admission');
      releasing=true;expectedResume=true;await call('Debugger.resume');check();paused=false;return;
    }
    if(p.reason!=='other'||p.hitBreakpoints?.length||!registration||!top.callFrameId||script.hasSourceURL!==true)throw Error('invalid native setup pause');
    const context=contexts.get(script.executionContextId),ownedFrame=frame,ownedEpoch=epoch;
    if(!context?.uniqueId||context.auxData?.isDefault!==true||context.auxData.frameId!==rootId||!ownedFrame||ownedFrame.parentId||!ownedFrame.loaderId)throw Error('native setup context or root missing');
    if(initial){await navigation;check()}
    if(initial&&(!navigationIssued||(requestedLoader&&ownedFrame.loaderId!==requestedLoader)))throw Error('initial focus navigation mismatch');
    if(initial&&ownedFrame.url!==requestedUrl){const chain=chains.get(ownedFrame.loaderId);if(!chain||!chain.valid||chain.first!==fetchUrl(requestedUrl)||chain.last!==fetchUrl(ownedFrame.url)||chain.frameId!==rootId)throw Error('redirect chain mismatch')}
    const ownedUrl=ownedFrame.url;
    const alive=()=>{check();if(!paused||frame!==ownedFrame||frame.url!==ownedUrl||epoch!==ownedEpoch||contexts.get(context.id)?.uniqueId!==context.uniqueId)throw Error('focus document retired')};
    const command=async(method,params)=>{alive();const value=await call(method,params);alive();return value};
    const actual=await command('Debugger.getScriptSource',{scriptId:top.location.scriptId});
    if(actual.scriptSource!==source)throw Error('native setup source mismatch');
    const read=async()=>{
      const result=await command('Debugger.evaluateOnCallFrame',{callFrameId:top.callFrameId,returnByValue:true,silent:true,timeout:Math.min(timeout,2000),expression:'({url:location.href,focused:document.hasFocus()})'});
      if(result.exceptionDetails||result.result?.value?.url!==ownedFrame.url||typeof result.result.value.focused!=='boolean')throw Error('native focus observation invalid');
      return result.result.value.focused;
    };
    const creationFocused=await read();observe('preauthor_focus_observed',{focused:creationFocused});alive();await activate(targetId);alive();
    const until=Date.now()+Math.min(timeout,2000);let focused=false;
    while(Date.now()<until){if(await read()){focused=true;break}await delay(20);alive()}
    if(!focused)throw Error('native pre-author focus unavailable');
    // addScriptToEvaluateOnNewDocument runs this exact source before frame scripts.
    // No page-authored counter or callback is trusted to establish this ordering.
    const admission={uniqueId:context.uniqueId,contextId:context.id,frameId:rootId,loaderId:ownedFrame.loaderId,epoch:ownedEpoch,url:ownedFrame.url,registration};
    admissions.set(context.id,admission);observe('preauthor_focus_admitted',admission);
    alive();initial=false;releasing=true;expectedResume=true;await call('Debugger.resume');check();
    paused=false;firstResolve(admission);
  }
  function startPause(p){
    paused=true;expectedResume=false;releasing=false;
    work=admit(p);work.catch(()=>{});
    job=bounded(work).catch(fail).finally(()=>{
      job=undefined;releasing=false;
      if(queuedPause&&!failure&&!stopped){const next=queuedPause;queuedPause=undefined;startPause(next)}
    });
  }
  return {
    async install(id) {
      rootId=id;await call('Debugger.enable');
      registration=(await call('Page.addScriptToEvaluateOnNewDocument',{source,runImmediately:false,includeCommandLineAPI:false})).identifier;
      if(!registration)throw Error('native focus registration missing');
    },
    beginNavigation(url){check();requestedUrl=url;navigationIssued=true},
    navigationReply(reply){try{check();if(reply.errorText||!reply.loaderId||(reply.frameId&&reply.frameId!==rootId))throw Error('initial navigation rejected');requestedLoader=reply.loaderId;navigationResolve();
      if(frame&&frame.loaderId!==requestedLoader)throw Error('initial navigation loader mismatch');
      const admitted=[...admissions.values()][0];if(admitted&&admitted.loaderId!==requestedLoader)throw Error('admitted initial loader mismatch');}catch(error){fail(error);throw error}},
    event(message) {
      if(stopped||message.sessionId)return;
      const p=message.params||{};
      try {
        switch(message.method){
          case 'Runtime.executionContextCreated': contexts.set(p.context.id,p.context);break;
          case 'Runtime.executionContextDestroyed': contexts.delete(p.executionContextId);admissions.delete(p.executionContextId);for(const [id,s] of scripts)if(s.executionContextId===p.executionContextId)scripts.delete(id);break;
          case 'Runtime.executionContextsCleared': contexts.clear();admissions.clear();scripts.clear();break;
          case 'Network.requestWillBeSent':
            if(p.type==='Document'&&p.frameId===rootId){const prior=chains.get(p.loaderId);chains.clear();chains.set(p.loaderId,{first:prior?.first||fetchUrl(p.request.url),last:fetchUrl(p.request.url),frameId:p.frameId,valid:prior?prior.valid&&p.redirectResponse?.url!==undefined&&fetchUrl(p.redirectResponse.url)===prior.last:true})}break;
          case 'Page.frameNavigated':if(p.frame.id===rootId){epoch++;frame={...p.frame};admissions.clear()}break;
          case 'Page.frameDetached':if(p.frameId===rootId)invalidate();break;
          case 'Page.frameStartedNavigating':if(p.frameId===rootId&&!['sameDocument','historySameDocument'].includes(p.navigationType))invalidate();break;
          case 'Page.navigatedWithinDocument':if(p.frameId===rootId&&frame){if(paused&&!releasing)throw Error('same-document navigation during focus barrier');frame.url=p.url;for(const a of admissions.values())a.url=p.url}break;
          case 'Debugger.scriptParsed':if(p.url===sourceURL)scripts.set(p.scriptId,p);break;
          case 'Debugger.resumed':paused=false;if(!expectedResume)throw Error('unexpected debugger resume');expectedResume=false;break;
          case 'Debugger.paused':
            if(job){if(!releasing||paused||queuedPause)throw Error('overlapping debugger pause');queuedPause=p;break}
            if(paused)throw Error('overlapping debugger pause');startPause(p);break;
        }
      }catch(error){fail(error)}
    },
    async ready(){try{await bounded(first);check()}catch(error){fail(error);throw error}},
    async idle(){while(job)await job;check()},
    async finish(id,uniqueId){while(job)await job;check();if(paused)throw Error('cannot remove setup while paused');
      if(!this.admitted(id,uniqueId))throw Error('focus completion identity retired');
      await call('Page.removeScriptToEvaluateOnNewDocument',{identifier:registration});check();
      if(!this.admitted(id,uniqueId))throw Error('focus completion identity changed during setup removal');
    },
    admitted(id,uniqueId){check();const a=admissions.get(id);return !!a&&contexts.get(id)?.uniqueId===uniqueId&&a.uniqueId===uniqueId&&a.epoch===epoch&&a.loaderId===frame?.loaderId},
    async stop(){stopped=true;navigationResolve();firstReject(Error('focus barrier stopped'));if(work)await work.catch(()=>{});if(job)await job},
    status(){return {stopped,paused,failed:failure?.message,registration,sourceURL}},
  };
}

// A failed pause must never be resumed by debugger detachment during cleanup.
export async function shutdownFocusSession({cancel,barrier,stopBrowser,detach,release}) {
  const errors=[];
  try{cancel()}catch(error){errors.push(error)}
  try{await barrier?.stop()}catch(error){errors.push(error)}
  let terminated=false;
  try{await stopBrowser();terminated=true}catch(error){errors.push(error)}
  if(terminated){
    try{detach()}catch(error){errors.push(error)}
    try{await release()}catch(error){errors.push(error)}
  }
  if(errors.length)throw new AggregateError(errors,'Chromium focus cleanup failed');
}
