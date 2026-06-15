use tokio::process::Command;

use crate::target::ResolvedTarget;

/// 构造本地或 SSH 命令。远程命令作为 ssh 的单个参数交给远端登录 shell 执行。
pub fn build_command(target: &ResolvedTarget, command: &str) -> Command {
    if target.is_local() {
        let mut cmd = Command::new("sh");
        cmd.arg("-lc").arg(command);
        return cmd;
    }

    let mut cmd = Command::new("ssh");
    cmd.arg("-p")
        .arg(target.port.unwrap_or(22).to_string())
        .arg("-o")
        .arg(format!("ConnectTimeout={}", target.connect_timeout_s));
    if target.host_key_policy == "accept_new" {
        cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    }
    if let Some(destination) = target.ssh_destination() {
        cmd.arg(destination);
    }
    cmd.arg(command);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_target_uses_shell_exec() {
        let target = ResolvedTarget {
            name: None,
            host: None,
            port: None,
            username: None,
            connect_timeout_s: 10,
            host_key_policy: "openssh_default".to_string(),
        };
        let command = build_command(&target, "echo ok");
        assert_eq!(command.as_std().get_program(), "sh");
    }
}
