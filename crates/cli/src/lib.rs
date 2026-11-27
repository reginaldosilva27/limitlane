//! `ctc`: command-line companion. The menu bar app embeds this too, so the
//! app binary itself can act as the statusline hook.

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use ctc_core::aggregate::{Report, Totals};
use ctc_core::alerts::fmt_duration;
use ctc_core::forecast::{Basis, Forecast};
use ctc_core::hook::{self, HookStatus};
use ctc_core::limits::{self, StatuslineInput};
use ctc_core::summary::{fmt_tokens, Engine, Summary};
use ctc_core::Paths;
use std::fmt::Write as _;
use std::io::{Read, Write};

#[derive(Parser)]
#[command(name = "ctc", version, about = "Claude usage in your terminal: limits, forecast, cost per project")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Limits, forecast, burn rate and today's totals
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Tokens and API-equivalent cost per project
    Projects {
        #[arg(long, default_value_t = 7)]
        days: u32,
        #[arg(long, default_value_t = 15)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Tokens and cost per day
    Daily {
        #[arg(long, default_value_t = 7)]
        days: u32,
        #[arg(long)]
        json: bool,
    },
    /// Tokens and cost per model
    Models {
        #[arg(long, default_value_t = 7)]
        days: u32,
        #[arg(long)]
        json: bool,
    },
    /// Context window usage of recent sessions
    Sessions {
        #[arg(long)]
        json: bool,
    },
    /// Machine-readable export for scripts and dashboards
    Export {
        #[arg(long, default_value_t = 30)]
        days: u32,
        #[arg(long, value_enum, default_value_t = Format::Json)]
        format: Format,
        /// Row grouping for CSV output
        #[arg(long, value_enum, default_value_t = GroupBy::Project)]
        by: GroupBy,
    },
    /// Manage the Claude Code statusline hook
    Hook {
        #[command(subcommand)]
        action: HookAction,
    },
    /// Statusline entry point called by Claude Code (reads JSON on stdin)
    Statusline {
        #[arg(long = "ctc-hook", hide = true)]
        marker: bool,
        /// Run the statusline that was configured before the hook
        #[arg(long)]
        chain: bool,
    },
}

#[derive(Subcommand)]
enum HookAction {
    /// Register the hook in ~/.claude/settings.json (keeps an existing statusline)
    Install,
    /// Remove the hook and restore the previous statusline
    Uninstall,
    /// Show whether the hook is installed
    Status,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Json,
    Csv,
}

#[derive(Clone, Copy, ValueEnum)]
enum GroupBy {
    Project,
    Day,
    Model,
    Session,
}

/// Run the CLI with `args` (without the program name). Returns the exit code.
pub fn run(args: Vec<String>) -> i32 {
    let cli = match Cli::try_parse_from(std::iter::once("ctc".to_string()).chain(args)) {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() { 2 } else { 0 };
        }
    };
    let paths = Paths::from_env();
    match execute(cli.command, paths) {
        Ok(text) => {
            // Ignore broken pipes (`ctc export | head`).
            let _ = std::io::stdout().write_all(text.as_bytes());
            0
        }
        Err(e) => {
            eprintln!("ctc: {e:#}");
            1
        }
    }
}

fn execute(command: Command, paths: Paths) -> Result<String> {
    let now = ctc_core::now();
    let offset = ctc_core::local_offset();
    let mut engine = Engine::new(paths.clone());
    Ok(match command {
        Command::Status { json } => {
            let s = engine.summary(1, now, offset);
            if json { to_json(&s)? } else { render_status(&s, offset) }
        }
        Command::Projects { days, limit, json } => {
            let r = engine.summary(days, now, offset).report;
            if json { to_json(&r.by_project)? } else { render_rows(&r, days, "PROJECT", r.by_project.iter().take(limit).map(|p| (p.name.clone(), &p.totals))) }
        }
        Command::Daily { days, json } => {
            let r = engine.summary(days, now, offset).report;
            if json { to_json(&r.by_day)? } else { render_rows(&r, days, "DAY", r.by_day.iter().map(|d| (d.date.clone(), &d.totals))) }
        }
        Command::Models { days, json } => {
            let r = engine.summary(days, now, offset).report;
            if json { to_json(&r.by_model)? } else { render_rows(&r, days, "MODEL", r.by_model.iter().map(|m| (m.model.clone(), &m.totals))) }
        }
        Command::Sessions { json } => {
            let s = engine.summary(1, now, offset);
            if json { to_json(&s.sessions)? } else { render_sessions(&s) }
        }
        Command::Export { days, format, by } => {
            let s = engine.summary(days, now, offset);
            match format {
                Format::Json => to_json(&s)?,
                Format::Csv => render_csv(&s.report, by),
            }
        }
        Command::Hook { action } => match action {
            HookAction::Install => {
                let exe = std::env::current_exe()?;
                let out = hook::install(&paths, &exe)?;
                let mut t = format!("Hook installed in {}\n  {}\n", out.settings_file, out.command);
                if out.wrapped_existing {
                    t.push_str("Your existing statusline was kept and still renders as before.\n");
                }
                t.push_str("Limits appear after the next response in any Claude Code session.\n");
                t
            }
            HookAction::Uninstall => {
                if hook::uninstall(&paths)? { "Hook removed; previous statusline restored.\n".into() } else { "Hook was not installed; nothing changed.\n".into() }
            }
            HookAction::Status => match hook::status(&paths) {
                HookStatus::Installed { command, chained } => {
                    let mut t = format!("installed: {command}\n");
                    if let Some(c) = chained {
                        let _ = writeln!(t, "chained statusline: {c}");
                    }
                    t
                }
                HookStatus::Other { command } => format!("not installed (another statusline is configured: {command})\nRun `ctc hook install` to wrap it; it will keep rendering.\n"),
                HookStatus::NotInstalled => "not installed\n".into(),
            },
        },
        Command::Statusline { chain, .. } => {
            statusline(&paths, chain, now);
            String::new()
        }
    })
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<String> {
    let mut s = serde_json::to_string_pretty(v)?;
    s.push('\n');
    Ok(s)
}

// ---------- statusline hook ----------

/// Never fails: a broken statusline would be visible in every Claude Code
/// session, so every error is swallowed.
fn statusline(paths: &Paths, chain: bool, now: i64) {
    let mut payload = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut payload);
    let input: StatuslineInput = serde_json::from_slice(&payload).unwrap_or_default();
    let _ = limits::record(paths, &input, now);

    if chain {
        if let Some(cmd) = hook::chained_command(paths) {
            if run_chained(&cmd, &payload) {
                return;
            }
        }
    }
    println!("{}", default_statusline(&input, now));
}

fn run_chained(cmd: &str, payload: &[u8]) -> bool {
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new("sh").arg("-c").arg(cmd).stdin(Stdio::piped()).spawn() else { return false };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload);
    }
    child.wait().is_ok()
}

/// The line shown in Claude Code when no other statusline was configured.
pub fn default_statusline(input: &StatuslineInput, now: i64) -> String {
    let mut parts = Vec::new();
    if let Some(name) = input.model.as_ref().and_then(|m| m.display_name.as_deref()) {
        parts.push(name.to_string());
    }
    if let Some(rl) = &input.rate_limits {
        if let Some(w) = rl.five_hour {
            parts.push(format!("5h {:.0}% ({})", w.used_percentage, fmt_duration(w.resets_at - now)));
        }
        if let Some(w) = rl.seven_day {
            parts.push(format!("7d {:.0}%", w.used_percentage));
        }
    }
    if let Some(p) = input.context_window.as_ref().and_then(|c| c.used_percentage) {
        parts.push(format!("ctx {p:.0}%"));
    }
    parts.join(" · ")
}

// ---------- human output ----------

fn clock(ts: i64, offset: chrono::FixedOffset) -> String {
    use chrono::TimeZone;
    offset.timestamp_opt(ts, 0).single().map(|d| d.format("%a %H:%M").to_string()).unwrap_or_default()
}

fn bar(pct: f64, width: usize) -> String {
    let filled = ((pct.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn forecast_line(f: &Forecast, now: i64, offset: chrono::FixedOffset) -> String {
    match (f.hits_limit_at, f.basis) {
        (Some(_), _) if f.used_pct >= 100.0 => "limit reached".into(),
        (Some(eta), _) => format!("at this pace: limit {} ({} before reset)", clock(eta, offset), fmt_duration(f.resets_at - eta)),
        (None, Basis::None) => "not enough data for a forecast yet".into(),
        (None, _) => {
            let _ = now;
            format!("at this pace: {:.0}% at reset", f.projected_pct_at_reset)
        }
    }
}

pub fn render_status(s: &Summary, offset: chrono::FixedOffset) -> String {
    let mut t = String::new();
    let now = s.generated_at;
    if s.forecasts.is_empty() {
        t.push_str("Limits: no data from Claude Code yet.\n");
        match &s.hook {
            HookStatus::Installed { .. } => t.push_str("  The hook is installed; limits appear after the next response in Claude Code\n  (Pro and Max plans only).\n"),
            _ => t.push_str("  Run `ctc hook install` to get the real 5-hour and weekly percentages.\n"),
        }
        if let Some(b) = &s.block_estimate {
            let _ = writeln!(t, "  Estimated 5-hour window: resets about {} · {} tokens · ${:.2}", clock(b.end, offset), fmt_tokens(b.totals.total_tokens), b.totals.cost_usd);
        }
    }
    for f in &s.forecasts {
        let _ = writeln!(t, "{:<7} {} {:>5.1}%   resets in {} ({})", f.window.label(), bar(f.used_pct, 20), f.used_pct, fmt_duration(f.seconds_to_reset), clock(f.resets_at, offset));
        let _ = writeln!(t, "        {}", forecast_line(f, now, offset));
    }
    if let Some(age) = s.limits_age_secs.filter(|a| *a > 900) {
        let _ = writeln!(t, "        (last reported by Claude Code {} ago)", fmt_duration(age));
    }
    let _ = writeln!(t, "\nBurn (last {} min): {}/min · ${:.2}/h API-equivalent", s.burn.window_minutes, fmt_tokens(s.burn.tokens_per_min.round() as u64), s.burn.cost_per_hour);
    let _ = writeln!(t, "Today: {} tokens · ${:.2} · {} responses", fmt_tokens(s.today.total_tokens), s.today.cost_usd, s.today.messages);
    t
}

fn render_rows<'a>(r: &Report, days: u32, head: &str, rows: impl Iterator<Item = (String, &'a Totals)>) -> String {
    let rows: Vec<(String, &Totals)> = rows.collect();
    let width = rows.iter().map(|(n, _)| n.chars().count()).max().unwrap_or(0).clamp(head.len(), 40);
    let mut t = format!("Last {days} day(s)\n\n{head:<width$}  {:>9}  {:>9}  {:>9}  {:>9}  {:>9}\n", "INPUT", "OUTPUT", "CACHE R", "CACHE W", "COST");
    for (name, tot) in rows.iter().chain(std::iter::once(&("TOTAL".to_string(), &r.totals))) {
        let name: String = name.chars().take(width).collect();
        let _ = writeln!(t, "{name:<width$}  {:>9}  {:>9}  {:>9}  {:>9}  {:>9}", fmt_tokens(tot.input), fmt_tokens(tot.output), fmt_tokens(tot.cache_read), fmt_tokens(tot.cache_write), format!("${:.2}", tot.cost_usd));
    }
    if r.totals.unpriced_tokens > 0 {
        let _ = writeln!(t, "\n{} tokens came from models missing in the pricing table and are not in COST.", fmt_tokens(r.totals.unpriced_tokens));
    }
    t.push_str("\nCOST is what this usage would cost at API list prices, not what a subscription bills.\n");
    t
}

fn render_sessions(s: &Summary) -> String {
    if s.sessions.is_empty() {
        return "No Claude Code sessions in the last 6 hours.\n".into();
    }
    let mut t = format!("{:<24}  {:<14}  {:<26}  {:>9}  {}\n", "SESSION", "MODEL", "CONTEXT", "TOKENS", "LAST SEEN");
    for v in &s.sessions {
        let ctx = match v.context_used_pct {
            Some(p) => format!("{} {:>4.0}%", bar(p, 20), p),
            None => "(size not reported yet)".into(),
        };
        let name: String = v.name.chars().take(24).collect();
        let model: String = v.model.clone().unwrap_or_default().chars().take(14).collect();
        let _ = writeln!(t, "{name:<24}  {model:<14}  {ctx:<26}  {:>9}  {} ago", v.context_tokens.map(fmt_tokens).unwrap_or_else(|| "-".into()), fmt_duration(s.generated_at - v.last_seen));
    }
    t
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

fn render_csv(r: &Report, by: GroupBy) -> String {
    let label = match by {
        GroupBy::Project => "project",
        GroupBy::Day => "date",
        GroupBy::Model => "model",
        GroupBy::Session => "session_id",
    };
    let mut t = format!("{label},input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,total_tokens,cost_usd,responses\n");
    let mut row = |name: &str, x: &Totals| {
        let _ = writeln!(t, "{},{},{},{},{},{},{:.6},{}", csv_field(name), x.input, x.output, x.cache_read, x.cache_write, x.total_tokens, x.cost_usd, x.messages);
    };
    match by {
        GroupBy::Project => r.by_project.iter().for_each(|p| row(&p.project, &p.totals)),
        GroupBy::Day => r.by_day.iter().for_each(|d| row(&d.date, &d.totals)),
        GroupBy::Model => r.by_model.iter().for_each(|m| row(&m.model, &m.totals)),
        GroupBy::Session => r.by_session.iter().for_each(|s| row(&s.session_id, &s.totals)),
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_statusline_line() {
        let input: StatuslineInput = serde_json::from_str(
            r#"{"model":{"display_name":"Opus"},"context_window":{"used_percentage":25},
                "rate_limits":{"five_hour":{"used_percentage":42.4,"resets_at":7500},"seven_day":{"used_percentage":18,"resets_at":99999}}}"#,
        )
        .unwrap();
        assert_eq!(default_statusline(&input, 0), "Opus · 5h 42% (2h 05m) · 7d 18% · ctx 25%");
        assert_eq!(default_statusline(&StatuslineInput::default(), 0), "");
    }

    #[test]
    fn csv_escapes_fields() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b \"c\""), "\"a,b \"\"c\"\"\"");
    }

    #[test]
    fn bar_is_fixed_width() {
        assert_eq!(bar(50.0, 10).chars().count(), 10);
        assert_eq!(bar(250.0, 4), "████");
        assert_eq!(bar(0.0, 4), "░░░░");
    }
}
