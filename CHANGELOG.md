# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Each version heading matches its git tag without the leading `v`
(for example, `## [0.4]` is released as tag `v0.4`). The release workflow
publishes the matching section as the GitHub Release notes.

## [Unreleased]

## [0.6.1] - 2026-10-14

### Fixed

- Scheduler: restore screen via RAII `FadeGuard` on error or unwinding panic so the screen is never left dark.
- Windows: isolate overlay HWND lifecycle and message pump onto a dedicated owner thread to ensure thread-safe window destruction and reliable message dispatch across threads.
- Windows: enable per-monitor DPI awareness (`DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2`) so the overlay covers all monitors accurately across mixed DPI configurations.
- Windows: release overlay mutex before joining the overlay thread during teardown.
- X11: query dynamic root window geometry on each fade cycle to support hotplugged monitors and resolution changes.
- X11: validate `map_window` synchronously to catch and handle X server errors immediately.

## [0.6] - 2026-10-14

### Added

- Optional screen fade-out and fade-in effect around beacon cycles (`fade_duration`, default `"0ms"` / disabled).
  - All monitors are faded to black before the beacon sequence (including window focus switching) and faded back in immediately after, reducing visual annoyance.
  - Windows: transparent layered full-screen overlay covering all monitors with input pass-through.
  - X11: override-redirect root overlay with opacity animation via `_NET_WM_WINDOW_OPACITY` and empty input shape for click pass-through.
  - Wayland and compositor-less X11 environments automatically skip the effect with a single warning.

## [0.5] - 2026-10-14

### Added

- Tray menu controls for **Beacon interval** (`1m`, `2m`, `4m`, `5m`, `10m`) and
  **Reminder before beacon** (`Off`, `10s`, `30s`, `1m`) on Linux and Windows.
  Changes apply from the next wait and are not written to the config file. A
  configured value that is not a preset is added to the menu.

### Changed

- The default `interval` is now `"10m"` (previously `"4m"`).

### Known limitations

- The Windows tray submenus have not been verified on a real desktop yet.

## [0.4] - 2026-10-14

### Added

- Pre-beacon desktop notification. Set `notify_before` (for example `"30s"`) to
  be warned that a beacon is about to be sent. It is disabled by default
  (`"0s"`). If the wait is shorter than this value, the notice appears
  immediately.
  - Linux: sent through the `org.freedesktop.Notifications` D-Bus service.
  - Windows: shown as a tray balloon notification.

### Known limitations

- The Windows notification path has not been verified on a real desktop yet.

## [0.3] - 2026-10-13

### Added

- Tray daemon mode: `rdpbeacon run --daemon` detaches from the terminal and
  keeps a tray icon with controls (including Quit).
- Linux tray via StatusNotifierItem and Windows tray via the notification area.
- Optional rotating log file for daemon mode, toggled from the tray menu.
- Startup acknowledgement for the detached process, so launch failures are
  reported instead of being lost.
- Validation notes in `docs/tray-daemon-validation.md`.

### Fixed

- Hardened tray ownership and concurrent logging in daemon mode.

### Upgrade notes

- Linux daemon mode requires a tray host. Startup fails if none is available.

## [0.2] - 2026-10-08

### Changed

- Beacons now use mouse mode by default (`mode = "mouse"`).
- The default key for key mode is now `Shift_L` (previously `F15`).
- When you are active in another local window, the Horizon session is still
  kept alive in the background. Cycles are skipped only while you are active
  inside the Horizon window itself.

### Fixed

- More reliable key beacons: added focus settle and dwell delays and send dual
  pulses.

### Upgrade notes

- To keep the previous behavior, set `mode = "key"` and `key = "F15"` explicitly.

## [0.1] - 2026-10-07

Initial public release.

### Added

- Beacon scheduler with a configurable `interval` and random `jitter`, plus
  idle-time checks (`idle_threshold`) and a `max_misses` shutdown guard.
- Platform backends: X11 (XTest), Wayland (`/dev/uinput`), and Windows.
- Horizon window detection by title or class (`window_match`), preferring the
  remote session window over launcher windows.
- Focus restoration: the previously active window regains focus after a beacon
  (X11 and Windows).
- CLI subcommands `check`, `once`, and `run`, with `-v` / `--verbose` debug logging.
- TOML configuration (`config.example.toml`), Apache-2.0 license, and CI for
  Linux and Windows.

[Unreleased]: https://github.com/hclhls/RDPBeacon/compare/v0.6.1...HEAD
[0.6.1]: https://github.com/hclhls/RDPBeacon/compare/v0.6...v0.6.1
[0.6]: https://github.com/hclhls/RDPBeacon/compare/v0.5...v0.6
[0.5]: https://github.com/hclhls/RDPBeacon/compare/v0.4...v0.5
[0.4]: https://github.com/hclhls/RDPBeacon/compare/v0.3...v0.4
[0.3]: https://github.com/hclhls/RDPBeacon/compare/v0.2...v0.3
[0.2]: https://github.com/hclhls/RDPBeacon/compare/v0.1...v0.2
[0.1]: https://github.com/hclhls/RDPBeacon/releases/tag/v0.1
