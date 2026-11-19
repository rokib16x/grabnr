# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- **Sleep:** grabnr keeps the computer from going to sleep on its own while something is downloading (Preferences → System; on by default). Closing the lid still sleeps.

### Fixed
- **Torrents:** peers that are choking us are no longer dropped after 10 seconds of quiet. Each drop pushed that peer's next try further out (up to an hour), so downloads ran in short bursts and slowly dried up.
- **Torrents:** a torrent that receives nothing for 90 seconds (after sleep, or a network drop) rebuilds its peer list and asks the trackers and DHT again instead of sitting at 0 B/s for hours.
- **Torrents:** links that go away and come back (Wi-Fi after sleep, a cable plugged in) are followed while a torrent runs; peer connections whose far end vanished are closed instead of being held open forever.
- **Downloads:** a short network outage no longer fails the download. Failed pieces go to the back of the queue instead of being retried by every connection at once, network errors count against a piece only while other pieces are arriving, and with no network at all grabnr waits for it to come back.
- **Downloads:** a server that throttled a link (HTTP 429/503) gets connections back a minute after it stops, instead of leaving the link on one connection for the rest of the download.
- **Downloads:** a connection that died during sleep can no longer hang the first request or a single-stream download forever.
- **Window:** the Connections list and the link count now pick up links that join or leave while the window is open (Wi-Fi coming back after wake), instead of showing only the links present when it opened.
- **Downloads:** automatic retries start over after a run that made progress, so a long download is not marked failed because of drops hours apart.

## [0.1.4] - 2026-11-14

### Added
- **Torrents:** with several links, peer connections are spread over all of them, so even a single large file downloads over Wi-Fi and Ethernet at once.

### Changed
- **Look:** new logo and app icon, Plus Jakarta Sans and JetBrains Mono fonts, Phosphor icons, a speed overview strip, redesigned file tiles and download rows.
- **Browser extension:** new icons; version now follows the app.

### Fixed
- **Sidebar:** high speeds and long link names no longer widen the sidebar or add a horizontal scrollbar.

## [0.1.3] - 2026-10-08

### Fixed
- **Downloads:** the progress figure no longer falls behind the real download. Bytes saved mid-chunk were counted twice as they were taken off the running total, so the percentage and size lagged until you paused and resumed.

## [0.1.2] - 2026-10-04

### Fixed
- **Browser extension:** a download is now taken before the browser opens its own "Save as" panel, so the panel no longer appears next to grabnr's copy.
- **Browser extension:** downloads handed over from the browser open grabnr's Add Download sheet first (folder, name, checksum, sign-in), as when you add a link yourself. Turn this off in the extension options ("Ask in grabnr before starting").
- **Window:** long file names no longer push the list wider than the window.

## [0.1.1] - 2026-10-04

First signed release: the macOS apps are signed with a Developer ID certificate and notarized by Apple, and installed apps can check for and install updates. Everything below was added since 0.1.0.

### Added
- **Updates:** Preferences has Check for updates, which installs a signed update and restarts (active in release builds that carry the update key). The main window is built only when opened, so the app idles at about 33 MB of its own memory.
- **Releases:** the Release workflow can be run by hand as a dry run, and docs/RELEASING.md lists the secrets and steps.
- **Engine:** speed limits (overall and per link), checksum verification (SHA-256, SHA-1, MD5), disk-space check, adaptive connections per link, slow-link protection at the end of a download.
- **Reliability:** links that join or leave a running download, restart on a changed IP, stalled-connection recovery, revival of failed links, automatic retries with growing delays.
- **Platforms:** Linux and Windows builds in CI and the release workflow, per-platform window settings, one running copy at a time, sleep and shell actions for each system, popover placement for a taskbar at the bottom of the screen.
- **Languages:** the UI is available in English, Spanish, French, German, Portuguese and Bengali, chosen in Preferences or from the system language.
- **macOS integration:** a Safari extension build and a Services menu workflow ("Download with grabnr").
- **Browsers:** a Firefox build of the extension (`npm run build:extension`), and expired links re-captured from the browser continue the failed download instead of starting a new one.
- **Torrents:** magnet links and `.torrent` files. Multi-file torrents are split by file over your connections; the browser extension leaves `.torrent` files to the browser by default.
- **Protocols:** FTP and SFTP downloads, split over every link like web downloads. SFTP remembers server keys and refuses a changed one; user names and passwords typed into a link are kept out of the list.
- **Streams:** HLS (`.m3u8`) downloads, with a quality picker, AES-128 decryption, resume, and conversion to MP4 when ffmpeg is installed.
- **Sources:** mirrors, Metalink (`.meta4`, `.metalink`), batch link lists, a page link picker, per-download sign-in (Basic, Bearer) and proxies (HTTP, HTTPS, SOCKS5).
- **Queue:** "Download next / last" priority, drag-to-reorder, duplicate detection.
- **Integrations:** a webhook for finished and failed downloads, `grabnr://add?url=` links, a macOS quarantine flag on finished files, thumbnails in the inspector, settings and link-list export/import, a copyable diagnostics report.
- **Resume:** progress inside a piece is saved and continued after a pause or crash, so nothing but the last MiB is ever fetched twice; finished pieces are checked by checksum when a download resumes, so a crash or power loss can no longer leave silent holes in the file; a replaced link keeps its progress (Change link…).
- **Security:** optional Keychain storage for credentials.
- **Schedule:** rules that hold downloads, cap the speed or lift the cap by day and hour; an action to run when all downloads finish (sleep, quit or a command).
- **App:** macOS-style window with sidebar, categories, sorting, inspector, History with statistics and CSV export, drag and drop, first-run onboarding, command palette, menu bar popover, Dock progress, notification sound.
- **Project:** UI end-to-end tests, CI, release workflow, brand assets, contributing and security docs.

### Changed
- Partial downloads from earlier builds restart once: resume state is now keyed per download and carries piece checksums.
- "Connections per link" is now the maximum; each link starts with four and grows while it helps.
- The UI was redesigned around an Apple-style layout.

### Fixed
- Turning every link back on in Manage Connections now sticks. The "all links" choice was being ignored when saved.
- The general speed limit now applies to all downloads together instead of to each one separately.
- **Security:** sign-in headers and cookies were sent to the redirected host and to mirrors on other hosts. They now go only to the host you gave, over the same scheme and port.
- The inspector no longer shows below the list on narrow windows.

## [0.1.0]

Initial prototype: download engine, desktop app, menu bar icon, browser extension.
