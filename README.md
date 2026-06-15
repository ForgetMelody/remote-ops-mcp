# RemoteOps MCP

RemoteOps MCP 是一个通用远程操作 MCP server，面向 SSH 命令管理、长任务输出跟踪和基于 `rsync` 的文件同步。

## 当前能力

- MCP transport：stdio。
- 命令执行：本机 `local`、命名 target、临时内联 target（`[user@]devbox.example.com[:port]`）。
- 认证：默认 OpenSSH 配置/密钥/agent；可在 TOML 中显式配置 `method = "password"` 和明文 `password`。
- 配置：TOML，工具调用会按 `--config` 路径重读配置，新增 target/auth 不需要重启 MCP server。
- Job 模型：`remote_start`、`remote_follow`、`remote_stop`、`remote_job_status`、`remote_job_list`。
- 文件操作：`remote_file_list`、`remote_file_stat`、`remote_file_sync`、`remote_file_compare`。
- 会话调试：`remote_session_ensure`、`remote_session_exec`、`remote_session_start/follow/signal/cancel` 提供常驻 OpenSSH shell，会保留 cwd/env/source 状态。
- 文件同步后端：第一版只启用系统 `rsync`。

## 构建、安装与验证

Rust/Cargo 常用形态：

- `cargo build --release --locked`：生成发布二进制到 `target/release/`。
- `cargo install --path . --locked --force`：类似本项目的本机 install，把可执行文件安装到 `$CARGO_HOME/bin`，默认是 `~/.cargo/bin`。
- `cargo package` / `cargo publish`：面向 crates.io 的源码包发布；系统包可另接 `cargo-deb`、`cargo-rpm`、`cargo-dist` 等工具。

本机安装：

```bash
cargo build --release --locked
cargo install --path . --locked --force
```

质量检查：

```bash
cargo fmt --all -- --check
cargo test --all
cargo clippy --all-targets --all-features -- -D warnings
```

## 运行

```bash
remote-ops-mcp --config ~/.config/remote-ops/config.toml
```

MCP client 配置示例：

```json
{
  "mcpServers": {
    "remote-ops": {
      "type": "stdio",
      "command": "/path/to/remote-ops-mcp",
      "args": ["--config", "/path/to/remote-ops/config.toml"],
      "env": {
        "RUST_LOG": "remote_ops_mcp=info"
      },
      "enabled": true,
      "timeout": 30000
    }
  }
}
```

## Target 和认证配置

- 临时目标：工具调用的 `target` 可直接传 `[user@]devbox.example.com[:port]`，例如 `deploy@devbox.example.com`，无需写入 `targets`。
- 热加载范围覆盖 target、auth 和每次调用读取的默认超时/输出参数；`output_max_bytes` 影响 JobManager 环形缓冲大小，仍在 server 启动时固定。
- 命名目标：`[targets.<name>]` 适合常用设备；每次工具调用都会重读 `--config` 文件，因此新增或修改 target/auth 后无需重启 MCP server。
- 明文密码：在 `[defaults.auth]` 或 `[targets.devbox]` 中配置 `method = "password"` 与 `password = "<password>"`。运行时通过 `sshpass -e` 和 `SSHPASS` 环境变量传递，避免密码出现在进程 argv 或 MCP 返回的 target 结构中。
- 默认认证：未配置 auth 时使用 `method = "openssh"`，沿用系统 OpenSSH 配置、密钥和 agent。

```toml
[defaults]
host_key_policy = "accept_new"

[defaults.auth]
method = "password"
password = "<password>"

[targets.<password>]
host = "devbox.example.com"
port = 22
username = "deploy"
```

使用示例：

```json
{"target": "deploy@devbox.example.com", "command": "hostname"}
```

## 常驻 SSH 会话

一次性工具保持隔离：`remote_run` 每次新建 SSH 进程，`remote_file_*` 每次独立走 `ssh/rsync`。需要旧版 viobot-remote 风格调试时，使用 `remote_session_*`：

- `remote_session_ensure`：按解析后的 `host + port + username + session_tag` 复用 idle 且 alive 的会话；没有可复用会话则新建。
- `remote_session_exec`：在同一远端 shell 内执行短命令，保留 `cd`、`export`、`source` 等状态。
- `remote_session_start` / `remote_session_follow`：运行长任务并增量拉取输出。
- `remote_session_signal`：向运行中的会话命令发送 `SIGINT` / `SIGTSTP` / `SIGQUIT`，对应 Ctrl-C / Ctrl-Z / Ctrl-\\。
- `remote_session_cancel`：取消命令并关闭该会话，避免 marker 状态不一致后继续复用。

示例：

```json
{"target": "deploy@devbox.example.com", "session_tag": "run", "keepalive_s": 30}
```

拿到 `session_id` 后：

```json
{"session_id": "<password>", "command": "cd /tmp && export REMOTE_OPS_SESSION_TEST=ok"}
```

同一 `session_id` 的后续命令会继承该 shell 状态。



## 工具接口

### 命令管理

- `remote_backend_health`
- `remote_target_probe`
- `remote_run`
- `remote_start`
- `remote_follow`
- `remote_stop`
- `remote_job_status`
- `remote_job_list`

### 常驻会话

- `remote_session_open`
- `remote_session_ensure`
- `remote_session_close`
- `remote_session_get`
- `remote_session_list`
- `remote_session_exec`
- `remote_session_start`
- `remote_session_follow`
- `remote_session_signal`
- `remote_session_cancel`

### 文件操作

- `remote_file_list`
- `remote_file_stat`
- `remote_file_sync`
- `remote_file_compare`

## 设计记录

详细方案、命名、架构和落地状态见 `docs/remote-ops-mcp-plan.md`。
