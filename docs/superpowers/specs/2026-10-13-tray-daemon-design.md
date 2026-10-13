# Tray Daemon Mode Design

## Intent and agreed requirements

RDPBeacon should keep running without an open terminal and provide local tray
controls for enabling or disabling file logging and quitting. The user selected
Linux and Windows support, logging disabled by default, and a single-process
tray and beacon architecture. The tray belongs to the local desktop running
RDPBeacon, not the remote Windows desktop inside Horizon.

This document proposes the remaining behavior for review. It extends the original
CLI-only design; the existing beacon delivery behavior remains the baseline.

## User interface and command behavior

- Add `rdpbeacon run --daemon`. The flag applies only to `run`.
- The launch command returns after confirming that the background application
  has initialized its backend and registered its tray. Closing the launching
  terminal then leaves the application running.
- A short-lived launcher starts a detached copy of the same executable. After
  startup, one process owns the tray, logger, and beacon worker. There is no
  separate persistent service or controller process.
- Preserve `run`, `once`, and `check` behavior and terminal logging.
- Preserve global configuration and backend options in daemon mode. Resolve the
  configuration path before detaching so relative paths retain their meaning.
- `-v` selects debug verbosity when file logging is enabled; it does not enable
  logging automatically. Existing `RUST_LOG` filtering remains available and also
  does not override the disabled logging state.
- The tray menu contains a disabled status entry (`Running`, `Stopping`, or
  `Stopped: <reason>`), a checkable `Enable logging` item, and `Quit`.
- `Enable logging` starts unchecked. Checking it enables file logging;
  unchecking it disables file logging without affecting beacon scheduling.
- Logging state is session-local and resets to disabled at the next launch.
- Use an embedded application icon. No application window is opened.

## Architecture and ownership

Keep the existing single Rust crate. Add focused modules for detached startup,
tray control, and daemon logging, with platform-specific tray implementations.

The application coordinator owns the stop flag and receives control events from
the tray. A worker thread creates and owns the input backend and calls the
existing scheduler loop. Creating the backend inside the worker avoids requiring
the existing `Backend` trait objects to cross threads or acquire new `Send` bounds.

The Windows thread owning the tray runs a Win32 message loop. Linux uses a
StatusNotifierItem service on the session D-Bus; its callbacks send events to the
coordinator. Neither tray callback performs a beacon or waits for worker shutdown.

Use `ksni` with its blocking API for Linux and the existing `windows` crate with
additional required Win32 features for Windows. Linux does not acquire a GTK or
AppIndicator development-library requirement. Keep these dependencies and APIs
behind target-specific compilation guards.

The shared control events are logging toggle requests and quit requests. Worker
startup and completion are reported separately. The tray reflects the actual
logging state after a request succeeds, not merely the clicked checkbox state.

## Detached startup and startup failure

Use a hidden internal child invocation to distinguish the detached application
from the public launcher. Carry configuration, backend selection, verbosity,
working-directory context, and environment into the child. Prevent recursive
launching.

On Linux, the child establishes an independent session before desktop resources
or worker threads are created. Preserve the display and session D-Bus environment.
On Windows, create the child without a console and independently of the launching
console's control events. Do not change the whole binary to the Windows GUI
subsystem, since normal CLI commands still need console output.

Use an inherited startup channel for one success or failure response. The launcher
waits at most ten seconds. Backend initialization, tray registration, or startup
channel failure returns a nonzero launch result with an English explanation.
On timeout, terminate and reap the child; do not leave an unconfirmed background
application running. Partial startup must clean up the tray and worker.

Successful tray registration means that the desktop's tray service accepted the
item; the application cannot guarantee that desktop settings make the icon visible.
If Linux has no StatusNotifierWatcher, reject daemon startup with instructions to
enable a compatible tray host. Normal CLI mode remains usable.

## Logging

Daemon mode installs a thread-safe logger with a runtime-controlled file sink.
Normal CLI mode continues to use `env_logger`. The daemon logger can reuse its
filtering and formatting facilities while gating writes through shared state.

- Linux path: `$XDG_STATE_HOME/rdpbeacon/rdpbeacon.log`, falling back to
  `$HOME/.local/state/rdpbeacon/rdpbeacon.log` when the state variable is absent
  or not an absolute path.
- Windows path: `%LOCALAPPDATA%\RDPBeacon\logs\rdpbeacon.log`.
- If a required user directory cannot be resolved, enabling logging fails with
  a visible tray error; beacon processing continues.
- Create directories and open the file only when logging is enabled. Use
  user-private permissions on Linux.
- Include a timestamp, severity, and message. Do not include terminal colors.
- Enabling appends a marker and subsequent records; it does not replay records
  emitted while disabled. Disabling flushes and closes the sink before reporting
  success. Records racing with a toggle follow the sink's serialized ordering.
- Bound retained files: rotate at 5 MiB, retaining the current file and one backup
  named `rdpbeacon.log.1`. Recheck existing size when enabling.
- Open, write, or rotation failure disables the sink, clears the checkbox, and
  reports the logging error in tray status without stopping beacons. Error
  reporting must not recursively log through the failing sink.
- A successful later enable request clears the logging error. A worker failure
  takes precedence over a logging error in the status entry.

## Shutdown and worker errors

`Quit` sets the existing scheduler stop flag and marks the tray as `Stopping`.
The scheduler's interruptible sleep exits promptly. If a cycle is in progress,
allow its normal input completion and focus-restoration attempt to finish.
Do not introduce cancellation in the middle of focus switching.

The coordinator waits for worker completion while keeping tray event handling
responsive, then flushes logging, removes the tray, and exits. Repeated quit
requests are harmless. Existing foreground Ctrl-C behavior is preserved.

When the existing `max_misses` policy stops the scheduler, or the worker otherwise
terminates unexpectedly, stop beacon processing and retain the tray with a
`Stopped` error status. Users can enable logging for later application events or
choose `Quit`; there is no automatic retry or restart menu in this change. The
stop reason is kept in memory and is written if logging is subsequently enabled.

## Scope boundaries

This change adds no login autostart, OS service installation, pause/resume menu,
configuration editor, remote control, or single-instance enforcement. Concurrent
launches remain separate applications, as with concurrent foreground runs.

Daemon mode preserves beacon intervals, mouse mode defaults, focus switching,
Wayland restrictions, and remote screen-lock limitations. Tray availability does
not improve input delivery or prove that the remote session received a beacon.

## Verification and documentation

Automated tests cover CLI flag acceptance and rejection, launch argument/path
preservation, startup success/failure/timeout cleanup, logging disabled by
default, toggles and file-open failures, rotation, worker completion/error state,
and idempotent quit behavior. Use injected startup and tray adapters for lifecycle
tests; do not require a graphical desktop in unit tests.

Run existing scheduler regression tests to preserve focus restoration and
shutdown behavior. Run locked dependency checks, tests, and release builds on
Linux and Windows in CI. Linux CI must not need GTK development packages.

Manual acceptance checks on both operating systems:

1. Launch daemon mode and close its terminal; beacons continue and the tray stays.
2. Verify logging is initially unchecked and no log file is created on a fresh
   profile. Enable, disable, and re-enable logging and inspect the file.
3. Quit during a sleep and during a beacon; check process exit and the existing
   focus-restoration behavior.
4. Verify backend/tray startup failures reach the launcher and leave no worker.
5. Stop the worker through the missing-window limit and inspect the retained tray.
6. On Linux, exercise a compatible tray host under X11 and Wayland; document
   GNOME's extension requirement and test the missing-host failure path.
7. Verify existing CLI modes still print diagnostics and respond to Ctrl-C.

Update README command examples, tray-host prerequisites, log paths, rotation,
and smoke-test instructions. Record the actual desktops tested; cross-platform
compilation alone is not tray or input-delivery validation.

## Reference documentation

- [ksni API and blocking feature](https://docs.rs/ksni/latest/ksni/)
- [env_logger builder and filters](https://docs.rs/env_logger/latest/env_logger/struct.Builder.html)
- [GNOME AppIndicator and KStatusNotifierItem support](https://extensions.gnome.org/extension/615/appindicator-support/)
