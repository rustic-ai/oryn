// Hydrate the server application; all cart/quote/order writes still reach SQLite.
export function paymentMessageAccepted(event, frame, status) {
  return Boolean(frame && status && event.source === frame.contentWindow &&
    event.origin === status.dataset.origin && event.data?.type === 'controlled-payment' &&
    event.data.quote === status.dataset.quote && typeof event.data.receipt === 'string' && /^[0-9a-f]{32}$/.test(event.data.receipt));
}

export function hydrateShop(document, window, fetcher = fetch) {
  let generation = 0;
  let pending;
  const current = () => document.querySelector('#shop-app');
  if (!current()) return;
  const ready = () => { current().dataset.state = 'ready'; };
  async function navigate(url, options = {}, push = true) {
    const mine = ++generation;
    pending?.abort();
    pending = new AbortController();
    current().dataset.state = 'loading';
    try {
      const response = await fetcher(url, {...options, signal: pending.signal, credentials: 'same-origin'});
      // 409 is an application recovery page, never treated as a successful order.
      if (!response.ok && response.status !== 409) throw new Error(`Request failed (${response.status})`);
      const destination = new URL(response.url);
      if (destination.origin !== window.location.origin || !destination.pathname.startsWith(current().dataset.base + '/') ||
          !response.headers.get('content-type')?.startsWith('text/html')) throw new Error('Foreign application response');
      const markup = await response.text();
      if (mine !== generation) return;
      const parsed = new DOMParser().parseFromString(markup, 'text/html');
      const replacement = parsed.querySelector('#shop-app');
      if (!replacement || replacement.dataset.base !== current().dataset.base) throw new Error('Foreign application response');
      current().replaceWith(document.importNode(replacement, true));
      if (push) window.history.pushState(null, '', response.url);
      ready();
    } catch (error) {
      if (mine !== generation) return;
      current().dataset.state = 'error';
      current().setAttribute('data-error', error.message);
    }
  }
  document.addEventListener('click', event => {
    const link = event.target.closest('a');
    if (!link || !current().contains(link)) return;
    const url = new URL(link.href, window.location.href);
    if (url.origin !== window.location.origin || !url.pathname.startsWith(current().dataset.base + '/')) return;
    event.preventDefault();
    navigate(url);
  });
  document.addEventListener('submit', event => {
    const form = event.target;
    if (!current().contains(form)) return;
    event.preventDefault();
    if (!form.reportValidity() || current().dataset.state !== 'ready') return;
    const data = new FormData(form, event.submitter);
    const url = new URL(form.action, window.location.href);
    if (form.method.toLowerCase() === 'get') {
      url.search = new URLSearchParams(data).toString();
      navigate(url);
    } else {
      navigate(url, {method: 'POST', body: new URLSearchParams(data)});
    }
  });
  window.addEventListener('message', event => {
    const status = document.querySelector('#payment-status');
    if (paymentMessageAccepted(event, document.querySelector('#payment-frame'), status)) {
      status.dataset.state = 'authorized';
      status.textContent = 'Controlled payment authorized';
    }
  });
  window.addEventListener('popstate', () => navigate(window.location.href, {}, false));
  window.addEventListener('pagehide', () => { ++generation; pending?.abort(); });
  window.addEventListener('pageshow', event => {
    // A parked request may already have committed on the server. Restore the
    // current view with GET; never replay a cart or checkout POST on resume.
    if (event.persisted && current()?.dataset.state === 'loading') {
      navigate(window.location.href, {}, false);
    }
  });
  ready();
}
if (typeof document !== 'undefined') hydrateShop(document, window);
