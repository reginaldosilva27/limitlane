//! The single document the menu bar app and the CLI render.

use crate::aggregate::{self, BlockEstimate, Burn, Report, Totals};
use crate::config::{Config, MenuBarStyle};
use crate::forecast::{forecast, Forecast, WindowKind};
use crate::hook::{self, HookStatus};
use crate::limits::{self, LimitsSnapshot};
use crate::paths::Paths;
use crate::pricing::Pricing;
use crate::transcript::{Entry, Scanner};
use chrono::FixedOffset;
use serde::Serialize;

/// Context window state of one Claude Code session.
#[derive(Debug, Clone, Serialize)]
pub struct SessionView {
    pub session_id: String,
    pub name: String,
    pub project: String,
    pub model: Option<String>,
    /// 0 to 100; only known when the hook reported it.
    pub context_used_pct: Option<f64>,
    pub context_tokens: Option<u64>,
    pub context_size: Option<u64>,
    /// Session cost as estimated by Claude Code itself.
    pub cost_usd: Option<f64>,
    pub last_seen: i64,
    /// `hook` (reported by Claude Code) or `transcript` (derived from logs).
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub generated_at: i64,
    pub hook: HookStatus,
    /// Real limits from Claude Code, when the hook has reported them.
    pub limits: Option<LimitsSnapshot>,
    /// Seconds since Claude Code last reported limits.
    pub limits_age_secs: Option<i64>,
    pub forecasts: Vec<Forecast>,
    pub burn: Burn,
    /// Heuristic 5-hour window, only filled when `limits` has no 5-hour data.
    pub block_estimate: Option<BlockEstimate>,
    pub today: Totals,
    pub report: Report,
    pub sessions: Vec<SessionView>,
    pub pricing_as_of: String,
}

impl Summary {
    pub fn five_hour_pct(&self) -> Option<f64> {
        self.limits.as_ref().and_then(|l| l.five_hour).map(|w| w.used_percentage)
    }
    pub fn seven_day_pct(&self) -> Option<f64> {
        self.limits.as_ref().and_then(|l| l.seven_day).map(|w| w.used_percentage)
    }

    /// Text shown next to the menu bar icon.
    pub fn menu_bar_title(&self, style: MenuBarStyle) -> String {
        let pct = |v: Option<f64>| v.map(|p| format!("{p:.0}%")).unwrap_or_else(|| "–".into());
        match style {
            MenuBarStyle::IconOnly => String::new(),
            MenuBarStyle::FiveHour => pct(self.five_hour_pct()),
            MenuBarStyle::SevenDay => pct(self.seven_day_pct()),
            MenuBarStyle::Both => match (self.five_hour_pct(), self.seven_day_pct()) {
                (None, None) => fmt_tokens(self.today.total_tokens),
                (a, b) => format!("{} · {}", pct(a), pct(b)),
            },
        }
    }
}

/// Compact token count: `950`, `12.3K`, `4.56M`, `1.20B`.
pub fn fmt_tokens(n: u64) -> String {
    let f = n as f64;
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}K", f / 1e3)
    } else if n < 1_000_000_000 {
        format!("{:.2}M", f / 1e6)
    } else {
        format!("{:.2}B", f / 1e9)
    }
}

const SCAN_DAYS: u32 = 30;
const SESSION_MAX_AGE: i64 = 6 * 3600;
const HISTORY_LOOKBACK: i64 = 26 * 3600;

/// Holds the scan cache between refreshes.
pub struct Engine {
    pub paths: Paths,
    scanner: Scanner,
}

impl Engine {
    pub fn new(paths: Paths) -> Self {
        Self { paths, scanner: Scanner::new() }
    }

    /// Deduplicated entries for the last `days` local days.
    pub fn entries(&mut self, now: i64, days: u32, offset: FixedOffset) -> Vec<Entry> {
        let since = aggregate::range_start(now, days, offset);
        self.scanner.scan(&self.paths.project_roots(), since)
    }

    /// Build the summary with a report covering the last `range_days` days.
    pub fn summary(&mut self, range_days: u32, now: i64, offset: FixedOffset) -> Summary {
        let config = Config::load(&self.paths);
        let pricing = Pricing::with_overrides(&config.pricing_overrides);
        let range_days = range_days.clamp(1, 365);
        let entries = self.entries(now, range_days.max(SCAN_DAYS), offset);

        let limits = limits::load_limits(&self.paths, now);
        let history = limits::load_history(&self.paths, now - HISTORY_LOOKBACK);
        let mut forecasts = Vec::new();
        if let Some(l) = &limits {
            if let Some(w) = &l.five_hour {
                forecasts.push(forecast(WindowKind::FiveHour, w, &history, now));
            }
            if let Some(w) = &l.seven_day {
                forecasts.push(forecast(WindowKind::SevenDay, w, &history, now));
            }
        }
        let has_five_hour = limits.as_ref().is_some_and(|l| l.five_hour.is_some());
        let block_estimate = if has_five_hour { None } else { aggregate::estimate_block(&entries, &pricing, now) };

        let today = aggregate::build_report(&entries, &pricing, now, 1, offset).totals;
        let report = aggregate::build_report(&entries, &pricing, now, range_days, offset);

        Summary {
            generated_at: now,
            hook: hook::status(&self.paths),
            limits_age_secs: limits.as_ref().map(|l| now - l.captured_at),
            limits,
            forecasts,
            burn: aggregate::burn(&entries, &pricing, now, 30),
            block_estimate,
            today,
            report,
            sessions: self.sessions(&entries, now),
            pricing_as_of: pricing.as_of.clone(),
        }
    }

    fn sessions(&self, entries: &[Entry], now: i64) -> Vec<SessionView> {
        let mut out: Vec<SessionView> = limits::load_sessions(&self.paths, now, SESSION_MAX_AGE)
            .into_iter()
            .map(|s| {
                let project = s.project_dir.or(s.cwd).unwrap_or_default();
                SessionView {
                    name: s.session_name.filter(|n| !n.is_empty()).unwrap_or_else(|| aggregate::display_name(&project)),
                    session_id: s.session_id,
                    project,
                    model: s.model_name.or(s.model_id),
                    context_used_pct: s.context_used_pct,
                    context_tokens: s.context_tokens,
                    context_size: s.context_size,
                    cost_usd: s.cost_usd,
                    last_seen: s.captured_at,
                    source: "hook",
                }
            })
            .collect();

        // Sessions the hook has not seen: the last main-thread response tells
        // us how many tokens sit in the context, but not the window size.
        let mut latest: std::collections::HashMap<&str, &Entry> = std::collections::HashMap::new();
        for e in entries.iter().filter(|e| !e.is_sidechain && e.ts > now - SESSION_MAX_AGE && e.ts <= now) {
            latest.insert(e.session_id.as_str(), e);
        }
        for (id, e) in latest {
            if out.iter().any(|s| s.session_id == id) {
                continue;
            }
            out.push(SessionView {
                session_id: id.to_string(),
                name: aggregate::display_name(&e.project),
                project: e.project.clone(),
                model: Some(e.model.clone()),
                context_used_pct: None,
                context_tokens: Some(e.usage.prompt_tokens()),
                context_size: None,
                cost_usd: None,
                last_seen: e.ts,
                source: "transcript",
            });
        }
        out.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then(a.session_id.cmp(&b.session_id)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::StatuslineInput;
    use crate::transcript::testutil::line;

    fn ts(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()
    }

    #[test]
    fn end_to_end_summary() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        let proj = claude.join("projects").join("-w-alpha");
        std::fs::create_dir_all(&proj).unwrap();
        let now = crate::now();
        let iso = |t: i64| chrono::DateTime::from_timestamp(t, 0).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let body = [
            line(&iso(now - 600), "sess-a", "/w/alpha", "claude-opus-5-5", "m1", "r1", 1_000, 2_000, 100_000, 0),
            line(&iso(now - 300), "sess-a", "/w/alpha", "claude-opus-5-5", "m2", "r2", 1_000, 2_000, 150_000, 0),
            line(&iso(now - 200), "sess-b", "/w/beta", "claude-sonnet-5-5", "m3", "r3", 500, 500, 0, 0),
        ]
        .join("\n");
        std::fs::write(proj.join("sess-a.jsonl"), body).unwrap();

        let paths = Paths { claude_dirs: vec![claude], state_dir: dir.path().join("state") };
        let offset = FixedOffset::east_opt(0).unwrap();
        let mut engine = Engine::new(paths.clone());

        // Before the hook reports anything: estimate only.
        let s = engine.summary(7, now, offset);
        assert!(s.limits.is_none());
        assert!(s.forecasts.is_empty());
        assert_eq!(s.hook, HookStatus::NotInstalled);
        assert_eq!(s.report.totals.messages, 3);
        assert_eq!(s.report.by_day.len(), 7);
        assert_eq!(s.sessions.len(), 2);
        assert_eq!(s.sessions.iter().find(|x| x.session_id == "sess-a").unwrap().context_tokens, Some(151_000));
        assert!(s.sessions.iter().all(|x| x.source == "transcript" && x.context_used_pct.is_none()));
        assert_eq!(s.menu_bar_title(MenuBarStyle::Both), fmt_tokens(s.today.total_tokens));
        assert!(s.burn.tokens > 0);

        // After the hook reports limits and context for sess-a.
        let input: StatuslineInput = serde_json::from_str(&format!(
            r#"{{"session_id":"sess-a","workspace":{{"project_dir":"/w/alpha"}},"model":{{"display_name":"Opus"}},
               "context_window":{{"used_percentage":75.5,"total_input_tokens":151000,"context_window_size":200000}},
               "rate_limits":{{"five_hour":{{"used_percentage":42.4,"resets_at":{}}},"seven_day":{{"used_percentage":18,"resets_at":{}}}}}}}"#,
            now + 7_200,
            now + 300_000
        ))
        .unwrap();
        limits::record(&paths, &input, now - 30).unwrap();

        let s = engine.summary(1, now, offset);
        assert_eq!(s.five_hour_pct(), Some(42.4));
        assert_eq!(s.forecasts.len(), 2);
        assert_eq!(s.limits_age_secs, Some(30));
        assert!(s.block_estimate.is_none());
        assert_eq!(s.menu_bar_title(MenuBarStyle::Both), "42% · 18%");
        assert_eq!(s.menu_bar_title(MenuBarStyle::SevenDay), "18%");
        assert_eq!(s.menu_bar_title(MenuBarStyle::IconOnly), "");
        let a = s.sessions.iter().find(|x| x.session_id == "sess-a").unwrap();
        assert_eq!((a.source, a.context_used_pct, a.name.as_str()), ("hook", Some(75.5), "alpha"));
        assert_eq!(s.sessions.len(), 2);

        // The summary is plain JSON for the UI and `--json`.
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["forecasts"][0]["window"], "five_hour");
        assert_eq!(json["hook"]["state"], "not_installed");
        let _ = ts("2026-01-01T00:00:00Z");
    }

    #[test]
    fn token_formatting() {
        assert_eq!(fmt_tokens(950), "950");
        assert_eq!(fmt_tokens(12_340), "12.3K");
        assert_eq!(fmt_tokens(4_560_000), "4.56M");
        assert_eq!(fmt_tokens(1_200_000_000), "1.20B");
    }
}
