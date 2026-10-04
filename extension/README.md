# grabnr browser extension

Manifest V3 extension that hands browser downloads to the grabnr desktop
download manager. Chrome, Brave and Edge only for now (the manifest carries a
`browser_specific_settings` block for a possible Firefox port, but Firefox is
not supported or tested).

## Install (unpacked)

1. Start the grabnr desktop app (the extension talks to `http://127.0.0.1:17653`).
2. Open `chrome://extensions` (Brave: `brave://extensions`, Edge: `edge://extensions`).
3. Enable **Developer mode**.
4. Click **Load unpacked** and select this `extension/` folder.

## Pair

1. In the grabnr app, click **Allow pairing** (open for 60 seconds).
2. In the extension popup or options page, click **Pair**.

The token is stored in `chrome.storage.local`. You can also paste a token
manually in the options page, and use **Test connection** to verify it.

## How capture works

The service worker listens to `chrome.downloads.onCreated`. For each http(s)
download it sends the URL, referrer, cookies, user agent and suggested
filename to grabnr. Only when the app answers 200 does it cancel and erase the
browser's copy. The context menu entry **Download with grabnr** (links, images,
video, audio) sends the item with the page as referrer.

Toolbar badge: `OFF` when capture is disabled, red `!` when the app was
unreachable on the last attempt.

## Limits

- The browser briefly starts the download before it is cancelled; you may see a
  flash in the download bar, and a partial temp file is removed on cancel.
- Fail open: if grabnr is not running, not paired, or errors, the browser keeps
  the download. You get at most one quiet notification per minute.
- The app must be running for capture to work.
- `blob:`, `data:`, `file:`, `filesystem:` and extension URLs are never captured.
- There is no hold-Alt bypass for downloads; use the skip lists or the toggle.
- The minimum-size filter only applies when the browser reports a size at
  capture time, which is often not the case; unknown sizes are captured.

## Development

Icons: `python3 tools/make-icons.py`. Logic test (no browser needed):
`node test/harness.mjs`. The extension has not been loaded into a real browser
by the author of this scaffold; only the Node harness has been run.
