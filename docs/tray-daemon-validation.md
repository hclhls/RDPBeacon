# Tray Daemon Validation

## Tested environments

- Linux: Ubuntu, GNOME with a StatusNotifierItem host, X11.
- Windows target: `x86_64-pc-windows-gnu`, cross-compiled with Rust 1.99 and
  MinGW; executed in an isolated Wine prefix. This is not native Windows
  desktop validation.

## Automated checks

- Linux: 74 unit tests passed, locked compilation check and release build passed.
- Windows target: 59 unit tests passed under Wine, locked compilation check
  and release build passed.
- Rust formatting and whitespace checks passed. An official temporary rustfmt
  installation was used because the host's default Cargo lacked cargo-fmt.

Test counts include the final review regressions. The Windows run explicitly
included the normally ignored interactive menu test under Wine.

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
A regression probe kept the Watcher alive while its host-registration property
became false; the daemon exited in that case too.

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

## Final review fixes

The independent reviewer identified four issues. Windows callback state now uses
raw allocation ownership until its native window is destroyed. Its native UI
loop runs on a separate thread so a modal menu cannot block the coordinator;
close/shutdown requests dismiss the menu. Linux additionally polls the uncached
host-registration property with a one-second D-Bus timeout. Log writers use a
shared lock file and reopen the current log under that lock before every write,
preventing stale handles from rotating another process's replacement file.
The lock handle is closed after each write, including error paths. This avoids
a lock-release error reproduced under Wine with Rust 1.99's explicit unlock.

Windows menu tests cover coordinator responsiveness, close while the menu is
open, and destroying the host while its menu is open. Shutdown uses persistent
state checked after modal dispatch rather than a WM_QUIT message that a nested
menu loop could consume. The callback ownership regression checks pointer
stability and exactly-once destruction; it is not a Miri run.

Additional tradeoffs are one Windows UI thread, one Linux monitoring thread and
a periodic D-Bus query, and a private lock file plus per-record file opens.
Single-instance enforcement, autostart, worker auto-restart, and a guarantee
that remote screen locking never disconnects remain outside the approved scope.
