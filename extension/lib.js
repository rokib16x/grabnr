// Shared logic for background, popup and options. Only ever called from
// extension contexts (the app rejects requests carrying a web-page Origin).

// Tests may set globalThis.__GRABNR_BASE_URL before importing this module.
export const BASE_URL = globalThis.__GRABNR_BASE_URL || 'http://127.0.0.1:17653';

export const DEFAULTS = {
  enabled: true,
  token: '',
  // .torrent files are handed to the browser: grabnr would otherwise download the content they describe.
  skipExtensions: ['torrent'],
  skipDomains: [],
  minSizeMB: 0,
};

export async function getSettings() {
  const stored = await chrome.storage.local.get(Object.keys(DEFAULTS));
  return { ...DEFAULTS, ...stored };
}

export async function saveSettings(patch) {
  await chrome.storage.local.set(patch);
}

async function api(path, { method = 'GET', token, body } = {}) {
  const headers = {};
  if (token) headers.Authorization = `Bearer ${token}`;
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  const res = await fetch(BASE_URL + path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  let data = null;
  try { data = await res.json(); } catch { /* non-JSON body */ }
  return { status: res.status, data };
}

/** -> {running, paired, version}. Never throws. */
export async function ping() {
  try {
    const { status, data } = await api('/ping');
    if (status === 200 && data && data.app === 'grabnr') {
      return { running: true, paired: !!data.paired, version: data.version };
    }
  } catch { /* connection refused */ }
  return { running: false, paired: false, version: null };
}

/** Ask the app for a token (only works inside its 60 s pairing window). */
export async function pair() {
  try {
    const { status, data } = await api('/pair');
    if (status === 200 && data && data.token) {
      await saveSettings({ token: data.token });
      return { ok: true };
    }
    if (status === 403) return { ok: false, reason: 'pairing_closed' };
    return { ok: false, reason: 'error' };
  } catch {
    return { ok: false, reason: 'not_running' };
  }
}

/**
 * POST /add. Returns {ok, status, id?, error?}; status 0 means unreachable.
 * `req` is the API body: {url, filename?, referrer?, cookies?, userAgent?, headers?}.
 */
export async function addDownload(req, token) {
  try {
    const { status, data } = await api('/add', { method: 'POST', token, body: req });
    if (status === 200) return { ok: true, status, id: data && data.id };
    return { ok: false, status, error: data && data.error };
  } catch {
    return { ok: false, status: 0, error: 'not_running' };
  }
}

/** Authenticated no-op check: empty body -> 400 means token accepted, 401 means bad. */
export async function checkToken(token) {
  const r = await addDownload({}, token);
  if (r.status === 400) return 'ok';
  if (r.status === 401) return 'bad_token';
  if (r.status === 0) return 'not_running';
  return 'unexpected';
}

export function isCapturableUrl(url) {
  return typeof url === 'string' && /^https?:\/\//i.test(url);
}

function hostOf(url) {
  try { return new URL(url).hostname.toLowerCase(); } catch { return ''; }
}

function extOf(name) {
  const m = /\.([A-Za-z0-9]+)$/.exec(name || '');
  return m ? m[1].toLowerCase() : '';
}

export function basename(path) {
  return (path || '').split(/[\\/]/).pop();
}

/** Decide whether a browser download should be handed to grabnr. */
export function shouldCapture(item, settings) {
  if (!settings.enabled) return false;
  const url = item.finalUrl || item.url;
  if (!isCapturableUrl(url)) return false;

  const host = hostOf(url);
  const domainHit = (settings.skipDomains || []).some((d) => {
    d = String(d).trim().toLowerCase().replace(/^\*?\./, '');
    return d && (host === d || host.endsWith('.' + d));
  });
  if (domainHit) return false;

  // Extension from the suggested filename, else from the URL path.
  let ext = extOf(basename(item.filename));
  if (!ext) { try { ext = extOf(new URL(url).pathname); } catch { /* ignore */ } }
  const skipExt = (settings.skipExtensions || []).map((e) => String(e).trim().toLowerCase().replace(/^\./, ''));
  if (ext && skipExt.includes(ext)) return false;

  // Size is frequently unknown (-1/0) at capture time; only apply when known.
  const minBytes = (Number(settings.minSizeMB) || 0) * 1024 * 1024;
  if (minBytes > 0 && item.fileSize > 0 && item.fileSize < minBytes) return false;
  return true;
}

export async function cookieHeaderFor(url) {
  try {
    const cookies = await chrome.cookies.getAll({ url });
    return cookies.map((c) => `${c.name}=${c.value}`).join('; ');
  } catch { return ''; }
}

/** Build the /add body from a browser download item. */
export async function buildRequest(item) {
  const url = item.finalUrl || item.url;
  const req = { url, userAgent: navigator.userAgent };
  if (item.referrer) req.referrer = item.referrer;
  const name = basename(item.filename);
  if (name) req.filename = name;
  const cookies = await cookieHeaderFor(url);
  if (cookies) req.cookies = cookies;
  return req;
}
