// Hydrate existing server markup and load the search route as a separate chunk.
export function hydrateResearch(document, window, load = () => import('./research-search.mjs')) {
  const app = document.querySelector('#research-app');
  if (!app) return;
  const form = document.querySelector('#search-form');
  const results = document.querySelector('#results');
  const status = document.querySelector('#search-status');
  let generation = 0;
  let pending;
  async function navigate(query, page, push = true) {
    const current = ++generation;
    pending?.abort();
    const controller = new AbortController();
    pending = controller;
    app.dataset.state = 'loading';
    status.textContent = 'Loading sources';
    try {
      const {fetchResults, searchRoute} = await load();
      if (current !== generation) return;
      const {url, api} = searchRoute(app.dataset.base, query, page, window.location.origin);
      const text = await fetchResults(api, {signal: controller.signal});
      if (current !== generation) return;
      // Only this fixture's fixed, independently hashed HTML is inserted.
      results.innerHTML = text;
      if (push) window.history.pushState(null, '', url);
      app.dataset.page = page;
      app.dataset.state = 'ready';
      status.textContent = 'Sources loaded';
    } catch (error) {
      if (current !== generation || controller.signal.aborted) return;
      app.dataset.state = 'error';
      status.textContent = error.message;
    }
  }
  form.addEventListener('submit', event => {
    event.preventDefault();
    if (form.reportValidity()) navigate(form.elements.q.value, '1');
  });
  results.addEventListener('click', event => {
    const link = event.target.closest('a');
    if (!link) return;
    const url = new URL(link.href, window.location.href);
    if (url.origin !== window.location.origin || url.pathname !== `${app.dataset.base}/search`) return;
    event.preventDefault();
    navigate(url.searchParams.get('q'), url.searchParams.get('page') || '1');
  });
  const onPop = () => {
    const url = new URL(window.location.href);
    const query = url.searchParams.get('q') || 'energy';
    form.elements.q.value = query;
    return navigate(query, url.searchParams.get('page') || '1', false);
  };
  window.addEventListener('popstate', onPop);
  window.addEventListener('pagehide', () => { ++generation; pending?.abort(); });
  window.addEventListener('pageshow', event => {
    if (event.persisted && app.dataset.state === 'loading') return onPop();
  });
  app.dataset.page = new URL(window.location.href).searchParams.get('page') || '1';
  app.dataset.state = 'ready';
  return {navigate};
}
if (typeof document !== 'undefined') hydrateResearch(document, window);
