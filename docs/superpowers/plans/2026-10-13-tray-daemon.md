# Tray Daemon Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Native execution is recommended; no subagent tool is available in this session.

**Goal:** Add a detached Linux and Windows tray mode with optional file logging and graceful quit.

**Architecture:** One persistent process owns a coordinator, a beacon worker, and a platform tray adapter. A temporary launcher confirms startup through the child's piped standard output before returning. The worker constructs its own backend; tray and logger events are serialized by the coordinator.

**Tech Stack:** Rust 2021, existing CLI/scheduler/log dependencies, Linux `ksni` blocking API and `rustix` process API, Windows native notification-area/message-loop APIs through the existing `windows` crate.

**Spec:** `docs/superpowers/specs/2026-10-13-tray-daemon-design.md`

## Global Constraints

- Linux X11, Linux Wayland, and Windows; local desktop tray only.
- Public command: `rdpbeacon run --daemon`; normal `run`, `once`, and `check` retain their behavior.
- One persistent application process; no persistent launcher, service, or IPC controller.
- Logging disabled by default; `-v` and `RUST_LOG` select filters without enabling logging.
- Menu: disabled status, checkable `Enable logging`, and `Quit`.
- Startup confirmation timeout: ten seconds; failure or timeout cleans up the child.
- Log size threshold: 5 MiB; current file plus `rdpbeacon.log.1` backup.
- Quit completes the current cycle and focus-restoration attempt before exit.
- Worker failure stops beacons but retains the tray with its stop reason.
- No GTK development-library requirement; no autostart, pause/resume, config editor, or single-instance enforcement.
- English project text and Conventional Commits; Japanese user communication.
- Do not stage the user's untracked `AGENTS.md` or local configurations/logs.

## Review Focus

- Paths with spaces, Unicode, or relative components must survive detached startup (Task 2).
- Startup timeout or malformed output must leave no orphan child or running worker (Tasks 2 and 6).
- Existing oversized logs and file rotation/write failures must not crash the beacon worker (Task 3).
- Worker completion racing with repeated Quit must not deadlock the tray thread (Tasks 1 and 6).
- Linux tray-host loss and Windows Explorer restart must not leave a silently uncontrollable beacon worker (Tasks 4 and 5).

## File map

- `src/main.rs`: command parsing and dispatch; retain foreground flows.
- `src/daemon/mod.rs`: coordinator, worker lifecycle, shared control events/state.
- `src/daemon/launch.rs`: detached spawning, argument normalization, startup protocol.
- `src/daemon/logging.rs`: runtime-gated file logging, path resolution, rotation.
- `src/tray/mod.rs`: adapter interface and target selection.
- `src/tray/linux.rs`: StatusNotifierItem service and menu updates.
- `src/tray/windows.rs`: hidden window, tray icon, context menu, message loop.
- `src/tray/icon.rs`: embedded icon pixels shared by adapters.
- `Cargo.toml` / `Cargo.lock`: target dependencies and Win32 features.
- `README.md` / `.github/workflows/ci.yml`: usage, manual verification, locked CI.
- Unit tests live alongside the modules. `src/scheduler.rs` is modified only if meaningful lifecycle regression tests require it.

## Shared interfaces

All tasks use these names and types; define them in Task 1 and keep ownership explicit.

```rust
// src/daemon/mod.rs
pub enum ControlEvent {
    SetLogging(bool), Quit, TrayUnavailable(String), LoggingFailed(String),
    WorkerReady, WorkerFinished(Result<(), String>),
}
pub enum WorkerState { Starting, Running, Stopping, Stopped(String) }
pub struct AppState {
    pub worker: WorkerState,
    pub logging_enabled: bool,
    pub logging_error: Option<String>,
}
pub struct TrayView {
    pub status: String,
    pub logging_enabled: bool,
    pub quitting: bool,
}
impl AppState { pub fn view(&self) -> TrayView; }
pub fn run(cfg: Config, kind: BackendKind, verbose: bool) -> anyhow::Result<()>;

// src/tray/mod.rs
pub trait TrayHost {
    fn update(&mut self, view: &TrayView) -> anyhow::Result<()>;
    fn pump(&mut self, timeout: Duration) -> anyhow::Result<()>;
}
pub fn create(events: Sender<ControlEvent>) -> anyhow::Result<Box<dyn TrayHost>>;

// src/daemon/logging.rs
pub struct LoggingControl { /* shared synchronized sink */ }
impl LoggingControl {
    pub fn set_enabled(&self, enabled: bool) -> anyhow::Result<()>;
    pub fn enabled(&self) -> bool;
}
pub fn install(verbose: bool, events: Sender<ControlEvent>)
    -> anyhow::Result<LoggingControl>;

// src/daemon/launch.rs
pub struct LaunchOptions {
    pub config: Option<PathBuf>, pub backend: Option<BackendKind>, pub verbose: bool,
}
pub fn launch(options: LaunchOptions) -> anyhow::Result<()>;
pub fn prepare_child() -> anyhow::Result<()>;
pub fn report_startup(result: Result<(), String>) -> anyhow::Result<()>;
```

## Task 1: Coordinator state and graceful lifecycle

**Files:** Create `src/daemon/mod.rs`; add the module declaration in `src/main.rs`.

**Interfaces:** Define the shared daemon types above. Add pure state methods
`request_quit(&mut self)`, `worker_finished(&mut self, Result<(), String>)`, and
`logging_result(&mut self, bool, Result<(), String>)` for deterministic tests.
The runtime `run` function is integrated in Task 6, not exposed as a working CLI yet.

- [ ] Write failing tests: `initial_state_has_logging_disabled` asserts false;
  `quit_is_idempotent` asserts two requests produce Stopping;
  `worker_error_takes_precedence_over_logging_error` asserts the view contains the
  worker reason; `quit_then_worker_completion_is_terminal` asserts completion
  permits shutdown without restarting or joining twice.
- [ ] Run `cargo test --locked daemon::tests`; expect the new assertions or missing interfaces to fail.
- [ ] Implement the types and state transitions. Running/Stopping/Stopped status
  strings are English; logging failures never imply worker termination.
- [ ] Run `cargo test --locked daemon::tests`; expect PASS.
- [ ] Commit the module and declaration: `feat(daemon): model tray controls and worker lifecycle`.

## Task 2: Detached startup and command parsing

**Files:** Create `src/daemon/launch.rs`; modify `src/main.rs`, `Cargo.toml`, and `Cargo.lock`.

**Interfaces:** Implement `LaunchOptions`, `launch`, `prepare_child`, and
`report_startup`. Change `Commands::Run` to carry public `daemon: bool` and hidden
`internal_daemon: bool` arguments. Update existing CLI assertions accordingly.

- [ ] Write failing CLI tests: `run_accepts_daemon_flag`,
  `once_rejects_daemon_flag`, and `check_rejects_daemon_flag`. Test the hidden
  argument does not appear in help and that internal invocation never relaunches.
- [ ] Write startup tests using an injectable executable/child adapter: preserve
  a relative Unicode configuration path with spaces as an absolute argument;
  preserve backend and verbose options; success returns only after READY;
  explicit error, premature EOF, malformed protocol, and timeout all terminate
  and reap an unfinished child. Use a short injected timeout in tests.
- [ ] Run `cargo test --locked daemon::launch` and CLI tests; expect FAIL.
- [ ] Implement the public launcher and hidden child dispatch. Launch using
  `std::env::current_exe` and structured `Command::arg` values, never a shell.
  Child stdin/stderr are null and stdout is the startup pipe. Use a bounded
  reader and a ten-second deadline. Protocol: `READY` or `ERROR <single-line reason>`.
  Close the startup output after reporting; no later stdout writes.
- [ ] On Linux, add target-specific `rustix` process support and call `setsid`
  inside the child before threads start. On Windows, use `CommandExt` creation
  flags `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP`, preserving redirected handles.
- [ ] Preserve inherited environment and working directory. Resolve an explicit
  config path before spawning. Foreground dispatch continues to initialize
  `env_logger`; daemon child dispatch reserves logger installation for Task 3.
- [ ] Run targeted tests and `cargo check --locked`; expect PASS. Commit:
  `feat(daemon): add detached launch and startup acknowledgement`.

## Task 3: Runtime file logging with bounded retention

**Files:** Create `src/daemon/logging.rs`; modify daemon module declarations.

**Interfaces:** Implement `LoggingControl` and `install` from the shared interfaces.
Expose a testable `resolve_log_path(state_home: Option<&Path>, home: Option<&Path>,
local_app_data: Option<&Path>, windows: bool) -> anyhow::Result<PathBuf>`.
Keep sink construction injectable for deterministic write/rotation failures.

- [ ] Write failing tests proving: disabled initialization creates no directory;
  enabling appends a marker and filtered records; disabling closes the file and
  suppresses future records; re-enabling appends; verbose/filter selection alone
  never enables the sink. Test Linux absolute state path, relative-state fallback,
  Windows local app-data path, and missing base directory error.
- [ ] Add tests: existing log at 5 MiB rotates before new records; only one backup
  remains; open/write/rename failure disables the sink and emits LoggingFailed
  without recursive logging. Test concurrent toggles/writes using controlled
  barriers and verify no records are written after disable returns.
- [ ] Run `cargo test --locked daemon::logging`; expect FAIL.
- [ ] Implement a `log::Log` wrapper with `env_logger` filtering/formatting and a
  mutex-protected optional file sink. Resolve directories lazily. Use Linux
  directory mode 0700 and file mode 0600 for new artifacts. Append without color,
  rotate before a record that exceeds 5 MiB, flush/close on disable, and recover
  from later enable attempts. Reject invalid base paths with useful context.
- [ ] Run targeted tests; expect PASS. Commit:
  `feat(logging): add optional rotating daemon file logs`.

## Task 4: Linux tray adapter

**Files:** Create `src/tray/mod.rs`, `src/tray/linux.rs`, `src/tray/icon.rs`;
modify `src/main.rs`, `Cargo.toml`, and `Cargo.lock`.

**Interfaces:** Implement `TrayHost` and Linux `create`. Use the shared
`Sender<ControlEvent>` and `TrayView`. Add `icon_rgba() -> (Vec<u8>, u32, u32)`
for a small embedded application icon; convert to KSNI ARGB at the adapter edge.

- [ ] Write failing tests for menu model: status is disabled; logging starts
  unchecked; toggles send SetLogging; Quit sends Quit; a failed logging request
  is reflected by a subsequent unchecked view. Test host loss sends
  TrayUnavailable instead of leaving beacon processing uncontrolled.
- [ ] Run `cargo test --locked tray`; expect FAIL.
- [ ] Add target-specific `ksni` 0.3 with `blocking` support and implement the
  StatusNotifierItem service. Reject initial registration without a watcher;
  report the GNOME extension/tray-host requirement. Keep callbacks nonblocking,
  service handle owned until shutdown, and unregister on drop. A host loss after
  startup requests graceful application shutdown with a retained failure reason
  where display remains possible; document that the tray may disappear.
- [ ] Implement icon conversion and menu updates without GTK dependencies.
  `pump` gives the coordinator a bounded wait while KSNI handles D-Bus internally.
- [ ] Run targeted tests and `cargo check --locked`; expect PASS. Commit:
  `feat(tray): add Linux StatusNotifierItem controls`.

## Task 5: Windows tray adapter

**Files:** Create `src/tray/windows.rs`; modify `src/tray/mod.rs`, `Cargo.toml`, and `Cargo.lock`.

**Interfaces:** Implement Windows `create` and `TrayHost` on the owning coordinator
thread. Consume the same icon, view, and control events as Task 4.

- [ ] Write failing tests for command routing: toggle and quit IDs send the
  expected events, unrelated IDs are ignored, view updates change checkbox and
  disabled status text. Test Explorer restart triggers icon re-registration;
  re-registration failure sends TrayUnavailable. Use a fake Win32 adapter.
- [ ] Run those tests on Windows; expect FAIL.
- [ ] Add Windows features for Shell, GDI, and LibraryLoader as required by
  notification icons, menus, bitmap/icon ownership, and window registration.
  Implement an invisible window and Shell_NotifyIcon lifecycle. Use native menu
  APIs for the status, check item, separator, and Quit. Ensure window callback
  state and native resources outlive all callbacks and are released on drop.
- [ ] Implement a bounded message pump and TaskbarCreated handling. Callbacks
  enqueue events; they never join the worker or write log files.
- [ ] Run Windows targeted tests, check, and release build; expect PASS. Commit:
  `feat(tray): add Windows notification area controls`.

## Task 6: Integrate worker, logging, tray, and startup reporting

**Files:** Modify `src/daemon/mod.rs`, `src/main.rs`, `src/daemon/launch.rs`;
add lifecycle tests alongside these modules and scheduler regression tests if needed.

**Interfaces:** Complete `daemon::run(Config, BackendKind, bool)` using the Task
1 state, Task 3 logger, and Task 4/5 tray adapter. Use injected backend factory,
tray, and startup reporter in tests; production uses existing `make_backend` and
`scheduler::run_loop` unchanged.

- [ ] Write failing integration tests: report READY only after backend and tray
  readiness; backend/tray failure reports ERROR and cleans up; quit during sleep
  stops promptly; quit during a cycle waits for restoration; repeated Quit and
  worker completion race exits once; missing-window failure retains a Stopped
  tray; later logging enable records its saved reason. Cover worker panic with
  a Stopped reason and startup reporter failure with complete cleanup.
- [ ] Run `cargo test --locked daemon`; expect FAIL.
- [ ] Implement startup ordering, worker-owned backend construction, worker
  completion reporting, coordinator event dispatch, actual checkbox updates,
  and asynchronous logging error delivery. Use the existing atomic stop flag;
  retain worker failures for tray display rather than silently removing the icon.
- [ ] On Quit or tray-host loss, set stop once, keep pumping until completion,
  join once, flush logging, and remove the tray. Explicitly treat intentional
  shutdown separately from an unexpected worker completion. Never cancel a cycle
  in the middle of focus restoration.
- [ ] Run daemon tests and the full existing suite; expect PASS. Commit:
  `feat(daemon): integrate tray lifecycle with beacon worker`.

## Task 7: Documentation, CI, and final verification

**Files:** Modify `README.md` and `.github/workflows/ci.yml`.

**Interfaces:** Document the public behavior shipped by Tasks 1-6; no new controls.

- [ ] Add daemon command examples, menu descriptions, local tray-host prerequisites,
  log locations/rotation, disabled default, and startup error behavior. Explain
  that tray-host loss causes graceful shutdown and that worker failures retain
  the tray where the host is available. Preserve the existing remote-lock caveat.
- [ ] Extend manual checks for terminal closure, log toggles, Quit during sleep
  and focus switching, missing-window stop status, host loss, and Explorer restart.
- [ ] Use `--locked` for CI check/test/release commands on both existing runners;
  do not install GTK. Verify Windows jobs compile the native tray implementation.
- [ ] Run `cargo fmt --check`, `cargo check --locked`, `cargo test --locked`,
  `cargo build --release --locked`, and `git diff --check`; expect all PASS and
  no compiler warnings. Fix only changes needed for the feature and verification.
- [ ] Perform available local desktop smoke tests with an explicitly temporary
  config. Record exact desktop, tray-host, and outcomes; do not claim Windows or
  Wayland smoke coverage without executing those checks. Keep logs untracked.
- [ ] Review the whole diff against the spec and Review Focus. Commit:
  `docs: document tray daemon usage and validation`.

## Handoff

Review this plan before implementation. Native execution in this session is
recommended because the coordinator, launcher, and tray tasks share lifecycle
interfaces, and no subagent tool is available. Do not promise an independent
reviewer where the environment provides none. Use a focused feature worktree at
execution time, retaining the approved spec and plan commits as its baseline.
Publishing, tagging, or pushing this feature is a separate action from the
previous v0.2 release request.
