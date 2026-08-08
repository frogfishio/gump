//! Developer-process packaging of public variable contracts + protected values
//! (FORMATS §5–§7 / F05 / GUMP-N007).
//!
//! Values are resolved only here (env / local override). Public metadata carries
//! names and contracts; ciphertext alone carries value bytes.

use std::collections::BTreeMap;

use gump_manifest::{Classification, Encoding, Inject, Manifest, Variable};
use gump_protocol::pb::{
    AppIdentityV1, InjectionKind, ProtectedConfigV1, ProtectedValueV1, ReleaseMetadataV1,
    RuntimeVariableV1, ValueClassification, ValueEncoding,
};
use gump_types::{CapsuleId, ClusterId, Secret};
use prost::Message;

use crate::error::{CliError, CliErrorKind};

/// Packaged Capsule configuration segments ready for AEAD + signing.
#[derive(Debug)]
pub struct PackagedConfig {
    /// Canonical `ReleaseMetadataV1` protobuf (no protected values).
    pub public_metadata: Vec<u8>,
    /// Canonical `ProtectedConfigV1` protobuf plaintext (Secret-wrapped).
    pub protected_plaintext: Secret<Vec<u8>>,
}

/// Resolve manifest variables and encode public + protected packaging records.
///
/// Fail-closed before Capsule publish: required unset, oversize, malformed
/// source, unknown/unsupported provider schemes.
pub fn package_release_config(
    manifest: &Manifest,
    capsule_id: CapsuleId,
    cluster_id: ClusterId,
) -> Result<PackagedConfig, CliError> {
    package_release_config_with_env(manifest, capsule_id, cluster_id, &BTreeMap::new())
}

/// Same as [`package_release_config`], with optional in-process env overrides
/// (tests / controlled packaging without mutating process environment).
pub fn package_release_config_with_env(
    manifest: &Manifest,
    capsule_id: CapsuleId,
    cluster_id: ClusterId,
    env_overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<PackagedConfig, CliError> {
    let mut runtime_variables = Vec::new();
    let mut protected_values = Vec::new();

    for (name, var) in &manifest.runtime.variables {
        runtime_variables.push(to_runtime_variable(name, var)?);
        if let Some(value) = resolve_variable(manifest, name, var, env_overrides)? {
            let bytes = value.expose().clone();
            protected_values.push(ProtectedValueV1 {
                logical_name: name.clone(),
                classification: classification_wire(var.classification) as i32,
                encoding: encoding_wire(var.encoding) as i32,
                injection: injection_wire(var.inject) as i32,
                present: true,
                value: bytes,
            });
        } else {
            // Optional unset: record absence with empty bytes (FORMATS §7).
            protected_values.push(ProtectedValueV1 {
                logical_name: name.clone(),
                classification: classification_wire(var.classification) as i32,
                encoding: encoding_wire(var.encoding) as i32,
                injection: injection_wire(var.inject) as i32,
                present: false,
                value: Vec::new(),
            });
        }
    }

    // BTreeMap iteration is already sorted; keep explicit sort for protobuf contract.
    runtime_variables.sort_by(|a, b| a.logical_name.cmp(&b.logical_name));
    protected_values.sort_by(|a, b| a.logical_name.cmp(&b.logical_name));

    let public = ReleaseMetadataV1 {
        schema: "gump.release/1".to_string(),
        capsule_id: capsule_id.as_bytes().to_vec(),
        cluster_id: cluster_id.as_bytes().to_vec(),
        app: Some(AppIdentityV1 {
            namespace: manifest.app.namespace.to_string(),
            app_id: manifest.app.id.to_string(),
            workload_id: None,
            description: None,
            version_annotation: None,
        }),
        // Full normalized ManifestV1 / archive / build provenance land with later
        // packaging slices; N007 requires runtime_variables contracts + IDs.
        normalized_manifest: None,
        archive: None,
        build: None,
        required_capabilities: Vec::new(),
        runtime_variables,
    };

    let protected = ProtectedConfigV1 {
        schema: "gump.protected/1".to_string(),
        capsule_id: capsule_id.as_bytes().to_vec(),
        cluster_id: cluster_id.as_bytes().to_vec(),
        values: protected_values,
    };

    let mut public_metadata = Vec::with_capacity(public.encoded_len());
    public.encode(&mut public_metadata).map_err(|e| {
        CliError::new(
            CliErrorKind::Capsule,
            format!("encode release metadata: {e}"),
        )
    })?;

    let mut protected_bytes = Vec::with_capacity(protected.encoded_len());
    protected.encode(&mut protected_bytes).map_err(|e| {
        CliError::new(
            CliErrorKind::Crypto,
            format!("encode protected config: {e}"),
        )
    })?;

    Ok(PackagedConfig {
        public_metadata,
        protected_plaintext: Secret::new(protected_bytes),
    })
}

fn to_runtime_variable(name: &str, var: &Variable) -> Result<RuntimeVariableV1, CliError> {
    Ok(RuntimeVariableV1 {
        logical_name: name.to_string(),
        required: var.required,
        classification: classification_wire(var.classification) as i32,
        encoding: encoding_wire(var.encoding) as i32,
        max_bytes: var.max_bytes.unwrap_or(0),
        injection: injection_wire(var.inject) as i32,
        inherited_fd: var.fd.map(u32::from),
        reference_env: var.reference_env.clone(),
    })
}

fn resolve_variable(
    manifest: &Manifest,
    name: &str,
    var: &Variable,
    env_overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<Option<Secret<Vec<u8>>>, CliError> {
    let source = manifest
        .local
        .as_ref()
        .and_then(|local| local.variables.get(name))
        .map(|lv| lv.source.as_str())
        .unwrap_or(var.source.as_str());

    let raw = match read_source(source, env_overrides)? {
        Some(v) => v,
        None => {
            if var.required {
                return Err(CliError::new(
                    CliErrorKind::Policy,
                    format!("required variable {name:?} unset (source scheme redacted)"),
                ));
            }
            return Ok(None);
        }
    };

    if let Some(max) = var.max_bytes {
        if raw.expose().len() as u64 > max {
            return Err(CliError::new(
                CliErrorKind::Policy,
                format!(
                    "variable {name:?} exceeds max_bytes bound ({max}; got {} bytes)",
                    raw.expose().len()
                ),
            ));
        }
    }

    if matches!(var.encoding, Some(Encoding::Utf8) | None)
        && std::str::from_utf8(raw.expose()).is_err()
    {
        return Err(CliError::new(
            CliErrorKind::Policy,
            format!("variable {name:?} is not valid UTF-8"),
        ));
    }

    Ok(Some(raw))
}

fn read_source(
    source: &str,
    env_overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<Option<Secret<Vec<u8>>>, CliError> {
    if let Some(env_name) = source.strip_prefix("env:") {
        if env_name.is_empty() || env_name.bytes().any(|b| b == 0) {
            return Err(CliError::new(
                CliErrorKind::Policy,
                "malformed env variable source",
            ));
        }
        if let Some(bytes) = env_overrides.get(env_name) {
            return Ok(Some(Secret::new(bytes.clone())));
        }
        match std::env::var_os(env_name) {
            Some(os) => {
                let bytes = os_to_bytes(os)?;
                Ok(Some(Secret::new(bytes)))
            }
            None => Ok(None),
        }
    } else if let Some(literal) = source.strip_prefix("literal:") {
        // Test / local-dev only; never for production secret providers.
        Ok(Some(Secret::new(literal.as_bytes().to_vec())))
    } else if source.starts_with("provider:") {
        Err(CliError::new(
            CliErrorKind::Policy,
            "variable provider error: provider scheme not configured",
        ))
    } else {
        Err(CliError::new(
            CliErrorKind::Policy,
            "malformed variable source (unsupported scheme)",
        ))
    }
}

fn os_to_bytes(os: std::ffi::OsString) -> Result<Vec<u8>, CliError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(os.into_vec())
    }
    #[cfg(not(unix))]
    {
        os.into_string()
            .map(|s| s.into_bytes())
            .map_err(|_| CliError::new(CliErrorKind::Policy, "variable value is not valid Unicode"))
    }
}

fn classification_wire(c: Classification) -> ValueClassification {
    match c {
        Classification::Internal => ValueClassification::Internal,
        Classification::Secret => ValueClassification::Secret,
    }
}

fn encoding_wire(e: Option<Encoding>) -> ValueEncoding {
    match e {
        Some(Encoding::Utf8) | None => ValueEncoding::Utf8,
        Some(Encoding::Bytes) => ValueEncoding::Bytes,
    }
}

fn injection_wire(i: Inject) -> InjectionKind {
    match i {
        Inject::Env => InjectionKind::Env,
        Inject::Fd => InjectionKind::Fd,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gump_manifest::parse_manifest_str;

    fn manifest_with_greeting() -> Manifest {
        parse_manifest_str(
            r#"
schema = "gump/1"
[app]
id = "pack-test"
namespace = "ci"
[workload]
lifetime = "finite"
coordination = "independent"
success = "all_exit_zero"
[package]
root = "."
include = ["bin/hello"]
[runtime]
driver = "native"
command = ["./bin/hello"]
[runtime.variables.GREETING]
source = "env:GUMP_N007_GREETING"
required = true
classification = "internal"
encoding = "utf8"
max_bytes = "4KiB"
inject = "env"
[runtime.variables.TOKEN]
source = "env:GUMP_N007_TOKEN"
required = true
classification = "secret"
encoding = "utf8"
max_bytes = "4KiB"
inject = "env"
[telemetry]
protocol = "ratatouille/0.1"
format = "ndjson"
"#,
        )
        .expect("manifest")
    }

    fn fixed_v7(tag: u8) -> [u8; 16] {
        let mut b = [
            0x01, 0x8f, 0x4a, 0x00, 0x00, 0x00, 0x70, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        b[15] = tag;
        b
    }

    #[test]
    fn packages_contracts_without_secret_bytes_in_public() {
        let canary = "n007-canary-SECRET-value-9f3a";
        let m = manifest_with_greeting();
        let cap = CapsuleId::from_bytes(fixed_v7(0x11)).unwrap();
        let clu = ClusterId::from_bytes(fixed_v7(0x12)).unwrap();
        let mut env = BTreeMap::new();
        env.insert("GUMP_N007_GREETING".to_string(), b"hello".to_vec());
        env.insert("GUMP_N007_TOKEN".to_string(), canary.as_bytes().to_vec());
        let packed = package_release_config_with_env(&m, cap, clu, &env).unwrap();

        assert!(
            !packed
                .public_metadata
                .windows(canary.len())
                .any(|w| w == canary.as_bytes())
        );
        assert!(!String::from_utf8_lossy(&packed.public_metadata).contains(canary));

        let release = ReleaseMetadataV1::decode(packed.public_metadata.as_slice()).unwrap();
        assert_eq!(release.schema, "gump.release/1");
        assert_eq!(release.runtime_variables.len(), 2);
        assert!(
            release
                .runtime_variables
                .iter()
                .any(|v| v.logical_name == "TOKEN"
                    && v.classification == ValueClassification::Secret as i32)
        );

        let protected =
            ProtectedConfigV1::decode(packed.protected_plaintext.expose().as_slice()).unwrap();
        assert_eq!(protected.schema, "gump.protected/1");
        let token = protected
            .values
            .iter()
            .find(|v| v.logical_name == "TOKEN")
            .unwrap();
        assert_eq!(token.value, canary.as_bytes());
        assert!(token.present);
    }

    #[test]
    fn required_unset_fails_closed() {
        let m = manifest_with_greeting();
        let err = package_release_config(
            &m,
            CapsuleId::from_bytes(fixed_v7(0x13)).unwrap(),
            ClusterId::from_bytes(fixed_v7(0x14)).unwrap(),
        )
        .unwrap_err();
        assert_eq!(err.kind(), CliErrorKind::Policy);
        let msg = err.to_string();
        assert!(!msg.contains("n007-canary"));
        assert!(msg.contains("unset") || msg.contains("required"));
    }

    #[test]
    fn provider_scheme_fails_before_publish() {
        let m = parse_manifest_str(
            r#"
schema = "gump/1"
[app]
id = "pack-test"
namespace = "ci"
[workload]
lifetime = "finite"
coordination = "independent"
success = "all_exit_zero"
[package]
root = "."
include = ["bin/hello"]
[runtime]
driver = "native"
command = ["./bin/hello"]
[runtime.variables.X]
source = "provider:vault/path"
required = true
classification = "secret"
inject = "env"
[telemetry]
protocol = "ratatouille/0.1"
format = "ndjson"
"#,
        )
        .unwrap();
        let err = package_release_config(
            &m,
            CapsuleId::from_bytes(fixed_v7(0x15)).unwrap(),
            ClusterId::from_bytes(fixed_v7(0x16)).unwrap(),
        )
        .unwrap_err();
        assert_eq!(err.kind(), CliErrorKind::Policy);
        assert!(err.to_string().contains("provider"));
    }
}
