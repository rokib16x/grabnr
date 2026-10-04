// Node harness: stubs `chrome`, runs a fake grabnr server, drives background.js.
// Run: node test/harness.mjs
import http from 'node:http';
import assert from 'node:assert/strict';

const TOKEN = 'tok123';
const server = { mode: 'ok', adds: [] }; // mode: ok | 401 | 400
const srv = http.createServer((req, res) => {
  const send = (code, obj) => { res.writeHead(code, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(obj)); };
  if (req.url === '/ping') return send(200, { app: 'grabnr', version: '0.1.0', paired: true });
  if (req.url === '/pair') return send(403, { error: 'pairing_closed' });
  if (req.url === '/add' && req.method === 'POST') {
    let b = ''; req.on('data', (d) => (b += d));
    return req.on('end', () => {
      server.adds.push({ auth: req.headers.authorization, body: JSON.parse(b) });
      if (server.mode === '401' || req.headers.authorization !== `Bearer ${TOKEN}`) return send(401, { error: 'unauthorized' });
      send(200, { id: 'abc' });
    });
  }
  send(404, {});
});
await new Promise((r) => srv.listen(0, '127.0.0.1', r));
globalThis.__GRABNR_BASE_URL = `http://127.0.0.1:${srv.address().port}`;

// ---- chrome stub ----
const store = { enabled: true, token: TOKEN };
const calls = { cancel: [], erase: [], notify: [], badge: [] };
const listeners = {};
const ev = (name) => ({ addListener: (f) => (listeners[name] = f) });
globalThis.chrome = {
  downloads: {
    onCreated: ev('created'),
    cancel: async (id) => calls.cancel.push(id),
    erase: async (q) => calls.erase.push(q.id),
  },
  cookies: { getAll: async () => [{ name: 'a', value: 'b' }, { name: 'c', value: 'd' }] },
  storage: {
    local: {
      get: async (keys) => Object.fromEntries(keys.filter((k) => k in store).map((k) => [k, store[k]])),
      set: async (o) => Object.assign(store, o),
    },
    onChanged: ev('storage'),
  },
  contextMenus: { removeAll: (cb) => cb && cb(), create: () => {}, onClicked: ev('menu') },
  action: { setBadgeText: async (o) => calls.badge.push(o.text), setBadgeBackgroundColor: async () => {} },
  runtime: { onInstalled: ev('installed'), onStartup: ev('startup') },
  notifications: { create: (o) => calls.notify.push(o) },
};
Object.defineProperty(globalThis, 'navigator', { value: { userAgent: 'TestUA/1.0' }, configurable: true });

const { handleDownload } = await import('../background.js');

let nextId = 1;
const reset = () => { calls.cancel.length = calls.erase.length = calls.notify.length = 0; server.adds.length = 0; };
async function run(extra = {}, settings = {}, mode = 'ok') {
  Object.assign(store, { enabled: true, token: TOKEN, skipExtensions: [], skipDomains: [], minSizeMB: 0 }, settings);
  server.mode = mode; reset();
  const item = { id: nextId++, url: 'https://example.com/files/a.zip', referrer: 'https://example.com/', filename: '/home/u/Downloads/a.zip', fileSize: -1, ...extra };
  await handleDownload(item);
  return item;
}
const captured = () => server.adds.length === 1 && calls.cancel.length === 1 && calls.erase.length === 1;
const ignored = () => server.adds.length === 0 && calls.cancel.length === 0 && calls.erase.length === 0;

// (a)+(b) success
let item = await run({ finalUrl: 'https://cdn.example.com/a.zip' });
assert.equal(server.adds[0].auth, `Bearer ${TOKEN}`);
assert.deepEqual(server.adds[0].body, {
  url: 'https://cdn.example.com/a.zip', userAgent: 'TestUA/1.0', referrer: 'https://example.com/',
  filename: 'a.zip', cookies: 'a=b; c=d',
});
assert.deepEqual(calls.cancel, [item.id]); assert.deepEqual(calls.erase, [item.id]);
// same id twice is a no-op
const before = server.adds.length; await handleDownload(item); assert.equal(server.adds.length, before);

// (c) 401 -> fail open, notified
await run({}, {}, '401');
assert.equal(server.adds.length, 1); assert.equal(calls.cancel.length + calls.erase.length, 0);
assert.equal(calls.notify.length, 1);
// bad token also 401
await run({}, { token: 'wrong' }); assert.equal(calls.cancel.length, 0);

// server down
const port = srv.address().port; await new Promise((r) => srv.close(r));
await run({ id: nextId++ }); assert.equal(calls.cancel.length + calls.erase.length, 0);
assert.equal(calls.badge.at(-1), '!');
await new Promise((r) => srv.listen(port, '127.0.0.1', r));

// (d) ignored schemes
for (const url of ['blob:https://x.com/uuid', 'data:text/plain,hi', 'file:///tmp/a', 'filesystem:https://x/y', 'chrome-extension://abc/x']) {
  await run({ url }); assert.ok(ignored(), url);
}

// (e) settings
await run({}, { enabled: false }); assert.ok(ignored(), 'disabled');
await run({}, { skipDomains: ['example.com'] }); assert.ok(ignored(), 'skipDomain');
await run({ url: 'https://dl.example.com/a.zip' }, { skipDomains: ['example.com'] }); assert.ok(ignored(), 'skipDomain subdomain');
await run({}, { skipDomains: ['other.org'] }); assert.ok(captured(), 'other domain captured');
await run({}, { skipExtensions: ['.ZIP'] }); assert.ok(ignored(), 'skipExt');
await run({}, { skipExtensions: ['pdf'] }); assert.ok(captured(), 'other ext captured');
await run({ fileSize: 1024 }, { minSizeMB: 1 }); assert.ok(ignored(), 'below min size');
await run({ fileSize: 5 * 1024 * 1024 }, { minSizeMB: 1 }); assert.ok(captured(), 'above min size');
await run({ fileSize: -1 }, { minSizeMB: 1 }); assert.ok(captured(), 'unknown size captured');

console.log('harness: all assertions passed');
srv.close();
process.exit(0);
