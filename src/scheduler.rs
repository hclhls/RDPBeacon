use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use anyhow::Result;
use rand::Rng;

use crate::backend::Backend;
use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleOutcome {
    SkippedActive,
    SkippedNoWindow,
    Sent,
}

pub fn next_sleep(cfg: &Config, rng: &mut impl Rng) -> Duration {
    let min_sleep = Duration::from_secs(1);
    let low = cfg.interval.saturating_sub(cfg.jitter).max(min_sleep);
    let high = cfg.interval.saturating_add(cfg.jitter).max(min_sleep);

    if low >= high {
        low
    } else {
        rng.gen_range(low..=high)
    }
}

pub fn run_cycle(b: &dyn Backend, cfg: &Config) -> Result<CycleOutcome> {
    let idle = b.idle_time()?;
    if idle < cfg.idle_threshold {
        log::debug!(
            "User active (idle time {:?} < threshold {:?}); skipping beacon",
            idle,
            cfg.idle_threshold
        );
        return Ok(CycleOutcome::SkippedActive);
    }

    let horizon_win = b.find_horizon_window(&cfg.window_match)?;
    let Some(horizon_win) = horizon_win else {
        log::warn!("Horizon window matching '{}' not found", cfg.window_match);
        return Ok(CycleOutcome::SkippedNoWindow);
    };

    if b.supports_focus() {
        let prev_window = b.active_window()?;
        if prev_window == Some(horizon_win) {
            log::debug!("Horizon window {horizon_win} is already focused");
            b.send_beacon(cfg.mode, &cfg.key)?;
        } else {
            log::debug!(
                "Focusing Horizon window {horizon_win} (previous window: {prev_window:?})"
            );
            b.focus(horizon_win)?;
            let beacon_res = b.send_beacon(cfg.mode, &cfg.key);
            std::thread::sleep(Duration::from_millis(100));
            if let Some(prev) = prev_window {
                log::debug!("Restoring focus to previous window {prev}");
                if let Err(e) = b.focus(prev) {
                    log::warn!("Failed to restore focus to previous window {prev}: {e}");
                }
            }
            beacon_res?;
        }
    } else {
        b.send_beacon(cfg.mode, &cfg.key)?;
    }

    Ok(CycleOutcome::Sent)
}

fn interruptible_sleep(duration: Duration, stop: &AtomicBool) -> bool {
    let chunk = Duration::from_millis(200);
    let mut remaining = duration;
    while remaining > Duration::ZERO {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let this_sleep = remaining.min(chunk);
        std::thread::sleep(this_sleep);
        remaining = remaining.saturating_sub(this_sleep);
    }
    !stop.load(Ordering::Relaxed)
}

pub fn run_loop(b: &dyn Backend, cfg: &Config, stop: &AtomicBool) -> Result<()> {
    let mut consecutive_misses: u32 = 0;
    let mut rng = rand::thread_rng();

    while !stop.load(Ordering::Relaxed) {
        let sleep_dur = next_sleep(cfg, &mut rng);
        if !interruptible_sleep(sleep_dur, stop) {
            break;
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }

        match run_cycle(b, cfg) {
            Ok(outcome) => match outcome {
                CycleOutcome::SkippedNoWindow => {
                    consecutive_misses += 1;
                    log::warn!(
                        "Horizon window not found ({}/{})",
                        consecutive_misses,
                        cfg.max_misses
                    );
                    if cfg.max_misses > 0 && consecutive_misses >= cfg.max_misses {
                        anyhow::bail!(
                            "Window matching '{}' not found after {} consecutive attempts",
                            cfg.window_match,
                            cfg.max_misses
                        );
                    }
                }
                CycleOutcome::Sent => {
                    consecutive_misses = 0;
                    log::info!("Beacon sent successfully");
                }
                CycleOutcome::SkippedActive => {
                    consecutive_misses = 0;
                    log::debug!("User active; skipped cycle");
                }
            },
            Err(e) => {
                log::error!("Cycle execution failed: {e:#}");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::WindowId;
    use crate::config::Mode;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum MockCall {
        IdleTime,
        FindHorizonWindow(String),
        ActiveWindow,
        Focus(WindowId),
        SendBeacon(Mode, String),
        SupportsFocus,
    }

    struct MockBackend {
        idle_time: Duration,
        horizon_window: Option<WindowId>,
        active_window: Option<WindowId>,
        supports_focus: bool,
        fail_focus_on: Option<WindowId>,
        fail_send_beacon: bool,
        calls: Arc<Mutex<Vec<MockCall>>>,
    }

    impl MockBackend {
        fn new() -> Self {
            Self {
                idle_time: Duration::from_secs(300),
                horizon_window: Some(100),
                active_window: Some(200),
                supports_focus: true,
                fail_focus_on: None,
                fail_send_beacon: false,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn calls(&self) -> Vec<MockCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Backend for MockBackend {
        fn idle_time(&self) -> Result<Duration> {
            self.calls.lock().unwrap().push(MockCall::IdleTime);
            Ok(self.idle_time)
        }

        fn find_horizon_window(&self, matcher: &str) -> Result<Option<WindowId>> {
            self.calls.lock().unwrap().push(MockCall::FindHorizonWindow(matcher.to_string()));
            Ok(self.horizon_window)
        }

        fn active_window(&self) -> Result<Option<WindowId>> {
            self.calls.lock().unwrap().push(MockCall::ActiveWindow);
            Ok(self.active_window)
        }

        fn focus(&self, w: WindowId) -> Result<()> {
            self.calls.lock().unwrap().push(MockCall::Focus(w));
            if let Some(target) = self.fail_focus_on {
                if target == w {
                    anyhow::bail!("failed to focus window {w}");
                }
            }
            Ok(())
        }

        fn send_beacon(&self, mode: Mode, key: &str) -> Result<()> {
            self.calls.lock().unwrap().push(MockCall::SendBeacon(mode, key.to_string()));
            if self.fail_send_beacon {
                anyhow::bail!("synthetic beacon injection error");
            }
            Ok(())
        }

        fn supports_focus(&self) -> bool {
            self.calls.lock().unwrap().push(MockCall::SupportsFocus);
            self.supports_focus
        }
    }

    #[test]
    fn skips_when_user_active() {
        let mut mock = MockBackend::new();
        mock.idle_time = Duration::from_secs(60);
        let cfg = Config::default();

        let outcome = run_cycle(&mock, &cfg).unwrap();
        assert_eq!(outcome, CycleOutcome::SkippedActive);

        let calls = mock.calls();
        assert!(!calls.iter().any(|c| matches!(c, MockCall::SendBeacon(..))));
    }

    #[test]
    fn sends_when_idle_and_window_found() {
        let mut mock = MockBackend::new();
        mock.idle_time = Duration::from_secs(200);
        mock.horizon_window = Some(42);
        let cfg = Config::default();

        let outcome = run_cycle(&mock, &cfg).unwrap();
        assert_eq!(outcome, CycleOutcome::Sent);

        let calls = mock.calls();
        assert!(calls.iter().any(|c| matches!(c, MockCall::SendBeacon(..))));
    }

    #[test]
    fn no_window_gives_skipped_no_window() {
        let mut mock = MockBackend::new();
        mock.idle_time = Duration::from_secs(200);
        mock.horizon_window = None;
        let cfg = Config::default();

        let outcome = run_cycle(&mock, &cfg).unwrap();
        assert_eq!(outcome, CycleOutcome::SkippedNoWindow);

        let calls = mock.calls();
        assert!(!calls.iter().any(|c| matches!(c, MockCall::SendBeacon(..))));
    }

    #[test]
    fn focus_and_restore_order() {
        let mut mock = MockBackend::new();
        mock.idle_time = Duration::from_secs(300);
        mock.horizon_window = Some(101);
        mock.active_window = Some(202);
        mock.supports_focus = true;
        let cfg = Config::default();

        let outcome = run_cycle(&mock, &cfg).unwrap();
        assert_eq!(outcome, CycleOutcome::Sent);

        let relevant: Vec<_> = mock.calls().into_iter().filter(|c| matches!(
            c,
            MockCall::ActiveWindow | MockCall::Focus(_) | MockCall::SendBeacon(..)
        )).collect();

        assert_eq!(
            relevant,
            vec![
                MockCall::ActiveWindow,
                MockCall::Focus(101),
                MockCall::SendBeacon(cfg.mode, cfg.key.clone()),
                MockCall::Focus(202),
            ]
        );
    }

    #[test]
    fn skips_focus_when_already_active() {
        let mut mock = MockBackend::new();
        mock.idle_time = Duration::from_secs(300);
        mock.horizon_window = Some(101);
        mock.active_window = Some(101);
        mock.supports_focus = true;
        let cfg = Config::default();

        let outcome = run_cycle(&mock, &cfg).unwrap();
        assert_eq!(outcome, CycleOutcome::Sent);

        let relevant: Vec<_> = mock.calls().into_iter().filter(|c| matches!(
            c,
            MockCall::ActiveWindow | MockCall::Focus(_) | MockCall::SendBeacon(..)
        )).collect();

        assert_eq!(
            relevant,
            vec![
                MockCall::ActiveWindow,
                MockCall::SendBeacon(cfg.mode, cfg.key.clone()),
            ]
        );
    }

    #[test]
    fn no_focus_calls_when_unsupported() {
        let mut mock = MockBackend::new();
        mock.idle_time = Duration::from_secs(300);
        mock.horizon_window = Some(101);
        mock.active_window = Some(202);
        mock.supports_focus = false;
        let cfg = Config::default();

        let outcome = run_cycle(&mock, &cfg).unwrap();
        assert_eq!(outcome, CycleOutcome::Sent);

        let calls = mock.calls();
        assert!(!calls.iter().any(|c| matches!(c, MockCall::ActiveWindow)));
        assert!(!calls.iter().any(|c| matches!(c, MockCall::Focus(_))));
        assert!(calls.iter().any(|c| matches!(c, MockCall::SendBeacon(..))));
    }

    #[test]
    fn restore_skipped_if_prev_missing() {
        let mut mock = MockBackend::new();
        mock.idle_time = Duration::from_secs(300);
        mock.horizon_window = Some(101);
        mock.active_window = None;
        mock.supports_focus = true;
        let cfg = Config::default();

        let outcome = run_cycle(&mock, &cfg).unwrap();
        assert_eq!(outcome, CycleOutcome::Sent);

        let relevant: Vec<_> = mock.calls().into_iter().filter(|c| matches!(
            c,
            MockCall::ActiveWindow | MockCall::Focus(_) | MockCall::SendBeacon(..)
        )).collect();

        assert_eq!(
            relevant,
            vec![
                MockCall::ActiveWindow,
                MockCall::Focus(101),
                MockCall::SendBeacon(cfg.mode, cfg.key.clone()),
            ]
        );
    }

    #[test]
    fn restore_error_does_not_fail_cycle() {
        let mut mock = MockBackend::new();
        mock.idle_time = Duration::from_secs(300);
        mock.horizon_window = Some(101);
        mock.active_window = Some(202);
        mock.supports_focus = true;
        mock.fail_focus_on = Some(202);
        let cfg = Config::default();

        let outcome = run_cycle(&mock, &cfg).unwrap();
        assert_eq!(outcome, CycleOutcome::Sent);
    }

    #[test]
    fn next_sleep_within_bounds_and_min_1s() {
        let mut rng = rand::thread_rng();

        let mut cfg = Config::default();
        cfg.interval = Duration::from_secs(10);
        cfg.jitter = Duration::from_secs(2);
        for _ in 0..100 {
            let sleep = next_sleep(&cfg, &mut rng);
            assert!(sleep >= Duration::from_secs(8), "sleep {:?} < 8s", sleep);
            assert!(sleep <= Duration::from_secs(12), "sleep {:?} > 12s", sleep);
        }

        cfg.interval = Duration::from_secs(2);
        cfg.jitter = Duration::from_secs(5);
        for _ in 0..100 {
            let sleep = next_sleep(&cfg, &mut rng);
            assert!(sleep >= Duration::from_secs(1), "sleep {:?} < 1s", sleep);
            assert!(sleep <= Duration::from_secs(7), "sleep {:?} > 7s", sleep);
        }

        cfg.interval = Duration::from_millis(200);
        cfg.jitter = Duration::from_millis(50);
        for _ in 0..20 {
            let sleep = next_sleep(&cfg, &mut rng);
            assert!(sleep >= Duration::from_secs(1), "sleep {:?} < 1s", sleep);
        }
    }

    #[test]
    fn run_loop_exits_after_max_misses() {
        let mut mock = MockBackend::new();
        mock.idle_time = Duration::from_secs(300);
        mock.horizon_window = None;

        let mut cfg = Config::default();
        cfg.interval = Duration::from_secs(1);
        cfg.jitter = Duration::from_secs(0);
        cfg.max_misses = 2;

        let stop = AtomicBool::new(false);
        let res = run_loop(&mock, &cfg, &stop);
        assert!(res.is_err(), "expected run_loop to exit with error after max misses");
        let err_msg = format!("{:#}", res.unwrap_err());
        assert_eq!(
            err_msg,
            format!(
                "Window matching '{}' not found after {} consecutive attempts",
                cfg.window_match, cfg.max_misses
            )
        );
    }

    #[test]
    fn run_loop_stops_promptly_on_flag() {
        let mock = Arc::new(MockBackend::new());
        let mut cfg = Config::default();
        cfg.interval = Duration::from_secs(30);
        cfg.jitter = Duration::from_secs(0);

        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let mock_clone = Arc::clone(&mock);

        let start = std::time::Instant::now();
        let handle = std::thread::spawn(move || {
            run_loop(mock_clone.as_ref(), &cfg, &stop_clone)
        });

        std::thread::sleep(Duration::from_millis(50));
        stop.store(true, Ordering::Relaxed);

        let res = handle.join().expect("thread should not panic");
        let elapsed = start.elapsed();

        assert!(res.is_ok(), "run_loop should exit cleanly on stop flag");
        assert!(
            elapsed < Duration::from_millis(500),
            "loop took too long to exit: {:?}",
            elapsed
        );
    }

    #[test]
    fn restore_focus_runs_even_if_send_beacon_fails() {
        let mut mock = MockBackend::new();
        mock.horizon_window = Some(100);
        mock.active_window = Some(200);
        mock.supports_focus = true;
        mock.fail_send_beacon = true;
        let cfg = Config::default();

        let res = run_cycle(&mock, &cfg);
        assert!(res.is_err(), "cycle should fail when beacon injection fails");

        let calls = mock.calls();
        assert_eq!(
            calls,
            vec![
                MockCall::IdleTime,
                MockCall::FindHorizonWindow("Omnissa Horizon Client".to_string()),
                MockCall::SupportsFocus,
                MockCall::ActiveWindow,
                MockCall::Focus(100),
                MockCall::SendBeacon(Mode::Key, "F15".to_string()),
                MockCall::Focus(200),
            ],
            "focus should be restored to previous window even if beacon injection errors"
        );
    }

    #[test]
    fn run_loop_continues_after_cycle_error() {
        let mut mock = MockBackend::new();
        mock.idle_time = Duration::from_secs(300);
        mock.horizon_window = Some(100);
        mock.active_window = Some(200);
        mock.supports_focus = true;
        mock.fail_send_beacon = true;

        let mut cfg = Config::default();
        cfg.interval = Duration::from_millis(100);
        cfg.jitter = Duration::from_millis(0);

        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let mock_arc = Arc::new(mock);
        let mock_clone = Arc::clone(&mock_arc);

        let handle = std::thread::spawn(move || {
            run_loop(mock_clone.as_ref(), &cfg, &stop_clone)
        });

        // Loop clamps sleep to min 1s; sleep 1.2s to let at least one error cycle execute and continue
        std::thread::sleep(Duration::from_millis(1200));
        stop.store(true, Ordering::Relaxed);

        let res = handle.join().expect("thread should not panic");
        assert!(res.is_ok(), "run_loop should not crash on single cycle errors");
        assert!(!mock_arc.calls().is_empty(), "loop should have executed a cycle despite errors");
    }
}
