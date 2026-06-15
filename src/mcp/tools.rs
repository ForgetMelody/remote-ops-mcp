use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    error::RemoteError,
    job::{FollowResult, JobStatus},
    runner::file_sync::{FileCommandResult, SyncDirection},
    target::ResolvedTarget,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct TargetRequest {
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunRequest {
    pub target: Option<String>,
    pub command: String,
    pub timeout_s: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StartRequest {
    pub target: Option<String>,
    pub command: String,
    pub initial_wait_s: Option<u64>,
    pub follow_limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FollowRequest {
    pub job_id: String,
    pub cursor: Option<String>,
    pub wait_s: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobRequest {
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PathRequest {
    pub target: Option<String>,
    pub path: String,
    pub timeout_s: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileSyncRequest {
    pub target: Option<String>,
    pub direction: SyncDirection,
    pub local_path: String,
    pub remote_path: String,
    pub delete: Option<bool>,
    pub checksum: Option<bool>,
    pub dry_run: Option<bool>,
    pub timeout_s: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileCompareRequest {
    pub target: Option<String>,
    pub local_path: String,
    pub remote_path: String,
    pub checksum: Option<bool>,
    pub timeout_s: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolEnvelope<T> {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RemoteError>,
}

impl<T> ToolEnvelope<T> {
    pub fn ok(data: T) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(error: RemoteError) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BackendHealth {
    pub server: String,
    pub version: String,
    pub tools: BackendTools,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BackendTools {
    pub ssh: bool,
    pub rsync: bool,
    pub sftp: bool,
    pub scp: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TargetProbe {
    pub target: ResolvedTarget,
    pub reachable: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunResponse {
    pub result: FollowResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StartResponse {
    pub job_id: String,
    pub initial: FollowResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobListResponse {
    pub jobs: Vec<JobStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileTextResponse {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileCommandResponse {
    pub result: FileCommandResult,
}
