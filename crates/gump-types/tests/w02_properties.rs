//! Property-style coverage for W02 exit criteria.

use gump_types::{
    BoundedString, CancelToken, Clock, ClusterId, DurationMillis, IdError, Label, ManualClock,
    ReasonCode, SafeError, Secret,
};
use uuid::Uuid;

#[test]
fn label_property_charset_and_length() {
    // Accept only lowercase alnum/hyphen shapes within 63 bytes.
    for len in 1..=63 {
        let s: String = std::iter::repeat('a').take(len).collect();
        assert!(Label::parse(&s).is_ok(), "len={len}");
    }
    assert!(Label::parse(&"a".repeat(64)).is_err());

    for bad in ['A', '_', '.', '/', ' '] {
        let s = format!("a{bad}b");
        assert!(Label::parse(&s).is_err(), "byte {bad:?}");
    }
}

#[test]
fn id_generation_is_v7_and_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..64 {
        let id = ClusterId::new();
        assert_eq!(id.as_uuid().get_version(), Some(uuid::Version::SortRand));
        assert!(seen.insert(id.to_hyphenated()));
    }
}

#[test]
fn rejects_v4_nil_as_id() {
    assert_eq!(ClusterId::from_uuid(Uuid::nil()), Err(IdError::NotVersion7));
}

#[test]
fn secret_debug_never_contains_plaintext_property() {
    let samples = [
        "hunter2",
        "sk-live-abcdefghijklmnopqrstuvwxyz",
        "-----BEGIN PRIVATE KEY-----\nnice-try\n",
        "\0\u{1}\u{7f}secret",
    ];
    for sample in samples {
        let secret = Secret::new(sample.to_owned());
        let dbg = format!("{secret:?}");
        let disp = format!("{secret}");
        assert_eq!(dbg, "Secret(***)");
        assert_eq!(disp, "***");
        assert!(!dbg.contains(sample));
        assert!(!disp.contains(sample));
    }
}

#[test]
fn safe_error_plus_secret_context_stays_redacted() {
    let secret = Secret::new("top-secret-dek".to_owned());
    let err = SafeError::new(ReasonCode::Internal, SafeError::redact_context(secret));
    let out = format!("{err:?} | {err}");
    assert!(!out.contains("top-secret-dek"));
    assert!(out.contains("<redacted>") || out.contains("internal"));
}

#[test]
fn cancel_and_clock_compose() {
    let clock = ManualClock::new(0);
    let token = CancelToken::new();
    clock.advance(DurationMillis::from_millis(10));
    assert_eq!(clock.now().as_millis(), 10);
    token.cancel();
    assert!(token.is_cancelled());
}

#[test]
fn bounded_string_rejects_oversize_property() {
    assert!(BoundedString::<1>::try_from_str("x").is_ok());
    assert!(BoundedString::<1>::try_from_str("xx").is_err());
    assert!(BoundedString::<8>::try_from_str("12345678").is_ok());
    assert!(BoundedString::<8>::try_from_str("123456789").is_err());
    assert!(BoundedString::<32>::try_from_str(&"y".repeat(32)).is_ok());
    assert!(BoundedString::<32>::try_from_str(&"y".repeat(33)).is_err());
}
