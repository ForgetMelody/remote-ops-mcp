use tokio::process::Command;

use crate::{
    config::AuthMethod,
    error::{ErrorKind, RemoteOpsError, Result},
    target::ResolvedTarget,
};

/// 构造本地或 SSH 命令。远程命令作为 ssh 的单个参数交给远端登录 shell 执行。
pub fn build_command(target: &ResolvedTarget, command: &str) -> Result<Command> {
    if target.is_local() {
        let mut cmd = Command::new("sh");
        cmd.arg("-lc").arg(command);
        return Ok(cmd);
    }

    let mut ssh = build_ssh_base(target)?;
    if let Some(destination) = target.ssh_destination() {
        ssh.arg(destination);
    }
    ssh.arg(command);
    Ok(ssh)
}

/// 构造常驻交互 shell 使用的 SSH 命令。调用方负责接管 stdin/stdout/stderr。
pub fn build_interactive_shell(
    target: &ResolvedTarget,
    keepalive_s: Option<u64>,
) -> Result<Command> {
    if target.is_local() {
        return Err(RemoteOpsError::Remote(
            ErrorKind::ProtocolError,
            "interactive SSH session requires a remote target".to_string(),
        ));
    }

    let mut ssh = build_ssh_base(target)?;
    ssh.arg("-tt");
    if let Some(keepalive_s) = keepalive_s {
        ssh.arg("-o")
            .arg(format!("ServerAliveInterval={keepalive_s}"))
            .arg("-o")
            .arg("ServerAliveCountMax=3");
    }
    if let Some(destination) = target.ssh_destination() {
        ssh.arg(destination);
    }
    Ok(ssh)
}

/// 构造 SSH 基础命令，供命令执行和远程文件工具复用。
pub fn build_ssh_base(target: &ResolvedTarget) -> Result<Command> {
    let mut command = match target.auth.method {
        AuthMethod::Openssh => Command::new("ssh"),
        AuthMethod::Password => {
            let password = target.auth.password.as_deref().ok_or_else(|| {
                RemoteOpsError::Remote(
                    ErrorKind::ProtocolError,
                    "password auth requires auth.password".to_string(),
                )
            })?;
            let mut command = Command::new("sshpass");
            command.arg("-e").arg("ssh").env("SSHPASS", password);
            command
        }
    };
    append_ssh_options(&mut command, target);
    Ok(command)
}

/// 构造 rsync -e 参数。sshpass 使用环境变量拿密码，返回值不包含明文密码。
pub fn build_rsync_remote_shell(target: &ResolvedTarget) -> Result<(String, Option<String>)> {
    let mut shell = match target.auth.method {
        AuthMethod::Openssh => "ssh".to_string(),
        AuthMethod::Password => {
            let password = target.auth.password.clone().ok_or_else(|| {
                RemoteOpsError::Remote(
                    ErrorKind::ProtocolError,
                    "password auth requires auth.password".to_string(),
                )
            })?;
            return Ok((
                format!("sshpass -e ssh{}", ssh_options_string(target)),
                Some(password),
            ));
        }
    };
    shell.push_str(&ssh_options_string(target));
    Ok((shell, None))
}

fn append_ssh_options(command: &mut Command, target: &ResolvedTarget) {
    command
        .arg("-p")
        .arg(target.port.unwrap_or(22).to_string())
        .arg("-o")
        .arg(format!("ConnectTimeout={}", target.connect_timeout_s));
    if target.host_key_policy == "accept_new" {
        command.arg("-o").arg("StrictHostKeyChecking=accept-new");
    }
    if target.auth.method == AuthMethod::Password {
        command
            .arg("-o")
            .arg("PreferredAuthentications=password,keyboard-interactive")
            .arg("-o")
            .arg("PubkeyAuthentication=no")
            .arg("-o")
            .arg("NumberOfPasswordPrompts=1");
    }
}

fn ssh_options_string(target: &ResolvedTarget) -> String {
    let mut options = format!(
        " -p {} -o ConnectTimeout={}",
        target.port.unwrap_or(22),
        target.connect_timeout_s
    );
    if target.host_key_policy == "accept_new" {
        options.push_str(" -o StrictHostKeyChecking=accept-new");
    }
    if target.auth.method == AuthMethod::Password {
        options.push_str(" -o PreferredAuthentications=password,keyboard-interactive");
        options.push_str(" -o PubkeyAuthentication=no");
        options.push_str(" -o NumberOfPasswordPrompts=1");
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;

    #[test]
    fn local_target_uses_shell_exec() {
        let target = ResolvedTarget {
            name: None,
            host: None,
            port: None,
            username: None,
            connect_timeout_s: 10,
            host_key_policy: "openssh_default".to_string(),
            auth: AuthConfig::default(),
        };
        let command = build_command(&target, "echo ok").unwrap();
        assert_eq!(command.as_std().get_program(), "sh");
    }

    #[test]
    fn password_target_uses_sshpass_without_argv_password() {
        let target = ResolvedTarget {
            name: Some("<password>".to_string()),
            host: Some("devbox.example.com".to_string()),
            port: Some(22),
            username: Some("deploy".to_string()),
            connect_timeout_s: 10,
            host_key_policy: "accept_new".to_string(),
            auth: AuthConfig {
                method: AuthMethod::Password,
                password: Some("<password>".to_string()),
            },
        };
        let command = build_command(&target, "printf ok").unwrap();
        assert_eq!(command.as_std().get_program(), "sshpass");
        let args: Vec<_> = command.as_std().get_args().collect();
        assert!(!args.iter().any(|arg| arg.to_string_lossy().contains("<password>")));
    }
}
