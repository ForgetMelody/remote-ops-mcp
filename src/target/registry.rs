use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    config::{AppConfig, AuthConfig, AuthMethod, TargetConfig},
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
    pub auth: AuthConfig,
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
        None => Ok(local_target(config, None)),
        Some(name) if name == "local" => Ok(local_target(config, Some(name))),
        Some(name) => {
            if let Some(target) = config.targets.get(&name) {
                return resolve_named_target(config, name, target);
            }
            resolve_inline_target(config, name)
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
        auth: target
            .auth
            .clone()
            .unwrap_or_else(|| config.defaults.auth.clone()),
    })
}

fn local_target(config: &AppConfig, name: Option<String>) -> ResolvedTarget {
    ResolvedTarget {
        name,
        host: None,
        port: None,
        username: None,
        connect_timeout_s: config.defaults.connect_timeout_s,
        host_key_policy: config.defaults.host_key_policy.clone(),
        auth: AuthConfig {
            method: AuthMethod::Openssh,
            password: None,
        },
    }
}

fn resolve_inline_target(config: &AppConfig, raw: String) -> Result<ResolvedTarget> {
    let spec = parse_inline_target(&raw)?;
    Ok(ResolvedTarget {
        name: Some(raw),
        host: Some(spec.host),
        port: Some(spec.port.unwrap_or(22)),
        username: spec.username,
        connect_timeout_s: config.defaults.connect_timeout_s,
        host_key_policy: config.defaults.host_key_policy.clone(),
        auth: config.defaults.auth.clone(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InlineTargetSpec {
    username: Option<String>,
    host: String,
    port: Option<u16>,
}

fn parse_inline_target(raw: &str) -> Result<InlineTargetSpec> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(RemoteOpsError::Remote(
            ErrorKind::ProtocolError,
            "target must not be empty".to_string(),
        ));
    }
    if trimmed.contains('/') || trimmed.contains(char::is_whitespace) {
        return Err(unknown_target(raw));
    }
    let (username, host_port) = split_user(trimmed)?;
    let (host, port) = split_host_port(host_port)?;
    validate_host(raw, host)?;
    Ok(InlineTargetSpec {
        username,
        host: host.to_string(),
        port,
    })
}

fn split_user(input: &str) -> Result<(Option<String>, &str)> {
    let Some((username, host_port)) = input.rsplit_once('@') else {
        return Ok((None, input));
    };
    if username.is_empty() || host_port.is_empty() {
        return Err(unknown_target(input));
    }
    Ok((Some(username.to_string()), host_port))
}

fn split_host_port(input: &str) -> Result<(&str, Option<u16>)> {
    if input.starts_with('[') {
        return split_bracket_host_port(input);
    }
    match input.rsplit_once(':') {
        Some((host, _)) if host.contains(':') => Ok((input, None)),
        Some((host, port_text)) => Ok((host, Some(parse_port(input, port_text)?))),
        None => Ok((input, None)),
    }
}

fn split_bracket_host_port(input: &str) -> Result<(&str, Option<u16>)> {
    let Some(close) = input.find(']') else {
        return Err(unknown_target(input));
    };
    let host = &input[1..close];
    let rest = &input[close + 1..];
    if rest.is_empty() {
        return Ok((host, None));
    }
    let Some(port_text) = rest.strip_prefix(':') else {
        return Err(unknown_target(input));
    };
    Ok((host, Some(parse_port(input, port_text)?)))
}

fn parse_port(raw: &str, port_text: &str) -> Result<u16> {
    if port_text.is_empty() {
        return Err(unknown_target(raw));
    }
    port_text.parse::<u16>().map_err(|_| unknown_target(raw))
}

fn validate_host(raw: &str, host: &str) -> Result<()> {
    if host.is_empty() || host.starts_with('-') {
        return Err(unknown_target(raw));
    }
    Ok(())
}

fn unknown_target(name: &str) -> RemoteOpsError {
    RemoteOpsError::Remote(ErrorKind::ProtocolError, format!("unknown target '{name}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_inline_user_host_port() {
        let config = AppConfig::default();
        let target = resolve_target(&config, Some("deploy@devbox.example.com".to_string())).unwrap();
        assert_eq!(target.name.as_deref(), Some("deploy@devbox.example.com"));
        assert_eq!(target.username.as_deref(), Some("deploy"));
        assert_eq!(target.host.as_deref(), Some("devbox.example.com"));
        assert_eq!(target.port, Some(2222));
        assert_eq!(target.auth.method, AuthMethod::Openssh);
    }

    #[test]
    fn rejects_path_like_unknown_target() {
        let config = AppConfig::default();
        let err =
            resolve_target(&config, Some("deploy@devbox.example.com/tmp".to_string())).unwrap_err();
        assert!(err.to_string().contains("unknown target"));
    }

    #[test]
    fn named_target_inherits_default_password_auth() {
        let mut config = AppConfig::default();
        config.defaults.auth = AuthConfig {
            method: AuthMethod::Password,
            password: Some("<password>".to_string()),
        };
        config.targets.insert(
            "<password>".to_string(),
            TargetConfig {
                host: "devbox.example.com".to_string(),
                username: Some("deploy".to_string()),
                ..TargetConfig::default()
            },
        );
        let target = resolve_target(&config, Some("<password>".to_string())).unwrap();
        assert_eq!(target.auth.method, AuthMethod::Password);
        assert_eq!(target.auth.password.as_deref(), Some("<password>"));
    }
}
