//! C04 exit evidence: typed record SM — transactions, properties, budgets.
//!
//! Authority: docs/v1/DELIVERY.md C04, PROTOCOL.md §6–§7.

use gump_memory::{
    ApplyError, BudgetClass, Command, Comparison, Expected, KeyPrefix, MemoryBudgets, MutateOp,
    RecordKey, Txn, TypedRecordMachine,
};

fn key(prefix: KeyPrefix, suffix: &str) -> RecordKey {
    RecordKey::new(prefix, suffix).unwrap()
}

#[test]
fn put_delete_advances_revision_and_reads_back() {
    let mut m = TypedRecordMachine::with_defaults();
    let k = key(KeyPrefix::ClusterMeta, "");
    let r = m
        .apply(Command::Put {
            key: k.clone(),
            expected: Expected::Absent,
            payload: b"meta-v1".to_vec(),
            leased: false,
        })
        .unwrap();
    assert_eq!(r.revision, 1);
    assert_eq!(m.get(&k).unwrap().payload, b"meta-v1");
    assert_eq!(m.get(&k).unwrap().revision, 1);

    m.apply(Command::Delete {
        key: k.clone(),
        expected: Expected::ExactRevision(1),
    })
    .unwrap();
    assert_eq!(m.revision(), 2);
    assert!(m.get(&k).is_none());
}

#[test]
fn expected_revision_and_digest_preconditions() {
    let mut m = TypedRecordMachine::with_defaults();
    let k = key(KeyPrefix::Names, "default/app");
    m.apply(Command::Put {
        key: k.clone(),
        expected: Expected::Absent,
        payload: b"w1".to_vec(),
        leased: false,
    })
    .unwrap();
    let dig = m.get(&k).unwrap().digest;

    let err = m
        .apply(Command::Put {
            key: k.clone(),
            expected: Expected::ExactRevision(99),
            payload: b"w2".to_vec(),
            leased: false,
        })
        .unwrap_err();
    assert!(matches!(err, ApplyError::PreconditionFailed { .. }));

    m.apply(Command::Put {
        key: k.clone(),
        expected: Expected::ExactDigest(dig),
        payload: b"w2".to_vec(),
        leased: false,
    })
    .unwrap();
    assert_eq!(m.get(&k).unwrap().payload, b"w2");
}

#[test]
fn txn_atomic_success_and_failure_branches() {
    let mut m = TypedRecordMachine::with_defaults();
    let a = key(KeyPrefix::Units, "u1");
    let b = key(KeyPrefix::Units, "u2");
    m.apply(Command::Put {
        key: a.clone(),
        expected: Expected::Absent,
        payload: b"a0".to_vec(),
        leased: false,
    })
    .unwrap();

    // Success path: comparison holds → both ops at one revision.
    let r = m
        .apply(Command::Txn(Txn {
            comparisons: vec![Comparison {
                key: a.clone(),
                expected: Expected::ExactRevision(1),
            }],
            success_ops: vec![
                MutateOp::Put {
                    key: a.clone(),
                    expected: Expected::Any,
                    payload: b"a1".to_vec(),
                    leased: false,
                },
                MutateOp::Put {
                    key: b.clone(),
                    expected: Expected::Absent,
                    payload: b"b0".to_vec(),
                    leased: false,
                },
            ],
            failure_ops: vec![],
        }))
        .unwrap();
    assert_eq!(r.txn_succeeded, Some(true));
    assert_eq!(m.get(&a).unwrap().payload, b"a1");
    assert_eq!(m.get(&b).unwrap().payload, b"b0");
    // Two puts → revision advanced twice within the txn.
    assert_eq!(m.revision(), 3);

    // Failure path: comparison fails → failure_ops only.
    let before = m.revision();
    let r = m
        .apply(Command::Txn(Txn {
            comparisons: vec![Comparison {
                key: a.clone(),
                expected: Expected::ExactRevision(1), // stale
            }],
            success_ops: vec![MutateOp::Delete {
                key: a.clone(),
                expected: Expected::Any,
            }],
            failure_ops: vec![MutateOp::Put {
                key: key(KeyPrefix::Reasons, "units/u1"),
                expected: Expected::Any,
                payload: b"conflict".to_vec(),
                leased: false,
            }],
        }))
        .unwrap();
    assert_eq!(r.txn_succeeded, Some(false));
    assert!(m.get(&a).is_some(), "success delete must not run");
    assert_eq!(
        m.get(&key(KeyPrefix::Reasons, "units/u1"))
            .unwrap()
            .payload,
        b"conflict"
    );
    assert!(m.revision() > before);
}

#[test]
fn txn_rolls_back_on_mid_apply_error() {
    let mut m = TypedRecordMachine::with_defaults();
    let a = key(KeyPrefix::Executions, "e1");
    m.apply(Command::Put {
        key: a.clone(),
        expected: Expected::Absent,
        payload: b"ok".to_vec(),
        leased: false,
    })
    .unwrap();
    let rev_before = m.revision();
    let usage_before = m.usage();

    // Second put exceeds AuthorityController max (8 KiB) — txn must restore.
    let huge = vec![0u8; 9 * 1024];
    let err = m
        .apply(Command::Txn(Txn {
            comparisons: vec![],
            success_ops: vec![
                MutateOp::Put {
                    key: a.clone(),
                    expected: Expected::Any,
                    payload: b"changed".to_vec(),
                    leased: false,
                },
                MutateOp::Put {
                    key: key(KeyPrefix::AuthorityController, ""),
                    expected: Expected::Absent,
                    payload: huge,
                    leased: true,
                },
            ],
            failure_ops: vec![],
        }))
        .unwrap_err();
    assert!(matches!(err, ApplyError::Value(_)));
    assert_eq!(m.revision(), rev_before);
    assert_eq!(m.usage(), usage_before);
    assert_eq!(m.get(&a).unwrap().payload, b"ok");
}

#[test]
fn budget_exhaustion_rejects_growth_never_evicts() {
    let budgets = MemoryBudgets {
        authoritative_bytes: 64,
        leased_bytes: 32,
        history_bytes: 32,
    };
    let mut m = TypedRecordMachine::new(budgets);
    let k1 = key(KeyPrefix::ClusterMeta, "a");
    m.apply(Command::Put {
        key: k1.clone(),
        expected: Expected::Absent,
        payload: vec![1u8; 40],
        leased: false,
    })
    .unwrap();
    assert_eq!(m.usage().authoritative_bytes, 40);

    let err = m
        .apply(Command::Put {
            key: key(KeyPrefix::ClusterMeta, "b"),
            expected: Expected::Absent,
            payload: vec![2u8; 40],
            leased: false,
        })
        .unwrap_err();
    assert!(matches!(
        err,
        ApplyError::Budget(gump_memory::BudgetError::Exhausted {
            class: BudgetClass::Authoritative,
            ..
        })
    ));
    // Existing authoritative state retained.
    assert_eq!(m.get(&k1).unwrap().payload.len(), 40);
    assert_eq!(m.usage().authoritative_bytes, 40);
}

#[test]
fn leased_bytes_use_leased_budget() {
    let budgets = MemoryBudgets {
        authoritative_bytes: 16,
        leased_bytes: 100,
        history_bytes: 16,
    };
    let mut m = TypedRecordMachine::new(budgets);
    m.apply(Command::Put {
        key: key(KeyPrefix::Placements, "p1"),
        expected: Expected::Absent,
        payload: vec![9u8; 50],
        leased: true,
    })
    .unwrap();
    assert_eq!(m.usage().leased_bytes, 50);
    assert_eq!(m.usage().authoritative_bytes, 0);
}

#[test]
fn history_class_uses_history_budget() {
    let budgets = MemoryBudgets {
        authoritative_bytes: 8,
        leased_bytes: 8,
        history_bytes: 64,
    };
    let mut m = TypedRecordMachine::new(budgets);
    m.apply(Command::Put {
        key: key(KeyPrefix::WorkloadsHistory, "w/1"),
        expected: Expected::Absent,
        payload: vec![7u8; 40],
        leased: false,
    })
    .unwrap();
    assert_eq!(m.usage().history_bytes, 40);
}

#[test]
fn property_put_idempotent_under_exact_digest() {
    // Property-style: for several payloads, ExactDigest put with same body is a no-op replace
    // that still advances revision when expected matches.
    let mut m = TypedRecordMachine::with_defaults();
    let k = key(KeyPrefix::Custody, "n1");
    for i in 0..16u8 {
        let payload = vec![i; 8];
        m.apply(Command::Put {
            key: k.clone(),
            expected: if i == 0 {
                Expected::Absent
            } else {
                Expected::Any
            },
            payload: payload.clone(),
            leased: true,
        })
        .unwrap();
        let dig = m.get(&k).unwrap().digest;
        let rev = m.revision();
        m.apply(Command::Put {
            key: k.clone(),
            expected: Expected::ExactDigest(dig),
            payload,
            leased: true,
        })
        .unwrap();
        assert!(m.revision() > rev);
        assert_eq!(m.get(&k).unwrap().digest, dig);
    }
}

#[test]
fn default_budgets_match_protocol() {
    let b = MemoryBudgets::default();
    assert_eq!(b.authoritative_bytes, 64 * 1024 * 1024);
    assert_eq!(b.leased_bytes, 32 * 1024 * 1024);
    assert_eq!(b.history_bytes, 32 * 1024 * 1024);
}
