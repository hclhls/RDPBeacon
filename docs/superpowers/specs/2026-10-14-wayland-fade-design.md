# Wayland Screen Fade Design

## Goal

Extend the existing screen fade (see `2026-10-14-screen-fade-design.md`) to
Wayland. When `fade_duration > 0`, all monitors fade to black before the beacon
sequence and fade back in afterwards, exactly as on Windows and X11.

## Agreed Decisions

- Mechanism: `zwlr_layer_shell_v1` overlay surfaces. Supported compositors are
  those implementing layer-shell (Sway, Hyprland, KDE Plasma, COSMIC).
- GNOME/Mutter and any other compositor without layer-shell are out of scope:
  the fade is skipped with a one-time warning and the beacon is still sent.
- A fullscreen `xdg_toplevel` is rejected: it would take keyboard focus and
  swallow the uinput beacon, which is delivered to the active window.
- Configuration, scheduler flow, and the `Backend` trait are unchanged.
  `WaylandBackend` overrides `fade_out` and `fade_in`.
- No new config option, CLI flag, or tray entry (YAGNI).

## Rendering Approach

One layer-shell surface per `wl_output`:

- Layer `overlay`, anchored to all four edges, exclusive zone `-1`, size
  `0x0` (compositor chooses the output size).
- Keyboard interactivity `none` and an empty input region, so focus and input
  are never affected and clicks pass through.
- Content: a single 1x1 premultiplied ARGB8888 black `wl_shm` pixel, stretched to
  the output with `wp_viewporter`. Per frame, a new 1x1 buffer with the current
  alpha is attached and committed (4 bytes per frame regardless of resolution).
- Alpha comes from the shared `fade_alpha(elapsed, duration)` (fade-out) and
  `255 - fade_alpha(...)` (fade-in), stepped every `FADE_STEP`.

## Architecture

A new module `src/backend/wayland_fade.rs` (Linux only) keeps the protocol code
out of `wayland.rs`.

- `Session` holds the Wayland connection, event queue, bound globals, and the
  per-output overlays. It is created lazily on the first `fade_out` and kept in
  a `Mutex<FadeSlot>` (`Uninit`, `Unsupported`, `Ready(Session)`) on
  `WaylandBackend`, as X11 does with its overlay window.
- `fade_out(duration)` and `fade_in(duration)` run the animation inline on the
  caller's thread. If binding fails (no `zwlr_layer_shell_v1` or no
  `wp_viewporter`) or the connection fails, a one-time warning is logged, the
  slot becomes `Unsupported`, and later calls are no-ops.
- `Session`'s `Drop` destroys the overlays, so a crash or early exit never
  leaves the screen black.
- Outputs hot-plugged during a fade are ignored until the next fade.

Pure helper: `premultiplied_black_pixel(alpha: u8) -> [u8; 4]` returns the
little-endian ARGB8888 bytes for a black pixel at the given opacity (RGB is
zero because premultiplied black is always zero). It is the unit-tested part.

## Dependencies

Linux-only additions to `Cargo.toml`: `wayland-client`, `wayland-protocols`
(viewporter), `wayland-protocols-wlr` (layer-shell). Shared memory uses
`rustix` (already a dependency) for `memfd_create`/`mmap`.

## Error Handling

- Missing protocols or connection failure: warn once, skip fade, return `Ok`.
- Overlay creation failure mid-way: destroy anything created and return `Err`;
  the scheduler already logs a warning and sends the beacon without fading.
- Failure during the animation: destroy the overlay so `fade_in` becomes a
  no-op.
- `fade_in` always destroys the overlay, even on error.

## Testing

- Unit tests for `premultiplied_black_pixel` (alpha 0, 255, midpoint).
- Unit tests that `fade_out` with zero duration and `fade_in` without a prior
  `fade_out` return `Ok` on a `test_backend()` without opening a connection.
- Scheduler tests are unchanged; they already cover the fade ordering.
- Rendering cannot be unit tested. Update the README fade smoke-test
  checklist: Wayland on a layer-shell compositor (fade visible, beacon still
  delivered, input not blocked) and on GNOME (skipped with a warning, beacon
  sent). Update the README `fade_duration` row and Wayland limitations.
