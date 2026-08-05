//! C06 exit evidence: learner transfer and joint-change suite.
//!
//! Authority: docs/v1/DELIVERY.md C06, PROTOCOL.md §14, DECISIONS D006,
//! CONFORMANCE joiner-crash / joint-membership intersection.

use std::collections::BTreeSet;

use gump_memory::{
    can_commit_joint, ClusterIncarnation, JointConfig, MemberPhase, MembershipCluster,
    MembershipError, SnapshotOffer,
};

fn avail(ids: &[u64]) -> BTreeSet<u64> {
    ids.iter().copied().collect()
}

fn join_and_promote(c: &mut MembershipCluster, id: MemberId) {
    let offer = SnapshotOffer::from_bytes(id, format!("snap-{id}").into_bytes());
    c.begin_join(id, id, offer.digest).unwrap();
    c.complete_transfer(id, &offer).unwrap();
    c.begin_promote(id).unwrap();
    let mut available = c.voters().clone();
    available.insert(id);
    c.commit_joint(&available).unwrap();
}

type MemberId = u64;

#[test]
fn init_seed_is_sole_voter() {
    let mut c = MembershipCluster::new();
    c.init(1, ClusterIncarnation::new(1)).unwrap();
    assert_eq!(c.voters().iter().copied().collect::<Vec<_>>(), vec![1]);
    assert!(c.can_vote(1));
    assert!(!c.can_vote(2));
}

#[test]
fn joiner_is_non_voting_during_transfer() {
    let mut c = MembershipCluster::new();
    c.init(1, ClusterIncarnation::new(1)).unwrap();
    let offer = SnapshotOffer::from_bytes(10, b"ram-state".to_vec());
    c.begin_join(2, offer.committed_index, offer.digest)
        .unwrap();
    assert_eq!(c.member(2).unwrap().phase, MemberPhase::Transferring);
    assert!(!c.can_vote(2));
    assert!(c.learners().contains(&2));
    assert!(!c.voters().contains(&2));
}

#[test]
fn joiner_crash_during_transfer_never_votes() {
    let mut c = MembershipCluster::new();
    c.init(1, ClusterIncarnation::new(1)).unwrap();
    let offer = SnapshotOffer::from_bytes(3, b"snap".to_vec());
    c.begin_join(9, offer.committed_index, offer.digest)
        .unwrap();
    c.abort_transfer(9).unwrap();
    assert!(c.member(9).is_none());
    assert!(!c.can_vote(9));
    assert!(!c.learners().contains(&9));
    assert!(!c.voters().contains(&9));
}

#[test]
fn transfer_verifies_digest_and_index_then_learner() {
    let mut c = MembershipCluster::new();
    c.init(1, ClusterIncarnation::new(1)).unwrap();
    let offer = SnapshotOffer::from_bytes(42, b"memory-blob".to_vec());
    c.begin_join(2, 42, offer.digest).unwrap();
    c.complete_transfer(2, &offer).unwrap();
    assert_eq!(c.member(2).unwrap().phase, MemberPhase::Learner);
    assert!(!c.can_vote(2));

    let bad = SnapshotOffer {
        committed_index: 42,
        digest: offer.digest,
        bytes: b"tampered".to_vec(),
    };
    let mut c2 = MembershipCluster::new();
    c2.init(1, ClusterIncarnation::new(1)).unwrap();
    c2.begin_join(2, 42, offer.digest).unwrap();
    assert!(matches!(
        c2.complete_transfer(2, &bad),
        Err(MembershipError::Snapshot(_))
    ));
}

#[test]
fn promote_via_joint_change() {
    let mut c = MembershipCluster::new();
    c.init(1, ClusterIncarnation::new(1)).unwrap();
    let offer = SnapshotOffer::from_bytes(1, b"s".to_vec());
    c.begin_join(2, 1, offer.digest).unwrap();
    c.complete_transfer(2, &offer).unwrap();
    c.begin_promote(2).unwrap();

    let joint = c.joint().unwrap().clone();
    assert_eq!(joint.old_voters, BTreeSet::from([1]));
    assert_eq!(joint.new_voters, BTreeSet::from([1, 2]));
    assert!(!c.can_vote(2));

    // maj(old)=1 satisfied by seed; maj(new)=2 needs both.
    assert!(matches!(
        c.commit_joint(&avail(&[1])),
        Err(MembershipError::JointQuorumNotMet)
    ));
    c.commit_joint(&avail(&[1, 2])).unwrap();
    assert!(c.can_vote(2));
    assert_eq!(c.voters(), &BTreeSet::from([1, 2]));
    assert!(c.joint().is_none());
}

#[test]
fn joint_membership_intersection_requires_both_majorities() {
    let joint = JointConfig::new(BTreeSet::from([1, 2, 3]), BTreeSet::from([2, 3, 4])).unwrap();
    assert!(!can_commit_joint(&joint, &avail(&[1, 4])).unwrap());
    assert!(can_commit_joint(&joint, &avail(&[2, 3])).unwrap());
    assert_eq!(joint.intersection(), BTreeSet::from([2, 3]));
}

#[test]
fn drain_and_remove_via_joint() {
    let mut c = MembershipCluster::new();
    c.init(1, ClusterIncarnation::new(1)).unwrap();
    join_and_promote(&mut c, 2);
    join_and_promote(&mut c, 3);
    assert_eq!(c.voters(), &BTreeSet::from([1, 2, 3]));

    c.begin_drain(2).unwrap();
    assert_eq!(c.member(2).unwrap().phase, MemberPhase::Draining);
    assert!(matches!(
        c.commit_joint(&avail(&[1])),
        Err(MembershipError::JointQuorumNotMet)
    ));
    c.commit_joint(&avail(&[1, 3])).unwrap();
    assert!(!c.voters().contains(&2));
    assert!(c.member(2).is_none());
}

#[test]
fn cannot_drain_last_voter() {
    let mut c = MembershipCluster::new();
    c.init(1, ClusterIncarnation::new(1)).unwrap();
    assert!(matches!(
        c.begin_drain(1),
        Err(MembershipError::LastVoter)
    ));
}

#[test]
fn replayed_join_rejected() {
    let mut c = MembershipCluster::new();
    c.init(1, ClusterIncarnation::new(1)).unwrap();
    let offer = SnapshotOffer::from_bytes(1, b"x".to_vec());
    c.begin_join(2, 1, offer.digest).unwrap();
    assert!(matches!(
        c.begin_join(2, 1, offer.digest),
        Err(MembershipError::MemberExists(2))
    ));
}
