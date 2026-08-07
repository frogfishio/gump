//! Local unsealed `gump run` path (D014 / CONFORMANCE §6).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gump_capsule::archive::{
    ArchiveEntry, ExtractLimits, materialize_application_archive, pack_archive,
};
use gump_driver::{
    AttemptContext, Driver, DriverKind, HostProbe, IoEndpoints, NativeDriver, ReleaseRoot,
    ResourceGrant, RuntimeSpec, ScriptDriver, SecretPlan, StartFence,
};
use gump_manifest::capture::{
    CapturePlan, VirtualTree, apply_prepare_outputs, capture_workspace, verify_captured_bytes,
};
use gump_manifest::{Driver as ManifestDriver, Manifest, parse_manifest_str};
use gump_types::{AttemptId, CapsuleId};

use crate::error::{CliError, CliErrorKind};

#[derive(Clone, Debug)]
pub struct LocalRunOptions {
    pub workspace: PathBuf,
    pub manifest_path: PathBuf,
    pub state_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalRunReport {
    pub mode: &'static str,
    pub namespace: String,
    pub app_id: String,
    pub capsule_id: String,
    pub release_root: PathBuf,
    pub command_vector: Vec<String>,
    pub workdir: PathBuf,
    pub telemetry_filter: Option<String>,
    pub exit_code: Option<i32>,
}

/// Normalized local parity plan shared by `run` and sealed `test`.
#[derive(Clone, Debug)]
pub struct LocalParityPlan {
    pub manifest: Manifest,
    pub archive: Vec<u8>,
    pub command_vector: Vec<String>,
    pub workdir_rel: Option<String>,
    pub driver_kind: DriverKind,
    pub interpreter: Option<Vec<String>>,
    pub telemetry_filter: Option<String>,
}

pub fn local_parity_plan(
    workspace: &Path,
    manifest_path: &Path,
) -> Result<LocalParityPlan, CliError> {
    let manifest_abs = if manifest_path.is_absolute() {
        manifest_path.to_path_buf()
    } else {
        workspace.join(manifest_path)
    };
    let text = fs::read_to_string(&manifest_abs).map_err(|e| {
        CliError::new(
            CliErrorKind::Io,
            format!("read manifest {}: {e}", manifest_abs.display()),
        )
    })?;
    let manifest = parse_manifest_str(&text)
        .map_err(|e| CliError::new(CliErrorKind::Manifest, e.to_string()))?;

    let package_root = workspace.join(&manifest.package.root);
    let plan = CapturePlan::from_package(&manifest.package)
        .map_err(|e| CliError::new(CliErrorKind::Capture, e.to_string()))?;
    let mut tree = capture_workspace(&package_root, &plan)
        .map_err(|e| CliError::new(CliErrorKind::Capture, e.to_string()))?;
    if let Some(prepare) = &manifest.prepare {
        // F07: prepare outputs must already be staged by the caller/tooling;
        // we only merge declared outputs if a staging dir exists.
        let staging = workspace.join(".gump").join("prepare-staging");
        if staging.is_dir() {
            apply_prepare_outputs(
                &package_root,
                &mut tree,
                &staging,
                &prepare.outputs,
                manifest.package.allow_sensitive_files,
            )
            .map_err(|e| CliError::new(CliErrorKind::Capture, e.to_string()))?;
        }
    }

    let entries = virtual_tree_to_archive_entries(&tree)?;
    let archive =
        pack_archive(&entries).map_err(|e| CliError::new(CliErrorKind::Archive, e.to_string()))?;

    let (driver_kind, interpreter) = match manifest.runtime.driver {
        ManifestDriver::Native => (DriverKind::Native, None),
        ManifestDriver::Script => (
            DriverKind::Script,
            Some(manifest.runtime.interpreter.clone().ok_or_else(|| {
                CliError::new(
                    CliErrorKind::Policy,
                    "script driver requires runtime.interpreter",
                )
            })?),
        ),
        ManifestDriver::Oci => {
            return Err(CliError::new(
                CliErrorKind::Policy,
                "OCI driver is not part of F07 local parity",
            ));
        }
    };

    let command_vector = manifest
        .runtime
        .command
        .iter()
        .map(|c| c.trim_start_matches("./").to_string())
        .collect();

    let workdir_rel = manifest.runtime.workdir.as_ref().and_then(|w| {
        let w = w.trim_start_matches("./");
        if w.is_empty() || w == "." {
            None
        } else {
            Some(w.to_string())
        }
    });

    Ok(LocalParityPlan {
        telemetry_filter: manifest.telemetry.as_ref().and_then(|t| t.filter.clone()),
        manifest,
        archive,
        command_vector,
        workdir_rel,
        driver_kind,
        interpreter,
    })
}

pub fn run_local(opts: LocalRunOptions) -> Result<LocalRunReport, CliError> {
    let plan = local_parity_plan(&opts.workspace, &opts.manifest_path)?;
    execute_plan(
        &opts.workspace,
        opts.state_root,
        "run",
        &plan,
        &plan.archive,
    )
}

pub(crate) fn execute_plan(
    workspace: &Path,
    state_root: Option<PathBuf>,
    mode: &'static str,
    plan: &LocalParityPlan,
    archive: &[u8],
) -> Result<LocalRunReport, CliError> {
    let state = state_root.unwrap_or_else(|| workspace.join(".gump").join("state"));
    let capsule_id = CapsuleId::new();
    let mat =
        materialize_application_archive(&state, capsule_id, archive, &ExtractLimits::default())
            .map_err(|e| CliError::new(CliErrorKind::Archive, e.to_string()))?;

    let release = ReleaseRoot::new(&mat.root);
    let attempt_root = state
        .join("attempts")
        .join(AttemptId::new().to_hyphenated());
    fs::create_dir_all(&attempt_root)
        .map_err(|e| CliError::new(CliErrorKind::Io, e.to_string()))?;

    let runtime = RuntimeSpec {
        kind: plan.driver_kind,
        command: plan.command_vector.clone(),
        interpreter: plan.interpreter.clone(),
        workdir: plan.workdir_rel.clone(),
    };
    let ctx = AttemptContext {
        attempt_id: AttemptId::new(),
        attempt_root: attempt_root.clone(),
    };

    let exit_code = match plan.driver_kind {
        DriverKind::Native => drive(&NativeDriver::new(), &release, &runtime, &ctx)?,
        DriverKind::Script => drive(&ScriptDriver::new(), &release, &runtime, &ctx)?,
    };

    let workdir = match &plan.workdir_rel {
        Some(rel) => mat.root.join(rel),
        None => mat.root.clone(),
    };

    Ok(LocalRunReport {
        mode,
        namespace: plan.manifest.app.namespace.to_string(),
        app_id: plan.manifest.app.id.to_string(),
        capsule_id: capsule_id.to_hyphenated(),
        release_root: mat.root,
        command_vector: effective_argv(plan),
        workdir,
        telemetry_filter: plan.telemetry_filter.clone(),
        exit_code,
    })
}

fn drive<D: Driver>(
    driver: &D,
    release: &ReleaseRoot,
    runtime: &RuntimeSpec,
    ctx: &AttemptContext,
) -> Result<Option<i32>, CliError> {
    let _caps = driver
        .probe(&HostProbe {
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
        })
        .map_err(|e| CliError::new(CliErrorKind::Driver, e.to_string()))?;
    let prepared = driver
        .prepare(release, runtime, ctx)
        .map_err(|e| CliError::new(CliErrorKind::Driver, e.to_string()))?;
    let admission = driver
        .admit(
            prepared,
            ResourceGrant {
                max_processes: Some(64),
            },
            &SecretPlan { deferred: true },
        )
        .map_err(|e| CliError::new(CliErrorKind::Driver, e.to_string()))?;
    let mut running = driver
        .start(
            admission,
            StartFence { generation: 1 },
            &IoEndpoints {
                capture_stdout: true,
                capture_stderr: true,
            },
        )
        .map_err(|e| CliError::new(CliErrorKind::Driver, e.to_string()))?;
    // Finite local run: wait for exit. Pipe drains live in the driver (STL-04);
    // do not hard-kill after a fixed wall clock — continuous workloads belong
    // to the agent supervisor loop, not `gump run`.
    loop {
        let obs = driver
            .observe(&mut running)
            .map_err(|e| CliError::new(CliErrorKind::Driver, e.to_string()))?;
        if !obs.running {
            return Ok(obs.exit_code);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn effective_argv(plan: &LocalParityPlan) -> Vec<String> {
    match &plan.interpreter {
        Some(interp) => {
            let mut v = interp.clone();
            v.extend(plan.command_vector.iter().cloned());
            v
        }
        None => plan.command_vector.clone(),
    }
}

fn virtual_tree_to_archive_entries(tree: &VirtualTree) -> Result<Vec<ArchiveEntry>, CliError> {
    let mut entries = Vec::new();
    let mut dirs = std::collections::BTreeSet::new();
    for rel in tree.paths() {
        let mut parent = Path::new(rel);
        while let Some(p) = parent.parent() {
            if p.as_os_str().is_empty() {
                break;
            }
            dirs.insert(p.to_string_lossy().replace('\\', "/"));
            parent = p;
        }
        let entry = tree.get(rel).expect("path from iterator");
        // STL-05: pack retained capture bytes only — never re-open source_path.
        verify_captured_bytes(entry)
            .map_err(|e| CliError::new(CliErrorKind::Capture, e.to_string()))?;
        let bytes = entry.bytes.clone();
        let executable = entry.executable;
        entries.push(
            ArchiveEntry::file(rel, bytes, executable)
                .map_err(|e| CliError::new(CliErrorKind::Archive, e.to_string()))?,
        );
    }
    for d in dirs {
        entries.push(
            ArchiveEntry::directory(d)
                .map_err(|e| CliError::new(CliErrorKind::Archive, e.to_string()))?,
        );
    }
    Ok(entries)
}
