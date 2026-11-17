use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::engine::AgentGitEngine;

/// Runs the MCP v1.x stdio JSON-RPC 2.0 server loop.
pub fn run_stdio_server(default_repo: PathBuf) -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("Parse error: {e}") }
                });
                writeln!(out, "{err_resp}")?;
                out.flush()?;
                continue;
            }
        };

        let id = req.get("id").cloned();
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

        // Notifications have no `id` and must not receive a response.
        if id.is_none() || id.as_ref() == Some(&Value::Null) && method.starts_with("notifications/")
        {
            continue;
        }

        let resp = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": { "listChanged": false }
                    },
                    "serverInfo": {
                        "name": "agentgit",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
            "ping" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {}
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "tools": tool_definitions() }
            }),
            "tools/call" => {
                let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let call_result = handle_tool_call(tool_name, &args, &default_repo);
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": call_result
                })
            }
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Method not found: {method}") }
            }),
        };

        writeln!(out, "{resp}")?;
        out.flush()?;
    }

    Ok(())
}

pub fn handle_tool_call(name: &str, args: &Value, default_repo: &Path) -> Value {
    let repo_path = args
        .get("repo_path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| default_repo.to_path_buf());

    let res: Result<Value, String> = (|| match name {
        "agentgit_spawn_shadow" => {
            let agent_id = req_str(args, "agent_id")?;
            let role = args.get("role").and_then(Value::as_str).unwrap_or("worker");
            let model = args.get("model_name").and_then(Value::as_str);
            let engine = match AgentGitEngine::open(&repo_path) {
                Ok(e) => e,
                Err(_) => {
                    AgentGitEngine::init(&repo_path)
                        .map_err(|e| e.to_string())?
                        .0
                }
            };
            let agent = engine
                .spawn(agent_id, role, model)
                .map_err(|e| e.to_string())?;
            Ok(serde_json::to_value(agent).map_err(|e| e.to_string())?)
        }
        "agentgit_checkpoint" => {
            let agent_id = req_str(args, "agent_id")?;
            let tool_name = args
                .get("tool_name")
                .and_then(Value::as_str)
                .unwrap_or("edit_file");
            let summary = req_str(args, "summary")?;
            let prompt_text = args
                .get("prompt_text")
                .and_then(Value::as_str)
                .unwrap_or("");
            let cost = args
                .get("token_cost_usd")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let mut engine = AgentGitEngine::open(&repo_path).map_err(|e| e.to_string())?;
            let node = engine
                .checkpoint(agent_id, tool_name, summary, prompt_text, cost)
                .map_err(|e| e.to_string())?;
            Ok(serde_json::to_value(node).map_err(|e| e.to_string())?)
        }
        "agentgit_causal_log" => {
            let agent_id = args.get("agent_id").and_then(Value::as_str);
            let search = args.get("search").and_then(Value::as_str);
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(30) as usize;
            let engine = AgentGitEngine::open(&repo_path).map_err(|e| e.to_string())?;
            let nodes = engine
                .log(agent_id, search, limit)
                .map_err(|e| e.to_string())?;
            Ok(serde_json::to_value(nodes).map_err(|e| e.to_string())?)
        }
        "agentgit_diff_node" => {
            let node_id = req_str(args, "node_id")?;
            let vs_node_id = args.get("vs_node_id").and_then(Value::as_str);
            let engine = AgentGitEngine::open(&repo_path).map_err(|e| e.to_string())?;
            let diff = engine
                .diff_node(node_id, vs_node_id)
                .map_err(|e| e.to_string())?;
            Ok(json!({ "node_id": node_id, "diff": diff }))
        }
        "agentgit_rollback_causal" => {
            let mut engine = AgentGitEngine::open(&repo_path).map_err(|e| e.to_string())?;
            if let Some(node_id) = args.get("node_id").and_then(Value::as_str) {
                let rb = engine.rollback_node(node_id).map_err(|e| e.to_string())?;
                Ok(serde_json::to_value(rb).map_err(|e| e.to_string())?)
            } else {
                let agent_id = req_str(args, "agent_id")?;
                let steps = args.get("steps").and_then(Value::as_u64).unwrap_or(1) as usize;
                let rb = engine
                    .rollback_agent_steps(agent_id, steps)
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::to_value(rb).map_err(|e| e.to_string())?)
            }
        }
        "agentgit_verify_and_promote" => {
            let agent_id = req_str(args, "agent_id")?;
            let verify_cmd = args.get("verify_cmd").and_then(Value::as_str);
            let promote = args
                .get("promote")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let squash = args.get("squash").and_then(Value::as_bool).unwrap_or(true);
            let commit_msg = args.get("commit_msg").and_then(Value::as_str);
            let mut engine = AgentGitEngine::open(&repo_path).map_err(|e| e.to_string())?;

            let gate = if let Some(cmd) = verify_cmd {
                let g = engine.verify(agent_id, cmd).map_err(|e| e.to_string())?;
                if g.exit_code != 0 {
                    return Err(format!(
                        "Verification gate failed (exit {}):\n{}",
                        g.exit_code, g.output_excerpt
                    ));
                }
                Some(g)
            } else {
                None
            };

            let promoted = if promote {
                Some(
                    engine
                        .promote(agent_id, squash, commit_msg)
                        .map_err(|e| e.to_string())?,
                )
            } else {
                None
            };

            Ok(json!({ "gate": gate, "promoted": promoted }))
        }
        _ => Err(format!("Unknown tool: {name}")),
    })();

    match res {
        Ok(payload) => json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&payload).unwrap_or_default()
            }]
        }),
        Err(msg) => json!({
            "isError": true,
            "content": [{
                "type": "text",
                "text": msg
            }]
        }),
    }
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Missing required argument '{key}'"))
}

pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "agentgit_spawn_shadow",
            "description": "Spawn an isolated Git shadow worktree for a subagent without touching the human branch.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Alphanumeric agent identifier (e.g. frontend-1)" },
                    "role": { "type": "string", "description": "Subagent role description" },
                    "model_name": { "type": "string", "description": "Model identifier" },
                    "repo_path": { "type": "string", "description": "Optional repository root path" }
                },
                "required": ["agent_id"]
            }
        },
        {
            "name": "agentgit_checkpoint",
            "description": "Capture an atomic Git tree snapshot and record a causal DAG node in SQLite.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string" },
                    "tool_name": { "type": "string" },
                    "summary": { "type": "string" },
                    "prompt_text": { "type": "string" },
                    "token_cost_usd": { "type": "number" },
                    "repo_path": { "type": "string" }
                },
                "required": ["agent_id", "summary"]
            }
        },
        {
            "name": "agentgit_causal_log",
            "description": "Query the causal DAG log with optional agent filter or FTS5 search.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string" },
                    "search": { "type": "string" },
                    "limit": { "type": "integer" },
                    "repo_path": { "type": "string" }
                }
            }
        },
        {
            "name": "agentgit_diff_node",
            "description": "Inspect unified diff for a causal node against its parent or another node.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "string" },
                    "vs_node_id": { "type": "string" },
                    "repo_path": { "type": "string" }
                },
                "required": ["node_id"]
            }
        },
        {
            "name": "agentgit_rollback_causal",
            "description": "Surgically roll back a causal node (and its descendants) or N steps for an agent, preserving other agents.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "string" },
                    "agent_id": { "type": "string" },
                    "steps": { "type": "integer" },
                    "repo_path": { "type": "string" }
                }
            }
        },
        {
            "name": "agentgit_verify_and_promote",
            "description": "Run verification inside an agent shadow worktree and optionally promote to human HEAD after pre-flight conflict check.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string" },
                    "verify_cmd": { "type": "string" },
                    "promote": { "type": "boolean" },
                    "squash": { "type": "boolean" },
                    "commit_msg": { "type": "string" },
                    "repo_path": { "type": "string" }
                },
                "required": ["agent_id"]
            }
        }
    ])
}
