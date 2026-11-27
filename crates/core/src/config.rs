//! User configuration (`~/.paceline/config.json`).

use crate::paths::{write_atomic, Paths};
use crate::pricing::Rate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MenuBarStyle {
    /// Meter icon plus "5h% · 7d%".
    Both,
    /// Meter icon plus the 5-hour percentage.
    FiveHour,
    /// Meter icon plus the weekly percentage.
    SevenDay,
    /// Meter icon only.
    IconOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Notify when a window crosses these percentages.
    pub thresholds: Vec<u32>,
    /// Notify when the forecast says a limit will be hit before it resets.
    pub forecast_alerts: bool,
    pub notifications: bool,
    /// Background refresh interval in seconds.
    pub refresh_secs: u64,
    pub menu_bar: MenuBarStyle,
    /// Extra or replacement pricing rows (same shape as the built-in table).
    pub pricing_overrides: Vec<Rate>,
}

impl Default for Config {
    fn default() -> Self {
        Self { thresholds: vec![50, 75, 90], forecast_alerts: true, notifications: true, refresh_secs: 30, menu_bar: MenuBarStyle::Both, pricing_overrides: Vec::new() }
    }
}

impl Config {
    pub fn load(paths: &Paths) -> Self {
        std::fs::read(paths.config_file()).ok().and_then(|b| serde_json::from_slice::<Config>(&b).ok()).map(Config::sanitized).unwrap_or_default()
    }

    pub fn save(&self, paths: &Paths) -> std::io::Result<()> {
        let body = serde_json::to_vec_pretty(&self.clone().sanitized()).unwrap_or_default();
        write_atomic(&paths.config_file(), &body)
    }

    pub fn sanitized(mut self) -> Self {
        self.thresholds.retain(|t| (1..=100).contains(t));
        self.thresholds.sort_unstable();
        self.thresholds.dedup();
        self.refresh_secs = self.refresh_secs.clamp(5, 3600);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_sanitize() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths { claude_dirs: vec![], state_dir: dir.path().to_path_buf() };
        assert_eq!(Config::load(&paths).thresholds, vec![50, 75, 90]);

        let cfg = Config { thresholds: vec![90, 0, 50, 50, 250], refresh_secs: 1, menu_bar: MenuBarStyle::IconOnly, ..Default::default() };
        cfg.save(&paths).unwrap();
        let back = Config::load(&paths);
        assert_eq!(back.thresholds, vec![50, 90]);
        assert_eq!(back.refresh_secs, 5);
        assert_eq!(back.menu_bar, MenuBarStyle::IconOnly);
    }

    #[test]
    fn partial_file_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths { claude_dirs: vec![], state_dir: dir.path().to_path_buf() };
        std::fs::write(paths.config_file(), br#"{"notifications": false}"#).unwrap();
        let c = Config::load(&paths);
        assert!(!c.notifications);
        assert!(c.forecast_alerts);
        assert_eq!(c.refresh_secs, 30);
    }
}
