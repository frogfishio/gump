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
pub enum FdReferenceValue {
    /// Export `/proc/self/fd/N` for consumers expecting a file name.
    ProcPath,
    /// Export `N` for consumers which take ownership of an inherited descriptor.
    DescriptorNumber,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InjectForm {
    Env,
    Fd {
        fd: u16,
        reference_env: Option<String>,
        reference_value: FdReferenceValue,
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
        InjectForm::Fd {
            fd, reference_env, ..
        } => {
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

/// Anonymous memory-backed file holding secret bytes for FD inheritance.
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
        if let InjectForm::Fd {
            fd,
            reference_env,
            reference_value,
        } = &v.form
        {
            let mut file = anonymous_secret_file()?;
            file.write_all(v.bytes.expose()).map_err(|e| {
                DriverError::new(DriverErrorKind::Start, format!("anon fd write: {e}"))
            })?;
            file.flush().map_err(|e| {
                DriverError::new(DriverErrorKind::Start, format!("anon fd flush: {e}"))
            })?;
            file.seek(SeekFrom::Start(0)).map_err(|e| {
                DriverError::new(DriverErrorKind::Start, format!("anon fd rewind: {e}"))
            })?;
            seal_secret_file(&file)?;
            let reference_env = reference_env.as_ref().map(|name| {
                let value = match reference_value {
                    FdReferenceValue::ProcPath => format!("/proc/self/fd/{fd}"),
                    FdReferenceValue::DescriptorNumber => fd.to_string(),
                };
                (name.clone(), value)
            });
            out.push(PreparedFd {
                target_fd: i32::from(*fd),
                file,
                reference_env,
            });
        }
    }
    Ok(out)
}

#[cfg(target_os = "linux")]
fn anonymous_secret_file() -> Result<std::fs::File, DriverError> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd;

    let name = CString::new("gump-secret").expect("static memfd name");
    // SAFETY: `memfd_create` returns a new owned descriptor or -1. The descriptor
    // is immediately wrapped in `File`; no pathname is ever created.
    #[allow(unsafe_code)]
    let fd = unsafe {
        libc::syscall(
            libc::SYS_memfd_create,
            name.as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        ) as i32
    };
    if fd < 0 {
        return Err(DriverError::new(
            DriverErrorKind::Start,
            format!("create secret memfd: {}", std::io::Error::last_os_error()),
        ));
    }
    // SAFETY: ownership of the newly-created descriptor transfers to `File`.
    #[allow(unsafe_code)]
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

#[cfg(target_os = "linux")]
fn seal_secret_file(file: &std::fs::File) -> Result<(), DriverError> {
    use std::os::fd::AsRawFd;

    let seals = libc::F_SEAL_SEAL | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE;
    // SAFETY: `file` owns a valid memfd and `F_ADD_SEALS` takes an integer bitmask.
    #[allow(unsafe_code)]
    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, seals) };
    if result < 0 {
        return Err(DriverError::new(
            DriverErrorKind::Start,
            format!("seal secret memfd: {}", std::io::Error::last_os_error()),
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn anonymous_secret_file() -> Result<std::fs::File, DriverError> {
    // macOS has no memfd_create. `tempfile()` creates mode-0600 storage and
    // unlinks it immediately; this is local-development compatibility only.
    // Production Linux fails closed through the sealed-memfd path above.
    tempfile::tempfile().map_err(|e| {
        DriverError::new(
            DriverErrorKind::Start,
            format!("create unlinked secret descriptor: {e}"),
        )
    })
}

#[cfg(target_os = "macos")]
fn seal_secret_file(_file: &std::fs::File) -> Result<(), DriverError> {
    // POSIX shared memory has no Linux-style write seals. The object was
    // unlinked before bytes were written and remains reachable only by FD.
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn anonymous_secret_file() -> Result<std::fs::File, DriverError> {
    Err(DriverError::new(
        DriverErrorKind::Start,
        "memory-only FD secret injection is unsupported on this platform",
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn seal_secret_file(_file: &std::fs::File) -> Result<(), DriverError> {
    unreachable!("anonymous_secret_file fails on unsupported platforms")
}
