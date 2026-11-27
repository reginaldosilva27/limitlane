//! Reads Claude Code session transcripts (`~/.claude/projects/**/*.jsonl`).

use crate::pricing::Usage;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One assistant response with token usage.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Unix epoch seconds.
    pub ts: i64,
    pub session_id: String,
    /// Working directory of the session; falls back to the transcript folder name.
    pub project: String,
    pub model: String,
    pub usage: Usage,
    /// True for subagent traffic.
    pub is_sidechain: bool,
    /// Dedup key (`message.id:requestId`) when the transcript provides one.
    pub key: Option<String>,
}

#[derive(Deserialize)]
struct RawLine {
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default, rename = "sessionId")]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default, rename = "requestId")]
    request_id: Option<String>,
    #[serde(default, rename = "isSidechain")]
    is_sidechain: Option<bool>,
    #[serde(default)]
    message: Option<RawMessage>,
}

#[derive(Deserialize)]
struct RawMessage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<RawUsage>,
}

#[derive(Deserialize)]
struct RawUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation: Option<RawCacheCreation>,
}

#[derive(Deserialize)]
struct RawCacheCreation {
    #[serde(default)]
    ephemeral_5m_input_tokens: Option<u64>,
    #[serde(default)]
    ephemeral_1h_input_tokens: Option<u64>,
}

/// Parse one transcript line. Returns `None` for anything that is not an
/// assistant response carrying usage (user turns, tool results, summaries,
/// synthetic messages, malformed lines).
pub fn parse_line(line: &str, fallback_project: &str, fallback_session: &str) -> Option<Entry> {
    // Cheap pre-filter: most lines are large tool payloads without usage.
    if !line.contains("\"usage\"") {
        return None;
    }
    let raw: RawLine = serde_json::from_str(line).ok()?;
    let msg = raw.message?;
    let u = msg.usage?;
    let model = msg.model.unwrap_or_default();
    if model.is_empty() || model.starts_with('<') {
        return None;
    }
    let ts = chrono::DateTime::parse_from_rfc3339(raw.timestamp.as_deref()?).ok()?.timestamp();

    let total_write = u.cache_creation_input_tokens.unwrap_or(0);
    let (w5, w1) = match u.cache_creation {
        Some(c) => {
            let w1 = c.ephemeral_1h_input_tokens.unwrap_or(0);
            let w5 = c.ephemeral_5m_input_tokens.unwrap_or(0);
            // Trust the top-level total if the breakdown does not add up.
            if w1 + w5 == 0 { (total_write, 0) } else { (w5, w1) }
        }
        None => (total_write, 0),
    };
    let usage = Usage {
        input: u.input_tokens.unwrap_or(0),
        output: u.output_tokens.unwrap_or(0),
        cache_read: u.cache_read_input_tokens.unwrap_or(0),
        cache_write_5m: w5,
        cache_write_1h: w1,
    };
    if usage.total() == 0 {
        return None;
    }
    let key = match (msg.id, raw.request_id) {
        (Some(m), Some(r)) => Some(format!("{m}:{r}")),
        (Some(m), None) => Some(m),
        _ => None,
    };
    Some(Entry {
        ts,
        session_id: raw.session_id.unwrap_or_else(|| fallback_session.to_string()),
        project: raw.cwd.filter(|c| !c.is_empty()).unwrap_or_else(|| fallback_project.to_string()),
        model,
        usage,
        is_sidechain: raw.is_sidechain.unwrap_or(false),
        key,
    })
}

struct CachedFile {
    mtime: SystemTime,
    len: u64,
    entries: Vec<Entry>,
}

/// Incremental transcript scanner. Files are re-parsed only when their size or
/// modification time changes, so repeated scans are cheap.
#[derive(Default)]
pub struct Scanner {
    cache: HashMap<PathBuf, CachedFile>,
}

impl Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// All deduplicated entries with `ts >= since`, oldest first.
    pub fn scan(&mut self, roots: &[PathBuf], since: i64) -> Vec<Entry> {
        let mut files = Vec::new();
        for root in roots {
            collect_jsonl(root, root, 0, &mut files);
        }
        files.sort();

        let since_time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(since.max(0) as u64);
        let mut seen: Vec<PathBuf> = Vec::with_capacity(files.len());
        let mut keep: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        for (path, top) in &files {
            let Ok(meta) = std::fs::metadata(path) else { continue };
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            // A file last written before `since` cannot contain newer entries.
            if mtime < since_time {
                continue;
            }
            seen.push(path.clone());
            keep.insert(path.clone());
            let fresh = matches!(self.cache.get(path), Some(c) if c.mtime == mtime && c.len == meta.len());
            if !fresh {
                let entries = read_file(path, top);
                self.cache.insert(path.clone(), CachedFile { mtime, len: meta.len(), entries });
            }
        }
        self.cache.retain(|p, _| keep.contains(p));

        // Dedup across files: resumed sessions and streamed responses repeat
        // the same message. Keep the copy with the most output tokens.
        let mut out: Vec<Entry> = Vec::new();
        let mut index: HashMap<&str, usize> = HashMap::new();
        for path in &seen {
            for e in &self.cache[path].entries {
                if e.ts < since {
                    continue;
                }
                match e.key.as_deref() {
                    Some(k) => match index.get(k) {
                        Some(&i) => {
                            if e.usage.output >= out[i].usage.output {
                                out[i] = e.clone();
                            }
                        }
                        None => {
                            index.insert(k, out.len());
                            out.push(e.clone());
                        }
                    },
                    None => out.push(e.clone()),
                }
            }
        }
        out.sort_by_key(|e| e.ts);
        out
    }
}

fn read_file(path: &Path, fallback_project: &str) -> Vec<Entry> {
    use std::io::BufRead;
    let Ok(file) = std::fs::File::open(path) else { return Vec::new() };
    let session = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    let mut out = Vec::new();
    for line in std::io::BufReader::new(file).split(b'\n') {
        let Ok(bytes) = line else { break };
        let Ok(text) = std::str::from_utf8(&bytes) else { continue };
        if let Some(e) = parse_line(text, fallback_project, &session) {
            out.push(e);
        }
    }
    out
}

/// Collect `(file, top-level project folder name)` pairs under `root`.
fn collect_jsonl(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(PathBuf, String)>) {
    if depth > 6 {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for item in read.flatten() {
        let path = item.path();
        let Ok(kind) = item.file_type() else { continue };
        if kind.is_dir() {
            collect_jsonl(root, &path, depth + 1, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let top = path
                .strip_prefix(root)
                .ok()
                .and_then(|rel| rel.components().next())
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push((path, top));
        }
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    /// A transcript line shaped like the ones Claude Code writes.
    pub fn line(ts: &str, session: &str, cwd: &str, model: &str, msg: &str, req: &str, input: u64, output: u64, cread: u64, cwrite: u64) -> String {
        format!(
            r#"{{"parentUuid":null,"isSidechain":false,"cwd":"{cwd}","sessionId":"{session}","type":"assistant","timestamp":"{ts}","requestId":"{req}","message":{{"id":"{msg}","type":"message","role":"assistant","model":"{model}","content":[{{"type":"text","text":"hi \"usage\" there"}}],"usage":{{"input_tokens":{input},"cache_creation_input_tokens":{cwrite},"cache_read_input_tokens":{cread},"output_tokens":{output},"service_tier":"standard"}}}}}}"#
        )
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::line;
    use super::*;

    #[test]
    fn parses_assistant_usage() {
        let l = line("2026-11-26T12:00:00.000Z", "s1", "/Users/r/proj", "claude-opus-5-5", "msg_1", "req_1", 10, 20, 30, 40);
        let e = parse_line(&l, "fallback", "file").unwrap();
        assert_eq!(e.ts, 1795694400);
        assert_eq!(e.project, "/Users/r/proj");
        assert_eq!(e.session_id, "s1");
        assert_eq!(e.usage, Usage { input: 10, output: 20, cache_read: 30, cache_write_5m: 40, cache_write_1h: 0 });
        assert_eq!(e.key.as_deref(), Some("msg_1:req_1"));
    }

    #[test]
    fn splits_cache_write_ttls() {
        let l = r#"{"timestamp":"2026-11-26T12:00:00Z","message":{"id":"m","model":"claude-sonnet-5-5","usage":{"input_tokens":1,"output_tokens":1,"cache_creation_input_tokens":300,"cache_creation":{"ephemeral_5m_input_tokens":100,"ephemeral_1h_input_tokens":200}}}}"#;
        let e = parse_line(l, "p", "sess").unwrap();
        assert_eq!((e.usage.cache_write_5m, e.usage.cache_write_1h), (100, 200));
        assert_eq!(e.project, "p");
        assert_eq!(e.session_id, "sess");
    }

    #[test]
    fn ignores_noise() {
        assert!(parse_line(r#"{"type":"user","message":{"role":"user","content":"usage"}}"#, "p", "s").is_none());
        assert!(parse_line("not json \"usage\"", "p", "s").is_none());
        let synthetic = line("2026-11-26T12:00:00Z", "s", "/p", "<synthetic>", "m", "r", 1, 1, 0, 0);
        assert!(parse_line(&synthetic, "p", "s").is_none());
        let zero = line("2026-11-26T12:00:00Z", "s", "/p", "claude-opus-5-5", "m", "r", 0, 0, 0, 0);
        assert!(parse_line(&zero, "p", "s").is_none());
    }

    #[test]
    fn scan_dedups_filters_and_caches() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("-Users-r-proj");
        std::fs::create_dir_all(proj.join("s1").join("subagents")).unwrap();
        let a = [
            line("2026-11-26T10:00:00Z", "s1", "/Users/r/proj", "claude-opus-5-5", "m1", "r1", 10, 5, 0, 0),
            // streamed duplicate of m1 with the final output count
            line("2026-11-26T10:00:01Z", "s1", "/Users/r/proj", "claude-opus-5-5", "m1", "r1", 10, 50, 0, 0),
            line("2026-11-20T10:00:00Z", "s1", "/Users/r/proj", "claude-opus-5-5", "old", "r0", 99, 99, 0, 0),
        ]
        .join("\n");
        std::fs::write(proj.join("s1.jsonl"), a).unwrap();
        // resumed session repeats m1 in a second file
        let b = [
            line("2026-11-26T10:00:01Z", "s2", "/Users/r/proj", "claude-opus-5-5", "m1", "r1", 10, 50, 0, 0),
            line("2026-11-26T11:00:00Z", "s2", "/Users/r/proj", "claude-opus-5-5", "m2", "r2", 7, 3, 0, 0),
        ]
        .join("\n");
        std::fs::write(proj.join("s2.jsonl"), b).unwrap();
        let sub = line("2026-11-26T11:30:00Z", "s1", "/Users/r/proj", "claude-haiku-5-5", "m3", "r3", 1, 1, 0, 0);
        std::fs::write(proj.join("s1").join("subagents").join("agent-x.jsonl"), sub).unwrap();

        let since = chrono::DateTime::parse_from_rfc3339("2026-11-25T00:00:00Z").unwrap().timestamp();
        let mut sc = Scanner::new();
        let roots = vec![dir.path().to_path_buf()];
        let got = sc.scan(&roots, since);
        assert_eq!(got.len(), 3, "m1 once, m2, m3; old entry filtered");
        assert_eq!(got[0].usage.output, 50, "kept the final streamed copy");
        assert_eq!(got.iter().map(|e| e.usage.output).sum::<u64>(), 54);
        assert_eq!(sc.cache.len(), 3);

        // Unchanged files come from the cache and give the same answer.
        assert_eq!(sc.scan(&roots, since), got);

        // Appending to a file is picked up.
        let mut more = std::fs::read_to_string(proj.join("s2.jsonl")).unwrap();
        more.push('\n');
        more.push_str(&line("2026-11-26T12:00:00Z", "s2", "/Users/r/proj", "claude-opus-5-5", "m4", "r4", 1, 100, 0, 0));
        std::fs::write(proj.join("s2.jsonl"), more).unwrap();
        assert_eq!(sc.scan(&roots, since).len(), 4);
    }
}
