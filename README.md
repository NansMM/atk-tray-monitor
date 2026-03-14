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

## GitHub Actions

- `CI` runs on every `push` to `main` and on every `pull_request`.
- This workflow installs Node.js 22 and stable Rust, then runs `npm run build` and `cargo check --manifest-path src-tauri/Cargo.toml`.
- `Release Desktop` runs manually or on a `v*` tag such as `v0.1.0`.
- This workflow builds the Windows Tauri bundles (`.msi` and NSIS `.exe` installer), publishes them as artifacts, and automatically attaches them to the GitHub release on tags.

## Notes

- A browser preview mode remains available to work on the UI without the Tauri runtime.
- If a raw HID dongle name is returned, the backend remaps it to a more readable product name for the interface.

## License

The backend uses `libatk-rs`, which is licensed under GPL-3.0. If you want to distribute the application as proprietary software, you will need to accept that constraint or replace this dependency with an in-house implementation or a more permissive alternative.
