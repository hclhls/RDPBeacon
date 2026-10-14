use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::screensaver;
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
use x11rb::protocol::xproto::{
    self, Atom, AtomEnum, ClientMessageData, ClientMessageEvent, ConnectionExt, EventMask, Window,
};
use x11rb::protocol::xtest;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::backend::fade::{fade_alpha, FADE_STEP};
use crate::backend::{Backend, WindowId};
use crate::config::Mode;

#[allow(unused_imports)]
pub use crate::backend::title_matches;

pub fn opacity_cardinal(alpha: u8) -> u32 {
    (alpha as u32) * 0x0101_0101
}

pub fn keysym_from_str(key: &str) -> Option<u32> {
    let key_trimmed = key.trim();
    if let Some(hex) = key_trimmed
        .strip_prefix("0x")
        .or_else(|| key_trimmed.strip_prefix("0X"))
    {
        if let Ok(val) = u32::from_str_radix(hex, 16) {
            return Some(val);
        }
    }

    let upper = key_trimmed.to_ascii_uppercase();
    if upper.starts_with('F') {
        if let Ok(num) = upper[1..].parse::<u32>() {
            if (1..=24).contains(&num) {
                return Some(0xffbe + num - 1);
            }
        }
    }

    match upper.as_str() {
        "SHIFT" | "SHIFT_L" => Some(0xffe1),
        "SHIFT_R" => Some(0xffe2),
        "CTRL" | "CONTROL_L" => Some(0xffe3),
        "CONTROL_R" => Some(0xffe4),
        "ALT" | "ALT_L" => Some(0xffe9),
        "ALT_R" => Some(0xffea),
        "SPACE" => Some(0x0020),
        "RETURN" | "ENTER" => Some(0xff0d),
        "ESCAPE" | "ESC" => Some(0xff1b),
        "BACKSPACE" => Some(0xff08),
        "TAB" => Some(0xff09),
        _ => {
            if key_trimmed.chars().count() == 1 {
                let c = key_trimmed.chars().next().unwrap();
                if c.is_ascii() {
                    return Some(c as u32);
                }
            }
            None
        }
    }
}

pub struct X11Backend {
    conn: RustConnection,
    root: Window,
    width: u16,
    height: u16,
    depth: u8,
    visual: xproto::Visualid,
    black_pixel: u32,
    atom_net_client_list: Atom,
    atom_net_active_window: Atom,
    atom_net_wm_name: Atom,
    atom_net_wm_cm_s0: Atom,
    atom_net_wm_window_opacity: Atom,
    overlay: Mutex<Option<Window>>,
    compositor_warned: AtomicBool,
}

impl X11Backend {
    pub fn new() -> Result<Self> {
        let (conn, screen_num) =
            x11rb::connect(None).context("Failed to connect to X11 display server")?;
        let screen = conn
            .setup()
            .roots
            .get(screen_num)
            .ok_or_else(|| anyhow::anyhow!("No X11 screen for index {screen_num}"))?;
        let root = screen.root;
        let width = screen.width_in_pixels;
        let height = screen.height_in_pixels;
        let depth = screen.root_depth;
        let visual = screen.root_visual;
        let black_pixel = screen.black_pixel;

        let atom_net_client_list = conn.intern_atom(false, b"_NET_CLIENT_LIST")?.reply()?.atom;
        let atom_net_active_window = conn
            .intern_atom(false, b"_NET_ACTIVE_WINDOW")?
            .reply()?
            .atom;
        let atom_net_wm_name = conn.intern_atom(false, b"_NET_WM_NAME")?.reply()?.atom;
        let atom_net_wm_cm_s0 = conn.intern_atom(false, b"_NET_WM_CM_S0")?.reply()?.atom;
        let atom_net_wm_window_opacity = conn
            .intern_atom(false, b"_NET_WM_WINDOW_OPACITY")?
            .reply()?
            .atom;

        Ok(Self {
            conn,
            root,
            width,
            height,
            depth,
            visual,
            black_pixel,
            atom_net_client_list,
            atom_net_active_window,
            atom_net_wm_name,
            atom_net_wm_cm_s0,
            atom_net_wm_window_opacity,
            overlay: Mutex::new(None),
            compositor_warned: AtomicBool::new(false),
        })
    }

    fn get_window_title(&self, w: Window) -> Result<String> {
        let reply = self
            .conn
            .get_property(false, w, self.atom_net_wm_name, AtomEnum::ANY, 0, 1024)?
            .reply()?;
        if !reply.value.is_empty() {
            return Ok(String::from_utf8_lossy(&reply.value).to_string());
        }

        let reply = self
            .conn
            .get_property(false, w, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 1024)?
            .reply()?;
        if !reply.value.is_empty() {
            return Ok(String::from_utf8_lossy(&reply.value).to_string());
        }

        Ok(String::new())
    }

    fn get_window_class(&self, w: Window) -> Result<String> {
        let reply = self
            .conn
            .get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::ANY, 0, 1024)?
            .reply()?;
        if !reply.value.is_empty() {
            let s = reply
                .value
                .iter()
                .map(|&b| if b == 0 { b' ' } else { b })
                .collect::<Vec<u8>>();
            return Ok(String::from_utf8_lossy(&s).to_string());
        }
        Ok(String::new())
    }

    fn keysym_to_keycode(&self, keysym: u32) -> Result<u8> {
        let setup = self.conn.setup();
        let min = setup.min_keycode;
        let max = setup.max_keycode;
        let count = max.saturating_sub(min) + 1;
        let mapping = self.conn.get_keyboard_mapping(min, count)?.reply()?;
        let per_key = mapping.keysyms_per_keycode as usize;
        if per_key == 0 {
            anyhow::bail!("No keyboard mapping available");
        }

        let mut unused_keycode = None;
        for (i, chunk) in mapping.keysyms.chunks(per_key).enumerate() {
            let keycode = min + i as u8;
            if chunk.iter().any(|&ks| ks == keysym) {
                return Ok(keycode);
            }
            if unused_keycode.is_none() && chunk.iter().all(|&ks| ks == 0) {
                unused_keycode = Some(keycode);
            }
        }

        // On Linux / evdev keyboards, keycode 193 (evdev KEY_F15 185 + 8) is often mapped
        // to XF86Launch6 (0x1008ff46) rather than XK_F15 (0xffcc). If requesting F15 and keycode
        // 193 is valid, use keycode 193 directly to avoid altering user keymaps.
        if keysym == 0xffcc && (min..=max).contains(&193) {
            log::warn!(
                "Key 'F15' may not be forwarded by Horizon Client on Linux (unmapped in 104-key PC layouts). \
                 Consider using 'Shift_L' or mode = 'mouse'."
            );
            return Ok(193);
        }

        if let Some(keycode) = unused_keycode {
            let mut new_keysyms = vec![0; per_key];
            new_keysyms[0] = keysym;
            self.conn
                .change_keyboard_mapping(1, keycode, per_key as u8, &new_keysyms)?;
            self.conn.flush()?;
            return Ok(keycode);
        }

        anyhow::bail!("Keysym 0x{keysym:x} not mapped to any keycode and no free keycode found")
    }
}

impl Backend for X11Backend {
    fn idle_time(&self) -> Result<Duration> {
        let reply = screensaver::query_info(&self.conn, self.root)?.reply()?;
        Ok(Duration::from_millis(reply.ms_since_user_input as u64))
    }

    fn find_horizon_window(&self, matcher: &str) -> Result<Option<WindowId>> {
        let reply = self
            .conn
            .get_property(
                false,
                self.root,
                self.atom_net_client_list,
                AtomEnum::WINDOW,
                0,
                4096,
            )?
            .reply()?;

        let windows: Vec<u32> = reply
            .value32()
            .map(|iter| iter.collect())
            .unwrap_or_default();

        let mut best: Option<(i32, WindowId, String, String)> = None;

        for (idx, &w) in windows.iter().enumerate() {
            let title = self.get_window_title(w).unwrap_or_default();
            let class = self.get_window_class(w).unwrap_or_default();
            let score = crate::backend::score_window(&title, &class, matcher, idx);
            if let Some(score) = score {
                log::trace!(
                    "Window {w:#x} ({w}): title=\"{title}\", class=\"{class}\", score={score}"
                );
                if best
                    .as_ref()
                    .map_or(true, |(best_score, ..)| score > *best_score)
                {
                    best = Some((score, w as WindowId, title, class));
                }
            }
        }

        if let Some((score, id, title, class)) = &best {
            log::debug!("Selected Horizon window {id} (score {score}): \"{title}\" [{class}]");
        }

        Ok(best.map(|(_, id, ..)| id))
    }

    fn active_window(&self) -> Result<Option<WindowId>> {
        let reply = self
            .conn
            .get_property(
                false,
                self.root,
                self.atom_net_active_window,
                AtomEnum::WINDOW,
                0,
                1,
            )?
            .reply()?;

        let wid = reply.value32().and_then(|mut it| it.next()).unwrap_or(0);
        if wid == 0 {
            Ok(None)
        } else {
            Ok(Some(wid as WindowId))
        }
    }

    fn focus(&self, w: WindowId) -> Result<()> {
        let event = ClientMessageEvent {
            response_type: xproto::CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window: w as u32,
            type_: self.atom_net_active_window,
            data: ClientMessageData::from([
                2, // source indication: pager / other client
                x11rb::CURRENT_TIME,
                0,
                0,
                0,
            ]),
        };

        let event_mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
        self.conn.send_event(false, self.root, event_mask, event)?;
        self.conn.flush()?;

        // Wait up to 500 ms for window manager to confirm focus activation
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        while std::time::Instant::now() < deadline {
            if let Ok(Some(active)) = self.active_window() {
                if active == w {
                    return Ok(());
                }
            }
            if let Ok(cookie) = self.conn.get_input_focus() {
                if let Ok(reply) = cookie.reply() {
                    if reply.focus == w as u32 {
                        return Ok(());
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(15));
        }

        log::debug!("Window {w} did not confirm focus activation within timeout; continuing");
        Ok(())
    }

    fn send_beacon(&self, mode: Mode, key: &str) -> Result<()> {
        match mode {
            Mode::Key => {
                let keysym = keysym_from_str(key)
                    .ok_or_else(|| anyhow::anyhow!("Unknown or unsupported key: {key}"))?;
                let keycode = self.keysym_to_keycode(keysym)?;
                // Send two taps: the first pulse ensures window/input activation hooks settle,
                // and the second guarantees reception by the remote desktop session.
                for i in 0..2 {
                    if i > 0 {
                        std::thread::sleep(Duration::from_millis(30));
                    }
                    xtest::fake_input(
                        &self.conn,
                        xproto::KEY_PRESS_EVENT,
                        keycode,
                        x11rb::CURRENT_TIME,
                        0,
                        0,
                        0,
                        0,
                    )?;
                    self.conn.flush()?;
                    std::thread::sleep(Duration::from_millis(20));
                    xtest::fake_input(
                        &self.conn,
                        xproto::KEY_RELEASE_EVENT,
                        keycode,
                        x11rb::CURRENT_TIME,
                        0,
                        0,
                        0,
                        0,
                    )?;
                    self.conn.flush()?;
                }
            }
            Mode::Mouse => {
                xtest::fake_input(
                    &self.conn,
                    xproto::MOTION_NOTIFY_EVENT,
                    1, // detail = 1 means relative coordinates
                    x11rb::CURRENT_TIME,
                    0,
                    1, // root_x = +1
                    0,
                    0,
                )?;
                self.conn.flush()?;
                std::thread::sleep(std::time::Duration::from_millis(10));
                xtest::fake_input(
                    &self.conn,
                    xproto::MOTION_NOTIFY_EVENT,
                    1, // detail = 1 means relative coordinates
                    x11rb::CURRENT_TIME,
                    0,
                    -1, // root_x = -1
                    0,
                    0,
                )?;
                self.conn.flush()?;
            }
        }
        Ok(())
    }

    fn supports_focus(&self) -> bool {
        true
    }

    fn fade_out(&self, duration: Duration) -> Result<()> {
        if duration.is_zero() {
            return Ok(());
        }

        let owner = self
            .conn
            .get_selection_owner(self.atom_net_wm_cm_s0)?
            .reply()?
            .owner;
        if owner == x11rb::NONE {
            if !self.compositor_warned.swap(true, Ordering::Relaxed) {
                log::warn!("No compositing manager running (_NET_WM_CM_S0 unowned); skipping screen fade");
            }
            return Ok(());
        }

        // Clean up any stale overlay first
        if let Some(old) = self.overlay.lock().unwrap().take() {
            let _ = self.conn.destroy_window(old);
            let _ = self.conn.flush();
        }

        // Query the current root size so resolution changes after startup are respected.
        let (width, height) = match self
            .conn
            .get_geometry(self.root)
            .map_err(anyhow::Error::from)
            .and_then(|c| c.reply().map_err(anyhow::Error::from))
        {
            Ok(g) => (g.width, g.height),
            Err(e) => {
                log::debug!("Failed to query root geometry, using cached size: {e}");
                (self.width, self.height)
            }
        };

        let wid = self.conn.generate_id()?;
        let values = xproto::CreateWindowAux::new()
            .override_redirect(1)
            .background_pixel(self.black_pixel)
            .event_mask(EventMask::NO_EVENT);

        let create_res = self.conn.create_window(
            self.depth,
            wid,
            self.root,
            0,
            0,
            width,
            height,
            0,
            xproto::WindowClass::INPUT_OUTPUT,
            self.visual,
            &values,
        );
        if let Err(e) = create_res {
            return Err(e.into());
        }

        // Set empty input shape so user input passes through
        if let Err(e) = self.conn.shape_rectangles(
            SO::SET,
            SK::INPUT,
            xproto::ClipOrdering::UNSORTED,
            wid,
            0,
            0,
            &[],
        ) {
            let _ = self.conn.destroy_window(wid);
            let _ = self.conn.flush();
            return Err(e.into());
        }

        // Set initial opacity to 0
        if let Err(e) = self.conn.change_property32(
            xproto::PropMode::REPLACE,
            wid,
            self.atom_net_wm_window_opacity,
            AtomEnum::CARDINAL,
            &[opacity_cardinal(0)],
        ) {
            let _ = self.conn.destroy_window(wid);
            let _ = self.conn.flush();
            return Err(e.into());
        }

        // Check the request synchronously so asynchronous X errors are not silently dropped.
        if let Err(e) = self
            .conn
            .map_window(wid)
            .map_err(anyhow::Error::from)
            .and_then(|c| c.check().map_err(anyhow::Error::from))
        {
            let _ = self.conn.destroy_window(wid);
            let _ = self.conn.flush();
            return Err(e);
        }

        // Save overlay so Drop or fade_in will clean it up
        *self.overlay.lock().unwrap() = Some(wid);

        let start = std::time::Instant::now();
        loop {
            let elapsed = start.elapsed();
            let alpha = fade_alpha(elapsed, duration);
            let card = opacity_cardinal(alpha);
            if let Err(e) = self.conn.change_property32(
                xproto::PropMode::REPLACE,
                wid,
                self.atom_net_wm_window_opacity,
                AtomEnum::CARDINAL,
                &[card],
            ) {
                let _ = self.conn.destroy_window(wid);
                let _ = self.conn.flush();
                *self.overlay.lock().unwrap() = None;
                return Err(e.into());
            }
            let _ = self.conn.flush();
            if elapsed >= duration {
                break;
            }
            std::thread::sleep(FADE_STEP);
        }

        Ok(())
    }

    fn fade_in(&self, duration: Duration) -> Result<()> {
        let wid = match self.overlay.lock().unwrap().take() {
            Some(w) => w,
            None => return Ok(()),
        };

        if duration.is_zero() {
            let _ = self.conn.destroy_window(wid);
            let _ = self.conn.flush();
            return Ok(());
        }

        let start = std::time::Instant::now();
        loop {
            let elapsed = start.elapsed();
            let alpha = 255 - fade_alpha(elapsed, duration);
            let card = opacity_cardinal(alpha);
            let _ = self.conn.change_property32(
                xproto::PropMode::REPLACE,
                wid,
                self.atom_net_wm_window_opacity,
                AtomEnum::CARDINAL,
                &[card],
            );
            let _ = self.conn.flush();
            if elapsed >= duration {
                break;
            }
            std::thread::sleep(FADE_STEP);
        }

        let _ = self.conn.destroy_window(wid);
        let _ = self.conn.flush();
        Ok(())
    }
}

impl Drop for X11Backend {
    fn drop(&mut self) {
        if let Ok(mut lock) = self.overlay.lock() {
            if let Some(wid) = lock.take() {
                let _ = self.conn.destroy_window(wid);
                let _ = self.conn.flush();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_title_case_insensitive() {
        assert!(title_matches("VMware Horizon Client", "other", "horizon"));
        assert!(title_matches("vmware HORIZON client", "", "Horizon"));
    }

    #[test]
    fn matches_wm_class() {
        assert!(title_matches("Untitled Document", "vmware-view", "view"));
        assert!(title_matches("Window", "VMware-View", "VIEW"));
    }

    #[test]
    fn no_match_returns_false() {
        assert!(!title_matches("Google Chrome", "google-chrome", "horizon"));
        assert!(!title_matches("", "", "view"));
    }

    #[test]
    fn maps_keysyms_correctly() {
        assert_eq!(keysym_from_str("F15"), Some(0xffcc));
        assert_eq!(keysym_from_str("f15"), Some(0xffcc));
        assert_eq!(keysym_from_str("F1"), Some(0xffbe));
        assert_eq!(keysym_from_str("0xffcc"), Some(0xffcc));
        assert_eq!(keysym_from_str("space"), Some(0x0020));
        assert_eq!(keysym_from_str("nonexistentkey123"), None);
    }

    #[test]
    fn x11_backend_real_if_display() {
        if std::env::var("DISPLAY").is_ok() {
            if let Ok(backend) = X11Backend::new() {
                assert!(backend.supports_focus());
                assert!(backend.idle_time().is_ok());
                assert!(backend.active_window().is_ok());
                assert_eq!(
                    backend
                        .find_horizon_window("this_is_an_unlikely_match_string_xyz")
                        .unwrap(),
                    None
                );
            }
        }
    }

    #[test]
    fn opacity_value_maps_alpha_to_cardinal() {
        assert_eq!(opacity_cardinal(0), 0);
        assert_eq!(opacity_cardinal(255), 0xffff_ffff);
        let mut prev = 0;
        for a in 1..=255 {
            let curr = opacity_cardinal(a);
            assert!(curr > prev, "expected monotonic increase at {a}");
            prev = curr;
        }
    }
}
