//! Task-local override for the effective workspace root during channel tool execution.
//!
//! When `[workspace] per_sender_isolation` is enabled, the agent loop runs tools with
//! [`crate::security::SecurityPolicy::effective_workspace_dir`] set to the per-sender
//! directory (e.g. `{workspace}/{per_sender_subdir}/tg_<id>/`) instead of the global
//! workspace root.

use std::path::PathBuf;

tokio::task_local! {
    /// When `Some`, file/shell tools resolve relative paths and cwd against this directory.
    pub static CHANNEL_WORKSPACE_OVERRIDE: Option<PathBuf>;
}

/// Returns the per-channel workspace override when the current async task is inside
/// [`CHANNEL_WORKSPACE_OVERRIDE::scope`].
#[must_use]
pub fn channel_workspace_override() -> Option<PathBuf> {
    CHANNEL_WORKSPACE_OVERRIDE
        .try_with(|o: &Option<PathBuf>| o.clone())
        .ok()
        .flatten()
}
