# Contributing to grabnr

Thanks for helping. Small, focused pull requests are easiest to review.

## Set up

You need Rust (stable), Node 22 and, for the desktop app, macOS with Xcode command line tools.

```bash
npm install
npm run tauri dev          # run the desktop app
cargo run -p grabnr-cli -- links
```

The UI also runs in a plain browser with a mock backend (`npm run dev`, then open http://localhost:1420), which is the quickest way to iterate on layout. The mock lives in `src/mock.ts`.

## Before you open a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run check               # type-check, UI tests, extension harness
```

CI runs the same checks.

## Where things are

| Path | What |
|---|---|
| `crates/grabnr-core` | The download engine. Most logic and most tests live here. |
| `crates/grabnr-cli` | The `grabnr` command line tool. |
| `src-tauri` | The desktop app: queue, settings, history, menu bar, local API for the extension. |
| `src` | React UI. |
| `extension` | Chrome/Brave/Edge extension. |

## Guidelines

- **Test engine behaviour against the local range server** in `crates/grabnr-core/tests/download.rs`. It can fail requests, stall connections, require auth and count bytes per server. Prefer a test there over reasoning about timing.
- **Keep the UI honest.** Do not add a control the backend cannot honour. If a feature is partly built, say so in `FEATURES.md`.
- **Credentials never go to the UI.** Headers, proxies and tokens stay in the backend (`ItemView` omits them on purpose).
- **No telemetry**, and no network calls except the ones a download or a user action needs.
- Match the surrounding code: short comments that explain why, not what.
- Do not commit generated files or build output (`dist`, `target`, `node_modules`).

## Reporting problems

Use the issue templates. For anything security related, follow [SECURITY.md](SECURITY.md) instead of opening a public issue.
