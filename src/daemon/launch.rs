use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Context;
use crate::config::BackendKind;

pub struct LaunchOptions {
    pub config: Option<PathBuf>,
    pub backend: Option<BackendKind>,
    pub verbose: bool,
}

fn child_args(options: &LaunchOptions, cwd: &Path) -> Vec<OsString> {
    let mut args = Vec::new();
    if let Some(path) = &options.config {
        args.push("--config".into());
        args.push(cwd.join(path).into_os_string());
    }
    if let Some(kind) = options.backend {
        args.push("--backend".into());
        args.push(match kind { BackendKind::X11 => "x11", BackendKind::Wayland => "wayland", BackendKind::Windows => "windows" }.into());
    }
    if options.verbose { args.push("--verbose".into()); }
    args.extend(["run".into(), "--internal-daemon".into()]);
    args
}

pub fn launch(options: LaunchOptions) -> anyhow::Result<()> {
    let mut command = Command::new(std::env::current_exe().context("Locate RDPBeacon executable")?);
    command.args(child_args(&options, &std::env::current_dir()?))
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00000200);
    }
    let mut child = command.spawn().context("Start tray daemon")?;
    wait_for_startup(&mut child, Duration::from_secs(10))?;
    println!("RDPBeacon is running in the tray (PID {})", child.id());
    Ok(())
}

fn wait_for_startup(child: &mut Child, timeout: Duration) -> anyhow::Result<()> {
    let result = (|| {
        let stdout = child.stdout.take().context("Missing daemon startup channel")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).take(4096).read_line(&mut line).map(|_| line);
            let _ = tx.send(result);
        });
        let line = rx.recv_timeout(timeout).context("Daemon startup timed out or channel closed")??;
        if line == "READY\n" { return Ok(()); }
        if let Some(reason) = line.strip_prefix("ERROR ") {
            anyhow::bail!("Daemon startup failed: {}", reason.trim());
        }
        anyhow::bail!("Daemon exited before readiness or sent an invalid startup response");
    })();
    if result.is_err() {
        let _ = child.kill();
        child.wait().context("Reap failed daemon child")?;
    }
    result
}

pub fn prepare_child() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    rustix::process::setsid().context("Detach daemon from launching terminal")?;
    Ok(())
}

pub fn report_startup(result: Result<(), String>) -> anyhow::Result<()> {
    {
        let mut stdout = std::io::stdout().lock();
        match result {
            Ok(()) => writeln!(stdout, "READY")?,
            Err(reason) => writeln!(stdout, "ERROR {}", reason.replace(['\n', '\r'], " ").chars().take(2000).collect::<String>())?,
        }
        stdout.flush()?;
    }
    #[cfg(target_os = "linux")]
    rustix::stdio::dup2_stdout(std::fs::File::options().write(true).open("/dev/null")?)?;
    #[cfg(windows)]
    {
        use std::os::windows::io::IntoRawHandle;
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::System::Console::{GetStdHandle, SetStdHandle, STD_OUTPUT_HANDLE};
        let file = std::fs::File::options().write(true).open("NUL")?;
        unsafe {
            let old = GetStdHandle(STD_OUTPUT_HANDLE)?;
            SetStdHandle(STD_OUTPUT_HANDLE, HANDLE(file.into_raw_handle()))?;
            let _ = CloseHandle(old);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unicode_relative_config_backend_and_verbose() {
        let options = LaunchOptions {
            config: Some(PathBuf::from("config folder/設定.toml")),
            backend: Some(BackendKind::X11), verbose: true,
        };
        let args = child_args(&options, Path::new("/tmp/base"));
        assert_eq!(args, ["--config", "/tmp/base/config folder/設定.toml", "--backend", "x11", "--verbose", "run", "--internal-daemon"].map(OsString::from));
    }

    #[cfg(unix)]
    fn fixture(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]).stdout(Stdio::piped());
        command
    }

    #[cfg(unix)]
    #[test]
    fn startup_waits_for_ready() {
        let mut child = fixture("printf 'READY\\n'; sleep 5").spawn().unwrap();
        assert!(wait_for_startup(&mut child, Duration::from_secs(1)).is_ok());
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn startup_failures_reap_child() {
        for script in ["printf 'ERROR no tray\\n'; exec sleep 5", "printf 'garbage\\n'; exec sleep 5", "exit 0", "exec sleep 5", "printf 'READY'; exec sleep 5"] {
            let mut child = fixture(script).spawn().unwrap();
            assert!(wait_for_startup(&mut child, Duration::from_millis(100)).is_err());
            assert!(child.try_wait().unwrap().is_some(), "orphan for {script}");
        }
    }
}
