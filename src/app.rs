use std::{path::PathBuf, sync::Arc};

use clap::Parser;

use crate::{
    config::{AppConfig, load_config},
    job::JobManager,
};

/// RemoteOps MCP 命令行参数。
#[derive(Debug, Parser)]
pub struct Cli {
    #[arg(long, env = "REMOTE_OPS_CONFIG")]
    pub config: Option<PathBuf>,
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub jobs: JobManager,
}

impl AppState {
    pub async fn load(cli: &Cli) -> crate::error::Result<Self> {
        let config = load_config(cli.config.as_deref()).await?;
        let output_max_bytes = config.defaults.output_max_bytes;
        Ok(Self {
            config: Arc::new(config),
            jobs: JobManager::new(output_max_bytes),
        })
    }
}
