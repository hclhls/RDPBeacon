# RDPBeacon

[![CI](https://github.com/hclhls/RDPBeacon/actions/workflows/ci.yml/badge.svg)](https://github.com/hclhls/RDPBeacon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20Windows-lightgrey.svg)](#)

A lightweight, cross-platform CLI daemon written in Rust that keeps logged-in **Omnissa Horizon Client** sessions active on your local machine by periodically sending a small, harmless simulated input ("beacon"), preventing idle timeout disconnects.

---

## Features

- **Harmless Beacons**: Uses a tiny 1-pixel mouse nudge and back by default (`mode = "mouse"`). Key mode remains available with `mode = "key"` and `Shift_L` as the default key. Keep the pointer inside the Horizon desktop surface when using mouse mode.
- **Idle Detection**: If you are actively working inside Horizon, beacon cycles are skipped automatically since your typing keeps the session alive. Background Horizon sessions are kept alive even while you work in other local windows.
- **Smart Focus Management (X11 & Windows)**: Automatically saves your currently focused window, focuses the Horizon Client window, allows window input hooks to settle (~250 ms), injects the beacon, dwells (~150 ms) to ensure transmission to the remote VM, and cleanly restores your original window.
- **Wayland Support**: Uses Linux `/dev/uinput` to inject synthetic inputs without requiring root or display-server privileges.
- **Cross-Platform**: First-class support for Linux (X11), Linux (Wayland), and Windows.
- **Zero Caching**: Window lookup is re-evaluated every cycle, gracefully handling client restarts or window title changes.
- **Graceful Shutdown**: Responsive to `SIGINT` / `Ctrl-C` even during sleep intervals.
- **Tray Daemon**: `run --daemon` keeps running after the launching terminal closes. Use the local tray to enable file logging or quit on Linux and Windows.

---

## Architecture

RDPBeacon is built as a single Rust crate with a decoupled, trait-based backend architecture:

- **`config`**: Strongly-typed configuration powered by `serde` and `toml`, supporting human-friendly duration strings (e.g., `"4m"`, `"20s"`).
- **`scheduler`**: Core loop that manages randomized sleep intervals (`interval ± jitter`), checks system idle time against `idle_threshold`, invokes the backend, and handles retry/exit logic on missing windows (`max_misses`).
- **`backend` (`Backend` trait)**: Standard interface for idle detection, window lookup, focus switching, and beacon injection.
  - **Linux (X11)**: Uses `x11rb` with XScreenSaver for idle detection, EWMH (`_NET_ACTIVE_WINDOW`, `_NET_CLIENT_LIST`) for window queries and focus switching, and XTest (`fake_input`) for simulated input.
  - **Linux (Wayland)**: Uses `evdev` to create a virtual input device via `/dev/uinput`. Due to Wayland's security model, input is emitted globally.
  - **Windows**: Uses the `windows` crate (Win32 API) with `GetLastInputInfo` for idle detection, `EnumWindows` and `GetWindowTextW` for window lookup, `SetForegroundWindow` (with `AttachThreadInput` fallback) for focus switching, and `SendInput` for simulated events.

---

## Installation

### Prerequisites

- [Rust](https://www.rust-lang.org/) (stable toolchain, 2021 edition)

### Building from Source

```bash
git clone https://github.com/hclhls/RDPBeacon.git
cd RDPBeacon
cargo build --release
```

The compiled binary will be located at `target/release/rdpbeacon` (or `target/release/rdpbeacon.exe` on Windows).

---

## CLI Usage

```
rdpbeacon [OPTIONS] <COMMAND>
```

### Global Options

- `-c, --config <PATH>`: Path to a TOML configuration file.
- `-b, --backend <x11|wayland|windows>`: Manually override the display backend (overriding auto-detection and configuration file).
- `-v, --verbose`: Enable verbose (debug) logging.
- `-h, --help`: Print help information.
- `-V, --version`: Print version information.

### Subcommands

| Subcommand | Description |
| :--- | :--- |
| `run` | Runs the beacon loop in the terminal. Add `--daemon` to detach with tray controls. Handles `Ctrl-C` for clean shutdown. |
| `once` | Executes a single beacon cycle immediately (ignoring `idle_threshold`). Prints the outcome (`Sent`, `SkippedNoWindow`, or `SkippedActive`). |
| `check` | Inspects your environment: reports the detected/selected backend, current system idle time, and Horizon window status. |

### Examples

```bash
# Check environment and detect Horizon window
rdpbeacon check

# Trigger an immediate beacon test cycle
rdpbeacon once

# Run the daemon with default settings
rdpbeacon run

# Run with a custom config file
rdpbeacon --config config.toml run

# Run in the background with a tray icon
rdpbeacon --config config.toml run --daemon

# Force the X11 backend
rdpbeacon --backend x11 run
```

---

## Tray Daemon Mode

`rdpbeacon run --daemon` returns after the background application has initialized
its backend and registered its local tray. You can then close the terminal.
An initialization failure or a ten-second startup timeout returns an error and
cleans up the child process. Use a regular configuration file; temporary shell
process-substitution paths may disappear before the background process reads them.

The tray menu provides:

- **Status**: `Running`, `Stopping`, or `Stopped: <reason>`.
- **Enable logging**: An initially unchecked toggle. Enabling it appends file
  logs; disabling it stops recording while beacons continue. Each launch starts
  with logging disabled. `-v` enables debug verbosity when logging is turned on;
  `RUST_LOG` controls filtering without turning logging on.
- **Quit**: Stops the worker after any current beacon and focus-restoration
  attempt finish, then removes the tray. Repeated clicks are harmless.

Log files are created only when logging is enabled:

| Platform | Log path |
| :--- | :--- |
| Linux | `$XDG_STATE_HOME/rdpbeacon/rdpbeacon.log`, or `$HOME/.local/state/rdpbeacon/rdpbeacon.log` when the state variable is absent or relative |
| Windows | `%LOCALAPPDATA%\RDPBeacon\logs\rdpbeacon.log` |

Records include a Unix timestamp in milliseconds, severity, and message. Files
rotate at 5 MiB, keeping the current log and one `rdpbeacon.log.1` backup. A
private `rdpbeacon.log.lock` file serializes writers from concurrent launches.
Logging
errors clear the checkbox and appear in status; the beacon worker continues.
Enabling logging after a worker failure records its saved stop reason.

Linux requires a running session D-Bus and a StatusNotifierItem tray host. KDE
and other compatible desktops provide this; GNOME typically uses the
[AppIndicator and KStatusNotifierItem Support extension](https://extensions.gnome.org/extension/615/appindicator-support/).
No GTK development packages are required to build RDPBeacon. If the tray host
is unavailable, daemon startup fails. If it disappears later, the application
shuts down gracefully. Windows uses the native notification area and restores
its icon when Explorer restarts; a failed restoration triggers graceful shutdown.

The existing `max_misses` limit stops beacon processing when Horizon remains
missing. Foreground `run` exits with an error; daemon mode keeps the tray with
the stopped reason until you choose Quit. There is no automatic restart, login
autostart, or single-instance enforcement. Each launch starts a separate worker.

---

## Configuration

RDPBeacon can be configured using a TOML file. See [`config.example.toml`](config.example.toml) for a commented template.

### Configuration Fields

| Option | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `interval` | duration | `"4m"` | Base interval between beacon cycles (e.g., `"4m"`, `"240s"`). |
| `jitter` | duration | `"20s"` | Random variance added or subtracted to the interval (`interval ± jitter`). Sleep duration is clamped to a minimum of 1s. |
| `idle_threshold` | duration | `"3m"` | Inactivity threshold before sending a beacon when the Horizon window is currently focused. If you are actively typing inside Horizon, the cycle is skipped. Background sessions are kept alive regardless. |
| `notify_before` | duration | `"0s"` | Show a desktop notification this long before each beacon (D-Bus on Linux, tray balloon on Windows). `"0s"` disables it; if the wait is shorter than this value, the notice is shown immediately. |
| `key` | string | `"Shift_L"` | Key to simulate when `mode = "key"`. Supports `Shift_L`, `Ctrl`, `Alt`, `F1`–`F24`, `Space`, `Enter`, `Tab`, `Esc`, arrow keys. (`Shift_L` is recommended as it produces no character and maps cleanly across standard 101/104-key PC layouts). |
| `mode` | string | `"mouse"` | Beacon mode: `"mouse"` (nudges cursor +1 px and -1 px) or `"key"` (presses and releases `key` in Horizon). Mouse mode requires the pointer to be inside the Horizon desktop surface. |
| `window_match` | string | `"Omnissa Horizon Client"` | Case-insensitive substring matched against window title (and WM_CLASS on X11). Re-evaluated every cycle. |
| `max_misses` | integer | `5` | Maximum consecutive cycles the Horizon Client window can be missing before `run` exits with an error. |
| `backend` | string | `None` (auto) | Display backend override (`"x11"`, `"wayland"`, or `"windows"`). |

---

## Wayland Setup

Wayland compositors intentionally isolate applications and prevent cross-client window manipulation or focus switching. RDPBeacon uses the Linux `/dev/uinput` kernel interface via `evdev` to inject synthetic input events.

### Configuring `/dev/uinput` Permissions

By default, `/dev/uinput` requires root permissions. To run RDPBeacon as a standard user:

1. Create a udev rule allowing members of the `input` group to access `uinput`:
   ```bash
   echo 'KERNEL=="uinput", GROUP="input", MODE="0660"' | sudo tee /etc/udev/rules.d/99-uinput.rules
   ```

2. Add your user account to the `input` group:
   ```bash
   sudo usermod -aG input $USER
   ```

3. Reload and trigger udev rules:
   ```bash
   sudo udevadm control --reload-rules && sudo udevadm trigger
   ```

4. Log out and back into your session (or run `newgrp input`) for group changes to take effect.

### Wayland Limitations

> [!NOTE]
> - **No Focus Switching on Wayland**: Because Wayland does not permit client applications to inspect or focus windows belonging to other applications, RDPBeacon cannot automatically bring the Omnissa Horizon Client window to the foreground on Wayland. Synthetic events are delivered to the currently active window.
> - **No Cross-Compositor Idle Detection**: Wayland compositor isolation also prevents querying global user inactivity without compositor-specific protocols. On Wayland, beacons fire unconditionally at the configured interval regardless of local activity.

---

## Caveats & Policy Compliance

> [!IMPORTANT]
> - **Organizational Policy**: RDPBeacon is an own-use productivity tool. You are responsible for ensuring that running this utility complies with your organization's IT security, Acceptable Use, and remote work policies.
> - **Input Inactivity vs. Hard Session Limits**: RDPBeacon only simulates local user input to prevent **inactivity/idle timeouts**. It does **not** and cannot bypass hard administrative session lifetime limits (such as an 8-hour maximum session duration enforced by the Horizon Connection Server or identity provider).
> - **Local Screen Auto-Lock / Screensaver**: Because beacons inject real operating system input, running RDPBeacon also resets your local desktop's idle timer, which may prevent local screen savers or auto-lock from engaging while the daemon is running. Always manually lock your screen (e.g. `Win+L` or `Super+L`) when stepping away from your workstation.
> - **Locked Remote Windows Desktops**: Local input injection does not confirm that a locked remote desktop received the event or reset its idle timer. RDPBeacon cannot guarantee session retention after locking Windows inside Horizon. See [Troubleshooting screen-lock timeouts](docs/troubleshooting-screen-lock.md) for diagnostics and the server-side settings that retain a locked session.

---

## Manual Smoke Test Checklist

Follow this checklist to verify your RDPBeacon installation:

1. **Check Environment (`rdpbeacon check`)**:
   - Ensure the detected backend matches your desktop session (`X11`, `Wayland`, or `Windows`).
   - Confirm that the reported idle time increases when your hands are off the keyboard/mouse (X11 & Windows).
   - Start Omnissa Horizon Client and confirm `rdpbeacon check` reports `Horizon window: found`.

2. **Test Single Beacon (`rdpbeacon once`)**:
   - Focus another window (e.g., a terminal or text editor).
   - For the default mouse mode, keep the pointer inside the Horizon desktop surface.
   - Run `rdpbeacon once`.
   - On X11 and Windows: verify that the Horizon window is briefly focused, the beacon is sent, and focus returns to your previous window within ~100 ms.
   - On Wayland: verify that the beacon event is emitted (e.g., using `wev` or by observing a 1 px mouse nudge in mouse mode).

3. **Test Daemon Loop (`rdpbeacon run`)**:
   - Run `rdpbeacon` with a short interval:
     ```bash
     rdpbeacon --config <(printf 'interval = "10s"\nidle_threshold = "5s"\njitter = "2s"\n') run
     ```
   - On X11/Windows, move your mouse or type: observe that cycles are skipped with "User active".
   - Leave the system idle for > 5 seconds: observe that the beacon fires and resets the Horizon Client idle timeout.
   - Press `Ctrl-C`: verify the daemon shuts down immediately and cleanly.

4. **Verify Remote VM Session**:
   - Confirm that the remote desktop session within Omnissa Horizon Client remains active and does not disconnect due to inactivity.

5. **Test Tray Mode (`rdpbeacon run --daemon`)**:
   - Launch with a regular temporary TOML file, close the terminal, and verify the tray remains.
   - On a fresh profile, verify logging is unchecked and no log file exists.
   - Enable logging, inspect the file, disable logging, and enable it again to verify append behavior.
   - Choose Quit during a sleep and during a beacon; verify process exit and focus restoration.
   - With a short interval and `max_misses = 1`, close Horizon and verify the tray reports Stopped; enable logging to capture the reason, then quit.
   - Linux: test with a compatible tray host on X11 and Wayland. An isolated D-Bus session without a tray host must fail startup. Host loss must stop the daemon.

6. **Test Pre-Beacon Notification (`notify_before`)**:
   - Run with a short interval and a lead time:
     ```bash
     rdpbeacon --config <(printf 'interval = "20s"\njitter = "0s"\nnotify_before = "10s"\n') run
     ```
   - Verify that a desktop notification ("Beacon will be sent in 10s") appears about 10 seconds before each beacon (D-Bus notification on Linux, tray balloon on Windows).
   - Set `notify_before = "0s"` and verify that no notification appears.
   - Windows: restart Explorer and verify the icon returns and the menu remains usable.
   - Record the desktops actually tested. Wine tests or cross-compilation do not replace Windows desktop or Wayland input-delivery checks.

---

## Contributing

Contributions, bug reports, and feature requests are welcome! Please check out [CONTRIBUTING.md](CONTRIBUTING.md) for local setup, development guidelines, and pull request procedures.

---

## Security

If you discover a potential security issue, please refer to our [Security Policy](SECURITY.md) for reporting guidelines.

---

## License

This project is licensed under the Apache License, Version 2.0. See the [LICENSE](LICENSE) file for details.
