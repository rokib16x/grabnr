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

Built and tested (see [FEATURES.md](FEATURES.md) for the full list):
- **Engine:** shared chunk queue over every link, tail racing, adaptive connections per link, resume across restarts, retries with backoff, throttle handling, mirrors and Metalink, checksums, speed limits, proxies and sign-in, links that join or leave mid-download, recovery from stalled connections.
- **App:** macOS-style window with a sidebar, categories, inspector, chunk map and per-link speed graph; menu bar popover; history and statistics; onboarding; command palette; Dock progress.
- **Browser extension** for Chrome, Brave and Edge.

Not done yet: FTP/SFTP, torrents, HLS, the scheduler, Safari and Firefox extensions, signed builds and auto-update, Windows and Linux apps.

**Not yet proven on real hardware:** that traffic splits across two links with different gateways. The engine is tested against a local range server and the interface binding is checked by `grabnr spike`, but nobody has yet measured the speed-up on two physical connections.

## Develop

```bash
npm install
npm run tauri dev        # the desktop app
npm run check            # UI type-check and tests, extension harness
cargo test --workspace   # engine, CLI and app tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

## CLI

```bash
cargo run -p grabnr-cli -- links
cargo run -p grabnr-cli -- get https://proof.ovh.net/files/100Mb.dat -o ~/Downloads
cargo run -p grabnr-cli -- get URL --only en0,en5 --limit 20 --checksum sha256:<hex>
cargo run -p grabnr-cli -- get URL --mirror https://mirror.example/file --proxy socks5://127.0.0.1:1080
cargo run -p grabnr-cli -- get https://example.org/file.meta4   # a Metalink file adds its mirrors and hash
cargo run -p grabnr-cli -- get https://example.org/video.m3u8 --quality 720   # a streaming video, split over every link
cargo run -p grabnr-cli -- spike --secs 8
```

The spike reports the public IP seen through each link in each bind mode.
`Bound` means egress differs from the default route. `SameEgress` means the same ISP, or the bind was ignored.
Proving that bandwidth adds up needs two links with different gateways.

## Browser extension

Load `extension/` unpacked (see `extension/README.md`). No pairing: grabnr trusts the extension by its fixed ID.
Downloads started in the browser are sent to grabnr and the browser copy is cancelled. If grabnr is not running, the browser downloads normally.
The app listens on `127.0.0.1:17653` only, refuses requests from web pages, and accepts `/add` only from the official extension's ID or with a token.
