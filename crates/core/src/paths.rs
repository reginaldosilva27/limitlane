//! Where Claude Code keeps its data and where this tool keeps its own state.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    /// Claude Code config directories (usually just `~/.claude`).
    pub claude_dirs: Vec<PathBuf>,
    /// This tool's own directory (`~/.claudetokenscounter`).
    pub state_dir: PathBuf,
}

impl Paths {
    /// Resolve from the environment.
    ///
    /// * `CLAUDE_CONFIG_DIR` (comma-separated) overrides the Claude directories.
    /// * `CTC_HOME` overrides this tool's state directory.
    pub fn from_env() -> Self {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let claude_dirs = match std::env::var("CLAUDE_CONFIG_DIR") {
            Ok(v) if !v.trim().is_empty() => v.split(',').map(|s| PathBuf::from(s.trim())).collect(),
            _ => vec![home.join(".claude"), home.join(".config").join("claude")],
        };
        let state_dir = match std::env::var_os("CTC_HOME") {
            Some(v) if !v.is_empty() => PathBuf::from(v),
            _ => home.join(".claudetokenscounter"),
        };
        Self { claude_dirs, state_dir }
    }

    /// Existing `projects/` directories holding session transcripts.
    pub fn project_roots(&self) -> Vec<PathBuf> {
        self.claude_dirs.iter().map(|d| d.join("projects")).filter(|p| p.is_dir()).collect()
    }

    /// The user-level Claude Code settings file.
    pub fn settings_file(&self) -> PathBuf {
        let existing = self.claude_dirs.iter().find(|d| d.is_dir());
        existing.or(self.claude_dirs.first()).cloned().unwrap_or_default().join("settings.json")
    }

    pub fn limits_file(&self) -> PathBuf {
        self.state_dir.join("limits.json")
    }
    pub fn history_file(&self) -> PathBuf {
        self.state_dir.join("history.jsonl")
    }
    pub fn sessions_dir(&self) -> PathBuf {
        self.state_dir.join("sessions")
    }
    pub fn alerts_file(&self) -> PathBuf {
        self.state_dir.join("alerts.json")
    }
    pub fn config_file(&self) -> PathBuf {
        self.state_dir.join("config.json")
    }
    pub fn original_statusline_file(&self) -> PathBuf {
        self.state_dir.join("original_statusline.json")
    }
}

/// Write a file atomically (temp file + rename) so concurrent readers never
/// see a half-written document.
pub fn write_atomic(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}
