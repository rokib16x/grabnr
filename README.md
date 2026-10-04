# grabnr

Download faster by using every network connection at once: Wi-Fi, Ethernet, USB or phone tethering.
Files are split into chunks, and a worker bound to each interface pulls chunks from a shared queue,
so faster links take more chunks.

Built with Tauri 2 and Rust (small installer, low memory). macOS first, then Windows and Linux.

## Layout

| Path | What |
|---|---|
| `crates/grabnr-core` | Engine: interface discovery, binding (`IP_BOUND_IF`), transfers |
| `crates/grabnr-cli` | `grabnr` CLI built on the same engine |
| `src-tauri` | Tauri app shell |
| `src` | React + TypeScript UI |
| `FEATURES.md` | Full feature list |

## Status

Working now (`grabnr-core`, tested):
- Probe (size, range support, ETag/Last-Modified, filename), redirects resolved once
- Shared work-stealing chunk queue (1-8 MB), tail racing, writes at file offsets, atomic rename
- Per-link worker pools with persistent HTTP/1.1 connections, retry with backoff, throttle back-off on 429/503/403
- A failing link drops out and the others take its chunks
- Pause/resume across restarts (SQLite); a changed file discards the partial download instead of mixing versions
- Single-stream fallback for servers without range support

Not done yet: the Tauri download list UI, adaptive connection growth, checksums, FTP/SFTP/torrents, the rest of `FEATURES.md`.

Still to prove on real hardware: that traffic splits across two links with different gateways.

```bash
cargo run -p grabnr-cli -- get https://proof.ovh.net/files/100Mb.dat -o ~/Downloads
cargo test -p grabnr-core
```

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
