# RDPBeacon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust CLI daemon that periodically sends a tiny input beacon to a logged-in Omnissa Horizon Client session to prevent idle timeout.

**Architecture:** A single crate. A `Backend` trait abstracts per-OS idle detection, window lookup, focus and input injection. A `scheduler` drives cycles against the trait, so it is unit-tested with a mock. Backends for X11, Wayland (uinput) and Windows are `cfg`/runtime selected.

**Tech Stack:** Rust (stable, edition 2021), `clap`, `serde` + `toml`, `anyhow`, `log` + `env_logger`, `rand`, `ctrlc`; `x11rb` (X11), `evdev` (Wayland/uinput), `windows` crate (Windows).

**Spec:** `docs/superpowers/specs/2026-10-06-rdpbeacon-design.md`

## Global Constraints

- Platforms: Linux X11, Linux Wayland, Windows. No macOS, GUI/tray, or network-protocol keep-alive.
- Default key `F15`; default `window_match` = `"Omnissa Horizon Client"`; default `max_misses` = 5.
- Mouse mode moves 1 px and back.
- Focus restore delay about 100 ms after the beacon (X11 and Windows). Wayland never switches focus.
- Beacon fires only when idle time >= `idle_threshold`.
- uinput permission errors print the udev/`input` group fix and never auto-escalate.
- Backend detection via `XDG_SESSION_TYPE` and `WAYLAND_DISPLAY`; the `--backend` flag overrides it.
- Subcommands: `run`, `once`, `check`.

## Review Focus

- `interval` <= `jitter`: the sleep must never go zero or negative (clamp to a minimum of 1 s).
- Horizon window title or class changes between cycles: re-lookup every cycle, never cache the handle.
- Restore target window has closed: restoring must not error; skip it and log.
- Ctrl-C during sleep: exit promptly (not after a full interval).
- Missing or malformed config file: a clear error naming the field, and defaults apply when the file is absent.

---

### Task 1: Project scaffold, config and Backend trait

**Files:**
- Create: `Cargo.toml`, `src/main.rs`, `src/config.rs`, `src/backend/mod.rs`, `.gitignore`
- Test: `src/config.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub struct Config { interval: Duration, jitter: Duration, idle_threshold: Duration, key: String, mode: Mode, window_match: String, max_misses: u32, backend: Option<BackendKind> }`
  - `pub enum Mode { Key, Mouse }`
  - `pub enum BackendKind { X11, Wayland, Windows }`
  - `Config::load(path: Option<&Path>) -> anyhow::Result<Config>`
  - `pub type WindowId = u64;`
  - `pub trait Backend { fn idle_time(&self) -> Result<Duration>; fn find_horizon_window(&self, matcher: &str) -> Result<Option<WindowId>>; fn active_window(&self) -> Result<Option<WindowId>>; fn focus(&self, w: WindowId) -> Result<()>; fn send_beacon(&self, mode: Mode, key: &str) -> Result<()>; fn supports_focus(&self) -> bool; }`

- [ ] **Step 1: Write failing tests** `defaults_when_no_file`, `parses_toml_overrides` (`interval = "5m"` becomes 300 s), and `malformed_toml_names_the_field`.
- [ ] **Step 2: Run** `cargo test config` and expect FAIL (does not compile).
- [ ] **Step 3: Implement** `Config::load` (serde + humantime-style duration parsing with the `humantime-serde` crate) and the `Backend` trait in `src/backend/mod.rs`. Defaults: `interval` 4m, `jitter` 20s, `idle_threshold` 3m, matching the Global Constraints.
- [ ] **Step 4: Run** `cargo test config` and expect PASS.
- [ ] **Step 5: Commit** `git commit -m "feat: scaffold, config, Backend trait"`

### Task 2: Scheduler with mock backend

**Files:**
- Create: `src/scheduler.rs`
- Test: `src/scheduler.rs` (inline, with a `MockBackend`)

**Interfaces:**
- Consumes: `Backend`, `Config` (Task 1)
- Produces:
  - `pub enum CycleOutcome { SkippedActive, SkippedNoWindow, Sent }`
  - `pub fn run_cycle(b: &dyn Backend, cfg: &Config) -> Result<CycleOutcome>`
  - `pub fn next_sleep(cfg: &Config, rng: &mut impl Rng) -> Duration` (clamped to >= 1 s)
  - `pub fn run_loop(b: &dyn Backend, cfg: &Config, stop: &AtomicBool) -> Result<()>` (exits after `max_misses` consecutive `SkippedNoWindow`)

- [ ] **Step 1: Write failing tests**
  - `skips_when_user_active`: idle below the threshold gives `SkippedActive` and no beacon is sent.
  - `sends_when_idle_and_window_found`: gives `Sent`.
  - `no_window_gives_skipped_no_window`.
  - `focus_and_restore_order`: with `supports_focus() == true`, the mock call log is `active_window, focus(h), send_beacon, focus(prev)`.
  - `no_focus_calls_when_unsupported`.
  - `restore_skipped_if_prev_missing`: `active_window` returns `None`, so there is no restore call and no error.
  - `next_sleep_within_bounds_and_min_1s`: includes a case where `jitter >= interval`.
  - `run_loop_exits_after_max_misses`.
  - `run_loop_stops_promptly_on_flag`: the sleep is chunked into ≤200 ms slices that check `stop`.
- [ ] **Step 2: Run** `cargo test scheduler` and expect FAIL.
- [ ] **Step 3: Implement** the functions above. `run_cycle` order: idle check, window lookup, save active window, focus, beacon, 100 ms wait, restore.
- [ ] **Step 4: Run** `cargo test scheduler` and expect PASS.
- [ ] **Step 5: Commit** `git commit -m "feat: scheduler with mock-tested cycle logic"`

### Task 3: CLI, backend selection and logging

**Files:**
- Modify: `src/main.rs`, `src/backend/mod.rs`
- Test: `src/backend/mod.rs` (inline)

**Interfaces:**
- Consumes: Tasks 1 and 2.
- Produces: `pub fn detect_backend_kind(session_type: Option<&str>, wayland_display: Option<&str>, is_windows: bool) -> Result<BackendKind>`, and `pub fn make_backend(kind: BackendKind) -> Result<Box<dyn Backend>>`. The CLI is `rdpbeacon [--config PATH] [--backend x11|wayland|windows] <run|once|check>`. Ctrl-C sets the stop flag via `ctrlc`.

- [ ] **Step 1: Write failing tests**: `detects_x11`, `detects_wayland_via_session_type`, `detects_wayland_via_display_var`, `ambiguous_returns_error_mentioning_backend_flag`, `windows_wins_on_windows`.
- [ ] **Step 2: Run** `cargo test backend` and expect FAIL.
- [ ] **Step 3: Implement** detection, the clap CLI, and the `run`/`once`/`check` handlers. `check` prints the backend, the idle time and the Horizon window (found or not). `once` runs a single `run_cycle` ignoring the idle threshold.
- [ ] **Step 4: Run** `cargo test` and expect PASS; `cargo run -- --help` lists the three subcommands.
- [ ] **Step 5: Commit** `git commit -m "feat: CLI and backend detection"`

### Task 4: X11 backend

**Files:**
- Create: `src/backend/x11.rs`; Modify: `Cargo.toml` (`x11rb` with `xtest`, `screensaver` features), `src/backend/mod.rs`
- Test: `src/backend/x11.rs` (pure helper tests)

**Interfaces:**
- Consumes: `Backend` trait.
- Produces: `pub struct X11Backend`, `X11Backend::new() -> Result<Self>`, `fn title_matches(title: &str, class: &str, matcher: &str) -> bool` (case-insensitive substring on the title or class).

- [ ] **Step 1: Write failing tests**: `matches_title_case_insensitive`, `matches_wm_class`, `no_match_returns_false`.
- [ ] **Step 2: Run** `cargo test x11` and expect FAIL.
- [ ] **Step 3: Implement** the `Backend` trait using XScreenSaver `QueryInfo` for idle time, `_NET_CLIENT_LIST` plus `_NET_WM_NAME`/`WM_CLASS` for lookup, `_NET_ACTIVE_WINDOW` for get and focus (client message), and XTest `fake_input` for the key and mouse beacon. `supports_focus()` returns true.
- [ ] **Step 4: Run** `cargo test x11` and expect PASS. Manual (under X11): `cargo run -- check` finds the Horizon window; `cargo run -- once` focuses and restores.
- [ ] **Step 5: Commit** `git commit -m "feat: X11 backend"`

### Task 5: Wayland (uinput) backend

**Files:**
- Create: `src/backend/wayland.rs`; Modify: `Cargo.toml` (`evdev`), `src/backend/mod.rs`
- Test: `src/backend/wayland.rs` (inline)

**Interfaces:**
- Consumes: `Backend` trait.
- Produces: `pub struct WaylandBackend`, `WaylandBackend::new() -> Result<Self>`, `fn key_code(name: &str) -> Result<evdev::Key>`, `fn permission_hint() -> &'static str` (the udev rule and `input` group instructions).

- [ ] **Step 1: Write failing tests**: `key_code_f15`, `key_code_unknown_errors`, `permission_hint_mentions_udev_and_input_group`.
- [ ] **Step 2: Run** `cargo test wayland` and expect FAIL.
- [ ] **Step 3: Implement** a virtual device created via `/dev/uinput`. On `EACCES`, return an error that includes `permission_hint()`. `supports_focus()` returns false. `find_horizon_window` returns `Some(0)` as a sentinel, and `active_window` returns `None`. `idle_time` tries the `ext-idle-notify` protocol (using `wayland-client` and `wayland-protocols`). If it is unsupported, it returns `Duration::MAX`, so the beacon always fires, and a one-time warning is logged.
- [ ] **Step 4: Run** `cargo test wayland` and expect PASS. Manual (under Wayland): `cargo run -- once` types F15 into the active window (verify with `wev`).
- [ ] **Step 5: Commit** `git commit -m "feat: Wayland uinput backend"`

### Task 6: Windows backend

**Files:**
- Create: `src/backend/windows.rs`; Modify: `Cargo.toml` (`windows` crate, target-gated), `src/backend/mod.rs`
- Test: `src/backend/windows.rs` (`#[cfg(windows)]` pure helper tests)

**Interfaces:**
- Consumes: `Backend` trait.
- Produces: `pub struct WindowsBackend`, `WindowsBackend::new() -> Result<Self>`, and `fn title_matches(title: &str, matcher: &str) -> bool`.

- [ ] **Step 1: Write failing tests**: `matches_case_insensitive_substring`, `no_match_returns_false`.
- [ ] **Step 2: Run** `cargo test windows` on Windows and expect FAIL.
- [ ] **Step 3: Implement** idle time via `GetLastInputInfo`, lookup via `EnumWindows`+`GetWindowTextW` (visible windows only), focus via `SetForegroundWindow`, with an `AttachThreadInput` trick as the first fallback and the global-input fallback logged on failure (per spec), and the beacon via `SendInput`. `supports_focus()` returns true.
- [ ] **Step 4: Run** `cargo test` on Windows and expect PASS. Manual: `rdpbeacon.exe check` and `once` against a running Horizon Client.
- [ ] **Step 5: Commit** `git commit -m "feat: Windows backend"`

### Task 7: README and CI

**Files:**
- Create: `README.md`, `config.example.toml`, `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: the whole crate.

- [ ] **Step 1: Write** the README. It covers usage, the config fields, the Wayland uinput setup and limitation, the policy and hard-session-limit caveats, and the manual smoke-test checklist from the spec.
- [ ] **Step 2: Write** `ci.yml` with `cargo build` and `cargo test` on `ubuntu-latest` and `windows-latest`.
- [ ] **Step 3: Verify** `cargo build --release && cargo test` pass locally. The CI file is valid YAML.
- [ ] **Step 4: Commit** `git commit -m "docs: README, example config, CI"`

---

**Self-review:** Every spec section maps to a task:
- Config and trait: Task 1.
- Scheduler, data flow, error handling: Task 2.
- CLI and detection: Task 3.
- X11, Wayland and Windows backends: Tasks 4 to 6.
- Testing, CI and caveats: Tasks 2 and 7.

Every Review Focus line is pinned by a test, except two. The config error test is in Task 1 and the Ctrl-C test is in Task 2. The stale-window and closed-restore-target cases are in Task 2. Type and method names are consistent across tasks.
