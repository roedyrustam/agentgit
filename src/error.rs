use thiserror::Error;

/// Domain-specific errors for AgentGit operations.
#[derive(Debug, Error)]
pub enum AgentGitError {
    #[error("Not inside a valid Git repository: {0}")]
    NotAGitRepo(String),

    #[error("AgentGit is not initialized in this repository. Run `agentgit init` first.")]
    NotInitialized,

    #[error("Invalid identifier '{0}': must match [a-zA-Z0-9_-]+ and contain no path separators")]
    InvalidIdentifier(String),

    #[error("Agent '{0}' not found in active session")]
    AgentNotFound(String),

    #[error("Agent '{0}' already exists in active session")]
    AgentAlreadyExists(String),

    #[error("Causal node '{0}' not found")]
    NodeNotFound(String),

    #[error("Merge conflict detected during promote pre-flight:\n{0}")]
    MergeConflict(String),

    #[error("Git command failed ({cmd}): {stderr}")]
    GitCommandFailed { cmd: String, stderr: String },

    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, AgentGitError>;
