/* Browser-specific transport. Arguments are transient; logs never contain input. */
(() => {
  const pending = new Map();
  const elements = new Map();
  const documentToken = crypto.randomUUID();
  let sequence = 0;
  globalThis.__oryn_testdriver_log = [];
  globalThis.__oryn_testdriver_overflow = false;
  globalThis.__oryn_testdriver_log.push = function(event) {
    if (this.length >= 4096) { globalThis.__oryn_testdriver_overflow = true; return this.length; }
    return Array.prototype.push.call(this, event);
  };
  globalThis.__oryn_driver_element = id => elements.get(id);
  globalThis.__oryn_driver_reply = (token, id, result, error) => {
    if (token !== documentToken) return;
    const entry = pending.get(id);
    if (!entry) return;
    clearTimeout(entry.timer);
    pending.delete(id);
    elements.delete(id);
    for (const reference of entry.references) elements.delete(reference);
    __oryn_testdriver_log.push({id, operation: entry.operation,
      outcome: error === 'unsupported' ? 'unsupported' : error ? 'failed' : 'completed', backend: 'chromium'});
    if (error) entry.reject(new Error(error)); else entry.resolve(result);
  };
  function invoke(operation, element, args) {
    const id = ++sequence;
    __oryn_testdriver_log.push({id, operation, outcome: 'started', backend: 'chromium'});
    return new Promise((resolve, reject) => {
      if (pending.size >= 64) {
        globalThis.__oryn_testdriver_overflow = true;
        reject(new Error('automation_queue_limit'));
        return;
      }
      const timer = setTimeout(() => __oryn_driver_reply(documentToken, id, null, 'automation_timeout'), 10000);
      const references = [];
      pending.set(id, {resolve, reject, timer, operation, references});
      if (element) elements.set(id, element);
      if (typeof globalThis.__oryn_driver_request !== 'function') {
        __oryn_driver_reply(documentToken, id, null, 'bridge_unavailable');
        return;
      }
      globalThis.__oryn_driver_request(JSON.stringify({token: documentToken, id, operation, args}, (key, value) => {
        if (key === 'origin' && value?.nodeType === 1) {
          const reference = `${id}:${references.length}`;
          elements.set(reference, value);
          references.push(reference);
          return {element: reference};
        }
        return value;
      }));
    });
  }
  const driver = globalThis.test_driver_internal;
  for (const operation of Object.keys(driver)) {
    if (typeof driver[operation] === 'function') {
      driver[operation] = () => invoke(operation, null, null);
    }
  }
  driver.click = element => invoke('click', element, null);
  driver.send_keys = (element, keys) => invoke('send_keys', element, {keys});
  driver.action_sequence = actions => invoke('action_sequence', null, {actions});
  driver.get_computed_label = element => invoke('get_computed_label', element, null);
  driver.get_computed_role = element => invoke('get_computed_role', element, null);
  driver.get_accessibility_properties_for_element = element => invoke('get_accessibility_properties_for_element', element, null);
  driver.get_accessibility_properties_for_accessibility_node = id => invoke('get_accessibility_properties_for_accessibility_node', null, {id});
})();
