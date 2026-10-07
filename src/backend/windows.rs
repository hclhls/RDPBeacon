#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use anyhow::{Context, Result};
#[cfg(windows)]
use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetTickCount64;
#[cfg(windows)]
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE,
    KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, LASTINPUTINFO, MAPVK_VK_TO_VSC, MOUSEINPUT,
    MOUSEEVENTF_MOVE, VIRTUAL_KEY,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible, SetForegroundWindow,
};

#[cfg(windows)]
use crate::backend::{Backend, WindowId};
#[cfg(windows)]
use crate::config::Mode;

pub fn title_matches(title: &str, matcher: &str) -> bool {
    crate::backend::title_matches(title, "", matcher)
}

pub fn parse_virtual_key(key: &str) -> Option<u16> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        if let Ok(val) = u16::from_str_radix(hex, 16) {
            return Some(val);
        }
    }

    let upper = trimmed.to_ascii_uppercase();
    let stripped = upper.strip_prefix("VK_").unwrap_or(&upper);

    if stripped.starts_with('F') {
        if let Ok(num) = stripped[1..].parse::<u16>() {
            if (1..=24).contains(&num) {
                // VK_F1 is 0x70 (112), VK_F15 is 0x7E (126), VK_F24 is 0x87 (135)
                return Some(0x70 + num - 1);
            }
        }
    }

    match stripped {
        "SPACE" => Some(0x20),
        "RETURN" | "ENTER" => Some(0x0D),
        "ESCAPE" | "ESC" => Some(0x1B),
        "BACKSPACE" => Some(0x08),
        "TAB" => Some(0x09),
        "SHIFT" | "SHIFT_L" | "SHIFT_R" => Some(0x10),
        "CTRL" | "CONTROL" | "CONTROL_L" | "CONTROL_R" => Some(0x11),
        "ALT" | "MENU" | "ALT_L" | "ALT_R" => Some(0x12),
        "LEFT" => Some(0x25),
        "UP" => Some(0x26),
        "RIGHT" => Some(0x27),
        "DOWN" => Some(0x28),
        _ => {
            if stripped.len() == 1 {
                let c = stripped.chars().next().unwrap();
                if c.is_ascii_alphanumeric() {
                    return Some(c as u16);
                }
            }
            None
        }
    }
}

#[cfg(windows)]
pub struct WindowsBackend;

#[cfg(windows)]
impl WindowsBackend {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
}

#[cfg(windows)]
struct EnumContext<'a> {
    matcher: &'a str,
    best: Option<(i32, HWND, String, String)>,
    index: usize,
}

#[cfg(windows)]
unsafe extern "system" fn enum_window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }

    let mut buf = [0u16; 512];
    let len = GetWindowTextW(hwnd, &mut buf);
    let title = if len > 0 {
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        String::new()
    };

    let mut class_buf = [0u16; 256];
    let class_len = GetClassNameW(hwnd, &mut class_buf);
    let class = if class_len > 0 {
        String::from_utf16_lossy(&class_buf[..class_len as usize])
    } else {
        String::new()
    };

    let ctx = &mut *(lparam.0 as *mut EnumContext);
    let idx = ctx.index;
    ctx.index += 1;

    if let Some(score) = crate::backend::score_window(&title, &class, ctx.matcher, idx) {
        if ctx.best.as_ref().map_or(true, |(best_score, ..)| score > *best_score) {
            ctx.best = Some((score, hwnd, title, class));
        }
    }

    BOOL(1)
}

#[cfg(windows)]
impl Backend for WindowsBackend {
    fn supports_focus(&self) -> bool {
        true
    }

    fn idle_time(&self) -> Result<Duration> {
        let mut lii = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        unsafe {
            GetLastInputInfo(&mut lii)
                .ok()
                .context("GetLastInputInfo failed")?;
            let tick64 = GetTickCount64();
            let idle_ms = (tick64 as u32).wrapping_sub(lii.dwTime);
            Ok(Duration::from_millis(idle_ms as u64))
        }
    }

    fn find_horizon_window(&self, matcher: &str) -> Result<Option<WindowId>> {
        if matcher.is_empty() {
            return Ok(None);
        }
        let mut ctx = EnumContext {
            matcher,
            best: None,
            index: 0,
        };
        unsafe {
            let _ = EnumWindows(Some(enum_window_proc), LPARAM(&mut ctx as *mut _ as isize));
        }
        if let Some((score, hwnd, title, class)) = ctx.best {
            let wid = hwnd.0 as usize as WindowId;
            log::debug!(
                "Selected Horizon window {wid:#x} (score {score}): \"{title}\" [{class}]"
            );
            Ok(Some(wid))
        } else {
            Ok(None)
        }
    }

    fn active_window(&self) -> Result<Option<WindowId>> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_invalid() || hwnd.0.is_null() {
            Ok(None)
        } else {
            Ok(Some(hwnd.0 as usize as WindowId))
        }
    }

    fn focus(&self, w: WindowId) -> Result<()> {
        let hwnd = HWND(w as usize as *mut core::ffi::c_void);
        unsafe {
            if SetForegroundWindow(hwnd).as_bool() {
                return Ok(());
            }

            // Fallback: Windows foreground lock trick with AttachThreadInput
            let cur_thread = GetCurrentThreadId();
            let fg_hwnd = GetForegroundWindow();
            let fg_thread = if !fg_hwnd.is_invalid() && !fg_hwnd.0.is_null() {
                GetWindowThreadProcessId(fg_hwnd, None)
            } else {
                0
            };

            let mut attached = false;
            if fg_thread != 0 && fg_thread != cur_thread {
                attached = AttachThreadInput(cur_thread, fg_thread, true).as_bool();
                if attached {
                    log::debug!(
                        "Attached thread input {cur_thread} -> {fg_thread} to focus window {w:#x}"
                    );
                }
            }

            let success = SetForegroundWindow(hwnd).as_bool();

            if attached {
                let _ = AttachThreadInput(cur_thread, fg_thread, false);
                log::debug!("Detached thread input {cur_thread} -> {fg_thread}");
            }

            if !success {
                log::warn!(
                    "Failed to focus window {w:#x} due to Windows foreground lock; continuing with global input fallback"
                );
            }
        }
        Ok(())
    }

    fn send_beacon(&self, mode: Mode, key: &str) -> Result<()> {
        match mode {
            Mode::Key => {
                let vk = parse_virtual_key(key)
                    .map(VIRTUAL_KEY)
                    .ok_or_else(|| anyhow::anyhow!("Unknown or unsupported key: {key}"))?;
                let scan = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16;
                let down = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: vk,
                            wScan: scan,
                            dwFlags: KEYBD_EVENT_FLAGS(0),
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                let up = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: vk,
                            wScan: scan,
                            dwFlags: KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                let sent = unsafe {
                    SendInput(&[down], std::mem::size_of::<INPUT>() as i32)
                };
                if sent != 1 {
                    anyhow::bail!("SendInput failed to send keyboard down event (sent {sent} of 1)");
                }
                std::thread::sleep(Duration::from_millis(10));
                let sent = unsafe {
                    SendInput(&[up], std::mem::size_of::<INPUT>() as i32)
                };
                if sent != 1 {
                    anyhow::bail!("SendInput failed to send keyboard up event (sent {sent} of 1)");
                }
            }
            Mode::Mouse => {
                let move_right = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: 1,
                            dy: 0,
                            mouseData: 0,
                            dwFlags: MOUSEEVENTF_MOVE,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                let move_back = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: -1,
                            dy: 0,
                            mouseData: 0,
                            dwFlags: MOUSEEVENTF_MOVE,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                let sent = unsafe {
                    SendInput(&[move_right], std::mem::size_of::<INPUT>() as i32)
                };
                if sent != 1 {
                    anyhow::bail!("SendInput failed to send mouse move event (sent {sent} of 1)");
                }
                std::thread::sleep(Duration::from_millis(10));
                let sent = unsafe {
                    SendInput(&[move_back], std::mem::size_of::<INPUT>() as i32)
                };
                if sent != 1 {
                    anyhow::bail!("SendInput failed to send mouse return event (sent {sent} of 1)");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_case_insensitive_substring() {
        assert!(title_matches("Omnissa Horizon Client", "horizon"));
        assert!(title_matches("OMNISSA HORIZON CLIENT - Desktop 1", "horizon client"));
        assert!(title_matches("Omnissa Horizon Client", "OMNISSA"));
        assert!(title_matches("Horizon", "HORIZON"));
    }

    #[test]
    fn no_match_returns_false() {
        assert!(!title_matches("Omnissa Horizon Client", "Citrix"));
        assert!(!title_matches("Firefox", "horizon"));
        assert!(!title_matches("Omnissa Horizon Client", ""));
        assert!(!title_matches("", "horizon"));
    }

    #[test]
    fn parses_virtual_keys() {
        assert_eq!(parse_virtual_key("F15"), Some(0x7E));
        assert_eq!(parse_virtual_key("f15"), Some(0x7E));
        assert_eq!(parse_virtual_key("VK_F15"), Some(0x7E));
        assert_eq!(parse_virtual_key("F1"), Some(0x70));
        assert_eq!(parse_virtual_key("F24"), Some(0x87));
        assert_eq!(parse_virtual_key("0x7e"), Some(0x7E));
        assert_eq!(parse_virtual_key("0X7E"), Some(0x7E));
        assert_eq!(parse_virtual_key("SPACE"), Some(0x20));
        assert_eq!(parse_virtual_key("ENTER"), Some(0x0D));
        assert_eq!(parse_virtual_key("ESC"), Some(0x1B));
        assert_eq!(parse_virtual_key("A"), Some(0x41));
        assert_eq!(parse_virtual_key("unknown_key"), None);
        assert_eq!(parse_virtual_key(""), None);
    }
}
