use std::path::Path;
use std::process::Command;

use serde_json::json;
use tempfile::TempDir;

use agentgit::engine::AgentGitEngine;
use agentgit::error::AgentGitError;
use agentgit::git_ops;
use agentgit::mcp;

fn setup_git_repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let p = dir.path();

    run_git_ok(p, &["init"]);
    run_git_ok(p, &["config", "user.email", "test@agentgit.dev"]);
    run_git_ok(p, &["config", "user.name", "AgentGit Test"]);
    run_git_ok(p, &["config", "core.autocrlf", "false"]);

    std::fs::write(p.join("README.md"), "# Test Repo\n").unwrap();
    std::fs::write(p.join("shared.txt"), "line1\n").unwrap();
    run_git_ok(p, &["add", "."]);
    run_git_ok(p, &["commit", "-m", "initial commit"]);
    dir
}

fn run_git_ok(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git command");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_parallel_agents_causal_rollback_and_promote() {
    let repo = setup_git_repo();
    let root = repo.path();

    let (mut engine, _session) = AgentGitEngine::init(root).expect("init");

    let agent_a = engine
        .spawn("frontend-1", "UI", Some("gemini-4"))
        .expect("spawn A");
    let agent_b = engine
        .spawn("backend-1", "API", Some("gemini-4"))
        .expect("spawn B");

    let wt_a = engine.repo_root.join(&agent_a.worktree_path);
    let wt_b = engine.repo_root.join(&agent_b.worktree_path);

    // Agent A: step 1 & step 2
    std::fs::write(wt_a.join("ui.txt"), "v1\n").unwrap();
    let n_a1 = engine
        .checkpoint("frontend-1", "write_file", "Create ui v1", "build ui", 0.01)
        .expect("cp A1");

    std::fs::write(wt_a.join("ui.txt"), "v2-broken\n").unwrap();
    let n_a2 = engine
        .checkpoint("frontend-1", "edit_file", "Break ui v2", "tweak ui", 0.01)
        .expect("cp A2");

    // Agent B: step 1
    std::fs::write(wt_b.join("api.txt"), "endpoint ok\n").unwrap();
    let _n_b1 = engine
        .checkpoint("backend-1", "write_file", "Create api", "build api", 0.02)
        .expect("cp B1");

    // Verify Agent B with a real command
    let gate = engine
        .verify("backend-1", "git status --short")
        .expect("verify B");
    assert_eq!(gate.exit_code, 0);

    // Rollback Agent A step 2 -> should restore v1, leaving Agent B untouched
    let rb = engine.rollback_node(&n_a2.id).expect("rollback A2");
    assert_eq!(rb.rolled_back_node_ids, vec![n_a2.id.clone()]);
    assert_eq!(rb.restored_commit_oid, n_a1.commit_oid);
    assert_eq!(
        std::fs::read_to_string(wt_a.join("ui.txt")).unwrap(),
        "v1\n"
    );
    assert_eq!(
        std::fs::read_to_string(wt_b.join("api.txt")).unwrap(),
        "endpoint ok\n"
    );

    // Human HEAD should still be clean (no ui.txt or api.txt yet)
    assert!(!root.join("api.txt").exists());

    // Promote Agent B to human HEAD
    let promoted = engine
        .promote("backend-1", true, Some("feat: add api"))
        .expect("promote B");
    assert_eq!(promoted.agent_id, "backend-1");
    assert_eq!(
        std::fs::read_to_string(root.join("api.txt")).unwrap(),
        "endpoint ok\n"
    );

    // FTS5 search
    let found = engine.log(None, Some("Create"), 10).expect("fts log");
    assert!(found.len() >= 2);

    // GC removes promoted worktree
    let removed = engine.gc().expect("gc");
    assert_eq!(removed, 1);
    assert!(!wt_b.exists());
}

#[test]
fn test_promote_conflict_preflight_protects_human_head() {
    let repo = setup_git_repo();
    let root = repo.path();
    let (mut engine, _) = AgentGitEngine::init(root).expect("init");

    let a = engine.spawn("agent-a", "worker", None).unwrap();
    let b = engine.spawn("agent-b", "worker", None).unwrap();

    std::fs::write(
        engine.repo_root.join(&a.worktree_path).join("shared.txt"),
        "from-a\n",
    )
    .unwrap();
    engine
        .checkpoint("agent-a", "edit_file", "Edit shared A", "p", 0.0)
        .unwrap();

    std::fs::write(
        engine.repo_root.join(&b.worktree_path).join("shared.txt"),
        "from-b\n",
    )
    .unwrap();
    engine
        .checkpoint("agent-b", "edit_file", "Edit shared B", "p", 0.0)
        .unwrap();

    // Agent A promotes cleanly
    engine.promote("agent-a", true, Some("feat: A")).unwrap();

    // Agent B conflicts during pre-flight and does NOT corrupt human HEAD
    let err = engine
        .promote("agent-b", true, Some("feat: B"))
        .unwrap_err();
    assert!(matches!(err, AgentGitError::MergeConflict(_)));
    assert_eq!(
        std::fs::read_to_string(root.join("shared.txt")).unwrap(),
        "from-a\n"
    );
}

#[test]
fn test_mcp_tool_dispatch_and_identifier_guard() {
    assert!(git_ops::validate_identifier("../evil").is_err());
    assert!(git_ops::validate_identifier("agent-1_ok").is_ok());

    let repo = setup_git_repo();
    let root = repo.path().to_path_buf();

    let spawn_res = mcp::handle_tool_call(
        "agentgit_spawn_shadow",
        &json!({ "agent_id": "mcp-1", "role": "tester" }),
        &root,
    );
    assert!(spawn_res.get("isError").is_none());

    let log_res = mcp::handle_tool_call("agentgit_causal_log", &json!({}), &root);
    assert!(log_res.get("isError").is_none());
}

#[test]
fn test_eight_parallel_agents_stress_and_purge() {
    let repo = setup_git_repo();
    let root = repo.path().to_path_buf();

    let (engine, _) = AgentGitEngine::init(&root).expect("init");
    let mut handles = Vec::new();

    for i in 0..8 {
        let repo_root = root.clone();
        handles.push(std::thread::spawn(move || {
            let mut eng = AgentGitEngine::open(&repo_root).expect("open");
            let agent_id = format!("swarm-{i}");
            let agent = eng
                .spawn(&agent_id, "worker", Some("gemini-4"))
                .expect("spawn");
            let wt = eng.repo_root.join(&agent.worktree_path);

            for step in 1..=2 {
                std::fs::write(wt.join(format!("file_{i}.txt")), format!("v{step}\n")).unwrap();
                eng.checkpoint(
                    &agent_id,
                    "write_file",
                    &format!("Agent {i} step {step}"),
                    "parallel task",
                    0.005,
                )
                .expect("checkpoint");
            }
        }));
    }

    for h in handles {
        h.join().expect("thread join");
    }

    // Verify all 16 causal nodes exist and human HEAD is untouched
    let nodes = engine.log(None, None, 50).expect("log");
    assert_eq!(nodes.len(), 16);
    assert!(!root.join("file_0.txt").exists());
    assert!(!root.join("file_7.txt").exists());

    // Diff a node and test multi-step rollback on swarm-0
    let diff = engine.diff_node(&nodes[0].id, None).expect("diff_node");
    assert!(!diff.is_empty());

    let mut eng_mut = AgentGitEngine::open(&root).expect("open mut");
    let rb = eng_mut
        .rollback_agent_steps("swarm-0", 1)
        .expect("rollback_agent_steps");
    assert_eq!(rb.rolled_back_node_ids.len(), 1);
    assert_eq!(
        std::fs::read_to_string(
            eng_mut
                .repo_root
                .join(".agentgit/worktrees/swarm-0/file_0.txt")
        )
        .unwrap(),
        "v1\n"
    );

    // Purge all .agentgit state (Legal & Privacy /delete-account backend)
    drop(engine);
    eng_mut.purge_all().expect("purge_all");
    assert!(!root.join(".agentgit").exists());
}
