use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{AgentGitError, Result};

/// Returns the current Unix timestamp in milliseconds.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub repo_root: String,
    pub base_branch: String,
    pub base_commit_oid: String,
    pub status: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub model_name: Option<String>,
    pub worktree_path: String,
    pub ref_name: String,
    pub status: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalNodeRecord {
    pub id: String,
    pub session_id: String,
    pub agent_id: String,
    pub parent_node_id: Option<String>,
    pub tree_oid: String,
    pub commit_oid: String,
    pub prompt_hash: String,
    pub summary: String,
    pub tool_name: String,
    pub token_cost_usd: f64,
    pub gate_status: String,
    pub is_rolled_back: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTouchInput {
    pub file_path: String,
    pub change_type: String,
    pub lines_added: i64,
    pub lines_deleted: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTouchRecord {
    pub id: i64,
    pub node_id: String,
    pub file_path: String,
    pub change_type: String,
    pub lines_added: i64,
    pub lines_deleted: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateRunRecord {
    pub id: String,
    pub node_id: String,
    pub command: String,
    pub exit_code: i64,
    pub duration_ms: i64,
    pub output_excerpt: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagSnapshot {
    pub session: SessionRecord,
    pub agents: Vec<AgentRecord>,
    pub nodes: Vec<CausalNodeRecord>,
    pub file_touches: Vec<FileTouchRecord>,
    pub gate_runs: Vec<GateRunRecord>,
}

/// Embedded SQLite WAL + FTS5 persistence layer for AgentGit.
pub struct StateDb {
    conn: Connection,
}

impl StateDb {
    /// Opens (or creates) `.agentgit/state.db` and applies WAL + schema invariants.
    pub fn open(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            PRAGMA busy_timeout = 5000;
            ",
        )?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                repo_root TEXT NOT NULL,
                base_branch TEXT NOT NULL,
                base_commit_oid TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('active', 'completed', 'aborted')) DEFAULT 'active',
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS agents (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                model_name TEXT,
                worktree_path TEXT NOT NULL,
                ref_name TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('idle', 'working', 'promoted', 'rolled_back')) DEFAULT 'idle',
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS causal_nodes (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                parent_node_id TEXT REFERENCES causal_nodes(id) ON DELETE SET NULL,
                tree_oid TEXT NOT NULL,
                commit_oid TEXT NOT NULL,
                prompt_hash TEXT NOT NULL,
                summary TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                token_cost_usd REAL NOT NULL DEFAULT 0.0,
                gate_status TEXT NOT NULL CHECK (gate_status IN ('pending', 'passed', 'failed')) DEFAULT 'pending',
                is_rolled_back INTEGER NOT NULL CHECK (is_rolled_back IN (0, 1)) DEFAULT 0,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS file_touches (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                node_id TEXT NOT NULL REFERENCES causal_nodes(id) ON DELETE CASCADE,
                file_path TEXT NOT NULL,
                change_type TEXT NOT NULL CHECK (change_type IN ('added', 'modified', 'deleted', 'renamed')),
                lines_added INTEGER NOT NULL DEFAULT 0,
                lines_deleted INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS gate_runs (
                id TEXT PRIMARY KEY,
                node_id TEXT NOT NULL REFERENCES causal_nodes(id) ON DELETE CASCADE,
                command TEXT NOT NULL,
                exit_code INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                output_excerpt TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_agents_session ON agents(session_id);
            CREATE INDEX IF NOT EXISTS idx_nodes_session ON causal_nodes(session_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_nodes_agent ON causal_nodes(agent_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_nodes_parent ON causal_nodes(parent_node_id);
            CREATE INDEX IF NOT EXISTS idx_touches_node ON file_touches(node_id);
            CREATE INDEX IF NOT EXISTS idx_touches_path ON file_touches(file_path);
            CREATE INDEX IF NOT EXISTS idx_gates_node ON gate_runs(node_id);

            CREATE VIRTUAL TABLE IF NOT EXISTS causal_fts USING fts5(
                node_id UNINDEXED,
                agent_id,
                summary,
                tool_name,
                tokenize = 'porter unicode61'
            );
            ",
        )?;
        Ok(())
    }

    pub fn ensure_active_session(
        &self,
        repo_root: &str,
        base_branch: &str,
        base_commit_oid: &str,
    ) -> Result<SessionRecord> {
        if let Some(existing) = self.get_active_session()? {
            return Ok(existing);
        }
        let session = SessionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            repo_root: repo_root.to_string(),
            base_branch: base_branch.to_string(),
            base_commit_oid: base_commit_oid.to_string(),
            status: "active".to_string(),
            created_at: now_ms(),
        };
        self.conn.execute(
            "INSERT INTO sessions (id, repo_root, base_branch, base_commit_oid, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session.id,
                session.repo_root,
                session.base_branch,
                session.base_commit_oid,
                session.status,
                session.created_at
            ],
        )?;
        Ok(session)
    }

    pub fn get_active_session(&self) -> Result<Option<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, repo_root, base_branch, base_commit_oid, status, created_at
             FROM sessions WHERE status = 'active' ORDER BY created_at DESC LIMIT 1",
        )?;
        let row = stmt
            .query_row([], |r| {
                Ok(SessionRecord {
                    id: r.get(0)?,
                    repo_root: r.get(1)?,
                    base_branch: r.get(2)?,
                    base_commit_oid: r.get(3)?,
                    status: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })
            .optional()?;
        Ok(row)
    }

    pub fn insert_agent(&self, agent: &AgentRecord) -> Result<()> {
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM agents WHERE id = ?1",
                params![agent.id],
                |r| r.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(AgentGitError::AgentAlreadyExists(agent.id.clone()));
        }
        self.conn.execute(
            "INSERT INTO agents (id, session_id, role, model_name, worktree_path, ref_name, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                agent.id,
                agent.session_id,
                agent.role,
                agent.model_name,
                agent.worktree_path,
                agent.ref_name,
                agent.status,
                agent.created_at
            ],
        )?;
        Ok(())
    }

    pub fn get_agent(&self, agent_id: &str) -> Result<AgentRecord> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, role, model_name, worktree_path, ref_name, status, created_at
             FROM agents WHERE id = ?1",
        )?;
        stmt.query_row(params![agent_id], |r| {
            Ok(AgentRecord {
                id: r.get(0)?,
                session_id: r.get(1)?,
                role: r.get(2)?,
                model_name: r.get(3)?,
                worktree_path: r.get(4)?,
                ref_name: r.get(5)?,
                status: r.get(6)?,
                created_at: r.get(7)?,
            })
        })
        .optional()?
        .ok_or_else(|| AgentGitError::AgentNotFound(agent_id.to_string()))
    }

    pub fn list_agents(&self, session_id: &str) -> Result<Vec<AgentRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, role, model_name, worktree_path, ref_name, status, created_at
             FROM agents WHERE session_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |r| {
            Ok(AgentRecord {
                id: r.get(0)?,
                session_id: r.get(1)?,
                role: r.get(2)?,
                model_name: r.get(3)?,
                worktree_path: r.get(4)?,
                ref_name: r.get(5)?,
                status: r.get(6)?,
                created_at: r.get(7)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn update_agent_status(&self, agent_id: &str, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE agents SET status = ?1 WHERE id = ?2",
            params![status, agent_id],
        )?;
        Ok(())
    }

    pub fn latest_active_node_for_agent(&self, agent_id: &str) -> Result<Option<CausalNodeRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, agent_id, parent_node_id, tree_oid, commit_oid,
                    prompt_hash, summary, tool_name, token_cost_usd, gate_status,
                    is_rolled_back, created_at
             FROM causal_nodes
             WHERE agent_id = ?1 AND is_rolled_back = 0
             ORDER BY created_at DESC LIMIT 1",
        )?;
        let row = stmt
            .query_row(params![agent_id], map_causal_node)
            .optional()?;
        Ok(row)
    }

    pub fn insert_causal_node(
        &mut self,
        node: &CausalNodeRecord,
        touches: &[FileTouchInput],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO causal_nodes (
                id, session_id, agent_id, parent_node_id, tree_oid, commit_oid,
                prompt_hash, summary, tool_name, token_cost_usd, gate_status,
                is_rolled_back, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                node.id,
                node.session_id,
                node.agent_id,
                node.parent_node_id,
                node.tree_oid,
                node.commit_oid,
                node.prompt_hash,
                node.summary,
                node.tool_name,
                node.token_cost_usd,
                node.gate_status,
                if node.is_rolled_back { 1 } else { 0 },
                node.created_at
            ],
        )?;

        for touch in touches {
            tx.execute(
                "INSERT INTO file_touches (node_id, file_path, change_type, lines_added, lines_deleted)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    node.id,
                    touch.file_path,
                    touch.change_type,
                    touch.lines_added,
                    touch.lines_deleted
                ],
            )?;
        }

        tx.execute(
            "INSERT INTO causal_fts (node_id, agent_id, summary, tool_name)
             VALUES (?1, ?2, ?3, ?4)",
            params![node.id, node.agent_id, node.summary, node.tool_name],
        )?;

        tx.execute(
            "UPDATE agents SET status = 'working' WHERE id = ?1",
            params![node.agent_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_node(&self, node_id: &str) -> Result<CausalNodeRecord> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, agent_id, parent_node_id, tree_oid, commit_oid,
                    prompt_hash, summary, tool_name, token_cost_usd, gate_status,
                    is_rolled_back, created_at
             FROM causal_nodes WHERE id = ?1",
        )?;
        stmt.query_row(params![node_id], map_causal_node)
            .optional()?
            .ok_or_else(|| AgentGitError::NodeNotFound(node_id.to_string()))
    }

    pub fn list_nodes(
        &self,
        session_id: &str,
        agent_filter: Option<&str>,
        search_query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<CausalNodeRecord>> {
        if let Some(q) = search_query.map(str::trim).filter(|s| !s.is_empty()) {
            let safe_q = format!("\"{}\"", q.replace('"', "\"\""));
            let mut stmt = self.conn.prepare(
                "SELECT n.id, n.session_id, n.agent_id, n.parent_node_id, n.tree_oid, n.commit_oid,
                        n.prompt_hash, n.summary, n.tool_name, n.token_cost_usd, n.gate_status,
                        n.is_rolled_back, n.created_at
                 FROM causal_nodes n
                 INNER JOIN causal_fts f ON n.id = f.node_id
                 WHERE n.session_id = ?1
                   AND causal_fts MATCH ?2
                   AND (?3 IS NULL OR n.agent_id = ?3)
                 ORDER BY n.created_at DESC
                 LIMIT ?4",
            )?;
            let rows = stmt.query_map(
                params![session_id, safe_q, agent_filter, limit as i64],
                map_causal_node,
            )?;
            return rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into);
        }

        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, agent_id, parent_node_id, tree_oid, commit_oid,
                    prompt_hash, summary, tool_name, token_cost_usd, gate_status,
                    is_rolled_back, created_at
             FROM causal_nodes
             WHERE session_id = ?1 AND (?2 IS NULL OR agent_id = ?2)
             ORDER BY created_at DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![session_id, agent_filter, limit as i64], map_causal_node)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Returns the target node ID and all causal descendant node IDs using a recursive CTE.
    pub fn collect_causal_subtree(&self, root_node_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE subtree(id) AS (
                SELECT id FROM causal_nodes WHERE id = ?1
                UNION ALL
                SELECT c.id FROM causal_nodes c
                INNER JOIN subtree s ON c.parent_node_id = s.id
            )
            SELECT id FROM subtree",
        )?;
        let rows = stmt.query_map(params![root_node_id], |r| r.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Marks the target node and all its causal descendants as rolled back.
    pub fn mark_nodes_rolled_back(&mut self, node_ids: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in node_ids {
            tx.execute(
                "UPDATE causal_nodes SET is_rolled_back = 1 WHERE id = ?1",
                params![id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn insert_gate_run(&mut self, gate: &GateRunRecord) -> Result<()> {
        let status = if gate.exit_code == 0 { "passed" } else { "failed" };
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO gate_runs (id, node_id, command, exit_code, duration_ms, output_excerpt, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                gate.id,
                gate.node_id,
                gate.command,
                gate.exit_code,
                gate.duration_ms,
                gate.output_excerpt,
                gate.created_at
            ],
        )?;
        tx.execute(
            "UPDATE causal_nodes SET gate_status = ?1 WHERE id = ?2",
            params![status, gate.node_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_dag_snapshot(&self, session_id: &str) -> Result<DagSnapshot> {
        let session = self
            .get_active_session()?
            .ok_or(AgentGitError::NotInitialized)?;
        let agents = self.list_agents(session_id)?;
        let nodes = self.list_nodes(session_id, None, None, 200)?;

        let mut touch_stmt = self.conn.prepare(
            "SELECT ft.id, ft.node_id, ft.file_path, ft.change_type, ft.lines_added, ft.lines_deleted
             FROM file_touches ft
             INNER JOIN causal_nodes n ON ft.node_id = n.id
             WHERE n.session_id = ?1
             ORDER BY ft.id ASC",
        )?;
        let file_touches = touch_stmt
            .query_map(params![session_id], |r| {
                Ok(FileTouchRecord {
                    id: r.get(0)?,
                    node_id: r.get(1)?,
                    file_path: r.get(2)?,
                    change_type: r.get(3)?,
                    lines_added: r.get(4)?,
                    lines_deleted: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut gate_stmt = self.conn.prepare(
            "SELECT gr.id, gr.node_id, gr.command, gr.exit_code, gr.duration_ms, gr.output_excerpt, gr.created_at
             FROM gate_runs gr
             INNER JOIN causal_nodes n ON gr.node_id = n.id
             WHERE n.session_id = ?1
             ORDER BY gr.created_at DESC",
        )?;
        let gate_runs = gate_stmt
            .query_map(params![session_id], |r| {
                Ok(GateRunRecord {
                    id: r.get(0)?,
                    node_id: r.get(1)?,
                    command: r.get(2)?,
                    exit_code: r.get(3)?,
                    duration_ms: r.get(4)?,
                    output_excerpt: r.get(5)?,
                    created_at: r.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(DagSnapshot {
            session,
            agents,
            nodes,
            file_touches,
            gate_runs,
        })
    }
}

fn map_causal_node(r: &rusqlite::Row<'_>) -> rusqlite::Result<CausalNodeRecord> {
    let rolled_back_int: i64 = r.get(11)?;
    Ok(CausalNodeRecord {
        id: r.get(0)?,
        session_id: r.get(1)?,
        agent_id: r.get(2)?,
        parent_node_id: r.get(3)?,
        tree_oid: r.get(4)?,
        commit_oid: r.get(5)?,
        prompt_hash: r.get(6)?,
        summary: r.get(7)?,
        tool_name: r.get(8)?,
        token_cost_usd: r.get(9)?,
        gate_status: r.get(10)?,
        is_rolled_back: rolled_back_int != 0,
        created_at: r.get(12)?,
    })
}
