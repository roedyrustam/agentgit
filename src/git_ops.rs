use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use crate::db::FileTouchInput;
use crate::error::{AgentGitError, Result};

/// Validates an agent or node identifier to prevent path traversal and Git ref injection.
pub fn validate_identifier(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AgentGitError::InvalidIdentifier(id.to_string()));
    }
    Ok(())
}

fn run_git(cwd: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd).args(args);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let output = cmd.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let msg = if stderr.is_empty() { stdout } else { stderr };
        return Err(AgentGitError::GitCommandFailed {
            cmd: format!("git {}", args.join(" ")),
            stderr: msg,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Resolves the canonical Git repository root from any directory inside the repo.
pub fn resolve_repo_root(start: &Path) -> Result<PathBuf> {
    let out = run_git(start, &["rev-parse", "--show-toplevel"], &[])
        .map_err(|_| AgentGitError::NotAGitRepo(start.display().to_string()))?;
    let canonical = std::fs::canonicalize(PathBuf::from(out))?;
    // Strip Windows verbatim prefix (\\?\) because `git worktree add` rejects //?/ paths.
    #[cfg(windows)]
    {
        let s = canonical.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            return Ok(PathBuf::from(stripped));
        }
    }
    Ok(canonical)
}

/// Returns `(current_branch, head_commit_oid)`.
pub fn head_branch_and_oid(repo_root: &Path) -> Result<(String, String)> {
    let branch = run_git(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"], &[])?;
    let oid = run_git(repo_root, &["rev-parse", "HEAD"], &[])?;
    Ok((branch, oid))
}

/// Creates an isolated detached shadow worktree and initializes its ephemeral ref.
pub fn spawn_shadow_worktree(
    repo_root: &Path,
    worktree_path: &Path,
    ref_name: &str,
    base_oid: &str,
) -> Result<()> {
    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    run_git(repo_root, &["update-ref", ref_name, base_oid], &[])?;
    let wt_str = worktree_path.to_string_lossy();
    run_git(
        repo_root,
        &["worktree", "add", "--detach", &wt_str, base_oid],
        &[],
    )?;
    Ok(())
}

pub struct SnapshotResult {
    pub tree_oid: String,
    pub commit_oid: String,
    pub touches: Vec<FileTouchInput>,
}

/// Captures an atomic Git tree + commit snapshot from a shadow worktree using a temporary index.
pub fn capture_shadow_snapshot(
    worktree_path: &Path,
    ref_name: &str,
    parent_oid: &str,
    message: &str,
) -> Result<SnapshotResult> {
    let tmp_index = std::env::temp_dir().join(format!("agentgit-idx-{}.idx", uuid::Uuid::new_v4()));
    let tmp_idx_str = tmp_index.to_string_lossy().to_string();
    let env = [("GIT_INDEX_FILE", tmp_idx_str.as_str())];

    // Stage all files in the shadow worktree into the isolated temp index, excluding .agentgit
    let stage_res = run_git(
        worktree_path,
        &["add", "-A", "--", ".", ":(exclude).agentgit"],
        &env,
    );
    if let Err(e) = stage_res {
        let _ = std::fs::remove_file(&tmp_index);
        return Err(e);
    }

    let tree_oid = match run_git(worktree_path, &["write-tree"], &env) {
        Ok(oid) => oid,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_index);
            return Err(e);
        }
    };
    let _ = std::fs::remove_file(&tmp_index);

    let commit_oid = run_git(
        worktree_path,
        &["commit-tree", &tree_oid, "-p", parent_oid, "-m", message],
        &[],
    )?;

    run_git(worktree_path, &["update-ref", ref_name, &commit_oid], &[])?;

    let touches = collect_file_touches(worktree_path, parent_oid, &commit_oid)?;

    Ok(SnapshotResult {
        tree_oid,
        commit_oid,
        touches,
    })
}

fn collect_file_touches(
    cwd: &Path,
    parent_oid: &str,
    commit_oid: &str,
) -> Result<Vec<FileTouchInput>> {
    let numstat = run_git(
        cwd,
        &[
            "diff-tree",
            "--no-commit-id",
            "--numstat",
            "-r",
            parent_oid,
            commit_oid,
        ],
        &[],
    )?;
    let status_out = run_git(
        cwd,
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-status",
            "-r",
            parent_oid,
            commit_oid,
        ],
        &[],
    )?;

    let mut status_map = std::collections::HashMap::new();
    for line in status_out.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 2 {
            let change_type = match parts[0].chars().next().unwrap_or('M') {
                'A' => "added",
                'D' => "deleted",
                'R' => "renamed",
                _ => "modified",
            };
            let path = parts.last().copied().unwrap_or_default().to_string();
            status_map.insert(path, change_type.to_string());
        }
    }

    let mut touches = Vec::new();
    for line in numstat.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            let added = parts[0].parse::<i64>().unwrap_or(0);
            let deleted = parts[1].parse::<i64>().unwrap_or(0);
            let file_path = parts[2].to_string();
            let change_type = status_map
                .get(&file_path)
                .cloned()
                .unwrap_or_else(|| "modified".to_string());
            touches.push(FileTouchInput {
                file_path,
                change_type,
                lines_added: added,
                lines_deleted: deleted,
            });
        }
    }

    Ok(touches)
}

/// Restores a shadow worktree and its ephemeral ref back to `target_oid`.
pub fn restore_worktree_to_commit(
    worktree_path: &Path,
    ref_name: &str,
    target_oid: &str,
) -> Result<()> {
    run_git(worktree_path, &["update-ref", ref_name, target_oid], &[])?;
    run_git(worktree_path, &["reset", "--hard", target_oid], &[])?;
    run_git(worktree_path, &["clean", "-fd"], &[])?;
    Ok(())
}

/// Returns unified diff between two Git commits.
pub fn diff_commits(repo_root: &Path, from_oid: &str, to_oid: &str) -> Result<String> {
    run_git(repo_root, &["diff", from_oid, to_oid], &[])
}

/// Runs `git merge-tree --write-tree` pre-flight check and promotes changes to human `HEAD`.
pub fn promote_to_human_head(
    repo_root: &Path,
    source_oid: &str,
    base_oid: &str,
    squash: bool,
    commit_msg: &str,
) -> Result<String> {
    // Pre-flight conflict detection without touching human HEAD
    let merge_base_arg = format!("--merge-base={base_oid}");
    let preflight = Command::new("git")
        .current_dir(repo_root)
        .args([
            "merge-tree",
            "--write-tree",
            &merge_base_arg,
            "HEAD",
            source_oid,
        ])
        .output()?;

    if !preflight.status.success() {
        let stdout = String::from_utf8_lossy(&preflight.stdout).to_string();
        let stderr = String::from_utf8_lossy(&preflight.stderr).to_string();
        return Err(AgentGitError::MergeConflict(format!("{stdout}\n{stderr}")));
    }

    if squash {
        run_git(repo_root, &["merge", "--squash", source_oid], &[])?;
        run_git(repo_root, &["commit", "-m", commit_msg], &[])?;
    } else {
        run_git(
            repo_root,
            &["merge", "--no-ff", "-m", commit_msg, source_oid],
            &[],
        )?;
    }

    run_git(repo_root, &["rev-parse", "HEAD"], &[])
}

/// Removes a shadow worktree with Windows-safe retry-with-backoff and deletes its ephemeral ref.
pub fn remove_worktree_with_retry(
    repo_root: &Path,
    worktree_path: &Path,
    ref_name: &str,
) -> Result<()> {
    if worktree_path.exists() {
        let wt_str = worktree_path.to_string_lossy().to_string();
        let mut last_err = None;
        for attempt in 0..5u64 {
            match run_git(repo_root, &["worktree", "remove", "--force", &wt_str], &[]) {
                Ok(_) => {
                    last_err = None;
                    break;
                }
                Err(e) => {
                    last_err = Some(e);
                    thread::sleep(Duration::from_millis(50 * (attempt + 1)));
                }
            }
        }
        if let Some(e) = last_err {
            return Err(e);
        }
    }
    let _ = run_git(repo_root, &["worktree", "prune"], &[]);
    let _ = run_git(repo_root, &["update-ref", "-d", ref_name], &[]);
    Ok(())
}
