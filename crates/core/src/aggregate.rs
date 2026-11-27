//! Roll transcript entries up by project, model, day and session.

use crate::pricing::Pricing;
use crate::transcript::Entry;
use chrono::{FixedOffset, TimeZone};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct Totals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
    /// API-equivalent cost in USD for the priced part.
    pub cost_usd: f64,
    /// Tokens from models missing in the pricing table (not in `cost_usd`).
    pub unpriced_tokens: u64,
    pub messages: u64,
}

impl Totals {
    pub fn add(&mut self, e: &Entry, pricing: &Pricing) {
        self.input += e.usage.input;
        self.output += e.usage.output;
        self.cache_read += e.usage.cache_read;
        self.cache_write += e.usage.cache_write();
        self.total_tokens += e.usage.total();
        self.messages += 1;
        match pricing.cost(&e.model, &e.usage) {
            Some(c) => self.cost_usd += c,
            None => self.unpriced_tokens += e.usage.total(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectRow {
    /// Full working directory.
    pub project: String,
    /// Last path component, for display.
    pub name: String,
    pub totals: Totals,
    pub sessions: usize,
    pub last_active: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelRow {
    pub model: String,
    pub totals: Totals,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayRow {
    /// Local calendar date, `YYYY-MM-DD`.
    pub date: String,
    pub totals: Totals,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    pub session_id: String,
    pub project: String,
    pub name: String,
    pub totals: Totals,
    pub first_active: i64,
    pub last_active: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub since: i64,
    pub until: i64,
    pub days: u32,
    pub totals: Totals,
    pub by_project: Vec<ProjectRow>,
    pub by_model: Vec<ModelRow>,
    /// One row per local day in the range, oldest first, including empty days.
    pub by_day: Vec<DayRow>,
    /// Sessions sorted by cost, highest first.
    pub by_session: Vec<SessionRow>,
}

pub fn display_name(project: &str) -> String {
    let trimmed = project.trim_end_matches('/');
    trimmed.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(trimmed).to_string()
}

/// Start (epoch seconds) of the local day that is `days - 1` days before the
/// day containing `now`. `days = 1` means "today".
pub fn range_start(now: i64, days: u32, offset: FixedOffset) -> i64 {
    let local = offset.timestamp_opt(now, 0).single().expect("valid timestamp");
    let midnight = local.date_naive().and_hms_opt(0, 0, 0).expect("midnight exists");
    let start = offset.from_local_datetime(&midnight).single().expect("fixed offset is unambiguous");
    start.timestamp() - (days.max(1) as i64 - 1) * 86_400
}

fn local_date(ts: i64, offset: FixedOffset) -> String {
    offset.timestamp_opt(ts, 0).single().map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

fn by_cost_then_tokens(a: &Totals, b: &Totals) -> std::cmp::Ordering {
    b.cost_usd.partial_cmp(&a.cost_usd).unwrap_or(std::cmp::Ordering::Equal).then(b.total_tokens.cmp(&a.total_tokens))
}

/// Build the report for the last `days` local days ending at `now`.
pub fn build_report(entries: &[Entry], pricing: &Pricing, now: i64, days: u32, offset: FixedOffset) -> Report {
    let since = range_start(now, days, offset);
    let mut totals = Totals::default();
    let mut projects: BTreeMap<&str, (Totals, std::collections::BTreeSet<&str>, i64)> = BTreeMap::new();
    let mut models: BTreeMap<&str, Totals> = BTreeMap::new();
    let mut day_map: BTreeMap<String, Totals> = BTreeMap::new();
    let mut sessions: BTreeMap<&str, (Totals, &str, i64, i64)> = BTreeMap::new();

    for d in 0..days.max(1) as i64 {
        // +12h keeps us safely inside each local day.
        day_map.insert(local_date(since + d * 86_400 + 43_200, offset), Totals::default());
    }

    for e in entries.iter().filter(|e| e.ts >= since && e.ts <= now) {
        totals.add(e, pricing);
        let p = projects.entry(e.project.as_str()).or_insert_with(|| (Totals::default(), Default::default(), 0));
        p.0.add(e, pricing);
        p.1.insert(e.session_id.as_str());
        p.2 = p.2.max(e.ts);
        models.entry(e.model.as_str()).or_default().add(e, pricing);
        day_map.entry(local_date(e.ts, offset)).or_default().add(e, pricing);
        let s = sessions.entry(e.session_id.as_str()).or_insert_with(|| (Totals::default(), e.project.as_str(), e.ts, e.ts));
        s.0.add(e, pricing);
        s.2 = s.2.min(e.ts);
        s.3 = s.3.max(e.ts);
    }

    let mut by_project: Vec<ProjectRow> = projects
        .into_iter()
        .map(|(project, (t, sess, last))| ProjectRow { project: project.to_string(), name: display_name(project), totals: t, sessions: sess.len(), last_active: last })
        .collect();
    by_project.sort_by(|a, b| by_cost_then_tokens(&a.totals, &b.totals));

    let mut by_model: Vec<ModelRow> = models.into_iter().map(|(m, t)| ModelRow { model: m.to_string(), totals: t }).collect();
    by_model.sort_by(|a, b| by_cost_then_tokens(&a.totals, &b.totals));

    let by_day = day_map.into_iter().map(|(date, totals)| DayRow { date, totals }).collect();

    let mut by_session: Vec<SessionRow> = sessions
        .into_iter()
        .map(|(id, (t, project, first, last))| SessionRow {
            session_id: id.to_string(),
            project: project.to_string(),
            name: display_name(project),
            totals: t,
            first_active: first,
            last_active: last,
        })
        .collect();
    by_session.sort_by(|a, b| by_cost_then_tokens(&a.totals, &b.totals));

    Report { since, until: now, days: days.max(1), totals, by_project, by_model, by_day, by_session }
}

/// Model family for display: `claude-fable-5-2` and `claude-3-5-sonnet` become
/// `Fable` and `Sonnet`.
pub fn model_family(model: &str) -> String {
    let m = model.to_ascii_lowercase();
    for known in ["fable", "mythos", "opus", "sonnet", "haiku"] {
        if m.contains(known) {
            let mut c = known.chars();
            return c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default();
        }
    }
    m.trim_start_matches("claude-").split('-').next().unwrap_or("other").to_string()
}

/// One model family's part of the usage in a period.
#[derive(Debug, Clone, Serialize)]
pub struct FamilyShare {
    pub family: String,
    pub totals: Totals,
    /// 0 to 100, by API-equivalent cost (by tokens when nothing is priced).
    pub share_pct: f64,
}

/// Usage per model family between `since` and `now`, largest first.
pub fn family_shares(entries: &[Entry], pricing: &Pricing, since: i64, now: i64) -> Vec<FamilyShare> {
    let mut map: BTreeMap<String, Totals> = BTreeMap::new();
    for e in entries.iter().filter(|e| e.ts >= since && e.ts <= now) {
        map.entry(model_family(&e.model)).or_default().add(e, pricing);
    }
    let cost: f64 = map.values().map(|t| t.cost_usd).sum();
    let tokens: u64 = map.values().map(|t| t.total_tokens).sum();
    let mut out: Vec<FamilyShare> = map
        .into_iter()
        .map(|(family, totals)| {
            let share_pct = if cost > 0.0 {
                totals.cost_usd / cost * 100.0
            } else if tokens > 0 {
                totals.total_tokens as f64 / tokens as f64 * 100.0
            } else {
                0.0
            };
            FamilyShare { family, totals, share_pct }
        })
        .collect();
    out.sort_by(|a, b| b.share_pct.partial_cmp(&a.share_pct).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Recent pace from transcripts, independent of any limit data.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Burn {
    pub window_minutes: u32,
    pub tokens_per_min: f64,
    pub cost_per_hour: f64,
    pub tokens: u64,
}

pub fn burn(entries: &[Entry], pricing: &Pricing, now: i64, window_minutes: u32) -> Burn {
    let since = now - window_minutes as i64 * 60;
    let mut t = Totals::default();
    for e in entries.iter().filter(|e| e.ts > since && e.ts <= now) {
        t.add(e, pricing);
    }
    Burn {
        window_minutes,
        tokens_per_min: t.total_tokens as f64 / window_minutes.max(1) as f64,
        cost_per_hour: t.cost_usd * 60.0 / window_minutes.max(1) as f64,
        tokens: t.total_tokens,
    }
}

/// A guess at the current 5-hour session window when no limit data is
/// available: it starts at the first message after a gap of five hours or
/// more, floored to the hour. This is a heuristic, not what Anthropic enforces.
#[derive(Debug, Clone, Serialize)]
pub struct BlockEstimate {
    pub start: i64,
    pub end: i64,
    pub totals: Totals,
}

pub const FIVE_HOURS: i64 = 5 * 3600;
pub const SEVEN_DAYS: i64 = 7 * 86_400;

pub fn estimate_block(entries: &[Entry], pricing: &Pricing, now: i64) -> Option<BlockEstimate> {
    let mut start: Option<i64> = None;
    let mut totals = Totals::default();
    let mut last = 0i64;
    for e in entries.iter().filter(|e| e.ts <= now) {
        let expired = match start {
            Some(s) => e.ts >= s + FIVE_HOURS || e.ts - last >= FIVE_HOURS,
            None => true,
        };
        if expired {
            start = Some(e.ts - e.ts.rem_euclid(3600));
            totals = Totals::default();
        }
        totals.add(e, pricing);
        last = e.ts;
    }
    let start = start?;
    (now < start + FIVE_HOURS).then_some(BlockEstimate { start, end: start + FIVE_HOURS, totals })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::Usage;

    fn e(ts: &str, session: &str, project: &str, model: &str, input: u64, output: u64) -> Entry {
        Entry {
            ts: chrono::DateTime::parse_from_rfc3339(ts).unwrap().timestamp(),
            session_id: session.into(),
            project: project.into(),
            model: model.into(),
            usage: Usage { input, output, ..Default::default() },
            is_sidechain: false,
            key: None,
        }
    }

    fn brt() -> FixedOffset {
        FixedOffset::west_opt(3 * 3600).unwrap()
    }

    #[test]
    fn range_start_is_local_midnight() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-26T01:30:00Z").unwrap().timestamp();
        // 01:30 UTC is still Nov 25, 22:30 in Sao Paulo.
        let today = range_start(now, 1, brt());
        assert_eq!(today, chrono::DateTime::parse_from_rfc3339("2026-11-25T03:00:00Z").unwrap().timestamp());
        assert_eq!(range_start(now, 7, brt()), today - 6 * 86_400);
    }

    #[test]
    fn report_groups_and_sorts() {
        let p = Pricing::builtin();
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-26T18:00:00Z").unwrap().timestamp();
        let entries = vec![
            e("2026-11-25T02:00:00Z", "s0", "/w/old", "claude-opus-5-5", 1_000_000, 0), // Nov 24 local
            e("2026-11-25T15:00:00Z", "s1", "/w/alpha", "claude-opus-5-5", 1_000_000, 0), // $4
            e("2026-11-26T15:00:00Z", "s2", "/w/beta/", "claude-sonnet-5-5", 1_000_000, 1_000_000), // $12
            e("2026-11-26T16:00:00Z", "s3", "/w/beta/", "mystery-model", 500, 500),
        ];
        let r = build_report(&entries, &p, now, 2, brt());
        assert_eq!(r.by_day.iter().map(|d| d.date.as_str()).collect::<Vec<_>>(), ["2026-11-25", "2026-11-26"]);
        assert!((r.totals.cost_usd - 16.0).abs() < 1e-9);
        assert_eq!(r.totals.unpriced_tokens, 1000);
        assert_eq!(r.totals.messages, 3);
        assert_eq!(r.by_project[0].name, "beta");
        assert_eq!(r.by_project[0].sessions, 2);
        assert_eq!(r.by_project[1].name, "alpha");
        assert_eq!(r.by_model[0].model, "claude-sonnet-5-5");
        assert_eq!(r.by_session[0].session_id, "s2");

        let today = build_report(&entries, &p, now, 1, brt());
        assert_eq!(today.by_day.len(), 1);
        assert!((today.totals.cost_usd - 12.0).abs() < 1e-9);
    }

    #[test]
    fn empty_days_are_kept() {
        let r = build_report(&[], &Pricing::builtin(), 1_795_000_000, 7, brt());
        assert_eq!(r.by_day.len(), 7);
        assert_eq!(r.totals, Totals::default());
    }

    #[test]
    fn families_and_shares() {
        assert_eq!(model_family("claude-fable-5-2"), "Fable");
        assert_eq!(model_family("claude-3-5-sonnet-20241022"), "Sonnet");
        assert_eq!(model_family("claude-opus-5-5-20261001"), "Opus");
        assert_eq!(model_family("gpt-x"), "gpt");

        let p = Pricing::builtin();
        let entries = vec![
            e("2026-11-26T10:00:00Z", "s", "/p", "claude-fable-5-2", 3_000_000, 0),  // $24
            e("2026-11-26T11:00:00Z", "s", "/p", "claude-opus-5-5", 1_000_000, 0),   // $4
            e("2026-11-26T12:00:00Z", "s", "/p", "claude-opus-5", 800_000, 0),       // $4
            e("2026-11-20T12:00:00Z", "s", "/p", "claude-haiku-4-5", 9_000_000, 0),  // outside
        ];
        let since = chrono::DateTime::parse_from_rfc3339("2026-11-26T00:00:00Z").unwrap().timestamp();
        let shares = family_shares(&entries, &p, since, since + 86_400);
        assert_eq!(shares.iter().map(|s| s.family.as_str()).collect::<Vec<_>>(), ["Fable", "Opus"]);
        assert!((shares[0].share_pct - 75.0).abs() < 1e-9);
        assert!((shares[1].totals.cost_usd - 8.0).abs() < 1e-9);
    }

    #[test]
    fn burn_rate_over_window() {
        let p = Pricing::builtin();
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-26T18:00:00Z").unwrap().timestamp();
        let entries = vec![
            e("2026-11-26T17:00:00Z", "s", "/p", "claude-opus-5-5", 9_000_000, 0),
            e("2026-11-26T17:45:00Z", "s", "/p", "claude-opus-5-5", 300_000, 0),
        ];
        let b = burn(&entries, &p, now, 30);
        assert_eq!(b.tokens, 300_000);
        assert!((b.tokens_per_min - 10_000.0).abs() < 1e-9);
        assert!((b.cost_per_hour - 2.4).abs() < 1e-9);
    }

    #[test]
    fn block_estimate_restarts_after_gap() {
        let p = Pricing::builtin();
        let entries = vec![
            e("2026-11-26T08:10:00Z", "s", "/p", "claude-opus-5-5", 100, 0),
            e("2026-11-26T09:00:00Z", "s", "/p", "claude-opus-5-5", 100, 0),
            e("2026-11-26T15:20:00Z", "s", "/p", "claude-opus-5-5", 7, 0),
            e("2026-11-26T16:00:00Z", "s", "/p", "claude-opus-5-5", 3, 0),
        ];
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-26T17:00:00Z").unwrap().timestamp();
        let b = estimate_block(&entries, &p, now).unwrap();
        assert_eq!(b.start, chrono::DateTime::parse_from_rfc3339("2026-11-26T15:00:00Z").unwrap().timestamp());
        assert_eq!(b.totals.total_tokens, 10);
        let late = chrono::DateTime::parse_from_rfc3339("2026-11-26T21:00:00Z").unwrap().timestamp();
        assert!(estimate_block(&entries, &p, late).is_none());
    }
}
