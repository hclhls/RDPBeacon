use crate::daemon::{AppState, ControlEvent, TrayView};
use std::sync::mpsc::Sender;
use std::time::Duration;

mod icon;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

const LOGGING_ID: u32 = 1;
const QUIT_ID: u32 = 2;

struct MenuModel {
    view: TrayView,
    events: Sender<ControlEvent>,
    #[cfg(any(windows, test))]
    unavailable: bool,
}

impl MenuModel {
    fn new(events: Sender<ControlEvent>) -> Self {
        Self {
            view: AppState::default().view(),
            events,
            #[cfg(any(windows, test))]
            unavailable: false,
        }
    }
    fn update(&mut self, view: &TrayView) {
        self.view = view.clone();
    }
    fn activate(&mut self, id: u32) {
        let event = match id {
            LOGGING_ID if !self.view.quitting => {
                ControlEvent::SetLogging(!self.view.logging_enabled)
            }
            QUIT_ID => ControlEvent::Quit,
            _ => return,
        };
        let _ = self.events.send(event);
    }
    #[cfg(any(windows, test))]
    fn restore_icon(&mut self, register: impl FnOnce() -> bool) {
        if !register() {
            self.unavailable("Could not restore Windows tray after Explorer restart");
        }
    }
    #[cfg(any(windows, test))]
    fn unavailable(&mut self, reason: &str) {
        if !self.unavailable {
            self.unavailable = true;
            let _ = self
                .events
                .send(ControlEvent::TrayUnavailable(reason.into()));
        }
    }
}

pub trait TrayHost {
    fn update(&mut self, view: &TrayView) -> anyhow::Result<()>;
    fn pump(&mut self, timeout: Duration) -> anyhow::Result<()>;
}

pub fn create(events: Sender<ControlEvent>) -> anyhow::Result<Box<dyn TrayHost>> {
    #[cfg(target_os = "linux")]
    return linux::create(events);
    #[cfg(windows)]
    return windows::create(events);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explorer_restart_reregisters_or_reports_failure() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut menu = MenuModel::new(tx);
        let mut calls = 0;
        menu.restore_icon(|| {
            calls += 1;
            true
        });
        assert_eq!(calls, 1);
        assert!(rx.try_recv().is_err());
        menu.restore_icon(|| false);
        assert!(matches!(
            rx.recv().unwrap(),
            ControlEvent::TrayUnavailable(_)
        ));
    }
    #[test]
    fn menu_events_use_confirmed_logging_state() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut menu = MenuModel::new(tx);
        menu.activate(LOGGING_ID);
        assert!(matches!(rx.recv().unwrap(), ControlEvent::SetLogging(true)));
        assert!(!menu.view.logging_enabled);
        menu.update(&TrayView {
            status: "Running".into(),
            logging_enabled: true,
            quitting: false,
        });
        menu.activate(LOGGING_ID);
        assert!(matches!(
            rx.recv().unwrap(),
            ControlEvent::SetLogging(false)
        ));
        menu.activate(QUIT_ID);
        assert!(matches!(rx.recv().unwrap(), ControlEvent::Quit));
        menu.activate(99);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn host_failure_is_reported_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut menu = MenuModel::new(tx);
        menu.unavailable("host lost");
        menu.unavailable("host lost again");
        assert!(
            matches!(rx.recv().unwrap(), ControlEvent::TrayUnavailable(reason) if reason == "host lost")
        );
        assert!(rx.try_recv().is_err());
    }
}
