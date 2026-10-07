use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use anyhow::{Context, Result};
use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{AttributeSet, EventType, InputEvent, Key, RelativeAxisType};

use crate::backend::{Backend, WindowId};
use crate::config::Mode;

static WARNED_IDLE: AtomicBool = AtomicBool::new(false);

pub fn permission_hint() -> &'static str {
    "To grant access to /dev/uinput, create a udev rule:\n  \
     echo 'KERNEL==\"uinput\", GROUP=\"input\", MODE=\"0660\"' | sudo tee /etc/udev/rules.d/99-uinput.rules\n\
     and add your user to the input group:\n  \
     sudo usermod -aG input $USER\n\
     Then reload udev rules (sudo udevadm control --reload-rules && sudo udevadm trigger) and re-login."
}

pub fn wrap_uinput_err(err: std::io::Error) -> anyhow::Error {
    if err.kind() == std::io::ErrorKind::PermissionDenied || err.raw_os_error() == Some(13) {
        anyhow::anyhow!(
            "Permission denied accessing /dev/uinput: {err}.\n\n{}",
            permission_hint()
        )
    } else {
        anyhow::anyhow!("Failed to access /dev/uinput: {err}")
    }
}

pub fn key_code(name: &str) -> Result<Key> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Key name cannot be empty");
    }

    let upper = trimmed.to_ascii_uppercase();
    let stripped = upper.strip_prefix("KEY_").unwrap_or(&upper);

    if stripped.starts_with('F') {
        if let Ok(num) = stripped[1..].parse::<u32>() {
            let key = match num {
                1 => Some(Key::KEY_F1),
                2 => Some(Key::KEY_F2),
                3 => Some(Key::KEY_F3),
                4 => Some(Key::KEY_F4),
                5 => Some(Key::KEY_F5),
                6 => Some(Key::KEY_F6),
                7 => Some(Key::KEY_F7),
                8 => Some(Key::KEY_F8),
                9 => Some(Key::KEY_F9),
                10 => Some(Key::KEY_F10),
                11 => Some(Key::KEY_F11),
                12 => Some(Key::KEY_F12),
                13 => Some(Key::KEY_F13),
                14 => Some(Key::KEY_F14),
                15 => Some(Key::KEY_F15),
                16 => Some(Key::KEY_F16),
                17 => Some(Key::KEY_F17),
                18 => Some(Key::KEY_F18),
                19 => Some(Key::KEY_F19),
                20 => Some(Key::KEY_F20),
                21 => Some(Key::KEY_F21),
                22 => Some(Key::KEY_F22),
                23 => Some(Key::KEY_F23),
                24 => Some(Key::KEY_F24),
                _ => None,
            };
            if let Some(k) = key {
                return Ok(k);
            }
        }
    }

    match stripped {
        "SPACE" => return Ok(Key::KEY_SPACE),
        "ENTER" | "RETURN" => return Ok(Key::KEY_ENTER),
        "ESC" | "ESCAPE" => return Ok(Key::KEY_ESC),
        "BACKSPACE" => return Ok(Key::KEY_BACKSPACE),
        "TAB" => return Ok(Key::KEY_TAB),
        "SHIFT" | "LEFTSHIFT" | "SHIFT_L" => return Ok(Key::KEY_LEFTSHIFT),
        "RIGHTSHIFT" | "SHIFT_R" => return Ok(Key::KEY_RIGHTSHIFT),
        "CTRL" | "CONTROL" | "LEFTCTRL" | "CONTROL_L" => return Ok(Key::KEY_LEFTCTRL),
        "RIGHTCTRL" | "CONTROL_R" => return Ok(Key::KEY_RIGHTCTRL),
        "ALT" | "LEFTALT" | "ALT_L" => return Ok(Key::KEY_LEFTALT),
        "RIGHTALT" | "ALT_R" => return Ok(Key::KEY_RIGHTALT),
        "UP" => return Ok(Key::KEY_UP),
        "DOWN" => return Ok(Key::KEY_DOWN),
        "LEFT" => return Ok(Key::KEY_LEFT),
        "RIGHT" => return Ok(Key::KEY_RIGHT),
        "PAGEUP" => return Ok(Key::KEY_PAGEUP),
        "PAGEDOWN" => return Ok(Key::KEY_PAGEDOWN),
        "HOME" => return Ok(Key::KEY_HOME),
        "END" => return Ok(Key::KEY_END),
        "INSERT" => return Ok(Key::KEY_INSERT),
        "DELETE" => return Ok(Key::KEY_DELETE),
        "CAPSLOCK" => return Ok(Key::KEY_CAPSLOCK),
        "NUMLOCK" => return Ok(Key::KEY_NUMLOCK),
        "SCROLLLOCK" => return Ok(Key::KEY_SCROLLLOCK),
        "A" => return Ok(Key::KEY_A),
        "B" => return Ok(Key::KEY_B),
        "C" => return Ok(Key::KEY_C),
        "D" => return Ok(Key::KEY_D),
        "E" => return Ok(Key::KEY_E),
        "F" => return Ok(Key::KEY_F),
        "G" => return Ok(Key::KEY_G),
        "H" => return Ok(Key::KEY_H),
        "I" => return Ok(Key::KEY_I),
        "J" => return Ok(Key::KEY_J),
        "K" => return Ok(Key::KEY_K),
        "L" => return Ok(Key::KEY_L),
        "M" => return Ok(Key::KEY_M),
        "N" => return Ok(Key::KEY_N),
        "O" => return Ok(Key::KEY_O),
        "P" => return Ok(Key::KEY_P),
        "Q" => return Ok(Key::KEY_Q),
        "R" => return Ok(Key::KEY_R),
        "S" => return Ok(Key::KEY_S),
        "T" => return Ok(Key::KEY_T),
        "U" => return Ok(Key::KEY_U),
        "V" => return Ok(Key::KEY_V),
        "W" => return Ok(Key::KEY_W),
        "X" => return Ok(Key::KEY_X),
        "Y" => return Ok(Key::KEY_Y),
        "Z" => return Ok(Key::KEY_Z),
        "0" => return Ok(Key::KEY_0),
        "1" => return Ok(Key::KEY_1),
        "2" => return Ok(Key::KEY_2),
        "3" => return Ok(Key::KEY_3),
        "4" => return Ok(Key::KEY_4),
        "5" => return Ok(Key::KEY_5),
        "6" => return Ok(Key::KEY_6),
        "7" => return Ok(Key::KEY_7),
        "8" => return Ok(Key::KEY_8),
        "9" => return Ok(Key::KEY_9),
        _ => {}
    }

    if let Some(hex) = stripped.strip_prefix("0X") {
        if let Ok(val) = u16::from_str_radix(hex, 16) {
            return Ok(Key::new(val));
        }
    }

    anyhow::bail!("Unknown or unsupported key name for Wayland backend: {name}")
}

pub struct WaylandBackend {
    device: Mutex<Option<VirtualDevice>>,
}

impl WaylandBackend {
    pub fn new() -> Result<Self> {
        let mut keys = AttributeSet::<Key>::new();
        for code in 1..=0x2ff {
            keys.insert(Key::new(code));
        }

        let mut rel_axes = AttributeSet::<RelativeAxisType>::new();
        rel_axes.insert(RelativeAxisType::REL_X);
        rel_axes.insert(RelativeAxisType::REL_Y);

        let builder = VirtualDeviceBuilder::new()
            .map_err(wrap_uinput_err)?;

        let device = builder
            .name("RDPBeacon Virtual Device")
            .with_keys(&keys)
            .map_err(wrap_uinput_err)?
            .with_relative_axes(&rel_axes)
            .map_err(wrap_uinput_err)?
            .build()
            .map_err(wrap_uinput_err)?;

        // Give the compositor/libinput a brief moment to detect the new uinput device
        // before immediately injecting events (especially important for `once` command).
        std::thread::sleep(Duration::from_millis(300));

        Ok(Self {
            device: Mutex::new(Some(device)),
        })
    }

    #[cfg(test)]
    pub(crate) fn test_backend() -> Self {
        Self {
            device: Mutex::new(None),
        }
    }
}

impl Backend for WaylandBackend {
    fn idle_time(&self) -> Result<Duration> {
        if !WARNED_IDLE.swap(true, Ordering::Relaxed) {
            log::warn!(
                "Wayland idle detection is not implemented; firing beacon unconditionally at configured interval"
            );
        }
        Ok(Duration::MAX)
    }

    fn find_horizon_window(&self, _matcher: &str) -> Result<Option<WindowId>> {
        Ok(Some(0))
    }

    fn active_window(&self) -> Result<Option<WindowId>> {
        Ok(None)
    }

    fn focus(&self, _w: WindowId) -> Result<()> {
        Ok(())
    }

    fn send_beacon(&self, mode: Mode, key: &str) -> Result<()> {
        let mut guard = self
            .device
            .lock()
            .map_err(|e| anyhow::anyhow!("Virtual device lock poisoned: {e}"))?;
        let dev = guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Virtual device is not available"))?;

        match mode {
            Mode::Key => {
                let code = key_code(key)?;
                let down = InputEvent::new_now(EventType::KEY, code.code(), 1);
                let up = InputEvent::new_now(EventType::KEY, code.code(), 0);
                for i in 0..2 {
                    if i > 0 {
                        std::thread::sleep(Duration::from_millis(30));
                    }
                    dev.emit(&[down])
                        .context("Failed to emit key down event via uinput")?;
                    std::thread::sleep(Duration::from_millis(20));
                    dev.emit(&[up])
                        .context("Failed to emit key up event via uinput")?;
                }
            }
            Mode::Mouse => {
                let move_right = InputEvent::new_now(
                    EventType::RELATIVE,
                    RelativeAxisType::REL_X.0,
                    1,
                );
                let move_left = InputEvent::new_now(
                    EventType::RELATIVE,
                    RelativeAxisType::REL_X.0,
                    -1,
                );
                dev.emit(&[move_right])
                    .context("Failed to emit relative mouse +1 px motion via uinput")?;
                std::thread::sleep(std::time::Duration::from_millis(10));
                dev.emit(&[move_left])
                    .context("Failed to emit relative mouse -1 px motion via uinput")?;
            }
        }
        Ok(())
    }

    fn supports_focus(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_code_f15() {
        let key = key_code("F15").expect("F15 should be recognized");
        assert_eq!(key, Key::KEY_F15);

        let key_lower = key_code("f15").expect("f15 should be recognized case-insensitively");
        assert_eq!(key_lower, Key::KEY_F15);

        let key_prefix = key_code("key_f15").expect("KEY_F15 should be recognized");
        assert_eq!(key_prefix, Key::KEY_F15);
    }

    #[test]
    fn key_code_unknown_errors() {
        assert!(key_code("unknown_key_xyz_123").is_err());
        assert!(key_code("F999").is_err());
        assert!(key_code("").is_err());
    }

    #[test]
    fn permission_hint_mentions_udev_and_input_group() {
        let hint = permission_hint();
        assert!(
            hint.to_lowercase().contains("udev"),
            "permission hint must mention udev"
        );
        assert!(
            hint.to_lowercase().contains("input"),
            "permission hint must mention input group"
        );
        assert!(
            hint.contains("KERNEL==\"uinput\""),
            "permission hint must mention KERNEL==\"uinput\""
        );
        assert!(
            hint.to_lowercase().contains("usermod"),
            "permission hint must mention usermod"
        );
    }

    #[test]
    fn wrap_uinput_err_permission_denied_includes_hint() {
        let err1 = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Permission denied");
        let anyhow_err1 = wrap_uinput_err(err1);
        let msg1 = anyhow_err1.to_string();
        assert!(msg1.contains("Permission denied"));
        assert!(msg1.contains("KERNEL==\"uinput\""));
        assert!(msg1.contains("input"));

        let err2 = std::io::Error::from_raw_os_error(13);
        let anyhow_err2 = wrap_uinput_err(err2);
        let msg2 = anyhow_err2.to_string();
        assert!(msg2.contains("Permission denied"));
        assert!(msg2.contains("KERNEL==\"uinput\""));
        assert!(msg2.contains("input"));
    }

    #[test]
    fn wayland_backend_trait_semantics() {
        let backend = WaylandBackend::test_backend();
        assert!(!backend.supports_focus());
        assert_eq!(backend.find_horizon_window("any").unwrap(), Some(0));
        assert_eq!(backend.active_window().unwrap(), None);
        assert!(backend.focus(0).is_ok());
        assert_eq!(backend.idle_time().unwrap(), Duration::MAX);
    }

    #[test]
    fn key_code_various_mappings() {
        assert_eq!(key_code("F1").unwrap(), Key::KEY_F1);
        assert_eq!(key_code("F24").unwrap(), Key::KEY_F24);
        assert_eq!(key_code("space").unwrap(), Key::KEY_SPACE);
        assert_eq!(key_code("Enter").unwrap(), Key::KEY_ENTER);
        assert_eq!(key_code("return").unwrap(), Key::KEY_ENTER);
        assert_eq!(key_code("esc").unwrap(), Key::KEY_ESC);
        assert_eq!(key_code("escape").unwrap(), Key::KEY_ESC);
        assert_eq!(key_code("backspace").unwrap(), Key::KEY_BACKSPACE);
        assert_eq!(key_code("tab").unwrap(), Key::KEY_TAB);
        assert_eq!(key_code("shift").unwrap(), Key::KEY_LEFTSHIFT);
        assert_eq!(key_code("ctrl").unwrap(), Key::KEY_LEFTCTRL);
        assert_eq!(key_code("alt").unwrap(), Key::KEY_LEFTALT);
        assert_eq!(key_code("a").unwrap(), Key::KEY_A);
        assert_eq!(key_code("Z").unwrap(), Key::KEY_Z);
        assert_eq!(key_code("0").unwrap(), Key::KEY_0);
        assert_eq!(key_code("9").unwrap(), Key::KEY_9);
        assert_eq!(key_code("0x1e").unwrap(), Key::new(0x1e));
    }
}
