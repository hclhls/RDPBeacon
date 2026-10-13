use super::{ControlEvent, MenuModel, TrayHost, TrayView, LOGGING_ID, QUIT_ID};
use anyhow::Context;
use ksni::blocking::TrayMethods;
use std::sync::mpsc::Sender;
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
        use ksni::menu::{CheckmarkItem, StandardItem};
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
}

pub fn create(events: Sender<ControlEvent>) -> anyhow::Result<Box<dyn TrayHost>> {
    let handle = LinuxTray { menu: MenuModel::new(events.clone()) }.spawn()
        .context("Register Linux tray: enable a StatusNotifierItem tray host (GNOME requires the AppIndicator and KStatusNotifierItem Support extension)")?;
    Ok(Box::new(LinuxHost {
        handle,
        events,
        closed_reported: false,
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
        self.handle.shutdown().wait();
    }
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
            quitting: false,
        });
        assert!(matches!(&tray.menu()[1], ksni::MenuItem::Checkmark(item) if !item.checked));
    }
}
