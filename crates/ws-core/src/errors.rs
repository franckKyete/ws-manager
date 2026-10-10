use thiserror::Error;

#[derive(Error, Debug)]
pub enum WSError {
    #[error("{0}")]
    General(String),

    #[error("{0}")]
    Workspace(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Workspace '{0}' already exists.")]
    WorkspaceExists(String),

    #[error("Workspace '{0}' does not exist.")]
    WorkspaceNotFound(String),

    #[error("{0}")]
    RepositoryNotFound(String),

    #[error("{0}")]
    BranchNotFound(String),

    #[error("{0}")]
    BranchAlreadyExists(String),

    #[error("Git error: {message}")]
    Git {
        message: String,
        command: Option<String>,
        returncode: Option<i32>,
        stderr: Option<String>,
    },

    #[error("Rollback error: {0}")]
    Rollback(String),

    #[error("{0}")]
    RepoFrozen(String),

    #[error("{0}")]
    RepoAlreadyInWorkspace(String),

    #[error("{0}")]
    RepoNotInWorkspace(String),

    #[error("{0}")]
    WorkspaceUncommitted(String),

    #[error("{0}")]
    WorkspaceUnmerged(String),

    #[error("{0}")]
    SessionStop(String),

    #[error("wshub error ({status_code}): {message}")]
    Hub {
        message: String,
        status_code: u16,
        details: Option<serde_json::Value>,
    },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, WSError>;
