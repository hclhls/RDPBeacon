# RDPBeacon Design

## Goal
A Rust CLI daemon that periodically sends a tiny, harmless input ("beacon") to a logged-in Omnissa Horizon Client session on the local machine, preventing idle timeout disconnects.

## Scope and Assumptions
- Horizon Client runs locally on Linux (X11), Linux (Wayland) or Windows, with an already logged-in session.
- Beacon = simulated local input (key, default F15; or 1 px mouse nudge and back). No network-level keep-alive.
- Own-use tool. It only helps against input-inactivity timeouts, not hard session lifetime limits set by an admin. The user must ensure use complies with their organisation's policy.
- Output: a single Rust binary with a CLI.

## Behaviour
- Fire only when the user has been idle for at least `idle_threshold` (the user's own input already resets the timeout).
- X11 and Windows: save the active window, focus the Horizon window, send the beacon, wait ~100 ms, restore the saved window.
- Wayland: no focus switching (not possible compositor-neutrally). Send a global beacon via `/dev/uinput`; this relies on Horizon being the active window. Documented limitation. Mouse nudge is the fallback mode.

## Architecture
Single crate, trait-based backends.

- `config`: TOML file plus CLI flags. Fields: `interval`, `jitter`, `idle_threshold`, `key` (default F15), `mode` (key|mouse), `window_match` (default "Omnissa Horizon Client"), `max_misses` (default 5), `backend` override.
- `scheduler`: loop with `interval ± jitter`; skips the cycle when idle time is below the threshold; depends only on the `Backend` trait.
- `Backend` trait: `idle_time()`, `find_horizon_window()`, `active_window()`, `focus(window)`, `send_beacon(mode)`.
  - `x11`: `x11rb` with XTest and XScreenSaver extensions, and EWMH `_NET_ACTIVE_WINDOW`.
  - `wayland`: `evdev` with `/dev/uinput`; idle time via `ext-idle-notify` where supported.
  - `windows`: `windows` crate; `SendInput`, `GetLastInputInfo`, `EnumWindows`, `SetForegroundWindow`.
- `main`: subcommands `run`, `once` (send a single beacon), `check` (report detected backend and Horizon window).

## Data Flow (one cycle)
1. Sleep `interval ± jitter`.
2. `idle_time()` below `idle_threshold`: skip and log at debug level.
3. `find_horizon_window()`.
4. X11 and Windows: save the active window, focus Horizon, send the beacon, wait ~100 ms, restore. Wayland: send the beacon directly.
5. Log the result and loop.

## Error Handling
- Window not found: warn and skip; after `max_misses` consecutive misses, exit with a clear message (optionally keep retrying via config).
- uinput permission denied: print the fix (udev rule or `input` group); never auto-escalate.
- Windows foreground lock: if `SetForegroundWindow` fails, fall back to global input and log it.
- Backend detection via `XDG_SESSION_TYPE` and `WAYLAND_DISPLAY`; `--backend` overrides it. Clean exit on SIGINT or Ctrl-C.
- Beacon safety: F15 default; mouse mode moves 1 px and back.

## Testing
- Unit tests: scheduler, jitter bounds and idle-threshold logic against a mock `Backend`; config parsing.
- Manual per-OS smoke tests: `check`, `once`, then `run` with a short interval against a session with a short idle timeout.
- CI: `cargo build` and `cargo test` with `cfg`-gated backends on Linux and Windows runners.

## Non-Goals (YAGNI)
- No GUI or tray UI, no macOS support, no network-protocol keep-alive, no compositor-specific Wayland focus APIs.
