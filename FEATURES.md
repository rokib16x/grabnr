# grabnr features

## 1. Network and interfaces
- Discover Wi-Fi, Ethernet, USB/phone tethering, Thunderbolt/USB Ethernet and cellular dongles
- Per-interface IP, gateway, type, link speed, signal and status
- Enable/disable and weight per interface
- Bind sockets to an interface (`IP_BOUND_IF`), IPv4 and IPv6
- Verify the bind works; label `BOUND` or `MULTI_CONNECTION`
- Detect interfaces sharing a gateway or ISP and warn that they won't add bandwidth
- Detect VPN/virtual interfaces and allow excluding them
- Built-in per-interface speed test
- Hot-plug: new links join a running download, removed links drain safely
- Sleep/wake, IP change and Wi-Fi roaming recovery
- Metered-connection awareness and per-interface data caps
- Per-interface rules (for example never use cellular, prefer Ethernet)

## 2. Download engine
- HTTP/HTTPS (HTTP/1.1, HTTP/2), FTP, SFTP
- Range probe with single-connection fallback
- Adaptive 1-8 MB chunks, shared work-stealing queue
- Multiple persistent connections per interface, grown and shrunk automatically
- Tail racing: duplicate the slowest chunks, first copy wins
- Slow-link protection: a mismatched link cannot drag the total below the best single link
- Retry with exponential backoff; back off on 429/503/403
- Preallocated file, writes at offsets, atomic rename on completion
- Redirects, `Content-Disposition` filenames
- Auth (Basic, Bearer), headers, cookies, referrer, user agent
- HTTP and SOCKS proxies, per download
- Mirrors and Metalink
- Batch URL lists and page link grabber
- HLS/DASH streams
- BitTorrent and magnet links over multiple interfaces, selective files
- Folder/archive downloads with optional auto-extract

## 3. Reliability and integrity
- Pause/resume across quit and crash, state in SQLite
- ETag/Last-Modified check; refuse to resume if the file changed
- SHA-256/SHA-1/MD5 verification, typed in or from the server
- Per-chunk verification and re-download
- Disk-space check, temp-file cleanup, repair/re-verify

## 4. Queue and scheduling
- Priorities and reordering, concurrent download limit
- Overall and per-interface speed limits
- Time-of-day scheduler and bandwidth profiles
- On-complete actions (quit, sleep, shut down, run script)
- Auto-retry failed downloads, duplicate detection

## 5. User interface
- Download list: progress, speed, ETA, size, per-interface share
- Chunk grid coloured by interface
- Live speed graph per interface
- Detail view with connections and log
- Sort, filter, search, categories
- Drag-and-drop links/files, clipboard URL detection
- Menu bar icon with live speed, Dock progress
- Light/dark, notifications, sounds, shortcuts, command palette
- Onboarding with setup checks
- Localization

## 6. macOS integration
- Browser extensions (Chrome, Safari, Firefox)
- URL scheme handler, Share Sheet extension, Finder context menu
- Spotlight, quarantine attribute handling, launch at login
- Signed and notarized builds, auto-update

## 7. Automation and developer features
- CLI on the same engine
- Local REST and WebSocket API
- Event hooks and webhooks
- Queue/settings import and export
- Logs, diagnostics export, debug panel

## 8. History and data
- Searchable history, statistics (bytes per interface, time saved), CSV export

## 9. Security and privacy
- No telemetry by default
- TLS validation with clear errors
- Credentials in the macOS Keychain
- Sandbox-friendly design

## 10. Quality goals
- Installer under about 15 MB, under 100 MB RAM idle
- Scheduler unit tests, local range-server integration tests, UI end-to-end tests
- CI for build, test, sign, release
