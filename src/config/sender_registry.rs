//! SQLite-backed registry mapping `stable_id` (e.g. Telegram user digits) → random UUID v4.
//!
//! Used by per-sender workspace isolation so that sandbox folder names are
//! opaque UUIDs instead of `tg_<chat_id>`.
//!
//! Follows the `heartbeat/store.rs` pattern: fresh connection per call, WAL
//! mode, idempotent schema creation.

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// DB file lives inside the per-sender subdir, next to the sandbox folders.
fn db_path(workspace_dir: &Path, per_sender_subdir: &str) -> PathBuf {
    workspace_dir
        .join(per_sender_subdir.trim_matches('/'))
        .join("registry.db")
}

/// Open the registry DB, ensure schema exists, run `f`.
fn with_connection<T>(
    workspace_dir: &Path,
    per_sender_subdir: &str,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let path = db_path(workspace_dir, per_sender_subdir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let conn = Connection::open(&path)
        .with_context(|| format!("Failed to open sender registry DB at {}", path.display()))?;

    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;

         CREATE TABLE IF NOT EXISTS sender_registry (
            stable_id   TEXT PRIMARY KEY,
            uuid        TEXT NOT NULL,
            created_at  TEXT NOT NULL
         );
         CREATE UNIQUE INDEX IF NOT EXISTS idx_sender_registry_uuid
            ON sender_registry(uuid);",
    )?;

    f(&conn)
}

/// Get the existing UUID for `stable_id`, or insert a new one.
///
/// Returns `(uuid_string, was_just_created)`.
pub fn get_or_create_uuid(
    workspace_dir: &Path,
    per_sender_subdir: &str,
    stable_id: &str,
) -> Result<(String, bool)> {
    with_connection(workspace_dir, per_sender_subdir, |conn| {
        // Try to find existing.
        let existing: Option<String> = conn
            .query_row(
                "SELECT uuid FROM sender_registry WHERE stable_id = ?1",
                params![stable_id],
                |row| row.get(0),
            )
            .ok();

        if let Some(uuid) = existing {
            return Ok((uuid, false));
        }

        let uuid = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        // INSERT OR IGNORE handles the unlikely race where another process
        // inserted the same stable_id between our SELECT and INSERT.
        conn.execute(
            "INSERT OR IGNORE INTO sender_registry (stable_id, uuid, created_at)
             VALUES (?1, ?2, ?3)",
            params![stable_id, uuid, now],
        )
        .context("Failed to insert sender registry entry")?;

        // Re-read in case INSERT OR IGNORE was a no-op (race condition).
        let actual: String = conn.query_row(
            "SELECT uuid FROM sender_registry WHERE stable_id = ?1",
            params![stable_id],
            |row| row.get(0),
        )?;

        let created = actual == uuid;
        Ok((actual, created))
    })
}

/// Look up the UUID for a `stable_id`. Returns `None` if not registered.
pub fn lookup_uuid(
    workspace_dir: &Path,
    per_sender_subdir: &str,
    stable_id: &str,
) -> Result<Option<String>> {
    with_connection(workspace_dir, per_sender_subdir, |conn| {
        let result = conn
            .query_row(
                "SELECT uuid FROM sender_registry WHERE stable_id = ?1",
                params![stable_id],
                |row| row.get::<_, String>(0),
            )
            .ok();
        Ok(result)
    })
}

/// Reverse lookup: find the `stable_id` for a given UUID.
pub fn lookup_stable_id(
    workspace_dir: &Path,
    per_sender_subdir: &str,
    uuid: &str,
) -> Result<Option<String>> {
    with_connection(workspace_dir, per_sender_subdir, |conn| {
        let result = conn
            .query_row(
                "SELECT stable_id FROM sender_registry WHERE uuid = ?1",
                params![uuid],
                |row| row.get::<_, String>(0),
            )
            .ok();
        Ok(result)
    })
}

/// Scan for legacy `tg_<digits>` folders and migrate them to UUID-based names.
///
/// For each migrated folder:
/// 1. Generate a UUID v4 and register it.
/// 2. Rename the directory.
/// 3. Update the memory namespace in `brain.db` (best-effort).
///
/// Returns the count of migrated folders.
pub fn migrate_legacy_folders(workspace_dir: &Path, per_sender_subdir: &str) -> Result<usize> {
    let base_dir = workspace_dir.join(per_sender_subdir.trim_matches('/'));
    if !base_dir.exists() {
        return Ok(0);
    }

    let entries = std::fs::read_dir(&base_dir)
        .with_context(|| format!("Failed to read {}", base_dir.display()))?;

    let mut count = 0usize;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Skipping unreadable dir entry: {e}");
                continue;
            }
        };

        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Only migrate directories matching tg_<digits>
        if !name_str.starts_with("tg_") {
            continue;
        }
        let digits = &name_str[3..];
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        let stable_id = digits;

        // Check if already registered (idempotent)
        if lookup_uuid(workspace_dir, per_sender_subdir, stable_id)?.is_some() {
            // Already in registry — folder should already be renamed.
            // If the tg_ folder still exists, something went wrong previously.
            tracing::warn!(
                "Legacy folder tg_{stable_id} still exists but is already registered — skipping"
            );
            continue;
        }

        let uuid = Uuid::new_v4().to_string();

        // Register the mapping
        if let Err(e) = with_connection(workspace_dir, per_sender_subdir, |conn| {
            let now = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO sender_registry (stable_id, uuid, created_at)
                 VALUES (?1, ?2, ?3)",
                params![stable_id, uuid, now],
            )?;
            Ok(())
        }) {
            tracing::error!("Failed to register tg_{stable_id}: {e}");
            continue;
        }

        // Rename the folder
        let old_path = entry.path();
        let new_path = base_dir.join(&uuid);
        if let Err(e) = std::fs::rename(&old_path, &new_path) {
            tracing::error!(
                "Failed to rename {} -> {}: {e}",
                old_path.display(),
                new_path.display()
            );
            // Attempt to clean up the registry entry so it's retried next time
            let _ = with_connection(workspace_dir, per_sender_subdir, |conn| {
                conn.execute(
                    "DELETE FROM sender_registry WHERE stable_id = ?1",
                    params![stable_id],
                )?;
                Ok::<(), anyhow::Error>(())
            });
            continue;
        }

        // Update memory namespace in brain.db (best-effort)
        let brain_db = workspace_dir.join("memory").join("brain.db");
        if brain_db.exists() {
            if let Ok(conn) = Connection::open(&brain_db) {
                let old_ns = format!("tg_{stable_id}");
                match conn.execute(
                    "UPDATE memories SET namespace = ?1 WHERE namespace = ?2",
                    rusqlite::params![uuid, old_ns],
                ) {
                    Ok(updated) => {
                        if updated > 0 {
                            tracing::info!(
                                "Updated {updated} memory namespace entries: {old_ns} -> {uuid}"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to update memory namespace for tg_{stable_id}: {e}");
                    }
                }
            }
        }

        tracing::info!("Migrated per-sender folder: tg_{stable_id} -> {uuid}");
        count += 1;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workspace() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        // Create the per_sender subdir
        std::fs::create_dir_all(ws.join("sandbox")).unwrap();
        (dir, ws)
    }

    #[test]
    fn test_get_or_create_uuid_new() {
        let (_tmp, ws) = temp_workspace();
        let (uuid, created) = get_or_create_uuid(&ws, "sandbox", "12345").unwrap();
        assert!(created);
        assert!(Uuid::parse_str(&uuid).is_ok(), "Should be a valid UUID");
    }

    #[test]
    fn test_get_or_create_uuid_idempotent() {
        let (_tmp, ws) = temp_workspace();
        let (uuid1, created1) = get_or_create_uuid(&ws, "sandbox", "99999").unwrap();
        let (uuid2, created2) = get_or_create_uuid(&ws, "sandbox", "99999").unwrap();
        assert!(created1);
        assert!(!created2);
        assert_eq!(uuid1, uuid2);
    }

    #[test]
    fn test_lookup_uuid_missing() {
        let (_tmp, ws) = temp_workspace();
        assert_eq!(lookup_uuid(&ws, "sandbox", "00000").unwrap(), None);
    }

    #[test]
    fn test_lookup_stable_id_reverse() {
        let (_tmp, ws) = temp_workspace();
        let (uuid, _) = get_or_create_uuid(&ws, "sandbox", "42").unwrap();
        let found = lookup_stable_id(&ws, "sandbox", &uuid).unwrap();
        assert_eq!(found.as_deref(), Some("42"));
    }

    #[test]
    fn test_migrate_legacy_folders() {
        let (_tmp, ws) = temp_workspace();
        let sandbox = ws.join("sandbox");

        // Create two legacy folders with a dummy file each
        std::fs::create_dir_all(sandbox.join("tg_111").join("sub")).unwrap();
        std::fs::write(sandbox.join("tg_111").join("USER.md"), "user1").unwrap();
        std::fs::create_dir_all(sandbox.join("tg_222")).unwrap();
        std::fs::write(sandbox.join("tg_222").join("USER.md"), "user2").unwrap();

        // Also create a non-tg folder that should be left alone
        std::fs::create_dir_all(sandbox.join("other")).unwrap();

        let count = migrate_legacy_folders(&ws, "sandbox").unwrap();
        assert_eq!(count, 2);

        // Verify tg_* folders no longer exist
        assert!(!sandbox.join("tg_111").exists());
        assert!(!sandbox.join("tg_222").exists());

        // Verify "other" is untouched
        assert!(sandbox.join("other").exists());

        // Verify registry has both entries
        let uuid1 = lookup_uuid(&ws, "sandbox", "111").unwrap().unwrap();
        let uuid2 = lookup_uuid(&ws, "sandbox", "222").unwrap().unwrap();

        // Verify files were preserved
        assert!(sandbox.join(&uuid1).join("USER.md").exists());
        assert!(sandbox.join(&uuid2).join("USER.md").exists());

        // Verify content is preserved
        let content = std::fs::read_to_string(sandbox.join(&uuid1).join("USER.md")).unwrap();
        assert_eq!(content, "user1");
    }

    #[test]
    fn test_migrate_idempotent() {
        let (_tmp, ws) = temp_workspace();
        let sandbox = ws.join("sandbox");

        std::fs::create_dir_all(sandbox.join("tg_333")).unwrap();

        let count1 = migrate_legacy_folders(&ws, "sandbox").unwrap();
        assert_eq!(count1, 1);

        let count2 = migrate_legacy_folders(&ws, "sandbox").unwrap();
        assert_eq!(count2, 0);
    }
}
