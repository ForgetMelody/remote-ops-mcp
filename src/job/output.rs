use std::collections::VecDeque;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 输出流类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
    System,
}

/// 单条输出块。`seq` 是单调递增序号，不暴露 byte offset。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutputChunk {
    pub seq: u64,
    pub stream: OutputStream,
    pub text: String,
}

/// 每个 Job 的环形输出缓冲。
#[derive(Debug)]
pub struct OutputBuffer {
    max_bytes: usize,
    next_seq: u64,
    retained_bytes: usize,
    chunks: VecDeque<OutputChunk>,
}

impl OutputBuffer {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            next_seq: 1,
            retained_bytes: 0,
            chunks: VecDeque::new(),
        }
    }

    pub fn push(&mut self, stream: OutputStream, text: impl Into<String>) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        let chunk = OutputChunk {
            seq: self.next_seq,
            stream,
            text,
        };
        self.next_seq += 1;
        self.retained_bytes += chunk.text.len();
        self.chunks.push_back(chunk);
        self.trim_to_limit();
    }

    pub fn snapshot_from(&self, after_seq: u64, limit: usize) -> (Vec<OutputChunk>, u64, bool) {
        let mut total = 0usize;
        let mut chunks = Vec::new();
        let mut truncated = false;
        for chunk in self.chunks.iter().filter(|chunk| chunk.seq > after_seq) {
            let len = chunk.text.len();
            if !chunks.is_empty() && total + len > limit {
                truncated = true;
                break;
            }
            if chunks.is_empty() && len > limit {
                let mut clipped = chunk.clone();
                clipped.text.truncate(limit);
                chunks.push(clipped);
                truncated = true;
                break;
            }
            total += len;
            chunks.push(chunk.clone());
        }
        let next_seq = chunks.last().map_or(after_seq, |chunk| chunk.seq);
        (chunks, next_seq, truncated)
    }

    #[cfg(test)]
    pub fn all_text(&self) -> String {
        let mut out = String::new();
        for chunk in &self.chunks {
            out.push_str(&chunk.text);
        }
        out
    }

    fn trim_to_limit(&mut self) {
        while self.retained_bytes > self.max_bytes {
            let Some(front) = self.chunks.pop_front() else {
                self.retained_bytes = 0;
                break;
            };
            self.retained_bytes = self.retained_bytes.saturating_sub(front.text.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_chunks_after_cursor() {
        let mut buffer = OutputBuffer::new(1024);
        buffer.push(OutputStream::Stdout, "a");
        buffer.push(OutputStream::Stderr, "b");
        let (chunks, cursor, truncated) = buffer.snapshot_from(1, 1024);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "b");
        assert_eq!(cursor, 2);
        assert!(!truncated);
    }

    #[test]
    fn trims_old_chunks() {
        let mut buffer = OutputBuffer::new(3);
        buffer.push(OutputStream::Stdout, "abc");
        buffer.push(OutputStream::Stdout, "de");
        assert_eq!(buffer.all_text(), "de");
    }
}
