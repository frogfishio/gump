//! Cooperative cancellation without tying foundation types to Tokio.

use core::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Signal that work should stop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// Shareable cancellation token (cloneable, wait-free check).
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Returns `Err(Cancelled)` when cancelled.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    /// Cancels when dropped — useful for scoped request lifetimes.
    pub fn guard(&self) -> CancellationGuard {
        CancellationGuard {
            token: self.clone(),
            disarm: false,
        }
    }
}

/// Cancels the associated token on drop unless disarmed.
#[derive(Debug)]
pub struct CancellationGuard {
    token: CancelToken,
    disarm: bool,
}

impl CancellationGuard {
    pub fn disarm(mut self) {
        self.disarm = true;
    }

    pub fn token(&self) -> &CancelToken {
        &self.token
    }
}

impl Drop for CancellationGuard {
    fn drop(&mut self) {
        if !self.disarm {
            self.token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_is_visible_to_clones() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(token.check().is_ok());
        token.cancel();
        assert_eq!(clone.check(), Err(Cancelled));
    }

    #[test]
    fn guard_cancels_on_drop() {
        let token = CancelToken::new();
        {
            let _g = token.guard();
            assert!(!token.is_cancelled());
        }
        assert!(token.is_cancelled());
    }

    #[test]
    fn guard_disarm_skips_cancel() {
        let token = CancelToken::new();
        {
            let g = token.guard();
            g.disarm();
        }
        assert!(!token.is_cancelled());
    }
}
