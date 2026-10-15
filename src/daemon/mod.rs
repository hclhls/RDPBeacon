pub mod launch;
pub mod logging;
mod runtime;
pub use runtime::run;

use std::time::Duration;

#[derive(Debug)]
pub enum ControlEvent {
    SetLogging(bool),
    SetInterval(Duration),
    SetNotifyBefore(Duration),
    SetFade(bool),
    Quit,
    TrayUnavailable(String),
    LoggingFailed(String),
    WorkerReady,
    WorkerFinished(Result<(), String>),
}

#[derive(Debug)]
pub enum WorkerState {
    Starting,
    Running,
    Stopping,
    Stopped(String),
}

pub struct AppState {
    pub worker: WorkerState,
    pub logging_enabled: bool,
    pub logging_error: Option<String>,
    pub interval: Duration,
    pub notify_before: Duration,
    pub fade_enabled: bool,
    quitting: bool,
    finished: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayView {
    pub status: String,
    pub logging_enabled: bool,
    pub interval: Duration,
    pub notify_before: Duration,
    pub fade_enabled: bool,
    pub quitting: bool,
}

impl Default for AppState {
    fn default() -> Self {
        let defaults = crate::config::Config::default();
        Self {
            interval: defaults.interval,
            notify_before: defaults.notify_before,
            fade_enabled: defaults.fade_enabled(),
            worker: WorkerState::Starting,
            logging_enabled: false,
            logging_error: None,
            quitting: false,
            finished: false,
        }
    }
}

impl AppState {
    pub fn view(&self) -> TrayView {
        let status = match &self.worker {
            WorkerState::Starting => "Starting".into(),
            WorkerState::Stopping => "Stopping".into(),
            WorkerState::Stopped(reason) => format!("Stopped: {reason}"),
            WorkerState::Running => self.logging_error.as_ref().map_or_else(
                || "Running".into(),
                |reason| format!("Running; logging error: {reason}"),
            ),
        };
        TrayView {
            status,
            logging_enabled: self.logging_enabled,
            interval: self.interval,
            notify_before: self.notify_before,
            fade_enabled: self.fade_enabled,
            quitting: self.quitting,
        }
    }

    pub fn request_quit(&mut self) {
        self.quitting = true;
        if !self.finished {
            self.worker = WorkerState::Stopping;
        }
    }

    pub fn worker_finished(&mut self, result: Result<(), String>) {
        self.finished = true;
        self.worker = WorkerState::Stopped(result.err().unwrap_or_else(|| "Worker exited".into()));
    }

    pub fn logging_result(&mut self, enabled: bool, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.logging_enabled = enabled;
                self.logging_error = None;
            }
            Err(reason) => {
                self.logging_enabled = false;
                self.logging_error = Some(reason);
            }
        }
    }

    pub fn can_exit(&self) -> bool {
        self.quitting && self.finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_view_uses_config_defaults() {
        let defaults = crate::config::Config::default();
        let view = AppState::default().view();
        assert_eq!(view.interval, defaults.interval);
        assert_eq!(view.notify_before, defaults.notify_before);
    }

    #[test]
    fn initial_state_has_logging_disabled() {
        assert!(!AppState::default().view().logging_enabled);
    }

    #[test]
    fn initial_state_has_fade_disabled_by_default() {
        assert!(!AppState::default().view().fade_enabled);
    }

    #[test]
    fn quit_is_idempotent() {
        let mut state = AppState::default();
        state.worker = WorkerState::Running;
        state.request_quit();
        state.request_quit();
        assert!(matches!(state.worker, WorkerState::Stopping));
        assert!(state.view().quitting);
    }

    #[test]
    fn worker_error_takes_precedence_over_logging_error() {
        let mut state = AppState::default();
        state.logging_result(true, Err("disk full".into()));
        state.worker_finished(Err("Horizon missing".into()));
        assert_eq!(state.view().status, "Stopped: Horizon missing");
        assert!(!state.view().logging_enabled);
    }

    #[test]
    fn quit_then_worker_completion_is_terminal() {
        let mut state = AppState::default();
        state.request_quit();
        state.worker_finished(Ok(()));
        state.request_quit();
        assert!(state.can_exit());
    }
}
