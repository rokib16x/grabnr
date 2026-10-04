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

Phase 0 spike: prove that traffic can be pinned to a chosen interface and that two links add up.

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
