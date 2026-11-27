//! "At this pace, when do I hit the limit?"

use crate::limits::{Sample, Window, SAME_WINDOW_SECS};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    FiveHour,
    SevenDay,
}

impl WindowKind {
    pub fn length_secs(self) -> i64 {
        match self {
            WindowKind::FiveHour => 5 * 3600,
            WindowKind::SevenDay => 7 * 86_400,
        }
    }
    /// How far back to look when measuring the recent pace.
    fn lookback_secs(self) -> i64 {
        match self {
            WindowKind::FiveHour => 30 * 60,
            WindowKind::SevenDay => 24 * 3600,
        }
    }
    /// Minimum time span of history needed to trust a recent pace.
    fn min_span_secs(self) -> i64 {
        match self {
            WindowKind::FiveHour => 5 * 60,
            WindowKind::SevenDay => 3600,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            WindowKind::FiveHour => "5-hour",
            WindowKind::SevenDay => "weekly",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Measured over the recent lookback period.
    Recent,
    /// Average since the window opened.
    WindowAverage,
    /// Not enough information.
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct Forecast {
    pub window: WindowKind,
    pub used_pct: f64,
    pub resets_at: i64,
    pub seconds_to_reset: i64,
    pub basis: Basis,
    /// Percentage points per hour.
    pub rate_pct_per_hour: f64,
    /// Where usage lands at reset if the pace holds (can exceed 100).
    pub projected_pct_at_reset: f64,
    /// When 100% is reached, if that happens before the reset.
    pub hits_limit_at: Option<i64>,
}

impl Forecast {
    pub fn will_hit(&self) -> bool {
        self.hits_limit_at.is_some()
    }
}

/// Forecast one window. `history` may contain samples from any window; only
/// those belonging to this one are used.
pub fn forecast(kind: WindowKind, window: &Window, history: &[Sample], now: i64) -> Forecast {
    let used = window.used_percentage;
    let to_reset = (window.resets_at - now).max(0);

    let points: Vec<(i64, f64)> = history
        .iter()
        .filter_map(|s| match kind {
            WindowKind::FiveHour => Some((s.t, s.fh?, s.fr?)),
            WindowKind::SevenDay => Some((s.t, s.sd?, s.sr?)),
        })
        .filter(|(t, _, reset)| (reset - window.resets_at).abs() <= SAME_WINDOW_SECS && *t >= now - kind.lookback_secs() && *t <= now)
        .map(|(t, pct, _)| (t, pct))
        .collect();

    // Recent pace: from the oldest sample in the lookback to the current value
    // *now*. Measuring up to now makes the pace decay while you are idle.
    let recent = points.first().and_then(|&(t0, p0)| {
        let dt = now - t0;
        (dt >= kind.min_span_secs()).then(|| (used - p0).max(0.0) / (dt as f64 / 3600.0))
    });

    let elapsed = now - (window.resets_at - kind.length_secs());
    let (rate, basis) = match recent {
        Some(r) => (r, Basis::Recent),
        None if elapsed >= kind.min_span_secs() => (used / (elapsed as f64 / 3600.0), Basis::WindowAverage),
        None => (0.0, Basis::None),
    };

    let projected = used + rate * (to_reset as f64 / 3600.0);
    let hits_limit_at = if used >= 100.0 {
        Some(now)
    } else if rate > 0.0 {
        let eta = now + ((100.0 - used) / rate * 3600.0).ceil() as i64;
        (eta < window.resets_at).then_some(eta)
    } else {
        None
    };

    Forecast { window: kind, used_pct: used, resets_at: window.resets_at, seconds_to_reset: to_reset, basis, rate_pct_per_hour: rate, projected_pct_at_reset: projected, hits_limit_at }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s5(t: i64, pct: f64, reset: i64) -> Sample {
        Sample { t, fh: Some(pct), fr: Some(reset), sd: None, sr: None }
    }

    #[test]
    fn recent_pace_predicts_hit_before_reset() {
        // Window resets at t=18000 (opened at 0). Now 9000 (2.5h in), at 60%.
        // 20 minutes ago it was 40%: 60 pct/h, 40 points left: 40 minutes.
        let w = Window { used_percentage: 60.0, resets_at: 18_000 };
        let f = forecast(WindowKind::FiveHour, &w, &[s5(7_800, 40.0, 18_000), s5(8_400, 50.0, 18_000)], 9_000);
        assert_eq!(f.basis, Basis::Recent);
        assert!((f.rate_pct_per_hour - 60.0).abs() < 1e-9);
        assert_eq!(f.hits_limit_at, Some(9_000 + 2_400));
        assert!(f.will_hit());
        assert!((f.projected_pct_at_reset - 210.0).abs() < 1e-9);
    }

    #[test]
    fn slow_pace_is_safe() {
        let w = Window { used_percentage: 30.0, resets_at: 18_000 };
        let f = forecast(WindowKind::FiveHour, &w, &[s5(7_800, 28.0, 18_000)], 9_000);
        // 2 points in 20 min = 6 pct/h; 2.5h left: 45% at reset.
        assert!((f.projected_pct_at_reset - 45.0).abs() < 1e-9);
        assert!(!f.will_hit());
    }

    #[test]
    fn idle_time_decays_the_pace() {
        let w = Window { used_percentage: 60.0, resets_at: 18_000 };
        let hist = [s5(7_800, 40.0, 18_000)];
        let busy = forecast(WindowKind::FiveHour, &w, &hist, 8_400);
        let idle = forecast(WindowKind::FiveHour, &w, &hist, 9_500);
        assert!(idle.rate_pct_per_hour < busy.rate_pct_per_hour);
    }

    #[test]
    fn falls_back_to_window_average_without_history() {
        // 1h into the window at 25%: 25 pct/h, 4h left: 125% at reset, hit in 3h.
        let w = Window { used_percentage: 25.0, resets_at: 18_000 };
        let f = forecast(WindowKind::FiveHour, &w, &[], 3_600);
        assert_eq!(f.basis, Basis::WindowAverage);
        assert_eq!(f.hits_limit_at, Some(3_600 + 10_800));
        // Samples from a previous window are ignored.
        let other = forecast(WindowKind::FiveHour, &w, &[s5(3_000, 99.0, 1_000)], 3_600);
        assert_eq!(other.basis, Basis::WindowAverage);
    }

    #[test]
    fn no_basis_right_after_the_window_opens() {
        let w = Window { used_percentage: 1.0, resets_at: 18_000 };
        let f = forecast(WindowKind::FiveHour, &w, &[], 60);
        assert_eq!(f.basis, Basis::None);
        assert!(!f.will_hit());
    }

    #[test]
    fn already_at_limit() {
        let w = Window { used_percentage: 100.0, resets_at: 18_000 };
        assert_eq!(forecast(WindowKind::FiveHour, &w, &[], 9_000).hits_limit_at, Some(9_000));
    }

    #[test]
    fn weekly_uses_its_own_fields() {
        let reset = 7 * 86_400;
        let w = Window { used_percentage: 50.0, resets_at: reset };
        let hist = [Sample { t: 3 * 86_400, fh: Some(90.0), fr: Some(1), sd: Some(30.0), sr: Some(reset) }];
        // Day 4 now: +20 points in 24h, 3 days left: 110% at reset.
        let f = forecast(WindowKind::SevenDay, &w, &hist, 4 * 86_400);
        assert_eq!(f.basis, Basis::Recent);
        assert!((f.projected_pct_at_reset - 110.0).abs() < 1e-6);
        assert!(f.will_hit());
    }
}
