use std::{process::Stdio, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin},
    sync::{Mutex, mpsc},
    time,
};
use uuid::Uuid;

use crate::{
    error::{ErrorKind, RemoteOpsError, Result},
    job::output::OutputStream,
    runner::ssh::build_interactive_shell,
    target::ResolvedTarget,
};

/// 单个常驻 OpenSSH 子进程。命令通过远端交互 shell stdin 写入，输出按 marker 截断。
pub struct OpenSshShellSession {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    events: Mutex<mpsc::UnboundedReceiver<ShellEvent>>,
    command_lock: Mutex<()>,
    active_interrupt: Mutex<Option<String>>,
}

enum ShellEvent {
    Data(OutputStream, Vec<u8>),
    Eof,
}

impl OpenSshShellSession {
    pub async fn spawn(target: &ResolvedTarget, keepalive_s: Option<u64>) -> Result<Self> {
        let mut command = build_interactive_shell(target, keepalive_s)?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| {
            RemoteOpsError::Remote(
                ErrorKind::InternalError,
                "session stdin pipe unavailable".to_string(),
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            RemoteOpsError::Remote(
                ErrorKind::InternalError,
                "session stdout pipe unavailable".to_string(),
            )
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            RemoteOpsError::Remote(
                ErrorKind::InternalError,
                "session stderr pipe unavailable".to_string(),
            )
        })?;

        let (tx, events) = mpsc::unbounded_channel();
        tokio::spawn(read_stream(stdout, OutputStream::Stdout, tx.clone()));
        tokio::spawn(read_stream(stderr, OutputStream::Stderr, tx));

        let session = Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            events: Mutex::new(events),
            command_lock: Mutex::new(()),
            active_interrupt: Mutex::new(None),
        };
        session.initialize().await?;
        Ok(session)
    }

    pub async fn run_command<F>(&self, command: &str, mut on_chunk: F) -> Result<i32>
    where
        F: FnMut(OutputStream, &[u8]),
    {
        if command.trim().is_empty() {
            return Err(RemoteOpsError::Remote(
                ErrorKind::ProtocolError,
                "command must not be empty".to_string(),
            ));
        }
        let _command_guard = self.command_lock.lock().await;
        self.ensure_alive().await?;
        self.drain_events().await;

        let start_marker = new_marker("START");
        let exit_marker = new_marker("RC");
        let payload = build_command_payload(command, &start_marker, &exit_marker);
        {
            let mut active_interrupt = self.active_interrupt.lock().await;
            *active_interrupt = Some(exit_marker.clone());
        }
        let result = async {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(payload.as_bytes()).await?;
            stdin.flush().await?;
            drop(stdin);
            self.read_command_output(
                start_marker.as_bytes(),
                exit_marker.as_bytes(),
                &mut on_chunk,
            )
            .await
        }
        .await;
        {
            let mut active_interrupt = self.active_interrupt.lock().await;
            *active_interrupt = None;
        }
        result
    }

    pub async fn send_control(&self, signal: ControlSignal) -> Result<()> {
        self.ensure_alive().await?;
        let marker = self.active_interrupt.lock().await.clone();
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&[signal.byte()]).await?;
        if let Some(marker) = marker {
            let command = build_exit_marker_command(&marker, "130");
            stdin.write_all(b"\n").await?;
            stdin.write_all(command.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
        }
        stdin.flush().await?;
        Ok(())
    }

    pub async fn is_alive(&self) -> bool {
        let mut child = self.child.lock().await;
        matches!(child.try_wait(), Ok(None))
    }

    pub async fn close(&self) {
        {
            let mut stdin = self.stdin.lock().await;
            let _ = stdin.write_all(b"exit\n").await;
            let _ = stdin.flush().await;
        }
        if time::timeout(Duration::from_millis(500), async {
            let mut child = self.child.lock().await;
            child.wait().await
        })
        .await
        .is_ok()
        {
            return;
        }
        let mut child = self.child.lock().await;
        let _ = child.start_kill();
        let _ = child.wait().await;
    }

    async fn initialize(&self) -> Result<()> {
        let marker = new_marker("READY");
        let payload = format!(
            "stty -echo 2>/dev/null || true\n{}\n",
            build_plain_marker_command(&marker)
        );
        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(payload.as_bytes()).await?;
            stdin.flush().await?;
        }
        self.wait_for_plain_marker(marker.as_bytes()).await?;
        self.drain_events().await;
        Ok(())
    }

    async fn ensure_alive(&self) -> Result<()> {
        let mut child = self.child.lock().await;
        match child.try_wait() {
            Ok(None) => Ok(()),
            Ok(Some(status)) => Err(RemoteOpsError::Remote(
                ErrorKind::SessionLost,
                format!("ssh session exited: {status}"),
            )),
            Err(err) => Err(RemoteOpsError::Io(err)),
        }
    }

    async fn drain_events(&self) {
        let mut events = self.events.lock().await;
        while events.try_recv().is_ok() {}
    }

    async fn recv_event(&self) -> Option<ShellEvent> {
        let mut events = self.events.lock().await;
        events.recv().await
    }

    async fn wait_for_plain_marker(&self, marker: &[u8]) -> Result<()> {
        let mut pending = Vec::new();
        loop {
            match time::timeout(Duration::from_millis(100), self.recv_event()).await {
                Ok(Some(ShellEvent::Data(OutputStream::Stdout, chunk))) => {
                    pending.extend_from_slice(&chunk);
                    if pending.windows(marker.len()).any(|window| window == marker) {
                        return Ok(());
                    }
                    trim_pending(&mut pending, marker.len() + 64);
                }
                Ok(Some(ShellEvent::Data(_, _))) => {}
                Ok(Some(ShellEvent::Eof)) | Ok(None) => {
                    return Err(RemoteOpsError::Remote(
                        ErrorKind::SessionLost,
                        "ssh session closed before ready marker".to_string(),
                    ));
                }
                Err(_) => self.ensure_alive().await?,
            }
        }
    }

    async fn read_command_output<F>(
        &self,
        start_marker: &[u8],
        exit_marker: &[u8],
        on_chunk: &mut F,
    ) -> Result<i32>
    where
        F: FnMut(OutputStream, &[u8]),
    {
        let mut pending = Vec::new();
        let mut phase = MarkerPhase::Start;
        let mut digits = Vec::new();
        let tail_limit = start_marker.len().max(exit_marker.len()) + 64;

        loop {
            match time::timeout(Duration::from_millis(100), self.recv_event()).await {
                Ok(Some(ShellEvent::Data(OutputStream::Stdout, chunk))) => {
                    pending.extend_from_slice(&chunk);
                    if let Some(exit_code) = process_stdout_pending(
                        &mut pending,
                        start_marker,
                        exit_marker,
                        &mut phase,
                        &mut digits,
                        tail_limit,
                        on_chunk,
                    )? {
                        return Ok(exit_code);
                    }
                }
                Ok(Some(ShellEvent::Data(stream, chunk))) => {
                    if phase != MarkerPhase::Start {
                        on_chunk(stream, &chunk);
                    }
                }
                Ok(Some(ShellEvent::Eof)) | Ok(None) => {
                    return Err(RemoteOpsError::Remote(
                        ErrorKind::SessionLost,
                        "ssh session closed before command marker".to_string(),
                    ));
                }
                Err(_) => self.ensure_alive().await?,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlSignal {
    Sigint,
    Sigtstp,
    Sigquit,
}

impl ControlSignal {
    fn byte(self) -> u8 {
        match self {
            Self::Sigint => 0x03,
            Self::Sigtstp => 0x1a,
            Self::Sigquit => 0x1c,
        }
    }
}

pub fn resolve_control_signal(signal: &str) -> Result<ControlSignal> {
    let normalized = signal.trim().to_ascii_uppercase().replace([' ', '-'], "_");
    match normalized.as_str() {
        "SIGINT" | "INT" | "CTRL_C" | "CTRLC" => Ok(ControlSignal::Sigint),
        "SIGTSTP" | "TSTP" | "CTRL_Z" | "CTRLZ" => Ok(ControlSignal::Sigtstp),
        "SIGQUIT" | "QUIT" | "CTRL_\\" | "CTRL\\" => Ok(ControlSignal::Sigquit),
        _ => Err(RemoteOpsError::Remote(
            ErrorKind::ProtocolError,
            format!("unsupported signal '{signal}'"),
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerPhase {
    Start,
    ExitMarker,
    ExitCode,
}

async fn read_stream<R>(mut reader: R, stream: OutputStream, tx: mpsc::UnboundedSender<ShellEvent>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut buffer = [0u8; 4096];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => {
                let _ = tx.send(ShellEvent::Eof);
                return;
            }
            Ok(n) => {
                if tx
                    .send(ShellEvent::Data(stream, buffer[..n].to_vec()))
                    .is_err()
                {
                    return;
                }
            }
            Err(_) => {
                let _ = tx.send(ShellEvent::Eof);
                return;
            }
        }
    }
}

fn process_stdout_pending<F>(
    pending: &mut Vec<u8>,
    start_marker: &[u8],
    exit_marker: &[u8],
    phase: &mut MarkerPhase,
    digits: &mut Vec<u8>,
    tail_limit: usize,
    on_chunk: &mut F,
) -> Result<Option<i32>>
where
    F: FnMut(OutputStream, &[u8]),
{
    if *phase == MarkerPhase::Start {
        if let Some(index) = find_bytes(pending, start_marker) {
            pending.drain(..index + start_marker.len());
            *phase = MarkerPhase::ExitMarker;
        } else {
            trim_pending(pending, tail_limit);
            return Ok(None);
        }
    }

    if *phase == MarkerPhase::ExitMarker {
        if let Some(index) = find_bytes(pending, exit_marker) {
            if index > 0 {
                on_chunk(OutputStream::Stdout, &pending[..index]);
            }
            pending.drain(..index + exit_marker.len());
            *phase = MarkerPhase::ExitCode;
        } else {
            if pending.len() > tail_limit {
                let flush_len = pending.len() - tail_limit;
                on_chunk(OutputStream::Stdout, &pending[..flush_len]);
                pending.drain(..flush_len);
            }
            return Ok(None);
        }
    }

    if *phase == MarkerPhase::ExitCode {
        let mut consumed = 0usize;
        while consumed < pending.len() {
            let byte = pending[consumed];
            if byte.is_ascii_digit() {
                digits.push(byte);
            } else if !digits.is_empty() {
                pending.drain(..=consumed);
                let text = String::from_utf8_lossy(digits);
                let exit_code = text.parse::<i32>().map_err(|err| {
                    RemoteOpsError::Remote(
                        ErrorKind::ProtocolError,
                        format!("invalid session command exit code '{text}': {err}"),
                    )
                })?;
                return Ok(Some(exit_code));
            } else if byte != b'\r' && byte != b'\n' {
                return Err(RemoteOpsError::Remote(
                    ErrorKind::ProtocolError,
                    "session marker was not followed by an exit code".to_string(),
                ));
            }
            consumed += 1;
        }
        pending.clear();
    }

    Ok(None)
}

fn build_command_payload(command: &str, start_marker: &str, exit_marker: &str) -> String {
    let function_name = format!("__remote_ops_run_{}", Uuid::now_v7().simple());
    let mut payload = String::new();
    payload.push_str("stty -echo 2>/dev/null || true\n");
    payload.push_str(&function_name);
    payload.push_str("() {\n");
    payload.push_str(command);
    if !command.ends_with('\n') {
        payload.push('\n');
    }
    payload.push_str("}\n");
    payload.push_str(&build_plain_marker_command(start_marker));
    payload.push_str("; trap '__REMOTE_OPS_RC=130; ");
    payload.push_str(
        &build_exit_marker_command(exit_marker, "$__REMOTE_OPS_RC").replace('\'', "'\\''"),
    );
    payload.push_str("; return 130' INT TERM QUIT; ");
    payload.push_str(&function_name);
    payload.push_str("; __REMOTE_OPS_RC=$?; trap - INT TERM QUIT; unset -f ");
    payload.push_str(&function_name);
    payload.push_str(" 2>/dev/null || true; ");
    payload.push_str(&build_exit_marker_command(exit_marker, "$__REMOTE_OPS_RC"));
    payload.push('\n');
    payload
}

fn build_plain_marker_command(marker: &str) -> String {
    let (head, tail) = split_marker(marker);
    format!(
        "__REMOTE_OPS_A={}; __REMOTE_OPS_B={}; printf '\\n%s%s\\n' \"$__REMOTE_OPS_A\" \"$__REMOTE_OPS_B\"",
        shell_quote(head),
        shell_quote(tail)
    )
}

fn build_exit_marker_command(marker: &str, status_expr: &str) -> String {
    let (head, tail) = split_marker(marker);
    format!(
        "__REMOTE_OPS_A={}; __REMOTE_OPS_B={}; printf '\\n%s%s%s\\n' \"$__REMOTE_OPS_A\" \"$__REMOTE_OPS_B\" \"{}\"",
        shell_quote(head),
        shell_quote(tail),
        status_expr
    )
}

fn split_marker(marker: &str) -> (&str, &str) {
    let midpoint = marker.len() / 2;
    marker.split_at(midpoint)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn new_marker(kind: &str) -> String {
    format!("__REMOTE_OPS_{kind}_{}__", Uuid::now_v7().simple())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn trim_pending(pending: &mut Vec<u8>, tail_limit: usize) {
    if pending.len() > tail_limit {
        let trim = pending.len() - tail_limit;
        pending.drain(..trim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_command_does_not_echo_full_marker() {
        let marker = "__REMOTE_OPS_RC_abcdef__";
        let command = build_exit_marker_command(marker, "$__REMOTE_OPS_RC");
        assert!(!command.contains(marker));
    }

    #[test]
    fn parses_marker_split_across_chunks() {
        let start = b"__REMOTE_OPS_START_x__";
        let exit_marker = b"__REMOTE_OPS_RC_x__";
        let mut pending = Vec::new();
        let mut phase = MarkerPhase::Start;
        let mut digits = Vec::new();
        let mut output = Vec::new();

        pending.extend_from_slice(b"prompt noise __REMOTE_OPS_START_x__hello __REMOTE");
        assert!(
            process_stdout_pending(
                &mut pending,
                start,
                exit_marker,
                &mut phase,
                &mut digits,
                exit_marker.len() + 4,
                &mut |_, chunk| output.extend_from_slice(chunk),
            )
            .unwrap()
            .is_none()
        );
        pending.extend_from_slice(b"_OPS_RC_x__7\n");
        let exit = process_stdout_pending(
            &mut pending,
            start,
            exit_marker,
            &mut phase,
            &mut digits,
            exit_marker.len() + 4,
            &mut |_, chunk| output.extend_from_slice(chunk),
        )
        .unwrap();

        assert_eq!(exit, Some(7));
        assert_eq!(String::from_utf8_lossy(&output), "hello ");
    }

    #[test]
    fn resolves_signal_aliases() {
        assert_eq!(
            resolve_control_signal("ctrl-c").unwrap(),
            ControlSignal::Sigint
        );
        assert_eq!(
            resolve_control_signal("TSTP").unwrap(),
            ControlSignal::Sigtstp
        );
        assert_eq!(
            resolve_control_signal("CTRL_\\").unwrap(),
            ControlSignal::Sigquit
        );
    }
}
