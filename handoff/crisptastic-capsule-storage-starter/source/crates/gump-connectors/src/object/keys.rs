//! Canonical object key helpers (DECISIONS D008).

use gump_types::{CapsuleId, ClusterId};

use super::types::{ObjectKey, ObjectStoreError};

/// Final immutable Capsule key:
/// `clusters/<cluster-id>/capsules/<capsule-id>.capsule`
pub fn final_capsule_key(
    cluster: ClusterId,
    capsule: CapsuleId,
) -> Result<ObjectKey, ObjectStoreError> {
    ObjectKey::new(format!(
        "clusters/{}/capsules/{}.capsule",
        cluster.to_hyphenated(),
        capsule.to_hyphenated()
    ))
}

/// True when `key` is a final Capsule object (not quarantine).
pub fn is_final_capsule_key(key: &ObjectKey) -> bool {
    let s = key.as_str();
    let Some((_, rest)) = s.split_once("clusters/") else {
        return false;
    };
    let Some((_, capsule_part)) = rest.split_once("/capsules/") else {
        return false;
    };
    capsule_part.ends_with(".capsule") && !capsule_part.contains('/')
}

/// Parse `(cluster, capsule)` from a final Capsule object key.
pub fn parse_final_capsule_key(key: &ObjectKey) -> Option<(ClusterId, CapsuleId)> {
    let s = key.as_str();
    let rest = s.strip_prefix("clusters/")?;
    let (cluster_s, after) = rest.split_once("/capsules/")?;
    let capsule_s = after.strip_suffix(".capsule")?;
    if capsule_s.contains('/') {
        return None;
    }
    let cluster = cluster_s.parse().ok()?;
    let capsule = capsule_s.parse().ok()?;
    Some((cluster, capsule))
}

/// Non-authoritative quarantine key for an in-flight upload.
pub fn quarantine_key(
    cluster: ClusterId,
    capsule: CapsuleId,
    upload: u64,
) -> Result<ObjectKey, ObjectStoreError> {
    ObjectKey::new(format!(
        "clusters/{}/quarantine/{}/{}.capsule",
        cluster.to_hyphenated(),
        capsule.to_hyphenated(),
        upload
    ))
}
