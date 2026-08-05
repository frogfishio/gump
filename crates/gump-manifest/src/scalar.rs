//! Duration and byte-size scalars (FORMATS.md §1).

use crate::error::{ManifestError, ManifestErrorKind};

/// Parse `10s` / `5m` / `1h` / `250ms` into milliseconds.
pub fn parse_duration_millis(raw: &str) -> Result<u64, ManifestError> {
    let raw = raw.trim();
    let (digits, mult) = if let Some(rest) = raw.strip_suffix("ms") {
        (rest, 1u64)
    } else if let Some(rest) = raw.strip_suffix('s') {
        (rest, 1_000)
    } else if let Some(rest) = raw.strip_suffix('m') {
        (rest, 60_000)
    } else if let Some(rest) = raw.strip_suffix('h') {
        (rest, 3_600_000)
    } else {
        return Err(ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "duration",
            format!("expected <n>ms|s|m|h, got {raw:?}"),
        ));
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "duration",
            format!("duration digits invalid: {raw:?}"),
        ));
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return Err(ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "duration",
            format!("duration must not have leading zeros: {raw:?}"),
        ));
    }
    let n: u64 = digits.parse().map_err(|_| {
        ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "duration",
            format!("duration out of range: {raw:?}"),
        )
    })?;
    n.checked_mul(mult).ok_or_else(|| {
        ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "duration",
            format!("duration overflow: {raw:?}"),
        )
    })
}

/// Parse raw bytes or `KiB`/`MiB`/`GiB` into a `u64` byte count.
pub fn parse_byte_size(raw: &str) -> Result<u64, ManifestError> {
    let raw = raw.trim();
    let (digits, mult) = if let Some(rest) = raw.strip_suffix("GiB") {
        (rest, 1024u64 * 1024 * 1024)
    } else if let Some(rest) = raw.strip_suffix("MiB") {
        (rest, 1024 * 1024)
    } else if let Some(rest) = raw.strip_suffix("KiB") {
        (rest, 1024)
    } else {
        (raw, 1)
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "bytes",
            format!("expected <n>[KiB|MiB|GiB], got {raw:?}"),
        ));
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return Err(ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "bytes",
            format!("byte size must not have leading zeros: {raw:?}"),
        ));
    }
    let n: u64 = digits.parse().map_err(|_| {
        ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "bytes",
            format!("byte size out of range: {raw:?}"),
        )
    })?;
    n.checked_mul(mult).ok_or_else(|| {
        ManifestError::new(
            ManifestErrorKind::InvalidValue,
            "bytes",
            format!("byte size overflow: {raw:?}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration_millis("0ms").unwrap(), 0);
        assert_eq!(parse_duration_millis("10s").unwrap(), 10_000);
        assert_eq!(parse_duration_millis("5m").unwrap(), 300_000);
        assert_eq!(parse_duration_millis("1h").unwrap(), 3_600_000);
        assert!(parse_duration_millis("10").is_err());
        assert!(parse_duration_millis("01s").is_err());
    }

    #[test]
    fn bytes() {
        assert_eq!(parse_byte_size("64").unwrap(), 64);
        assert_eq!(parse_byte_size("4KiB").unwrap(), 4096);
        assert_eq!(parse_byte_size("64MiB").unwrap(), 64 * 1024 * 1024);
        assert!(parse_byte_size("64MB").is_err());
    }
}
