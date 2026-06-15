use std::{process::Stdio, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time,
};

use crate::{
    error::{ErrorKind, RemoteOpsError, Result},
    job::output::{OutputBuffer, OutputStream},
};

/// 子进程运行结果。
#[derive(Debug, Clone)]
pub struct ProcessOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// 执行本地命令，禁止 shell 拼接；调用方必须逐项传入 argv。
pub async fn run_command(mut command: Command, timeout_s: u64) -> Result<ProcessOutput> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
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
    let stdout_task = tokio::spawn(read_to_string(stdout));
    let stderr_task = tokio::spawn(read_to_string(stderr));

    let status = match time::timeout(Duration::from_secs(timeout_s), child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            let _ = child.kill().await;
            let stdout = join_reader(stdout_task).await?;
            let stderr = join_reader(stderr_task).await?;
            return Ok(ProcessOutput {
                exit_code: None,
                stdout,
                stderr,
                timed_out: true,
            });
        }
    };

    let stdout = join_reader(stdout_task).await?;
    let stderr = join_reader(stderr_task).await?;
    Ok(ProcessOutput {
        exit_code: status.code(),
        stdout,
        stderr,
        timed_out: false,
    })
}

pub async fn read_to_buffer<R>(
    mut reader: R,
    stream: OutputStream,
    buffer: std::sync::Arc<tokio::sync::Mutex<OutputBuffer>>,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut local = vec![0u8; 4096];
    loop {
        let n = reader.read(&mut local).await?;
        if n == 0 {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&local[..n]).into_owned();
        buffer.lock().await.push(stream, text);
    }
}

async fn join_reader(
    task: tokio::task::JoinHandle<std::io::Result<String>>,
) -> std::io::Result<String> {
    match task.await {
        Ok(result) => result,
        Err(err) => Err(std::io::Error::other(err.to_string())),
    }
}

async fn read_to_string<R>(mut reader: R) -> std::io::Result<String>
where
    R: AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
