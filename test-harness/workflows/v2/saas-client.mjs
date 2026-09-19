// Hydrate server-rendered forms. Every record mutation still crosses the same
// authenticated, CSRF-checked backend used by normal form submission.
let active = true;
let pending = null;
let generation = 0;
const base = location.pathname.replace(/\/[^/]*$/, '/');

function app() { return document.querySelector('#saas-app'); }
function busy(root, value) {
  root.dataset.state = value ? 'saving' : 'ready';
  root.setAttribute('aria-busy', String(value));
  for (const button of root.querySelectorAll('button')) button.disabled = value;
}

addEventListener('pagehide', () => {
  active = false;
  generation++;
  pending?.abort();
});
addEventListener('pageshow', event => {
  if (!event.persisted) return;
  // Authentication and a possibly committed save must be read again from the
  // server. URL replacement performs a fresh GET without resubmitting a form.
  active = false;
  generation++;
  pending?.abort();
  location.replace(location.href);
});

document.addEventListener('submit', async event => {
  const form = event.target;
  const root = app();
  if (!(form instanceof HTMLFormElement) || !root?.contains(form)) return;
  const target = new URL(form.action, location.href);
  if (form.method.toLowerCase() !== 'post' || target.origin !== location.origin ||
      !target.pathname.startsWith(base)) return;
  event.preventDefault();
  if (!active || pending || root.dataset.state === 'error') return;
  // Construct before disabling controls; preserve successful-control semantics.
  const body = new URLSearchParams(new FormData(form, event.submitter));
  const controller = new AbortController();
  pending = controller;
  const current = ++generation;
  busy(root, true);
  try {
    const response = await fetch(target, {method: 'POST', body, credentials: 'same-origin',
      cache: 'no-store', signal: controller.signal});
    const text = await response.text();
    if (!active || current !== generation) return;
    const destination = new URL(response.url);
    if (destination.origin !== location.origin || !destination.pathname.startsWith(base) ||
        !response.headers.get('content-type')?.startsWith('text/html')) {
      throw new Error('Invalid application response');
    }
    const parsed = new DOMParser().parseFromString(text, 'text/html');
    const next = parsed.querySelector('#saas-app') || parsed.body;
    // Scripts from response documents are not executed. This module owns the
    // hydrated form behavior throughout the document lifetime.
    for (const script of next.querySelectorAll('script')) script.remove();
    root.replaceChildren(...[...next.childNodes].map(node => document.importNode(node, true)));
    root.dataset.version = root.querySelector('#version')?.textContent || '0';
    root.dataset.response = String(response.status);
    history.replaceState(null, '', destination.href);
    busy(root, false);
  } catch (error) {
    if (!active || current !== generation) return;
    busy(root, false);
    root.dataset.state = 'error';
    let message = root.querySelector('#save-status');
    if (!message) {
      message = document.createElement('p');
      message.id = 'save-status';
      message.setAttribute('role', 'alert');
      root.append(message);
    }
    message.textContent = 'Save outcome unknown. Reload the record before trying again.';
    // The server may have committed before transport failed. Do not silently
    // retry the write or let a second submit claim a fresh outcome.
    for (const button of root.querySelectorAll('button')) button.disabled = true;
  } finally {
    if (current === generation) pending = null;
  }
});
const initial = app();
if (initial) {
  initial.dataset.version = initial.querySelector('#version')?.textContent || '0';
  initial.dataset.response = '200';
  busy(initial, false);
}

// An explicit controlled retry surface preserves the exact previous request,
// including its optimistic version. A repeated click must be rejected by the
// backend instead of being transformed into a new accepted versioned save.
const replayKey = `oryn-controlled-write:${base}`;
let replaySequence = 0;
function installReplay() {
  const root = app();
  if (!root?.querySelector('#record') || !sessionStorage.getItem(replayKey) || root.querySelector('#repeat-save')) return;
  const button = document.createElement('button');
  button.type = 'button'; button.id = 'repeat-save'; button.textContent = 'Repeat saved request';
  const status = document.createElement('p'); status.id = 'duplicate-status';
  button.addEventListener('click', async () => {
    if (!active || pending || root.dataset.state === 'error') return;
    const saved = JSON.parse(sessionStorage.getItem(replayKey));
    const target = new URL(saved.url, location.href);
    if (target.origin !== location.origin || !target.pathname.startsWith(base)) throw new Error('Foreign replay target');
    const controller = new AbortController();
    pending = controller;
    const current = ++generation;
    button.disabled = true;
    try {
      const response = await fetch(target, {method: 'POST',body: new URLSearchParams(saved.body),credentials: 'same-origin',cache: 'no-store',signal: controller.signal});
      await response.text();
      if (!active || current !== generation) return;
      status.dataset.status = String(response.status);
      status.dataset.count = String(++replaySequence);
      status.textContent = `Repeated request status ${response.status}`;
    } catch (error) {
      if (!active || current !== generation) return;
      root.dataset.state = 'error';
      status.textContent = 'Save outcome unknown. Reload the record before trying again.';
    } finally {
      if (current === generation) {
        pending = null;
        button.disabled = root.dataset.state === 'error';
      }
    }
  });
  root.append(button,status);
}
document.addEventListener('submit', event => {
  const form = event.target;
  if (!(form instanceof HTMLFormElement)) return;
  const root = app();
  if (!active || pending || !root?.contains(form) || root.dataset.state === 'error' ||
      form.method.toLowerCase() !== 'post') return;
  const target = new URL(form.action,location.href);
  if (target.origin === location.origin && target.pathname.startsWith(base) && /\/(create|update)$/.test(target.pathname)) {
    // The main submit listener constructs the same successful controls before
    // disabling them. Cache that native form payload at capture time.
    sessionStorage.setItem(replayKey,JSON.stringify({url:target.href,body:new URLSearchParams(new FormData(form,event.submitter)).toString()}));
  }
},true);
new MutationObserver(installReplay).observe(document,{childList:true,subtree:true});
installReplay();
