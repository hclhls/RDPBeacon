use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use anyhow::Context;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NOTIFYICONDATAW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE};
use windows::Win32::UI::WindowsAndMessaging::*;
use super::{ControlEvent, MenuModel, TrayHost, TrayView, LOGGING_ID, QUIT_ID};

const CALLBACK: u32 = WM_APP + 1;

struct WindowState {
    menu: MenuModel,
    icon: HICON,
    taskbar_created: u32,
}

struct WindowsHost {
    hwnd: HWND,
    state: Box<WindowState>,
    instance: HINSTANCE,
}

fn icon_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd, uID: 1, uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: CALLBACK, hIcon: icon, ..Default::default()
    };
    for (dest, value) in data.szTip.iter_mut().zip("RDPBeacon".encode_utf16()) { *dest = value; }
    data
}

unsafe fn register_icon(hwnd: HWND, icon: HICON) -> bool {
    Shell_NotifyIconW(NIM_ADD, &icon_data(hwnd, icon)).as_bool()
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState;
    if !ptr.is_null() {
        if message == (*ptr).taskbar_created {
            let icon = (*ptr).icon;
            (*ptr).menu.restore_icon(|| register_icon(hwnd, icon));
            return LRESULT(0);
        }
        if message == CALLBACK && matches!(lparam.0 as u32, WM_RBUTTONUP | WM_LBUTTONUP | WM_CONTEXTMENU) {
            // Copy the view before entering the nested native menu loop: callbacks
            // may reenter this window procedure while the menu is displayed.
            let view = (*ptr).menu.view.clone();
            match show_menu(hwnd, &view) {
                Ok(id) => (*ptr).menu.activate(id),
                Err(error) => (*ptr).menu.unavailable(&format!("Windows tray menu failed: {error:#}")),
            }
            return LRESULT(0);
        }
        if message == WM_CLOSE {
            (*ptr).menu.activate(QUIT_ID);
            return LRESULT(0);
        }
        if message == WM_NCDESTROY { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0); }
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

unsafe fn show_menu(hwnd: HWND, view: &TrayView) -> anyhow::Result<u32> {
    let menu = CreatePopupMenu()?;
    let result = (|| {
        let status: Vec<u16> = view.status.replace('&', "&&").encode_utf16().chain(Some(0)).collect();
        AppendMenuW(menu, MF_STRING | MF_DISABLED, 0, PCWSTR(status.as_ptr()))?;
        let mut flags = MF_STRING;
        if view.logging_enabled { flags |= MF_CHECKED; }
        if view.quitting { flags |= MF_DISABLED; }
        AppendMenuW(menu, flags, LOGGING_ID as usize, w!("Enable logging"))?;
        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(menu, MF_STRING, QUIT_ID as usize, w!("Quit"))?;
        let mut point = POINT::default();
        GetCursorPos(&mut point)?;
        let _ = SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x, point.y, 0, hwnd, None).0 as u32;
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        Ok(id)
    })();
    let _ = DestroyMenu(menu);
    result
}

pub fn create(events: Sender<ControlEvent>) -> anyhow::Result<Box<dyn TrayHost>> {
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let class = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: instance,
            lpszClassName: w!("RDPBeaconTray"), ..Default::default() };
        if RegisterClassW(&class) == 0 { anyhow::bail!("Register Windows tray class: {}", windows::core::Error::from_win32()); }
        let (mut pixels, width, height) = super::icon::icon_rgba();
        let mut mask = vec![0u8; (width * height / 8) as usize];
        for (i, pixel) in pixels.chunks_exact_mut(4).enumerate() {
            if pixel[3] == 0 { mask[i / 8] |= 0x80 >> (i % 8); }
            pixel.swap(0, 2);
        }
        let icon = match CreateIcon(instance, width as i32, height as i32, 1, 32, mask.as_ptr(), pixels.as_ptr()) {
            Ok(icon) => icon,
            Err(error) => { let _ = UnregisterClassW(w!("RDPBeaconTray"), instance); return Err(error.into()); }
        };
        let mut state = Box::new(WindowState { menu: MenuModel::new(events), icon,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")) });
        if state.taskbar_created == 0 {
            let _ = DestroyIcon(icon);
            let _ = UnregisterClassW(w!("RDPBeaconTray"), instance);
            anyhow::bail!("Register Explorer restart notification failed");
        }
        let hwnd = match CreateWindowExW(WINDOW_EX_STYLE::default(), w!("RDPBeaconTray"), w!("RDPBeacon"),
            WINDOW_STYLE::default(), 0, 0, 0, 0, None, None, instance,
            Some((&mut *state as *mut WindowState).cast())) {
            Ok(hwnd) => hwnd,
            Err(error) => { let _ = DestroyIcon(icon); let _ = UnregisterClassW(w!("RDPBeaconTray"), instance); return Err(error.into()); }
        };
        let host = WindowsHost { hwnd, state, instance };
        if !register_icon(hwnd, icon) { anyhow::bail!("Register Windows notification icon failed"); }
        Ok(Box::new(host))
    }
}

impl TrayHost for WindowsHost {
    fn update(&mut self, view: &TrayView) -> anyhow::Result<()> {
        self.state.menu.update(view);
        Ok(())
    }
    fn pump(&mut self, timeout: Duration) -> anyhow::Result<()> {
        let deadline = Instant::now() + timeout;
        unsafe {
            loop {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    if message.message == WM_QUIT { self.state.menu.activate(QUIT_ID); }
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() { break; }
                let result = MsgWaitForMultipleObjects(None, false, remaining.as_millis() as u32, QS_ALLINPUT);
                if result == windows::Win32::Foundation::WAIT_FAILED {
                    return Err(windows::core::Error::from_win32()).context("Pump Windows tray messages");
                }
            }
        }
        Ok(())
    }
}

impl Drop for WindowsHost {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(self.hwnd, self.state.icon));
            let _ = DestroyWindow(self.hwnd);
            let _ = DestroyIcon(self.state.icon);
            let _ = UnregisterClassW(w!("RDPBeaconTray"), self.instance);
        }
    }
}
