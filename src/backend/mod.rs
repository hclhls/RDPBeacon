use std::time::Duration;
use anyhow::Result;

use crate::config::{BackendKind, Mode};

pub type WindowId = u64;

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
    anyhow::bail!("{kind:?} backend not yet implemented")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BackendKind;

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
}

