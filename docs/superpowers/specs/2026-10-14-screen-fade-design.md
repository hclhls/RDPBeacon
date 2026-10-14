# Screen Fade Around Beacon Design

## Goal

Reduce the visual annoyance of a beacon cycle by optionally fading the whole
screen (all monitors) to black before the beacon sequence and fading back in
afterwards. The visible disturbance comes mainly from focusing the Horizon
window, sending the beacon, and restoring focus; the fade covers that whole
block.

## Agreed Decisions

- Scope: all monitors are faded to black.
- Platforms: Windows and X11 are implemented. Wayland skips the fade and logs
  a warning (no standard compositor-wide overlay is available).
- Configuration: a single `fade_duration` setting. `0ms` (default) disables the
  feature; any other value is used for both fade-out and fade-in.
- X11 without a compositing manager: skip the fade and log a warning.
- No tray menu entry and no CLI flag (YAGNI).
- Approach: a black full-screen overlay whose opacity is animated.

## Configuration

- `Config::fade_duration: Duration`, parsed with `humantime-serde`, default
  `"0ms"`.
- Documented in `config.example.toml` and the README.

## Backend Interface

Add to the `Backend` trait in `src/backend/mod.rs`:

- `fn fade_out(&self, duration: Duration) -> Result<()>`
- `fn fade_in(&self, duration: Duration) -> Result<()>`

Default implementations log a warning and return `Ok(())`, so Wayland needs no
override.

The overlay created by `fade_out` stays visible until `fade_in` completes. It is
held in backend state (`Mutex<Option<Overlay>>`) and is destroyed when `fade_in`
finishes and on `Drop`, so a crash or early exit never leaves the screen black.

Both calls are no-ops when `duration` is zero or, on X11, when no compositing
manager owns `_NET_WM_CM_S0` (a warning is logged once).

A pure, platform-independent helper `fade_alpha(elapsed, duration) -> u8`
computes the opacity and is shared by the backends.

## Scheduler Flow

In `run_cycle`, only when the cycle will actually send a beacon and
`fade_duration > 0`:

1. `fade_out`
2. `focus(horizon)` (if needed)
3. `send_beacon`
4. `focus(prev)` (if needed)
5. `fade_in`

Rules:

- Skipped cycles never fade.
- A `fade_out` failure logs a warning and the beacon is still sent.
- `fade_in` always runs, even if `focus` or `send_beacon` failed. A `fade_in`
  failure logs a warning, forcibly destroys the overlay, and does not fail the
  cycle.
- A stop request during a fade does not abort it; the cycle completes and fades
  in.

## Platform Implementations

- Windows (`src/backend/windows.rs`): one black window covering the virtual
  screen with `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TRANSPARENT |
  WS_EX_TOOLWINDOW`. Alpha is stepped with `SetLayeredWindowAttributes` roughly
  every 16 ms. `WS_EX_TRANSPARENT` lets input pass through.
- X11 (`src/backend/x11.rs`): one override-redirect black window covering the
  root, opacity set through `_NET_WM_WINDOW_OPACITY` at the same step interval,
  with an empty input shape so input passes through.
- Wayland: default (skip with warning).

## Error Handling

- Overlay creation failure: the backend returns `Err`; the scheduler logs a
  warning and continues without fading (the beacon is still sent).
- Failure during `fade_out`: destroy the overlay so `fade_in` becomes a no-op.

## Testing

Unit tests in `#[cfg(test)]` modules:

- Scheduler mock backend records fade calls; verify order
  `fade_out, focus(horizon), send_beacon, focus(prev), fade_in`.
- No fade calls on skipped cycles or when `fade_duration` is zero.
- `fade_in` still called when `send_beacon` fails.
- Beacon still sent when `fade_out` fails.
- `fade_alpha` boundaries: zero elapsed, midpoint, elapsed >= duration, zero
  duration.
- Config parsing and the `0ms` default.

Rendering cannot be unit tested; add Windows and X11 (with and without a
compositor) items to the README smoke-test checklist.
