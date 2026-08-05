//! Deterministic simulation harness (DELIVERY W05 / CONFORMANCE §4).
//!
//! Controls monotonic time, message delivery (loss / delay / duplicate /
//! reorder / partition), process crash, and restart-with-empty-memory.
//! Higher workstreams (C03+, H02+) compose on this layer; W05 only needs a
//! smoke suite that proves the controls are deterministic.

mod net;
mod process;
mod rng;
mod world;

pub use net::{Delivered, Envelope, LinkFaults, Network, NetworkError, PeerId};
pub use process::{ProcessStatus, SimProcess};
pub use rng::SimRng;
pub use world::{SimWorld, StepOutcome};
