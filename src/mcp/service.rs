use std::process::Stdio;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use tokio::process::Command;

use crate::{
    app::AppState,
    error::{ErrorKind, RemoteOpsError},
    mcp::{
        result::envelope_result,
        tools::{
            BackendHealth, BackendTools, FileCommandResponse, FileCompareRequest, FileSyncRequest,
            FileTextResponse, FollowRequest, JobListResponse, JobRequest, PathRequest, RunRequest,
            RunResponse, SessionCommandRequest, SessionEnsureResponse, SessionExecRequest,
            SessionExecResponse, SessionFollowRequest, SessionFollowResponse, SessionIdRequest,
            SessionListResponse, SessionOpenRequest, SessionOpenResponse, SessionSignalRequest,
            SessionSignalResponse, SessionStartRequest, SessionStartResponse, StartRequest,
            StartResponse, TargetProbe, TargetRequest,
        },
    },
    runner::{
        file_sync::{SyncOptions, compare_path, list_path, stat_path, sync_path},
        process::run_command,
        ssh::build_command,
    },
    target::resolve_target,
};

type McpToolResult = std::result::Result<CallToolResult, McpError>;

#[derive(Clone)]
pub struct RemoteOpsService {
    state: AppState,
    tool_router: rmcp::handler::server::router::tool::ToolRouter<RemoteOpsService>,
}

impl RemoteOpsService {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    async fn command_available(program: &str) -> bool {
        Command::new(program)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok()
    }
}

#[tool_router]
impl RemoteOpsService {
    #[tool(description = "Report local backend capability for ssh, sshpass, rsync, sftp and scp")]
    async fn remote_backend_health(&self) -> McpToolResult {
        let data = BackendHealth {
            server: "remote-ops".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            tools: BackendTools {
                ssh: Self::command_available("ssh").await,
                sshpass: Self::command_available("sshpass").await,
                rsync: Self::command_available("rsync").await,
                sftp: Self::command_available("sftp").await,
                scp: Self::command_available("scp").await,
            },
        };
        envelope_result::<BackendHealth>(Ok(data))
    }

    #[tool(description = "Probe a configured remote target, or local target when omitted")]
    async fn remote_target_probe(
        &self,
        Parameters(request): Parameters<TargetRequest>,
    ) -> McpToolResult {
        let result = async {
            let config = self.state.current_config().await?;
            let target = resolve_target(&config, request.target)?;
            let command = build_command(&target, "printf remote_ops_probe_ok")?;
            let output = run_command(command, target.connect_timeout_s).await?;
            Ok(TargetProbe {
                target,
                reachable: output.exit_code == Some(0)
                    && output.stdout.contains("remote_ops_probe_ok"),
                exit_code: output.exit_code,
                stdout: output.stdout,
                stderr: output.stderr,
            })
        }
        .await;
        envelope_result::<TargetProbe>(result)
    }

    #[tool(description = "Run a command and return its complete output")]
    async fn remote_run(&self, Parameters(request): Parameters<RunRequest>) -> McpToolResult {
        let result = async {
            if request.command.trim().is_empty() {
                return Err(RemoteOpsError::Remote(
                    ErrorKind::ProtocolError,
                    "command must not be empty".to_string(),
                ));
            }
            let config = self.state.current_config().await?;
            let target = resolve_target(&config, request.target.clone())?;
            let timeout_s = request.timeout_s.unwrap_or(config.defaults.run_timeout_s);
            let command = build_command(&target, &request.command)?;
            let result = self
                .state
                .jobs
                .run_sync(command, target.name, request.command, timeout_s)
                .await?;
            Ok(RunResponse { result })
        }
        .await;
        envelope_result::<RunResponse>(result)
    }

    #[tool(description = "Start a long-running command as a job")]
    async fn remote_start(&self, Parameters(request): Parameters<StartRequest>) -> McpToolResult {
        let result = async {
            if request.command.trim().is_empty() {
                return Err(RemoteOpsError::Remote(
                    ErrorKind::ProtocolError,
                    "command must not be empty".to_string(),
                ));
            }
            let config = self.state.current_config().await?;
            let target = resolve_target(&config, request.target.clone())?;
            let command = build_command(&target, &request.command)?;
            let job_id = self
                .state
                .jobs
                .start(command, target.name, request.command)
                .await?;
            let initial = self
                .state
                .jobs
                .follow(
                    &job_id,
                    None,
                    request.follow_limit.unwrap_or(config.defaults.follow_limit),
                    request
                        .initial_wait_s
                        .unwrap_or(config.defaults.initial_wait_s),
                )
                .await?;
            Ok(StartResponse { job_id, initial })
        }
        .await;
        envelope_result::<StartResponse>(result)
    }

    #[tool(description = "Follow job output from an opaque cursor")]
    async fn remote_follow(&self, Parameters(request): Parameters<FollowRequest>) -> McpToolResult {
        let result = async {
            let config = self.state.current_config().await?;
            self.state
                .jobs
                .follow(
                    &request.job_id,
                    request.cursor.as_deref(),
                    request.limit.unwrap_or(config.defaults.follow_limit),
                    request.wait_s.unwrap_or(config.defaults.follow_wait_s),
                )
                .await
        }
        .await;
        envelope_result(result)
    }

    #[tool(description = "Stop a running job")]
    async fn remote_stop(&self, Parameters(request): Parameters<JobRequest>) -> McpToolResult {
        let result = self.state.jobs.stop(&request.job_id).await;
        envelope_result(result)
    }

    #[tool(description = "Get one job status")]
    async fn remote_job_status(
        &self,
        Parameters(request): Parameters<JobRequest>,
    ) -> McpToolResult {
        let result = self.state.jobs.status(&request.job_id).await;
        envelope_result(result)
    }

    #[tool(description = "List all jobs kept by this server process")]
    async fn remote_job_list(&self) -> McpToolResult {
        let jobs = self.state.jobs.list().await;
        envelope_result::<JobListResponse>(Ok(JobListResponse { jobs }))
    }

    #[tool(description = "Always create a new persistent SSH shell session")]
    async fn remote_session_open(
        &self,
        Parameters(request): Parameters<SessionOpenRequest>,
    ) -> McpToolResult {
        let result = async {
            let config = self.state.current_config().await?;
            let target = resolve_target(&config, request.target)?;
            let session = self
                .state
                .sessions
                .open(target, request.session_tag, request.keepalive_s)
                .await?;
            Ok(SessionOpenResponse { session })
        }
        .await;
        envelope_result::<SessionOpenResponse>(result)
    }

    #[tool(description = "Reuse an idle persistent SSH shell session, or create one")]
    async fn remote_session_ensure(
        &self,
        Parameters(request): Parameters<SessionOpenRequest>,
    ) -> McpToolResult {
        let result = async {
            let config = self.state.current_config().await?;
            let target = resolve_target(&config, request.target)?;
            let (session, created) = self
                .state
                .sessions
                .ensure(target, request.session_tag, request.keepalive_s)
                .await?;
            Ok(SessionEnsureResponse { session, created })
        }
        .await;
        envelope_result::<SessionEnsureResponse>(result)
    }

    #[tool(description = "Close one persistent SSH shell session")]
    async fn remote_session_close(
        &self,
        Parameters(request): Parameters<SessionIdRequest>,
    ) -> McpToolResult {
        let result = self.state.sessions.close(&request.session_id).await;
        envelope_result(result)
    }

    #[tool(description = "Get one persistent SSH shell session")]
    async fn remote_session_get(
        &self,
        Parameters(request): Parameters<SessionIdRequest>,
    ) -> McpToolResult {
        let result = self.state.sessions.get(&request.session_id).await;
        envelope_result(result)
    }

    #[tool(description = "List persistent SSH shell sessions kept by this server process")]
    async fn remote_session_list(&self) -> McpToolResult {
        let sessions = self.state.sessions.list().await;
        envelope_result::<SessionListResponse>(Ok(SessionListResponse { sessions }))
    }

    #[tool(description = "Run a command in an existing persistent SSH shell session")]
    async fn remote_session_exec(
        &self,
        Parameters(request): Parameters<SessionExecRequest>,
    ) -> McpToolResult {
        let result = async {
            let result = self
                .state
                .sessions
                .exec(&request.session_id, request.command)
                .await?;
            Ok(SessionExecResponse { result })
        }
        .await;
        envelope_result::<SessionExecResponse>(result)
    }

    #[tool(description = "Start a long-running command in a persistent SSH shell session")]
    async fn remote_session_start(
        &self,
        Parameters(request): Parameters<SessionStartRequest>,
    ) -> McpToolResult {
        let result = async {
            let config = self.state.current_config().await?;
            let command_id = self
                .state
                .sessions
                .start(&request.session_id, request.command)
                .await?;
            let initial = self
                .state
                .sessions
                .follow(
                    &command_id,
                    None,
                    request.follow_limit.unwrap_or(config.defaults.follow_limit),
                    request
                        .initial_wait_s
                        .unwrap_or(config.defaults.initial_wait_s),
                )
                .await?;
            Ok(SessionStartResponse {
                command_id,
                initial,
            })
        }
        .await;
        envelope_result::<SessionStartResponse>(result)
    }

    #[tool(description = "Follow persistent SSH shell session command output")]
    async fn remote_session_follow(
        &self,
        Parameters(request): Parameters<SessionFollowRequest>,
    ) -> McpToolResult {
        let result = async {
            let config = self.state.current_config().await?;
            let result = self
                .state
                .sessions
                .follow(
                    &request.command_id,
                    request.cursor.as_deref(),
                    request.limit.unwrap_or(config.defaults.follow_limit),
                    request.wait_s.unwrap_or(config.defaults.follow_wait_s),
                )
                .await?;
            Ok(SessionFollowResponse { result })
        }
        .await;
        envelope_result::<SessionFollowResponse>(result)
    }

    #[tool(
        description = "Send Ctrl-C, Ctrl-Z, or Ctrl-\\ to a running persistent SSH shell command"
    )]
    async fn remote_session_signal(
        &self,
        Parameters(request): Parameters<SessionSignalRequest>,
    ) -> McpToolResult {
        let result = async {
            let result = self
                .state
                .sessions
                .signal(
                    request.command_id.as_deref(),
                    request.session_id.as_deref(),
                    request.signal.as_deref().unwrap_or("SIGINT"),
                )
                .await?;
            Ok(SessionSignalResponse { result })
        }
        .await;
        envelope_result::<SessionSignalResponse>(result)
    }

    #[tool(description = "Cancel a persistent SSH shell session command and close its session")]
    async fn remote_session_cancel(
        &self,
        Parameters(request): Parameters<SessionCommandRequest>,
    ) -> McpToolResult {
        let result = async {
            let result = self.state.sessions.cancel(&request.command_id).await?;
            Ok(SessionFollowResponse { result })
        }
        .await;
        envelope_result::<SessionFollowResponse>(result)
    }

    #[tool(description = "List one local or remote directory path")]
    async fn remote_file_list(
        &self,
        Parameters(request): Parameters<PathRequest>,
    ) -> McpToolResult {
        let result = async {
            let config = self.state.current_config().await?;
            let target = resolve_target(&config, request.target)?;
            let output = list_path(
                &target,
                &request.path,
                request.timeout_s.unwrap_or(config.defaults.run_timeout_s),
            )
            .await?;
            Ok(FileTextResponse {
                exit_code: output.exit_code,
                stdout: output.stdout,
                stderr: output.stderr,
                timed_out: output.timed_out,
            })
        }
        .await;
        envelope_result::<FileTextResponse>(result)
    }

    #[tool(description = "Stat one local or remote path")]
    async fn remote_file_stat(
        &self,
        Parameters(request): Parameters<PathRequest>,
    ) -> McpToolResult {
        let result = async {
            let config = self.state.current_config().await?;
            let target = resolve_target(&config, request.target)?;
            let output = stat_path(
                &target,
                &request.path,
                request.timeout_s.unwrap_or(config.defaults.run_timeout_s),
            )
            .await?;
            Ok(FileTextResponse {
                exit_code: output.exit_code,
                stdout: output.stdout,
                stderr: output.stderr,
                timed_out: output.timed_out,
            })
        }
        .await;
        envelope_result::<FileTextResponse>(result)
    }

    #[tool(description = "Synchronize files with rsync backend")]
    async fn remote_file_sync(
        &self,
        Parameters(request): Parameters<FileSyncRequest>,
    ) -> McpToolResult {
        let result = async {
            let config = self.state.current_config().await?;
            let target = resolve_target(&config, request.target)?;
            let result = sync_path(
                &target,
                SyncOptions {
                    direction: request.direction,
                    local_path: &request.local_path,
                    remote_path: &request.remote_path,
                    delete: request.delete.unwrap_or(false),
                    checksum: request.checksum.unwrap_or(false),
                    dry_run: request.dry_run.unwrap_or(false),
                    timeout_s: request.timeout_s.unwrap_or(config.defaults.run_timeout_s),
                },
            )
            .await?;
            Ok(FileCommandResponse { result })
        }
        .await;
        envelope_result::<FileCommandResponse>(result)
    }

    #[tool(description = "Compare local and remote paths using rsync dry-run")]
    async fn remote_file_compare(
        &self,
        Parameters(request): Parameters<FileCompareRequest>,
    ) -> McpToolResult {
        let result = async {
            let config = self.state.current_config().await?;
            let target = resolve_target(&config, request.target)?;
            let result = compare_path(
                &target,
                &request.local_path,
                &request.remote_path,
                request.checksum.unwrap_or(false),
                request.timeout_s.unwrap_or(config.defaults.run_timeout_s),
            )
            .await?;
            Ok(FileCommandResponse { result })
        }
        .await;
        envelope_result::<FileCommandResponse>(result)
    }
}

#[tool_handler]
impl ServerHandler for RemoteOpsService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "remote-ops".to_string(),
                title: Some("RemoteOps MCP".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
                description: Some("SSH command jobs, output cursors and rsync-backed file operations".to_string()),
                icons: None,
                website_url: None,
            },
            instructions: Some("RemoteOps MCP provides SSH command jobs, output cursors and rsync-backed file operations.".to_string()),
        }
    }
}
