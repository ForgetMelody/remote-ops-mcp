use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::{sync::Mutex, time};
use uuid::Uuid;

use crate::{
    error::{ErrorKind, RemoteOpsError, Result},
    job::{
        cursor::{decode_cursor, encode_cursor},
        output::{OutputBuffer, OutputChunk},
    },
    session::shell::{OpenSshShellSession, resolve_control_signal},
    target::ResolvedTarget,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Running,
    Closing,
    Closed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionInfo {
    pub session_id: String,
    pub target: Option<String>,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub session_tag: Option<String>,
    pub state: SessionState,
    pub created_at: String,
    pub last_used_at: String,
    pub current_command_id: Option<String>,
    pub current_command: Option<String>,
    pub current_command_started_at: Option<String>,
    pub last_command_id: Option<String>,
    pub last_command: Option<String>,
    pub last_command_status: Option<SessionCommandStatus>,
    pub last_command_exit_code: Option<i32>,
    pub last_command_ended_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionCommandStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl SessionCommandStatus {
    fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionExecResult {
    pub session_id: String,
    pub command_id: String,
    pub status: SessionCommandStatus,
    pub exit_code: Option<i32>,
    pub chunks: Vec<OutputChunk>,
    pub cursor: String,
    pub truncated: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionFollowResult {
    pub session_id: String,
    pub command_id: String,
    pub status: SessionCommandStatus,
    pub chunks: Vec<OutputChunk>,
    pub cursor: String,
    pub truncated: bool,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionSignalResult {
    pub session_id: String,
    pub command_id: Option<String>,
    pub signal: String,
    pub state: SessionState,
}

#[derive(Clone)]
pub struct SessionManager {
    sessions: Arc<Mutex<BTreeMap<String, Arc<SessionRecord>>>>,
    commands: Arc<Mutex<BTreeMap<String, Arc<SessionCommandRecord>>>>,
    output_max_bytes: usize,
}

struct SessionRecord {
    info: Mutex<SessionInfo>,
    shell: OpenSshShellSession,
}

struct SessionCommandRecord {
    session_id: String,
    command_id: String,
    status: Mutex<CommandRuntimeStatus>,
    output: Arc<StdMutex<OutputBuffer>>,
}

#[derive(Debug, Clone)]
struct CommandRuntimeStatus {
    status: SessionCommandStatus,
    exit_code: Option<i32>,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionKey {
    host: String,
    port: u16,
    username: Option<String>,
    session_tag: Option<String>,
}

impl SessionManager {
    pub fn new(output_max_bytes: usize) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(BTreeMap::new())),
            commands: Arc::new(Mutex::new(BTreeMap::new())),
            output_max_bytes,
        }
    }

    pub async fn open(
        &self,
        target: ResolvedTarget,
        session_tag: Option<String>,
        keepalive_s: Option<u64>,
    ) -> Result<SessionInfo> {
        let key = session_key(&target, session_tag.clone())?;
        let shell = OpenSshShellSession::spawn(&target, keepalive_s).await?;
        let session_id = new_id();
        let now = now_text();
        let info = SessionInfo {
            session_id: session_id.clone(),
            target: target.name.clone(),
            host: key.host,
            port: key.port,
            username: key.username,
            session_tag,
            state: SessionState::Idle,
            created_at: now.clone(),
            last_used_at: now,
            current_command_id: None,
            current_command: None,
            current_command_started_at: None,
            last_command_id: None,
            last_command: None,
            last_command_status: None,
            last_command_exit_code: None,
            last_command_ended_at: None,
        };
        let record = Arc::new(SessionRecord {
            info: Mutex::new(info.clone()),
            shell,
        });
        self.sessions.lock().await.insert(session_id, record);
        Ok(info)
    }

    pub async fn ensure(
        &self,
        target: ResolvedTarget,
        session_tag: Option<String>,
        keepalive_s: Option<u64>,
    ) -> Result<(SessionInfo, bool)> {
        let key = session_key(&target, session_tag.clone())?;
        let candidates: Vec<(String, Arc<SessionRecord>, String)> = {
            let sessions = self.sessions.lock().await;
            let mut values = Vec::new();
            for (session_id, record) in sessions.iter() {
                let info = record.info.lock().await.clone();
                if info.state == SessionState::Idle && info_matches_key(&info, &key) {
                    values.push((session_id.clone(), record.clone(), info.last_used_at));
                }
            }
            values
        };
        let mut candidates = candidates;
        candidates.sort_by(|a, b| b.2.cmp(&a.2));
        for (session_id, record, _) in candidates {
            let alive = record.shell.is_alive().await;
            if alive {
                let mut info = record.info.lock().await;
                info.last_used_at = now_text();
                return Ok((info.clone(), false));
            }
            self.evict(&session_id, SessionState::Failed).await;
        }
        let info = self.open(target, session_tag, keepalive_s).await?;
        Ok((info, true))
    }

    pub async fn close(&self, session_id: &str) -> Result<SessionInfo> {
        let record = self.remove_session(session_id).await?;
        {
            let mut info = record.info.lock().await;
            info.state = SessionState::Closing;
        }
        record.shell.close().await;
        let mut info = record.info.lock().await;
        info.state = SessionState::Closed;
        info.last_used_at = now_text();
        Ok(info.clone())
    }

    pub async fn get(&self, session_id: &str) -> Result<SessionInfo> {
        let record = self.session_record(session_id).await?;
        Ok(record.info.lock().await.clone())
    }

    pub async fn list(&self) -> Vec<SessionInfo> {
        let records: Vec<_> = self.sessions.lock().await.values().cloned().collect();
        let mut result = Vec::with_capacity(records.len());
        for record in records {
            result.push(record.info.lock().await.clone());
        }
        result
    }

    pub async fn exec(&self, session_id: &str, command: String) -> Result<SessionExecResult> {
        if command.trim().is_empty() {
            return Err(RemoteOpsError::Remote(
                ErrorKind::ProtocolError,
                "command must not be empty".to_string(),
            ));
        }
        let record = self.session_record(session_id).await?;
        let command_id = self.mark_session_running(&record, &command).await?;
        let output = Arc::new(StdMutex::new(OutputBuffer::new(self.output_max_bytes)));
        let output_sink = output.clone();
        let result = record
            .shell
            .run_command(&command, move |stream, chunk| {
                let text = String::from_utf8_lossy(chunk).into_owned();
                if !text.is_empty() {
                    output_sink
                        .lock()
                        .expect("output buffer poisoned")
                        .push(stream, text);
                }
            })
            .await;

        let (status, exit_code, error) = match result {
            Ok(exit_code) => {
                let status = command_status_from_exit(exit_code);
                (status, Some(exit_code), None)
            }
            Err(err) => {
                self.evict(session_id, SessionState::Failed).await;
                (SessionCommandStatus::Failed, None, Some(err.to_string()))
            }
        };
        self.mark_session_finished(&record, status, exit_code).await;
        let (chunks, next_seq, truncated) = output
            .lock()
            .expect("output buffer poisoned")
            .snapshot_from(0, self.output_max_bytes);
        Ok(SessionExecResult {
            session_id: session_id.to_string(),
            command_id,
            status,
            exit_code,
            chunks,
            cursor: encode_cursor(next_seq),
            truncated,
            error,
        })
    }

    pub async fn start(&self, session_id: &str, command: String) -> Result<String> {
        if command.trim().is_empty() {
            return Err(RemoteOpsError::Remote(
                ErrorKind::ProtocolError,
                "command must not be empty".to_string(),
            ));
        }
        let record = self.session_record(session_id).await?;
        let command_id = self.mark_session_running(&record, &command).await?;
        let command_record = Arc::new(SessionCommandRecord {
            session_id: session_id.to_string(),
            command_id: command_id.clone(),
            status: Mutex::new(CommandRuntimeStatus {
                status: SessionCommandStatus::Running,
                exit_code: None,
                error: None,
            }),
            output: Arc::new(StdMutex::new(OutputBuffer::new(self.output_max_bytes))),
        });
        self.commands
            .lock()
            .await
            .insert(command_id.clone(), command_record.clone());
        let manager = self.clone();
        let record_for_task = record.clone();
        tokio::spawn(async move {
            manager
                .run_async_command(record_for_task, command_record, command)
                .await;
        });
        Ok(command_id)
    }

    pub async fn follow(
        &self,
        command_id: &str,
        cursor: Option<&str>,
        limit: usize,
        wait_s: u64,
    ) -> Result<SessionFollowResult> {
        let command = self.command_record(command_id).await?;
        let after_seq = decode_cursor(cursor)?;
        let deadline = time::Instant::now() + Duration::from_secs(wait_s);
        loop {
            let status = command.status.lock().await.clone();
            let (chunks, next_seq, truncated) = command
                .output
                .lock()
                .expect("output buffer poisoned")
                .snapshot_from(after_seq, limit);
            if !chunks.is_empty() || status.status.is_terminal() || time::Instant::now() >= deadline
            {
                return Ok(SessionFollowResult {
                    session_id: command.session_id.clone(),
                    command_id: command.command_id.clone(),
                    status: status.status,
                    chunks,
                    cursor: encode_cursor(next_seq),
                    truncated,
                    exit_code: status.exit_code,
                    error: status.error,
                });
            }
            time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn signal(
        &self,
        command_id: Option<&str>,
        session_id: Option<&str>,
        signal: &str,
    ) -> Result<SessionSignalResult> {
        let control = resolve_control_signal(signal)?;
        let (session_id, command_id) = if let Some(command_id) = command_id {
            let command = self.command_record(command_id).await?;
            (command.session_id.clone(), Some(command_id.to_string()))
        } else if let Some(session_id) = session_id {
            (session_id.to_string(), None)
        } else {
            return Err(RemoteOpsError::Remote(
                ErrorKind::ProtocolError,
                "command_id or session_id is required".to_string(),
            ));
        };
        let record = self.session_record(&session_id).await?;
        {
            let info = record.info.lock().await;
            if info.state != SessionState::Running {
                return Err(RemoteOpsError::Remote(
                    ErrorKind::ProtocolError,
                    format!("session is not running: {session_id}"),
                ));
            }
        }
        record.shell.send_control(control).await?;
        let info = record.info.lock().await.clone();
        Ok(SessionSignalResult {
            session_id,
            command_id,
            signal: signal.to_string(),
            state: info.state,
        })
    }

    pub async fn cancel(&self, command_id: &str) -> Result<SessionFollowResult> {
        let command = self.command_record(command_id).await?;
        {
            let mut status = command.status.lock().await;
            status.status = SessionCommandStatus::Cancelled;
            status.error = Some("command cancelled".to_string());
        }
        let _ = self.close(&command.session_id).await;
        self.follow(command_id, None, self.output_max_bytes, 0)
            .await
    }

    async fn run_async_command(
        &self,
        record: Arc<SessionRecord>,
        command_record: Arc<SessionCommandRecord>,
        command: String,
    ) {
        let output = command_record.output.clone();
        let result = record
            .shell
            .run_command(&command, move |stream, chunk| {
                let text = String::from_utf8_lossy(chunk).into_owned();
                if !text.is_empty() {
                    output
                        .lock()
                        .expect("output buffer poisoned")
                        .push(stream, text);
                }
            })
            .await;
        let (status, exit_code, error) = match result {
            Ok(exit_code) => {
                let status = command_status_from_exit(exit_code);
                (status, Some(exit_code), None)
            }
            Err(err) => {
                self.evict(&command_record.session_id, SessionState::Failed)
                    .await;
                (SessionCommandStatus::Failed, None, Some(err.to_string()))
            }
        };
        {
            let mut runtime = command_record.status.lock().await;
            if runtime.status != SessionCommandStatus::Cancelled {
                runtime.status = status;
                runtime.exit_code = exit_code;
                runtime.error = error;
            }
        }
        self.mark_session_finished(&record, status, exit_code).await;
    }

    async fn mark_session_running(
        &self,
        record: &Arc<SessionRecord>,
        command: &str,
    ) -> Result<String> {
        let mut info = record.info.lock().await;
        if info.state != SessionState::Idle {
            return Err(RemoteOpsError::Remote(
                ErrorKind::ProtocolError,
                format!("session is busy: {}", info.session_id),
            ));
        }
        let command_id = new_id();
        info.state = SessionState::Running;
        info.current_command_id = Some(command_id.clone());
        info.current_command = Some(format_command(command));
        info.current_command_started_at = Some(now_text());
        info.last_used_at = now_text();
        Ok(command_id)
    }

    async fn mark_session_finished(
        &self,
        record: &Arc<SessionRecord>,
        status: SessionCommandStatus,
        exit_code: Option<i32>,
    ) {
        let mut info = record.info.lock().await;
        if info.state == SessionState::Closed || info.state == SessionState::Failed {
            return;
        }
        info.state = SessionState::Idle;
        info.last_command_id = info.current_command_id.take();
        info.last_command = info.current_command.take();
        info.last_command_status = Some(status);
        info.last_command_exit_code = exit_code;
        info.last_command_ended_at = Some(now_text());
        info.current_command_started_at = None;
        info.last_used_at = now_text();
    }

    async fn session_record(&self, session_id: &str) -> Result<Arc<SessionRecord>> {
        self.sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| {
                RemoteOpsError::Remote(
                    ErrorKind::ProtocolError,
                    format!("unknown session_id '{session_id}'"),
                )
            })
    }

    async fn command_record(&self, command_id: &str) -> Result<Arc<SessionCommandRecord>> {
        self.commands
            .lock()
            .await
            .get(command_id)
            .cloned()
            .ok_or_else(|| {
                RemoteOpsError::Remote(
                    ErrorKind::ProtocolError,
                    format!("unknown command_id '{command_id}'"),
                )
            })
    }

    async fn remove_session(&self, session_id: &str) -> Result<Arc<SessionRecord>> {
        self.sessions
            .lock()
            .await
            .remove(session_id)
            .ok_or_else(|| {
                RemoteOpsError::Remote(
                    ErrorKind::ProtocolError,
                    format!("unknown session_id '{session_id}'"),
                )
            })
    }

    async fn evict(&self, session_id: &str, state: SessionState) {
        let record = self.sessions.lock().await.remove(session_id);
        if let Some(record) = record {
            {
                let mut info = record.info.lock().await;
                info.state = state;
                info.last_used_at = now_text();
            }
            record.shell.close().await;
        }
    }
}

fn session_key(target: &ResolvedTarget, session_tag: Option<String>) -> Result<SessionKey> {
    let host = target.host.clone().ok_or_else(|| {
        RemoteOpsError::Remote(
            ErrorKind::ProtocolError,
            "session target must be remote".to_string(),
        )
    })?;
    Ok(SessionKey {
        host,
        port: target.port.unwrap_or(22),
        username: target.username.clone(),
        session_tag,
    })
}

fn info_matches_key(info: &SessionInfo, key: &SessionKey) -> bool {
    info.host == key.host
        && info.port == key.port
        && info.username == key.username
        && info.session_tag == key.session_tag
}

fn new_id() -> String {
    Uuid::now_v7().simple().to_string()
}

fn now_text() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{}Z", duration.as_secs(), duration.subsec_millis())
}

fn command_status_from_exit(exit_code: i32) -> SessionCommandStatus {
    match exit_code {
        0 => SessionCommandStatus::Succeeded,
        130 => SessionCommandStatus::Cancelled,
        _ => SessionCommandStatus::Failed,
    }
}

fn format_command(command: &str) -> String {
    const LIMIT: usize = 200;
    let command = command.replace('\n', "\\n");
    if command.len() <= LIMIT {
        command
    } else {
        format!("{}...(truncated)", &command[..LIMIT])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;

    #[test]
    fn session_key_uses_resolved_connection_identity() {
        let target = ResolvedTarget {
            name: Some("devbox".to_string()),
            host: Some("devbox.example.com".to_string()),
            port: Some(22),
            username: Some("deploy".to_string()),
            connect_timeout_s: 10,
            host_key_policy: "accept_new".to_string(),
            auth: AuthConfig::default(),
        };
        let key = session_key(&target, Some("run".to_string())).unwrap();
        assert_eq!(key.host, "devbox.example.com");
        assert_eq!(key.port, 22);
        assert_eq!(key.username.as_deref(), Some("deploy"));
        assert_eq!(key.session_tag.as_deref(), Some("run"));
    }

    #[test]
    fn local_session_target_is_rejected() {
        let target = ResolvedTarget {
            name: None,
            host: None,
            port: None,
            username: None,
            connect_timeout_s: 10,
            host_key_policy: "openssh_default".to_string(),
            auth: AuthConfig::default(),
        };
        assert!(session_key(&target, None).is_err());
    }
}
