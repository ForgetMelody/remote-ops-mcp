pub mod kind;
pub mod redact;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use kind::ErrorKind;

/// MCP 工具统一错误结构。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemoteError {
    pub kind: ErrorKind,
    pub message: String,
    pub retryable: bool,
    pub target: Option<String>,
    pub job_id: Option<String>,
}

impl RemoteError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        let retryable = matches!(
            kind,
            ErrorKind::ConnectFailed | ErrorKind::Timeout | ErrorKind::SessionLost
        );
        Self {
            kind,
            message: message.into(),
            retryable,
            target: None,
            job_id: None,
        }
    }

    #[cfg(test)]
    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    #[cfg(test)]
    pub fn with_job_id(mut self, job_id: impl Into<String>) -> Self {
        self.job_id = Some(job_id.into());
        self
    }
}

#[derive(Debug, Error)]
pub enum RemoteOpsError {
    #[error("{0:?}: {1}")]
    Remote(ErrorKind, String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

impl RemoteOpsError {
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Remote(kind, _) => *kind,
            Self::Io(_) => ErrorKind::InternalError,
            Self::Json(_) => ErrorKind::ProtocolError,
        }
    }

    pub fn into_remote_error(self) -> RemoteError {
        RemoteError::new(self.kind(), self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, RemoteOpsError>;
