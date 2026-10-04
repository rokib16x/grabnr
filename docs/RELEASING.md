# Releasing grabnr

## What a release contains

Pushing a tag such as `v0.2.0` runs `.github/workflows/release.yml`, which builds

- the desktop app for macOS (Apple silicon and Intel), Linux and Windows,
- the `grabnr` command line tool for macOS, Linux and Windows,
- the Chrome/Brave/Edge and Firefox extension zips,

and attaches them to a **draft** release. Nothing is public until you publish the draft.

Run the workflow by hand (Actions > Release > Run workflow) to try it without a tag: it builds everything and keeps the
results as workflow artifacts instead of creating a release.

## Before tagging

1. Update the version in `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and the crates, and move the
   notes in `CHANGELOG.md` from *Unreleased* to the new version.
2. Make sure CI is green on `main`.
3. Tag and push: `git tag v0.2.0 && git push origin v0.2.0`.
4. Open the draft release, check the files, edit the notes and publish.

## Repository secrets

None are required to build. Each one below unlocks one thing.

| Secret | Unlocks |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Signed update packages and `latest.json`, so installed apps can update themselves. Without them the build still works and the app is not updatable in place. |
| `MACOS_APP_CERT_P12`, `MACOS_CERT_PASSWORD`, `MACOS_KEYCHAIN_PASSWORD` | Signing the macOS app with your Developer ID Application certificate (a base64 `.p12`; the keychain password is any random string). |
| `NOTARY_KEY_ID`, `NOTARY_ISSUER_ID`, `NOTARY_KEY_P8` | Notarization with an App Store Connect API key (the `.p8`, base64). |

These are the same names the hidnr repository uses. GitHub never shows secret values back, so they are set once per repository.

### Update signing key

The public half is in `src-tauri/tauri.updater.conf.json` (`plugins.updater.pubkey`); the release workflow merges that file in only when the private-key secret exists, so builds without it have no updater. The private key was generated at `~/.tauri/grabnr.key` on the maintainer's machine; add its contents as the secret. Keep the private key secret and backed up:
if it is lost, installed copies can no longer be updated and users must reinstall. To make a new pair:

```bash
npx tauri signer generate -w ~/.tauri/grabnr.key
```

Put the private key's contents in the `TAURI_SIGNING_PRIVATE_KEY` secret and replace `pubkey` in the config.

## Dry run

Run the **Release** workflow by hand from the Actions tab (no tag). It builds every app, the CLI and the extensions and keeps
them as workflow artifacts; nothing is released. Pushing a `v*` tag makes a draft release you publish yourself.

## Unsigned builds

Without the Apple secrets the macOS app is not signed or notarized, and Gatekeeper refuses to open it the first time
("cannot be opened because the developer cannot be verified"). To open it anyway: Control-click the app, choose **Open**,
then **Open** again. Or remove the quarantine flag once: `xattr -dr com.apple.quarantine /Applications/grabnr.app`.

## Size and memory

The targets are an installer under about 15 MB and under 100 MB of memory when idle. `npm run tauri build` prints where the
bundles are; measure with `ls -l` on the `.dmg` and Activity Monitor (or `ps -o rss`) on the running app.
