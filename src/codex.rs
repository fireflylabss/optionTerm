//! Codex thread access: discover saved Codex conversations and render them as
//! readable Markdown transcripts.
//!
//! Codex stores its sessions under `~/.codex` (or `$CODEX_HOME`): one JSONL
//! "rollout" per conversation plus a `session_index.jsonl` that maps a thread
//! id to its most recent title. A thread can be renamed, so the id is the
//! stable key; the title only makes the exported filename friendlier.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

/// A single saved Codex conversation.
#[derive(Clone, Debug, PartialEq)]
pub struct CodexThread {
    /// Stable session id (`019e2e27-…`), the rollout filename's tail.
    pub id: String,
    /// Last known thread title.
    pub title: String,
    /// Absolute path to the `rollout-*.jsonl` transcript.
    pub file: PathBuf,
    /// `session_meta.payload.cwd`, the directory the thread was started in.
    pub cwd: Option<PathBuf>,
    /// Last update, as Unix epoch seconds (from `session_index.jsonl`).
    pub time: i64,
}

/// The Codex data directory (`~/.codex` or `$CODEX_HOME`), if present.
pub fn codex_home() -> Option<PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))?;
    home.is_dir().then_some(home)
}

/// Find the session file (rollout) for a thread id under the given sessions
/// tree (`<home>/sessions/YYYY/MM/DD/rollout-…-<id>.jsonl`).
fn find_rollout(sessions_root: &Path, id: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(sessions_root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    while let Some(dir) = dirs.pop() {
        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        dirs.push(p);
                    } else if p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with(&format!("-{id}.jsonl")))
                    {
                        return Some(p.to_path_buf());
                    }
                }
            }
            Err(_) => continue,
        }
    }
    None
}

/// All saved threads, newest first (by the index's update time).
pub fn list_threads() -> Vec<CodexThread> {
    let Some(home) = codex_home() else {
        return Vec::new();
    };
    let index = home.join("session_index.jsonl");
    let Ok(text) = std::fs::read_to_string(&index) else {
        return Vec::new();
    };
    let sessions_root = home.join("sessions");

    let mut entries: Vec<(String, String, i64)> = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let (Some(id), Some(updated)) = (
            v.get("id").and_then(|x| x.as_str()),
            v.get("updated_at").and_then(|x| x.as_str()),
        ) else {
            continue;
        };
        let title = v
            .get("thread_name")
            .and_then(|x| x.as_str())
            .unwrap_or("Untitled")
            .to_string();
        let ts = parse_iso_time(updated).unwrap_or(0);
        if !entries.iter().any(|(eid, _, _)| eid == id) {
            entries.push((id.to_string(), title, ts));
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.2));

    entries
        .into_iter()
        .filter_map(|(id, title, ts)| {
            let file = find_rollout(&sessions_root, &id)?;
            let cwd = session_cwd(&file);
            Some(CodexThread {
                id,
                title,
                file,
                cwd,
                time: ts,
            })
        })
        .collect()
}

/// The `session_meta.payload.cwd` for a rollout file.
fn session_cwd(file: &Path) -> Option<PathBuf> {
    let first = std::fs::read_to_string(file)
        .ok()?
        .lines()
        .next()?
        .to_string();
    let v: Value = serde_json::from_str(&first).ok()?;
    v.get("payload")
        .and_then(|p| p.get("cwd"))
        .and_then(|c| c.as_str())
        .map(PathBuf::from)
}

/// Render a rollout transcript to a readable Markdown document.
///
/// Messages are emitted in their original order; consecutive turns from the
/// same side are joined under a single heading.
pub fn render_markdown(file: &Path) -> Result<String> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;

    enum Role {
        User,
        Codex,
    }
    // (role, text)
    let mut blocks: Vec<(Role, String)> = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("event_msg") {
            continue;
        }
        let payload = v.get("payload").unwrap_or(&Value::Null);
        let Some(msg) = payload.get("message").and_then(|m| m.as_str()) else {
            continue;
        };
        let role = match payload.get("type").and_then(|t| t.as_str()) {
            Some("user_message") => Role::User,
            Some("agent_message") => Role::Codex,
            _ => continue,
        };
        // Merge consecutive messages from the same side into one block.
        let same_side = blocks.last().is_some_and(|(r, _)| {
            matches!(
                (r, &role),
                (Role::User, Role::User) | (Role::Codex, Role::Codex)
            )
        });
        if same_side {
            if let Some((_, last)) = blocks.last_mut() {
                last.push('\n');
                last.push_str(msg.trim());
            }
        } else {
            blocks.push((role, msg.trim().to_string()));
        }
    }

    if blocks.is_empty() {
        return Ok(String::from("# Codex transcript\n\n_(vazio)_\n"));
    }

    let mut out = String::from("# Codex transcript\n\n");
    for (role, msg) in blocks {
        match role {
            Role::User => out.push_str("## Você\n\n"),
            Role::Codex => out.push_str("## Codex\n\n"),
        }
        out.push_str(&msg);
        out.push_str("\n\n");
    }
    Ok(out)
}

/// A filesystem- and Markdown-safe filename for a thread title.
pub fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for c in title.chars() {
        if c.is_alphanumeric() {
            slug.push(c);
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "codex-thread".to_string()
    } else {
        slug
    }
}

/// Best-effort parse of an RFC3339 timestamp to a Unix epoch.
fn parse_iso_time(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z').unwrap_or(s);
    // `2026-05-22T01:45:24.333508774`
    let mut it = s.split(['-', 'T', ':', '.']);
    let (py, pm, pd) = (
        it.next()?.parse::<i64>().ok()?,
        it.next()?.parse::<u32>().ok()?,
        it.next()?.parse::<u32>().ok()?,
    );
    let (ph, pmi, psec) = (
        it.next()?.parse::<i64>().ok()?,
        it.next()?.parse::<i64>().ok()?,
        it.next()?.parse::<i64>().ok()?,
    );
    Some(days_from_civil(py, pm, pd) * 86_400 + ph * 3600 + pmi * 60 + psec)
}

/// Howard Hinnant's civil↔days algorithm: days since 1970-01-01.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_is_fragment_safe() {
        assert_eq!(
            slugify("Criar nosso Lovable.dev"),
            "Criar-nosso-Lovable-dev"
        );
        assert_eq!(slugify("  hello -- world  "), "hello-world");
        assert_eq!(slugify("!!!"), "codex-thread");
    }

    #[test]
    fn parses_iso_timestamps() {
        // 2026-05-22T01:45:24Z in epoch seconds (verified independently).
        let t = parse_iso_time("2026-05-22T01:45:24.333508774Z").unwrap();
        assert_eq!(t, 1779414324);
    }

    #[test]
    fn renders_known_message_types() {
        // A tiny fake rollout exercising both message kinds.
        let dir = std::env::temp_dir().join("option-codex-test");
        std::fs::create_dir_all(&dir).ok();
        let file = dir.join("rollout-1.jsonl");
        let body = [
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp\"}}",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"oi\"}}",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"olá\"}}",
        ]
        .join("\n");
        std::fs::write(&file, body).unwrap();

        let md = render_markdown(&file).unwrap();
        assert!(md.contains("## Você"));
        assert!(md.contains("## Codex"));
        assert!(md.contains("oi"));
        assert!(md.contains("olá"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finds_rollout_nested() {
        let root = std::env::temp_dir().join("option-codex-rollout-test");
        let deep = root.join("sessions").join("2026").join("05").join("22");
        std::fs::create_dir_all(&deep).ok();
        std::fs::write(deep.join("rollout-2026-05-22T01-45-24-abc123.jsonl"), "").ok();
        let found = find_rollout(&root.join("sessions"), "abc123");
        assert!(found.is_some_and(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("-abc123.jsonl"))
        }));
        std::fs::remove_dir_all(&root).ok();
    }
}
