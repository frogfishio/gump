//! W01 exit evidence: required crate membership + dependency-direction policy.
//!
//! Authority: docs/v1/README.md §5, docs/v1/DELIVERY.md W01, docs/v1/DECISIONS.md D001.
//!
//! Rules encoded here:
//! - Protocol crates must not depend on transport crates (types do not leak).
//! - Drivers and connectors must not depend on cluster memory (no direct state mutation).
//! - Dependencies may only flow downward (or sideways) across ownership layers.
//! - Product crates must not depend on `gump-gates` (tooling is not runtime).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const PRODUCT_CRATES: &[&str] = &[
    "gump-cli",
    "gump-manifest",
    "gump-capsule",
    "gump-crypto",
    "gump-protocol",
    "gump-memory",
    "gump-transport",
    "gump-scheduler",
    "gump-agent",
    "gump-driver",
    "gump-telemetry",
    "gump-connectors",
    "gump-server",
];

/// Ownership layers: a crate may depend on equal/lower layers only.
const LAYERS: &[(&str, u8)] = &[
    ("gump-crypto", 0),
    ("gump-protocol", 0),
    ("gump-manifest", 0),
    ("gump-driver", 0),
    ("gump-capsule", 1),
    ("gump-transport", 1),
    ("gump-connectors", 1),
    ("gump-telemetry", 1),
    ("gump-memory", 2),
    ("gump-scheduler", 3),
    ("gump-agent", 4),
    ("gump-server", 5),
    ("gump-cli", 5),
];

/// Absolute bans from docs/v1/README.md §5 and DELIVERY ownership boundaries.
const FORBIDDEN_EDGES: &[(&str, &str)] = &[
    ("gump-protocol", "gump-transport"),
    ("gump-driver", "gump-memory"),
    ("gump-connectors", "gump-memory"),
    ("gump-manifest", "gump-memory"),
    ("gump-crypto", "gump-memory"),
    ("gump-crypto", "gump-transport"),
    ("gump-protocol", "gump-agent"),
    ("gump-protocol", "gump-server"),
    ("gump-protocol", "gump-cli"),
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/gump-gates → workspace root")
        .to_path_buf()
}

fn layer_of(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

/// Collect direct `gump-*` dependencies from a package Cargo.toml.
fn gump_path_deps(cargo_toml: &str) -> BTreeSet<String> {
    let mut deps = BTreeSet::new();
    let mut in_deps = false;
    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_deps = matches!(
                trimmed,
                "[dependencies]"
                    | "[dev-dependencies]"
                    | "[build-dependencies]"
                    | "[target.'cfg(any())'.dependencies]"
            ) || (trimmed.starts_with("[target.") && trimmed.ends_with("dependencies]"));
            continue;
        }
        if !in_deps || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let key = trimmed.split('=').next().unwrap_or("").trim();
        if key.starts_with("gump-") {
            deps.insert(key.to_string());
        }
    }
    deps
}

#[test]
fn required_product_crates_exist() {
    let root = workspace_root();
    let mut missing = Vec::new();
    for name in PRODUCT_CRATES {
        let path = root.join("crates").join(name).join("Cargo.toml");
        if !path.is_file() {
            missing.push(name.to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "missing product crates from docs/v1/README.md §5: {missing:?}"
    );
}

#[test]
fn dependency_direction_holds() {
    let root = workspace_root();
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut violations = Vec::new();

    for name in PRODUCT_CRATES {
        let path = root.join("crates").join(name).join("Cargo.toml");
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let deps = gump_path_deps(&text);
        for dep in &deps {
            if dep == "gump-gates" {
                violations.push(format!("{name} must not depend on tooling crate gump-gates"));
            }
            if !PRODUCT_CRATES.contains(&dep.as_str()) && dep != "gump-gates" {
                violations.push(format!("{name} depends on unknown gump crate {dep}"));
            }
            for (from, to) in FORBIDDEN_EDGES {
                if name == from && dep == to {
                    violations.push(format!("forbidden edge {from} → {to}"));
                }
            }
            match (layer_of(name), layer_of(dep)) {
                (Some(from_l), Some(to_l)) if from_l < to_l => {
                    violations.push(format!(
                        "layer inversion: {name} (L{from_l}) → {dep} (L{to_l})"
                    ));
                }
                (None, _) => violations.push(format!("missing layer for {name}")),
                (_, None) if PRODUCT_CRATES.contains(&dep.as_str()) => {
                    violations.push(format!("missing layer for {dep}"));
                }
                _ => {}
            }
        }
        edges.insert((*name).to_string(), deps);
    }

    assert!(
        violations.is_empty(),
        "dependency-direction violations:\n  - {}",
        violations.join("\n  - ")
    );

    // Smoke: empty edge set is valid for W01 stubs.
    let _ = edges;
}

#[test]
fn workspace_lists_all_product_crates() {
    let root = workspace_root();
    let text = fs::read_to_string(root.join("Cargo.toml")).expect("workspace Cargo.toml");
    let mut missing = Vec::new();
    for name in PRODUCT_CRATES {
        let needle = format!("crates/{name}");
        if !text.contains(&needle) {
            missing.push(*name);
        }
    }
    assert!(
        missing.is_empty(),
        "root Cargo.toml workspace.members missing: {missing:?}"
    );
}
