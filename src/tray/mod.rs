use crate::daemon::{AppState, ControlEvent, TrayView};
use std::sync::mpsc::Sender;
use std::time::Duration;

#[cfg(any(windows, test))]
mod callback;
mod icon;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

const LOGGING_ID: u32 = 1;
const QUIT_ID: u32 = 2;
const INTERVAL_BASE_ID: u32 = 100;
const REMINDER_BASE_ID: u32 = 200;

const INTERVAL_PRESETS: [u64; 5] = [60, 120, 240, 300, 600];
const REMINDER_PRESETS: [u64; 4] = [0, 10, 30, 60];

/// Returns the presets plus `current` (when it is not already a preset), sorted.
fn options_with_current(presets: &[u64], current: Duration) -> Vec<Duration> {
    let mut options: Vec<Duration> = presets.iter().copied().map(Duration::from_secs).collect();
    if !options.contains(&current) {
        options.push(current);
        options.sort();
    }
    options
}

pub(crate) fn interval_options(current: Duration) -> Vec<Duration> {
    options_with_current(&INTERVAL_PRESETS, current)
}

pub(crate) fn reminder_options(current: Duration) -> Vec<Duration> {
    options_with_current(&REMINDER_PRESETS, current)
}

/// Formats a duration for menu labels, e.g. `4m`, `30s`, `1m 30s`.
pub(crate) fn duration_label(duration: Duration) -> String {
    let total = duration.as_secs();
    match (total / 60, total % 60) {
        (0, secs) => format!("{secs}s"),
        (mins, 0) => format!("{mins}m"),
        (mins, secs) => format!("{mins}m {secs}s"),
    }
}

pub(crate) fn reminder_label(duration: Duration) -> String {
    if duration.is_zero() {
        "Off".into()
    } else {
        duration_label(duration)
    }
}

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
            id if !self.view.quitting && id >= REMINDER_BASE_ID => {
                match reminder_options(self.view.notify_before).get((id - REMINDER_BASE_ID) as usize)
                {
                    Some(value) => ControlEvent::SetNotifyBefore(*value),
                    None => return,
                }
            }
            id if !self.view.quitting && id >= INTERVAL_BASE_ID => {
                match interval_options(self.view.interval).get((id - INTERVAL_BASE_ID) as usize) {
                    Some(value) => ControlEvent::SetInterval(*value),
                    None => return,
                }
            }
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
            interval: Duration::from_secs(600),
            notify_before: Duration::ZERO,
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
    fn timing_menu_ids_map_to_preset_events() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut menu = MenuModel::new(tx);
        menu.activate(INTERVAL_BASE_ID);
        assert!(
            matches!(rx.recv().unwrap(), ControlEvent::SetInterval(d) if d == Duration::from_secs(60))
        );
        menu.activate(REMINDER_BASE_ID);
        assert!(
            matches!(rx.recv().unwrap(), ControlEvent::SetNotifyBefore(d) if d.is_zero())
        );
        menu.activate(REMINDER_BASE_ID + 2);
        assert!(
            matches!(rx.recv().unwrap(), ControlEvent::SetNotifyBefore(d) if d == Duration::from_secs(30))
        );
        menu.activate(INTERVAL_BASE_ID + 50);
        menu.activate(REMINDER_BASE_ID + 50);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn timing_menu_ignored_while_quitting() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut menu = MenuModel::new(tx);
        let mut view = menu.view.clone();
        view.quitting = true;
        menu.update(&view);
        menu.activate(INTERVAL_BASE_ID);
        menu.activate(REMINDER_BASE_ID);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn custom_config_value_is_added_to_options() {
        let custom = Duration::from_secs(90);
        let options = interval_options(custom);
        assert_eq!(options.len(), INTERVAL_PRESETS.len() + 1);
        assert!(options.contains(&custom));
        assert!(options.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(
            interval_options(Duration::from_secs(240)).len(),
            INTERVAL_PRESETS.len()
        );
    }

    #[test]
    fn labels_are_human_readable() {
        assert_eq!(duration_label(Duration::from_secs(240)), "4m");
        assert_eq!(duration_label(Duration::from_secs(30)), "30s");
        assert_eq!(duration_label(Duration::from_secs(90)), "1m 30s");
        assert_eq!(reminder_label(Duration::ZERO), "Off");
        assert_eq!(reminder_label(Duration::from_secs(10)), "10s");
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
