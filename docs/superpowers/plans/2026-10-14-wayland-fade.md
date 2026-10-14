# Wayland Screen Fade Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `fade_duration` work on Wayland compositors that implement `zwlr_layer_shell_v1`, skipping with a one-time warning elsewhere.

**Architecture:** A new Linux-only module `src/backend/wayland_fade.rs` owns a lazily created `Session` (Wayland connection, event queue, bound globals, per-output layer-shell overlays). `WaylandBackend` holds it in a `Mutex<FadeSlot>` and overrides `fade_out`/`fade_in`, mirroring how `X11Backend` keeps its overlay. Each animation frame attaches a fresh 1x1 premultiplied ARGB8888 black `wl_shm` buffer, stretched to the output via `wp_viewporter`.

**Tech Stack:** Rust 2021, `wayland-client` 0.31, `wayland-protocols` 0.32 (viewporter), `wayland-protocols-wlr` 0.3 (layer-shell), `rustix` (`memfd_create`).

**Spec:** `docs/superpowers/specs/2026-10-14-wayland-fade-design.md`

**Deviation from spec (needs the spec amended in Task 3):** the spec describes a dedicated worker thread. A prototype showed the connection, queue, and surfaces are `Send`, so the session lives behind a `Mutex` and `fade_out`/`fade_in` run the animation inline, like X11. This is simpler and has the same behavior.

## Global Constraints

- Linux only: all new dependencies go under `[target.'cfg(target_os = "linux")'.dependencies]`.
- Code comments, docs, and commit messages are in English.
- Commit messages use Conventional Commits and end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
- Overlay: layer `overlay`, anchored to all four edges, exclusive zone `-1`, size `0x0`, keyboard interactivity `none`, empty input region.
- Reuse `fade_alpha` and `FADE_STEP` from `src/backend/fade.rs`; fade-in alpha is `255 - fade_alpha(...)`.
- Missing layer-shell or viewporter, or connection failure: warn once, return `Ok(())`, behave as no-op afterwards.
- The overlay must be destroyed in `fade_in` and on `Drop`.
- `cargo check` must be warning-free; `cargo test` must pass.

## Review Focus

- `fade_duration = 0ms` on Wayland: no connection is opened and nothing is drawn (test in Task 3).
- `fade_in` called without a prior successful `fade_out` (fade_out failed or was skipped): returns `Ok` and does nothing (test in Task 3).
- No `WAYLAND_DISPLAY` or no layer-shell (GNOME): warning logged once, beacon still sent, never an error that aborts the cycle (covered by `Session::connect` returning `Ok(None)` / backend marking `Unsupported`; verified by the manual smoke test in Task 3).
- Overlay never takes focus or input: keyboard interactivity `none` plus empty input region; verified by the manual smoke test in Task 3.
- Screen left black after a crash or error mid-fade: `Session::destroy` runs on animation error and in `Drop` (Task 2).

---

### Task 1: Dependencies and pixel helper

**Files:**
- Modify: `Cargo.toml` (Linux dependency table)
- Create: `src/backend/wayland_fade.rs`
- Modify: `src/backend/mod.rs` (module declaration, next to `mod wayland;`)

**Interfaces:**
- Produces: `pub fn premultiplied_black_pixel(alpha: u8) -> [u8; 4]` in `crate::backend::wayland_fade`; returns little-endian ARGB8888 bytes `[B, G, R, A]`.

- [ ] **Step 1: Add dependencies**

In `Cargo.toml`, change the `rustix` line and add three crates in the Linux table:

```toml
rustix = { version = "1", features = ["process", "stdio", "fs"] }
wayland-client = "0.31"
wayland-protocols = { version = "0.32", features = ["client"] }
wayland-protocols-wlr = { version = "0.3", features = ["client"] }
```

- [ ] **Step 2: Declare the module**

Run: `grep -n "mod wayland" src/backend/mod.rs`
Directly below the existing Linux `wayland` module declaration (keep its `#[cfg]` attribute style), add the same declaration for `wayland_fade`, e.g.:

```rust
#[cfg(target_os = "linux")]
pub mod wayland_fade;
```

- [ ] **Step 3: Write the failing test**

Create `src/backend/wayland_fade.rs`:

```rust
/// Returns the bytes of one little-endian ARGB8888 black pixel with the given opacity.
/// Premultiplied black always has zero colour channels, so only alpha varies.
pub fn premultiplied_black_pixel(alpha: u8) -> [u8; 4] {
    todo!("alpha = {alpha}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_pixel_is_all_zero() {
        assert_eq!(premultiplied_black_pixel(0), [0, 0, 0, 0]);
    }

    #[test]
    fn opaque_pixel_is_black_with_full_alpha() {
        assert_eq!(premultiplied_black_pixel(255), [0, 0, 0, 255]);
    }

    #[test]
    fn midpoint_keeps_colour_channels_zero() {
        assert_eq!(premultiplied_black_pixel(128), [0, 0, 0, 128]);
    }
}
```

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test wayland_fade`
Expected: FAIL (panics with `not yet implemented`).

- [ ] **Step 5: Implement**

Replace the `todo!` body with `[0, 0, 0, alpha]`.

- [ ] **Step 6: Run tests**

Run: `cargo test wayland_fade && cargo check`
Expected: 3 tests PASS. Dead-code warning for the new function is acceptable until Task 2 only if `cargo check` shows nothing else; add `#[allow(dead_code)]` is NOT needed because Task 2 uses it immediately.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/backend/mod.rs src/backend/wayland_fade.rs
git commit -m "feat(wayland): add fade pixel helper and Wayland dependencies

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Layer-shell `Session`

**Files:**
- Modify: `src/backend/wayland_fade.rs` (add everything above the `#[cfg(test)]` module)

**Interfaces:**
- Consumes: `premultiplied_black_pixel(alpha: u8) -> [u8; 4]` (Task 1); `crate::backend::fade::{fade_alpha, FADE_STEP}`.
- Produces (all `pub` on `Session`):
  - `Session::connect() -> anyhow::Result<Option<Session>>`: `Ok(None)` when layer-shell, viewporter, compositor, or shm globals are missing.
  - `Session::show(&mut self) -> Result<()>`: creates one configured overlay per output; on failure leaves no overlays.
  - `Session::animate(&mut self, duration: Duration, fading_out: bool) -> Result<()>`
  - `Session::has_overlay(&self) -> bool`
  - `Session::destroy(&mut self)`: idempotent; also called from `Drop`.

This module talks to a live compositor, so it has no unit tests beyond Task 1. It is verified by compiling here and by the smoke test in Task 3.

- [ ] **Step 1: Insert the implementation**

Put the imports at the top of `src/backend/wayland_fade.rs` (above the existing `premultiplied_black_pixel`), and the rest between that function and `#[cfg(test)]`:

```rust
use anyhow::{Context, Result};
use std::io::Write;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};
use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::{
    wl_buffer::{self, WlBuffer},
    wl_compositor::WlCompositor,
    wl_output::WlOutput,
    wl_region::WlRegion,
    wl_registry::WlRegistry,
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

use crate::backend::fade::{fade_alpha, FADE_STEP};
```

```rust
/// One black layer-shell surface covering a single output.
struct Overlay {
    surface: WlSurface,
    viewport: WpViewport,
    layer: ZwlrLayerSurfaceV1,
}

/// Dispatch state; counts configured layer surfaces and maps each one to its viewport
/// so the configure handler can stretch the 1x1 buffer to the output size.
struct State {
    configured: usize,
    viewports: Vec<(ZwlrLayerSurfaceV1, WpViewport)>,
}

/// A Wayland connection plus the globals needed to draw the fade overlay.
pub struct Session {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    compositor: WlCompositor,
    shm: WlShm,
    viewporter: WpViewporter,
    layer_shell: ZwlrLayerShellV1,
    outputs: Vec<WlOutput>,
    overlays: Vec<Overlay>,
}

impl Session {
    /// Connects and binds the required globals.
    /// Returns `Ok(None)` when the compositor lacks layer-shell or viewporter support.
    pub fn connect() -> Result<Option<Self>> {
        let conn = Connection::connect_to_env().context("Failed to connect to Wayland display")?;
        let (globals, mut queue) =
            registry_queue_init::<State>(&conn).context("Failed to enumerate Wayland globals")?;
        let qh = queue.handle();

        let compositor = bind_global(&globals, &qh, 4..=6);
        let shm = bind_global(&globals, &qh, 1..=1);
        let viewporter = bind_global(&globals, &qh, 1..=1);
        let layer_shell = bind_global(&globals, &qh, 1..=4);
        let (Some(compositor), Some(shm), Some(viewporter), Some(layer_shell)) =
            (compositor, shm, viewporter, layer_shell)
        else {
            return Ok(None);
        };

        let outputs: Vec<WlOutput> = globals
            .contents()
            .clone_list()
            .iter()
            .filter(|g| g.interface == "wl_output")
            .map(|g| globals.registry().bind(g.name, g.version.min(4), &qh, ()))
            .collect();

        let mut state = State {
            configured: 0,
            viewports: Vec::new(),
        };
        queue
            .roundtrip(&mut state)
            .context("Wayland roundtrip failed")?;

        Ok(Some(Self {
            conn,
            queue,
            state,
            compositor,
            shm,
            viewporter,
            layer_shell,
            outputs,
            overlays: Vec::new(),
        }))
    }

    pub fn has_overlay(&self) -> bool {
        !self.overlays.is_empty()
    }

    /// Creates one overlay per output and waits until the compositor configures all of them.
    pub fn show(&mut self) -> Result<()> {
        self.destroy();
        self.state.configured = 0;
        let qh = self.queue.handle();

        for output in &self.outputs {
            let surface = self.compositor.create_surface(&qh, ());
            let viewport = self.viewporter.get_viewport(&surface, &qh, ());
            let layer = self.layer_shell.get_layer_surface(
                &surface,
                Some(output),
                Layer::Overlay,
                "rdpbeacon-fade".into(),
                &qh,
                (),
            );
            layer.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
            layer.set_exclusive_zone(-1);
            layer.set_size(0, 0);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);

            // An empty input region makes the overlay click-through.
            let region = self.compositor.create_region(&qh, ());
            surface.set_input_region(Some(&region));
            region.destroy();

            self.state.viewports.push((layer.clone(), viewport.clone()));
            surface.commit();
            self.overlays.push(Overlay {
                surface,
                viewport,
                layer,
            });
        }

        if self.overlays.is_empty() {
            anyhow::bail!("No Wayland outputs found for the fade overlay");
        }

        for _ in 0..10 {
            self.queue.roundtrip(&mut self.state)?;
            if self.state.configured >= self.overlays.len() {
                return Ok(());
            }
        }
        self.destroy();
        anyhow::bail!("Compositor did not configure the fade overlay")
    }

    /// Attaches a fresh 1x1 black buffer with the given opacity to every overlay.
    fn set_alpha(&mut self, alpha: u8) -> Result<()> {
        let qh = self.queue.handle();
        let mut file = std::fs::File::from(
            rustix::fs::memfd_create("rdpbeacon-fade", rustix::fs::MemfdFlags::CLOEXEC)
                .context("memfd_create failed")?,
        );
        file.write_all(&premultiplied_black_pixel(alpha))
            .context("Failed to write fade pixel")?;

        let pool = self.shm.create_pool(file.as_fd(), 4, &qh, ());
        let buffer = pool.create_buffer(0, 1, 1, 4, wl_shm::Format::Argb8888, &qh, ());
        // The buffer stays valid after the pool is destroyed.
        pool.destroy();

        for o in &self.overlays {
            o.surface.attach(Some(&buffer), 0, 0);
            o.surface.damage_buffer(0, 0, 1, 1);
            o.surface.commit();
        }
        self.conn
            .flush()
            .context("Failed to flush Wayland connection")?;
        self.queue.roundtrip(&mut self.state)?;
        Ok(())
    }

    /// Animates opacity over `duration`: 0 -> 255 when `fading_out`, else 255 -> 0.
    pub fn animate(&mut self, duration: Duration, fading_out: bool) -> Result<()> {
        let start = Instant::now();
        loop {
            let elapsed = start.elapsed();
            let alpha = fade_alpha(elapsed, duration);
            self.set_alpha(if fading_out { alpha } else { 255 - alpha })?;
            if elapsed >= duration {
                return Ok(());
            }
            std::thread::sleep(FADE_STEP);
        }
    }

    /// Destroys all overlays. Safe to call repeatedly.
    pub fn destroy(&mut self) {
        for o in self.overlays.drain(..) {
            o.viewport.destroy();
            o.layer.destroy();
            o.surface.destroy();
        }
        self.state.viewports.clear();
        let _ = self.conn.flush();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.destroy();
    }
}

fn bind_global<I>(
    globals: &GlobalList,
    qh: &QueueHandle<State>,
    versions: std::ops::RangeInclusive<u32>,
) -> Option<I>
where
    I: Proxy + 'static,
    State: Dispatch<I, ()>,
{
    globals.bind(qh, versions, ()).ok()
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            layer.ack_configure(serial);
            if let Some((_, viewport)) = state.viewports.iter().find(|(l, _)| l == layer) {
                viewport.set_destination(width as i32, height as i32);
            }
            state.configured += 1;
        }
    }
}

impl Dispatch<WlBuffer, ()> for State {
    fn event(
        _: &mut Self,
        buffer: &WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            buffer.destroy();
        }
    }
}

delegate_noop!(State: WlCompositor);
delegate_noop!(State: WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: WlSurface);
delegate_noop!(State: WlRegion);
delegate_noop!(State: ignore WlOutput);
delegate_noop!(State: WpViewporter);
delegate_noop!(State: WpViewport);
delegate_noop!(State: ZwlrLayerShellV1);
```

- [ ] **Step 2: Compile**

Run: `cargo check 2>&1 | tail -30`
Expected: no errors. Dead-code warnings for `Session` items are expected until Task 3 wires them in; do not silence them.

- [ ] **Step 3: Commit**

```bash
git add src/backend/wayland_fade.rs
git commit -m "feat(wayland): add layer-shell fade overlay session

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Backend integration, docs, and smoke test

**Files:**
- Modify: `src/backend/wayland.rs` (struct, `new`, `test_backend`, trait impl, tests)
- Modify: `README.md` (fade_duration table row; Wayland limitations; smoke-test item at ~line 273)
- Modify: `config.example.toml` (fade_duration comment)
- Modify: `CHANGELOG.md` (`## [Unreleased]`)
- Modify: `docs/superpowers/specs/2026-10-14-wayland-fade-design.md` (worker thread -> inline `Mutex`)

**Interfaces:**
- Consumes: `Session::{connect, show, animate, destroy}` (Task 2).
- Produces: `WaylandBackend::fade_out/fade_in` overrides of `Backend`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/backend/wayland.rs`:

```rust
    #[test]
    fn fade_is_noop_when_duration_is_zero() {
        let backend = WaylandBackend::test_backend();
        backend.fade_out(Duration::ZERO).unwrap();
        backend.fade_in(Duration::ZERO).unwrap();
        assert!(matches!(*backend.fade.lock().unwrap(), FadeSlot::Uninit));
    }

    #[test]
    fn fade_in_without_fade_out_is_noop() {
        let backend = WaylandBackend::test_backend();
        backend.fade_in(Duration::from_millis(300)).unwrap();
        assert!(matches!(*backend.fade.lock().unwrap(), FadeSlot::Uninit));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test wayland_backend 2>&1 | tail`; or `cargo test fade_is_noop`
Expected: compile error (`FadeSlot` / `fade` not defined).

- [ ] **Step 3: Implement**

In `src/backend/wayland.rs` add to the imports `use crate::backend::wayland_fade::Session;` and define:

```rust
/// Lazily created fade session. `Unsupported` is sticky after one warning.
enum FadeSlot {
    Uninit,
    Unsupported,
    Ready(Session),
}
```

Add `fade: Mutex<FadeSlot>,` to `WaylandBackend`, initialise it with `Mutex::new(FadeSlot::Uninit)` in both `new()` and `test_backend()`, then add to `impl Backend for WaylandBackend`:

```rust
    fn fade_out(&self, duration: Duration) -> Result<()> {
        if duration.is_zero() {
            return Ok(());
        }
        let mut slot = self
            .fade
            .lock()
            .map_err(|e| anyhow::anyhow!("Fade lock poisoned: {e}"))?;

        if matches!(*slot, FadeSlot::Uninit) {
            *slot = match Session::connect() {
                Ok(Some(session)) => FadeSlot::Ready(session),
                Ok(None) => {
                    log::warn!(
                        "Wayland compositor lacks zwlr_layer_shell_v1 or wp_viewporter; skipping screen fade"
                    );
                    FadeSlot::Unsupported
                }
                Err(e) => {
                    log::warn!("Cannot set up Wayland screen fade ({e:#}); skipping");
                    FadeSlot::Unsupported
                }
            };
        }

        if let FadeSlot::Ready(session) = &mut *slot {
            let result = session
                .show()
                .and_then(|_| session.animate(duration, true));
            if result.is_err() {
                session.destroy();
            }
            return result;
        }
        Ok(())
    }

    fn fade_in(&self, duration: Duration) -> Result<()> {
        let mut slot = self
            .fade
            .lock()
            .map_err(|e| anyhow::anyhow!("Fade lock poisoned: {e}"))?;
        if let FadeSlot::Ready(session) = &mut *slot {
            if session.has_overlay() {
                let result = session.animate(duration, false);
                session.destroy();
                return result;
            }
        }
        Ok(())
    }
```

`Session` has its own `Drop` that destroys overlays, so `WaylandBackend` needs no extra `Drop`.

- [ ] **Step 4: Run tests**

Run: `cargo test && cargo check`
Expected: all tests PASS, no warnings (the Task 2 dead-code warnings are gone).

- [ ] **Step 5: Update docs**

- `README.md` fade_duration row (~line 166): replace "Wayland and compositor-less X11 skip with a warning." with "Wayland requires a compositor with `wlr-layer-shell` (Sway, Hyprland, KDE Plasma, COSMIC); GNOME and compositor-less X11 skip with a warning."
- `README.md` Wayland Limitations (~line 200): add a bullet noting the fade works only on layer-shell compositors and is skipped on GNOME.
- `README.md` smoke test (~line 273): replace the single Wayland bullet with two: **Wayland with layer-shell**: the screen fades to black on all outputs, the beacon is still delivered to the active window, clicks pass through, the screen fades back in, no overlay remains. **Wayland without layer-shell (GNOME)**: fade skipped with a warning (`Wayland compositor lacks zwlr_layer_shell_v1 or wp_viewporter; skipping screen fade`), beacon sent normally.
- `config.example.toml`: run `grep -n -B2 -A2 fade_duration config.example.toml` and update the comment the same way as the README row.
- `CHANGELOG.md` under `## [Unreleased]`:

```markdown
### Added

- Wayland: screen fade-out and fade-in via `wlr-layer-shell` overlays on every output (Sway, Hyprland, KDE Plasma, COSMIC). Compositors without layer-shell, such as GNOME, skip the fade with a one-time warning.
```

- Spec: in `docs/superpowers/specs/2026-10-14-wayland-fade-design.md`, replace the worker-thread paragraphs under "Architecture" with: the session is created lazily on the first `fade_out`, held in `Mutex<FadeSlot>` (`Uninit`, `Unsupported`, `Ready(Session)`) on `WaylandBackend`, and `fade_out`/`fade_in` run the animation inline on the caller's thread; `Session`'s `Drop` destroys the overlays.

- [ ] **Step 6: Smoke test on a layer-shell compositor (manual, requires Sway/Hyprland/KDE/COSMIC)**

Run: `cargo run -- --backend wayland --config <(printf 'fade_duration = "500ms"\n') -v once`
Expected: smooth fade to black, a beacon (verify with `wev` or a mouse nudge), smooth fade back in, process exits, no black layer remains. If no such compositor is available, state that explicitly in the PR instead of claiming it was tested. On GNOME, expect the one-time warning and a normal beacon.

- [ ] **Step 7: Commit**

```bash
git add src/backend/wayland.rs README.md config.example.toml CHANGELOG.md docs/superpowers/specs/2026-10-14-wayland-fade-design.md
git commit -m "feat(wayland): fade screen around beacon via layer-shell

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
