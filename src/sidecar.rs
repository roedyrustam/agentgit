use std::path::PathBuf;

use serde_json::{json, Value};
use tiny_http::{Header, Method, Response, Server, StatusCode};

use crate::engine::AgentGitEngine;
use crate::sidecar_html;

/// Starts the local-only Sidecar HTTP server on `127.0.0.1:<port>`.
pub fn run_sidecar_server(repo_path: PathBuf, port: u16) -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{port}");
    let server = Server::http(&addr)
        .map_err(|e| anyhow::anyhow!("Failed to bind Sidecar UI at {addr}: {e}"))?;
    let session_token = uuid::Uuid::new_v4().to_string();

    println!("AgentGit Sidecar Cockpit running at http://{addr}/");
    println!("Legal & Privacy Trio: /privacy | /terms | /delete-account");

    for mut req in server.incoming_requests() {
        let method = req.method().clone();
        let raw_url = req.url().to_string();
        let (path, query) = match raw_url.split_once('?') {
            Some((p, q)) => (p, q),
            None => (raw_url.as_str(), ""),
        };

        let response = match (method, path) {
            (Method::Get, "/") => html_resp(200, sidecar_html::render_cockpit_html(&session_token)),
            (Method::Get, "/privacy") => html_resp(200, sidecar_html::render_privacy_html()),
            (Method::Get, "/terms") => html_resp(200, sidecar_html::render_terms_html()),
            (Method::Get, "/delete-account") => html_resp(
                200,
                sidecar_html::render_delete_account_html(&session_token),
            ),
            (Method::Get, "/api/v1/dag") => {
                match AgentGitEngine::open(&repo_path).and_then(|e| e.dag_snapshot()) {
                    Ok(dag) => json_resp(200, serde_json::to_value(dag).unwrap_or_default()),
                    Err(e) => json_resp(500, json!({ "error": e.to_string() })),
                }
            }
            (Method::Get, "/api/v1/diff") => {
                let node_id = extract_query_param(query, "node_id").unwrap_or_default();
                match AgentGitEngine::open(&repo_path).and_then(|e| e.diff_node(&node_id, None)) {
                    Ok(diff) => json_resp(200, json!({ "node_id": node_id, "diff": diff })),
                    Err(e) => json_resp(400, json!({ "error": e.to_string() })),
                }
            }
            (Method::Post, "/api/v1/rollback") => {
                if !verify_token(&req, &session_token) {
                    json_resp(
                        403,
                        json!({ "error": "Invalid or missing X-AgentGit-Token" }),
                    )
                } else {
                    let body = read_json_body(&mut req);
                    let node_id = body
                        .get("node_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    match AgentGitEngine::open(&repo_path)
                        .and_then(|mut e| e.rollback_node(node_id))
                    {
                        Ok(rb) => json_resp(200, serde_json::to_value(rb).unwrap_or_default()),
                        Err(e) => json_resp(400, json!({ "error": e.to_string() })),
                    }
                }
            }
            (Method::Post, "/api/v1/promote") => {
                if !verify_token(&req, &session_token) {
                    json_resp(
                        403,
                        json!({ "error": "Invalid or missing X-AgentGit-Token" }),
                    )
                } else {
                    let body = read_json_body(&mut req);
                    let agent_id = body
                        .get("agent_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let squash = body.get("squash").and_then(Value::as_bool).unwrap_or(true);
                    match AgentGitEngine::open(&repo_path)
                        .and_then(|e| e.promote(agent_id, squash, None))
                    {
                        Ok(pr) => json_resp(200, serde_json::to_value(pr).unwrap_or_default()),
                        Err(e) => json_resp(400, json!({ "error": e.to_string() })),
                    }
                }
            }
            (Method::Post, "/api/v1/purge") => {
                if !verify_token(&req, &session_token) {
                    json_resp(
                        403,
                        json!({ "error": "Invalid or missing X-AgentGit-Token" }),
                    )
                } else {
                    match AgentGitEngine::open(&repo_path).and_then(|e| e.purge_all()) {
                        Ok(_) => json_resp(
                            200,
                            json!({ "message": "All .agentgit/ local state and shadow worktrees purged." }),
                        ),
                        Err(e) => json_resp(500, json!({ "error": e.to_string() })),
                    }
                }
            }
            _ => json_resp(404, json!({ "error": "Not found" })),
        };

        let _ = req.respond(response);
    }

    Ok(())
}

fn verify_token(req: &tiny_http::Request, expected: &str) -> bool {
    req.headers()
        .iter()
        .any(|h| h.field.equiv("X-AgentGit-Token") && h.value.as_str() == expected)
}

fn read_json_body(req: &mut tiny_http::Request) -> Value {
    let mut buf = String::new();
    let _ = req.as_reader().read_to_string(&mut buf);
    serde_json::from_str(&buf).unwrap_or_else(|_| json!({}))
}

fn extract_query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            Some(v.to_string())
        } else {
            None
        }
    })
}

fn html_resp(status: u16, body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    let hdr = Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap();
    Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(hdr)
}

fn json_resp(status: u16, val: Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let hdr = Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap();
    Response::from_string(val.to_string())
        .with_status_code(StatusCode(status))
        .with_header(hdr)
}
