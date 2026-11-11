/**
 * Data access layer.
 * 1) GET /api/leaderboard?profile=<id>   (live server)
 * 2) GET /catalog.json                   (bundled offline catalog)
 * Submissions: POST /api/leaderboard/submit (JSON).
 */

const JSON_HEADERS = { Accept: 'application/json' };

async function getJson(url, signal) {
  const res = await fetch(url, { signal, headers: JSON_HEADERS, cache: 'no-store' });
  if (!res.ok) throw new Error(`HTTP ${res.status} for ${url}`);
  const data = await res.json();
  if (!data || typeof data !== 'object' || !Array.isArray(data.entries)) {
    throw new Error(`Malformed payload from ${url}`);
  }
  return data;
}

/**
 * Try the live API first, then the bundled catalog.
 * @returns {Promise<{mode:'live'|'catalog', data:object}>}
 */
export async function fetchLeaderboard(profileId, signal) {
  const apiUrl = `/api/leaderboard?profile=${encodeURIComponent(profileId)}`;
  try {
    const data = await getJson(apiUrl, signal);
    return { mode: 'live', data };
  } catch (err) {
    if (signal && signal.aborted) throw err;
    // Live server unavailable → bundled catalog (filtered client-side).
    let catalog;
    try {
      catalog = await getJson('/catalog.json', signal);
    } catch (err2) {
      if (signal && signal.aborted) throw err2;
      // Relative-path retry (folder served under a sub-path).
      try {
        catalog = await getJson('catalog.json', signal);
      } catch (err3) {
        if (signal && signal.aborted) throw err3;
        const e = new Error(`Live API: ${err.message} · Catalog: ${err2.message}`);
        e.cause = err3;
        throw e;
      }
    }
    return { mode: 'catalog', data: catalog };
  }
}

/**
 * POST a measured run. Returns {ok, status, body} — body is parsed JSON when
 * possible, otherwise raw text. Network failures throw.
 */
export async function submitRun(payload) {
  const res = await fetch('/api/leaderboard/submit', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
    body: JSON.stringify(payload),
  });
  const text = await res.text();
  let body = null;
  try {
    body = JSON.parse(text);
  } catch {
    body = text;
  }
  return { ok: res.ok, status: res.status, body };
}