//! Redaction for outbound channel events.
//!
//! Goal: prevent leaking ZeroClaw internal workspace files (e.g. AGENTS.md,
//! USER.md, SOUL.md, skills/, telegram_files/) through any channel/event payload
//! when the owner requests privacy. This is enabled by default via
//! `security.redact_channel_events`.

use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

/// Replace sensitive internal file paths with a stable placeholder.
///
/// This is intentionally conservative: if we *recognize* an internal marker
/// anywhere in a string, we redact the full path-like token containing it.
pub fn redact_internal_paths_in_text_with_config(
    input: &str,
    cfg: &crate::config::schema::ChannelEventRedactionConfig,
) -> String {
    static PATH_TOKEN_RE: OnceLock<Regex> = OnceLock::new();
    static MARKER_IN_BRACKETS_RE: OnceLock<Regex> = OnceLock::new();

    // A "token" that often represents a path (keeps it simple: stop at whitespace or common delimiters).
    let token_re = PATH_TOKEN_RE.get_or_init(|| {
        // NOTE: Rust regex does not permit `\s` inside a character class.
        // Use an explicit whitespace set instead.
        //
        // This must never panic in production; fall back to a conservative token
        // matcher if the preferred pattern fails to compile.
        Regex::new(
            r#"(?x)
                (?P<token>
                    (?:file://)?
                    (?:[A-Za-z]:[\\/]|/)?              # optional drive or unix root
                    [^\t\r\n \]\)\}>,"]{1,512}          # path-ish (no whitespace/delims)
                )
            "#,
        )
        .unwrap_or_else(|_| {
            // Fallback: any non-whitespace run up to a sensible cap.
            Regex::new(r#"(?P<token>\S{1,512})"#).expect("fallback regex must compile")
        })
    });

    // Handle media markers like `[IMAGE:/path]` or `[Document: name] /path` by
    // redacting the embedded token, not the whole sentence.
    let bracket_re = MARKER_IN_BRACKETS_RE.get_or_init(|| {
        Regex::new(r"(?i)\[(IMAGE|VIDEO|VOICE|AUDIO|DOCUMENT|FILE):([^\]]+)\]")
            .expect("valid regex")
    });

    let globset = build_globset(&cfg.sensitive_globs);
    let lowered_markers: Vec<String> = cfg
        .sensitive_markers
        .iter()
        .map(|m| m.to_ascii_lowercase())
        .collect();

    // First, redact any path tokens that include an internal marker.
    let mut out = token_re
        .replace_all(input, |caps: &regex::Captures<'_>| {
            let token = caps.name("token").map(|m| m.as_str()).unwrap_or("");
            if is_sensitive_token(token, &globset, &lowered_markers) {
                "[REDACTED_INTERNAL_PATH]".to_string()
            } else {
                token.to_string()
            }
        })
        .to_string();

    // Then, ensure media markers don't leak internal paths inside the brackets.
    out = bracket_re
        .replace_all(&out, |caps: &regex::Captures<'_>| {
            let kind = caps.get(1).map(|m| m.as_str()).unwrap_or("FILE");
            let inner = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            if is_sensitive_token(inner, &globset, &lowered_markers) {
                format!("[{kind}:[REDACTED_INTERNAL_PATH]]")
            } else {
                caps.get(0).map(|m| m.as_str()).unwrap_or("").to_string()
            }
        })
        .to_string();

    out
}

/// Backward-compatible wrapper using the default config.
pub fn redact_internal_paths_in_text(input: &str) -> String {
    redact_internal_paths_in_text_with_config(
        input,
        &crate::config::schema::ChannelEventRedactionConfig::default(),
    )
}

/// Recursively redact internal paths in any JSON event payload.
pub fn redact_internal_paths_in_json_with_config(
    value: &mut Value,
    cfg: &crate::config::schema::ChannelEventRedactionConfig,
) {
    match value {
        Value::String(s) => {
            let redacted = redact_internal_paths_in_text_with_config(s, cfg);
            if redacted != *s {
                *s = redacted;
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_internal_paths_in_json_with_config(item, cfg);
            }
        }
        Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                redact_internal_paths_in_json_with_config(v, cfg);
            }
        }
        _ => {}
    }
}

/// Backward-compatible wrapper using the default config.
pub fn redact_internal_paths_in_json(value: &mut Value) {
    redact_internal_paths_in_json_with_config(
        value,
        &crate::config::schema::ChannelEventRedactionConfig::default(),
    );
}

fn build_globset(globs: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for g in globs {
        // Ignore invalid globs (fail-soft): redaction is best-effort and should not crash.
        if let Ok(glob) = Glob::new(g) {
            builder.add(glob);
        }
    }
    builder
        .build()
        .unwrap_or_else(|_| GlobSetBuilder::new().build().expect("empty globset"))
}

fn normalize_token_for_matching(token: &str) -> String {
    let token = token.trim().trim_matches(|c| matches!(c, '`' | '"' | '\''));
    let token = token.strip_prefix("file://").unwrap_or(token);
    token.replace('\\', "/")
}

fn is_sensitive_token(token: &str, globset: &GlobSet, lowered_markers: &[String]) -> bool {
    let norm = normalize_token_for_matching(token);
    if globset.is_match(&norm) {
        return true;
    }
    let lowered = norm.to_ascii_lowercase();
    lowered_markers.iter().any(|m| lowered.contains(m))
}
