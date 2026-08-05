//! Client reconnect policy after transport loss (PROTOCOL.md § clients retry).

use core::time::Duration;

/// Bounded exponential backoff for same-operation-ID retries after transport loss.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconnectPolicy {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub multiplier_num: u32,
    pub multiplier_den: u32,
    pub max_attempts: u32,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_secs(5),
            multiplier_num: 2,
            multiplier_den: 1,
            max_attempts: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconnectDecision {
    /// Retry after `delay`, attempt index is 0-based.
    Retry { attempt: u32, delay: Duration },
    /// Give up; caller surfaces transport failure.
    GiveUp { attempts: u32 },
}

impl ReconnectPolicy {
    /// Decide the next reconnect action after `failed_attempts` prior failures.
    pub fn after_failures(self, failed_attempts: u32) -> ReconnectDecision {
        if failed_attempts >= self.max_attempts {
            return ReconnectDecision::GiveUp {
                attempts: failed_attempts,
            };
        }
        let mut delay = self.initial_delay;
        for _ in 0..failed_attempts {
            delay = delay
                .checked_mul(self.multiplier_num)
                .unwrap_or(self.max_delay)
                / self.multiplier_den.max(1);
            if delay > self.max_delay {
                delay = self.max_delay;
                break;
            }
        }
        if delay > self.max_delay {
            delay = self.max_delay;
        }
        ReconnectDecision::Retry {
            attempt: failed_attempts,
            delay,
        }
    }
}
