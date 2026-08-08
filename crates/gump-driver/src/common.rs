//! Shared prepare/start helpers for native and script drivers.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::Driver;
use crate::abi::{
    Admission, AttemptContext, DRIVER_ABI, DriverCapabilities, DriverKind, HostProbe, IoEndpoints,
    Observation, PreparedHandle, ReleaseRoot, ResourceGrant, RunningHandle, RuntimeSpec,
    SecretPlan, Signal, StartFence,
};
use crate::error::{DriverError, DriverErrorKind};
use crate::path_beneath::{self, PathKind};
use crate::supervisor::{self, PipeDrains};

pub(crate) struct CommonDriver {
    pub kind: DriverKind,
    pub supports_interpreter: bool,
}

impl CommonDriver {
    fn capabilities(&self) -> DriverCapabilities {
        DriverCapabilities {
            kind: self.kind,
            abi: DRIVER_ABI,
            supports_process_group: cfg!(unix),
            supports_interpreter: self.supports_interpreter,
        }
    }

    fn resolve_argv(&self, runtime: &RuntimeSpec) -> Result<Vec<String>, DriverError> {
        if runtime.command.is_empty() {
            return Err(DriverError::new(
                DriverErrorKind::Prepare,
                "runtime.command must be non-empty",
            ));
        }
        match self.kind {
            DriverKind::Native => {
                if runtime.interpreter.is_some() {
                    return Err(DriverError::new(
                        DriverErrorKind::Policy,
                        "native driver rejects interpreter argv",
                    ));
                }
                if runtime.kind != DriverKind::Native {
                    return Err(DriverError::new(
                        DriverErrorKind::Policy,
                        "runtime.kind is not native",
                    ));
                }
                if runtime.command[0].is_empty() {
                    return Err(DriverError::new(
                        DriverErrorKind::Prepare,
                        "empty command[0]",
                    ));
                }
                Ok(runtime.command.clone())
            }
            DriverKind::Script => {
                if runtime.kind != DriverKind::Script {
                    return Err(DriverError::new(
                        DriverErrorKind::Policy,
                        "runtime.kind is not script",
                    ));
                }
                let interp = runtime.interpreter.as_ref().ok_or_else(|| {
                    DriverError::new(
                        DriverErrorKind::Prepare,
                        "script driver requires explicit interpreter argv",
                    )
                })?;
                if interp.is_empty() {
                    return Err(DriverError::new(
                        DriverErrorKind::Prepare,
                        "interpreter argv must be non-empty",
                    ));
                }
                if interp.len() >= 2 && interp[1] == "-c" {
                    return Err(DriverError::new(
                        DriverErrorKind::Policy,
                        "implicit shell -c is forbidden",
                    ));
                }
                let mut argv = interp.clone();
                argv.extend(runtime.command.iter().cloned());
                Ok(argv)
            }
        }
    }
}

impl Driver for CommonDriver {
    fn probe(&self, _host: &HostProbe) -> Result<DriverCapabilities, DriverError> {
        Ok(self.capabilities())
    }

    fn prepare(
        &self,
        release: &ReleaseRoot,
        runtime: &RuntimeSpec,
        ctx: &AttemptContext,
    ) -> Result<PreparedHandle, DriverError> {
        if !release.as_path().is_dir() {
            return Err(DriverError::new(
                DriverErrorKind::Prepare,
                "release root is not a directory",
            ));
        }
        ensure_empty_attempt_root(&ctx.attempt_root)?;
        let argv = self.resolve_argv(runtime)?;
        let workdir = match &runtime.workdir {
            Some(rel) => path_beneath::resolve_beneath(release.as_path(), rel, PathKind::Dir)?,
            None => release.as_path().to_path_buf(),
        };
        // Release root must be a real directory (not a symlink) before spawn (N001).
        path_beneath::assert_owned_cleanup_target(release.as_path()).map_err(|e| {
            DriverError::new(
                DriverErrorKind::Policy,
                format!("release root rejected: {}", e.message()),
            )
        })?;
        let argv = pin_release_paths(release.as_path(), runtime, argv, self.kind)?;
        Ok(PreparedHandle {
            attempt_id: ctx.attempt_id,
            attempt_root: ctx.attempt_root.clone(),
            release_root: release.path.clone(),
            argv,
            workdir,
            admitted: false,
        })
    }

    fn admit(
        &self,
        mut prepared: PreparedHandle,
        grant: ResourceGrant,
        secrets: &SecretPlan,
    ) -> Result<Admission, DriverError> {
        if prepared.admitted {
            return Err(DriverError::new(DriverErrorKind::State, "already admitted"));
        }
        if !secrets.deferred {
            return Err(DriverError::new(
                DriverErrorKind::Policy,
                "F06 drivers require SecretPlan.deferred=true until S07",
            ));
        }
        prepared.admitted = true;
        Ok(Admission { prepared, grant })
    }

    fn start(
        &self,
        admission: Admission,
        fence: StartFence,
        io: &IoEndpoints,
    ) -> Result<RunningHandle, DriverError> {
        if !admission.prepared.admitted {
            return Err(DriverError::new(
                DriverErrorKind::State,
                "start without admission",
            ));
        }
        let mut cmd = Command::new(&admission.prepared.argv[0]);
        if admission.prepared.argv.len() > 1 {
            cmd.args(&admission.prepared.argv[1..]);
        }
        cmd.current_dir(&admission.prepared.workdir);
        // Re-validate containment at start (narrow prepare→spawn TOCTOU; N001).
        revalidate_prepared_paths(&admission.prepared)?;
        cmd.env_clear();
        cmd.env(
            "GUMP_ATTEMPT_ID",
            admission.prepared.attempt_id.to_hyphenated(),
        );
        cmd.env(
            "GUMP_RELEASE_ROOT",
            admission.prepared.release_root.display().to_string(),
        );
        cmd.env(
            "GUMP_ATTEMPT_ROOT",
            admission.prepared.attempt_root.display().to_string(),
        );
        if let Some(max) = admission.grant.max_processes {
            cmd.env("GUMP_MAX_PROCESSES", max.to_string());
        }
        // Preserve PATH for host interpreter resolution (script driver).
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
        cmd.stdin(Stdio::null());
        cmd.stdout(if io.capture_stdout {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stderr(if io.capture_stderr {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| DriverError::new(DriverErrorKind::Start, format!("spawn failed: {e}")))?;
        // RUNTIME §9: start drains before the child can fill pipe buffers.
        let drains = PipeDrains::start_with(&mut child, io.pipe_sink.clone());
        Ok(RunningHandle {
            prepared: admission.prepared,
            child: Some(child),
            drains: Some(drains),
            fence,
            last_exit_code: None,
        })
    }

    fn observe(&self, running: &mut RunningHandle) -> Result<Observation, DriverError> {
        let Some(child) = running.child.as_mut() else {
            return Ok(Observation {
                running: false,
                exit_code: running.last_exit_code,
            });
        };
        match child.try_wait()? {
            Some(status) => {
                // STL-22: terminal observation finalizes the whole process tree.
                // Order: preserve primary exit → kill/reap group → bounded drain → clear handle.
                let code = status.code();
                running.finalize_terminal(Some(code));
                Ok(Observation {
                    running: false,
                    exit_code: code,
                })
            }
            None => Ok(Observation {
                running: true,
                exit_code: None,
            }),
        }
    }

    fn signal(&self, running: &mut RunningHandle, signal: Signal) -> Result<(), DriverError> {
        let Some(child) = running.child.as_mut() else {
            return Err(DriverError::new(
                DriverErrorKind::Signal,
                "no running child",
            ));
        };
        supervisor::signal_tree(child, signal)
    }

    fn terminate(
        &self,
        running: &mut RunningHandle,
        deadline: Duration,
    ) -> Result<(), DriverError> {
        // RUNTIME §16: graceful signal → drain/wait → kill process tree.
        self.signal(running, Signal::Term)?;
        let start = Instant::now();
        while start.elapsed() < deadline {
            let obs = self.observe(running)?;
            if !obs.running {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.kill(running)
    }

    fn kill(&self, running: &mut RunningHandle) -> Result<(), DriverError> {
        // Same containment path as terminal observe (STL-22).
        running.finalize_terminal(None);
        Ok(())
    }

    fn cleanup(&self, prepared: PreparedHandle) -> Result<(), DriverError> {
        if prepared.attempt_root.exists() {
            path_beneath::assert_owned_cleanup_target(&prepared.attempt_root)?;
            fs::remove_dir_all(&prepared.attempt_root)?;
        }
        Ok(())
    }
}

fn ensure_empty_attempt_root(attempt_root: &Path) -> Result<(), DriverError> {
    // Refuse `..` and symlinks before create_dir_all (which would follow a link).
    for c in attempt_root.components() {
        if matches!(c, std::path::Component::ParentDir) {
            return Err(DriverError::new(
                DriverErrorKind::Policy,
                format!(
                    "attempt root parent traversal rejected: {}",
                    attempt_root.display()
                ),
            ));
        }
    }
    match fs::symlink_metadata(attempt_root) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(DriverError::new(
                DriverErrorKind::Policy,
                format!(
                    "attempt root must not be a symlink: {}",
                    attempt_root.display()
                ),
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(attempt_root)?;
        }
        Err(e) => {
            return Err(DriverError::new(
                DriverErrorKind::Io,
                format!("stat attempt root {}: {e}", attempt_root.display()),
            ));
        }
    }
    path_beneath::assert_owned_cleanup_target(attempt_root)?;
    let mut entries = fs::read_dir(attempt_root)?;
    if entries.next().is_some() {
        return Err(DriverError::new(
            DriverErrorKind::Prepare,
            "attempt root must be empty before prepare",
        ));
    }
    Ok(())
}

/// Resolve release-relative command/script to absolute nofollow paths (N001).
fn pin_release_paths(
    release: &Path,
    runtime: &RuntimeSpec,
    mut argv: Vec<String>,
    kind: DriverKind,
) -> Result<Vec<String>, DriverError> {
    let map_not_found = |rel: &str, e: DriverError| -> DriverError {
        if e.kind() == DriverErrorKind::NotFound {
            DriverError::new(
                DriverErrorKind::NotFound,
                format!("path not found under release root: {rel}"),
            )
        } else {
            e
        }
    };
    match kind {
        DriverKind::Native => {
            let primary = &runtime.command[0];
            let resolved = path_beneath::resolve_beneath(release, primary, PathKind::File)
                .map_err(|e| map_not_found(primary, e))?;
            argv[0] = resolved.display().to_string();
        }
        DriverKind::Script => {
            let script = &runtime.command[0];
            let resolved = path_beneath::resolve_beneath(release, script, PathKind::File)
                .map_err(|e| map_not_found(script, e))?;
            // argv = interpreter… + command; pin the first command element.
            let interp_len = runtime.interpreter.as_ref().map(|i| i.len()).unwrap_or(0);
            if argv.len() <= interp_len {
                return Err(DriverError::new(
                    DriverErrorKind::Prepare,
                    "script argv missing release-relative command",
                ));
            }
            argv[interp_len] = resolved.display().to_string();
        }
    }
    Ok(argv)
}

fn revalidate_prepared_paths(prepared: &PreparedHandle) -> Result<(), DriverError> {
    path_beneath::assert_owned_cleanup_target(&prepared.release_root).map_err(|e| {
        DriverError::new(
            DriverErrorKind::Policy,
            format!("release root rejected at start: {}", e.message()),
        )
    })?;
    path_beneath::assert_owned_cleanup_target(&prepared.workdir).map_err(|e| {
        DriverError::new(
            DriverErrorKind::Policy,
            format!("workdir rejected at start: {}", e.message()),
        )
    })?;
    // Pinned native binary / script path must still be a non-symlink file.
    for (i, arg) in prepared.argv.iter().enumerate() {
        let p = Path::new(arg);
        if !p.is_absolute() {
            continue;
        }
        if !arg.starts_with(prepared.release_root.to_str().unwrap_or("\0")) {
            continue;
        }
        let meta = fs::symlink_metadata(p).map_err(|e| {
            DriverError::new(
                DriverErrorKind::Policy,
                format!("release path vanished before start (argv[{i}]): {e}"),
            )
        })?;
        if meta.file_type().is_symlink() {
            return Err(DriverError::new(
                DriverErrorKind::Policy,
                format!("release path became a symlink before start (argv[{i}])"),
            ));
        }
    }
    Ok(())
}
