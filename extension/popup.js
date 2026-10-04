import { getSettings, saveSettings, ping, pair, checkToken } from './lib.js';

const $ = (id) => document.getElementById(id);

async function refresh() {
  const [s, p] = await Promise.all([getSettings(), ping()]);
  $('enabled').checked = s.enabled;
  const el = $('state');
  if (!p.running) { el.textContent = 'Not running'; el.className = 'bad'; return; }
  // The app's "paired" flag is app-wide; confirm that *our* token works.
  const ok = s.token && (await checkToken(s.token)) === 'ok';
  el.textContent = ok ? 'Connected' : 'Running, not paired';
  el.className = ok ? 'ok' : 'warn';
}

$('enabled').addEventListener('change', async (e) => {
  await saveSettings({ enabled: e.target.checked });
});
$('pair').addEventListener('click', async () => {
  const r = await pair();
  $('msg').textContent = r.ok ? 'Paired.'
    : r.reason === 'pairing_closed' ? 'Click "Allow pairing" in the grabnr app first, then try again.'
    : r.reason === 'not_running' ? 'grabnr is not running.' : 'Pairing failed.';
  refresh();
});
$('opts').addEventListener('click', (e) => { e.preventDefault(); chrome.runtime.openOptionsPage(); });

refresh();
