# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- **Engine:** speed limits (overall and per link), checksum verification (SHA-256, SHA-1, MD5), disk-space check, adaptive connections per link, slow-link protection at the end of a download.
- **Reliability:** links that join or leave a running download, restart on a changed IP, stalled-connection recovery, revival of failed links, automatic retries with growing delays.
- **Sources:** mirrors, Metalink (`.meta4`, `.metalink`), batch link lists, a page link picker, per-download sign-in (Basic, Bearer) and proxies (HTTP, HTTPS, SOCKS5).
- **Queue:** "Download next / last" priority, duplicate detection.
- **App:** macOS-style window with sidebar, categories, sorting, inspector, History with statistics and CSV export, drag and drop, first-run onboarding, command palette, menu bar popover, Dock progress, notification sound.
- **Project:** CI, release workflow, brand assets, contributing and security docs.

### Changed
- "Connections per link" is now the maximum; each link starts with four and grows while it helps.
- The UI was redesigned around an Apple-style layout.

### Fixed
- **Security:** sign-in headers and cookies were sent to the redirected host and to mirrors on other hosts. They now go only to the host you gave, over the same scheme and port.
- The inspector no longer shows below the list on narrow windows.

## [0.1.0]

Initial prototype: download engine, desktop app, menu bar icon, browser extension.
