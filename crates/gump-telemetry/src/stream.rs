//! Binary-safe stdout/stderr capture (DELIVERY T02 / RUNTIME.md §14).
//!
//! Topics: `app/stdout` and `app/stderr`. Pipe reads are capped at 32 KiB;
//! records are ≤64 KiB. Longer lines and binary streams are split with
//! BEGIN/CONTINUE/END flags. Emitters must not block the drain.

use core::fmt;
use std::collections::VecDeque;

use crate::topic::validate_topic;

/// Normative captured-stream topics (RUNTIME.md §14).
pub const TOPIC_STDOUT: &str = "app/stdout";
pub const TOPIC_STDERR: &str = "app/stderr";

/// Maximum pipe read / preferred binary chunk size.
pub const MAX_READ_CHUNK: usize = 32 * 1024;

/// Maximum single telemetry record payload (D011).
pub const MAX_STREAM_RECORD_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum StreamKind {
    Stdout,
    Stderr,
}

impl StreamKind {
    pub const fn topic(self) -> &'static str {
        match self {
            Self::Stdout => TOPIC_STDOUT,
            Self::Stderr => TOPIC_STDERR,
        }
    }
}

/// Chunk framing flags for reconstruction within received chunks.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ChunkFlags(u8);

impl ChunkFlags {
    pub const BEGIN: Self = Self(0b001);
    pub const CONTINUE: Self = Self(0b010);
    pub const END: Self = Self(0b100);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl std::ops::BitOr for ChunkFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamRecord {
    pub topic: &'static str,
    pub stream_sequence: u64,
    pub flags: ChunkFlags,
    pub bytes: Vec<u8>,
    /// True when `bytes` is valid UTF-8 (hint only; binary remains allowed).
    pub utf8_hint: bool,
    /// Monotonic byte offset in the stream at the start of this record.
    pub receive_offset: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum EmitOutcome {
    Accepted,
    DroppedOldest,
}

/// Sink for captured stream records. Must not block the drain.
pub trait StreamEmitter {
    fn emit(&mut self, record: StreamRecord) -> EmitOutcome;
}

/// Bounded in-memory emitter for saturation tests and local buffering.
#[derive(Clone, Debug)]
pub struct BoundedRecordQueue {
    pub max_records: usize,
    pub records: VecDeque<StreamRecord>,
    pub accepted: u64,
    pub dropped_oldest: u64,
}

impl BoundedRecordQueue {
    pub fn new(max_records: usize) -> Self {
        Self {
            max_records: max_records.max(1),
            records: VecDeque::new(),
            accepted: 0,
            dropped_oldest: 0,
        }
    }
}

impl StreamEmitter for BoundedRecordQueue {
    fn emit(&mut self, record: StreamRecord) -> EmitOutcome {
        if self.records.len() >= self.max_records {
            let _ = self.records.pop_front();
            self.dropped_oldest += 1;
            self.records.push_back(record);
            self.accepted += 1;
            return EmitOutcome::DroppedOldest;
        }
        self.records.push_back(record);
        self.accepted += 1;
        EmitOutcome::Accepted
    }
}

/// Incremental stdout/stderr drain state machine.
#[derive(Debug)]
pub struct StreamDrain {
    kind: StreamKind,
    stream_sequence: u64,
    receive_offset: u64,
    pending: Vec<u8>,
    in_split: bool,
}

impl StreamDrain {
    pub fn new(kind: StreamKind) -> Result<Self, StreamCaptureError> {
        validate_topic(kind.topic()).map_err(|e| {
            StreamCaptureError::new(StreamCaptureErrorKind::Topic, e.to_string())
        })?;
        Ok(Self {
            kind,
            stream_sequence: 0,
            receive_offset: 0,
            pending: Vec::new(),
            in_split: false,
        })
    }

    pub fn kind(&self) -> StreamKind {
        self.kind
    }

    pub fn receive_offset(&self) -> u64 {
        self.receive_offset
    }

    /// Ingest bytes from a pipe read (callers should pass ≤ [`MAX_READ_CHUNK`]).
    pub fn push<E: StreamEmitter>(&mut self, data: &[u8], emitter: &mut E) {
        let data = if data.len() > MAX_READ_CHUNK {
            &data[..MAX_READ_CHUNK]
        } else {
            data
        };
        if data.is_empty() {
            return;
        }

        let mut i = 0;
        while i < data.len() {
            if let Some(rel) = data[i..].iter().position(|&b| b == b'\n') {
                let end = i + rel + 1;
                self.pending.extend_from_slice(&data[i..end]);
                self.flush_complete_line(emitter);
                i = end;
            } else {
                self.pending.extend_from_slice(&data[i..]);
                self.spill_oversized(emitter);
                break;
            }
        }
    }

    /// EOF: flush any remainder as a final record.
    pub fn finish<E: StreamEmitter>(&mut self, emitter: &mut E) {
        if self.pending.is_empty() {
            self.in_split = false;
            return;
        }
        self.spill_oversized(emitter);
        if self.pending.is_empty() {
            self.in_split = false;
            return;
        }
        let flags = if self.in_split {
            ChunkFlags::CONTINUE | ChunkFlags::END
        } else {
            ChunkFlags::BEGIN | ChunkFlags::END
        };
        let n = self.pending.len();
        self.emit_chunk(emitter, n, flags);
        self.in_split = false;
    }

    fn flush_complete_line<E: StreamEmitter>(&mut self, emitter: &mut E) {
        if self.pending.len() <= MAX_STREAM_RECORD_BYTES && !self.in_split {
            let n = self.pending.len();
            self.emit_chunk(emitter, n, ChunkFlags::BEGIN | ChunkFlags::END);
            return;
        }
        // Oversized line spanning the newline: chunk, ending with END.
        if !self.in_split {
            let take = MAX_STREAM_RECORD_BYTES.min(self.pending.len());
            let more = self.pending.len() > take;
            let flags = if more {
                ChunkFlags::BEGIN | ChunkFlags::CONTINUE
            } else {
                ChunkFlags::BEGIN | ChunkFlags::END
            };
            self.emit_chunk(emitter, take, flags);
            self.in_split = more;
        }
        while self.pending.len() > MAX_STREAM_RECORD_BYTES {
            self.emit_chunk(emitter, MAX_STREAM_RECORD_BYTES, ChunkFlags::CONTINUE);
        }
        if !self.pending.is_empty() {
            let n = self.pending.len();
            self.emit_chunk(emitter, n, ChunkFlags::CONTINUE | ChunkFlags::END);
            self.in_split = false;
        }
    }

    fn spill_oversized<E: StreamEmitter>(&mut self, emitter: &mut E) {
        while self.pending.len() >= MAX_STREAM_RECORD_BYTES {
            let flags = if self.in_split {
                ChunkFlags::CONTINUE
            } else {
                ChunkFlags::BEGIN | ChunkFlags::CONTINUE
            };
            self.emit_chunk(emitter, MAX_STREAM_RECORD_BYTES, flags);
            self.in_split = true;
        }
    }

    fn emit_chunk<E: StreamEmitter>(&mut self, emitter: &mut E, nbytes: usize, flags: ChunkFlags) {
        debug_assert!(nbytes > 0 && nbytes <= self.pending.len());
        let bytes: Vec<u8> = self.pending.drain(..nbytes).collect();
        let utf8_hint = std::str::from_utf8(&bytes).is_ok();
        let record = StreamRecord {
            topic: self.kind.topic(),
            stream_sequence: self.stream_sequence,
            flags,
            bytes,
            utf8_hint,
            receive_offset: self.receive_offset,
        };
        self.receive_offset = self.receive_offset.saturating_add(nbytes as u64);
        self.stream_sequence = self.stream_sequence.saturating_add(1);
        let _ = emitter.emit(record);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum StreamCaptureErrorKind {
    Topic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamCaptureError {
    kind: StreamCaptureErrorKind,
    message: String,
}

impl StreamCaptureError {
    pub fn new(kind: StreamCaptureErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> StreamCaptureErrorKind {
        self.kind
    }
}

impl fmt::Display for StreamCaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for StreamCaptureError {}
