use anyhow::Result;
use clap::Parser;
use remote_ops_mcp::{
    app::{AppState, Cli},
    mcp::RemoteOpsService,
};
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let state = AppState::load(&cli).await?;
    let service = RemoteOpsService::new(state).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("remote_ops_mcp=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
}
