use std::time::Duration;
use anyhow::Result;

use crate::config::Mode;

pub type WindowId = u64;

pub trait Backend {
    fn idle_time(&self) -> Result<Duration>;
    fn find_horizon_window(&self, matcher: &str) -> Result<Option<WindowId>>;
    fn active_window(&self) -> Result<Option<WindowId>>;
    fn focus(&self, w: WindowId) -> Result<()>;
    fn send_beacon(&self, mode: Mode, key: &str) -> Result<()>;
    fn supports_focus(&self) -> bool;
}
