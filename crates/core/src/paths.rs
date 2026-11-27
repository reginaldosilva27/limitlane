//! Where Claude Code keeps its data and where this tool keeps its own state.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    /// Claude Code config directories (usually just `~/.claude`).
    pub claude_dirs: Vec<PathBuf>,
    /// This tool's own directory (`~/.paceline`).
    pub state_dir: PathBuf,
}

impl Paths {
    /// Resolve from the environment.
    ///
    /// * `CLAUDE_CONFIG_DIR` (comma-separated) overrides the Claude directories.
    /// * `PACELINE_HOME` overrides this tool's state directory.
    pub fn from_env() -> Self {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let claude_dirs = match std::env::var("CLAUDE_CONFIG_DIR") {
            Ok(v) if !v.trim().is_empty() => v.split(',').map(|s| PathBuf::from(s.trim())).collect(),
            _ => vec![home.join(".claude"), home.join(".config").join("claude")],
        };
        let state_dir = match std::env::var_os("PACELINE_HOME") {
            Some(v) if !v.is_empty() => PathBuf::from(v),
            _ => {
                let dir = home.join(".paceline");
                migrate_legacy_state(&home.join(".claudetokenscounter"), &dir);
                dir
            }
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

/// Carry settings and limit history over from the directory used before the
/// project was renamed. Does nothing once the new directory exists.
pub fn migrate_legacy_state(old: &std::path::Path, new: &std::path::Path) {
    if old.is_dir() && !new.exists() {
        let _ = std::fs::rename(old, new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_state_moves_once() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (dir.path().join("old"), dir.path().join("new"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("config.json"), "{}").unwrap();
        migrate_legacy_state(&old, &new);
        assert!(new.join("config.json").exists() && !old.exists());
        // A later leftover never overwrites the new directory.
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("config.json"), "stale").unwrap();
        migrate_legacy_state(&old, &new);
        assert_eq!(std::fs::read_to_string(new.join("config.json")).unwrap(), "{}");
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
