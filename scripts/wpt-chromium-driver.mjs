/* Host-side automation: no synthetic page events and no persistent input log. */
import {setTimeout as delay} from 'node:timers/promises';

// A WPT callback may run inside the last input event, before its host command
// and page reply settle. Drain real work, then reread the live report; never
// synthesize a terminal automation event from a passing assertion.
export async function settleAutomation(getQueue, readCompletion, alive, deadline) {
  const check = () => { if (!alive()) throw new Error('reference_completion_context_retired'); };
  const bounded = async work => {
    const remaining = deadline - Date.now();
    if (remaining <= 0) throw new Error('reference_automation_drain_timeout');
    let timer;
    try {
      return await Promise.race([work, new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error('reference_automation_drain_timeout')), remaining);
      })]);
    } finally { clearTimeout(timer); }
  };
  for (;;) {
    check();
    const queue = getQueue();
    await bounded(queue);
    check();
    const completion = await bounded(Promise.resolve().then(readCompletion));
    check();
    if (!completion) throw new Error('reference_completion_disappeared');
    if (queue === getQueue()) return completion;
  }
}

export async function dispatchDriver(call, contextId, request, alive, state = {}) {
  const {operation, id, args} = request;
  const check = () => { if (!alive()) throw new Error('stale_context'); };
  const command = async (method, params = {}) => { check(); return call(method, params); };
  let objectId;
  const temporaryObjects = new Set();
  const target = async () => {
    if (objectId) return objectId;
    const value = await command('Runtime.evaluate', {contextId,
      expression: `globalThis.__oryn_driver_element(${JSON.stringify(id)})`});
    objectId = value.result?.objectId;
    if (!objectId) throw new Error('missing_target');
    return objectId;
  };
  const center = async (reference = id) => {
    const value = await command('Runtime.evaluate', {contextId,
      expression:`globalThis.__oryn_driver_element(${JSON.stringify(reference)})`});
    const object = value.result?.objectId;
    if (!object) throw new Error('missing_target');
    temporaryObjects.add(object);
    const {quads} = await command('DOM.getContentQuads', {objectId:object});
    const metrics = await command('Page.getLayoutMetrics');
    const viewport = metrics.cssLayoutViewport || metrics.layoutViewport;
    if (!quads?.length || !viewport) throw new Error('not_interactable');
    const q = quads[0], xs = [q[0],q[2],q[4],q[6]], ys = [q[1],q[3],q[5],q[7]];
    const left = Math.max(0, Math.min(...xs)), right = Math.min(viewport.clientWidth, Math.max(...xs));
    const top = Math.max(0, Math.min(...ys)), bottom = Math.min(viewport.clientHeight, Math.max(...ys));
    if (left >= right || top >= bottom) throw new Error('not_interactable');
    return {x:Math.floor((left+right)/2), y:Math.floor((top+bottom)/2)};
  };
  const keyNames = {'\uE003':'Backspace','\uE004':'Tab','\uE006':'Enter','\uE007':'Enter',
    '\uE00C':'Escape','\uE00E':'PageUp','\uE00F':'PageDown','\uE010':'End','\uE011':'Home',
    '\uE012':'ArrowLeft','\uE013':'ArrowUp','\uE014':'ArrowRight','\uE015':'ArrowDown','\uE017':'Delete',
    '\uE008':'Shift','\uE009':'Control','\uE00A':'Alt','\uE03D':'Meta'};
  const modifierBits = {Alt:1,Control:2,Meta:4,Shift:8};
  // CDP does not infer legacy key codes or physical keys from `key`.
  const keyMetadata = key => {
    const virtual = {Backspace:8,Tab:9,Enter:13,Shift:16,Control:17,Alt:18,
      Escape:27,PageUp:33,PageDown:34,End:35,Home:36,ArrowLeft:37,
      ArrowUp:38,ArrowRight:39,ArrowDown:40,Delete:46,Meta:91};
    return key in virtual ? {windowsVirtualKeyCode:virtual[key],
      code:modifierBits[key] ? key+'Left' : key} : {};
  };
  try {
    if (operation === 'action_sequence') {
      const sources = args?.actions;
      if (!Array.isArray(sources) || new Set(sources.map(s => s.id)).size !== sources.length ||
          sources.filter(s => s.type === 'pointer').length > 1 ||
          sources.filter(s => s.type === 'key').length > 1) throw new Error('unsupported');
      // Validate the entire variant before producing any input.
      for (const source of sources) {
        if (!['none','key','pointer'].includes(source.type) || !Array.isArray(source.actions) ||
            source.type === 'pointer' && (source.parameters?.pointerType || 'mouse') !== 'mouse') throw new Error('unsupported');
        for (const action of source.actions) {
          if (action.duration !== undefined && (!Number.isFinite(action.duration) || action.duration < 0 || action.duration > 10000)) throw new Error('unsupported');
          if (action.type === 'pause') continue;
          if (source.type === 'key' && ['keyDown','keyUp'].includes(action.type)) {
            if (Array.from(action.value || '').length !== 1 || /[\uE000-\uF8FF]/.test(action.value) && !keyNames[action.value]) throw new Error('unsupported');
          } else if (source.type === 'pointer' && action.type === 'pointerMove') {
            if ((!['viewport','pointer'].includes(action.origin || 'viewport') && typeof action.origin?.element !== 'string') || !Number.isFinite(action.x) || !Number.isFinite(action.y)) throw new Error('unsupported');
          } else if (!(source.type === 'pointer' && ['pointerDown','pointerUp'].includes(action.type) && [0,1,2].includes(action.button))) throw new Error('unsupported');
        }
      }
      let x = state.x || 0, y = state.y || 0;
      const pressed = new Set(), buttons = new Set();
      const names = ['left','middle','right'];
      const buttonMask = () => [...buttons].reduce((n, b) => n | [1,4,2][b], 0);
      const modifiers = () => [...pressed].reduce((n, key) => n | (modifierBits[key] || 0), 0);
      try {
        const length = Math.max(0, ...sources.map(s => s.actions.length));
        for (let tick = 0; tick < length; tick++) {
          const started = Date.now();
          let duration = 0;
          const moves = [];
          for (const source of sources) {
            const action = source.actions[tick];
            if (!action) continue;
            duration = Math.max(duration, action.duration || 0);
            if (action.type === 'pause') continue;
            if (source.type === 'key') {
              const key = keyNames[action.value] || action.value;
              if (action.type === 'keyDown') pressed.add(key); else pressed.delete(key);
              await command('Input.dispatchKeyEvent', {type:action.type, key, ...keyMetadata(key), modifiers:modifiers(),
                ...(action.type === 'keyDown' && !keyNames[action.value] ? {text:action.value} : {})});
            } else if (action.type === 'pointerMove') {
              const origin = action.origin?.element ? await center(action.origin.element) :
                action.origin === 'pointer' ? {x,y} : {x:0,y:0};
              moves.push({x:action.x + origin.x,
                y:action.y + origin.y, duration:action.duration || 0});
            } else {
              if (action.type === 'pointerDown' && !buttons.has(action.button)) {
                const previous = state.lastClick;
                state.clickCount = previous && previous.x === x && previous.y === y &&
                  previous.button === action.button && Date.now()-previous.time < 500 ? (state.clickCount || 0)+1 : 1;
              }
              if (action.type === 'pointerDown') buttons.add(action.button); else buttons.delete(action.button);
              await command('Input.dispatchMouseEvent', {type:action.type === 'pointerDown' ? 'mousePressed' : 'mouseReleased',
                x, y, button:names[action.button], buttons:buttonMask(), modifiers:modifiers(), clickCount:state.clickCount || 1});
              if (action.type === 'pointerUp') state.lastClick = {x,y,button:action.button,time:Date.now()};
            }
          }
          for (const move of moves) {
            const sx = x, sy = y, start = Date.now();
            let fraction;
            do {
              fraction = move.duration ? Math.min(1, (Date.now()-start)/move.duration) : 1;
              x = sx + (move.x-sx)*fraction; y = sy + (move.y-sy)*fraction;
              await command('Input.dispatchMouseEvent', {type:'mouseMoved', x, y, buttons:buttonMask(), modifiers:modifiers()});
              if (fraction < 1) await delay(16);
            } while (fraction < 1);
          }
          const remaining = duration - (Date.now()-started);
          if (remaining > 0) await delay(remaining);
          check();
        }
      } finally {
        state.x = x; state.y = y;
        // Release only while this document is still the live automation owner.
        if (alive()) {
          for (const key of pressed) await command('Input.dispatchKeyEvent', {type:'keyUp',key,...keyMetadata(key)});
          for (const button of buttons) await command('Input.dispatchMouseEvent', {type:'mouseReleased',x,y,button:names[button],clickCount:1});
        }
      }
      return null;
    }
    if (operation === 'click') {
      await command('DOM.scrollIntoViewIfNeeded', {objectId: await target()});
      const {x, y} = await center();
      await command('Input.dispatchMouseEvent', {type:'mouseMoved', x, y});
      await command('Input.dispatchMouseEvent', {type:'mousePressed', x, y, button:'left', clickCount:1});
      await command('Input.dispatchMouseEvent', {type:'mouseReleased', x, y, button:'left', clickCount:1});
      return null;
    }
    if (operation === 'send_keys') {
      const keys = Array.from(String(args?.keys ?? ''));
      // Modifier persistence in WebDriver send_keys differs from Actions. Until
      // qualified, reject that variant before focusing or producing any input.
      if (keys.some(key => modifierBits[keyNames[key]] || /[\uE000-\uF8FF]/.test(key) && !keyNames[key])) throw new Error('unsupported');
      const objectId = await target();
      // WebDriver permits the body as a keyboard target even when it has no
      // focusable area. Preserve the current focused control in that case.
      const body = await command('Runtime.callFunctionOn', {objectId,
        functionDeclaration:'function(){return this === this.ownerDocument.body && this.tabIndex < 0 && !this.isContentEditable}',
        returnByValue:true});
      if (body.exceptionDetails || typeof body.result?.value !== 'boolean') throw new Error('target_state_unverified');
      if (!body.result.value) await command('DOM.focus', {objectId});
      for (const key of keys) {
        const name = keyNames[key] || key;
        await command('Input.dispatchKeyEvent', {type:'keyDown', key:name, ...keyMetadata(name),
          ...(!keyNames[key] ? {text:key} : {})});
        await command('Input.dispatchKeyEvent', {type:'keyUp', key:name, ...keyMetadata(name)});
      }
      return null;
    }
    if (operation === 'get_computed_label' || operation === 'get_computed_role' ||
        operation === 'get_accessibility_properties_for_element' || operation === 'get_accessibility_properties_for_accessibility_node') {
      await command('Accessibility.enable');
      const byId = operation === 'get_accessibility_properties_for_accessibility_node';
      if (byId && !state.accessibilityIds?.has(args?.id)) throw new Error('stale_accessibility_node');
      const {nodes} = byId ? await command('Accessibility.getFullAXTree') :
        await command('Accessibility.getPartialAXTree', {objectId: await target(), fetchRelatives:false});
      const node = byId ? nodes?.find(node => node.nodeId === args.id) : nodes?.[0];
      if (!node) return {};
      if (operation === 'get_computed_label') return node.name?.value || '';
      if (operation === 'get_computed_role') return node.role?.value || '';
      // Match the serialization used by pinned WPT's Chrome accessibility backend.
      state.accessibilityIds ||= new Set();
      for (const id of [node.nodeId, node.parentId, ...(node.childIds || [])]) {
        if (id) state.accessibilityIds.add(id);
      }
      const result = {accessibilityId:node.nodeId, children:node.childIds || []};
      if (node.parentId) result.parent = node.parentId;
      for (const [source,destination] of [['role','role'],['name','label'],['value','value'],['description','description']]) {
        if (node[source]) result[destination] = node[source].value;
      }
      const allowed = new Set(['checked','pressed','level','multiline','orientation','required','roledescription','selected']);
      for (const property of node.properties || []) if (allowed.has(property.name)) result[property.name] = property.value?.value;
      return result;
    }
    // Action sequences require synchronized ticks, origins, modifiers and release
    // semantics. Fail closed until that complete variant is qualified.
    throw new Error('unsupported');
  } finally {
    if (objectId) await call('Runtime.releaseObject', {objectId}).catch(() => {});
    for (const objectId of temporaryObjects) await call('Runtime.releaseObject', {objectId}).catch(() => {});
  }
}
