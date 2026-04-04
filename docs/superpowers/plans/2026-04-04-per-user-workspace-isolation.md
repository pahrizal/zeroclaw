# Per-user workspace isolation — implementation plan

> **For agentic workers:** Execute steps with checkbox tracking; prefer small commits per task.

**Goal:** Implement per-sender (e.g. Telegram `sender_id`) workspace profiles with full isolation (memory, sessions, audit), dual-root bootstrap (global `IDENTITY`/`SOUL`/`AGENTS` + shared skills; per-user `USER` overlay + `MEMORY`), Option A two-part `USER` injection, and strict per-sender conversation state so user B never sees user A’s chat.

**Architecture:** Introduce **global workspace root** (existing main `workspace_dir`) and **per-user root** under `workspaces_dir`. Resolve per message: `sanitize(tg_<sender_id>)` → profile dir. Extend prompt building to load bootstrap files from both roots; avoid a single cached `Arc<String>` system prompt for content that varies per user when isolation is enabled. Thread **effective paths** through channel message handling and tools that read/write workspace files. Use **per-request** workspace resolution (no shared mutable “active workspace” for concurrent users).

**Tech stack:** Rust (zeroclaw-upstream), existing `WorkspaceProfile` / `WorkspaceManager`, `channels/mod.rs` prompt and memory paths.

**Spec:** `docs/superpowers/specs/2026-04-04-per-user-workspace-isolation-design.md`

---

## File map (expected touch points)

| Area | Files / modules |
|------|------------------|
| Config | `src/config/schema.rs` (`WorkspaceConfig` or channel-level flags), TOML examples |
| Workspace | `src/config/workspace.rs` — auto-create profile by sanitized name; load without exclusive global `active` for gateway |
| Prompt bootstrap | `src/channels/mod.rs` — `load_openclaw_bootstrap_files` or new `load_layered_bootstrap_files(global, per_user, …)`; `build_system_prompt*` call sites |
| Channel runtime | `src/channels/mod.rs` — `ChannelRuntimeContext`: optional `global_workspace_dir` + resolve `effective_workspace` per `ChannelMessage` |
| Gateway init | Where `ChannelRuntimeContext` is constructed (~5600+) — wire config, split paths |
| Memory / sessions | Memory adapter namespace, `session_store` paths — ensure they use **per-user** root when isolation enabled |
| Tools | Shell / file tools — `cwd` or workspace root must follow per-user dir |
| Telegram | `src/channels/telegram.rs` — ensure `sender_id` is always present for keying; document `allowed_message_thread_ids` vs this feature |
| Tests | `tests/integration/` or unit tests under `src/channels/` for keys and path sanitization |
| Redaction | `src/security/channel_event_redaction.rs` — patterns for new paths if needed |

---

### Task 1: Config and sender key contract

**Files:** `src/config/schema.rs`, example `config.toml` fragment in spec or docs.

- [ ] Add explicit flags, e.g. `workspace.per_sender_isolation` (bool) and document interaction with `workspace.enabled`.
- [ ] Define **sanitization rules** for directory names from `sender_id` (document: only `tg_<digits>` or allow hex for non-Telegram IDs later).
- [ ] Add unit tests for sanitization (reject `..`, slashes, empty).

---

### Task 2: Resolve per-user profile path (no global switch for concurrency)

**Files:** `src/config/workspace.rs`, new small helper e.g. `src/config/per_sender_workspace.rs` (optional).

- [ ] Implement `resolve_profile_dir(workspaces_dir, channel, sender_key) -> PathBuf` (or Telegram-specific helper if YAGNI for other channels).
- [ ] `ensure_profile_exists` — create dir + default `profile.toml` + seed `USER.md` / `MEMORY.md` templates if missing.
- [ ] Do **not** require calling `WorkspaceManager::switch` for each message; prefer **read-only lookup** of `WorkspaceProfile` by name or load `profile.toml` from resolved path per request.

---

### Task 3: Dual-root bootstrap injection (Option A)

**Files:** `src/channels/mod.rs` (`load_openclaw_bootstrap_files`, `build_system_prompt_with_mode_and_autonomy` or inner sections).

- [ ] Add `load_layered_user_and_memory(prompt, global_dir, per_user_dir, max_chars)`:
  - From **global**: `AGENTS.md`, `SOUL.md`, `TOOLS.md`, `IDENTITY.md`, then **global** `USER.md` under heading `### Global user context (shared)`.
  - From **per_user**: `USER.md` under `### This user (overlay)`.
  - From **per_user only**: `MEMORY.md`.
- [ ] Preserve order consistent with current `load_openclaw_bootstrap_files` where possible (`AGENTS` … `IDENTITY`, then layered USER, `BOOTSTRAP` from global if present, `MEMORY` from per-user).
- [ ] Unit test: two different `per_user` dirs produce different injected `MEMORY` / overlay text; global `IDENTITY` identical.

---

### Task 4: System prompt caching vs per-user variance

**Files:** `src/channels/mod.rs` (~5400–5600 init, ~2760 LLM path).

- [ ] When `per_sender_isolation` is on, **split** static prompt parts (tools, skills, safety, global identity files without per-user MEMORY) from **per-turn/per-user** parts, OR rebuild the variable section each message for that sender.
- [ ] Ensure `had_prior_history` branch still uses correct base: per-user bootstrap must not leak between senders.
- [ ] Performance: cache **global-only** `Arc<str>` fragment; append per-user fragment each call if needed.

---

### Task 5: Memory, sessions, audit namespaces

**Files:** memory constructors in channel setup (`SqliteMemory::new_named`, etc.), session store wiring.

- [ ] When isolation enabled, bind memory namespace / DB file paths to **per-user profile** (`WorkspaceProfile.effective_memory_namespace()` or `tg_<id>`).
- [ ] Verify `build_memory_context` scopes: align with spec — user B must not receive user A’s recalled snippets (namespace separation).
- [ ] Re-read group-chat dual recall (`sender` vs `history_key`); confirm `history_key` includes `sender` so “group” branch does not merge users (document in code comment if confusing).

---

### Task 6: Tools and attachment paths

**Files:** shell tool, `telegram.rs` attachment path (`workspace_dir/telegram_files/`), any `workspace_dir.join(...)`.

- [ ] Pass **per-user** workspace as effective cwd for file/shell operations when isolation is on.
- [ ] Telegram downloads: `telegram_files/` under per-user root (or subfolder) to avoid collisions.

---

### Task 7: Interruption / in-flight tasks

**Files:** `InFlightSenderTaskState`, debouncer keys in `channels/mod.rs`.

- [ ] Confirm in-flight work is keyed by **sender-scoped** key (already `interruption_scope_key` includes `sender`); add tests or assertions when isolation flag is on.
- [ ] Document: user B does not cancel user A’s run unless product requires it (default: independent).

---

### Task 8: Integration tests

- [ ] Simulate two senders same `chat_id`: alternating messages — assert distinct session files / memory namespaces / prompt MEMORY content.
- [ ] Regression: isolation off → current single-workspace behavior unchanged.

---

### Task 9: Docs and operator notes

- [ ] Document new TOML keys in upstream docs or README snippet.
- [ ] Note: `thread_not_allowed` is unrelated — operators need correct `allowed_message_thread_ids` for forum groups.

---

## Verification

- `cargo test` (scoped crates / packages as per repo convention).
- Manual: Telegram supergroup with two accounts sending alternately — no cross-memory in responses.

---

## Dependencies

- Implementation order: Task 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9.
