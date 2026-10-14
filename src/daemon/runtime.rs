use super::{launch, logging, AppState, ControlEvent, WorkerState};
use crate::backend::Backend;
use crate::config::{BackendKind, Config};
use crate::tray::TrayHost;
use anyhow::Context;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{
    mpsc::{self, Sender},
    Arc,
};
use std::thread::JoinHandle;
use std::time::Duration;

struct WorkerGuard {
    stop: Arc<AtomicBool>,
    start: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl WorkerGuard {
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.start.take();
    }
    fn join(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.stop();
        self.join();
    }
}

pub fn run(cfg: Config, kind: BackendKind, verbose: bool) -> anyhow::Result<()> {
    run_with(
        cfg,
        move |_| crate::backend::make_backend(kind),
        crate::tray::create,
        move |events| {
            let signals = events.clone();
            ctrlc::set_handler(move || {
                let _ = signals.send(ControlEvent::Quit);
            })
            .context("Install daemon stop handler")?;
            logging::install(verbose, events)
        },
        launch::report_startup,
    )
}

fn run_with<F, T, L, R>(
    cfg: Config,
    backend_factory: F,
    tray_factory: T,
    logging_factory: L,
    mut report: R,
) -> anyhow::Result<()>
where
    F: FnOnce(Sender<ControlEvent>) -> anyhow::Result<Box<dyn Backend>> + Send + 'static,
    T: FnOnce(Sender<ControlEvent>) -> anyhow::Result<Box<dyn TrayHost>>,
    L: FnOnce(Sender<ControlEvent>) -> anyhow::Result<logging::LoggingControl>,
    R: FnMut(Result<(), String>) -> anyhow::Result<()>,
{
    let (events, incoming) = mpsc::channel();
    let logging = logging_factory(events.clone())?;
    let mut tray = tray_factory(events.clone())?;
    let mut state = AppState::default();
    state.interval = cfg.interval;
    state.notify_before = cfg.notify_before;
    tray.update(&state.view())?;
    let stop = Arc::new(AtomicBool::new(false));
    let (start_tx, start_rx) = mpsc::channel();
    let worker_stop = stop.clone();
    let timing = Arc::new(crate::scheduler::LiveTiming::from_config(&cfg));
    let worker_timing = timing.clone();
    let thread = std::thread::Builder::new()
        .name("rdpbeacon-worker".into())
        .spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<()> {
                    let backend = backend_factory(events.clone())?;
                    events.send(ControlEvent::WorkerReady)?;
                    if start_rx.recv().is_err() {
                        return Ok(());
                    }
                    crate::scheduler::run_loop_live(
                        backend.as_ref(),
                        &cfg,
                        &worker_stop,
                        crate::notify::system_notifier().as_ref(),
                        &worker_timing,
                    )
                }))
                .unwrap_or_else(|panic| {
                    let reason = panic
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("unknown panic");
                    Err(anyhow::anyhow!("Beacon worker panicked: {reason}"))
                })
                .map_err(|error| format!("{error:#}"));
            let _ = events.send(ControlEvent::WorkerFinished(result));
        })
        .context("Start beacon worker")?;
    let mut worker = WorkerGuard {
        stop,
        start: Some(start_tx),
        thread: Some(thread),
    };
    let mut ready = false;
    let mut exit_error = None;
    let mut previous_view = state.view();

    loop {
        // Bound each batch so a busy sender cannot starve the desktop message loop.
        for _ in 0..64 {
            let event = match incoming.try_recv() {
                Ok(event) => event,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    state.request_quit();
                    worker.stop();
                    break;
                }
            };
            match event {
                ControlEvent::WorkerReady => {
                    if !state.quitting {
                        state.worker = WorkerState::Running;
                        tray.update(&state.view())?;
                        report(Ok(()))?;
                        ready = true;
                        if let Some(start) = worker.start.take() {
                            start.send(()).context("Start ready beacon worker")?;
                        }
                    }
                }
                ControlEvent::WorkerFinished(result) => {
                    worker.join();
                    if !ready && !state.quitting {
                        anyhow::bail!(
                            "Initialize daemon backend: {}",
                            result
                                .err()
                                .unwrap_or_else(|| "Worker exited before startup".into())
                        );
                    }
                    state.worker_finished(result);
                    if !state.quitting {
                        if let WorkerState::Stopped(reason) = &state.worker {
                            logging.record_status(&format!("Stopped: {reason}"));
                        }
                    }
                }
                ControlEvent::SetLogging(enabled) => {
                    if !state.quitting {
                        state.logging_result(
                            enabled,
                            logging
                                .set_enabled(enabled)
                                .map_err(|error| format!("{error:#}")),
                        );
                        if enabled && logging.enabled() {
                            if let WorkerState::Stopped(reason) = &state.worker {
                                logging.record_status(&format!("Stopped: {reason}"));
                            }
                        }
                    }
                }
                ControlEvent::SetInterval(interval) => {
                    if !state.quitting && !interval.is_zero() {
                        timing.set_interval(interval);
                        state.interval = interval;
                    }
                }
                ControlEvent::SetNotifyBefore(notify_before) => {
                    if !state.quitting {
                        timing.set_notify_before(notify_before);
                        state.notify_before = notify_before;
                    }
                }
                ControlEvent::LoggingFailed(reason) => {
                    if !logging.enabled() {
                        state.logging_result(false, Err(reason));
                    }
                }
                ControlEvent::Quit => {
                    state.request_quit();
                    worker.stop();
                }
                ControlEvent::TrayUnavailable(reason) => {
                    logging.record_status(&reason);
                    exit_error = Some(reason);
                    state.request_quit();
                    worker.stop();
                }
            }
        }
        let view = state.view();
        if view != previous_view {
            if let Err(error) = tray.update(&view) {
                exit_error = Some(format!("Update daemon tray: {error:#}"));
                state.request_quit();
                worker.stop();
            }
            previous_view = view;
        }
        if state.can_exit() {
            break;
        }
        if let Err(error) = tray.pump(Duration::from_millis(50)) {
            exit_error = Some(format!("Run daemon tray: {error:#}"));
            state.request_quit();
            worker.stop();
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    logging.set_enabled(false)?;
    drop(tray);
    if !ready {
        anyhow::bail!("Daemon stopped before startup completed");
    }
    if let Some(reason) = exit_error {
        anyhow::bail!("{reason}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::TrayView;
    use super::*;
    use crate::backend::{Backend, WindowId};
    use crate::config::Mode;
    use std::sync::Mutex;

    struct TestBackend {
        events: Sender<ControlEvent>,
        calls: Arc<Mutex<Vec<&'static str>>>,
        missing: bool,
        panic_idle: bool,
    }
    impl Backend for TestBackend {
        fn idle_time(&self) -> anyhow::Result<Duration> {
            if self.panic_idle {
                panic!("idle query panic");
            }
            Ok(Duration::MAX)
        }
        fn find_horizon_window(&self, _: &str) -> anyhow::Result<Option<WindowId>> {
            Ok(if self.missing { None } else { Some(1) })
        }
        fn active_window(&self) -> anyhow::Result<Option<WindowId>> {
            Ok(Some(2))
        }
        fn focus(&self, window: WindowId) -> anyhow::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(if window == 1 { "focus" } else { "restore" });
            Ok(())
        }
        fn send_beacon(&self, _: Mode, _: &str) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push("beacon");
            self.events.send(ControlEvent::Quit).unwrap();
            Ok(())
        }
        fn supports_focus(&self) -> bool {
            true
        }
    }

    struct TestTray {
        events: Sender<ControlEvent>,
        views: Arc<Mutex<Vec<TrayView>>>,
        quit_running: bool,
        log_stopped: bool,
    }
    impl crate::tray::TrayHost for TestTray {
        fn update(&mut self, view: &TrayView) -> anyhow::Result<()> {
            self.views.lock().unwrap().push(view.clone());
            if self.quit_running && view.status == "Running" {
                self.events.send(ControlEvent::Quit)?;
                self.events.send(ControlEvent::Quit)?;
            }
            if self.log_stopped && view.status.starts_with("Stopped:") {
                if view.logging_enabled {
                    self.events.send(ControlEvent::Quit)?;
                } else {
                    self.events.send(ControlEvent::SetLogging(true))?;
                }
            }
            Ok(())
        }
        fn pump(&mut self, timeout: Duration) -> anyhow::Result<()> {
            std::thread::sleep(timeout.min(Duration::from_millis(5)));
            Ok(())
        }
    }

    fn config() -> Config {
        Config {
            interval: Duration::from_secs(1),
            jitter: Duration::ZERO,
            max_misses: 1,
            ..Config::default()
        }
    }

    #[test]
    fn readiness_requires_backend_and_tray_then_quit_stops_sleep() {
        let ready = Arc::new(AtomicBool::new(false));
        let views = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let dir = tempfile::tempdir().unwrap();
        let start = std::time::Instant::now();
        run_with(
            config(),
            {
                let ready = ready.clone();
                let calls = calls.clone();
                move |events| {
                    ready.store(true, Ordering::Relaxed);
                    Ok(Box::new(TestBackend {
                        events,
                        calls,
                        missing: false,
                        panic_idle: false,
                    }))
                }
            },
            {
                let views = views.clone();
                move |events| {
                    Ok(Box::new(TestTray {
                        events,
                        views,
                        quit_running: true,
                        log_stopped: false,
                    }))
                }
            },
            |events| Ok(logging::for_path(dir.path().join("log"), events)),
            |result| {
                assert!(result.is_ok());
                assert!(ready.load(Ordering::Relaxed));
                assert!(views
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|view| view.status == "Running"));
                Ok(())
            },
        )
        .unwrap();
        assert!(calls.lock().unwrap().is_empty());
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn quit_during_cycle_waits_for_focus_restoration() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let dir = tempfile::tempdir().unwrap();
        run_with(
            config(),
            {
                let calls = calls.clone();
                move |events| {
                    Ok(Box::new(TestBackend {
                        events,
                        calls,
                        missing: false,
                        panic_idle: false,
                    }))
                }
            },
            |events| {
                Ok(Box::new(TestTray {
                    events,
                    views: Arc::new(Mutex::new(Vec::new())),
                    quit_running: false,
                    log_stopped: false,
                }))
            },
            |events| Ok(logging::for_path(dir.path().join("log"), events)),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(*calls.lock().unwrap(), ["focus", "beacon", "restore"]);
    }

    #[test]
    fn stopped_worker_retains_tray_and_later_logging_records_reason() {
        let views = Arc::new(Mutex::new(Vec::new()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        run_with(
            config(),
            |events| {
                Ok(Box::new(TestBackend {
                    events,
                    calls: Arc::new(Mutex::new(Vec::new())),
                    missing: true,
                    panic_idle: false,
                }))
            },
            {
                let views = views.clone();
                move |events| {
                    Ok(Box::new(TestTray {
                        events,
                        views,
                        quit_running: false,
                        log_stopped: true,
                    }))
                }
            },
            |events| Ok(logging::for_path(path.clone(), events)),
            |_| Ok(()),
        )
        .unwrap();
        assert!(views
            .lock()
            .unwrap()
            .iter()
            .any(|view| view.status.starts_with("Stopped:") && !view.quitting));
        assert!(std::fs::read_to_string(path)
            .unwrap()
            .contains("not found after 1 consecutive attempts"));
    }

    #[test]
    fn failed_startup_report_cleans_up_before_any_beacon() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let dir = tempfile::tempdir().unwrap();
        let result = run_with(
            config(),
            {
                let calls = calls.clone();
                move |events| {
                    Ok(Box::new(TestBackend {
                        events,
                        calls,
                        missing: false,
                        panic_idle: false,
                    }))
                }
            },
            |events| {
                Ok(Box::new(TestTray {
                    events,
                    views: Arc::new(Mutex::new(Vec::new())),
                    quit_running: false,
                    log_stopped: false,
                }))
            },
            |events| Ok(logging::for_path(dir.path().join("log"), events)),
            |_| anyhow::bail!("broken startup pipe"),
        );
        assert!(result.is_err());
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn backend_initialization_failure_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = run_with(
            config(),
            |_| anyhow::bail!("backend unavailable"),
            |events| {
                Ok(Box::new(TestTray {
                    events,
                    views: Arc::new(Mutex::new(Vec::new())),
                    quit_running: false,
                    log_stopped: false,
                }))
            },
            |events| Ok(logging::for_path(dir.path().join("log"), events)),
            |_| panic!("Must not report ready"),
        );
        assert!(format!("{:#}", result.unwrap_err()).contains("backend unavailable"));
    }

    #[test]
    fn tray_failure_never_starts_worker() {
        let dir = tempfile::tempdir().unwrap();
        assert!(run_with(
            config(),
            |_| panic!("Worker must not start"),
            |_| anyhow::bail!("no tray host"),
            |events| Ok(logging::for_path(dir.path().join("log"), events)),
            |_| panic!("Must not report ready")
        )
        .is_err());
    }
    #[test]
    fn worker_panic_retains_tray_and_records_reason() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        run_with(
            config(),
            |events| {
                Ok(Box::new(TestBackend {
                    events,
                    calls: Arc::new(Mutex::new(Vec::new())),
                    missing: false,
                    panic_idle: true,
                }))
            },
            |events| {
                Ok(Box::new(TestTray {
                    events,
                    views: Arc::new(Mutex::new(Vec::new())),
                    quit_running: false,
                    log_stopped: true,
                }))
            },
            |events| Ok(logging::for_path(path.clone(), events)),
            |_| Ok(()),
        )
        .unwrap();
        assert!(std::fs::read_to_string(path)
            .unwrap()
            .contains("Beacon worker panicked: idle query panic"));
    }
}
