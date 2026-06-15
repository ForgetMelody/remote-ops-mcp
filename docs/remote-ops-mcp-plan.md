# RemoteOps MCP 方案记录

## 定位

RemoteOps MCP 是一个通用的远程 SSH 命令管理、文件传输、操作调试 MCP 工具。

- 不绑定 Viobot。
- 不绑定 ROS 或特定设备。
- 不做堡垒机。
- 不重造 SSH、rsync、sftp、scp 协议。
- 核心抽象从 `session + command + offset` 改为 `target + job + cursor + transcript`。

## 命名

推荐并采用：

| 项 | 名称 |
|---|---|
| Product | RemoteOps MCP |
| MCP server name | `remote-ops` |
| Rust crate | `remote-ops-mcp` |
| Binary | `remote-ops-mcp` |
| Tool prefix | `remote_*` |
| Config dir | `~/.config/remote-ops/` |
| Env prefix | `REMOTE_OPS_` |

## 技术选型

采用全 Rust 单进程 MCP server。

```text
MCP client
  -> stdio
  -> remote-ops-mcp
      -> system ssh / rsync / sftp / scp
      -> remote host
```

第一版不拆 daemon。后续如果需要多个 MCP client 共享 job/session 状态，再扩展为常驻 daemon 或 Streamable HTTP server。

MCP SDK：

- SDK：`rmcp`
- 官方 Rust SDK：<https://github.com/modelcontextprotocol/rust-sdk>
- 文档：<https://rust.sdk.modelcontextprotocol.io/>

## 架构

```text
remote-ops-mcp
├── MCP Layer
│   ├── rmcp server
│   ├── tool schema
│   ├── request validation
│   └── MCP result/error mapping
│
├── Domain Layer
│   ├── TargetRegistry
│   ├── AuthProvider
│   ├── JobManager
│   ├── OutputStore
│   ├── SessionPool
│   └── ErrorMapper
│
├── Runner Layer
│   ├── SshRunner
│   ├── PtyRunner
│   ├── FileSyncRunner
│   └── ProcessRunner
│
└── Infra Layer
    ├── config loader
    ├── tracing
    ├── secret redaction
    ├── timeout/cancel
    └── platform capability probe
```

## 模块规划

```text
src/
├── main.rs
├── app.rs
├── mcp/
│   ├── mod.rs
│   ├── service.rs
│   ├── tools.rs
│   └── result.rs
├── target/
│   ├── mod.rs
│   └── registry.rs
├── job/
│   ├── mod.rs
│   ├── manager.rs
│   ├── state.rs
│   ├── output.rs
│   └── cursor.rs
├── runner/
│   ├── mod.rs
│   ├── process.rs
│   ├── ssh.rs
│   └── file_sync.rs
├── error/
│   ├── mod.rs
│   ├── kind.rs
│   └── redact.rs
├── config/
│   ├── mod.rs
│   └── model.rs
└── tests/
    └── mcp_smoke.rs
```

## MCP 工具

### 命令管理

- `remote_backend_health`
- `remote_target_probe`
- `remote_run`
- `remote_start`
- `remote_follow`
- `remote_stop`
- `remote_job_status`
- `remote_job_list`

### 文件传输

- `remote_file_list`
- `remote_file_stat`
- `remote_file_sync`
- `remote_file_compare`

## 默认参数

| Parameter | Scope | Default | Meaning | Rationale |
|---|---|---:|---|---|
| `transport` | server | `stdio` | MCP 传输方式 | MCP client 默认最常用 |
| `command_mode` | command | `exec` | SSH 执行模式 | stdout/stderr 和 exit code 更清晰 |
| `timeout_s` | `remote_run` | `30` | 同步等待秒数 | 避免诊断命令挂死 |
| `initial_wait_s` | `remote_start` | `1` | 启动后首批输出等待 | 快速暴露启动失败 |
| `follow_wait_s` | `remote_follow` | `5` | 无输出时长轮询等待 | 兼顾实时性和调用次数 |
| `follow_limit` | `remote_follow` | `8192` | 单次输出字节上限 | 适合日志阅读 |
| `output_max_bytes` | job | `8388608` | 每 job 输出环形缓冲 | 防止长日志吃满内存 |
| `connect_timeout_s` | target | `10` | SSH 连接超时 | 网络故障快速失败 |
| `file_backend` | file | `rsync` | 文件后端 | 第一版只启用可验证的成熟后端 |
| `delete` | file sync | `false` | 是否删除目标多余文件 | 默认避免破坏远端数据 |
| `checksum` | file sync | `false` | 是否强校验 | 默认更快 |
| `host_key_policy` | target | `openssh_default` | host key 策略 | 尊重用户 OpenSSH 配置 |

## 错误模型

统一错误类型：

```text
AuthFailed
ConnectFailed
HostKeyFailed
RemoteNonZeroExit
Timeout
CommandCancelled
SessionLost
OutputLimitExceeded
FileNotFound
PermissionDenied
ToolUnavailable
ProtocolError
InternalError
```

所有工具返回结构化结果，同时提供面向人的 transcript。

## 文件传输策略

优先级：

```text
rsync -> sftp -> scp
```

原则：

- Rust 只负责编排和错误归一化。
- 本地命令使用 `tokio::process::Command` 和逐项 `.arg()`。
- 不拼 shell 字符串。
- 默认不启用破坏性 `delete`。

## 安全边界

必须保证：

- secret 不进入日志和 transcript。
- 不默认使用 `sshpass -p`。
- 密码支持后续通过 `auth_ref`、环境变量或临时 askpass helper 实现。
- 默认尊重 OpenSSH host key 校验。
- 命令后端不通过 shell 拼接执行。

## 落地路线

1. 项目骨架和 MCP 连通。
2. 命令 Job MVP。
3. 文件传输 MVP。
4. 配置、安全、脱敏。
5. 会话池和 OpenSSH ControlMaster。
6. 发布和回归验证。

## 当前施工范围

本次先落地：

- Rust crate 骨架。
- 方案文档。
- MCP stdio server。
- `remote_backend_health`。
- `remote_target_probe`。
- `remote_run`。
- `remote_start` / `remote_follow` / `remote_stop` MVP。
- `remote_file_list` / `remote_file_stat` / `remote_file_sync` / `remote_file_compare` MVP。
- 基础单元测试。

## 当前落地状态

- 项目目录：`/path/to/remote-ops-mcp`。
- Git：本地仓库已初始化。
- 已采用 `rmcp` stdio server。
- 命令 MVP 支持本机执行和 OpenSSH 远程执行。
- Job MVP 支持 start、follow、stop、status、list。
- 文件 MVP 使用系统 `rsync`，本机/远端路径统一由 target 决定。

## 验证命令

```bash
cargo fmt --all -- --check
cargo test --all
cargo clippy --all-targets --all-features -- -D warnings
```

MCP 连通验证：

```bash
npx @modelcontextprotocol/inspector cargo run -- --config config.example.toml
```
