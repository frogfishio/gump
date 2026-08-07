//! Bridge STL-04 pipe drains into the D011 bounded LocalRing (STL-09).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use gump_driver::{PipeChunkSink, StreamKind as DriverStreamKind};

use crate::ring::{LocalRing, RingConfig};
use crate::stream::{EmitOutcome, StreamDrain, StreamEmitter, StreamKind, StreamRecord};

/// Fan-out target: segment pipe bytes and push into a shared [`LocalRing`].
pub struct AttemptPipeBridge {
    inner: Arc<Mutex<BridgeInner>>,
}

struct BridgeInner {
    ring: LocalRing,
    stdout: StreamDrain,
    stderr: StreamDrain,
}

impl AttemptPipeBridge {
    pub fn new(config: RingConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BridgeInner {
                ring: LocalRing::new(config),
                stdout: StreamDrain::new(StreamKind::Stdout).expect("stdout topic"),
                stderr: StreamDrain::new(StreamKind::Stderr).expect("stderr topic"),
            })),
        }
    }

    pub fn shared_sink(self) -> Arc<dyn PipeChunkSink> {
        Arc::new(self)
    }

    pub fn with_ring<R>(&self, f: impl FnOnce(&LocalRing) -> R) -> R {
        let g = self.inner.lock().expect("pipe bridge lock");
        f(&g.ring)
    }

    pub fn dropped_oldest(&self) -> u64 {
        self.with_ring(|r| r.dropped_oldest())
    }

    pub fn pushed(&self) -> u64 {
        self.with_ring(|r| r.pushed())
    }

    /// EOF flush for both drains (call after child exit / drain join).
    pub fn finish(&self) {
        let mut g = self.inner.lock().expect("pipe bridge lock");
        let now = Instant::now();
        let BridgeInner {
            ring,
            stdout,
            stderr,
        } = &mut *g;
        {
            let mut emitter = RingPush { ring, now };
            stdout.finish(&mut emitter);
        }
        {
            let mut emitter = RingPush { ring, now };
            stderr.finish(&mut emitter);
        }
    }
}

impl Clone for AttemptPipeBridge {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl PipeChunkSink for AttemptPipeBridge {
    fn on_chunk(&self, kind: DriverStreamKind, chunk: &[u8]) {
        let mut g = self.inner.lock().expect("pipe bridge lock");
        let now = Instant::now();
        let BridgeInner {
            ring,
            stdout,
            stderr,
        } = &mut *g;
        let mut emitter = RingPush { ring, now };
        match kind {
            DriverStreamKind::Stdout => stdout.push(chunk, &mut emitter),
            DriverStreamKind::Stderr => stderr.push(chunk, &mut emitter),
        }
    }
}

struct RingPush<'a> {
    ring: &'a mut LocalRing,
    now: Instant,
}

impl StreamEmitter for RingPush<'_> {
    fn emit(&mut self, record: StreamRecord) -> EmitOutcome {
        self.ring.push(record, self.now)
    }
}
