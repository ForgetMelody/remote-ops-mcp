pub mod model;

use std::path::Path;

use tokio::fs;

use crate::error::{ErrorKind, RemoteOpsError, Result};
pub use model::{AppConfig, Defaults, ServerConfig, TargetConfig};

pub async fn load_config(path: Option<&Path>) -> Result<AppConfig> {
    let Some(path) = path else {
        return Ok(AppConfig::default());
    };
    let content = fs::read_to_string(path).await.map_err(|err| {
        RemoteOpsError::Remote(
            ErrorKind::FileNotFound,
            format!("failed to read config '{}': {err}", path.display()),
        )
    })?;
    toml::from_str(&content).map_err(|err| {
        RemoteOpsError::Remote(
            ErrorKind::ProtocolError,
            format!("failed to parse config '{}': {err}", path.display()),
        )
    })
}
