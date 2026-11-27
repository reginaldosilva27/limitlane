//! Usage engine for Claude Tokens Counter.
//!
//! Two data sources, neither of which touches credentials or the network:
//!
//! * session transcripts under `~/.claude/projects` (tokens, cost, projects);
//! * the Claude Code statusline hook (real 5-hour and weekly percentages,
//!   context window per session).

pub mod aggregate;
pub mod alerts;
pub mod config;
pub mod forecast;
pub mod hook;
pub mod icon;
pub mod limits;
pub mod paths;
pub mod pricing;
pub mod summary;
pub mod transcript;

pub use paths::Paths;
pub use summary::{Engine, Summary};

/// Current time as Unix epoch seconds.
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The machine's current UTC offset.
pub fn local_offset() -> chrono::FixedOffset {
    use chrono::Offset;
    chrono::Local::now().offset().fix()
}
