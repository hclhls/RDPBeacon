# Tray Daemon Validation

## Tested environments

- Linux: Ubuntu, GNOME with a StatusNotifierItem host, X11.
- Windows target: `x86_64-pc-windows-gnu`, cross-compiled with Rust 1.99 and
  MinGW; executed in an isolated Wine prefix. This is not native Windows
  desktop validation.

## Automated checks

- Linux: 72 unit tests passed, locked compilation check and release build passed.
- Windows target: 56 unit tests passed under Wine, locked compilation check
  and release build passed.
- Rust formatting and whitespace checks passed. An official temporary rustfmt
  installation was used because the host's default Cargo lacked cargo-fmt.

Test counts describe the initial implementation before any final review fixes.
The final completion report records checks for the final commit.

## Desktop and lifecycle checks

Linux daemon startup registered the tray on the real desktop. Its process had
an independent session ID equal to its PID and no controlling terminal after
the launcher exited. The actual D-Bus menu exposed Running, an unchecked Enable
logging item, and Quit. Enabling created a file; disabling cleared the check;
re-enabling appended a second marker. Newly created directories had mode 0700
and the log had mode 0600. Quit removed both test daemons and their tray services.

A separate D-Bus session without a tray host rejected startup with a clear
message and exit status 1. A disposable StatusNotifierWatcher on an isolated
D-Bus session accepted startup; stopping that host caused the daemon to exit.

The Windows release executable launched successfully in Wine with `run --daemon`.
Posting the close request to its hidden tray window exercised the graceful Quit
path. Windows file logging and worker lifecycle tests ran in the Windows test
binary under Wine.

All desktop launch probes used a temporary regular configuration file with a
one-hour interval and a nonexistent session match. These probes sent no beacon
input to the user's Horizon session. Focus restoration and quitting during a
beacon were tested with the scheduler and coordinator test backends.

## Remaining platform checks

Native Windows notification-area presentation, Explorer restart recovery,
Wayland desktop integration, and live input delivery in daemon mode need manual
checks on those environments. Automated tests and Wine are not substitutes for
these checks. The README includes the manual checklist.

## Implementation decisions

- Managed worktree creation succeeded, but its path was outside writable roots.
  Implementation used the existing checkout on `feat/tray-daemon`, following
  the sandbox fallback. Working files are shared with the user's checkout.
- The coordinator lives in `src/daemon/runtime.rs` to separate runtime ownership
  from the state model, at the cost of one additional internal module.
- Existing Rust formatting was normalized to pass the required formatting check,
  increasing the formatting-only diff without changing beacon behavior.
- Temporary Rust/MinGW/Wine tools provided Windows checks. Differences between
  Wine and Windows remain a validation limitation.
- Review tools became available after planning. Native implementation remained
  selected and an independent final reviewer was used, adding review time.
