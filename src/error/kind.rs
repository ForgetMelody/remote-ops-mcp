use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 远程操作错误分类，供 MCP 调用方做稳定分支判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    AuthFailed,
    ConnectFailed,
    HostKeyFailed,
    RemoteNonZeroExit,
    Timeout,
    CommandCancelled,
    SessionLost,
    OutputLimitExceeded,
    FileNotFound,
    PermissionDenied,
    ToolUnavailable,
    ProtocolError,
    InternalError,
}
