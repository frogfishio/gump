//! Exact opt-in detection on health responses (HICCUP.md §7).

use crate::codec::{CodecError, Declaration, media_type_matches, parse_declaration};

#[derive(Clone, Debug, PartialEq)]
pub enum Detection {
    /// Ordinary health — Hiccup inactive / unchanged legacy path.
    Inactive,
    /// Exact media type + `hiccup: 1` + bounded JSON.
    Active(Declaration),
    /// Media type claimed but body unusable — discovery degrades; health unchanged.
    Malformed(CodecError),
}

/// Detect Hiccup activation from a successful health response.
///
/// Anything other than exact media type + valid `{ "hiccup": 1, ... }` is
/// [`Detection::Inactive`] (or [`Detection::Malformed`] when the media type
/// matches but the body does not).
pub fn detect_health_response(content_type: Option<&str>, body: &[u8]) -> Detection {
    let Some(ct) = content_type else {
        return Detection::Inactive;
    };
    if !media_type_matches(ct) {
        return Detection::Inactive;
    }
    match parse_declaration(body) {
        Ok(d) => Detection::Active(d),
        Err(e) => Detection::Malformed(e),
    }
}

/// True when content-type is absent or non-Hiccup (legacy corpus).
pub fn is_legacy_health(content_type: Option<&str>) -> bool {
    match content_type {
        None => true,
        Some(ct) => !media_type_matches(ct),
    }
}
