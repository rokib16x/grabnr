# grabnr browser extension

Manifest V3 extension that hands browser downloads to the grabnr desktop
download manager. Works in Chrome, Brave, Edge and Firefox (140 or newer).

## Install (unpacked)

1. Start the grabnr desktop app (the extension talks to `http://127.0.0.1:17653`).
2. Open `chrome://extensions` (Brave: `brave://extensions`, Edge: `edge://extensions`).
3. Enable **Developer mode**.
4. Click **Load unpacked** and select this `extension/` folder.

## Firefox

```bash
npm run build:extension      # writes extension/dist/grabnr-firefox.zip (and the Chrome zip)
```

1. Open `about:debugging#/runtime/this-firefox` and choose **Load Temporary Add-on**, then pick
   `extension/dist/firefox/manifest.json`. (A permanent install needs the add-on to be signed by Mozilla;
   Firefox Developer Edition and Nightly can install the zip unsigned.)
2. In the add-on's options, press **Grant site access**.
3. In the grabnr app press **Allow pairing**, then press **Pair** in the options. Firefox gives each
   install its own random address, so unlike Chrome it cannot be recognised automatically and needs this
   one-time pairing.

## Connecting

Nothing to pair. The manifest pins a public key, so this extension always has the ID
`nmfkcamnjeiknlpdpepoomkalaglenbb`, and grabnr trusts requests from exactly that extension
(browsers set the `Origin` header themselves; web pages cannot forge it). The popup shows
**Connected** when the app is running.

If the ID shown on the extensions page differs, you loaded a modified copy. In that case use
the **Advanced** section of the options page: press *Allow pairing* in grabnr, then **Pair**
(or paste the token), and use **Test connection** to verify.

**Tip:** Brave and Chrome can ask where to save every file. That dialog appears before the
extension sees the download, so turn off "Ask where to save each file" in the browser's
download settings for a seamless hand-off.

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
- Fail open: if grabnr is not running, not recognised, or errors, the browser keeps
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
