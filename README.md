<p align="center"><img src="assets/brand/readme-banner.svg" alt="grabnr" width="720"></p>

Download faster by using every network connection at once: Wi-Fi, Ethernet, USB or phone tethering.
Files are split into chunks, and a worker bound to each interface pulls chunks from a shared queue,
so faster links take more chunks.

Built with Tauri 2 and Rust (small installer, low memory). macOS first, then Windows and Linux.

## Layout

| Path | What |
|---|---|
| `crates/grabnr-core` | Engine: interface discovery, binding (`IP_BOUND_IF`), transfers |
| `crates/grabnr-cli` | `grabnr` CLI built on the same engine |
| `src-tauri` | Tauri app: download manager, queue, local API for the browser extension |
| `src` | React + TypeScript UI (download list, chunk grid, per-link speed graph, settings) |
| `extension` | Chrome/Brave/Edge extension that hands browser downloads to the app |
| `FEATURES.md` | Full feature list |

## Status

Working now (`grabnr-core`, tested):
- Probe (size, range support, ETag/Last-Modified, filename), redirects resolved once
- Shared work-stealing chunk queue (1-8 MB), tail racing, writes at file offsets, atomic rename
- Per-link worker pools with persistent HTTP/1.1 connections, retry with backoff, throttle back-off on 429/503/403
- A failing link drops out and the others take its chunks
- Pause/resume across restarts (SQLite); a changed file discards the partial download instead of mixing versions
- Single-stream fallback for servers without range support

Also working: the desktop app (queue, pause/resume, notifications, per-link share bar, chunk grid, speed graph, settings) and the browser-extension capture API.

Also: menu bar icon with live speed (open, pause all, resume all, quit), closing the window keeps grabnr running, and an optional launch-at-login that starts hidden.

Not done yet: Firefox/Safari extensions, adaptive connection growth, checksums, FTP/SFTP/torrents, the rest of `FEATURES.md`.

Still to prove on real hardware: that traffic splits across two links with different gateways.

```bash
cargo run -p grabnr-cli -- get https://proof.ovh.net/files/100Mb.dat -o ~/Downloads
cargo test -p grabnr-core
npm install && npm run tauri dev
node extension/test/harness.mjs
```

## Browser extension

Load `extension/` unpacked (see `extension/README.md`). No pairing: grabnr trusts the extension by its fixed ID.
Downloads started in the browser are sent to grabnr and the browser copy is cancelled. If grabnr is not running, the browser downloads normally.
The app listens on `127.0.0.1:17653` only, refuses requests from web pages, and accepts `/add` only from the official extension's ID or with a token.

```bash
cargo run -p grabnr-cli -- links
cargo run -p grabnr-cli -- spike --secs 8
npm install && npm run tauri dev
```

The spike reports the public IP seen through each link in each bind mode.
`Bound` means egress differs from the default route. `SameEgress` means the same ISP, or the bind was ignored.
Proving that bandwidth adds up needs two links with different gateways.

## License

MIT
