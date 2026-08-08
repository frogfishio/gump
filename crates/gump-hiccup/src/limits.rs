//! Bounds from docs/v1/HICCUP.md §10.

/// Protocol profile name.
pub const PROFILE: &str = "gump.hiccup/1";

/// Exact application media type (version=1).
pub const MEDIA_TYPE: &str = "application/vnd.gump.hiccup+json; version=1";

/// Offer header name on ordinary health GET.
pub const OFFER_HEADER: &str = "Hiccup-Offer";

/// Offer header value.
pub const OFFER_VALUE: &str = "1";

/// Authorization scheme for authenticated health POST.
pub const AUTH_SCHEME: &str = "Hiccup";

/// Environment entry naming the sealed token FD (descriptor number only).
pub const TOKEN_FD_ENV: &str = "GUMP_HICCUP_TOKEN_FD";

pub const MAX_DECLARATION_BYTES: usize = 64 * 1024;
pub const MAX_DELIVERY_BYTES: usize = 256 * 1024;
pub const MAX_PUBLIC_DATA_BYTES: usize = 8 * 1024;
pub const MAX_SECRET_DATA_BYTES: usize = 32 * 1024;
pub const MAX_JSON_DEPTH: usize = 16;
pub const MAX_LISTEN_TOPICS: usize = 32;
pub const MAX_PUBLISHERS_PER_TOPIC: usize = 10_000;
pub const MAX_INTRODUCTIONS_PER_POST: usize = 256;
pub const MAX_KEEPER_BYTES: usize = 64 * 1024 * 1024;
pub const TOKEN_BYTES: usize = 32;

/// Presence TTL floor (ms).
pub const MIN_PRESENCE_TTL_MS: u64 = 30_000;
/// Presence TTL cap (ms).
pub const MAX_PRESENCE_TTL_MS: u64 = 300_000;
/// Multiplier on health interval for derived TTL.
pub const PRESENCE_INTERVAL_MULT: u64 = 3;

/// Derived safety timeout: max(30s, 3×interval), capped at 5 minutes.
pub fn presence_ttl_ms(health_interval_ms: u64) -> u64 {
    let derived = health_interval_ms.saturating_mul(PRESENCE_INTERVAL_MULT);
    derived.clamp(MIN_PRESENCE_TTL_MS, MAX_PRESENCE_TTL_MS)
}
