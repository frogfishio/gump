//! Clocks for production and deterministic simulation (W02 / CONFORMANCE).

use core::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since an arbitrary epoch (monotonic within one clock instance).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct InstantMillis(u64);

impl InstantMillis {
    pub const fn from_millis(ms: u64) -> Self {
        Self(ms)
    }

    pub const fn as_millis(self) -> u64 {
        self.0
    }

    pub fn saturating_duration_since(self, earlier: Self) -> DurationMillis {
        DurationMillis(self.0.saturating_sub(earlier.0))
    }
}

/// Non-negative millisecond duration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DurationMillis(u64);

impl DurationMillis {
    pub const fn from_millis(ms: u64) -> Self {
        Self(ms)
    }

    pub const fn as_millis(self) -> u64 {
        self.0
    }
}

/// Clock abstraction so simulation (W05) can inject time.
pub trait Clock: Send + Sync {
    fn now(&self) -> InstantMillis;
}

/// Wall-clock backed by `SystemTime` (millisecond resolution).
#[derive(Clone, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> InstantMillis {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        InstantMillis(ms)
    }
}

/// Deterministic clock for tests and the simulation harness.
#[derive(Clone, Debug)]
pub struct ManualClock {
    millis: Arc<AtomicU64>,
}

impl ManualClock {
    pub fn new(start_ms: u64) -> Self {
        Self {
            millis: Arc::new(AtomicU64::new(start_ms)),
        }
    }

    pub fn advance(&self, by: DurationMillis) {
        self.millis.fetch_add(by.as_millis(), Ordering::SeqCst);
    }

    pub fn set(&self, at: InstantMillis) {
        self.millis.store(at.as_millis(), Ordering::SeqCst);
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Clock for ManualClock {
    fn now(&self) -> InstantMillis {
        InstantMillis(self.millis.load(Ordering::SeqCst))
    }
}

impl fmt::Display for InstantMillis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_advances() {
        let clock = ManualClock::new(100);
        assert_eq!(clock.now().as_millis(), 100);
        clock.advance(DurationMillis::from_millis(50));
        assert_eq!(clock.now().as_millis(), 150);
        let later = clock.now();
        clock.set(InstantMillis::from_millis(10));
        assert_eq!(
            later.saturating_duration_since(clock.now()).as_millis(),
            140
        );
    }

    #[test]
    fn system_clock_is_nonzero() {
        let now = SystemClock.now();
        assert!(now.as_millis() > 0);
    }
}
