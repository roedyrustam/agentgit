# AgentGit Documentation

## Overview
**AgentGit** (`agentgit`) is a local-first Causal Time-Travel & Shadow Worktree Engine for Multi-Agent AI workflows. It isolates parallel agent file edits into ephemeral Git shadow worktrees (`refs/agentgit/*`) and tracks causal execution graphs (**Prompt -> Tool Call -> File Diff -> Verification Gate**) in embedded SQLite (WAL + FTS5).

## Quick Start

```bash
# 1. Initialize AgentGit in any Git repository
agentgit init

# 2. Spawn isolated shadow worktrees for parallel agents
agentgit spawn --agent frontend-1 --role "UI Engineer"
agentgit spawn --agent backend-1 --role "API Engineer"

# 3. Capture causal checkpoints inside each agent's worktree
agentgit checkpoint --agent frontend-1 --tool write_file --summary "Add login form" --prompt "Build login"

# 4. Inspect causal DAG & search
agentgit log --limit 20
agentgit log --search "login"

# 5. Verify with tests/linters inside the shadow worktree
agentgit verify --agent frontend-1 --cmd "npm test"

# 6. Surgical rollback (leaves other agents intact)
agentgit rollback --node <NODE_ID>

# 7. Promote verified changes to human HEAD
agentgit promote --agent frontend-1 --squash

# 8. Launch MCP v1.x stdio server or Local Sidecar UI
agentgit mcp
agentgit ui --port 4317
```

## MCP v1.x Integration
Add to your IDE's MCP configuration (`mcp_config.json` / `.cursor/mcp.json` / `.claude.json`):

```json
{
  "mcpServers": {
    "agentgit": {
      "command": "agentgit",
      "args": ["mcp"]
    }
  }
}
```

## Legal & Privacy Routes (Sidecar UI)
- `/privacy` — Local-first privacy policy (UU PDP No. 27/2022 & GDPR Art. 13)
- `/terms` — Terms & conditions
- `/delete-account` — Permanent local `.agentgit/` data purge (`POST /api/v1/purge`)
