import { getSettings, saveSettings, ping, checkToken } from './lib.js';

const $ = (id) => document.getElementById(id);

async function refresh() {
  const [s, p] = await Promise.all([getSettings(), ping()]);
  $('enabled').checked = s.enabled;
  const el = $('state');
  if (!p.running) { el.textContent = 'grabnr is not running'; el.className = 'bad'; $('msg').textContent = 'Open the grabnr app. Until then, the browser downloads files itself.'; return; }
  // The app trusts this extension by its ID; a token is only needed for unofficial builds.
  const r = await checkToken(s.token);
  el.textContent = r === 'ok' ? 'Connected' : 'Running, but not authorised';
  el.className = r === 'ok' ? 'ok' : 'warn';
  $('msg').textContent = r === 'ok' ? '' : 'This build of the extension is not recognised. Open Options to enter a token.';
}

$('enabled').addEventListener('change', async (e) => {
  await saveSettings({ enabled: e.target.checked });
});
$('opts').addEventListener('click', (e) => {
  e.preventDefault();
  try { chrome.runtime.openOptionsPage(); } catch { chrome.tabs.create({ url: chrome.runtime.getURL('options.html') }); }
});

refresh();
