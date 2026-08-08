//! Scoped secret delivery for env / FD injection (RUNTIME §8 / S07 / GUMP-N009).
//!
//! Values are held only in process memory (`Secret`) and never written under
//! release or attempt roots.

use core::fmt;
use std::io::{Seek, SeekFrom, Write};

use gump_types::{AttemptId, CapsuleId, ClusterId, Secret, WorkloadId};

use crate::error::{DriverError, DriverErrorKind};

/// Exact authorization scope for one secret delivery (INV-013).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryScope {
    pub cluster_id: ClusterId,
    pub workload_id: WorkloadId,
    pub release_id: CapsuleId,
    pub unit: u32,
    pub attempt_id: AttemptId,
    pub node_id: u64,
    pub controller_epoch: u64,
    pub placement_fence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InjectForm {
    Env,
    Fd {
        fd: u16,
        reference_env: Option<String>,
    },
}

/// One declared runtime value ready for injection (not Clone — SECURITY §8).
pub struct SecretValue {
    pub logical_name: String,
    pub form: InjectForm,
    pub bytes: Secret<Vec<u8>>,
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretValue")
            .field("logical_name", &self.logical_name)
            .field("form", &self.form)
            .field("bytes", &self.bytes)
            .finish()
    }
}

/// Plan passed through admit → start. Deferred plans carry no plaintext values.
#[derive(Debug, Default)]
pub struct SecretPlan {
    /// Legacy F06 path: no secret material (tests / local run without S07).
    pub deferred: bool,
    pub scope: Option<DeliveryScope>,
    pub values: Vec<SecretValue>,
}

impl SecretPlan {
    pub fn deferred() -> Self {
        Self {
            deferred: true,
            scope: None,
            values: Vec::new(),
        }
    }

    pub fn scoped(scope: DeliveryScope, values: Vec<SecretValue>) -> Self {
        Self {
            deferred: false,
            scope: Some(scope),
            values,
        }
    }
}

pub(crate) fn validate_for_admit(
    plan: &SecretPlan,
    attempt_id: &AttemptId,
) -> Result<(), DriverError> {
    if plan.deferred {
        if !plan.values.is_empty() || plan.scope.is_some() {
            return Err(DriverError::new(
                DriverErrorKind::Policy,
                "deferred SecretPlan must not carry scope or values",
            ));
        }
        return Ok(());
    }
    let scope = plan.scope.as_ref().ok_or_else(|| {
        DriverError::new(
            DriverErrorKind::Policy,
            "non-deferred SecretPlan requires DeliveryScope",
        )
    })?;
    if &scope.attempt_id != attempt_id {
        return Err(DriverError::new(
            DriverErrorKind::Policy,
            "secret delivery attempt_id mismatch",
        ));
    }
    for v in &plan.values {
        validate_value(v)?;
    }
    Ok(())
}

pub(crate) fn validate_for_start(
    plan: &SecretPlan,
    fence_generation: u64,
) -> Result<(), DriverError> {
    if plan.deferred {
        return Ok(());
    }
    let scope = plan.scope.as_ref().ok_or_else(|| {
        DriverError::new(
            DriverErrorKind::Policy,
            "non-deferred SecretPlan requires DeliveryScope at start",
        )
    })?;
    if scope.placement_fence != fence_generation {
        return Err(DriverError::new(
            DriverErrorKind::Policy,
            "secret delivery placement fence mismatch",
        ));
    }
    Ok(())
}

fn validate_value(v: &SecretValue) -> Result<(), DriverError> {
    match &v.form {
        InjectForm::Env => {
            if v.logical_name.is_empty() || v.logical_name.bytes().any(|b| b == 0 || b == b'=') {
                return Err(DriverError::new(
                    DriverErrorKind::Policy,
                    "invalid env injection name",
                ));
            }
            let bytes = v.bytes.expose();
            if bytes.contains(&0) {
                return Err(DriverError::new(
                    DriverErrorKind::Policy,
                    "env injection forbids NUL bytes",
                ));
            }
            if std::str::from_utf8(bytes).is_err() {
                return Err(DriverError::new(
                    DriverErrorKind::Policy,
                    "env injection requires UTF-8",
                ));
            }
        }
        InjectForm::Fd { fd, reference_env } => {
            if *fd < 3 {
                return Err(DriverError::new(
                    DriverErrorKind::Policy,
                    "fd injection must use fd >= 3",
                ));
            }
            if let Some(name) = reference_env {
                if name.is_empty() || name.bytes().any(|b| b == 0 || b == b'=') {
                    return Err(DriverError::new(
                        DriverErrorKind::Policy,
                        "invalid fd reference_env name",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Anonymous (unlinked) file holding secret bytes for FD inheritance.
pub(crate) struct PreparedFd {
    pub target_fd: i32,
    pub file: std::fs::File,
    pub reference_env: Option<(String, String)>,
}

pub(crate) fn prepare_fds(plan: &SecretPlan) -> Result<Vec<PreparedFd>, DriverError> {
    let mut out = Vec::new();
    if plan.deferred {
        return Ok(out);
    }
    for v in &plan.values {
        if let InjectForm::Fd { fd, reference_env } = &v.form {
            let mut file = tempfile::tempfile().map_err(|e| {
                DriverError::new(DriverErrorKind::Start, format!("anon fd tempfile: {e}"))
            })?;
            file.write_all(v.bytes.expose()).map_err(|e| {
                DriverError::new(DriverErrorKind::Start, format!("anon fd write: {e}"))
            })?;
            file.flush().map_err(|e| {
                DriverError::new(DriverErrorKind::Start, format!("anon fd flush: {e}"))
            })?;
            file.seek(SeekFrom::Start(0)).map_err(|e| {
                DriverError::new(DriverErrorKind::Start, format!("anon fd rewind: {e}"))
            })?;
            let reference_env = reference_env
                .as_ref()
                .map(|name| (name.clone(), format!("/proc/self/fd/{fd}")));
            out.push(PreparedFd {
                target_fd: i32::from(*fd),
                file,
                reference_env,
            });
        }
    }
    Ok(out)
}
