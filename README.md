# atk-tray-monitor

Lightweight Windows application for monitoring the battery of ATK-compatible mice with a minimal interface and a tray-first integration.

## Stack

- Angular 21
- Tailwind CSS 4
- Tauri 2
- Rust
- `libatk-rs` for ATK HID communication

## Current interface

- Dynamic title based on the detected mouse, with a cleaned product name when the HID interface exposes one
- Compact one-line status tag: `Charging`, `Battery`, `Offline`, `Connecting`, `Preview`
- Battery level with a circular gauge
- Small persistent battery history across launches
- Compact window designed to be opened from the tray icon, then automatically hidden

## Desktop behavior

- Tray-first application on Windows
- Main window is hidden instead of being closed
- Single-instance behavior
- Optional auto-start with Windows
- Main settings exposed in the tray menu
- Configurable low-battery notifications
- Optional automatic desktop updates through the Tauri updater plugin

## Battery reading

- Heuristic detection of ATK-compatible HID devices, prioritizing ATK vendor interfaces and the reverse-engineered protocol signature used by `libatk-rs`
- Battery reading through `libatk-rs`
- Automatic refresh every 20 seconds on both frontend and backend
- Defensive normalization of abnormal jumps observed when plugging in during charging

## Prerequisites

1. Recommended Node.js LTS
2. Rust via `rustup`
3. Visual Studio Build Tools with the Desktop C++ workload

## Development

Frontend only:

```bash
npm start
```

Tauri application:

```bash
npm run tauri:dev
```

Frontend build:

```bash
npm run build
```

Desktop build:

```bash
npm run tauri:build
```

## Auto-update setup

The project is configured to use GitHub Releases as the update source.

Default updater endpoint:

```text
https://github.com/NansMM/atk-tray-monitor/releases/latest/download/latest.json
```

What must exist at build time:

- `TAURI_SIGNING_PRIVATE_KEY` or `TAURI_SIGNING_PRIVATE_KEY_PATH`: private key used by Tauri to sign updater artifacts
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: optional password for that private key

Local PowerShell example:

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY_PATH = (Resolve-Path .\.secrets\tauri-updater.key).Path
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "<your-key-password>"
npm run tauri:build
```

GitHub Releases flow:

- push a tag like `v0.1.0`
- the `Release Desktop` workflow builds the app on GitHub Actions
- `tauri-action` uploads the installers, signatures, and `latest.json` to the GitHub release
- installed apps query `releases/latest/download/latest.json`

Required GitHub repository secrets:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

Notes:

- The updater public key is committed in `src-tauri/updater-public.key`, so only the private signing material stays in GitHub secrets.
- `bundle.createUpdaterArtifacts` is enabled, so release builds generate updater signatures alongside the normal installers.
- The Angular frontend checks for updates automatically a few seconds after startup, then periodically while the app remains open.
- `TAURI_UPDATER_ENDPOINTS` remains overridable, but defaults to the GitHub Releases endpoint above.

## GitHub Actions

- `CI` runs on every `push` to `main` and on every `pull_request`.
- This workflow installs Node.js 22 and stable Rust, then runs `npm run build` and `cargo check --manifest-path src-tauri/Cargo.toml`.
- `Release Desktop` runs manually or on a `v*` tag such as `v0.1.0`.
- On tags, it uses `tauri-action` to publish the Windows installers, updater signatures, and `latest.json` to the GitHub release.

## Notes

- A browser preview mode remains available to work on the UI without the Tauri runtime.
- If a raw HID dongle name is returned, the backend remaps it to a more readable product name for the interface.

## License

The backend uses `libatk-rs`, which is licensed under GPL-3.0. If you want to distribute the application as proprietary software, you will need to accept that constraint or replace this dependency with an in-house implementation or a more permissive alternative.
