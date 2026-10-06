# RDPBeacon

A lightweight, cross-platform CLI daemon written in Rust that keeps logged-in **Omnissa Horizon Client** sessions active on your local machine by periodically sending a small, harmless simulated input ("beacon"), preventing idle timeout disconnects.

---

## Features

- **Harmless Beacons**: Simulates an unmapped function key (`F15` by default) or a tiny 1-pixel mouse nudge and back (`mode = "mouse"`).
- **Idle Detection**: Only sends a beacon when the local user has been inactive for at least `idle_threshold`. If you are actively working, cycles are skipped automatically.
- **Smart Focus Management (X11 & Windows)**: Automatically saves your currently focused window, focuses the Horizon Client window, injects the beacon, waits ~100 ms, and restores your original window.
- **Wayland Support**: Uses Linux `/dev/uinput` to inject synthetic inputs without requiring root or display-server privileges.
- **Cross-Platform**: First-class support for Linux (X11), Linux (Wayland), and Windows.
- **Zero Caching**: Window lookup is re-evaluated every cycle, gracefully handling client restarts or window title changes.
- **Graceful Shutdown**: Responsive to `SIGINT` / `Ctrl-C` even during sleep intervals.

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
git clone https://github.com/example/rdpbeacon.git
cd rdpbeacon
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
- `-h, --help`: Print help information.
- `-V, --version`: Print version information.

### Subcommands

| Subcommand | Description |
| :--- | :--- |
| `run` | Starts the daemon loop. Periodically sends beacons when idle. Exits after `max_misses` consecutive cycles where Horizon is not found. Handles `Ctrl-C` for clean shutdown. |
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

# Force the X11 backend
rdpbeacon --backend x11 run
```

---

## Configuration

RDPBeacon can be configured using a TOML file. See [`config.example.toml`](config.example.toml) for a commented template.

### Configuration Fields

| Option | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `interval` | duration | `"4m"` | Base interval between beacon cycles (e.g., `"4m"`, `"240s"`). |
| `jitter` | duration | `"20s"` | Random variance added or subtracted to the interval (`interval ± jitter`). Sleep duration is clamped to a minimum of 1s. |
| `idle_threshold` | duration | `"3m"` | Minimum user inactivity time before a beacon is fired. If you have been active within this duration, the cycle is skipped. |
| `key` | string | `"F15"` | Key to simulate when `mode = "key"`. Supports `F1`–`F24`, `Space`, `Enter`, `Tab`, `Esc`, arrow keys, or hex codes (e.g., `0x7E`). |
| `mode` | string | `"key"` | Beacon mode: `"key"` (presses and releases `key`) or `"mouse"` (nudges cursor +1 px and -1 px). |
| `window_match` | string | `"Omnissa Horizon Client"` | Case-insensitive substring matched against window title or class. Re-evaluated every cycle. |
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

### Wayland Limitation

> [!NOTE]
> **No Focus Switching on Wayland**: Because Wayland does not permit client applications to inspect or focus windows belonging to other applications, RDPBeacon cannot automatically bring the Omnissa Horizon Client window to the foreground on Wayland.
> 
> Keystrokes or mouse nudges are sent to the currently active window. It is recommended to keep your Horizon session focused or set `mode = "mouse"` so that harmless relative cursor movements are used instead of keystrokes.

---

## Caveats & Policy Compliance

> [!IMPORTANT]
> - **Organizational Policy**: RDPBeacon is an own-use productivity tool. You are responsible for ensuring that running this utility complies with your organization's IT security, Acceptable Use, and remote work policies.
> - **Input Inactivity vs. Hard Session Limits**: RDPBeacon only simulates local user input to prevent **inactivity/idle timeouts**. It does **not** and cannot bypass hard administrative session lifetime limits (such as an 8-hour maximum session duration enforced by the Horizon Connection Server or identity provider).

---

## Manual Smoke Test Checklist

Follow this checklist to verify your RDPBeacon installation:

1. **Check Environment (`rdpbeacon check`)**:
   - Ensure the detected backend matches your desktop session (`X11`, `Wayland`, or `Windows`).
   - Confirm that the reported idle time increases when your hands are off the keyboard/mouse.
   - Start Omnissa Horizon Client and confirm `rdpbeacon check` reports `Horizon window: found`.

2. **Test Single Beacon (`rdpbeacon once`)**:
   - Focus another window (e.g., a terminal or text editor).
   - Run `rdpbeacon once`.
   - On X11 and Windows: verify that the Horizon window is briefly focused, the beacon is sent, and focus returns to your previous window within ~100 ms.
   - On Wayland: verify that the beacon event is emitted (e.g., using `wev` or by observing a 1 px mouse nudge in mouse mode).

3. **Test Daemon Loop (`rdpbeacon run`)**:
   - Run `rdpbeacon` with a short interval (e.g. `rdpbeacon --config <(echo 'interval = "10s"\nidle_threshold = "5s"\njitter = "2s"') run`).
   - Move your mouse or type: observe that cycles are skipped with "User active".
   - Leave the system idle for > 5 seconds: observe that the beacon fires and resets the Horizon Client idle timeout.
   - Press `Ctrl-C`: verify the daemon shuts down immediately and cleanly.
