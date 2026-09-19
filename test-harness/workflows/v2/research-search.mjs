// Route changes fetch the same controlled source endpoint as the legacy corpus.
export async function fetchResults(url, {fetcher = fetch, signal} = {}) {
  const response = await fetcher(url, {signal, credentials: 'same-origin'});
  if (!response.ok) throw new Error(`Search request failed (${response.status})`);
  return response.text();
}

export function searchRoute(base, query, page, origin) {
  const url = new URL(`${base}/search`, origin);
  url.searchParams.set('q', query);
  url.searchParams.set('page', page);
  const api = new URL(url);
  api.pathname = `${base}/api`;
  return {url, api};
}
