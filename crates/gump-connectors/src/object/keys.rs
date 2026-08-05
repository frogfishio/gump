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
