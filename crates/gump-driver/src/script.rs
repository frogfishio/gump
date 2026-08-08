//! Script driver (`RUNTIME.md` §6) — native lifecycle plus explicit interpreter.

use crate::abi::DriverKind;
use crate::common::CommonDriver;

/// Script driver: interpreter argv is first-class; `/bin/sh -c` is never implied.
pub struct ScriptDriver {
    inner: CommonDriver,
}

impl ScriptDriver {
    pub fn new() -> Self {
        Self {
            inner: CommonDriver {
                kind: DriverKind::Script,
                supports_interpreter: true,
            },
        }
    }
}

impl Default for ScriptDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Driver for ScriptDriver {
    fn probe(
        &self,
        host: &crate::HostProbe,
    ) -> Result<crate::DriverCapabilities, crate::DriverError> {
        self.inner.probe(host)
    }

    fn prepare(
        &self,
        release: &crate::ReleaseRoot,
        runtime: &crate::RuntimeSpec,
        ctx: &crate::AttemptContext,
    ) -> Result<crate::PreparedHandle, crate::DriverError> {
        self.inner.prepare(release, runtime, ctx)
    }

    fn admit(
        &self,
        prepared: crate::PreparedHandle,
        grant: crate::ResourceGrant,
        secrets: crate::SecretPlan,
    ) -> Result<crate::Admission, crate::DriverError> {
        self.inner.admit(prepared, grant, secrets)
    }

    fn start(
        &self,
        admission: crate::Admission,
        fence: crate::StartFence,
        io: &crate::IoEndpoints,
    ) -> Result<crate::RunningHandle, crate::DriverError> {
        self.inner.start(admission, fence, io)
    }

    fn observe(
        &self,
        running: &mut crate::RunningHandle,
    ) -> Result<crate::Observation, crate::DriverError> {
        self.inner.observe(running)
    }

    fn signal(
        &self,
        running: &mut crate::RunningHandle,
        signal: crate::Signal,
    ) -> Result<(), crate::DriverError> {
        self.inner.signal(running, signal)
    }

    fn terminate(
        &self,
        running: &mut crate::RunningHandle,
        deadline: std::time::Duration,
    ) -> Result<(), crate::DriverError> {
        self.inner.terminate(running, deadline)
    }

    fn kill(&self, running: &mut crate::RunningHandle) -> Result<(), crate::DriverError> {
        self.inner.kill(running)
    }

    fn cleanup(&self, prepared: crate::PreparedHandle) -> Result<(), crate::DriverError> {
        self.inner.cleanup(prepared)
    }
}
