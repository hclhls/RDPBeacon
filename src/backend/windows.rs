#[cfg(windows)]
use anyhow::{Context, Result};
#[cfg(windows)]
use std::sync::{mpsc, Mutex, Once};
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use windows::core::w;
#[cfg(windows)]
use windows::Win32::Foundation::{COLORREF, HINSTANCE, BOOL, HWND, LPARAM, LRESULT, WPARAM};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetTickCount64;
#[cfg(windows)]
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE,
    KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, LASTINPUTINFO, MAPVK_VK_TO_VSC,
    MOUSEEVENTF_MOVE, MOUSEINPUT, VIRTUAL_KEY,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows, GetClassNameW,
    GetForegroundWindow, GetMessageW, GetSystemMetrics, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible, PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    SetLayeredWindowAttributes, ShowWindow, TranslateMessage, LWA_ALPHA, MSG, WM_APP, WM_CLOSE,
    WM_DESTROY, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SW_SHOWNOACTIVATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

#[cfg(windows)]
use crate::backend::fade::{fade_alpha, FADE_STEP};
#[cfg(windows)]
use crate::backend::{Backend, WindowId};
#[cfg(windows)]
use crate::config::Mode;

#[cfg(test)]
pub fn title_matches(title: &str, matcher: &str) -> bool {
    crate::backend::title_matches(title, "", matcher)
}

pub fn parse_virtual_key(key: &str) -> Option<u16> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
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
pub struct WindowsBackend {
    overlay: Mutex<Option<Overlay>>,
}

/// Private message used to change the overlay alpha on the thread that owns the window.
#[cfg(windows)]
const WM_FADE_ALPHA: u32 = WM_APP + 1;

#[cfg(windows)]
static REGISTER_OVERLAY_CLASS: Once = Once::new();

#[cfg(windows)]
unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_FADE_ALPHA => {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), wparam.0 as u8, LWA_ALPHA);
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// A full-screen black overlay owned by a dedicated thread with its own message pump.
///
/// Win32 windows can only be destroyed by the thread that created them. Keeping the window on
/// a dedicated pump thread lets any thread (including `Drop` on a different thread) close it
/// safely by posting `WM_CLOSE`, so a dark overlay can never be leaked.
#[cfg(windows)]
struct Overlay {
    hwnd: isize,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl Overlay {
    fn create() -> Result<Self> {
        let (tx, rx) = mpsc::channel::<Result<isize>>();
        let thread = std::thread::spawn(move || {
            let hwnd = match unsafe { Self::create_window() } {
                Ok(hwnd) => hwnd,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            if tx.send(Ok(hwnd.0 as isize)).is_err() {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                return;
            }
            unsafe {
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, HWND::default(), 0, 0).0 > 0 {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });

        match rx.recv() {
            Ok(Ok(hwnd)) => Ok(Self {
                hwnd,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                anyhow::bail!("Fade overlay thread exited unexpectedly")
            }
        }
    }

    unsafe fn create_window() -> Result<HWND> {
        // Without DPI awareness the virtual screen metrics are scaled and the overlay may not
        // cover every monitor.
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let class_name = w!("RDPBeaconFadeOverlay");
        REGISTER_OVERLAY_CLASS.call_once(|| {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(overlay_wndproc),
                hInstance: instance,
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                lpszClassName: class_name,
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                log::warn!("Failed to register fade overlay window class");
            }
        });

        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let cx = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let cy = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        let ex_style =
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;

        let hwnd = CreateWindowExW(
            ex_style,
            class_name,
            w!(""),
            WS_POPUP,
            x,
            y,
            cx,
            cy,
            HWND::default(),
            None,
            instance,
            None,
        )?;

        // Start fully transparent.
        if let Err(e) = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA) {
            let _ = DestroyWindow(hwnd);
            return Err(e.into());
        }

        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        Ok(hwnd)
    }

    fn set_alpha(&self, alpha: u8) -> Result<()> {
        unsafe {
            PostMessageW(
                HWND(self.hwnd as *mut _),
                WM_FADE_ALPHA,
                WPARAM(alpha as usize),
                LPARAM(0),
            )
        }
        .context("Failed to update fade overlay alpha")
    }
}

#[cfg(windows)]
impl Drop for Overlay {
    fn drop(&mut self) {
        // Posting is safe from any thread; the owner thread destroys the window and exits.
        unsafe {
            let _ = PostMessageW(HWND(self.hwnd as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(windows)]
impl WindowsBackend {
    pub fn new() -> Result<Self> {
        Ok(Self {
            overlay: Mutex::new(None),
        })
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
        if ctx
            .best
            .as_ref()
            .map_or(true, |(best_score, ..)| score > *best_score)
        {
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
            log::debug!("Selected Horizon window {wid:#x} (score {score}): \"{title}\" [{class}]");
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
                for i in 0..2 {
                    if i > 0 {
                        std::thread::sleep(Duration::from_millis(30));
                    }
                    let sent = unsafe { SendInput(&[down], std::mem::size_of::<INPUT>() as i32) };
                    if sent != 1 {
                        anyhow::bail!(
                            "SendInput failed to send keyboard down event (sent {sent} of 1)"
                        );
                    }
                    std::thread::sleep(Duration::from_millis(20));
                    let sent = unsafe { SendInput(&[up], std::mem::size_of::<INPUT>() as i32) };
                    if sent != 1 {
                        anyhow::bail!(
                            "SendInput failed to send keyboard up event (sent {sent} of 1)"
                        );
                    }
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
                let sent = unsafe { SendInput(&[move_right], std::mem::size_of::<INPUT>() as i32) };
                if sent != 1 {
                    anyhow::bail!("SendInput failed to send mouse move event (sent {sent} of 1)");
                }
                std::thread::sleep(Duration::from_millis(10));
                let sent = unsafe { SendInput(&[move_back], std::mem::size_of::<INPUT>() as i32) };
                if sent != 1 {
                    anyhow::bail!("SendInput failed to send mouse return event (sent {sent} of 1)");
                }
            }
        }
        Ok(())
    }

    fn fade_out(&self, duration: Duration) -> Result<()> {
        if duration.is_zero() {
            return Ok(());
        }

        // Dropping a stale overlay asks its owner thread to close it and joins that thread.
        drop(self.overlay.lock().unwrap().take());

        let overlay = Overlay::create().context("Failed to create fade overlay")?;
        // Publish the overlay immediately so Drop or fade_in can clean it up.
        *self.overlay.lock().unwrap() = Some(overlay);

        let start = std::time::Instant::now();
        loop {
            let elapsed = start.elapsed();
            let alpha = fade_alpha(elapsed, duration);
            if let Err(e) = self.set_overlay_alpha(alpha) {
                // Dropping the overlay closes it so the screen is never left dark.
                drop(self.overlay.lock().unwrap().take());
                return Err(e);
            }
            if elapsed >= duration {
                break;
            }
            std::thread::sleep(FADE_STEP);
        }

        Ok(())
    }

    fn fade_in(&self, duration: Duration) -> Result<()> {
        if self.overlay.lock().unwrap().is_none() {
            return Ok(());
        }

        if !duration.is_zero() {
            let start = std::time::Instant::now();
            loop {
                let elapsed = start.elapsed();
                let alpha = 255 - fade_alpha(elapsed, duration);
                if self.set_overlay_alpha(alpha).is_err() {
                    break;
                }
                if elapsed >= duration {
                    break;
                }
                std::thread::sleep(FADE_STEP);
            }
        }

        drop(self.overlay.lock().unwrap().take());
        Ok(())
    }
}

#[cfg(windows)]
impl WindowsBackend {
    fn set_overlay_alpha(&self, alpha: u8) -> Result<()> {
        match self.overlay.lock().unwrap().as_ref() {
            Some(overlay) => overlay.set_alpha(alpha),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_case_insensitive_substring() {
        assert!(title_matches("Omnissa Horizon Client", "horizon"));
        assert!(title_matches(
            "OMNISSA HORIZON CLIENT - Desktop 1",
            "horizon client"
        ));
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
