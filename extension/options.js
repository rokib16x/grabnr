import { getSettings, saveSettings, ping, pair, checkToken } from './lib.js';

const $ = (id) => document.getElementById(id);
const say = (t) => { $('msg').textContent = t; };
const lines = (s) => s.split(/[\n,]+/).map((x) => x.trim()).filter(Boolean);

async function load() {
  const s = await getSettings();
  $('enabled').checked = s.enabled;
  $('token').value = s.token;
  $('skipDomains').value = s.skipDomains.join('\n');
  $('skipExtensions').value = s.skipExtensions.join('\n');
  $('minSizeMB').value = s.minSizeMB;
}

$('pair').addEventListener('click', async () => {
  const r = await pair();
  if (r.ok) { say('Paired. Token saved.'); await load(); }
  else if (r.reason === 'pairing_closed') say('Pairing is closed. Click "Allow pairing" in the grabnr app, then press Pair again within 60 seconds.');
  else if (r.reason === 'not_running') say('grabnr is not running.');
  else say('Pairing failed.');
});

$('test').addEventListener('click', async () => {
  const p = await ping();
  if (!p.running) return say('grabnr is not running.');
  const r = await checkToken($('token').value.trim());
  say(r === 'ok' ? `Connected (grabnr ${p.version}).`
    : r === 'bad_token' ? 'Running, but this extension is not authorised. Use the Advanced token section.'
    : r === 'not_running' ? 'grabnr is not running.' : 'Unexpected response from grabnr.');
});

$('save').addEventListener('click', async () => {
  await saveSettings({
    enabled: $('enabled').checked,
    token: $('token').value.trim(),
    skipDomains: lines($('skipDomains').value),
    skipExtensions: lines($('skipExtensions').value),
    minSizeMB: Math.max(0, Number($('minSizeMB').value) || 0),
  });
  say('Saved.');
});

load();
