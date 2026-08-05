//! Simulated peer lifecycle: running, crashed, restart-empty.

use core::fmt;

/// Whether a simulated process can send/receive.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ProcessStatus {
    Running,
    Crashed,
}

impl fmt::Display for ProcessStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Crashed => write!(f, "crashed"),
        }
    }
}

/// Peer process with typed RAM that clears on restart (CONFORMANCE §4).
#[derive(Clone, Debug)]
pub struct SimProcess<S> {
    status: ProcessStatus,
    /// Monotonic crash/restart generation; increments on each crash.
    incarnation: u64,
    memory: S,
}

impl<S: Default> SimProcess<S> {
    pub fn new() -> Self {
        Self {
            status: ProcessStatus::Running,
            incarnation: 0,
            memory: S::default(),
        }
    }

    pub fn with_memory(memory: S) -> Self {
        Self {
            status: ProcessStatus::Running,
            incarnation: 0,
            memory,
        }
    }

    pub fn status(&self) -> ProcessStatus {
        self.status
    }

    pub fn is_running(&self) -> bool {
        self.status == ProcessStatus::Running
    }

    pub fn incarnation(&self) -> u64 {
        self.incarnation
    }

    pub fn memory(&self) -> &S {
        &self.memory
    }

    pub fn memory_mut(&mut self) -> &mut S {
        &mut self.memory
    }

    /// Crash: mark down and bump incarnation. Memory is retained until restart
    /// so crash-inspect tests can observe last RAM; restart clears it.
    pub fn crash(&mut self) {
        if self.status == ProcessStatus::Running {
            self.status = ProcessStatus::Crashed;
            self.incarnation = self.incarnation.saturating_add(1);
        }
    }

    /// Restart with empty memory (total loss of that peer's RAM).
    pub fn restart_empty(&mut self)
    where
        S: Default,
    {
        self.status = ProcessStatus::Running;
        self.memory = S::default();
    }
}

impl<S: Default> Default for SimProcess<S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_then_restart_clears_memory() {
        let mut p = SimProcess::with_memory(String::from("live-intent"));
        p.crash();
        assert_eq!(p.status(), ProcessStatus::Crashed);
        assert_eq!(p.incarnation(), 1);
        assert_eq!(p.memory(), "live-intent");
        p.restart_empty();
        assert!(p.is_running());
        assert_eq!(p.memory(), "");
        assert_eq!(p.incarnation(), 1);
    }
}
