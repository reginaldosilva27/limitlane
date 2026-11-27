//! Decides which notifications to send, each at most once per usage window.

use crate::config::Config;
use crate::forecast::{Basis, Forecast, WindowKind};
use crate::limits::SAME_WINDOW_SECS;
use crate::paths::{write_atomic, Paths};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Fired {
    pub resets_at: i64,
    /// Highest threshold already announced for this window.
    pub threshold: u32,
    pub forecast: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlertState {
    #[serde(default)]
    pub five_hour: Option<Fired>,
    #[serde(default)]
    pub seven_day: Option<Fired>,
}

impl AlertState {
    pub fn load(paths: &Paths) -> Self {
        std::fs::read(paths.alerts_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self, paths: &Paths) -> std::io::Result<()> {
        write_atomic(&paths.alerts_file(), &serde_json::to_vec(self).unwrap_or_default())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Alert {
    pub window: WindowKind,
    pub title: String,
    pub body: String,
}

/// Human duration such as "2h 05m", "45m" or "3d 4h".
pub fn fmt_duration(secs: i64) -> String {
    let s = secs.max(0);
    let (d, h, m) = (s / 86_400, (s % 86_400) / 3600, (s % 3600) / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    }
}

/// Evaluate the forecasts against the config and update `state`. Returns the
/// alerts to deliver now.
pub fn evaluate(state: &mut AlertState, forecasts: &[Forecast], cfg: &Config, now: i64) -> Vec<Alert> {
    let mut out = Vec::new();
    if !cfg.notifications {
        return out;
    }
    for f in forecasts {
        let slot = match f.window {
            WindowKind::FiveHour => &mut state.five_hour,
            WindowKind::SevenDay => &mut state.seven_day,
        };
        let same = matches!(slot, Some(s) if (s.resets_at - f.resets_at).abs() <= SAME_WINDOW_SECS);
        if !same {
            *slot = Some(Fired { resets_at: f.resets_at, threshold: 0, forecast: false });
        }
        let fired = slot.as_mut().expect("slot was just filled");
        let label = f.window.label();
        let reset_in = fmt_duration(f.resets_at - now);

        // Announce only the highest newly crossed threshold.
        let crossed = cfg.thresholds.iter().copied().filter(|&t| f.used_pct >= t as f64).max().unwrap_or(0);
        if crossed > fired.threshold {
            fired.threshold = crossed;
            out.push(Alert {
                window: f.window,
                title: format!("Claude {label} limit at {:.0}%", f.used_pct),
                body: format!("Passed {crossed}%. Resets in {reset_in}."),
            });
        }

        // Forecast warning, once per window. Skipped very early in a window
        // (a short burst would extrapolate wildly) and once already at 100%.
        let elapsed = now - (f.resets_at - f.window.length_secs());
        let settled = (f.basis == Basis::Recent && f.used_pct >= 10.0) || elapsed >= f.window.length_secs() / 10;
        if cfg.forecast_alerts && !fired.forecast && settled && f.used_pct < 100.0 {
            if let Some(eta) = f.hits_limit_at {
                fired.forecast = true;
                out.push(Alert {
                    window: f.window,
                    title: format!("On pace to hit the {label} limit"),
                    body: format!(
                        "At {:.0}% now, about {} until 100%, which is {} before it resets.",
                        f.used_pct,
                        fmt_duration(eta - now),
                        fmt_duration(f.resets_at - eta)
                    ),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fc(used: f64, hits: Option<i64>) -> Forecast {
        Forecast {
            window: WindowKind::FiveHour,
            used_pct: used,
            resets_at: 18_000,
            seconds_to_reset: 9_000,
            basis: Basis::Recent,
            rate_pct_per_hour: 10.0,
            projected_pct_at_reset: used,
            hits_limit_at: hits,
        }
    }

    #[test]
    fn thresholds_fire_once_and_only_the_highest() {
        let cfg = Config::default();
        let mut st = AlertState::default();
        assert!(evaluate(&mut st, &[fc(40.0, None)], &cfg, 9_000).is_empty());
        // App starts at 80%: one alert (75), not two.
        let a = evaluate(&mut st, &[fc(80.0, None)], &cfg, 9_000);
        assert_eq!(a.len(), 1);
        assert!(a[0].body.contains("75%"), "{}", a[0].body);
        assert!(evaluate(&mut st, &[fc(85.0, None)], &cfg, 9_100).is_empty());
        assert_eq!(evaluate(&mut st, &[fc(91.0, None)], &cfg, 9_200).len(), 1);
        assert!(evaluate(&mut st, &[fc(99.0, None)], &cfg, 9_300).is_empty());
    }

    #[test]
    fn new_window_rearms() {
        let cfg = Config::default();
        let mut st = AlertState::default();
        assert_eq!(evaluate(&mut st, &[fc(95.0, None)], &cfg, 9_000).len(), 1);
        let mut next = fc(55.0, None);
        next.resets_at = 36_000;
        assert_eq!(evaluate(&mut st, &[next], &cfg, 27_000).len(), 1);
    }

    #[test]
    fn forecast_alert_fires_once() {
        let cfg = Config { thresholds: vec![], ..Default::default() };
        let mut st = AlertState::default();
        let a = evaluate(&mut st, &[fc(60.0, Some(11_400))], &cfg, 9_000);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].body, "At 60% now, about 40m until 100%, which is 1h 50m before it resets.");
        assert!(evaluate(&mut st, &[fc(62.0, Some(11_000))], &cfg, 9_100).is_empty());
    }

    #[test]
    fn forecast_alert_waits_for_a_settled_window() {
        let cfg = Config { thresholds: vec![], ..Default::default() };
        let mut st = AlertState::default();
        // 5 minutes into the window at 4%: too early to shout.
        let mut early = fc(4.0, Some(5_000));
        early.basis = Basis::WindowAverage;
        assert!(evaluate(&mut st, &[early], &cfg, 300).is_empty());
    }

    #[test]
    fn disabled_notifications_send_nothing() {
        let cfg = Config { notifications: false, ..Default::default() };
        let mut st = AlertState::default();
        assert!(evaluate(&mut st, &[fc(99.0, Some(9_100))], &cfg, 9_000).is_empty());
    }

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(59), "0m");
        assert_eq!(fmt_duration(45 * 60), "45m");
        assert_eq!(fmt_duration(2 * 3600 + 5 * 60), "2h 05m");
        assert_eq!(fmt_duration(3 * 86_400 + 4 * 3600 + 60), "3d 4h");
    }
}
