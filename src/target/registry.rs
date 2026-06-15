use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    config::{AppConfig, TargetConfig},
    error::{ErrorKind, RemoteOpsError, Result},
};

/// MCP 对外引用的目标。`None` 表示本机执行。
pub type TargetName = Option<String>;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResolvedTarget {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub connect_timeout_s: u64,
    pub host_key_policy: String,
}

impl ResolvedTarget {
    pub fn is_local(&self) -> bool {
        self.host.is_none()
    }

    pub fn ssh_destination(&self) -> Option<String> {
        let host = self.host.as_ref()?;
        match &self.username {
            Some(username) if !username.is_empty() => Some(format!("{username}@{host}")),
            _ => Some(host.clone()),
        }
    }
}

pub fn resolve_target(config: &AppConfig, target: TargetName) -> Result<ResolvedTarget> {
    match target {
        None => Ok(ResolvedTarget {
            name: None,
            host: None,
            port: None,
            username: None,
            connect_timeout_s: config.defaults.connect_timeout_s,
            host_key_policy: config.defaults.host_key_policy.clone(),
        }),
        Some(name) if name == "local" => Ok(ResolvedTarget {
            name: Some(name),
            host: None,
            port: None,
            username: None,
            connect_timeout_s: config.defaults.connect_timeout_s,
            host_key_policy: config.defaults.host_key_policy.clone(),
        }),
        Some(name) => {
            let target = config.targets.get(&name).ok_or_else(|| {
                RemoteOpsError::Remote(ErrorKind::ProtocolError, format!("unknown target '{name}'"))
            })?;
            resolve_named_target(config, name, target)
        }
    }
}

fn resolve_named_target(
    config: &AppConfig,
    name: String,
    target: &TargetConfig,
) -> Result<ResolvedTarget> {
    if target.host.is_empty() {
        return Err(RemoteOpsError::Remote(
            ErrorKind::ProtocolError,
            format!("target '{name}' has empty host"),
        ));
    }
    Ok(ResolvedTarget {
        name: Some(name),
        host: Some(target.host.clone()),
        port: Some(target.port),
        username: target.username.clone(),
        connect_timeout_s: target
            .connect_timeout_s
            .unwrap_or(config.defaults.connect_timeout_s),
        host_key_policy: target
            .host_key_policy
            .clone()
            .unwrap_or_else(|| config.defaults.host_key_policy.clone()),
    })
}
