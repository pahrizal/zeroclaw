# Per-sender workspace isolation

This feature gives each messaging user (e.g. each Telegram user id) a **separate** long-term memory namespace and on-disk area for **per-user `USER.md` and `MEMORY.md`**, while keeping **one shared** global workspace for `IDENTITY.md`, `SOUL.md`, `AGENTS.md`, and skills.

## Requirements

- OpenClaw-style bootstrap files in the **global** workspace directory (the usual `workspace_dir` from config).
- Channels that populate `sender_stable_id` on inbound messages (Telegram sets it from the numeric `from.id`).

## Configuration (`[workspace]`)

| Key | Default | Meaning |
|-----|---------|---------|
| `per_sender_isolation` | `false` | Master switch. When `true`, per-sender layering and memory namespacing are enabled when `sender_stable_id` is present. |
| `per_sender_subdir` | `"per_sender_workspaces"` | Directory **under** the global workspace root. Each sender gets a subdirectory `tg_<digits>/` (digits = stable id, e.g. Telegram user id). |

Example:

```toml
[workspace]
enabled = false
workspaces_dir = "~/.zeroclaw/workspaces"
isolate_memory = true
isolate_secrets = true
isolate_audit = true
cross_workspace_search = false

# Per-sender isolation (Telegram user id → tg_<id> under workspace)
per_sender_isolation = true
per_sender_subdir = "per_sender_workspaces"
```

## Behavior

1. **On disk:** `{workspace_dir}/{per_sender_subdir}/tg_<user_id>/` is created on first message. Seed files are written if missing: per-user `USER.md`, `MEMORY.md`. The initial `USER.md` includes a **Sender snapshot** from the inbound message (channel, stable id, display identity, and—on Telegram—name, `@username`, and client language from the Bot API `from` object). If `USER.md` already exists, it is not overwritten.
2. **System prompt:** Global `IDENTITY.md`, `SOUL.md`, `AGENTS.md`, `TOOLS.md` come from the global workspace. **Global `USER.md`** plus **per-user `USER.md`** and **per-user `MEMORY.md`** are injected into the prompt for that turn (two-part USER overlay).
3. **Memory:** Vector/SQLite memory operations use a **namespace** per sender (`tg_<id>`), so recall/store do not mix users.
4. **Conversation history:** Still keyed by existing sender + channel + thread keys; different users do not share chat history.
5. **Tools and shell:** While handling a message with `sender_stable_id` and `per_sender_isolation = true`, the agent runs the tool loop with an **effective workspace** set to `{workspace_dir}/{per_sender_subdir}/tg_<id>/`. Relative paths, shell cwd, `glob_search`, `content_search`, and similar tools resolve against that directory. Absolute paths may still reach anywhere allowed by `[autonomy]` (including the global workspace root). `SecurityPolicy::prompt_summary` reflects the effective root during that turn.
6. **Telegram downloads:** Incoming documents/photos are saved under `{effective_workspace}/telegram_files/` (per user when isolation is on).

## AIEOS identity

If **AIEOS** identity is configured, the daemon uses a **Full** bootstrap (not IdentityOnly) to avoid conflicting with the AIEOS identity block. Per-user **overlay** still applies: only per-user `USER.md` / `MEMORY.md` are appended from the per-sender directory, not a second copy of global `USER.md`.

## Limits and follow-ups

- **Cross-user absolute paths:** Policy still allows paths under the global `workspace_dir`; a user could in theory reference another sender’s subdirectory by absolute path. Tightening this is optional hardening.
- **Sessions / audit** per sender are not part of this document; see the implementation plan for remaining tasks.

## Telegram forum threads

Messages filtered with `thread_not_allowed` are unrelated to this feature; fix `allowed_message_thread_ids` / topic configuration separately.
