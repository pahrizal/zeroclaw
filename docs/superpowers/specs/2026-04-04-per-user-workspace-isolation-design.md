# Per-user workspace isolation (ZeroClaw / Telegram)

**Status:** Approved for implementation planning  
**Date:** 2026-04-04

## Summary

Enable **Option B** isolation (memory, sessions, audit per user) while keeping a **single global** source for `IDENTITY.md`, `SOUL.md`, `AGENTS.md`, and **shared skills**. Per-user profile holds **layered `USER.md`** (global + per-user, **Option A**: two-part prompt injection) and **per-user `MEMORY.md`**. **Conversation context is strictly per sender**: when user B messages while user A has an active thread, B is a **new conversation** with **no access** to A’s chat history or in-flight session state.

## Goals

1. **Stable identity key:** Prefer Telegram `sender_id` (numeric) for directory names and namespaces; username is optional metadata only.
2. **Full isolation:** Per-user memory namespace, sessions, and audit (aligned with existing `WorkspaceProfile` / `WorkspaceConfig` semantics).
3. **Shared brain, layered user:** Global `IDENTITY.md` / `SOUL.md` / `AGENTS.md` + shared skills; **inject `USER` as two labeled sections** (global base + per-user extension); inject **per-user `MEMORY.md` only**.
4. **Interleaved chats:** User B never inherits user A’s **conversation history**, **rolling context**, or **ephemeral session** tied to A. Implementation must use **per-request (or per-sender) workspace context** and history keys that **always include sender**; no process-wide “active workspace” for concurrent multi-user handling.

## Non-goals (v1)

- Disk quotas, max user count, or rate limits (unless added later).
- Symlink-based “shadow” workspaces (avoid fragility).
- Copying global identity files into each user tree (avoid drift).

## Architecture

### Dual workspace roots

| Role | Path | Contents |
|------|------|----------|
| **Global root** | Configured main workspace dir (e.g. `workspace/` or `config/zeroclaw/workspace`) | `IDENTITY.md`, `SOUL.md`, `AGENTS.md`, `TOOLS.md`, optional `BOOTSTRAP.md`, **base** `USER.md`, shared `skills/` resolution |
| **Per-user root** | Under `workspaces_dir`, e.g. `…/workspaces/tg_<sender_id>/` | `profile.toml`, **per-user** `USER.md`, **per-user** `MEMORY.md`, user-scoped state |

Bootstrap / system prompt construction **reads identity trio and global USER from global root**; **injects MEMORY from per-user root only**; **injects USER as Option A**: concatenation with clear headings, e.g. `### Global user context (shared)` then `### This user (Telegram)` before per-user content.

### Sender resolution (Telegram)

- **Canonical key:** `sender_id` → sanitized directory segment `tg_<id>`.
- **Username:** Log/display only; do not use as sole primary key.

### Conversation isolation (“clear chat” between users)

**Requirement:** If user A is mid-conversation and user B sends a message, B must be treated as a **new, separate conversation** relative to A: **no shared chat transcript, no shared follow-up buffer** that could leak A’s context to B.

**Design intent:**

- **History / route keys** must continue to include **`msg.sender`** (already true for `conversation_history_key` in `channels/mod.rs`). Any change to workspace switching must **not** collapse history to channel-only.
- **Workspace manager / active profile:** For multi-user concurrency, the implementation **must not** rely on a single global “active workspace” mutation per message without scoping; prefer **per-message or per-async-task context** carrying `(global_workspace_dir, per_user_workspace_dir, profile)` for the resolved sender.
- **Follow-ups / interruptions:** Keys such as `interruption_scope_key` already include `sender`; verify all session-scoped state respects the same sender-scoped key when this feature is enabled.

### Configuration

- New or extended config flags, e.g. `workspace.enabled`, channel-level or global **`per_sender_isolation`** (exact name TBD in implementation), pointing `workspaces_dir` at the per-user profile parent directory.
- Auto-provision: on first message from a new sender, create per-user dir, `profile.toml`, and optional seed `USER.md` / `MEMORY.md` if missing.

## Security notes

- Sanitize sender-derived path segments; reject path traversal.
- Redaction / channel event policies (`channel_event_redaction`) may need to include per-user paths consistently.

## Testing (high level)

- Unit tests: path sanitization, dual-root bootstrap text (order and headings for USER).
- Integration: two simulated senders on same `chat_id` / thread; assert distinct memory namespaces and no cross-read of history buffers.

## References (code)

- `conversation_history_key`, `interruption_scope_key`: `zeroclaw-upstream/src/channels/mod.rs`
- `load_openclaw_bootstrap_files` / `build_system_prompt`: `zeroclaw-upstream/src/channels/mod.rs`
- `WorkspaceProfile`, `WorkspaceManager`: `zeroclaw-upstream/src/config/workspace.rs`
- `WorkspaceConfig`: `zeroclaw-upstream/src/config/schema.rs`

## Approval

- Sections 1–5 from design review: **approved**
- USER layering: **Option A** (two-part prompt injection)
- Interleaved users: **B is a new conversation** (no shared chat with A)

---

*Next step: implementation plan under `docs/superpowers/plans/` (see writing-plans skill).*
