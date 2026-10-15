use super::callback::CallbackState;
use super::{
    ControlEvent, MenuModel, TrayHost, TrayView, FADE_ID, INTERVAL_BASE_ID, LOGGING_ID, QUIT_ID,
    REMINDER_BASE_ID,
};
use anyhow::Context;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const CALLBACK: u32 = WM_APP + 1;
const UPDATE: u32 = WM_APP + 2;
const SHUTDOWN: u32 = WM_APP + 3;

struct WindowState {
    menu: MenuModel,
    icon: HICON,
    taskbar_created: u32,
    updates: Receiver<TrayView>,
    shutdown: bool,
}

struct NativeHost {
    hwnd: HWND,
    state: CallbackState<WindowState>,
    instance: HINSTANCE,
}

fn icon_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: CALLBACK,
        hIcon: icon,
        ..Default::default()
    };
    for (dest, value) in data.szTip.iter_mut().zip("RDPBeacon".encode_utf16()) {
        *dest = value;
    }
    data
}

unsafe fn register_icon(hwnd: HWND, icon: HICON) -> bool {
    Shell_NotifyIconW(NIM_ADD, &icon_data(hwnd, icon)).as_bool()
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState;
    if !ptr.is_null() {
        if message == UPDATE {
            while let Ok(view) = (*ptr).updates.try_recv() {
                (*ptr).menu.update(&view);
            }
            return LRESULT(0);
        }
        if message == SHUTDOWN {
            (*ptr).shutdown = true;
            let _ = EndMenu();
            return LRESULT(0);
        }
        if message == (*ptr).taskbar_created {
            let icon = (*ptr).icon;
            let restored = register_icon(hwnd, icon);
            (*ptr).menu.restore_icon(|| restored);
            return LRESULT(0);
        }
        if message == CALLBACK
            && matches!(
                lparam.0 as u32,
                WM_RBUTTONUP | WM_LBUTTONUP | WM_CONTEXTMENU
            )
        {
            // Copy the view before entering the nested native menu loop: callbacks
            // may reenter this window procedure while the menu is displayed.
            let view = (*ptr).menu.view.clone();
            match show_menu(hwnd, &view) {
                Ok(id) => (*ptr).menu.activate(id),
                Err(error) => (*ptr)
                    .menu
                    .unavailable(&format!("Windows tray menu failed: {error:#}")),
            }
            return LRESULT(0);
        }
        if message == WM_CLOSE {
            (*ptr).menu.activate(QUIT_ID);
            let _ = EndMenu();
            return LRESULT(0);
        }
        if message == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        }
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

fn wide(text: &str) -> Vec<u16> {
    text.replace('&', "&&")
        .encode_utf16()
        .chain(Some(0))
        .collect()
}

/// Appends a popup submenu whose items use ids `base_id + index`. The submenu is
/// owned by `menu` and destroyed with it.
unsafe fn append_radio_submenu(
    menu: HMENU,
    title: &str,
    base_id: u32,
    items: impl Iterator<Item = (String, bool)>,
    enabled: bool,
) -> anyhow::Result<()> {
    let submenu = CreatePopupMenu()?;
    let result = (|| {
        for (index, (label, checked)) in items.enumerate() {
            let mut flags = MF_STRING;
            if checked {
                flags |= MF_CHECKED;
            }
            let label = wide(&label);
            AppendMenuW(
                submenu,
                flags,
                (base_id + index as u32) as usize,
                PCWSTR(label.as_ptr()),
            )?;
        }
        let mut flags = MF_POPUP;
        if !enabled {
            flags |= MF_DISABLED;
        }
        let title = wide(title);
        AppendMenuW(menu, flags, submenu.0 as usize, PCWSTR(title.as_ptr()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = DestroyMenu(submenu);
    }
    result
}

unsafe fn show_menu(hwnd: HWND, view: &TrayView) -> anyhow::Result<u32> {
    let menu = CreatePopupMenu()?;
    let result = (|| {
        let status: Vec<u16> = view
            .status
            .replace('&', "&&")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        AppendMenuW(menu, MF_STRING | MF_DISABLED, 0, PCWSTR(status.as_ptr()))?;
        let mut flags = MF_STRING;
        if view.logging_enabled {
            flags |= MF_CHECKED;
        }
        if view.quitting {
            flags |= MF_DISABLED;
        }
        AppendMenuW(menu, flags, LOGGING_ID as usize, w!("Enable logging"))?;
        let interval_options = super::interval_options(view.interval);
        append_radio_submenu(
            menu,
            "Beacon interval",
            INTERVAL_BASE_ID,
            interval_options
                .iter()
                .map(|d| (super::duration_label(*d), *d == view.interval)),
            !view.quitting,
        )?;
        let reminder_options = super::reminder_options(view.notify_before);
        append_radio_submenu(
            menu,
            "Reminder before beacon",
            REMINDER_BASE_ID,
            reminder_options
                .iter()
                .map(|d| (super::reminder_label(*d), *d == view.notify_before)),
            !view.quitting,
        )?;
        let mut fade_flags = MF_STRING;
        if view.fade_enabled {
            fade_flags |= MF_CHECKED;
        }
        if view.quitting {
            fade_flags |= MF_DISABLED;
        }
        AppendMenuW(menu, fade_flags, FADE_ID as usize, w!("Enable fade effect"))?;
        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(menu, MF_STRING, QUIT_ID as usize, w!("Quit"))?;
        let mut point = POINT::default();
        GetCursorPos(&mut point)?;
        let _ = SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            None,
        )
        .0 as u32;
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        Ok(id)
    })();
    let _ = DestroyMenu(menu);
    result
}

fn create_native(
    events: Sender<ControlEvent>,
    updates: Receiver<TrayView>,
) -> anyhow::Result<NativeHost> {
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: w!("RDPBeaconTray"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            anyhow::bail!(
                "Register Windows tray class: {}",
                windows::core::Error::from_win32()
            );
        }
        let (mut pixels, width, height) = super::icon::icon_rgba();
        let mut mask = vec![0u8; (width * height / 8) as usize];
        for (i, pixel) in pixels.chunks_exact_mut(4).enumerate() {
            if pixel[3] == 0 {
                mask[i / 8] |= 0x80 >> (i % 8);
            }
            pixel.swap(0, 2);
        }
        let icon = match CreateIcon(
            instance,
            width as i32,
            height as i32,
            1,
            32,
            mask.as_ptr(),
            pixels.as_ptr(),
        ) {
            Ok(icon) => icon,
            Err(error) => {
                let _ = UnregisterClassW(w!("RDPBeaconTray"), instance);
                return Err(error.into());
            }
        };
        let state = CallbackState::new(WindowState {
            menu: MenuModel::new(events),
            icon,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            updates,
            shutdown: false,
        });
        if (*state.as_ptr()).taskbar_created == 0 {
            let _ = DestroyIcon(icon);
            let _ = UnregisterClassW(w!("RDPBeaconTray"), instance);
            anyhow::bail!("Register Explorer restart notification failed");
        }
        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("RDPBeaconTray"),
            w!("RDPBeacon"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            instance,
            Some(state.as_ptr().cast()),
        ) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                let _ = DestroyIcon(icon);
                let _ = UnregisterClassW(w!("RDPBeaconTray"), instance);
                return Err(error.into());
            }
        };
        let host = NativeHost {
            hwnd,
            state,
            instance,
        };
        if !register_icon(hwnd, icon) {
            anyhow::bail!("Register Windows notification icon failed");
        }
        Ok(host)
    }
}

struct WindowsHost {
    hwnd: isize,
    updates: Sender<TrayView>,
    ui: Option<JoinHandle<()>>,
}

pub fn create(events: Sender<ControlEvent>) -> anyhow::Result<Box<dyn TrayHost>> {
    Ok(Box::new(WindowsHost::new(events)?))
}

impl WindowsHost {
    fn new(events: Sender<ControlEvent>) -> anyhow::Result<Self> {
        let (updates, incoming) = mpsc::channel();
        let (ready, startup) = mpsc::channel();
        let ui = std::thread::Builder::new()
            .name("rdpbeacon-tray-ui".into())
            .spawn(move || {
                let result = (|| -> anyhow::Result<()> {
                    let host = create_native(events.clone(), incoming)?;
                    ready.send(Ok(host.hwnd.0 as isize))?;
                    // Native menus run a nested modal loop on this thread. The
                    // coordinator and beacon worker remain responsive independently.
                    unsafe {
                        loop {
                            let mut message = MSG::default();
                            let result = GetMessageW(&mut message, None, 0, 0).0;
                            if result == -1 {
                                return Err(windows::core::Error::from_win32())
                                    .context("Windows tray message loop");
                            }
                            if result == 0 {
                                break;
                            }
                            let _ = TranslateMessage(&message);
                            DispatchMessageW(&message);
                            // A modal menu can consume WM_QUIT. Keep shutdown state
                            // outside the message queue and check after it unwinds.
                            if (*host.state.as_ptr()).shutdown {
                                break;
                            }
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    let reason = format!("{error:#}");
                    let _ = ready.send(Err(reason.clone()));
                    let _ = events.send(ControlEvent::TrayUnavailable(reason));
                }
            })
            .context("Start Windows tray thread")?;
        match startup.recv() {
            Ok(Ok(hwnd)) => Ok(Self {
                hwnd,
                updates,
                ui: Some(ui),
            }),
            result => {
                let _ = ui.join();
                let reason = match result {
                    Ok(Err(reason)) => reason,
                    _ => "Windows tray thread exited before readiness".into(),
                };
                anyhow::bail!("{reason}");
            }
        }
    }
}

impl TrayHost for WindowsHost {
    fn update(&mut self, view: &TrayView) -> anyhow::Result<()> {
        self.updates
            .send(view.clone())
            .context("Update Windows tray state")?;
        unsafe { PostMessageW(HWND(self.hwnd as *mut _), UPDATE, WPARAM(0), LPARAM(0)) }
            .context("Wake Windows tray for state update")
    }
    fn pump(&mut self, timeout: Duration) -> anyhow::Result<()> {
        if self.ui.as_ref().is_some_and(|thread| thread.is_finished()) {
            anyhow::bail!("Windows tray thread stopped");
        }
        std::thread::sleep(timeout);
        Ok(())
    }
}

impl Drop for WindowsHost {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(HWND(self.hwnd as *mut _), SHUTDOWN, WPARAM(0), LPARAM(0));
        }
        if let Some(ui) = self.ui.take() {
            let _ = ui.join();
        }
    }
}

impl Drop for NativeHost {
    fn drop(&mut self) {
        unsafe {
            let icon = (*self.state.as_ptr()).icon;
            let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(self.hwnd, icon));
            let _ = DestroyWindow(self.hwnd);
            let _ = DestroyIcon(icon);
            let _ = UnregisterClassW(w!("RDPBeaconTray"), self.instance);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn open_menu(host: &WindowsHost) {
        let hwnd = HWND(host.hwnd as *mut _);
        unsafe {
            PostMessageW(hwnd, CALLBACK, WPARAM(1), LPARAM(WM_RBUTTONUP as isize)).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut entered = false;
        while Instant::now() < deadline {
            unsafe {
                let id = GetWindowThreadProcessId(hwnd, None);
                let mut info = GUITHREADINFO {
                    cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                    ..Default::default()
                };
                GetGUIThreadInfo(id, &mut info).unwrap();
                if info.flags.contains(GUI_INMENUMODE) {
                    entered = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(entered, "Native menu never opened");
    }

    #[test]
    #[ignore = "Requires an interactive Windows desktop or Wine tray"]
    fn menu_open_does_not_block_coordinator_and_close_ends_menu() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut host = WindowsHost::new(tx).unwrap();
        open_menu(&host);
        let hwnd = HWND(host.hwnd as *mut _);
        let start = Instant::now();
        host.pump(Duration::from_millis(20)).unwrap();
        assert!(start.elapsed() < Duration::from_millis(250));
        unsafe {
            PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
        }
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            ControlEvent::Quit
        ));
        drop(host);

        // Host loss may destroy the adapter while the native menu is still open.
        let (tx, _rx) = std::sync::mpsc::channel();
        let host = WindowsHost::new(tx).unwrap();
        open_menu(&host);
        drop(host);
    }
}
