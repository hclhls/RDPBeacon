use std::path::Path;
use std::time::Duration;
use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[serde(alias = "Key")]
    Key,
    #[serde(alias = "Mouse")]
    Mouse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    #[serde(alias = "X11", alias = "x11")]
    X11,
    #[serde(alias = "Wayland", alias = "wayland")]
    Wayland,
    #[serde(alias = "Windows", alias = "windows")]
    Windows,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(with = "humantime_serde")]
    pub interval: Duration,
    #[serde(with = "humantime_serde")]
    pub jitter: Duration,
    #[serde(with = "humantime_serde")]
    pub idle_threshold: Duration,
    pub key: String,
    pub mode: Mode,
    pub window_match: String,
    pub max_misses: u32,
    pub backend: Option<BackendKind>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(240),
            jitter: Duration::from_secs(20),
            idle_threshold: Duration::from_secs(180),
            key: "F15".to_string(),
            mode: Mode::Key,
            window_match: "Omnissa Horizon Client".to_string(),
            max_misses: 5,
            backend: None,
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let Some(path) = path else {
            return Ok(Self::default());
        };

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file '{}'", path.display()))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| format!("failed to parse config file '{}'", path.display()))?;

        Ok(config)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn defaults_when_no_file() {
        let cfg = Config::load(None).expect("loading without file should succeed");
        assert_eq!(cfg.interval, Duration::from_secs(240));
        assert_eq!(cfg.jitter, Duration::from_secs(20));
        assert_eq!(cfg.idle_threshold, Duration::from_secs(180));
        assert_eq!(cfg.key, "F15");
        assert_eq!(cfg.mode, Mode::Key);
        assert_eq!(cfg.window_match, "Omnissa Horizon Client");
        assert_eq!(cfg.max_misses, 5);
        assert_eq!(cfg.backend, None);
    }

    #[test]
    fn parses_toml_overrides() {
        let toml_str = r#"
            interval = "5m"
            mode = "mouse"
            max_misses = 10
        "#;
        let dir = std::env::temp_dir();
        let path = dir.join(format!("rdpbeacon_test_overrides_{}.toml", std::process::id()));
        std::fs::write(&path, toml_str).unwrap();
        let cfg = Config::load(Some(&path)).expect("loading valid toml should succeed");
        let _ = std::fs::remove_file(&path);

        assert_eq!(cfg.interval, Duration::from_secs(300));
        assert_eq!(cfg.mode, Mode::Mouse);
        assert_eq!(cfg.max_misses, 10);
        assert_eq!(cfg.jitter, Duration::from_secs(20));
        assert_eq!(cfg.idle_threshold, Duration::from_secs(180));
        assert_eq!(cfg.key, "F15");
        assert_eq!(cfg.window_match, "Omnissa Horizon Client");
        assert_eq!(cfg.backend, None);
    }

    #[test]
    fn malformed_toml_names_the_field() {
        let toml_str = r#"
            interval = "invalid_duration"
        "#;
        let dir = std::env::temp_dir();
        let path = dir.join(format!("rdpbeacon_test_malformed_{}.toml", std::process::id()));
        std::fs::write(&path, toml_str).unwrap();
        let res = Config::load(Some(&path));
        let _ = std::fs::remove_file(&path);

        assert!(res.is_err(), "expected error for malformed toml");
        let err_msg = format!("{:#}", res.unwrap_err());
        assert!(
            err_msg.contains("interval"),
            "error message '{err_msg}' should name the malformed field 'interval'"
        );
    }
}
