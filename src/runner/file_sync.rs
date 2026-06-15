use std::process::Stdio;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::{
    error::{ErrorKind, RemoteOpsError, Result},
    runner::process::{ProcessOutput, run_command},
    target::ResolvedTarget,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncDirection {
    Push,
    Pull,
}

#[derive(Debug, Clone)]
pub struct SyncOptions<'a> {
    pub direction: SyncDirection,
    pub local_path: &'a str,
    pub remote_path: &'a str,
    pub delete: bool,
    pub checksum: bool,
    pub dry_run: bool,
    pub timeout_s: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileCommandResult {
    pub backend: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

pub async fn list_path(
    target: &ResolvedTarget,
    path: &str,
    timeout_s: u64,
) -> Result<ProcessOutput> {
    if target.is_local() {
        let mut cmd = Command::new("find");
        cmd.arg(path).arg("-maxdepth").arg("1").arg("-print");
        return run_command(cmd, timeout_s).await;
    }
    let mut cmd = ssh_base(target);
    cmd.arg(format!("find {} -maxdepth 1 -print", shell_quote(path)));
    run_command(cmd, timeout_s).await
}

pub async fn stat_path(
    target: &ResolvedTarget,
    path: &str,
    timeout_s: u64,
) -> Result<ProcessOutput> {
    if target.is_local() {
        let mut cmd = Command::new("stat");
        cmd.arg(path);
        return run_command(cmd, timeout_s).await;
    }
    let mut cmd = ssh_base(target);
    cmd.arg(format!("stat {}", shell_quote(path)));
    run_command(cmd, timeout_s).await
}

pub async fn sync_path(
    target: &ResolvedTarget,
    options: SyncOptions<'_>,
) -> Result<FileCommandResult> {
    ensure_rsync_available().await?;
    let output = run_rsync(target, options).await?;
    Ok(FileCommandResult {
        backend: "rsync".to_string(),
        exit_code: output.exit_code,
        stdout: output.stdout,
        stderr: output.stderr,
        timed_out: output.timed_out,
    })
}

pub async fn compare_path(
    target: &ResolvedTarget,
    local_path: &str,
    remote_path: &str,
    checksum: bool,
    timeout_s: u64,
) -> Result<FileCommandResult> {
    ensure_rsync_available().await?;
    let output = run_rsync(
        target,
        SyncOptions {
            direction: SyncDirection::Push,
            local_path,
            remote_path,
            delete: false,
            checksum,
            dry_run: true,
            timeout_s,
        },
    )
    .await?;
    Ok(FileCommandResult {
        backend: "rsync".to_string(),
        exit_code: output.exit_code,
        stdout: output.stdout,
        stderr: output.stderr,
        timed_out: output.timed_out,
    })
}

async fn ensure_rsync_available() -> Result<()> {
    if command_exists("rsync").await {
        return Ok(());
    }
    Err(RemoteOpsError::Remote(
        ErrorKind::ToolUnavailable,
        "rsync is not available".to_string(),
    ))
}

async fn command_exists(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

async fn run_rsync(target: &ResolvedTarget, options: SyncOptions<'_>) -> Result<ProcessOutput> {
    let mut cmd = Command::new("rsync");
    cmd.arg("-a");
    if options.delete {
        cmd.arg("--delete");
    }
    if options.checksum {
        cmd.arg("--checksum");
    }
    if options.dry_run {
        cmd.arg("--dry-run").arg("--itemize-changes");
    }
    if !target.is_local() {
        cmd.arg("-e").arg(format!(
            "ssh -p {} -o ConnectTimeout={}",
            target.port.unwrap_or(22),
            target.connect_timeout_s
        ));
    }
    let remote = if target.is_local() {
        options.remote_path.to_string()
    } else {
        format!(
            "{}:{}",
            target
                .ssh_destination()
                .ok_or_else(|| RemoteOpsError::Remote(
                    ErrorKind::ProtocolError,
                    "missing ssh destination".to_string()
                ))?,
            options.remote_path
        )
    };
    match options.direction {
        SyncDirection::Push => {
            cmd.arg(options.local_path).arg(remote);
        }
        SyncDirection::Pull => {
            cmd.arg(remote).arg(options.local_path);
        }
    }
    run_command(cmd, options.timeout_s).await
}

fn ssh_base(target: &ResolvedTarget) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.arg("-p")
        .arg(target.port.unwrap_or(22).to_string())
        .arg("-o")
        .arg(format!("ConnectTimeout={}", target.connect_timeout_s));
    if let Some(destination) = target.ssh_destination() {
        cmd.arg(destination);
    }
    cmd
}

fn shell_quote(input: &str) -> String {
    if input.is_empty() {
        return "''".to_string();
    }
    let mut out = String::with_capacity(input.len() + 2);
    out.push('\'');
    for ch in input.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_shell_paths() {
        assert_eq!(shell_quote("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_quote(""), "''");
    }
}
