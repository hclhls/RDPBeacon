//! Opt-in integration test: runs the Wayland screen fade against a headless Sway.
//!
//! It skips itself unless `sway`, `grim`, and write access to `/dev/uinput` are available.
//! Sway runs in a private `XDG_RUNTIME_DIR`, so the user's real session is never touched.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn have(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn uinput_writable() -> bool {
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok()
}

struct Sway {
    child: Child,
    runtime_dir: PathBuf,
    display: String,
}

impl Sway {
    fn start(dir: &Path) -> Option<Self> {
        let config = dir.join("sway.conf");
        std::fs::write(&config, "output * bg #ffffff solid_color\n").ok()?;
        let runtime_dir = dir.join("run");
        std::fs::create_dir(&runtime_dir).ok()?;
        std::fs::set_permissions(
            &runtime_dir,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .ok()?;

        let child = Command::new("sway")
            .arg("--unsupported-gpu")
            .arg("-c")
            .arg(&config)
            .env("XDG_RUNTIME_DIR", &runtime_dir)
            .env("WLR_BACKENDS", "headless")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("WLR_RENDERER", "pixman")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut sway = Self {
            child,
            runtime_dir,
            display: String::new(),
        };

        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if let Ok(entries) = std::fs::read_dir(&sway.runtime_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with("wayland-") && !name.ends_with(".lock") {
                        sway.display = name;
                        // Give sway a moment to finish initialising its globals.
                        std::thread::sleep(Duration::from_millis(500));
                        return Some(sway);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("WAYLAND_DISPLAY", &self.display)
            .env_remove("DISPLAY");
        cmd
    }

    /// Returns the red channel of the top-left output pixel.
    fn sample(&self) -> u8 {
        let out = self
            .command("grim")
            .args(["-g", "10,10 1x1", "-t", "ppm", "-"])
            .output()
            .expect("failed to run grim");
        assert!(out.status.success(), "grim failed");
        out.stdout[out.stdout.len() - 3]
    }
}

impl Drop for Sway {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn fades_screen_during_beacon_and_restores_it() {
    if !have("sway") || !have("grim") || !uinput_writable() {
        eprintln!("skipping: needs sway, grim, and write access to /dev/uinput");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let sway = Sway::start(dir.path()).expect("headless sway did not start");
    assert_eq!(sway.sample(), 255, "background should be white before the fade");

    let config = dir.path().join("rdpbeacon.toml");
    std::fs::write(&config, "fade_duration = \"2s\"\nmode = \"mouse\"\n").unwrap();
    let mut beacon = sway
        .command(env!("CARGO_BIN_EXE_rdpbeacon"))
        .args(["--backend", "wayland", "--config"])
        .arg(&config)
        .arg("once")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start rdpbeacon");

    std::thread::sleep(Duration::from_millis(1000));
    let mid = sway.sample();
    assert!(mid < 255, "screen should be darkened mid-fade, got {mid}");

    let status = beacon.wait().unwrap();
    assert!(status.success(), "rdpbeacon exited with {status}");
    assert_eq!(sway.sample(), 255, "screen must be restored after the fade");
}
