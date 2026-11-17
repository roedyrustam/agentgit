# PRD: AgentGit (`agentgit`) — Causal Time-Travel & Shadow Worktree Engine for Multi-Agent AI

**Version:** 1.0.0-draft  
**Author:** Roedy Rustam  
**Date:** 2026-11-18  
**Status:** Approved for Architecture & Scaffolding  

---

## 1. Executive Summary
When multiple AI coding agents (e.g., `vibes-plug` subagents, Antigravity, Claude Code, Cursor, Codex) edit a repository concurrently, standard `git` workflows break down: working trees collide, `git stash` loses causal context, and reverting a single hallucinated agent turn risks wiping valid edits from other agents.

**AgentGit** is a **local-first, zero-daemon Causal Time-Travel & Shadow Worktree Engine** written in Rust. It isolates parallel agent writes into ephemeral Git shadow worktrees (`refs/agentgit/*`), records a causal execution graph (**Prompt -> Tool Call -> File Diff -> Verification Gate**) in embedded SQLite, and exposes surgical per-agent rollback and semantic merge capabilities via **CLI**, **MCP v1.x (`stdio`)**, and a **Local Sidecar Cockpit UI**.

---

## 2. Target Users & Personas
1. **Swarm-Native Engineers:** Developers running 2–9 parallel subagents who need isolated execution and clean, atomic commits.
2. **IDE Plugin & Agent Builders:** Maintainers of `vibes-plug`, `ctxbridge`, and custom MCP orchestrators needing a deterministic safety net below agent file writes.
3. **Solo & Team Reviewers:** Engineers auditing *why* an AI agent changed a specific file, including the prompt hash, tool call, and test/lint verdict.

---

## 3. Core Functional Requirements (FR)

### FR-1: Shadow Worktree Isolation & Ephemeral Refs
- `agentgit spawn --agent <id> [--base <ref>]`: Creates an isolated Git worktree under `.agentgit/worktrees/<agent-id>` linked to `refs/agentgit/agents/<agent-id>`.
- Zero pollution of `refs/heads/*` or human `git status` until explicitly promoted.
- Automatic cleanup of orphaned worktrees via `agentgit gc`.

### FR-2: Causal Checkpoint Engine (`<15 ms` Target)
- `agentgit checkpoint --agent <id> --tool <name> --prompt-hash <sha256> [--parent <node_id>]`: Captures an immutable Git tree snapshot without touching the human branch, storing causal metadata in `.agentgit/state.db` (SQLite WAL).
- Tracks `agent_id`, `parent_node_id`, `tree_oid`, `commit_oid`, `prompt_hash`, `tool_name`, `files_changed`, `token_cost_usd`, and `gate_status` (`pending | passed | failed`).

### FR-3: Surgical Causal Rollback & Replay
- `agentgit rollback --node <node_id>` or `--agent <id> --steps <n>`: Reverts only the target node and its causal descendants in the DAG while preserving orthogonal edits from other agents.
- `agentgit diff <node_a> [node_b]`: Semantic + file-level diff between any two causal nodes or between a shadow worktree and `HEAD`.

### FR-4: Quality-Gated Semantic Merge & Promotion
- `agentgit verify --agent <id> --cmd "<test_or_lint_cmd>"`: Runs verification (e.g., `cargo test`, `vitest`, `anti-slop`) inside the shadow worktree and stamps `gate_status` on the causal node.
- `agentgit promote --agent <id> [--squash]`: Promotes verified shadow commits to the active human branch with conflict pre-flight detection.

### FR-5: Native MCP v1.x Server (`agentgit mcp`)
Exposes deterministic tools over `stdio`:
- `agentgit_spawn_shadow`
- `agentgit_checkpoint`
- `agentgit_causal_log`
- `agentgit_diff_node`
- `agentgit_rollback_causal`
- `agentgit_verify_and_promote`

### FR-6: Local Sidecar Cockpit UI (`agentgit ui`)
- Local HTTP/SSE server (`127.0.0.1:<port>`) with ephemeral bearer token for mutating actions.
- Interactive Causal DAG timeline, split diff viewer, per-agent token/gate badges, and 1-click rollback/promote.

---

## 4. Mandatory Legal & Privacy Trio (Compliance Invariant)
To comply with **UU PDP No. 27/2022**, **GDPR Art. 13 & 17**, and IDE/Marketplace distribution policies, the Sidecar Web UI and companion documentation portal MUST implement:
1. **`/privacy` (Privacy Policy):** Explicit disclosure that AgentGit is 100% local-first; code diffs, prompts, and `.agentgit/state.db` never leave `127.0.0.1`.
2. **`/terms` (Terms & Conditions):** MIT/Apache-2.0 usage terms, liability boundaries for automated Git operations, and user responsibility for `promote`/`rollback`.
3. **`/delete-account` (User Data Deletion):** Local data purge endpoint (`POST /api/v1/purge`) and UI flow to permanently wipe `.agentgit/state.db`, shadow worktrees, and `refs/agentgit/*` with confirmation.

---

## 5. Non-Functional Requirements (NFR)
- **CLI Cold Start:** `< 10 ms` (single static Rust binary).
- **Checkpoint Latency:** `< 15 ms` for repositories up to 50,000 tracked files (using Git index/tree-object hashing).
- **Concurrency:** Safe concurrent writes from up to 16 parallel agents via SQLite `WAL` mode + `busy_timeout = 5000ms` + per-agent worktree locks.
- **Security:** Strictly `127.0.0.1` binding for Sidecar UI; parameterized SQL queries only; path traversal guard on all worktree paths; zero external telemetry.
- **Cross-Platform:** First-class support for Windows (NTFS long paths & file locking), macOS, and Linux.

---

## 6. Success Metrics & Exit Criteria
- Zero uncommitted human worktree corruption during 8-agent concurrent stress tests.
- 100% of causal rollbacks leave unrelated agent branches intact.
- Seamless plug-and-play installation via `cargo install agentgit`, `npx agentgit`, and `vibes-plug` MCP registration.
