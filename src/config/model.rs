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
        }
    }
}

/// 单个远程目标配置。认证细节默认交给 OpenSSH 配置和 agent 处理。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TargetConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub connect_timeout_s: Option<u64>,
    pub host_key_policy: Option<String>,
}

impl Default for TargetConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 22,
            username: None,
            connect_timeout_s: None,
            host_key_policy: None,
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
