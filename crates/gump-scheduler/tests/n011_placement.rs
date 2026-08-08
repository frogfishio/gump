//! GUMP-N011 / R01–R04: minimum arbitrary-workload placement.
//!
//! Authority: docs/v1/RUNTIME.md §1–2, docs/v1/NEXT_ACTIONS.md GUMP-N011,
//! docs/v1/DELIVERY.md R01–R04.

use std::collections::BTreeMap;

use gump_scheduler::{
    CapabilityReport, NodeResources, PlacementController, PlacementOutcome, ProtectionLevel,
    WorkloadRequirements, codes,
};
use gump_types::{NodeId, UnitId, WorkloadId};

fn v7(seed: u8) -> [u8; 16] {
    let mut b = [seed; 16];
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    b
}

fn node(seed: u8) -> NodeId {
    NodeId::from_bytes(v7(seed)).unwrap()
}

fn workload(seed: u8) -> WorkloadId {
    WorkloadId::from_bytes(v7(seed)).unwrap()
}

fn unit(seed: u8) -> UnitId {
    UnitId::from_bytes(v7(seed)).unwrap()
}

fn caps(pairs: &[(&str, ProtectionLevel)]) -> BTreeMap<String, ProtectionLevel> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
}

fn base_node(seed: u8) -> CapabilityReport {
    CapabilityReport {
        node_id: node(seed),
        revision: 1,
        placement_fence: 10,
        arch: "x86_64".into(),
        drivers: vec!["native".into(), "script".into()],
        capabilities: caps(&[
            ("cgroup", ProtectionLevel::Enforced),
            ("namespaces", ProtectionLevel::Enforced),
        ]),
        allocatable: NodeResources {
            millicores: 4000,
            memory_bytes: 8 * 1024 * 1024 * 1024,
            gpu_devices: 0,
            ports: 64,
        },
        drained: false,
    }
}

fn req(driver: &str, millicores: u32, memory_mib: u64) -> WorkloadRequirements {
    WorkloadRequirements {
        workload_id: workload(0x20),
        unit_id: unit(0x21),
        arch: "x86_64".into(),
        driver: driver.into(),
        required_enforced: vec!["cgroup".into(), "namespaces".into()],
        request: NodeResources {
            millicores,
            memory_bytes: memory_mib * 1024 * 1024,
            gpu_devices: 0,
            ports: 0,
        },
        requires_port: false,
        lifecycle_finite: true,
    }
}

#[test]
fn finite_native_and_continuous_native_schedule() {
    let mut ctl = PlacementController::new();
    ctl.upsert_report(base_node(1)).unwrap();

    let mut finite = req("native", 500, 256);
    finite.lifecycle_finite = true;
    finite.unit_id = unit(0x31);
    match ctl.place(&finite) {
        PlacementOutcome::Scheduled(p) => {
            assert_eq!(p.reservation.node_id, node(1));
            assert_eq!(p.reservation.capability_revision, 1);
        }
        other => panic!("finite native expected schedule, got {other:?}"),
    }

    let mut continuous = req("native", 500, 256);
    continuous.lifecycle_finite = false;
    continuous.unit_id = unit(0x32);
    continuous.workload_id = workload(0x22);
    assert!(matches!(
        ctl.place(&continuous),
        PlacementOutcome::Scheduled(_)
    ));
}

#[test]
fn script_driver_requires_script_capability_on_node() {
    let mut ctl = PlacementController::new();
    let mut n = base_node(2);
    n.drivers = vec!["native".into()]; // no script
    ctl.upsert_report(n).unwrap();

    let script = req("script", 100, 64);
    match ctl.place(&script) {
        PlacementOutcome::Unschedulable { matrix, .. } => {
            assert!(
                matrix[0]
                    .reasons
                    .iter()
                    .any(|r| r.code == codes::DRIVER_MISSING)
            );
        }
        other => panic!("expected unschedulable, got {other:?}"),
    }

    let mut ok = base_node(3);
    ok.drivers = vec!["native".into(), "script".into()];
    ctl.upsert_report(ok).unwrap();
    let mut script_ok = req("script", 100, 64);
    script_ok.unit_id = unit(0x33);
    assert!(matches!(
        ctl.place(&script_ok),
        PlacementOutcome::Scheduled(_)
    ));
}

#[test]
fn gpu_requesting_fixture() {
    let mut ctl = PlacementController::new();
    let mut cpu_only = base_node(4);
    cpu_only.allocatable.gpu_devices = 0;
    ctl.upsert_report(cpu_only).unwrap();

    let mut gpu_req = req("native", 1000, 1024);
    gpu_req.request.gpu_devices = 1;
    gpu_req.required_enforced.push("gpu".into());
    match ctl.place(&gpu_req) {
        PlacementOutcome::Unschedulable { matrix, .. } => {
            let codes_seen: Vec<_> = matrix[0].reasons.iter().map(|r| r.code).collect();
            assert!(
                codes_seen.contains(&codes::GPU) || codes_seen.contains(&codes::CAPABILITY_MISSING)
            );
        }
        other => panic!("expected unschedulable on cpu-only, got {other:?}"),
    }

    let mut gpu_node = base_node(5);
    gpu_node.allocatable.gpu_devices = 2;
    gpu_node
        .capabilities
        .insert("gpu".into(), ProtectionLevel::Enforced);
    ctl.upsert_report(gpu_node).unwrap();
    gpu_req.unit_id = unit(0x35);
    match ctl.place(&gpu_req) {
        PlacementOutcome::Scheduled(p) => assert_eq!(p.reservation.node_id, node(5)),
        other => panic!("expected GPU schedule, got {other:?}"),
    }
}

#[test]
fn portless_fixture_ignores_zero_port_nodes_only_when_port_required() {
    let mut ctl = PlacementController::new();
    let mut portless = base_node(6);
    portless.allocatable.ports = 0;
    ctl.upsert_report(portless).unwrap();

    let mut no_port = req("native", 100, 64);
    no_port.requires_port = false;
    assert!(matches!(
        ctl.place(&no_port),
        PlacementOutcome::Scheduled(_)
    ));

    let mut needs_port = req("native", 100, 64);
    needs_port.unit_id = unit(0x36);
    needs_port.requires_port = true;
    match ctl.place(&needs_port) {
        PlacementOutcome::Unschedulable { matrix, .. } => {
            assert!(
                matrix[0]
                    .reasons
                    .iter()
                    .any(|r| r.code == codes::PORT_REQUIRED)
            );
        }
        other => panic!("expected port rejection, got {other:?}"),
    }
}

#[test]
fn unschedulable_lists_every_hard_rejection() {
    let mut ctl = PlacementController::new();
    let mut bad = base_node(7);
    bad.arch = "aarch64".into();
    bad.drivers.clear();
    bad.drained = true;
    bad.allocatable.millicores = 1;
    bad.capabilities
        .insert("cgroup".into(), ProtectionLevel::Observed);
    ctl.upsert_report(bad).unwrap();

    let r = req("native", 2000, 512);
    match ctl.place(&r) {
        PlacementOutcome::Unschedulable { matrix, .. } => {
            let codes_seen: Vec<_> = matrix[0].reasons.iter().map(|x| x.code).collect();
            assert!(codes_seen.contains(&codes::NODE_DRAINED));
            assert!(codes_seen.contains(&codes::ARCH_MISMATCH));
            assert!(codes_seen.contains(&codes::DRIVER_MISSING));
            assert!(codes_seen.contains(&codes::CAPABILITY_NOT_ENFORCED));
            assert!(codes_seen.contains(&codes::MILLICORES));
        }
        other => panic!("expected multi-reason unschedulable, got {other:?}"),
    }
}

#[test]
fn reservation_before_admit_and_stale_fence_fails() {
    let mut ctl = PlacementController::new();
    ctl.upsert_report(base_node(8)).unwrap();
    let r = req("native", 100, 64);
    let plan = match ctl.place(&r) {
        PlacementOutcome::Scheduled(p) => p,
        other => panic!("expected schedule, got {other:?}"),
    };

    // Reservation committed before admit/launch.
    assert!(ctl.ledger.get(r.unit_id).is_some());

    let live = ctl.report(node(8)).unwrap().clone();
    ctl.admit(&plan.reservation, &live).expect("fresh admit");

    let mut stale_rev = live.clone();
    stale_rev.revision = 99;
    let err = ctl.admit(&plan.reservation, &stale_rev).unwrap_err();
    assert!(err.iter().any(|e| e.code == codes::STALE_CAPABILITY));

    let mut stale_fence = live;
    stale_fence.placement_fence = 0;
    let err = ctl.admit(&plan.reservation, &stale_fence).unwrap_err();
    assert!(err.iter().any(|e| e.code == codes::STALE_FENCE));
}

#[test]
fn observed_capability_rejected_when_enforcement_required() {
    let mut ctl = PlacementController::new();
    let mut n = base_node(9);
    n.capabilities
        .insert("memlock".into(), ProtectionLevel::Observed);
    ctl.upsert_report(n).unwrap();

    let mut r = req("native", 100, 64);
    r.required_enforced.push("memlock".into());
    match ctl.place(&r) {
        PlacementOutcome::Unschedulable { matrix, .. } => {
            assert!(
                matrix[0]
                    .reasons
                    .iter()
                    .any(|x| x.code == codes::CAPABILITY_NOT_ENFORCED)
            );
        }
        other => panic!("expected not-enforced rejection, got {other:?}"),
    }
}

#[test]
fn resource_structures_are_bounded() {
    let mut ctl = PlacementController::new();
    ctl.ledger = gump_scheduler::ResourceLedger::with_limits(2, 2);
    ctl.upsert_report(base_node(0xa0)).unwrap();
    ctl.upsert_report(base_node(0xa1)).unwrap();

    let mut a = req("native", 10, 16);
    a.unit_id = unit(0xa2);
    assert!(matches!(ctl.place(&a), PlacementOutcome::Scheduled(_)));
    let mut b = req("native", 10, 16);
    b.unit_id = unit(0xa3);
    assert!(matches!(ctl.place(&b), PlacementOutcome::Scheduled(_)));
    let mut c = req("native", 10, 16);
    c.unit_id = unit(0xa4);
    match ctl.place(&c) {
        PlacementOutcome::Unschedulable { summary, .. } => {
            assert_eq!(summary.code, codes::LEDGER_FULL);
        }
        other => panic!("expected ledger full, got {other:?}"),
    }
}
