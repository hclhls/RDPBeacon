use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{mpsc::Sender, Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use super::ControlEvent;
use anyhow::Context;
use log::Log;

const MAX_BYTES: u64 = 5 * 1024 * 1024;
type PathResolver = Arc<dyn Fn() -> anyhow::Result<PathBuf> + Send + Sync>;

struct FileSink {
    path: PathBuf,
}
struct SharedSink {
    file: Mutex<Option<FileSink>>,
    resolve: PathResolver,
    events: Sender<ControlEvent>,
}

#[derive(Clone)]
pub struct LoggingControl {
    shared: Arc<SharedSink>,
}
struct DaemonLogger {
    shared: Arc<SharedSink>,
    filter: env_logger::Logger,
}

pub fn resolve_log_path(
    state_home: Option<&Path>,
    home: Option<&Path>,
    local_app_data: Option<&Path>,
    windows: bool,
) -> anyhow::Result<PathBuf> {
    if windows {
        let base = local_app_data
            .filter(|path| path.is_absolute())
            .context("LOCALAPPDATA is missing or not an absolute path")?;
        Ok(base.join("RDPBeacon/logs/rdpbeacon.log"))
    } else if let Some(base) = state_home.filter(|path| path.is_absolute()) {
        Ok(base.join("rdpbeacon/rdpbeacon.log"))
    } else {
        let base = home
            .filter(|path| path.is_absolute())
            .context("HOME is missing or not an absolute path")?;
        Ok(base.join(".local/state/rdpbeacon/rdpbeacon.log"))
    }
}

fn build_logger(
    verbose: bool,
    filter: Option<&str>,
    resolve: PathResolver,
    events: Sender<ControlEvent>,
) -> (DaemonLogger, LoggingControl) {
    let mut builder = env_logger::Builder::new();
    builder.parse_filters(filter.unwrap_or(if verbose { "debug" } else { "info" }));
    let shared = Arc::new(SharedSink {
        file: Mutex::new(None),
        resolve,
        events,
    });
    (
        DaemonLogger {
            shared: shared.clone(),
            filter: builder.build(),
        },
        LoggingControl { shared },
    )
}

pub fn install(verbose: bool, events: Sender<ControlEvent>) -> anyhow::Result<LoggingControl> {
    let resolve = Arc::new(|| {
        let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        resolve_log_path(
            state.as_deref(),
            home.as_deref(),
            local.as_deref(),
            cfg!(windows),
        )
    });
    let directives = std::env::var("RUST_LOG").ok();
    let (logger, control) = build_logger(verbose, directives.as_deref(), resolve, events);
    log::set_max_level(logger.filter.filter());
    log::set_boxed_logger(Box::new(logger))
        .map_err(|e| anyhow::anyhow!("Install daemon logger: {e}"))?;
    Ok(control)
}

impl LoggingControl {
    pub fn record_status(&self, reason: &str) {
        let mut guard = self.shared.file.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sink) = guard.as_mut() {
            if let Err(error) = sink.write_record(format!("ERROR {reason}").as_bytes()) {
                *guard = None;
                let _ = self
                    .shared
                    .events
                    .send(ControlEvent::LoggingFailed(format!("{error:#}")));
            }
        }
    }
    pub fn enabled(&self) -> bool {
        self.shared
            .file
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    pub fn set_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        let mut guard = self.shared.file.lock().unwrap_or_else(|e| e.into_inner());
        if !enabled {
            guard.take();
            return Ok(());
        }
        if guard.is_some() {
            return Ok(());
        }
        let path = (self.shared.resolve)()?;
        let parent = path.parent().context("Log path has no parent directory")?;
        let mut dirs = std::fs::DirBuilder::new();
        dirs.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            dirs.mode(0o700);
        }
        dirs.create(parent)
            .with_context(|| format!("Create log directory {}", parent.display()))?;
        let mut sink = FileSink { path };
        sink.write_record(b"INFO Logging enabled")?;
        *guard = Some(sink);
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn for_path(path: PathBuf, events: Sender<ControlEvent>) -> LoggingControl {
    build_logger(false, None, Arc::new(move || Ok(path.clone())), events).1
}

fn open_private(path: &Path) -> anyhow::Result<File> {
    let mut options = OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("Open daemon log {}", path.display()))
}

fn open_lock(path: &Path) -> anyhow::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).context("Open daemon log lock")
}

impl FileSink {
    fn write_record(&mut self, message: &[u8]) -> anyhow::Result<()> {
        let lock = open_lock(&self.path.with_extension("log.lock"))?;
        lock.lock().context("Lock shared daemon log")?;
        // Closing this exclusively owned handle releases the lock on every exit
        // path, including rotation failures. No other handle clones exist.
        (|| {
            let millis = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let mut record = format!("[{millis}] ").into_bytes();
            record.extend_from_slice(message);
            record.truncate(MAX_BYTES as usize - 1);
            record.push(b'\n');
            // Every process reopens the current path while holding the same lock.
            // No writer retains an obsolete inode after another process rotates.
            let mut file = open_private(&self.path)?;
            if file.metadata()?.len().saturating_add(record.len() as u64) > MAX_BYTES {
                let backup = self.path.with_extension("log.1");
                match std::fs::remove_file(&backup) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e).context("Remove old log backup"),
                }
                drop(file);
                std::fs::rename(&self.path, &backup).context("Rotate daemon log")?;
                file = open_private(&self.path)?;
            }
            file.write_all(&record).context("Write daemon log")?;
            file.flush().context("Flush daemon log")
        })()
    }
}

impl Log for DaemonLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.filter.enabled(metadata)
            && self
                .shared
                .file
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some()
    }

    fn log(&self, record: &log::Record) {
        if !self.filter.matches(record) {
            return;
        }
        let mut guard = self.shared.file.lock().unwrap_or_else(|e| e.into_inner());
        let Some(sink) = guard.as_mut() else {
            return;
        };
        if let Err(error) =
            sink.write_record(format!("{} {}", record.level(), record.args()).as_bytes())
        {
            *guard = None;
            let _ = self
                .shared
                .events
                .send(ControlEvent::LoggingFailed(format!("{error:#}")));
        }
    }

    fn flush(&self) {
        // Each record is flushed before its cross-process lock is released.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Log;

    #[test]
    fn independent_sinks_preserve_backup_after_one_rotates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rdpbeacon.log");
        let (tx, _) = std::sync::mpsc::channel();
        let resolve: PathResolver = Arc::new({
            let path = path.clone();
            move || Ok(path.clone())
        });
        let (first, first_control) = build_logger(false, None, resolve.clone(), tx.clone());
        let (second, second_control) = build_logger(false, None, resolve, tx);
        first_control.set_enabled(true).unwrap();
        second_control.set_enabled(true).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(5 * 1024 * 1024)
            .unwrap();
        emit(&first, "first after rotation");
        emit(&second, "second after rotation");
        assert_eq!(
            std::fs::metadata(path.with_extension("log.1"))
                .unwrap()
                .len(),
            5 * 1024 * 1024
        );
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("first after rotation"));
        assert!(contents.contains("second after rotation"));
    }

    fn fixture(
        verbose: bool,
    ) -> (
        tempfile::TempDir,
        DaemonLogger,
        LoggingControl,
        std::sync::mpsc::Receiver<ControlEvent>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs/rdpbeacon.log");
        let (tx, rx) = std::sync::mpsc::channel();
        let (logger, control) = build_logger(verbose, None, Arc::new(move || Ok(path.clone())), tx);
        (dir, logger, control, rx)
    }

    fn emit(logger: &DaemonLogger, message: &str) {
        logger.log(
            &log::Record::builder()
                .args(format_args!("{message}"))
                .level(log::Level::Info)
                .target("rdpbeacon")
                .build(),
        );
    }

    #[test]
    fn toggles_are_lazy_and_do_not_replay_disabled_records() {
        let (dir, logger, control, _) = fixture(true);
        emit(&logger, "before enable");
        assert!(!dir.path().join("logs").exists());
        control.set_enabled(true).unwrap();
        emit(&logger, "first enabled");
        control.set_enabled(false).unwrap();
        emit(&logger, "after disable");
        control.set_enabled(true).unwrap();
        emit(&logger, "second enabled");
        control.set_enabled(false).unwrap();
        let contents = std::fs::read_to_string(dir.path().join("logs/rdpbeacon.log")).unwrap();
        assert!(contents.contains("Logging enabled"));
        assert!(contents.contains("first enabled"));
        assert!(contents.contains("second enabled"));
        assert!(!contents.contains("before enable"));
        assert!(!contents.contains("after disable"));
        assert!(!contents.contains('\u{1b}'));
    }

    #[test]
    fn filtering_does_not_enable_logging() {
        let (dir, logger, control, _) = fixture(false);
        assert!(!control.enabled());
        control.set_enabled(true).unwrap();
        logger.log(
            &log::Record::builder()
                .args(format_args!("filtered debug"))
                .level(log::Level::Debug)
                .target("rdpbeacon")
                .build(),
        );
        control.set_enabled(false).unwrap();
        assert!(
            !std::fs::read_to_string(dir.path().join("logs/rdpbeacon.log"))
                .unwrap()
                .contains("filtered debug")
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolves_user_paths_and_rejects_missing_home() {
        assert_eq!(
            resolve_log_path(Some(Path::new("/state")), None, None, false).unwrap(),
            PathBuf::from("/state/rdpbeacon/rdpbeacon.log")
        );
        assert_eq!(
            resolve_log_path(
                Some(Path::new("relative")),
                Some(Path::new("/home/user")),
                None,
                false
            )
            .unwrap(),
            PathBuf::from("/home/user/.local/state/rdpbeacon/rdpbeacon.log")
        );
        assert!(resolve_log_path(None, None, None, false).is_err());
        assert_eq!(
            resolve_log_path(None, None, Some(Path::new("/local")), true).unwrap(),
            PathBuf::from("/local/RDPBeacon/logs/rdpbeacon.log")
        );
    }

    #[test]
    fn rotates_existing_oversized_log_and_keeps_one_backup() {
        let (dir, logger, control, _) = fixture(false);
        let path = dir.path().join("logs/rdpbeacon.log");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(5 * 1024 * 1024).unwrap();
        std::fs::write(path.with_extension("log.1"), "old backup").unwrap();
        control.set_enabled(true).unwrap();
        emit(&logger, "fresh record");
        control.set_enabled(false).unwrap();
        assert_eq!(
            std::fs::metadata(path.with_extension("log.1"))
                .unwrap()
                .len(),
            5 * 1024 * 1024
        );
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("fresh record"));
        assert!(std::fs::metadata(path).unwrap().len() < 5 * 1024 * 1024);
    }

    #[test]
    fn failed_open_leaves_logging_disabled_and_can_recover() {
        let (dir, _, control, _) = fixture(false);
        std::fs::write(dir.path().join("logs"), "not a directory").unwrap();
        assert!(control.set_enabled(true).is_err());
        assert!(!control.enabled());
        std::fs::remove_file(dir.path().join("logs")).unwrap();
        control.set_enabled(true).unwrap();
        assert!(control.enabled());
    }

    #[test]
    fn rotation_failure_disables_sink_and_reports_once() {
        let (dir, logger, control, rx) = fixture(false);
        control.set_enabled(true).unwrap();
        let path = dir.path().join("logs/rdpbeacon.log");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(5 * 1024 * 1024)
            .unwrap();
        std::fs::create_dir(path.with_extension("log.1")).unwrap();
        emit(&logger, "fails rotation");
        assert!(!control.enabled());
        assert!(matches!(
            rx.try_recv().unwrap(),
            ControlEvent::LoggingFailed(_)
        ));
        emit(&logger, "no recursive report");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn concurrent_writes_after_disable_are_suppressed() {
        let (dir, logger, control, _) = fixture(false);
        control.set_enabled(true).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        std::thread::scope(|scope| {
            let barrier_clone = barrier.clone();
            let logger = &logger;
            scope.spawn(move || {
                barrier_clone.wait();
                emit(logger, "after serialized disable");
            });
            control.set_enabled(false).unwrap();
            barrier.wait();
        });
        assert!(
            !std::fs::read_to_string(dir.path().join("logs/rdpbeacon.log"))
                .unwrap()
                .contains("after serialized disable")
        );
    }
}
