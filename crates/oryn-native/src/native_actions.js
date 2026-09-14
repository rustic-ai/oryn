/* Realm-owned mouse/keyboard input. This is engine code, not a WPT adapter. */
(() => {
  const owner = document;
  let retired = false, queue = Promise.resolve();
  const pointer = {x:0,y:0,target:null,buttons:new Set(),down:new Map(),last:null,count:0};
  const keys = new Set();
  const modifiers = globalThis.__oryn_testdriver_modifiers ||= new Set();
  const special = {'\uE003':'Backspace','\uE004':'Tab','\uE007':'Enter',
    '\uE008':'Shift','\uE009':'Control','\uE00A':'Alt','\uE010':'End','\uE011':'Home',
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
    const init = {bubbles:!type.endsWith('enter')&&!type.endsWith('leave'),
      cancelable:!type.endsWith('enter')&&!type.endsWith('leave'),
      composed:!type.endsWith('enter')&&!type.endsWith('leave'),
      clientX:pointer.x,clientY:pointer.y,screenX:pointer.x,screenY:pointer.y,
      button,buttons:mask(),detail,relatedTarget,
      ctrlKey:modifiers.has('Control'),altKey:modifiers.has('Alt'),
      shiftKey:modifiers.has('Shift'),metaKey:modifiers.has('Meta'),
      pointerId:1,pointerType:'mouse',isPrimary:true,width:1,height:1,pressure:mask()?0.5:0};
    const event = type.startsWith('pointer') || type==='click' || type==='auxclick'
      ? new PointerEvent(type,init) : new MouseEvent(type,init);
    return __oryn_dispatch_user_input(target,type,false,event);
  }
  function hit() { return owner.elementFromPoint(pointer.x,pointer.y); }
  function move(x,y) {
    const old = pointer.target;
    pointer.x=x; pointer.y=y;
    const target=hit();
    if (target!==old) {
      emit(old,'pointerout',-1,0,target); emit(old,'mouseout',-1,0,target);
      emit(old,'pointerleave',-1,0,target); emit(old,'mouseleave',-1,0,target);
      emit(target,'pointerover',-1,0,old); emit(target,'mouseover',-1,0,old);
      emit(target,'pointerenter',-1,0,old); emit(target,'mouseenter',-1,0,old);
    }
    pointer.target=target;
    emit(target,'pointermove'); emit(target,'mousemove',0);
  }
  function point(action) {
    let x=0,y=0;
    const origin=action.origin || 'viewport';
    if (origin==='pointer') ({x,y}=pointer);
    else if (origin!=='viewport') {
      if (!origin.isConnected || origin.ownerDocument!==owner) throw new Error('stale action element');
      const r=origin.getClientRects()[0];
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
    const target=hit(),now=performance.now(),last=pointer.last;
    pointer.count=last&&last.target===target&&last.button===button&&last.x===pointer.x&&last.y===pointer.y&&now-last.time<500?pointer.count+1:1;
    const first=pointer.buttons.size===0;
    pointer.buttons.add(button); pointer.down.set(button,target);
    const allowed=emit(target,first?'pointerdown':'pointermove',button,pointer.count);
    if(allowed&&emit(target,'mousedown',button,pointer.count)) __oryn_focus_from_pointer(target);
  }
  function up(button,release=false) {
    if(!pointer.buttons.delete(button)) return;
    const target=hit(),initial=pointer.down.get(button);
    pointer.down.delete(button);
    emit(target,pointer.buttons.size?'pointermove':'pointerup',button,pointer.count);
    emit(target,'mouseup',button,pointer.count);
    if(!release&&target&&target===initial) {
      emit(target,button===0?'click':'auxclick',button,pointer.count);
      if(button===0&&pointer.count===2) emit(target,'dblclick',button,2);
      pointer.last={target,button,x:pointer.x,y:pointer.y,time:performance.now()};
    }
  }
  function validate(sources) {
    if(!Array.isArray(sources)||sources.length>16||new Set(sources.map(s=>s.id)).size!==sources.length||
       sources.filter(s=>s.type==='pointer').length>1||sources.filter(s=>s.type==='key').length>1) unsupported();
    for(const source of sources) {
      if(typeof source.id!=='string'||!['none','key','pointer'].includes(source.type)||!Array.isArray(source.actions)||source.actions.length>4096||
         source.type==='pointer'&&(source.parameters?.pointerType||'mouse')!=='mouse') unsupported();
      for(const a of source.actions) {
        if(!a||a.duration!==undefined&&(!Number.isInteger(a.duration)||a.duration<0||a.duration>10000)) unsupported();
        if(a.type==='pause') continue;
        if(source.type==='key'&&['keyDown','keyUp'].includes(a.type)) {
          if(typeof a.value!=='string'||[...a.value].length!==1||/[\uE000-\uF8FF]/.test(a.value)&&!special[a.value]) unsupported();
        } else if(source.type==='pointer'&&a.type==='pointerMove') {
          if(!Number.isInteger(a.x)||!Number.isInteger(a.y)||!['viewport','pointer'].includes(a.origin||'viewport')&&a.origin?.nodeType!==1) unsupported();
        } else if(!(source.type==='pointer'&&['pointerDown','pointerUp'].includes(a.type)&&[0,1,2].includes(a.button))) unsupported();
      }
    }
  }
  function release(explicit) {
    try {
      if(!retired&&document===owner) {
        for(const key of [...keys].reverse()) __oryn_testdriver_keyboard(keyboardTarget(explicit),key,false);
        for(const button of [...pointer.buttons].reverse()) up(button,true);
      }
    } finally { keys.clear(); modifiers.clear(); pointer.buttons.clear(); pointer.down.clear(); }
  }
  async function run(sources,explicit) {
    check(); validate(sources);
    // Pinned WPT resets Actions state before the next sequence, not before
    // resolving the current one (held keys remain observable until then).
    release(explicit);
    pointer.x=0; pointer.y=0; pointer.target=null;
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
          } else if(a.type==='pointerDown') down(a.button);
          else if(a.type==='pointerUp') up(a.button);
          else {
            const repeat=keys.has(a.value);
            if(a.type==='keyDown') keys.add(a.value);
            else if(!keys.delete(a.value)) continue;
            __oryn_testdriver_keyboard(keyboardTarget(explicit),a.value,a.type==='keyDown',a.type==='keyDown'&&repeat);
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
  globalThis.__oryn_testdriver_click=target=>{
    // Element click uses the same typed, trusted device events as Actions.
    // Keep validation and hit-testing inside the realm's serialized queue.
    const result=queue.then(()=>{
      check();
      if(!target||target.nodeType!==1) throw new TypeError('testdriver target is required');
      const moveAction={type:'pointerMove',origin:target,x:0,y:0};
      const destination=point(moveAction);
      const hitTarget=owner.elementFromPoint(destination.x,destination.y);
      if(!hitTarget||hitTarget!==target&&!target.contains(hitTarget))
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
