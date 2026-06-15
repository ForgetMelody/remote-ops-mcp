# RemoteOps MCP

RemoteOps MCP 是一个通用远程操作 MCP server，面向 SSH 命令管理、长任务输出跟踪和基于 `rsync` 的文件同步。

## 当前能力

- MCP transport：stdio。
- 命令执行：本机 `local` 和 OpenSSH 远程 target。
- Job 模型：`remote_start`、`remote_follow`、`remote_stop`、`remote_job_status`、`remote_job_list`。
- 文件操作：`remote_file_list`、`remote_file_stat`、`remote_file_sync`、`remote_file_compare`。
- 文件同步后端：第一版只启用系统 `rsync`。
- 配置：TOML，示例见 `config.example.toml`。

## 构建与验证

```bash
cargo fmt --all -- --check
cargo test --all
cargo clippy --all-targets --all-features -- -D warnings
```

## 运行

```bash
cargo run -- --config config.example.toml
```

MCP client 配置示例：

```json
{
  "mcpServers": {
    "remote-ops": {
      "command": "/path/to/remote-ops-mcp",
      "args": ["--config", "/path/to/remote-ops/config.toml"],
      "env": {
        "RUST_LOG": "remote_ops_mcp=info"
      }
    }
  }
}
```

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

### 文件操作

- `remote_file_list`
- `remote_file_stat`
- `remote_file_sync`
- `remote_file_compare`

## 设计记录

详细方案、命名、架构和落地状态见 `docs/remote-ops-mcp-plan.md`。
