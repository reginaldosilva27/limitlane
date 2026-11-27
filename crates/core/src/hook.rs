//! Installs this tool as Claude Code's statusline command.
//!
//! If a statusline is already configured it is preserved: the original is
//! saved and our hook runs it afterwards, so the terminal shows exactly what
//! it showed before. Uninstalling restores the original.

use crate::paths::{write_atomic, Paths};
use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::Path;

/// Present in every command line this tool installs.
pub const MARKER: &str = "--paceline-hook";
/// Marker written by builds from before the rename to Paceline.
pub const LEGACY_MARKER: &str = "--ctc-hook";

fn is_ours(command: &str) -> bool {
    command.contains(MARKER) || command.contains(LEGACY_MARKER)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum HookStatus {
    NotInstalled,
    Installed {
        command: String,
        /// The user's own statusline that runs after ours, if any.
        chained: Option<String>,
    },
    /// A statusline exists and does not belong to this tool.
    Other { command: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstallOutcome {
    pub command: String,
    pub wrapped_existing: bool,
    pub settings_file: String,
}

fn read_settings(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(anyhow!("{} is not a JSON object", path.display())),
        // Never overwrite a file we cannot parse.
        Err(e) => Err(anyhow!("{} is not valid JSON ({e}); fix it first", path.display())),
    }
}

fn write_settings(path: &Path, root: &Map<String, Value>) -> Result<()> {
    let mut body = serde_json::to_string_pretty(root)?;
    body.push('\n');
    write_atomic(path, body.as_bytes()).with_context(|| format!("writing {}", path.display()))
}

fn command_of(statusline: &Value) -> Option<&str> {
    statusline.get("command").and_then(Value::as_str)
}

fn saved_original(paths: &Paths) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(paths.original_statusline_file()).ok()?).ok()
}

/// The user's original statusline command, if one was wrapped at install time.
pub fn chained_command(paths: &Paths) -> Option<String> {
    saved_original(paths).as_ref().and_then(command_of).map(str::to_string)
}

pub fn status(paths: &Paths) -> HookStatus {
    let Ok(root) = read_settings(&paths.settings_file()) else { return HookStatus::NotInstalled };
    match root.get("statusLine").and_then(command_of) {
        Some(cmd) if is_ours(cmd) => HookStatus::Installed { command: cmd.to_string(), chained: chained_command(paths) },
        Some(cmd) => HookStatus::Other { command: cmd.to_string() },
        None => HookStatus::NotInstalled,
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Point Claude Code's statusline at `exe statusline --paceline-hook`.
pub fn install(paths: &Paths, exe: &Path) -> Result<InstallOutcome> {
    let settings_path = paths.settings_file();
    let mut root = read_settings(&settings_path)?;
    let existing = root.get("statusLine").cloned();

    let ours = existing.as_ref().and_then(command_of).is_some_and(is_ours);
    if !ours {
        match &existing {
            Some(orig) if command_of(orig).is_some() => {
                write_atomic(&paths.original_statusline_file(), serde_json::to_string_pretty(orig)?.as_bytes())?;
            }
            _ => {
                let _ = std::fs::remove_file(paths.original_statusline_file());
            }
        }
        // One-time safety copy of the untouched settings file.
        let backup = settings_path.with_extension("json.paceline-backup");
        if settings_path.exists() && !backup.exists() {
            std::fs::copy(&settings_path, &backup).ok();
        }
    }
    let wrapped = chained_command(paths).is_some();

    let mut command = format!("{} statusline {MARKER}", shell_quote(&exe.to_string_lossy()));
    if wrapped {
        command.push_str(" --chain");
    }
    // Keep the user's other statusline options (padding, refreshInterval).
    let mut obj = match existing {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    obj.insert("type".into(), json!("command"));
    obj.insert("command".into(), json!(command));
    root.insert("statusLine".into(), Value::Object(obj));
    write_settings(&settings_path, &root)?;

    Ok(InstallOutcome { command, wrapped_existing: wrapped, settings_file: settings_path.to_string_lossy().into_owned() })
}

/// Remove the hook and put back whatever statusline was there before.
pub fn uninstall(paths: &Paths) -> Result<bool> {
    let settings_path = paths.settings_file();
    let mut root = read_settings(&settings_path)?;
    let ours = root.get("statusLine").and_then(command_of).is_some_and(is_ours);
    if !ours {
        return Ok(false);
    }
    match saved_original(paths) {
        Some(orig) => {
            root.insert("statusLine".into(), orig);
        }
        None => {
            root.remove("statusLine");
        }
    }
    write_settings(&settings_path, &root)?;
    let _ = std::fs::remove_file(paths.original_statusline_file());
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dir: &Path) -> Paths {
        let claude = dir.join("claude");
        std::fs::create_dir_all(&claude).unwrap();
        Paths { claude_dirs: vec![claude], state_dir: dir.join("state") }
    }

    fn settings(p: &Paths) -> Value {
        serde_json::from_str(&std::fs::read_to_string(p.settings_file()).unwrap()).unwrap()
    }

    #[test]
    fn install_into_missing_settings_then_uninstall() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        assert_eq!(status(&p), HookStatus::NotInstalled);

        let out = install(&p, Path::new("/Applications/Paceline.app/Contents/MacOS/paceline")).unwrap();
        assert!(!out.wrapped_existing);
        assert_eq!(out.command, "'/Applications/Paceline.app/Contents/MacOS/paceline' statusline --paceline-hook");
        assert_eq!(settings(&p)["statusLine"]["type"], "command");
        assert!(matches!(status(&p), HookStatus::Installed { chained: None, .. }));

        assert!(uninstall(&p).unwrap());
        assert!(settings(&p).get("statusLine").is_none());
        assert!(!uninstall(&p).unwrap());
    }

    #[test]
    fn existing_statusline_is_wrapped_and_restored() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        let original = r#"{
  "model": "opus",
  "statusLine": { "type": "command", "command": "~/.claude/statusline.sh", "padding": 2 },
  "permissions": { "allow": ["Bash(ls:*)"] }
}"#;
        std::fs::write(p.settings_file(), original).unwrap();
        assert_eq!(status(&p), HookStatus::Other { command: "~/.claude/statusline.sh".into() });

        let out = install(&p, Path::new("/usr/local/bin/paceline")).unwrap();
        assert!(out.wrapped_existing);
        assert!(out.command.ends_with("--paceline-hook --chain"));
        let s = settings(&p);
        assert_eq!(s["statusLine"]["padding"], 2, "other options survive");
        assert_eq!(s["model"], "opus");
        assert_eq!(s["permissions"]["allow"][0], "Bash(ls:*)");
        // Key order is preserved.
        let text = std::fs::read_to_string(p.settings_file()).unwrap();
        assert!(text.find("\"model\"").unwrap() < text.find("\"statusLine\"").unwrap());
        assert_eq!(chained_command(&p).as_deref(), Some("~/.claude/statusline.sh"));
        assert_eq!(std::fs::read_to_string(p.settings_file().with_extension("json.paceline-backup")).unwrap(), original);

        // Re-installing (e.g. the app moved) keeps the saved original.
        let again = install(&p, Path::new("/opt/it's here/paceline")).unwrap();
        assert!(again.wrapped_existing);
        assert_eq!(again.command, r"'/opt/it'\''s here/paceline' statusline --paceline-hook --chain");
        assert_eq!(chained_command(&p).as_deref(), Some("~/.claude/statusline.sh"));

        assert!(uninstall(&p).unwrap());
        let s = settings(&p);
        assert_eq!(s["statusLine"]["command"], "~/.claude/statusline.sh");
        assert_eq!(s["statusLine"]["padding"], 2);
        assert_eq!(status(&p), HookStatus::Other { command: "~/.claude/statusline.sh".into() });
    }

    #[test]
    fn hook_from_before_the_rename_is_recognised_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        let old = r#"{"statusLine":{"type":"command","command":"'/old/target/debug/claude-tokens-counter' statusline --ctc-hook"}}"#;
        std::fs::write(p.settings_file(), old).unwrap();
        assert!(matches!(status(&p), HookStatus::Installed { .. }));
        let out = install(&p, Path::new("/Applications/Paceline.app/Contents/MacOS/paceline")).unwrap();
        assert!(!out.wrapped_existing, "our own old hook is not a statusline to chain");
        assert_eq!(out.command, "'/Applications/Paceline.app/Contents/MacOS/paceline' statusline --paceline-hook");
        assert!(uninstall(&p).unwrap());
        assert!(settings(&p).get("statusLine").is_none());
    }

    #[test]
    fn refuses_to_touch_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        std::fs::write(p.settings_file(), "{ not json").unwrap();
        assert!(install(&p, Path::new("/x")).is_err());
        assert_eq!(std::fs::read_to_string(p.settings_file()).unwrap(), "{ not json");
    }
}
