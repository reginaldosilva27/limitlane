//! Rate-limit and context data captured from Claude Code's statusline hook.
//!
//! Claude Code pipes a JSON document to the configured statusline command on
//! every render. For Pro/Max subscribers it includes the real 5-hour and
//! 7-day usage percentages, which is the only credential-free source for
//! them. The hook stores what it sees here; the app and CLI read it back.

use crate::paths::{write_atomic, Paths};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;

// ---------- statusline input (only the fields we use) ----------

#[derive(Debug, Default, Deserialize)]
pub struct StatuslineInput {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub session_name: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub model: Option<InModel>,
    #[serde(default)]
    pub workspace: Option<InWorkspace>,
    #[serde(default)]
    pub cost: Option<InCost>,
    #[serde(default)]
    pub context_window: Option<InContext>,
    #[serde(default)]
    pub rate_limits: Option<InRateLimits>,
}

#[derive(Debug, Default, Deserialize)]
pub struct InModel {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct InWorkspace {
    #[serde(default)]
    pub current_dir: Option<String>,
    #[serde(default)]
    pub project_dir: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct InCost {
    #[serde(default)]
    pub total_cost_usd: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct InContext {
    #[serde(default)]
    pub total_input_tokens: Option<u64>,
    #[serde(default)]
    pub context_window_size: Option<u64>,
    #[serde(default)]
    pub used_percentage: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct InRateLimits {
    #[serde(default)]
    pub five_hour: Option<Window>,
    #[serde(default)]
    pub seven_day: Option<Window>,
    #[serde(default)]
    pub spend_limit: Option<Spend>,
    /// Any other window Claude Code reports (for example a per-model weekly
    /// limit). Kept generically so new windows show up without a release.
    #[serde(flatten)]
    pub other: BTreeMap<String, serde_json::Value>,
}

// ---------- stored state ----------

/// One usage window as reported by Claude Code.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Window {
    /// 0 to 100.
    pub used_percentage: f64,
    /// Unix epoch seconds.
    pub resets_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spend {
    pub used_percentage: f64,
    pub resets_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LimitsSnapshot {
    /// When a limit value was last received from Claude Code.
    pub captured_at: i64,
    #[serde(default)]
    pub five_hour: Option<Window>,
    #[serde(default)]
    pub seven_day: Option<Window>,
    #[serde(default)]
    pub spend_limit: Option<Spend>,
    /// Windows other than the two standard ones, keyed by Claude Code's name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Window>,
    #[serde(default)]
    pub claude_code_version: Option<String>,
    /// When the last history sample was appended (hook bookkeeping).
    #[serde(default)]
    pub last_sample_at: i64,
}

impl LimitsSnapshot {
    pub fn is_empty(&self) -> bool {
        self.five_hour.is_none() && self.seven_day.is_none() && self.spend_limit.is_none() && self.extra.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub captured_at: i64,
    #[serde(default)]
    pub session_name: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub project_dir: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub context_used_pct: Option<f64>,
    #[serde(default)]
    pub context_tokens: Option<u64>,
    #[serde(default)]
    pub context_size: Option<u64>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

/// One point of limit history, used by the forecast.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub t: i64,
    /// 5-hour used percentage and its reset time.
    #[serde(default)]
    pub fh: Option<f64>,
    #[serde(default)]
    pub fr: Option<i64>,
    /// 7-day used percentage and its reset time.
    #[serde(default)]
    pub sd: Option<f64>,
    #[serde(default)]
    pub sr: Option<i64>,
}

/// Two reports describe the same window when their reset times agree within
/// this tolerance.
pub const SAME_WINDOW_SECS: i64 = 120;
const SAMPLE_EVERY_SECS: i64 = 300;
const HISTORY_KEEP_SECS: i64 = 8 * 86_400;
const HISTORY_MAX_BYTES: u64 = 512 * 1024;

/// Merge an incoming window into the stored one.
///
/// Several Claude Code sessions report independently and an idle one can hold
/// an older value. Within one window usage only grows, so the higher
/// percentage wins; a later reset time means a new window started.
fn merge_window(old: Option<Window>, new: Option<Window>, now: i64) -> Option<Window> {
    let old = old.filter(|w| w.resets_at > now);
    let new = new.filter(|w| w.resets_at > now);
    match (old, new) {
        (Some(o), Some(n)) => {
            if (o.resets_at - n.resets_at).abs() <= SAME_WINDOW_SECS {
                Some(if n.used_percentage >= o.used_percentage { n } else { o })
            } else if n.resets_at > o.resets_at {
                Some(n)
            } else {
                Some(o)
            }
        }
        (o, n) => n.or(o),
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &std::path::Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Store what one statusline invocation told us. Called by the hook.
pub fn record(paths: &Paths, input: &StatuslineInput, now: i64) -> std::io::Result<()> {
    // --- per-session context snapshot ---
    if let Some(id) = input.session_id.as_deref().filter(|s| is_safe_id(s)) {
        let ctx = input.context_window.as_ref();
        let snap = SessionSnapshot {
            session_id: id.to_string(),
            captured_at: now,
            session_name: input.session_name.clone(),
            cwd: input.workspace.as_ref().and_then(|w| w.current_dir.clone()).or_else(|| input.cwd.clone()),
            project_dir: input.workspace.as_ref().and_then(|w| w.project_dir.clone()),
            model_id: input.model.as_ref().and_then(|m| m.id.clone()),
            model_name: input.model.as_ref().and_then(|m| m.display_name.clone()),
            context_used_pct: ctx.and_then(|c| c.used_percentage),
            context_tokens: ctx.and_then(|c| c.total_input_tokens),
            context_size: ctx.and_then(|c| c.context_window_size),
            cost_usd: input.cost.as_ref().and_then(|c| c.total_cost_usd),
        };
        let body = serde_json::to_vec(&snap).unwrap_or_default();
        write_atomic(&paths.sessions_dir().join(format!("{id}.json")), &body)?;
    }

    // --- account-wide limits ---
    let Some(rl) = input.rate_limits.as_ref() else { return Ok(()) };
    let prev: LimitsSnapshot = read_json(&paths.limits_file()).unwrap_or_default();
    let mut extra: BTreeMap<String, Window> = prev.extra.iter().filter(|(_, w)| w.resets_at > now).map(|(k, w)| (k.clone(), *w)).collect();
    for (key, value) in &rl.other {
        if let Ok(w) = serde_json::from_value::<Window>(value.clone()) {
            if let Some(merged) = merge_window(extra.get(key).copied(), Some(w), now) {
                extra.insert(key.clone(), merged);
            }
        }
    }
    let mut next = LimitsSnapshot {
        captured_at: now,
        extra,
        five_hour: merge_window(prev.five_hour, rl.five_hour, now),
        seven_day: merge_window(prev.seven_day, rl.seven_day, now),
        spend_limit: rl.spend_limit.clone().or(prev.spend_limit.clone()).filter(|s| s.resets_at > now),
        claude_code_version: input.version.clone().or(prev.claude_code_version.clone()),
        last_sample_at: prev.last_sample_at,
    };
    if next.is_empty() {
        return Ok(());
    }

    let changed = next.five_hour != prev.five_hour || next.seven_day != prev.seven_day;
    if changed || now - prev.last_sample_at >= SAMPLE_EVERY_SECS {
        let sample = Sample {
            t: now,
            fh: next.five_hour.map(|w| w.used_percentage),
            fr: next.five_hour.map(|w| w.resets_at),
            sd: next.seven_day.map(|w| w.used_percentage),
            sr: next.seven_day.map(|w| w.resets_at),
        };
        append_sample(paths, &sample, now)?;
        next.last_sample_at = now;
    }
    write_atomic(&paths.limits_file(), &serde_json::to_vec(&next).unwrap_or_default())
}

fn is_safe_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn append_sample(paths: &Paths, sample: &Sample, now: i64) -> std::io::Result<()> {
    let path = paths.history_file();
    std::fs::create_dir_all(&paths.state_dir)?;
    if std::fs::metadata(&path).map(|m| m.len() > HISTORY_MAX_BYTES).unwrap_or(false) {
        let kept: Vec<String> = load_history(paths, now - HISTORY_KEEP_SECS).iter().filter_map(|s| serde_json::to_string(s).ok()).collect();
        let mut body = kept.join("\n");
        body.push('\n');
        write_atomic(&path, body.as_bytes())?;
    }
    let mut line = serde_json::to_string(sample).unwrap_or_default();
    line.push('\n');
    std::fs::OpenOptions::new().create(true).append(true).open(path)?.write_all(line.as_bytes())
}

/// The stored limits with expired windows dropped. `None` when nothing usable.
pub fn load_limits(paths: &Paths, now: i64) -> Option<LimitsSnapshot> {
    let mut snap: LimitsSnapshot = read_json(&paths.limits_file())?;
    snap.five_hour = snap.five_hour.filter(|w| w.resets_at > now);
    snap.seven_day = snap.seven_day.filter(|w| w.resets_at > now);
    snap.spend_limit = snap.spend_limit.filter(|s| s.resets_at > now);
    snap.extra.retain(|_, w| w.resets_at > now);
    (!snap.is_empty()).then_some(snap)
}

pub fn load_history(paths: &Paths, since: i64) -> Vec<Sample> {
    let Ok(text) = std::fs::read_to_string(paths.history_file()) else { return Vec::new() };
    text.lines().filter_map(|l| serde_json::from_str::<Sample>(l).ok()).filter(|s| s.t >= since).collect()
}

/// Session snapshots seen within `max_age` seconds, newest first. Snapshots
/// older than a week are deleted along the way.
pub fn load_sessions(paths: &Paths, now: i64, max_age: i64) -> Vec<SessionSnapshot> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir(paths.sessions_dir()) else { return out };
    for item in dir.flatten() {
        let path = item.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match read_json::<SessionSnapshot>(&path) {
            Some(s) if now - s.captured_at <= max_age => out.push(s),
            Some(s) if now - s.captured_at > 7 * 86_400 => {
                let _ = std::fs::remove_file(&path);
            }
            _ => {}
        }
    }
    out.sort_by(|a, b| b.captured_at.cmp(&a.captured_at));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn paths(dir: &std::path::Path) -> Paths {
        Paths { claude_dirs: vec![dir.join("claude")], state_dir: dir.join("state") }
    }

    fn input(json: &str) -> StatuslineInput {
        serde_json::from_str(json).unwrap()
    }

    const FULL: &str = r#"{
      "cwd": "/w/app", "session_id": "abc-123", "session_name": "refactor",
      "transcript_path": "/x.jsonl",
      "model": {"id": "claude-opus-5-5", "display_name": "Opus"},
      "workspace": {"current_dir": "/w/app/src", "project_dir": "/w/app", "added_dirs": []},
      "version": "2.1.290",
      "cost": {"total_cost_usd": 1.25, "total_duration_ms": 45000},
      "context_window": {"total_input_tokens": 50000, "total_output_tokens": 1200, "context_window_size": 200000, "used_percentage": 25, "current_usage": null},
      "rate_limits": {"five_hour": {"used_percentage": 23.5, "resets_at": 20000}, "seven_day": {"used_percentage": 41.2, "resets_at": 500000}},
      "vim": {"mode": "NORMAL"}
    }"#;

    #[test]
    fn records_limits_session_and_history() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        record(&p, &input(FULL), 10_000).unwrap();

        let l = load_limits(&p, 10_001).unwrap();
        assert_eq!(l.five_hour, Some(Window { used_percentage: 23.5, resets_at: 20_000 }));
        assert_eq!(l.seven_day.unwrap().used_percentage, 41.2);
        assert_eq!(l.claude_code_version.as_deref(), Some("2.1.290"));

        let s = load_sessions(&p, 10_001, 3600);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].context_used_pct, Some(25.0));
        assert_eq!(s[0].context_size, Some(200_000));
        assert_eq!(s[0].project_dir.as_deref(), Some("/w/app"));
        assert_eq!(s[0].model_name.as_deref(), Some("Opus"));

        assert_eq!(load_history(&p, 0).len(), 1);
    }

    #[test]
    fn missing_rate_limits_do_not_wipe_state() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        record(&p, &input(FULL), 10_000).unwrap();
        // A brand-new session has no rate_limits before its first response.
        record(&p, &input(r#"{"session_id":"new-1","context_window":{"used_percentage":null}}"#), 10_050).unwrap();
        assert_eq!(load_limits(&p, 10_060).unwrap().five_hour.unwrap().used_percentage, 23.5);
        assert_eq!(load_sessions(&p, 10_060, 3600).len(), 2);
    }

    #[test]
    fn stale_session_cannot_lower_usage_but_new_window_resets_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        record(&p, &input(FULL), 10_000).unwrap();
        let lower = r#"{"rate_limits":{"five_hour":{"used_percentage":10,"resets_at":20030},"seven_day":{"used_percentage":45,"resets_at":500000}}}"#;
        record(&p, &input(lower), 10_100).unwrap();
        let l = load_limits(&p, 10_101).unwrap();
        assert_eq!(l.five_hour.unwrap().used_percentage, 23.5, "same window keeps the higher value");
        assert_eq!(l.seven_day.unwrap().used_percentage, 45.0);

        let fresh = r#"{"rate_limits":{"five_hour":{"used_percentage":2,"resets_at":38000}}}"#;
        record(&p, &input(fresh), 20_500).unwrap();
        let l = load_limits(&p, 20_501).unwrap();
        assert_eq!(l.five_hour, Some(Window { used_percentage: 2.0, resets_at: 38_000 }));
        assert_eq!(l.seven_day.unwrap().used_percentage, 45.0, "window absent from input is kept");
    }

    #[test]
    fn unknown_windows_are_captured() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        let json = r#"{"rate_limits":{"five_hour":{"used_percentage":5,"resets_at":20000},
            "seven_day_fable":{"used_percentage":61.5,"resets_at":500000},
            "something_else":{"enabled":true}}}"#;
        record(&p, &input(json), 10_000).unwrap();
        let l = load_limits(&p, 10_001).unwrap();
        assert_eq!(l.extra.len(), 1);
        assert_eq!(l.extra["seven_day_fable"], Window { used_percentage: 61.5, resets_at: 500_000 });
        // A later report without it keeps it until it expires.
        record(&p, &input(r#"{"rate_limits":{"five_hour":{"used_percentage":6,"resets_at":20000}}}"#), 10_100).unwrap();
        assert_eq!(load_limits(&p, 10_101).unwrap().extra.len(), 1);
        assert!(load_limits(&p, 15_000).unwrap().extra.len() == 1);
        assert!(load_limits(&p, 600_000).is_none());
    }

    #[test]
    fn expired_windows_are_dropped_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        record(&p, &input(FULL), 10_000).unwrap();
        let l = load_limits(&p, 30_000).unwrap();
        assert!(l.five_hour.is_none());
        assert!(l.seven_day.is_some());
        assert!(load_limits(&p, 600_000).is_none());
    }

    #[test]
    fn history_is_throttled_when_nothing_changes() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        for t in [10_000, 10_010, 10_020, 10_299] {
            record(&p, &input(FULL), t).unwrap();
        }
        assert_eq!(load_history(&p, 0).len(), 1);
        record(&p, &input(FULL), 10_300).unwrap();
        assert_eq!(load_history(&p, 0).len(), 2);
        let bumped = FULL.replace("23.5", "24");
        record(&p, &input(&bumped), 10_310).unwrap();
        assert_eq!(load_history(&p, 0).len(), 3);
    }

    #[test]
    fn unsafe_session_ids_are_not_used_as_file_names() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        record(&p, &input(r#"{"session_id":"../../evil"}"#), 1).unwrap();
        assert!(load_sessions(&p, 2, 100).is_empty());
        assert!(!dir.path().join("evil.json").exists());
    }
}
