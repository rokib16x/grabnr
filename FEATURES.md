# grabnr features

`[x]` built and covered by tests or checked in the app, `[~]` partly built, `[ ]` not started.
Anything marked built has not yet been proven on two real links with different gateways; see README.

## 1. Network and interfaces
- [x] Discover Wi-Fi, Ethernet, USB/phone tethering and cellular links, with IP, gateway and type
- [~] Link speed and signal strength (link speed when the OS reports it; no signal strength)
- [x] Enable/disable links; per-link share of connections and per-link speed limit
- [x] Bind sockets to an interface (`IP_BOUND_IF`), IPv4 (IPv6 not yet)
- [x] Detect links sharing a gateway and warn that they will not add bandwidth
- [x] Exclude VPN/tunnel links by default
- [x] Per-link speed test (`spike`) and in-app link test
- [x] Hot-plug: new links join a running download, vanished links drain, changed IPs restart the link
- [x] Recover from sleep/wake and dead connections (stall timeout, link revival)
- [~] Rules: leave cellular out unless ticked, per-link caps. No "prefer Ethernet" ordering yet
- [ ] Metered-connection awareness and per-link data caps (macOS gives no reliable signal)

## 2. Download engine
- [x] HTTP/HTTPS over HTTP/1.1 (HTTP/2 is avoided on purpose: it would put every worker on one TCP connection)
- [ ] FTP, SFTP (needs a client that uses interface-bound sockets)
- [x] Range probe with single-connection fallback
- [x] 1-8 MB chunks, shared work-stealing queue
- [x] Persistent connections per link, grown and shrunk automatically
- [x] Tail racing: duplicate the slowest chunk, first copy wins
- [x] Slow-link protection: a much slower link stays out of the last chunks
- [x] Retry with exponential backoff; back off on 429/503/403
- [x] Preallocated file, writes at offsets, atomic rename on completion
- [x] Redirects, `Content-Disposition` filenames
- [x] Basic and Bearer auth, custom headers, cookies and referrer (from the browser extension)
- [x] HTTP, HTTPS and SOCKS5 proxies, per download and as a default
- [x] Mirrors and Metalink (`.meta4`, `.metalink`)
- [x] Batch URL lists and a page link grabber
- [~] HLS (`.m3u8`) video-on-demand: master playlists with a quality choice, AES-128, byte ranges, fMP4, resume per segment, optional MP4 via ffmpeg. No live streams, SAMPLE-AES or DASH; links cannot join mid-download yet
- [ ] BitTorrent and magnet links
- [ ] Folder/archive downloads with auto-extract

## 3. Reliability and integrity
- [x] Pause/resume across quit and crash, state in SQLite
- [x] ETag/Last-Modified check; a changed file discards the partial download
- [x] SHA-256 / SHA-1 / MD5 verification, typed in or from a Metalink file
- [x] Per-piece checksums: on resume every finished piece is re-read and checked, damaged ones are downloaded again
- [x] "Change link" for an expired or replaced link: keeps the pieces already downloaded if it is the same file
- [x] Resume inside a piece: progress within a piece is saved about every MiB and continues from there, re-checked by checksum
- [ ] Resume for servers without range support (those restart from zero)
- [x] Disk-space check before starting; partial-file cleanup on delete

## 4. Queue and scheduling
- [x] Concurrent download limit, "Download next / last" priority, drag-to-reorder in queue order
- [x] Overall and per-link speed limits
- [x] Automatic retry of failed downloads, with growing delays
- [x] Duplicate detection
- [x] Time-of-day schedule: hold downloads, cap the speed or lift the cap by day and hour, including overnight windows
- [x] One speed limit shared by all downloads, adjustable while they run
- [~] When all downloads finish: sleep, quit or run a command (no shut down)

## 5. User interface
- [x] macOS-style window: sidebar, download list, inspector, translucent sidebar, light and dark
- [x] Per-download progress, speed, ETA, size and per-link share bar
- [x] Chunk map coloured by link; live speed graph per link
- [x] Search, status filters, file-type categories, sorting
- [x] Drag and drop of links and text files; clipboard link detection; paste to add
- [x] Menu bar icon with live speed and a popover with active downloads
- [x] Dock progress and badge, notifications with optional sound
- [x] Command palette (⌘K), shortcuts (⌘N, ⌘,)
- [x] First-run onboarding with setup checks
- [x] History and statistics (bytes per link, estimated time saved), CSV export
- [~] Thumbnails of finished files in the inspector (macOS Quick Look; none for files it cannot preview)
- [ ] Localization

## 6. macOS integration
- [x] Chrome/Brave/Edge extension
- [ ] Safari and Firefox extensions
- [x] Launch at login (starts hidden in the menu bar)
- [~] `grabnr://add?url=` links (works in the packaged app; not testable in dev). No Share Sheet extension or Finder menu yet
- [x] Quarantine attribute on finished files (setting, on by default)
- [~] Release workflow builds the app; signing and notarization need an Apple Developer account
- [ ] Auto-update

## 7. Automation and developer features
- [x] CLI on the same engine (`links`, `get`, `spike`)
- [x] Local REST API for the extension, bound to 127.0.0.1
- [~] Webhook: a JSON event for each finished or failed download. No WebSocket events yet
- [x] Settings export/import (without secrets) and a link-list export of the queue
- [~] Diagnostics report you can copy (no live debug panel)

## 8. Security and privacy
- [x] No telemetry
- [x] TLS validation through rustls
- [x] Credentials kept in a 0600 file, never sent to the UI
- [~] Optional Keychain storage for sign-in headers and proxy passwords (the extension token stays in a 0600 file)

## 9. Quality
- [x] Unit tests, local range-server integration tests (mirrors, auth, proxy, hot-plug, stalls)
- [x] CI: format, clippy, tests on macOS/Linux, UI type-check and tests
- [x] UI end-to-end tests (Playwright, against the mock backend; the native shell is not covered)
- [ ] Installer under about 15 MB and under 100 MB RAM idle: not measured yet

## 10. Platforms
- [x] macOS (Apple silicon and Intel)
- [~] Linux and Windows: the engine builds in CI; interface binding is only proven on macOS
