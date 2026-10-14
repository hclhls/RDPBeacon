//! Desktop notifications used to warn the user before a beacon is sent.

/// Sends a short, best-effort desktop notification.
///
/// Implementations must never fail the caller: errors are logged and swallowed.
pub trait Notifier: Send + Sync {
    fn notify(&self, summary: &str, body: &str);
}

/// Notifier that discards every message.
#[allow(dead_code)]
pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    fn notify(&self, _summary: &str, _body: &str) {}
}

/// Returns the notifier for the current platform.
pub fn system_notifier() -> Box<dyn Notifier> {
    #[cfg(target_os = "linux")]
    {
        Box::new(DbusNotifier)
    }
    #[cfg(windows)]
    {
        Box::new(WindowsNotifier)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Box::new(NoopNotifier)
    }
}

#[cfg(target_os = "linux")]
struct DbusNotifier;

#[cfg(target_os = "linux")]
impl DbusNotifier {
    fn send(summary: &str, body: &str) -> anyhow::Result<()> {
        use std::collections::HashMap;
        use zbus::zvariant::Value;

        let connection = zbus::blocking::Connection::session()?;
        let hints: HashMap<&str, Value> = HashMap::new();
        connection.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "RDPBeacon",
                0u32,
                "",
                summary,
                body,
                Vec::<&str>::new(),
                hints,
                10_000i32,
            ),
        )?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Notifier for DbusNotifier {
    fn notify(&self, summary: &str, body: &str) {
        if let Err(e) = Self::send(summary, body) {
            log::warn!("Failed to send desktop notification: {e:#}");
        }
    }
}

#[cfg(windows)]
struct WindowsNotifier;

#[cfg(windows)]
impl WindowsNotifier {
    fn copy_wide<const N: usize>(dest: &mut [u16; N], text: &str) {
        for (slot, value) in dest.iter_mut().take(N - 1).zip(text.encode_utf16()) {
            *slot = value;
        }
    }

    /// Shows a balloon through a temporary notification-area icon that is
    /// removed again after a few seconds.
    fn send(summary: &str, body: &str) -> anyhow::Result<()> {
        use windows::core::w;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::Shell::{
            Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIIF_INFO, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, LoadIconW, HWND_MESSAGE, IDI_APPLICATION,
            WINDOW_EX_STYLE, WINDOW_STYLE,
        };

        unsafe {
            let instance = GetModuleHandleW(None)?;
            let hwnd: HWND = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("RDPBeaconNotify"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                None,
                instance,
                None,
            )?;
            let mut data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 2,
                uFlags: NIF_ICON | NIF_INFO,
                hIcon: LoadIconW(None, IDI_APPLICATION)?,
                dwInfoFlags: NIIF_INFO,
                ..Default::default()
            };
            Self::copy_wide(&mut data.szInfoTitle, summary);
            Self::copy_wide(&mut data.szInfo, body);
            if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                let _ = DestroyWindow(hwnd);
                anyhow::bail!("Shell_NotifyIconW(NIM_ADD) failed");
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            let _ = DestroyWindow(hwnd);
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Notifier for WindowsNotifier {
    fn notify(&self, summary: &str, body: &str) {
        let (summary, body) = (summary.to_owned(), body.to_owned());
        // The icon must stay alive while the balloon is visible; do not block the scheduler.
        std::thread::spawn(move || {
            if let Err(e) = Self::send(&summary, &body) {
                log::warn!("Failed to send desktop notification: {e:#}");
            }
        });
    }
}
