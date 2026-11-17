# AgentGit (`agentgit`)

**Local-First Causal Time-Travel & Shadow Worktree Engine for Multi-Agent AI**  
*Single static Rust binary · Git Shadow Worktrees · Embedded SQLite WAL/FTS5 · MCP v1.x · Local Sidecar Cockpit*

## Why AgentGit?
When multiple AI coding agents edit a repository in parallel, standard `git` workflows suffer from working-tree collisions, lost causal context, and coarse rollbacks. **AgentGit** isolates each agent in an ephemeral Git shadow worktree (`refs/agentgit/agents/<id>`), records every **Prompt -> Tool Call -> File Diff -> Gate Verdict** in SQLite, and lets you surgically roll back or promote verified work without touching human `HEAD`.

## Features
- **Zero-Pollution Shadow Worktrees (`agentgit spawn`)**: Parallel agents write to isolated `.agentgit/worktrees/<id>` directories sharing `.git/objects`.
- **Sub-15ms Atomic Checkpoints (`agentgit checkpoint`)**: Snapshots via isolated `GIT_INDEX_FILE` without mutating human staging or branches.
- **Causal DAG & FTS5 Search (`agentgit log`)**: Query by agent, prompt summary, or tool name.
- **Surgical Causal Rollback (`agentgit rollback`)**: Reverts a target node and its causal descendants while leaving orthogonal agents intact.
- **Quality-Gated Promote (`agentgit verify` & `agentgit promote`)**: Runs tests inside the shadow worktree and executes `git merge-tree --write-tree` conflict pre-flight before touching `HEAD`.
- **Native MCP v1.x Server (`agentgit mcp`)**: Ready for Antigravity, Claude Code, Cursor, and OpenAI Codex.
- **Local Sidecar Cockpit (`agentgit ui`)**: Dark precision telemetry UI at `http://127.0.0.1:4317` with `/privacy`, `/terms`, and `/delete-account`.

## Build & Test
```bash
cargo test
cargo build --release
```
