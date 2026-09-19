// Historical profile only. The native binary must explicitly enable
// historical-test-input; this adapter cannot grant native input authority.
'use strict';
window.__oryn_testdriver_log = [];
window.__oryn_testdriver_overflow = false;
window.__oryn_testdriver_log.push = function(event) {
  if (typeof globalThis.__oryn_record_automation_event === 'function') {
    __oryn_record_automation_event(event.id, event.operation, event.outcome);
  }
  if (this.length >= 4096) { window.__oryn_testdriver_overflow = true; return this.length; }
  return Array.prototype.push.call(this, event);
};
window.test_driver_internal.click = element => __oryn_testdriver_click(element);
window.test_driver_internal.send_keys = (element, keys) => __oryn_testdriver_send_keys(element, keys);
// Only the versioned engine scheduler may establish a successful trace.
window.test_driver_internal.action_sequence = (actions, context=null) => {
  const documents = new Set(actions.flatMap(source => source.actions || [])
    .filter(action => action.origin?.nodeType === 1).map(action => action.origin.ownerDocument));
  if (documents.size > 1) return Promise.reject(new Error('not implemented: mixed-frame action origins'));
  const target = context || (documents.size ? [...documents][0].defaultView : window);
  if (target.__oryn_testdriver_actions_version !== 1) return Promise.reject(new Error('not implemented: faithful action_sequence'));
  return target.__oryn_testdriver_actions(actions);
};
window.test_driver_internal.get_computed_label = element => Promise.resolve(__oryn_computed_accessibility(element).name);
window.test_driver_internal.get_computed_role = element => Promise.resolve(__oryn_computed_accessibility(element).role);
const __oryn_accessibility_nodes = new Map();
const __oryn_accessibility_ids = new WeakMap();
let __oryn_next_accessibility_id = 1;
const __oryn_accessibility_id = element => {
  let id = __oryn_accessibility_ids.get(element);
  if (!id) { id = `oryn-accessibility-${__oryn_next_accessibility_id++}`; __oryn_accessibility_ids.set(element, id); }
  return id;
};
const __oryn_accessibility_properties = (element, parent = null) => {
  const computed = __oryn_computed_accessibility(element);
  const accessibilityId = __oryn_accessibility_id(element);
  const properties = {
    accessibilityId,
    parent,
    role: computed.role,
    label: computed.name,
    children: [],
  };
  const role = String(computed.role || '');
  if (role === 'checkbox' || role === 'radio' || role === 'switch') {
    properties.checked = element.getAttribute('aria-checked') ?? (element.checked ? 'true' : 'false');
  }
  if (element.hasAttribute('aria-pressed')) properties.pressed = element.getAttribute('aria-pressed');
  for (const child of [...element.children]) {
    const childProperties = __oryn_accessibility_properties(child, accessibilityId);
    properties.children.push(childProperties.accessibilityId);
  }
  __oryn_accessibility_nodes.set(accessibilityId, properties);
  return properties;
};
window.test_driver_internal.get_accessibility_properties_for_element = element => Promise.resolve(__oryn_accessibility_properties(element));
window.test_driver_internal.get_accessibility_properties_for_accessibility_node = id => {
  if (!__oryn_accessibility_nodes.has(String(id))) return Promise.reject(new Error('stale accessibility node'));
  return Promise.resolve(__oryn_accessibility_nodes.get(String(id)));
};
let __oryn_operation_id = 0;
for (const operation of Object.keys(window.test_driver_internal)) {
  const implementation = window.test_driver_internal[operation];
  if (typeof implementation !== 'function') continue;
  window.test_driver_internal[operation] = async function(...args) {
    const id = ++__oryn_operation_id;
    __oryn_testdriver_log.push({id, operation, outcome: 'started', backend: 'oryn'});
    try {
      const result = await implementation.apply(this, args);
      __oryn_testdriver_log.push({id, operation, outcome: 'completed', backend: 'oryn'});
      return result;
    } catch (error) {
      const unsupported = String(error).includes('not implemented');
      __oryn_testdriver_log.push({id, operation, outcome: unsupported ? 'unsupported' : 'failed', backend: 'oryn'});
      throw error;
    }
  };
}
