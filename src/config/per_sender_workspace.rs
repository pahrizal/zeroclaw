//! Per-sender workspace paths and seed files for `[workspace] per_sender_isolation`.

use crate::channels::traits::ChannelMessage;

use anyhow::Result;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Optional platform metadata for per-sender `USER.md` seeding (e.g. Telegram `from`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SenderProfileHint {
    /// Real name when provided by the channel (e.g. Telegram first + last name).
    pub display_name: Option<String>,
    /// Channel-specific handle without `@` (e.g. Telegram username).
    pub username: Option<String>,
    /// e.g. Telegram `language_code` (IETF tag).
    pub language_code: Option<String>,
}

/// Build initial `USER.md` body from the inbound message that created the per-sender workspace.
#[must_use]
pub fn user_md_seed_content(msg: &ChannelMessage) -> String {
    let mut out = String::new();
    out.push_str("# USER.md — per-user overlay\n\n");
    out.push_str(
        "This file extends the global `USER.md` in the main workspace. The **Sender snapshot** below was auto-filled when this per-sender workspace was first created.\n\n",
    );
    out.push_str("## Sender snapshot\n\n");
    let _ = writeln!(out, "- **Channel:** `{}`", msg.channel);
    if let Some(sid) = msg.sender_stable_id.as_deref() {
        let _ = writeln!(out, "- **Stable user id:** `{sid}`");
    }
    let _ = writeln!(out, "- **Display identity (channel):** `{}`", msg.sender);
    if let Some(p) = &msg.sender_profile {
        if let Some(name) = p
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let _ = writeln!(out, "- **Name:** {name}");
        }
        if let Some(u) = p
            .username
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let _ = writeln!(out, "- **Username:** @{u}");
        }
        if let Some(lang) = p
            .language_code
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let _ = writeln!(out, "- **Language (client):** `{lang}`");
        }
    }
    out.push_str("\n## Preferences & notes\n\n");
    out.push_str("(Add preferences and context specific to this sender.)\n");
    out
}

/// Returns `Some("tg_<digits>")` when `stable_id` is a non-empty ASCII digit string.
pub fn sanitized_segment(stable_id: &str) -> Option<String> {
    let t = stable_id.trim();
    if t.is_empty() || !t.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("tg_{t}"))
}

/// Resolved per-user workspace root: `{global_workspace}/{per_sender_subdir}/{tg_id}/`.
pub fn per_user_workspace_dir(
    global_workspace: &Path,
    per_sender_subdir: &str,
    stable_id: &str,
) -> Option<PathBuf> {
    let seg = sanitized_segment(stable_id)?;
    let sub = per_sender_subdir.trim().trim_matches('/');
    if sub.is_empty() || sub.contains("..") || sub.contains('/') {
        return None;
    }
    Some(global_workspace.join(sub).join(seg))
}

const MEMORY_SEED: &str = r#"# MEMORY.md — Long-Term Memory (per user)

## Key Facts
(Add important facts here)
"#;

/// Create `USER.md` / `MEMORY.md` if missing so layered bootstrap has files to inject.
///
/// `USER.md` is seeded from `msg` (channel, stable id, optional [`SenderProfileHint`]).
pub async fn seed_per_sender_files(dir: &Path, msg: &ChannelMessage) -> Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let user = dir.join("USER.md");
    if !user.exists() {
        tokio::fs::write(&user, user_md_seed_content(msg)).await?;
    }
    let mem = dir.join("MEMORY.md");
    if !mem.exists() {
        tokio::fs::write(&mem, MEMORY_SEED).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_md_seed_includes_profile_fields() {
        let msg = ChannelMessage {
            id: "1".into(),
            sender: "alice".into(),
            reply_target: "t".into(),
            content: "hi".into(),
            channel: "telegram".into(),
            timestamp: 0,
            thread_ts: None,
            interruption_scope_id: None,
            sender_stable_id: Some("12345".into()),
            sender_profile: Some(super::SenderProfileHint {
                display_name: Some("Alice Example".into()),
                username: Some("alice".into()),
                language_code: Some("en".into()),
            }),
            attachments: vec![],
        };
        let s = user_md_seed_content(&msg);
        assert!(s.contains("telegram"));
        assert!(s.contains("12345"));
        assert!(s.contains("Alice Example"));
        assert!(s.contains("@alice"));
        assert!(s.contains("`en`"));
    }

    #[test]
    fn sanitized_segment_accepts_digits() {
        assert_eq!(
            sanitized_segment("356089143").as_deref(),
            Some("tg_356089143")
        );
        assert_eq!(sanitized_segment(""), None);
        assert_eq!(sanitized_segment("abc"), None);
        assert_eq!(sanitized_segment("12a34"), None);
    }

    #[test]
    fn per_user_workspace_joins_paths() {
        let p = per_user_workspace_dir(Path::new("/w"), "per_sender_workspaces", "123");
        assert_eq!(p, Some(PathBuf::from("/w/per_sender_workspaces/tg_123")));
    }
}
