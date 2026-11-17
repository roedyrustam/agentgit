use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::db::{
    now_ms, AgentRecord, CausalNodeRecord, DagSnapshot, GateRunRecord, SessionRecord, StateDb,
};
use crate::error::{AgentGitError, Result};
use crate::git_ops;

#[derive(Debug, Serialize)]
pub struct RollbackResult {
    pub rolled_back_node_ids: Vec<String>,
    pub restored_commit_oid: String,
    pub agent_id: String,
}

#[derive(Debug, Serialize)]
pub struct PromoteResult {
    pub agent_id: String,
    pub promoted_commit_oid: String,
    pub human_head_oid: String,
    pub squashed: bool,
}

/// Core orchestrator combining Git shadow worktrees and SQLite causal DAG state.
pub struct AgentGitEngine {
    pub repo_root: PathBuf,
    pub state_dir: PathBuf,
    pub db: StateDb,
}

impl AgentGitEngine {
    /// Initializes `.agentgit/` and creates an active session in the current Git repository.
    pub fn init(start_dir: &Path) -> Result<(Self, SessionRecord)> {
        let repo_root = git_ops::resolve_repo_root(start_dir)?;
        let state_dir = repo_root.join(".agentgit");
        std::fs::create_dir_all(state_dir.join("worktrees"))?;
        let db = StateDb::open(&state_dir.join("state.db"))?;
        let (branch, head_oid) = git_ops::head_branch_and_oid(&repo_root)?;
        let session = db.ensure_active_session(&repo_root.to_string_lossy(), &branch, &head_oid)?;
        Ok((
            Self {
                repo_root,
                state_dir,
                db,
            },
            session,
        ))
    }

    /// Opens an already-initialized AgentGit workspace.
    pub fn open(start_dir: &Path) -> Result<Self> {
        let repo_root = git_ops::resolve_repo_root(start_dir)?;
        let state_dir = repo_root.join(".agentgit");
        let db_path = state_dir.join("state.db");
        if !db_path.exists() {
            return Err(AgentGitError::NotInitialized);
        }
        let db = StateDb::open(&db_path)?;
        Ok(Self {
            repo_root,
            state_dir,
            db,
        })
    }

    fn active_session(&self) -> Result<SessionRecord> {
        self.db
            .get_active_session()?
            .ok_or(AgentGitError::NotInitialized)
    }

    pub fn spawn(
        &self,
        agent_id: &str,
        role: &str,
        model_name: Option<&str>,
    ) -> Result<AgentRecord> {
        git_ops::validate_identifier(agent_id)?;
        let session = self.active_session()?;
        let rel_worktree = format!(".agentgit/worktrees/{agent_id}");
        let abs_worktree = self.repo_root.join(&rel_worktree);
        let ref_name = format!("refs/agentgit/agents/{agent_id}");

        let agent = AgentRecord {
            id: agent_id.to_string(),
            session_id: session.id.clone(),
            role: role.to_string(),
            model_name: model_name.map(str::to_string),
            worktree_path: rel_worktree,
            ref_name: ref_name.clone(),
            status: "idle".to_string(),
            created_at: now_ms(),
        };

        self.db.insert_agent(&agent)?;
        if let Err(e) = git_ops::spawn_shadow_worktree(
            &self.repo_root,
            &abs_worktree,
            &ref_name,
            &session.base_commit_oid,
        ) {
            let _ = self.db.update_agent_status(agent_id, "rolled_back");
            return Err(e);
        }

        Ok(agent)
    }

    pub fn checkpoint(
        &mut self,
        agent_id: &str,
        tool_name: &str,
        summary: &str,
        prompt_text: &str,
        token_cost_usd: f64,
    ) -> Result<CausalNodeRecord> {
        git_ops::validate_identifier(agent_id)?;
        let session = self.active_session()?;
        let agent = self.db.get_agent(agent_id)?;
        let abs_worktree = self.repo_root.join(&agent.worktree_path);

        let parent_node = self.db.latest_active_node_for_agent(agent_id)?;
        let parent_oid = parent_node
            .as_ref()
            .map(|n| n.commit_oid.clone())
            .unwrap_or_else(|| session.base_commit_oid.clone());

        let snapshot =
            git_ops::capture_shadow_snapshot(&abs_worktree, &agent.ref_name, &parent_oid, summary)?;

        let mut hasher = Sha256::new();
        hasher.update(prompt_text.as_bytes());
        let prompt_hash = format!("{:x}", hasher.finalize());

        let node = CausalNodeRecord {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session.id,
            agent_id: agent_id.to_string(),
            parent_node_id: parent_node.map(|n| n.id),
            tree_oid: snapshot.tree_oid,
            commit_oid: snapshot.commit_oid,
            prompt_hash,
            summary: summary.to_string(),
            tool_name: tool_name.to_string(),
            token_cost_usd,
            gate_status: "pending".to_string(),
            is_rolled_back: false,
            created_at: now_ms(),
        };

        self.db.insert_causal_node(&node, &snapshot.touches)?;
        Ok(node)
    }

    pub fn log(
        &self,
        agent_filter: Option<&str>,
        search_query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<CausalNodeRecord>> {
        let session = self.active_session()?;
        self.db
            .list_nodes(&session.id, agent_filter, search_query, limit)
    }

    pub fn diff_node(&self, node_id: &str, vs_node_id: Option<&str>) -> Result<String> {
        let node = self.db.get_node(node_id)?;
        let from_oid = if let Some(other_id) = vs_node_id {
            self.db.get_node(other_id)?.commit_oid
        } else if let Some(parent_id) = &node.parent_node_id {
            self.db.get_node(parent_id)?.commit_oid
        } else {
            self.active_session()?.base_commit_oid
        };
        git_ops::diff_commits(&self.repo_root, &from_oid, &node.commit_oid)
    }

    pub fn rollback_node(&mut self, node_id: &str) -> Result<RollbackResult> {
        let session = self.active_session()?;
        let target = self.db.get_node(node_id)?;
        let agent = self.db.get_agent(&target.agent_id)?;
        let subtree = self.db.collect_causal_subtree(node_id)?;

        let restore_oid = if let Some(parent_id) = &target.parent_node_id {
            self.db.get_node(parent_id)?.commit_oid
        } else {
            session.base_commit_oid.clone()
        };

        // Compensating transaction: restore Git ref & worktree FIRST, then commit SQLite state.
        let abs_worktree = self.repo_root.join(&agent.worktree_path);
        git_ops::restore_worktree_to_commit(&abs_worktree, &agent.ref_name, &restore_oid)?;
        self.db.mark_nodes_rolled_back(&subtree)?;

        if target.parent_node_id.is_none() {
            self.db.update_agent_status(&agent.id, "rolled_back")?;
        }

        Ok(RollbackResult {
            rolled_back_node_ids: subtree,
            restored_commit_oid: restore_oid,
            agent_id: agent.id,
        })
    }

    pub fn rollback_agent_steps(&mut self, agent_id: &str, steps: usize) -> Result<RollbackResult> {
        git_ops::validate_identifier(agent_id)?;
        let session = self.active_session()?;
        let nodes: Vec<CausalNodeRecord> = self
            .db
            .list_nodes(&session.id, Some(agent_id), None, 200)?
            .into_iter()
            .filter(|n| !n.is_rolled_back)
            .collect();

        if nodes.is_empty() || steps == 0 {
            return Err(AgentGitError::NodeNotFound(format!(
                "No active checkpoints to roll back for agent '{agent_id}'"
            )));
        }
        let idx = (steps - 1).min(nodes.len() - 1);
        let target_id = nodes[idx].id.clone();
        self.rollback_node(&target_id)
    }

    pub fn verify(&mut self, agent_id: &str, command_str: &str) -> Result<GateRunRecord> {
        git_ops::validate_identifier(agent_id)?;
        let agent = self.db.get_agent(agent_id)?;
        let latest = self
            .db
            .latest_active_node_for_agent(agent_id)?
            .ok_or_else(|| {
                AgentGitError::NodeNotFound(format!("No active checkpoint for '{agent_id}'"))
            })?;

        let abs_worktree = self.repo_root.join(&agent.worktree_path);
        let start = Instant::now();

        let output = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .args(["/C", command_str])
                .current_dir(&abs_worktree)
                .output()?
        } else {
            Command::new("sh")
                .args(["-c", command_str])
                .current_dir(&abs_worktree)
                .output()?
        };

        let duration_ms = start.elapsed().as_millis() as i64;
        let exit_code = output.status.code().unwrap_or(-1) as i64;
        let mut combined = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&stderr);
        }
        let excerpt: String = combined.chars().take(2000).collect();

        let gate = GateRunRecord {
            id: uuid::Uuid::new_v4().to_string(),
            node_id: latest.id,
            command: command_str.to_string(),
            exit_code,
            duration_ms,
            output_excerpt: excerpt,
            created_at: now_ms(),
        };

        self.db.insert_gate_run(&gate)?;
        Ok(gate)
    }

    pub fn promote(
        &self,
        agent_id: &str,
        squash: bool,
        commit_msg: Option<&str>,
    ) -> Result<PromoteResult> {
        git_ops::validate_identifier(agent_id)?;
        let session = self.active_session()?;
        let _agent = self.db.get_agent(agent_id)?;
        let latest = self
            .db
            .latest_active_node_for_agent(agent_id)?
            .ok_or_else(|| {
                AgentGitError::NodeNotFound(format!("No active checkpoint for '{agent_id}'"))
            })?;

        let msg = commit_msg
            .map(str::to_string)
            .unwrap_or_else(|| format!("agentgit({agent_id}): {}", latest.summary));

        let human_head_oid = git_ops::promote_to_human_head(
            &self.repo_root,
            &latest.commit_oid,
            &session.base_commit_oid,
            squash,
            &msg,
        )?;

        self.db.update_agent_status(agent_id, "promoted")?;

        Ok(PromoteResult {
            agent_id: agent_id.to_string(),
            promoted_commit_oid: latest.commit_oid,
            human_head_oid,
            squashed: squash,
        })
    }

    pub fn gc(&self) -> Result<usize> {
        let session = self.active_session()?;
        let agents = self.db.list_agents(&session.id)?;
        let mut removed = 0usize;
        for agent in agents {
            if agent.status == "promoted" || agent.status == "rolled_back" {
                let abs_worktree = self.repo_root.join(&agent.worktree_path);
                if abs_worktree.exists() {
                    git_ops::remove_worktree_with_retry(
                        &self.repo_root,
                        &abs_worktree,
                        &agent.ref_name,
                    )?;
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    pub fn dag_snapshot(&self) -> Result<DagSnapshot> {
        let session = self.active_session()?;
        self.db.get_dag_snapshot(&session.id)
    }

    /// Permanently removes all shadow worktrees, ephemeral refs, and `.agentgit/` state.
    pub fn purge_all(self) -> Result<()> {
        if let Ok(session) = self.active_session() {
            if let Ok(agents) = self.db.list_agents(&session.id) {
                for agent in agents {
                    let abs_worktree = self.repo_root.join(&agent.worktree_path);
                    let _ = git_ops::remove_worktree_with_retry(
                        &self.repo_root,
                        &abs_worktree,
                        &agent.ref_name,
                    );
                }
            }
        }
        let state_dir = self.state_dir.clone();
        drop(self.db);
        if state_dir.exists() {
            std::fs::remove_dir_all(&state_dir)?;
        }
        Ok(())
    }
}
