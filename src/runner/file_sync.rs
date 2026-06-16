use std::{borrow::Cow, process::Stdio};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::{
    error::{ErrorKind, RemoteOpsError, Result},
    runner::{
        process::{ProcessOutput, run_command},
        ssh::{build_rsync_remote_shell, build_ssh_base},
    },
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
        let expanded = expand_local_tilde(path)?;
        let mut cmd = Command::new("find");
        cmd.arg(expanded.as_ref())
            .arg("-maxdepth")
            .arg("1")
            .arg("-print");
        return run_command(cmd, timeout_s).await;
    }
    let mut command = build_ssh_base(target)?;
    if let Some(destination) = target.ssh_destination() {
        command.arg(destination);
    }
    command.arg(format!(
        "find {} -maxdepth 1 -print",
        remote_shell_path(path)
    ));
    run_command(command, timeout_s).await
}

pub async fn stat_path(
    target: &ResolvedTarget,
    path: &str,
    timeout_s: u64,
) -> Result<ProcessOutput> {
    if target.is_local() {
        let expanded = expand_local_tilde(path)?;
        let mut cmd = Command::new("stat");
        cmd.arg(expanded.as_ref());
        return run_command(cmd, timeout_s).await;
    }
    let mut command = build_ssh_base(target)?;
    if let Some(destination) = target.ssh_destination() {
        command.arg(destination);
    }
    command.arg(format!("stat {}", remote_shell_path(path)));
    run_command(command, timeout_s).await
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
        let (remote_shell, password) = build_rsync_remote_shell(target)?;
        cmd.arg("-e").arg(remote_shell);
        if let Some(password) = password {
            cmd.env("SSHPASS", password);
        }
    }
    let local_path = expand_local_tilde(options.local_path)?;
    let remote_path = if target.is_local() {
        expand_local_tilde(options.remote_path)?.into_owned()
    } else {
        remote_rsync_path(options.remote_path).into_owned()
    };
    let remote = if target.is_local() {
        remote_path
    } else {
        format!(
            "{}:{}",
            target
                .ssh_destination()
                .ok_or_else(|| RemoteOpsError::Remote(
                    ErrorKind::ProtocolError,
                    "missing ssh destination".to_string()
                ))?,
            remote_path
        )
    };
    match options.direction {
        SyncDirection::Push => {
            cmd.arg(local_path.as_ref()).arg(remote);
        }
        SyncDirection::Pull => {
            cmd.arg(remote).arg(local_path.as_ref());
        }
    }
    run_command(cmd, options.timeout_s).await
}

fn expand_local_tilde(path: &str) -> Result<Cow<'_, str>> {
    expand_current_user_tilde(path, std::env::var("HOME").ok().as_deref())
}

fn expand_current_user_tilde<'a>(path: &'a str, home: Option<&str>) -> Result<Cow<'a, str>> {
    let Some(tail) = current_user_tilde_tail(path) else {
        return Ok(Cow::Borrowed(path));
    };
    let home = home.filter(|home| !home.is_empty()).ok_or_else(|| {
        RemoteOpsError::Remote(
            ErrorKind::ProtocolError,
            "HOME is not set; cannot expand '~'".to_string(),
        )
    })?;
    Ok(Cow::Owned(join_home(home, tail)))
}

fn remote_shell_path(path: &str) -> String {
    let Some(tail) = current_user_tilde_tail(path) else {
        return shell_quote(path);
    };
    remote_home_shell_path(tail)
}

fn remote_rsync_path(path: &str) -> Cow<'_, str> {
    let Some(tail) = current_user_tilde_tail(path) else {
        return Cow::Borrowed(path);
    };
    Cow::Owned(remote_home_shell_path(tail))
}

fn remote_home_shell_path(tail: &str) -> String {
    if tail.is_empty() {
        return r#""${HOME}""#.to_string();
    }
    format!(r#""${{HOME}}"/{}"#, shell_quote(tail))
}

fn current_user_tilde_tail(path: &str) -> Option<&str> {
    if path == "~" {
        return Some("");
    }
    path.strip_prefix("~/")
}

fn join_home(home: &str, tail: &str) -> String {
    if tail.is_empty() {
        return home.to_string();
    }
    if home == "/" {
        format!("/{tail}")
    } else {
        format!("{home}/{tail}")
    }
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

    #[test]
    fn expands_current_user_tilde_for_local_paths() {
        assert_eq!(
            &*expand_current_user_tilde("~", Some("/home/me")).unwrap(),
            "/home/me"
        );
        assert_eq!(
            &*expand_current_user_tilde("~/ws", Some("/home/me")).unwrap(),
            "/home/me/ws"
        );
        assert_eq!(
            &*expand_current_user_tilde("~/ws", Some("/")).unwrap(),
            "/ws"
        );
        assert_eq!(
            &*expand_current_user_tilde("~other/ws", Some("/home/me")).unwrap(),
            "~other/ws"
        );
    }

    #[test]
    fn rejects_local_tilde_without_home() {
        let err = expand_current_user_tilde("~/ws", None).unwrap_err();
        assert!(err.to_string().contains("HOME is not set"));
    }

    #[test]
    fn builds_remote_shell_paths_with_home_expansion() {
        assert_eq!(remote_shell_path("~"), r#""${HOME}""#);
        assert_eq!(remote_shell_path("~/a b"), r#""${HOME}"/'a b'"#);
        assert_eq!(remote_shell_path("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(remote_rsync_path("~/a b").as_ref(), r#""${HOME}"/'a b'"#);
        assert_eq!(remote_rsync_path("/tmp/a b").as_ref(), "/tmp/a b");
    }
}
