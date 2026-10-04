# Security policy

## Reporting a vulnerability

Please report security problems privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability**. Do not open a public issue for it.

Include what you found, how to reproduce it, and which version you tested. You will get an answer within a few days. Fixes ship in the next release, and the report is credited unless you prefer otherwise.

## What is in scope

grabnr runs a small HTTP API on `127.0.0.1:17653` so the browser extension can hand it downloads. Reports about that API are especially welcome:

- A web page, or another local user, being able to start downloads or read data without the extension or the token.
- Credentials (sign-in headers, cookies, proxies, the API token) leaking into the UI, logs, history or the CSV export.
- Path traversal or overwriting files outside the chosen download folder.
- Anything that makes grabnr download from, or send data to, a host the user did not ask for.

## How grabnr protects data today

- The API listens on loopback only, rejects requests from web pages, and accepts `/add` only from the official extension's ID or with a token.
- Credentials and the token are stored in files readable only by your user (mode `0600`) and are never sent to the UI.
- Downloads are verified with TLS through rustls. grabnr has no telemetry.
- Cookies and `Authorization` headers are not forwarded to other hosts after a redirect.

Known gaps, tracked in `FEATURES.md`: credentials are not yet in the macOS Keychain, and release builds are not yet signed or notarized.
