use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// MCP server 自身配置。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ServerConfig {
    pub transport: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            transport: "stdio".to_string(),
        }
    }
}

/// 全局默认值，所有时间单位均为秒。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Defaults {
    pub connect_timeout_s: u64,
    pub run_timeout_s: u64,
    pub initial_wait_s: u64,
    pub follow_wait_s: u64,
    pub follow_limit: usize,
    pub output_max_bytes: usize,
    pub host_key_policy: String,
    pub auth: AuthConfig,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            connect_timeout_s: 10,
            run_timeout_s: 30,
            initial_wait_s: 1,
            follow_wait_s: 5,
            follow_limit: 8192,
            output_max_bytes: 8 * 1024 * 1024,
            host_key_policy: "openssh_default".to_string(),
            auth: AuthConfig::default(),
        }
    }
}

/// SSH 认证方式。默认沿用 OpenSSH 配置、agent 和密钥。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    #[default]
    Openssh,
    Password,
}

/// 单个目标的认证配置。`password` 仅在 `method = "password"` 时使用。
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AuthConfig {
    pub method: AuthMethod,
    #[serde(skip_serializing)]
    #[schemars(skip)]
    pub password: Option<String>,
}

/// 单个远程目标配置。认证可显式指定，也可继续继承全局默认认证。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TargetConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub connect_timeout_s: Option<u64>,
    pub host_key_policy: Option<String>,
    pub auth: Option<AuthConfig>,
}

impl Default for TargetConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 22,
            username: None,
            connect_timeout_s: None,
            host_key_policy: None,
            auth: None,
        }
    }
}

/// RemoteOps MCP 配置根节点。
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub defaults: Defaults,
    pub targets: BTreeMap<String, TargetConfig>,
}
