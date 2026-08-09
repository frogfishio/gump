//! F01 exit evidence: valid/invalid corpus vs fixtures + gump.schema.json rules.
//!
//! Authority: docs/v1/DELIVERY.md F01, docs/v1/FORMATS.md §10,
//! `spec/v1/gump.schema.json` (unknown keys, required fields, scalars,
//! `deploy.coverage=all_nodes` ⊕ `units` conflict).

use std::fs;
use std::path::{Path, PathBuf};

use gump_manifest::{
    Coordination, Coverage, Driver, HealthBinding, Lifetime, ManifestErrorKind, PortValue,
    SchemaVersion, SuccessPolicy, parse_manifest_str,
};
use gump_types::Label;

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec/v1/fixtures")
        .canonicalize()
        .expect("spec/v1/fixtures")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn valid_fixtures_parse_and_normalize() {
    let root = fixtures_root();
    for name in [
        "minimal-finite.toml",
        "gpu-gang.toml",
        "kismet-all-nodes.toml",
    ] {
        let path = root.join(name);
        let manifest =
            parse_manifest_str(&read(&path)).unwrap_or_else(|e| panic!("{name} should parse: {e}"));
        assert_eq!(manifest.schema, SchemaVersion::Gump1);
        assert_eq!(manifest.runtime.driver, Driver::Native);
    }

    let minimal = parse_manifest_str(&read(&root.join("minimal-finite.toml"))).unwrap();
    assert_eq!(minimal.app.id.as_str(), "hello-job");
    assert_eq!(minimal.workload.lifetime, Lifetime::Finite);
    assert_eq!(minimal.workload.coordination, Coordination::Independent);
    assert_eq!(minimal.workload.success, SuccessPolicy::AllExitZero);
    assert_eq!(
        minimal.runtime.stop_timeout_ms,
        Some(10_000),
        "10s → milliseconds"
    );
    let greeting = minimal.runtime.variables.get("GREETING").unwrap();
    assert_eq!(greeting.max_bytes, Some(4096));

    let gang = parse_manifest_str(&read(&root.join("gpu-gang.toml"))).unwrap();
    assert_eq!(gang.workload.coordination, Coordination::Gang);
    assert_eq!(gang.deploy.as_ref().unwrap().units, Some(64));
    assert_eq!(gang.resources.as_ref().unwrap().gpu_count, Some(8));
    assert_eq!(
        gang.resources.as_ref().unwrap().memory_request,
        Some(64 * 1024 * 1024 * 1024)
    );

    let kismet = parse_manifest_str(&read(&root.join("kismet-all-nodes.toml"))).unwrap();
    assert_eq!(kismet.workload.lifetime, Lifetime::Continuous);
    assert_eq!(
        kismet.deploy.as_ref().unwrap().coverage,
        Some(Coverage::AllNodes)
    );
    assert!(kismet.deploy.as_ref().unwrap().units.is_none());
    let health = &kismet.runtime.ports[&Label::parse("health").unwrap()];
    assert_eq!(health.value, PortValue::Auto);
    let cluster = &kismet.runtime.ports[&Label::parse("cluster").unwrap()];
    assert_eq!(cluster.value, PortValue::Fixed(7600));
    assert_eq!(
        kismet
            .health
            .as_ref()
            .unwrap()
            .readiness
            .as_ref()
            .unwrap()
            .interval_ms,
        5_000
    );
    assert_eq!(
        kismet
            .health
            .as_ref()
            .unwrap()
            .liveness
            .as_ref()
            .unwrap()
            .path
            .as_deref(),
        Some("/health")
    );
    assert_eq!(
        kismet
            .discovery
            .as_ref()
            .unwrap()
            .hiccup
            .as_ref()
            .unwrap()
            .health_binding,
        Some(HealthBinding::Liveness)
    );
}

#[test]
fn invalid_fixtures_are_rejected() {
    let root = fixtures_root().join("invalid");
    let cases = [
        ("unknown-key.toml", ManifestErrorKind::UnknownKey),
        ("bad-schema.toml", ManifestErrorKind::Schema),
        ("missing-app.toml", ManifestErrorKind::MissingField),
        ("bad-duration.toml", ManifestErrorKind::InvalidValue),
        ("all-nodes-with-units.toml", ManifestErrorKind::Semantic),
    ];
    for (name, kind) in cases {
        let err =
            parse_manifest_str(&read(&root.join(name))).expect_err(&format!("{name} must fail"));
        assert_eq!(err.kind(), kind, "{name}: {err}");
    }
}

#[test]
fn schema_file_is_present_for_corpus_authority() {
    let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/v1/gump.schema.json");
    let text = read(&schema);
    assert!(text.contains("\"const\": \"gump/1\""));
    assert!(text.contains("additionalProperties"));
}
