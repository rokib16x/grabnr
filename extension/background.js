import { getSettings, addDownload, shouldCapture, buildRequest, cookieHeaderFor, isCapturableUrl } from './lib.js';

const NOTIFY_INTERVAL_MS = 60_000;
let lastNotify = 0;
// Ids already processed. Lost on service-worker restart, which is fine: a
// restarted worker only sees new onCreated events, and cancel is idempotent.
const handled = new Set();

async function refreshBadge(unreachable) {
  const { enabled } = await getSettings();
  if (!enabled) {
    await chrome.action.setBadgeText({ text: 'OFF' });
    await chrome.action.setBadgeBackgroundColor({ color: '#6b7280' });
  } else if (unreachable) {
    await chrome.action.setBadgeText({ text: '!' });
    await chrome.action.setBadgeBackgroundColor({ color: '#dc2626' });
  } else {
    await chrome.action.setBadgeText({ text: '' });
  }
}

function quietNotify(message) {
  const now = Date.now();
  if (now - lastNotify < NOTIFY_INTERVAL_MS) return;
  lastNotify = now;
  chrome.notifications.create({
    type: 'basic',
    iconUrl: 'icons/icon128.png',
    title: 'grabnr',
    message,
    silent: true,
  });
}

function failMessage(r) {
  if (r.status === 0) return 'grabnr is not running; the browser is downloading this file.';
  if (r.status === 401) return 'grabnr is not paired; open the extension options to pair.';
  return 'grabnr could not take this download; the browser is handling it.';
}

export async function handleDownload(item) {
  if (handled.has(item.id)) return;
  handled.add(item.id);
  const settings = await getSettings();
  if (!shouldCapture(item, settings)) return;

  const r = await addDownload(await buildRequest(item), settings.token);
  if (r.ok) {
    await refreshBadge(false);
    // Only after the app accepted it do we drop the browser's copy.
    try { await chrome.downloads.cancel(item.id); } catch { /* may be already done */ }
    try { await chrome.downloads.erase({ id: item.id }); } catch { /* ignore */ }
  } else {
    // Fail open: leave the browser download alone.
    await refreshBadge(true);
    quietNotify(failMessage(r));
  }
}

async function handleMenu(info) {
  const url = info.linkUrl || info.srcUrl;
  if (!isCapturableUrl(url)) return;
  const settings = await getSettings();
  const req = { url, userAgent: navigator.userAgent };
  if (info.pageUrl) req.referrer = info.pageUrl;
  const cookies = await cookieHeaderFor(url);
  if (cookies) req.cookies = cookies;
  const r = await addDownload(req, settings.token);
  if (r.ok) await refreshBadge(false);
  else { await refreshBadge(true); quietNotify(failMessage(r)); }
}

function createMenu() {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: 'grabnr-download',
      title: 'Download with grabnr',
      contexts: ['link', 'video', 'audio', 'image'],
    });
  });
}

chrome.downloads.onCreated.addListener((item) => { handleDownload(item); });
chrome.contextMenus.onClicked.addListener((info) => { handleMenu(info); });
chrome.runtime.onInstalled.addListener(() => { createMenu(); refreshBadge(false); });
chrome.runtime.onStartup.addListener(() => refreshBadge(false));
chrome.storage.onChanged.addListener((changes) => { if (changes.enabled) refreshBadge(false); });
