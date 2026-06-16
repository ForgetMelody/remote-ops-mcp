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

## 平台支持

- Linux：当前 CI 和 Release 覆盖 `x86_64-unknown-linux-gnu`、`aarch64-unknown-linux-gnu`。
- macOS：暂未纳入 Release 矩阵；发布前需要验证系统 `ssh`、`rsync`、`stat`、`find` 和可选 `sshpass`。
- Windows：暂不发布原生包；当前本地命令使用 POSIX shell，文件工具依赖 `rsync/stat/find`，建议先通过 WSL 使用。

## 安装

### 方式一：源码编译安装

依赖：

- Rust stable 与 Cargo。
- 系统命令：`ssh`、`rsync`。
- 可选命令：`sshpass`，仅在配置 `method = "password"` 时需要。

```bash
git clone "https://github.com/ForgetMelody/remote-ops-mcp"
cd remote-ops-mcp
cargo build --release --locked
cargo install --path . --locked --force
remote-ops-mcp --help
```

已在源码目录时，只需要：

```bash
cargo install --path . --locked --force
```

本地质量检查：

```bash
cargo fmt --all -- --check
cargo test --all --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

### 方式二：下载 Release 安装

Release 提供 Linux 可执行文件和 `tar.gz` 归档：

- `remote-ops-mcp-x86_64-unknown-linux-gnu`
- `remote-ops-mcp-aarch64-unknown-linux-gnu`
- `remote-ops-mcp-x86_64-unknown-linux-gnu.tar.gz`
- `remote-ops-mcp-aarch64-unknown-linux-gnu.tar.gz`

直接下载可执行文件到 `~/.local/bin`：

```bash
set -euo pipefail
REPO="ForgetMelody/remote-ops-mcp"

case "$(uname -m)" in
  x86_64|amd64) TARGET="x86_64-unknown-linux-gnu" ;;
  aarch64|arm64) TARGET="aarch64-unknown-linux-gnu" ;;
  *) echo "unsupported arch: $(uname -m)" >&2; exit 1 ;;
esac

mkdir -p "${HOME}/.local/bin"
curl -fL "https://github.com/${REPO}/releases/latest/download/remote-ops-mcp-${TARGET}" \
  -o "${HOME}/.local/bin/remote-ops-mcp"
chmod 0755 "${HOME}/.local/bin/remote-ops-mcp"
"${HOME}/.local/bin/remote-ops-mcp" --help
```

如果需要同时获取 README、示例配置和 LICENSE，下载归档：

```bash
set -euo pipefail
REPO="ForgetMelody/remote-ops-mcp"

case "$(uname -m)" in
  x86_64|amd64) TARGET="x86_64-unknown-linux-gnu" ;;
  aarch64|arm64) TARGET="aarch64-unknown-linux-gnu" ;;
  *) echo "unsupported arch: $(uname -m)" >&2; exit 1 ;;
esac

tmp_dir="$(mktemp -d)"
curl -fL "https://github.com/${REPO}/releases/latest/download/remote-ops-mcp-${TARGET}.tar.gz" \
  -o "${tmp_dir}/remote-ops-mcp.tar.gz"
tar -xzf "${tmp_dir}/remote-ops-mcp.tar.gz" -C "${tmp_dir}"
mkdir -p "${HOME}/.local/bin"
install -m 0755 "${tmp_dir}/remote-ops-mcp-${TARGET}/remote-ops-mcp" "${HOME}/.local/bin/remote-ops-mcp"
"${HOME}/.local/bin/remote-ops-mcp" --help
```

如需固定版本，把 `releases/latest/download` 改为 `releases/download/vX.Y.Z`。

## 配置方法

### 1. 创建配置文件

```bash
mkdir -p "${HOME}/.config/remote-ops"
cat > "${HOME}/.config/remote-ops/config.toml" <<'EOF'
[server]
transport = "stdio"

[defaults]
connect_timeout_s = 10
run_timeout_s = 30
initial_wait_s = 1
follow_wait_s = 5
follow_limit = 8192
output_max_bytes = 8388608
host_key_policy = "openssh_default"

[targets.devbox]
host = "devbox.example.com"
port = 22
username = "deploy"
host_key_policy = "accept_new"

# 如需明文密码认证，先安装 sshpass，然后取消下面三行注释。
# [targets.devbox.auth]
# method = "password"
# password = "<password>"
EOF
chmod 600 "${HOME}/.config/remote-ops/config.toml"
```

### 2. 启动 MCP server

```bash
remote-ops-mcp --config "${HOME}/.config/remote-ops/config.toml"
```

也可以用环境变量指定配置：

```bash
REMOTE_OPS_CONFIG="${HOME}/.config/remote-ops/config.toml" remote-ops-mcp
```

### 3. 配置 MCP client

把 `args` 中的路径替换为配置文件绝对路径：

```json
{
  "mcpServers": {
    "remote-ops": {
      "type": "stdio",
      "command": "remote-ops-mcp",
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

### 4. Target 和认证

- 本机执行：省略 `target`，或传 `target = "local"`。
- 临时远端：工具调用的 `target` 可直接传 `[user@]devbox.example.com[:port]`，例如 `deploy@devbox.example.com`，无需写入配置。
- 命名目标：`[targets.<name>]` 适合常用设备，例如 `target = "devbox"`。
- 热加载：每次工具调用都会重读 `--config` 文件，新增或修改 target/auth 后无需重启 MCP server。
- 默认认证：未配置 auth 时使用 `method = "openssh"`，沿用系统 OpenSSH 配置、密钥和 agent。
- 明文密码：在 `[defaults.auth]` 或 `[targets.devbox]` 中配置 `method = "password"` 与 `password = "<password>"`。运行时通过 `sshpass -e` 和 `SSHPASS` 环境变量传递，避免密码出现在进程 argv 或 MCP 返回的 target 结构中。不要把真实配置文件提交到仓库。
- `output_max_bytes` 会在 server 启动时固定为 Job/Session 输出缓冲容量；修改该值需要重启 MCP server。

使用示例：

```json
{"target": "deploy@devbox.example.com", "command": "hostname"}
```

```json
{"target": "devbox", "command": "hostname"}
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

## License

MIT. 详见 `LICENSE`。

## 设计记录

详细方案、命名、架构和落地状态见 `docs/remote-ops-mcp-plan.md`。
