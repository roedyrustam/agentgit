use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use agentgit::engine::AgentGitEngine;
use agentgit::{mcp, sidecar};

#[derive(Parser)]
#[command(
    name = "agentgit",
    version,
    about = "Local-first Causal Time-Travel & Shadow Worktree Engine for Multi-Agent AI"
)]
struct Cli {
    /// Repository directory (defaults to current working directory)
    #[arg(long, global = true)]
    repo: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize .agentgit/ in the current Git repository
    Init,

    /// Spawn an isolated shadow worktree for a subagent
    Spawn {
        #[arg(long)]
        agent: String,
        #[arg(long, default_value = "worker")]
        role: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        json: bool,
    },

    /// Capture an atomic Git snapshot and record a causal DAG node
    Checkpoint {
        #[arg(long)]
        agent: String,
        #[arg(long, default_value = "edit_file")]
        tool: String,
        #[arg(long)]
        summary: String,
        #[arg(long, default_value = "")]
        prompt: String,
        #[arg(long, default_value_t = 0.0)]
        cost: f64,
        #[arg(long)]
        json: bool,
    },

    /// Inspect the causal DAG log
    Log {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        search: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },

    /// Show unified diff for a causal node
    Diff {
        node_id: String,
        #[arg(long)]
        vs: Option<String>,
    },

    /// Surgically roll back a causal node (and descendants) or N steps for an agent
    Rollback {
        #[arg(long)]
        node: Option<String>,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long, default_value_t = 1)]
        steps: usize,
        #[arg(long)]
        json: bool,
    },

    /// Run a verification command inside an agent's shadow worktree
    Verify {
        #[arg(long)]
        agent: String,
        #[arg(long)]
        cmd: String,
        #[arg(long)]
        json: bool,
    },

    /// Promote verified shadow worktree changes to the human branch
    Promote {
        #[arg(long)]
        agent: String,
        #[arg(long, default_value_t = true)]
        squash: bool,
        #[arg(long)]
        message: Option<String>,
        #[arg(long)]
        json: bool,
    },

    /// Garbage-collect promoted or rolled-back shadow worktrees
    Gc,

    /// Start the MCP v1.x stdio JSON-RPC 2.0 server
    Mcp,

    /// Start the Local Sidecar Cockpit HTTP UI
    Ui {
        #[arg(long, default_value_t = 4317)]
        port: u16,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cwd = cli
        .repo
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    match cli.command {
        Commands::Init => {
            let (engine, session) = AgentGitEngine::init(&cwd)?;
            println!(
                "Initialized AgentGit in {} (session {}, base {} @ {})",
                engine.repo_root.display(),
                session.id,
                session.base_branch,
                &session.base_commit_oid[..7.min(session.base_commit_oid.len())]
            );
        }
        Commands::Spawn {
            agent,
            role,
            model,
            json,
        } => {
            let engine = AgentGitEngine::open(&cwd)?;
            let rec = engine.spawn(&agent, &role, model.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rec)?);
            } else {
                println!("Spawned shadow agent '{}' at {}", rec.id, rec.worktree_path);
            }
        }
        Commands::Checkpoint {
            agent,
            tool,
            summary,
            prompt,
            cost,
            json,
        } => {
            let mut engine = AgentGitEngine::open(&cwd)?;
            let node = engine.checkpoint(&agent, &tool, &summary, &prompt, cost)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&node)?);
            } else {
                println!(
                    "Checkpoint {} [{}] {} ({})",
                    &node.id[..8],
                    node.agent_id,
                    node.summary,
                    &node.commit_oid[..7.min(node.commit_oid.len())]
                );
            }
        }
        Commands::Log {
            agent,
            search,
            limit,
            json,
        } => {
            let engine = AgentGitEngine::open(&cwd)?;
            let nodes = engine.log(agent.as_deref(), search.as_deref(), limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&nodes)?);
            } else if nodes.is_empty() {
                println!("No causal nodes recorded.");
            } else {
                for n in nodes {
                    let rb = if n.is_rolled_back { " [ROLLED_BACK]" } else { "" };
                    println!(
                        "{} | {:<12} | {:<7} | {} | {}{}",
                        &n.id[..8],
                        n.agent_id,
                        n.gate_status,
                        &n.commit_oid[..7.min(n.commit_oid.len())],
                        n.summary,
                        rb
                    );
                }
            }
        }
        Commands::Diff { node_id, vs } => {
            let engine = AgentGitEngine::open(&cwd)?;
            let diff = engine.diff_node(&node_id, vs.as_deref())?;
            println!("{diff}");
        }
        Commands::Rollback {
            node,
            agent,
            steps,
            json,
        } => {
            let mut engine = AgentGitEngine::open(&cwd)?;
            let rb = if let Some(node_id) = node {
                engine.rollback_node(&node_id)?
            } else if let Some(agent_id) = agent {
                engine.rollback_agent_steps(&agent_id, steps)?
            } else {
                anyhow::bail!("Provide either --node <ID> or --agent <ID>");
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&rb)?);
            } else {
                println!(
                    "Rolled back {} node(s) for '{}' -> restored {}",
                    rb.rolled_back_node_ids.len(),
                    rb.agent_id,
                    &rb.restored_commit_oid[..7.min(rb.restored_commit_oid.len())]
                );
            }
        }
        Commands::Verify { agent, cmd, json } => {
            let mut engine = AgentGitEngine::open(&cwd)?;
            let gate = engine.verify(&agent, &cmd)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&gate)?);
            } else {
                println!(
                    "Gate '{}' for '{}' exited {} in {}ms",
                    gate.command, agent, gate.exit_code, gate.duration_ms
                );
                if !gate.output_excerpt.is_empty() {
                    println!("{}", gate.output_excerpt);
                }
            }
            if gate.exit_code != 0 {
                std::process::exit(gate.exit_code as i32);
            }
        }
        Commands::Promote {
            agent,
            squash,
            message,
            json,
        } => {
            let engine = AgentGitEngine::open(&cwd)?;
            let pr = engine.promote(&agent, squash, message.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&pr)?);
            } else {
                println!(
                    "Promoted '{}' to human HEAD {}",
                    pr.agent_id,
                    &pr.human_head_oid[..7.min(pr.human_head_oid.len())]
                );
            }
        }
        Commands::Gc => {
            let engine = AgentGitEngine::open(&cwd)?;
            let removed = engine.gc()?;
            println!("Garbage-collected {removed} shadow worktree(s).");
        }
        Commands::Mcp => {
            mcp::run_stdio_server(cwd)?;
        }
        Commands::Ui { port } => {
            sidecar::run_sidecar_server(cwd, port)?;
        }
    }

    Ok(())
}
