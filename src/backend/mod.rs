use std::time::Duration;
use anyhow::Result;

use crate::config::{BackendKind, Mode};

#[cfg(target_os = "linux")]
pub mod x11;
#[cfg(target_os = "linux")]
pub mod wayland;
#[cfg(any(windows, test))]
pub mod windows;

pub type WindowId = u64;

pub fn title_matches(title: &str, class: &str, matcher: &str) -> bool {
    if matcher.is_empty() {
        return false;
    }
    let matcher_lower = matcher.to_lowercase();
    let title_lower = title.to_lowercase();
    let class_lower = class.to_lowercase();

    if title_lower.contains(&matcher_lower) || class_lower.contains(&matcher_lower) {
        return true;
    }

    // If the matcher is targeting Horizon / VMware View (e.g., "Omnissa Horizon Client", "Horizon", "VMware View"),
    // also match associated remote session windows whose class is HorizonUnityWindow/UnityWindow/MKSWindow.
    if (matcher_lower.contains("horizon") || matcher_lower.contains("vmware") || matcher_lower.contains("view"))
        && is_session_window(title, class)
    {
        return true;
    }

    false
}

pub fn is_toolbar_window(title: &str) -> bool {
    title.trim().eq_ignore_ascii_case("BBar")
}

pub fn is_session_window(title: &str, class: &str) -> bool {
    let c = class.to_lowercase();
    let t = title.to_lowercase();

    // Known session window classes for Omnissa / VMware Horizon
    if c.contains("unitywindow") || c.contains("mkswindow") {
        return true;
    }

    // Typical session window titles:
    // e.g. "pc-95035 - 遠端桌面連線", "Desk-1 - Remote Desktop Connection",
    // or "<pool> - Omnissa Horizon Client" / "<pool> - VMware Horizon Client"
    if t.contains("遠端桌面連線") || t.contains("remote desktop") {
        return true;
    }

    if t.contains(" - omnissa horizon client")
        || t.contains(" - vmware horizon client")
        || t.contains(" - vmware view client")
    {
        return true;
    }

    false
}

pub fn is_launcher_window(title: &str, class: &str) -> bool {
    let t = title.trim();
    let c = class.to_lowercase();

    let is_generic_title = t.eq_ignore_ascii_case("Omnissa Horizon Client")
        || t.eq_ignore_ascii_case("VMware Horizon Client")
        || t.eq_ignore_ascii_case("VMware View Client")
        || t.eq_ignore_ascii_case("VMware View")
        || t.eq_ignore_ascii_case("Horizon Client");

    let is_generic_class = c.contains("horizon-client")
        || c.contains("vmware-view")
        || c.contains("wswc_ui");

    (is_generic_title || is_generic_class) && !is_session_window(title, class)
}

pub fn score_window(title: &str, class: &str, matcher: &str, index: usize) -> Option<i32> {
    if !title_matches(title, class, matcher) {
        return None;
    }

    // Toolbars/auxiliary drop-downs like BBar get lowest priority
    if is_toolbar_window(title) {
        return Some(-100 + index as i32);
    }

    // Actual remote session windows get highest priority
    if is_session_window(title, class) {
        return Some(200 + index as i32);
    }

    // Generic launcher/selector windows get lower priority
    if is_launcher_window(title, class) {
        return Some(10 + index as i32);
    }

    // Any other matching window
    Some(50 + index as i32)
}

pub trait Backend {
    fn idle_time(&self) -> Result<Duration>;
    fn find_horizon_window(&self, matcher: &str) -> Result<Option<WindowId>>;
    fn active_window(&self) -> Result<Option<WindowId>>;
    fn focus(&self, w: WindowId) -> Result<()>;
    fn send_beacon(&self, mode: Mode, key: &str) -> Result<()>;
    fn supports_focus(&self) -> bool;
}

pub fn detect_backend_kind(
    session_type: Option<&str>,
    wayland_display: Option<&str>,
    is_windows: bool,
) -> Result<BackendKind> {
    if is_windows {
        return Ok(BackendKind::Windows);
    }

    let is_wayland_session = session_type.map_or(false, |s| s.trim().eq_ignore_ascii_case("wayland"));
    let is_x11_session = session_type.map_or(false, |s| s.trim().eq_ignore_ascii_case("x11"));
    let has_wayland_display = wayland_display.map_or(false, |s| !s.trim().is_empty());

    let is_wayland = is_wayland_session || has_wayland_display;
    let is_x11 = is_x11_session;

    if is_wayland && is_x11 {
        anyhow::bail!(
            "Ambiguous backend detection: session is X11 but WAYLAND_DISPLAY is present. Please specify the backend explicitly with --backend"
        );
    }

    if is_wayland {
        return Ok(BackendKind::Wayland);
    }

    if is_x11 {
        return Ok(BackendKind::X11);
    }

    anyhow::bail!(
        "Could not detect display backend from environment (XDG_SESSION_TYPE / WAYLAND_DISPLAY). Please specify the backend explicitly with --backend"
    );
}

pub fn make_backend(kind: BackendKind) -> Result<Box<dyn Backend>> {
    match kind {
        #[cfg(target_os = "linux")]
        BackendKind::X11 => Ok(Box::new(x11::X11Backend::new()?)),
        #[cfg(target_os = "linux")]
        BackendKind::Wayland => Ok(Box::new(wayland::WaylandBackend::new()?)),
        #[cfg(windows)]
        BackendKind::Windows => Ok(Box::new(windows::WindowsBackend::new()?)),
        #[cfg(not(windows))]
        BackendKind::Windows => anyhow::bail!("Windows backend is only available on Windows"),
        #[allow(unreachable_patterns)]
        _ => anyhow::bail!("{kind:?} backend not yet implemented"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_x11() {
        let kind = detect_backend_kind(Some("x11"), None, false).unwrap();
        assert_eq!(kind, BackendKind::X11);
    }

    #[test]
    fn detects_wayland_via_session_type() {
        let kind = detect_backend_kind(Some("wayland"), None, false).unwrap();
        assert_eq!(kind, BackendKind::Wayland);
    }

    #[test]
    fn detects_wayland_via_display_var() {
        let kind = detect_backend_kind(None, Some("wayland-0"), false).unwrap();
        assert_eq!(kind, BackendKind::Wayland);
    }

    #[test]
    fn ambiguous_returns_error_mentioning_backend_flag() {
        let err1 = detect_backend_kind(None, None, false).unwrap_err();
        assert!(err1.to_string().contains("--backend"));

        let err2 = detect_backend_kind(Some("unknown"), None, false).unwrap_err();
        assert!(err2.to_string().contains("--backend"));

        let err3 = detect_backend_kind(Some("x11"), Some("wayland-0"), false).unwrap_err();
        assert!(err3.to_string().contains("--backend"));
    }

    #[test]
    fn windows_wins_on_windows() {
        let kind1 = detect_backend_kind(None, None, true).unwrap();
        assert_eq!(kind1, BackendKind::Windows);

        let kind2 = detect_backend_kind(Some("x11"), Some("wayland-0"), true).unwrap();
        assert_eq!(kind2, BackendKind::Windows);

        let kind3 = detect_backend_kind(Some("wayland"), None, true).unwrap();
        assert_eq!(kind3, BackendKind::Windows);
    }

    #[test]
    #[cfg(not(windows))]
    fn windows_backend_not_available_on_non_windows() {
        match make_backend(BackendKind::Windows) {
            Ok(_) => panic!("Expected error on non-Windows"),
            Err(err) => {
                assert!(err.to_string().contains("Windows backend is only available on Windows"));
            }
        }
    }

    #[test]
    fn identifies_session_and_launcher_types() {
        assert!(is_session_window(
            "pc-95035 - 遠端桌面連線",
            "遠端桌面連線.HorizonUnityWindow 遠端桌面連線"
        ));
        assert!(is_session_window(
            "Windows 11 VM - Omnissa Horizon Client",
            "vmware-view.HorizonUnityWindow"
        ));
        assert!(!is_session_window(
            "Omnissa Horizon Client",
            "horizon-client Horizon-client"
        ));
        assert!(is_launcher_window(
            "Omnissa Horizon Client",
            "horizon-client Horizon-client"
        ));
        assert!(is_toolbar_window("BBar"));
    }

    #[test]
    fn prioritizes_session_over_launcher() {
        let launcher_score = score_window(
            "Omnissa Horizon Client",
            "horizon-client Horizon-client",
            "horizon",
            0,
        )
        .unwrap();

        let session_score = score_window(
            "pc-95035 - 遠端桌面連線",
            "遠端桌面連線.HorizonUnityWindow 遠端桌面連線",
            "horizon",
            1,
        )
        .unwrap();

        let toolbar_score = score_window("BBar", "遠端桌面連線.HorizonUnityWindow", "horizon", 2).unwrap();

        assert!(
            session_score > launcher_score,
            "Session score {session_score} must be greater than launcher score {launcher_score}"
        );
        assert!(
            launcher_score > toolbar_score,
            "Launcher score {launcher_score} must be greater than toolbar score {toolbar_score}"
        );
    }
}

