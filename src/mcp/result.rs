use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, Content},
};
use serde::Serialize;
use serde_json::json;

use crate::{error::RemoteOpsError, mcp::tools::ToolEnvelope};

pub fn json_result<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(value).map_err(|err| {
        McpError::internal_error(
            "failed to serialize tool result",
            Some(json!({ "reason": err.to_string() })),
        )
    })?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

pub fn envelope_result<T: Serialize>(
    result: Result<T, RemoteOpsError>,
) -> Result<CallToolResult, McpError> {
    match result {
        Ok(data) => json_result(&ToolEnvelope::ok(data)),
        Err(err) => json_result(&ToolEnvelope::<T>::err(err.into_remote_error())),
    }
}
