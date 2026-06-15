use std::{collections::BTreeMap, process::Stdio, sync::Arc, time::Duration};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::{
    process::{Child, Command},
    sync::{Mutex, Notify},
    time,
};
use uuid::Uuid;

use crate::{
    error::{ErrorKind, RemoteOpsError, Result},
    job::{
        cursor::{decode_cursor, encode_cursor},
        output::{OutputBuffer, OutputChunk, OutputStream},
        state::JobState,
    },
    runner::process::{read_to_buffer, run_command},
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobStatus {
    pub job_id: String,
    pub target: Option<String>,
    pub command: String,
    pub state: JobState,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FollowResult {
    pub job_id: String,
    pub state: JobState,
    pub chunks: Vec<OutputChunk>,
    pub cursor: String,
    pub truncated: bool,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
}

struct JobRecord {
    status: Mutex<JobStatus>,
    output: Arc<Mutex<OutputBuffer>>,
    cancel: Notify,
    done: Notify,
}

#[derive(Clone)]
pub struct JobManager {
    jobs: Arc<Mutex<BTreeMap<String, Arc<JobRecord>>>>,
    output_max_bytes: usize,
}

impl JobManager {
    pub fn new(output_max_bytes: usize) -> Self {
        Self {
            jobs: Arc::new(Mutex::new(BTreeMap::new())),
            output_max_bytes,
        }
    }

    pub async fn run_sync(
        &self,
        command: Command,
        _target: Option<String>,
        _command_text: String,
        timeout_s: u64,
    ) -> Result<FollowResult> {
        let job_id = new_job_id();
        let output = run_command(command, timeout_s).await?;
        let state = if output.timed_out {
            JobState::TimedOut
        } else {
            JobState::Exited
        };
        let mut buffer = OutputBuffer::new(self.output_max_bytes);
        buffer.push(OutputStream::Stdout, output.stdout);
        buffer.push(OutputStream::Stderr, output.stderr);
        let (chunks, next_seq, truncated) = buffer.snapshot_from(0, self.output_max_bytes);
        let error = if output.timed_out {
            Some("command timed out".to_string())
        } else {
            None
        };
        Ok(FollowResult {
            job_id,
            state,
            chunks,
            cursor: encode_cursor(next_seq),
            truncated,
            exit_code: output.exit_code,
            error,
        })
    }

    pub async fn start(
        &self,
        mut command: Command,
        target: Option<String>,
        command_text: String,
    ) -> Result<String> {
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().ok_or_else(|| {
            RemoteOpsError::Remote(
                ErrorKind::InternalError,
                "stdout pipe unavailable".to_string(),
            )
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            RemoteOpsError::Remote(
                ErrorKind::InternalError,
                "stderr pipe unavailable".to_string(),
            )
        })?;
        let job_id = new_job_id();
        let output = Arc::new(Mutex::new(OutputBuffer::new(self.output_max_bytes)));
        let record = Arc::new(JobRecord {
            status: Mutex::new(JobStatus {
                job_id: job_id.clone(),
                target,
                command: command_text,
                state: JobState::Running,
                exit_code: None,
                error: None,
            }),
            output: output.clone(),
            cancel: Notify::new(),
            done: Notify::new(),
        });
        self.jobs
            .lock()
            .await
            .insert(job_id.clone(), record.clone());

        tokio::spawn(read_to_buffer(stdout, OutputStream::Stdout, output.clone()));
        tokio::spawn(read_to_buffer(stderr, OutputStream::Stderr, output));
        tokio::spawn(watch_child(record.clone(), child));
        Ok(job_id)
    }

    pub async fn follow(
        &self,
        job_id: &str,
        cursor: Option<&str>,
        limit: usize,
        wait_s: u64,
    ) -> Result<FollowResult> {
        let record = self.get(job_id).await?;
        let after_seq = decode_cursor(cursor)?;
        let deadline = time::Instant::now() + Duration::from_secs(wait_s);
        loop {
            let status = record.status.lock().await.clone();
            let (chunks, next_seq, truncated) =
                record.output.lock().await.snapshot_from(after_seq, limit);
            if !chunks.is_empty() || status.state.is_terminal() || time::Instant::now() >= deadline
            {
                return Ok(FollowResult {
                    job_id: job_id.to_string(),
                    state: status.state,
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

    pub async fn stop(&self, job_id: &str) -> Result<JobStatus> {
        let record = self.get(job_id).await?;
        let done = record.done.notified();
        {
            let mut status = record.status.lock().await;
            if status.state.is_terminal() {
                return Ok(status.clone());
            }
            status.state = JobState::Cancelled;
            status.error = Some("command cancelled".to_string());
        }
        record.cancel.notify_one();
        done.await;
        Ok(record.status.lock().await.clone())
    }

    pub async fn status(&self, job_id: &str) -> Result<JobStatus> {
        Ok(self.get(job_id).await?.status.lock().await.clone())
    }

    pub async fn list(&self) -> Vec<JobStatus> {
        let records: Vec<_> = self.jobs.lock().await.values().cloned().collect();
        let mut statuses = Vec::with_capacity(records.len());
        for record in records {
            statuses.push(record.status.lock().await.clone());
        }
        statuses
    }

    async fn get(&self, job_id: &str) -> Result<Arc<JobRecord>> {
        self.jobs.lock().await.get(job_id).cloned().ok_or_else(|| {
            RemoteOpsError::Remote(
                ErrorKind::ProtocolError,
                format!("unknown job_id '{job_id}'"),
            )
        })
    }
}

async fn watch_child(record: Arc<JobRecord>, mut child: Child) {
    tokio::select! {
        result = child.wait() => update_finished_status(&record, result).await,
        _ = record.cancel.notified() => {
            let _ = child.start_kill();
            let result = child.wait().await;
            update_cancelled_status(&record, result).await;
        }
    }
    record.done.notify_waiters();
}

async fn update_finished_status(
    record: &JobRecord,
    result: std::io::Result<std::process::ExitStatus>,
) {
    let mut status = record.status.lock().await;
    if status.state == JobState::Cancelled {
        return;
    }
    match result {
        Ok(exit) => {
            status.state = JobState::Exited;
            status.exit_code = exit.code();
        }
        Err(err) => {
            status.state = JobState::Failed;
            status.error = Some(err.to_string());
        }
    }
}

async fn update_cancelled_status(
    record: &JobRecord,
    result: std::io::Result<std::process::ExitStatus>,
) {
    let mut status = record.status.lock().await;
    status.state = JobState::Cancelled;
    status.exit_code = result.ok().and_then(|exit| exit.code());
    if status.error.is_none() {
        status.error = Some("command cancelled".to_string());
    }
}

fn new_job_id() -> String {
    Uuid::now_v7().to_string()
}
