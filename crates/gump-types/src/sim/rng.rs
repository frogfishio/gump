//! Seeded PRNG for reproducible fault decisions.

/// Tiny deterministic xorshift64*. Enough for smoke / property sims; not crypto.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimRng {
    state: u64,
}

impl SimRng {
    pub fn new(seed: u64) -> Self {
        // Avoid the all-zero fixed point of xorshift.
        Self {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    pub fn seed(&self) -> u64 {
        self.state
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Uniform in `0..bound` (bound > 0).
    pub fn gen_range(&mut self, bound: u64) -> u64 {
        debug_assert!(bound > 0);
        self.next_u64() % bound
    }

    /// Bernoulli trial: true with probability `numer/denom` (denom > 0).
    pub fn chance(&mut self, numer: u64, denom: u64) -> bool {
        debug_assert!(denom > 0);
        if numer == 0 {
            return false;
        }
        if numer >= denom {
            return true;
        }
        self.gen_range(denom) < numer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = SimRng::new(42);
        let mut b = SimRng::new(42);
        for _ in 0..32 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn zero_seed_is_usable() {
        let mut rng = SimRng::new(0);
        assert_ne!(rng.next_u64(), 0);
    }
}
