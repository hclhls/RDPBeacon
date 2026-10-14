# Screen Fade Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Optionally fade all monitors to black before a beacon sequence and back in afterwards, controlled by a single `fade_duration` setting.

**Architecture:** `Backend` gains `fade_out`/`fade_in` with warn-and-skip defaults (Wayland keeps the defaults). `run_cycle` wraps the existing focus/send/restore block between them. X11 and Windows implement a black full-screen overlay whose opacity is animated using a shared pure helper `fade_alpha`.

**Tech Stack:** Rust 2021, `x11rb` 0.13 (Linux), `windows` 0.58 (Windows), `humantime-serde`.

**Spec:** `docs/superpowers/specs/2026-10-14-screen-fade-design.md`

## Global Constraints

- `fade_duration` default is `"0ms"` (disabled); one value is used for both fade-out and fade-in.
- Windows and X11 only; Wayland skips with a warning (trait default).
- X11 without a compositing manager (`_NET_WM_CM_S0` unowned): skip with a warning.
- No tray menu entry and no CLI flag.
- Platform APIs stay inside `src/backend/{x11,windows}.rs` with existing conditional compilation.
- Overlay must never outlive the fade: destroyed on `fade_in` completion, on failure, and on `Drop`.
- Comments, docs and commit messages in English; Conventional Commits.
- `cargo check` must be warning-free and `cargo test` green after every task.

## Review Focus

- `fade_duration` given as an invalid string (e.g. `"abc"`) is rejected at config load like other durations.
- `send_beacon` failure: `fade_in` still runs and the error is still returned.
- `focus(horizon)` failure: `fade_in` still runs (overlay never left black).
- `fade_out` failure: beacon still sent, cycle outcome `Sent`.
- `fade_in` failure: cycle still returns `Sent`.
- `fade_alpha` with `elapsed > duration` and `duration == 0` does not overflow or divide by zero.

---

### Task 1: `fade_duration` config

**Files:**
- Modify: `src/config.rs` (struct, `Default`, tests)
- Modify: `config.example.toml`, `README.md` (config table/section)

**Interfaces:**
- Produces: `Config::fade_duration: Duration` (default `Duration::ZERO`).

- [ ] **Step 1: Write failing tests** in `src/config.rs` tests:
  `fade_duration_defaults_to_zero` (`Config::default().fade_duration == Duration::ZERO`),
  `parses_fade_duration` (`fade_duration = "300ms"` → `Duration::from_millis(300)`),
  `rejects_invalid_fade_duration` (`fade_duration = "abc"` → `Config::load` is `Err`).
- [ ] **Step 2:** Run `cargo test config::` — expect compile failure (no field).
- [ ] **Step 3:** Add `#[serde(with = "humantime_serde")] pub fade_duration: Duration` after `notify_before`; default `Duration::ZERO`.
- [ ] **Step 4:** Run `cargo test config::` — expect PASS.
- [ ] **Step 5:** Document `fade_duration` in `config.example.toml` (after `notify_before`, same comment style: purpose, `"0ms"` disables, Wayland and compositor-less X11 skip) and in README's configuration docs.
- [ ] **Step 6:** `git add src/config.rs config.example.toml README.md && git commit -m "feat(config): add fade_duration setting"`

### Task 2: `fade_alpha` helper and Backend trait methods

**Files:**
- Create: `src/backend/fade.rs` (declare `pub mod fade;` in `src/backend/mod.rs`)
- Modify: `src/backend/mod.rs` (trait)

**Interfaces:**
- Produces: `pub const FADE_STEP: Duration = Duration::from_millis(16)`;
  `pub fn fade_alpha(elapsed: Duration, duration: Duration) -> u8` (0 at start, 255 at or after `duration`, linear; returns 255 when `duration` is zero).
- Produces: `Backend::fade_out(&self, duration: Duration) -> Result<()>` and `Backend::fade_in(&self, duration: Duration) -> Result<()>`, default bodies `log::warn!("screen fade is not supported by this backend; skipping")` then `Ok(())`.

- [ ] **Step 1: Write failing tests** in `src/backend/fade.rs`: `alpha_is_zero_at_start`, `alpha_midpoint_is_about_half` (elapsed 150ms of 300ms → 127 or 128), `alpha_saturates_after_duration` (elapsed 500ms of 300ms → 255), `alpha_with_zero_duration_is_opaque`.
- [ ] **Step 2:** Run `cargo test fade::` — expect FAIL (undefined).
- [ ] **Step 3:** Implement `fade_alpha` and `FADE_STEP`; add the two trait methods with defaults.
- [ ] **Step 4:** Run `cargo test fade::` and `cargo check` — expect PASS, no warnings (use `#[allow(dead_code)]` only if a platform leaves an item unused).
- [ ] **Step 5:** `git add src/backend && git commit -m "feat(backend): add fade trait methods and alpha helper"`

### Task 3: Scheduler integration

**Files:**
- Modify: `src/scheduler.rs` (`run_cycle`, tests' `MockBackend`)

**Interfaces:**
- Consumes: `Backend::fade_out/fade_in`, `Config::fade_duration`.
- Produces: `run_cycle` unchanged signature. Internally, move the existing focus/send/restore block (lines 66-88) into `fn send_with_focus(b: &dyn Backend, cfg: &Config, horizon_win: WindowId, prev_window: Option<WindowId>, supports_focus: bool) -> Result<()>` so `fade_in` can run after it regardless of its result.

- [ ] **Step 1: Extend `MockBackend`:** add `MockCall::FadeOut(Duration)` and `MockCall::FadeIn(Duration)`, fields `fail_fade_out: bool`, `fail_fade_in: bool`, and trait overrides that record calls and return `Err` when the flag is set.
- [ ] **Step 2: Write failing tests:**
  `fade_wraps_focus_and_beacon_in_order` (calls after `ActiveWindow`: `FadeOut, Focus(100), SendBeacon, Focus(200), FadeIn`),
  `no_fade_when_duration_is_zero`,
  `no_fade_when_cycle_skipped` (user active in Horizon),
  `fade_in_runs_when_send_beacon_fails` (error returned, `FadeIn` recorded),
  `fade_in_runs_when_focus_fails` (`fail_focus_on = Some(100)`),
  `beacon_sent_when_fade_out_fails` (outcome `Sent`, `SendBeacon` recorded, `FadeIn` not called),
  `outcome_sent_when_fade_in_fails`.
- [ ] **Step 3:** Run `cargo test scheduler::` — expect the new tests FAIL.
- [ ] **Step 4:** In `run_cycle`, after the skip check: if `!cfg.fade_duration.is_zero()` call `b.fade_out(cfg.fade_duration)`; on `Err` log a warning and remember `faded = false`. Run `send_with_focus`. If `faded`, call `b.fade_in(cfg.fade_duration)` and log a warning on `Err`. Then `result?; Ok(CycleOutcome::Sent)`.
- [ ] **Step 5:** Run `cargo test` — expect all PASS (existing tests unchanged).
- [ ] **Step 6:** `git add src/scheduler.rs && git commit -m "feat(scheduler): fade the screen around the beacon sequence"`

### Task 4: X11 overlay

**Files:**
- Modify: `src/backend/x11.rs`

**Interfaces:**
- Consumes: `fade_alpha`, `FADE_STEP`.
- Produces: `Backend::fade_out/fade_in` overrides on the X11 backend; overlay window stored in a `Mutex<Option<Window>>` field; `Drop` destroys it.

- [ ] **Step 1: Write failing test** `opacity_value_maps_alpha_to_cardinal` for a pure `fn opacity_cardinal(alpha: u8) -> u32` (0 → 0, 255 → `0xFFFF_FFFF`, monotonic).
- [ ] **Step 2:** Run `cargo test x11::` — expect FAIL.
- [ ] **Step 3:** Implement `opacity_cardinal`, then `fade_out`: return `Ok(())` for zero duration; if `_NET_WM_CM_S0` has no owner, `log::warn!` once (an `AtomicBool` guard) and return `Ok(())`; create an override-redirect black window covering the root with an empty input shape (`x11rb::protocol::shape`, enabling the `shape` feature in `Cargo.toml`), set `_NET_WM_WINDOW_OPACITY` to 0 before mapping, map it, then step opacity up to full every `FADE_STEP`. On any error destroy the window and return the error. `fade_in` steps opacity down, then destroys the window; it is a no-op if no overlay exists. Implement `Drop` to destroy any remaining overlay.
- [ ] **Step 4:** Run `cargo test` and `cargo check` — expect PASS.
- [ ] **Step 5: Smoke test** on an X11 session with a compositor: `cargo run -- -v once` with `fade_duration = "300ms"` in a local config; confirm fade-out, beacon, fade-in, and no leftover window. Repeat with the compositor stopped; confirm the warning and no fade.
- [ ] **Step 6:** `git add src/backend/x11.rs Cargo.toml Cargo.lock && git commit -m "feat(x11): implement screen fade overlay"`

### Task 5: Windows overlay and smoke-test checklist

**Files:**
- Modify: `src/backend/windows.rs`, `Cargo.toml` (windows features as needed, e.g. `Win32_Graphics_Gdi` already present), `README.md` (smoke-test checklist)

**Interfaces:**
- Consumes: `fade_alpha`, `FADE_STEP`.
- Produces: Windows `Backend::fade_out/fade_in` overrides; overlay `HWND` in a `Mutex<Option<isize>>` field; `Drop` destroys it.

- [ ] **Step 1:** Implement `fade_out`: `Ok(())` for zero duration; create one black window over the virtual screen (`GetSystemMetrics` `SM_XVIRTUALSCREEN`/`SM_YVIRTUALSCREEN`/`SM_CXVIRTUALSCREEN`/`SM_CYVIRTUALSCREEN`) with `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`, black brush, alpha 0 via `SetLayeredWindowAttributes(.., LWA_ALPHA)`, shown with `SW_SHOWNOACTIVATE`; step alpha up with `fade_alpha` every `FADE_STEP`. On error destroy the window and return the error. `fade_in` steps alpha down then `DestroyWindow`; no-op without an overlay. `Drop` destroys any remaining overlay.
- [ ] **Step 2:** Run `cargo check` on Linux (Windows code is `cfg`-gated) and, if a Windows toolchain is available, `cargo check --target x86_64-pc-windows-msvc`; expect no warnings.
- [ ] **Step 3:** Add to the README smoke-test checklist: Windows fade (overlay covers all monitors, clicks pass through, no leftover window), X11 with compositor, X11 without compositor (warning, no fade), Wayland (warning, no fade).
- [ ] **Step 4: Smoke test** on Windows with `fade_duration = "300ms"` and `cargo run -- -v once`.
- [ ] **Step 5:** `git add src/backend/windows.rs README.md Cargo.toml && git commit -m "feat(windows): implement screen fade overlay"`
