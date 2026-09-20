/* Historical WPT adapter only. Production uses authenticated typed native input.
 * This file is installed exclusively by native historical-test-input opt-in. */
(() => {
  const owner = document;
  const boundarySnapshot = globalThis.__oryn_pointer_boundary_snapshot;
  const dispatchInput = globalThis.__oryn_dispatch_user_input;
  const dragInput = globalThis.__oryn_native_drag_input;
  const focusPointer = globalThis.__oryn_focus_from_pointer;
  const dispatchKeyboard = globalThis.__oryn_testdriver_keyboard;
  const NativePointerEvent = globalThis.PointerEvent, NativeMouseEvent = globalThis.MouseEvent, NativeWheelEvent = globalThis.WheelEvent;
  const hitTest = owner.elementFromPoint.bind(owner);
  const rects = Function.prototype.call.bind(Element.prototype.getClientRects);
  const contains = Function.prototype.call.bind(Node.prototype.contains);
  let retired = false, queue = Promise.resolve();
  const pointer = {x:0,y:0,target:null,buttons:new Set(),down:new Map(),last:null,count:0,suppressMouse:false};
  let dragCandidate=null, dragging=false, dragConsumed=false;
  const keys = new Set();
  const cancelInputs = [];
  const modifiers = globalThis.__oryn_testdriver_modifiers ||= new Set();
  const special = {'\uE003':'Backspace','\uE004':'Tab','\uE007':'Enter',
    '\uE008':'Shift','\uE009':'Control','\uE00A':'Alt','\uE00C':'Escape','\uE010':'End','\uE011':'Home',
    '\uE012':'ArrowLeft','\uE013':'ArrowUp','\uE014':'ArrowRight','\uE015':'ArrowDown',
    '\uE017':'Delete','\uE03D':'Meta'};
  const unsupported = () => { throw new Error('not implemented: action variant'); };
  const check = () => { if (retired || document !== owner) throw new Error('stale action context'); };
  const sleep = ms => new Promise(resolve => setTimeout(resolve,ms));
  const mask = () => [...pointer.buttons].reduce((n,b) => n | [1,4,2][b],0);
  const keyboardTarget = explicit => explicit || owner.activeElement || owner.body;
  function emit(target,type,button=-1,detail=0,relatedTarget=null) {
    check();
    if (!target?.isConnected) return true;
    const pointerEvent=type.startsWith('pointer') || type==='click' || type==='auxclick';
    const init = {view:globalThis,bubbles:!type.endsWith('enter')&&!type.endsWith('leave'),
      cancelable:!type.endsWith('enter')&&!type.endsWith('leave'),
      composed:!type.endsWith('enter')&&!type.endsWith('leave'),
      clientX:pointer.x,clientY:pointer.y,screenX:pointer.x,screenY:pointer.y,
      button:pointerEvent?button:Math.max(0,button),buttons:mask(),
      detail:type.startsWith('pointer')?0:detail,relatedTarget,
      ctrlKey:modifiers.has('Control'),altKey:modifiers.has('Alt'),
      shiftKey:modifiers.has('Shift'),metaKey:modifiers.has('Meta'),
      pointerId:1,pointerType:'mouse',isPrimary:true,width:1,height:1,pressure:mask()?0.5:0};
    const event = pointerEvent
      ? new NativePointerEvent(type,init) : new NativeMouseEvent(type,init);
    return dispatchInput(target,type,false,event);
  }
  function hit() { return hitTest(pointer.x,pointer.y); }
  function drag(operation,target=null) {
    const flags=(modifiers.has('Control')?1:0)|(modifiers.has('Alt')?2:0)|
      (modifiers.has('Shift')?4:0)|(modifiers.has('Meta')?8:0);
    return dragInput(operation,target,pointer.x,pointer.y,flags);
  }
  function move(x,y) {
    const old = pointer.target;
    pointer.x=x; pointer.y=y;
    const target=hit();
    if(!dragging&&dragCandidate&&pointer.buttons.has(0)&&
       (Math.abs(x-dragCandidate.x)>=4||Math.abs(y-dragCandidate.y)>=4)) {
      const source=dragCandidate.target;dragCandidate=null;
      dragging=drag('begin',source);
      if(dragging) {dragConsumed=true;emit(source,'pointercancel');pointer.suppressMouse=true;}
    }
    if(dragging) {
      dragging=drag('advance',target);pointer.target=target;return;
    }
    if (target!==old) {
      const [leaves,enters] = boundarySnapshot(old,target);
      emit(old,'pointerout',-1,0,target); emit(old,'mouseout',-1,0,target);
      for(const node of leaves) { emit(node,'pointerleave',-1,0,target); emit(node,'mouseleave',-1,0,target); }
      emit(target,'pointerover',-1,0,old); emit(target,'mouseover',-1,0,old);
      for(const node of enters) { emit(node,'pointerenter',-1,0,old); emit(node,'mouseenter',-1,0,old); }
    }
    pointer.target=target;
    emit(target,'pointermove');
    if(!pointer.suppressMouse) emit(target,'mousemove',0);
  }
  function point(action) {
    let x=0,y=0;
    const origin=action.origin || 'viewport';
    if (origin==='pointer') ({x,y}=pointer);
    else if (origin!=='viewport') {
      if (!origin.isConnected || origin.ownerDocument!==owner) throw new Error('stale action element');
      const r=rects(origin)[0];
      if (!r) throw new Error('action element is not interactable');
      const left=Math.max(0,Math.min(r.left,r.right)),right=Math.min(innerWidth,Math.max(r.left,r.right));
      const top=Math.max(0,Math.min(r.top,r.bottom)),bottom=Math.min(innerHeight,Math.max(r.top,r.bottom));
      if(left>=right||top>=bottom) throw new Error('action element is not interactable');
      x=Math.floor((left+right)/2); y=Math.floor((top+bottom)/2);
    }
    x+=action.x; y+=action.y;
    if(x<0||y<0||x>=innerWidth||y>=innerHeight) throw new Error('action move out of bounds');
    return {x,y};
  }
  function down(button) {
    if(pointer.buttons.has(button)) return;
    if(button===0) dragConsumed=false;
    const target=hit(),now=performance.now(),last=pointer.last;
    pointer.count=last&&last.target===target&&last.button===button&&last.x===pointer.x&&last.y===pointer.y&&now-last.time<500?pointer.count+1:1;
    const first=pointer.buttons.size===0;
    pointer.buttons.add(button); pointer.down.set(button,target);
    cancelInputs.push({type:'pointer',value:button});
    const allowed=emit(target,first?'pointerdown':'pointermove',button,pointer.count);
    if(first) pointer.suppressMouse=!allowed;
    if(!pointer.suppressMouse&&emit(target,'mousedown',button,pointer.count)) {
      focusPointer(target);
      if(button===0) dragCandidate={target,x:pointer.x,y:pointer.y};
    }
  }
  function up(button,release=false) {
    if(!pointer.buttons.delete(button)) return;
    const target=hit(),initial=pointer.down.get(button);
    pointer.down.delete(button);
    if(button===0) {
      dragCandidate=null;
      if(dragConsumed) {
        try {if(dragging)drag(release?'cancel':'drop',target);}
        finally {dragging=false;dragConsumed=false;pointer.suppressMouse=false;pointer.last=null;}
        return;
      }
    }
    emit(target,pointer.buttons.size?'pointermove':'pointerup',button,pointer.count);
    if(!pointer.suppressMouse) emit(target,'mouseup',button,pointer.count);
    if(pointer.buttons.size===0) pointer.suppressMouse=false;
    const clickTarget=!release&&initial&&target?boundarySnapshot(initial,target)[2]:null;
    if(clickTarget) {
      emit(clickTarget,button===0?'click':'auxclick',button,pointer.count);
      if(button===0&&pointer.count===2) emit(clickTarget,'dblclick',button,2);
      pointer.last={target:clickTarget,button,x:pointer.x,y:pointer.y,time:performance.now()};
    }
  }
  function validate(sources) {
    if(!Array.isArray(sources)||sources.length>16||new Set(sources.map(s=>s.id)).size!==sources.length||
       sources.filter(s=>s.type==='pointer').length>1||sources.filter(s=>s.type==='key').length>1) unsupported();
    for(const source of sources) {
      if(typeof source.id!=='string'||!['none','key','pointer','wheel'].includes(source.type)||!Array.isArray(source.actions)||source.actions.length>4096||
         source.type==='pointer'&&(source.parameters?.pointerType||'mouse')!=='mouse') unsupported();
      for(const a of source.actions) {
        if(!a||a.duration!==undefined&&(!Number.isInteger(a.duration)||a.duration<0||a.duration>10000)) unsupported();
        if(a.type==='pause') continue;
        if(source.type==='key'&&['keyDown','keyUp'].includes(a.type)) {
          if(typeof a.value!=='string'||[...a.value].length!==1||/[\uE000-\uF8FF]/.test(a.value)&&!special[a.value]) unsupported();
        } else if(source.type==='pointer'&&a.type==='pointerMove') {
          if(!Number.isInteger(a.x)||!Number.isInteger(a.y)||!['viewport','pointer'].includes(a.origin||'viewport')&&a.origin?.nodeType!==1) unsupported();
        } else if(source.type==='wheel'&&a.type==='scroll') {
          if(![a.x,a.y,a.deltaX,a.deltaY].every(Number.isInteger)||a.origin==='pointer'||
             (a.origin||'viewport')!=='viewport'&&a.origin?.nodeType!==1) unsupported();
        } else if(!(source.type==='pointer'&&['pointerDown','pointerUp'].includes(a.type)&&[0,1,2].includes(a.button))) unsupported();
      }
    }
  }
  function release(explicit) {
    try {
      if(!retired&&document===owner) {
        while(cancelInputs.length) {
          const input=cancelInputs.pop();
          if(input.type==='key') {
            if(keys.delete(input.value)) dispatchKeyboard(keyboardTarget(explicit),input.value,false);
          } else up(input.value,true);
        }
      }
    } finally {
      if(dragging) {try {drag('cancel');} finally {dragging=false;}}
      dragCandidate=null;dragConsumed=false;cancelInputs.length=0;keys.clear();modifiers.clear();pointer.buttons.clear();pointer.down.clear();pointer.suppressMouse=false;
    }
  }
  async function run(sources,explicit) {
    check(); validate(sources);
    // Successive action calls share device state until explicit release.
    // Retain the previous hit target for boundary events across calls.
    try {
      const length=Math.max(0,...sources.map(s=>s.actions.length));
      for(let tick=0;tick<length;tick++) {
        check();
        const started=performance.now();
        const duration=Math.max(0,...sources.map(s=>s.actions[tick]?.duration||0));
        const moves=[];
        for(const source of sources) {
          const a=source.actions[tick]; if(!a||a.type==='pause') continue;
          if(a.type==='pointerMove') {
            const destination=point(a),ms=a.duration??duration;
            if(ms) moves.push({...destination,ms,sx:pointer.x,sy:pointer.y}); else move(destination.x,destination.y);
          } else if(a.type==='scroll') {
            const at=point(a),target=hitTest(at.x,at.y);
            if(target) dispatchInput(target,'wheel',false,new NativeWheelEvent('wheel',{
              view:globalThis,bubbles:true,cancelable:true,composed:true,
              clientX:at.x,clientY:at.y,screenX:at.x,screenY:at.y,
              deltaX:a.deltaX,deltaY:a.deltaY,deltaZ:0,deltaMode:0,
              ctrlKey:modifiers.has('Control'),altKey:modifiers.has('Alt'),
              shiftKey:modifiers.has('Shift'),metaKey:modifiers.has('Meta')
            }));
          } else if(a.type==='pointerDown') down(a.button);
          else if(a.type==='pointerUp') up(a.button);
          else {
            const repeat=keys.has(a.value);
            if(a.type==='keyDown') {
              if((a.value==='\uE00C'||a.value==='Escape')&&dragging) {
                try {drag('cancel');} finally {dragging=false;}
              }
              keys.add(a.value);
              if(!repeat) cancelInputs.push({type:'key',value:a.value});
            }
            else if(!keys.delete(a.value)) continue;
            dispatchKeyboard(keyboardTarget(explicit),a.value,a.type==='keyDown',a.type==='keyDown'&&repeat);
          }
        }
        while(moves.length) {
          check();
          for(let i=moves.length-1;i>=0;i--) {
            const m=moves[i],fraction=Math.min(1,(performance.now()-started)/m.ms);
            move(m.sx+(m.x-m.sx)*fraction,m.sy+(m.y-m.sy)*fraction);
            if(fraction===1) moves.splice(i,1);
          }
          if(moves.length) await sleep(16);
        }
        const remaining=duration-(performance.now()-started);
        if(remaining>0) await sleep(remaining);
        check();
      }
    } catch(error) { release(explicit); throw error; }
  }
  globalThis.__oryn_testdriver_actions=(sources,explicit=null)=>{
    // Validate and copy the request now: callers cannot mutate queued actions.
    try { validate(sources); } catch(error) { return Promise.reject(error); }
    const snapshot=sources.map(s=>({...s,parameters:{...s.parameters},actions:s.actions.map(a=>({...a}))}));
    const result=queue.then(()=>run(snapshot,explicit));
    queue=result.catch(()=>{});
    return result;
  };
  globalThis.__oryn_testdriver_release_actions=(explicit=null)=>{
    const result=queue.then(()=>{check();release(explicit)});
    queue=result.catch(()=>{});
    return result;
  };
  globalThis.__oryn_testdriver_click=target=>{
    // Element click uses the same typed, trusted device events as Actions.
    // Keep validation and hit-testing inside the realm's serialized queue.
    const result=queue.then(()=>{
      check();
      if(!target||target.nodeType!==1) throw new TypeError('testdriver target is required');
      const moveAction={type:'pointerMove',origin:target,x:0,y:0};
      const destination=point(moveAction);
      const hitTarget=hitTest(destination.x,destination.y);
      if(!hitTarget||hitTarget!==target&&!contains(target,hitTarget))
        throw new Error('element click intercepted');
      return run([{type:'pointer',id:'oryn-element-click',parameters:{pointerType:'mouse'},
        actions:[moveAction,{type:'pointerDown',button:0},{type:'pointerUp',button:0}]}],null);
    });
    queue=result.catch(()=>{});
    return result;
  };
  globalThis.__oryn_testdriver_actions_version=1;
  addEventListener('pagehide',()=>{retired=true;release();});
})();
