use std::{path::PathBuf, sync::Arc};

use clap::Parser;

use crate::{
    config::{Defaults, load_config},
    job::JobManager,
    session::SessionManager,
};

/// RemoteOps MCP 命令行参数。
#[derive(Debug, Parser)]
pub struct Cli {
    #[arg(long, env = "REMOTE_OPS_CONFIG")]
    pub config: Option<PathBuf>,
}

#[derive(Clone)]
pub struct AppState {
    pub startup_defaults: Defaults,
    pub config_path: Option<Arc<PathBuf>>,
    pub jobs: JobManager,
    pub sessions: SessionManager,
}

impl AppState {
    pub async fn load(cli: &Cli) -> crate::error::Result<Self> {
        let config = load_config(cli.config.as_deref()).await?;
        let output_max_bytes = config.defaults.output_max_bytes;
        Ok(Self {
            startup_defaults: config.defaults,
            config_path: cli.config.clone().map(Arc::new),
            jobs: JobManager::new(output_max_bytes),
            sessions: SessionManager::new(output_max_bytes),
        })
    }

    /// 每次工具调用重读配置文件，让 MCP server 不重启也能感知 target/auth 变化。
    pub async fn current_config(&self) -> crate::error::Result<crate::config::AppConfig> {
        match self.config_path.as_deref() {
            Some(path) => load_config(Some(path.as_path())).await,
            None => Ok(crate::config::AppConfig {
                defaults: self.startup_defaults.clone(),
                ..crate::config::AppConfig::default()
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[tokio::test]
    async fn current_config_rereads_config_file() {
        let path = std::env::temp_dir().join(format!(
            "remote-ops-hot-reload-{}.toml",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, "[defaults]\nrun_timeout_s = 11\n").unwrap();

        let cli = Cli {
            config: Some(path.clone()),
        };
        let state = AppState::load(&cli).await.unwrap();
        fs::write(
            &path,
            "[defaults]\nrun_timeout_s = 12\n\n[targets.hot]\nhost = \"127.0.0.1\"\n",
        )
        .unwrap();

        let config = state.current_config().await.unwrap();
        assert_eq!(config.defaults.run_timeout_s, 12);
        assert!(config.targets.contains_key("hot"));

        fs::remove_file(path).unwrap();
    }
}
