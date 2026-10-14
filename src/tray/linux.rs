use super::{
    ControlEvent, MenuModel, TrayHost, TrayView, INTERVAL_BASE_ID, LOGGING_ID, QUIT_ID,
    REMINDER_BASE_ID,
};
use anyhow::Context;
use ksni::blocking::TrayMethods;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
    Arc,
};
use std::thread::JoinHandle;
use std::time::Duration;

struct LinuxTray {
    menu: MenuModel,
}

impl ksni::Tray for LinuxTray {
    fn id(&self) -> String {
        "rdpbeacon".into()
    }
    fn title(&self) -> String {
        "RDPBeacon".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let (mut pixels, width, height) = super::icon::icon_rgba();
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.rotate_right(1);
        }
        vec![ksni::Icon {
            width: width as i32,
            height: height as i32,
            data: pixels,
        }]
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu};
        let interval_options = super::interval_options(self.menu.view.interval);
        let reminder_options = super::reminder_options(self.menu.view.notify_before);
        let enabled = !self.menu.view.quitting;
        vec![
            StandardItem {
                label: self.menu.view.status.clone(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Enable logging".into(),
                checked: self.menu.view.logging_enabled,
                enabled: !self.menu.view.quitting,
                activate: Box::new(|tray: &mut Self| tray.menu.activate(LOGGING_ID)),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Beacon interval".into(),
                enabled,
                submenu: vec![RadioGroup {
                    selected: interval_options
                        .iter()
                        .position(|d| *d == self.menu.view.interval)
                        .unwrap_or(0),
                    select: Box::new(|tray: &mut Self, index| {
                        tray.menu.activate(INTERVAL_BASE_ID + index as u32)
                    }),
                    options: interval_options
                        .iter()
                        .map(|d| RadioItem {
                            label: super::duration_label(*d),
                            ..Default::default()
                        })
                        .collect(),
                }
                .into()],
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Reminder before beacon".into(),
                enabled,
                submenu: vec![RadioGroup {
                    selected: reminder_options
                        .iter()
                        .position(|d| *d == self.menu.view.notify_before)
                        .unwrap_or(0),
                    select: Box::new(|tray: &mut Self, index| {
                        tray.menu.activate(REMINDER_BASE_ID + index as u32)
                    }),
                    options: reminder_options
                        .iter()
                        .map(|d| RadioItem {
                            label: super::reminder_label(*d),
                            ..Default::default()
                        })
                        .collect(),
                }
                .into()],
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| tray.menu.activate(QUIT_ID)),
                ..Default::default()
            }
            .into(),
        ]
    }
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        let _ = self.menu.events.send(ControlEvent::TrayUnavailable(format!(
            "Linux tray host unavailable: {reason:?}"
        )));
        false
    }
}

struct LinuxHost {
    handle: ksni::blocking::Handle<LinuxTray>,
    events: Sender<ControlEvent>,
    closed_reported: bool,
    monitor_stop: Arc<AtomicBool>,
    monitor: Option<JoinHandle<()>>,
}

pub fn create(events: Sender<ControlEvent>) -> anyhow::Result<Box<dyn TrayHost>> {
    let handle = LinuxTray { menu: MenuModel::new(events.clone()) }.spawn()
        .context("Register Linux tray: enable a StatusNotifierItem tray host (GNOME requires the AppIndicator and KStatusNotifierItem Support extension)")?;
    let monitor_stop = Arc::new(AtomicBool::new(false));
    let monitor = match start_host_monitor(events.clone(), monitor_stop.clone()) {
        Ok(monitor) => monitor,
        Err(error) => {
            handle.shutdown().wait();
            return Err(error);
        }
    };
    Ok(Box::new(LinuxHost {
        handle,
        events,
        closed_reported: false,
        monitor_stop,
        monitor: Some(monitor),
    }))
}

impl TrayHost for LinuxHost {
    fn update(&mut self, view: &TrayView) -> anyhow::Result<()> {
        self.handle.update(|tray| tray.menu.update(view));
        Ok(())
    }
    fn pump(&mut self, timeout: Duration) -> anyhow::Result<()> {
        if self.handle.is_closed() && !self.closed_reported {
            self.closed_reported = true;
            let _ = self.events.send(ControlEvent::TrayUnavailable(
                "Linux tray service stopped".into(),
            ));
        }
        std::thread::sleep(timeout);
        Ok(())
    }
}

impl Drop for LinuxHost {
    fn drop(&mut self) {
        self.monitor_stop.store(true, Ordering::Relaxed);
        self.handle.shutdown().wait();
        if let Some(monitor) = self.monitor.take() {
            let _ = monitor.join();
        }
    }
}

fn start_host_monitor(
    events: Sender<ControlEvent>,
    stop: Arc<AtomicBool>,
) -> anyhow::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("rdpbeacon-tray-monitor".into())
        .spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                let connection = zbus::blocking::connection::Builder::session()?
                    .method_timeout(Duration::from_secs(1))
                    .build()?;
                let proxy: zbus::blocking::Proxy = zbus::blocking::proxy::Builder::new(&connection)
                    .destination("org.kde.StatusNotifierWatcher")?
                    .path("/StatusNotifierWatcher")?
                    .interface("org.kde.StatusNotifierWatcher")?
                    .cache_properties(zbus::proxy::CacheProperties::No)
                    .build()?;
                while !stop.load(Ordering::Relaxed) {
                    if !proxy.get_property::<bool>("IsStatusNotifierHostRegistered")? {
                        anyhow::bail!("Linux tray display host unregistered");
                    }
                    for _ in 0..10 {
                        if stop.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                Ok(())
            })();
            if !stop.load(Ordering::Relaxed) {
                if let Err(error) = result {
                    let _ = events.send(ControlEvent::TrayUnavailable(format!(
                        "Monitor Linux tray host: {error:#}"
                    )));
                }
            }
        })
        .context("Start tray display-host monitor")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ksni::Tray;
    #[test]
    fn menu_disables_status_and_reflects_logging_failure() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut tray = LinuxTray {
            menu: MenuModel::new(tx),
        };
        let menu = tray.menu();
        assert!(matches!(&menu[0], ksni::MenuItem::Standard(item) if !item.enabled));
        if let ksni::MenuItem::Checkmark(item) = &menu[1] {
            assert!(!item.checked);
            (item.activate)(&mut tray);
        } else {
            panic!("Missing logging checkbox");
        }
        assert!(matches!(rx.recv().unwrap(), ControlEvent::SetLogging(true)));
        tray.menu.update(&TrayView {
            status: "Running; logging error: disk full".into(),
            logging_enabled: false,
            interval: Duration::from_secs(600),
            notify_before: Duration::ZERO,
            quitting: false,
        });
        assert!(matches!(&tray.menu()[1], ksni::MenuItem::Checkmark(item) if !item.checked));
    }

    #[test]
    fn timing_submenus_select_presets_and_mark_current() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut tray = LinuxTray {
            menu: MenuModel::new(tx),
        };
        tray.menu.update(&TrayView {
            status: "Running".into(),
            logging_enabled: false,
            interval: Duration::from_secs(300),
            notify_before: Duration::from_secs(30),
            quitting: false,
        });
        let menu = tray.menu();
        let ksni::MenuItem::SubMenu(interval) = &menu[2] else {
            panic!("Missing interval submenu");
        };
        let ksni::MenuItem::RadioGroup(group) = &interval.submenu[0] else {
            panic!("Missing interval radio group");
        };
        assert_eq!(group.selected, 3);
        (group.select)(&mut tray, 0);
        assert!(
            matches!(rx.recv().unwrap(), ControlEvent::SetInterval(d) if d == Duration::from_secs(60))
        );
        let ksni::MenuItem::SubMenu(reminder) = &menu[3] else {
            panic!("Missing reminder submenu");
        };
        let ksni::MenuItem::RadioGroup(group) = &reminder.submenu[0] else {
            panic!("Missing reminder radio group");
        };
        assert_eq!(group.selected, 2);
        (group.select)(&mut tray, 0);
        assert!(
            matches!(rx.recv().unwrap(), ControlEvent::SetNotifyBefore(d) if d.is_zero())
        );
    }
}
