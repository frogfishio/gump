//! Bounded zero-to-one bootstrap contracts (`gump.bootstrap/1`).
//!
//! This module contains public, secret-free handoff types and the small wire
//! messages shared by the bootstrap server and CLI. It intentionally contains
//! no transport, filesystem, secret-provider, or cluster implementation.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use zeroize::Zeroize as _;

pub const ACTIVATION_SCHEMA: &str = "gump.bootstrap-activation/1";
pub const HANDOFF_SCHEMA: &str = "gump.bootstrap-handoff/1";
pub const INITIALIZE_SCHEMA: &str = "gump.bootstrap-initialize/1";
pub const RESULT_SCHEMA: &str = "gump.bootstrap-result/1";
pub const BOOTSTRAP_PROTOCOL: &str = "gump.bootstrap/1";
pub const MAX_ACTIVATION_BUNDLE_BYTES: usize = 8 * 1024;
pub const MAX_HANDOFF_BYTES: usize = 16 * 1024;
pub const MAX_INITIALIZE_BYTES: usize = 64 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 32 * 1024;

#[derive(Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationBundle {
    pub schema: String,
    pub incarnation: String,
    pub endpoint: String,
    pub bootstrap_protocol: String,
    pub build_identity: String,
    pub endpoint_identity: String,
    pub activation_code: String,
    pub expires_at: String,
}

impl core::fmt::Debug for ActivationBundle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ActivationBundle")
            .field("schema", &self.schema)
            .field("incarnation", &self.incarnation)
            .field("endpoint", &self.endpoint)
            .field("bootstrap_protocol", &self.bootstrap_protocol)
            .field("build_identity", &self.build_identity)
            .field("endpoint_identity", &self.endpoint_identity)
            .field("activation_code", &"***")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl Drop for ActivationBundle {
    fn drop(&mut self) {
        self.activation_code.zeroize();
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootstrapHandoff {
    pub schema: String,
    pub handoff_id: String,
    pub incarnation: String,
    pub endpoint: String,
    pub bootstrap_protocol: String,
    pub build_identity: String,
    pub machine_identity: String,
    pub ssh_trust_mode: String,
    pub ssh_host_key: String,
    pub endpoint_identity: String,
    pub expires_at: String,
    pub binding_digest: String,
    pub secret_ref: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HandoffBinding<'a> {
    schema: &'a str,
    handoff_id: &'a str,
    incarnation: &'a str,
    endpoint: &'a str,
    bootstrap_protocol: &'a str,
    build_identity: &'a str,
    machine_identity: &'a str,
    ssh_trust_mode: &'a str,
    ssh_host_key: &'a str,
    endpoint_identity: &'a str,
    expires_at: &'a str,
}

impl BootstrapHandoff {
    /// SHA-256 over the RFC 8785-compatible fixed string-only projection.
    ///
    /// The projection contains only strings. Converting it to a JSON value
    /// places object members in lexicographic order; `serde_json` then emits the
    /// UTF-8 string encoding and escaping required by JCS for this constrained
    /// value domain.
    pub fn computed_binding_digest(&self) -> Result<String, String> {
        let projection = HandoffBinding {
            schema: &self.schema,
            handoff_id: &self.handoff_id,
            incarnation: &self.incarnation,
            endpoint: &self.endpoint,
            bootstrap_protocol: &self.bootstrap_protocol,
            build_identity: &self.build_identity,
            machine_identity: &self.machine_identity,
            ssh_trust_mode: &self.ssh_trust_mode,
            ssh_host_key: &self.ssh_host_key,
            endpoint_identity: &self.endpoint_identity,
            expires_at: &self.expires_at,
        };
        let value = serde_json::to_value(projection).map_err(|e| e.to_string())?;
        let encoded = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        Ok(format!("sha256:{}", lower_hex(&Sha256::digest(encoded))))
    }

    pub fn validate(&self, now: OffsetDateTime) -> Result<(), String> {
        require_exact(&self.schema, HANDOFF_SCHEMA, "schema")?;
        require_exact(
            &self.bootstrap_protocol,
            BOOTSTRAP_PROTOCOL,
            "bootstrapProtocol",
        )?;
        validate_common(
            &self.incarnation,
            &self.endpoint,
            &self.build_identity,
            &self.endpoint_identity,
            &self.expires_at,
            now,
        )?;
        bounded_nonempty(&self.handoff_id, 128, "handoffId")?;
        bounded_nonempty(&self.machine_identity, 512, "machineIdentity")?;
        if !matches!(
            self.ssh_trust_mode.as_str(),
            "pre-established" | "provider-attested" | "operator-accepted"
        ) {
            return Err("sshTrustMode is not recognized".into());
        }
        bounded_nonempty(&self.ssh_host_key, 256, "sshHostKey")?;
        bounded_nonempty(&self.secret_ref, 1024, "secretRef")?;
        let expected = self.computed_binding_digest()?;
        if !constant_time_eq(expected.as_bytes(), self.binding_digest.as_bytes()) {
            return Err("bindingDigest does not match the handoff".into());
        }
        Ok(())
    }
}

impl ActivationBundle {
    pub fn validate(&self, now: OffsetDateTime) -> Result<(), String> {
        require_exact(&self.schema, ACTIVATION_SCHEMA, "schema")?;
        require_exact(
            &self.bootstrap_protocol,
            BOOTSTRAP_PROTOCOL,
            "bootstrapProtocol",
        )?;
        validate_common(
            &self.incarnation,
            &self.endpoint,
            &self.build_identity,
            &self.endpoint_identity,
            &self.expires_at,
            now,
        )?;
        if !(32..=256).contains(&self.activation_code.len()) {
            return Err("activationCode length is outside 32..=256 bytes".into());
        }
        Ok(())
    }
}

#[derive(Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InitializeRequest {
    pub schema: String,
    pub session_id: String,
    pub transcript_digest: String,
    pub handoff_binding_digest: String,
    pub activation_code: String,
    pub management_client_csr_der_base64: String,
    pub management_client_identity_ref: String,
    /// Existing bounded server-parameter document. Secret bytes travel only
    /// inside the pinned TLS session and remain in memory on the server.
    pub server_parameters: serde_json::Value,
}

impl core::fmt::Debug for InitializeRequest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InitializeRequest")
            .field("schema", &self.schema)
            .field("session_id", &self.session_id)
            .field("transcript_digest", &self.transcript_digest)
            .field("handoff_binding_digest", &self.handoff_binding_digest)
            .field("activation_code", &"***")
            .field(
                "management_client_csr_der_base64",
                &self.management_client_csr_der_base64,
            )
            .field(
                "management_client_identity_ref",
                &self.management_client_identity_ref,
            )
            .field("server_parameters", &"***")
            .finish()
    }
}

impl Drop for InitializeRequest {
    fn drop(&mut self) {
        self.activation_code.zeroize();
    }
}

impl InitializeRequest {
    pub fn validate(&self) -> Result<(), String> {
        require_exact(&self.schema, INITIALIZE_SCHEMA, "schema")?;
        bounded_nonempty(&self.session_id, 128, "sessionId")?;
        validate_sha256(&self.transcript_digest, "transcriptDigest")?;
        validate_sha256(&self.handoff_binding_digest, "handoffBindingDigest")?;
        if !(32..=256).contains(&self.activation_code.len()) {
            return Err("activationCode length is outside 32..=256 bytes".into());
        }
        bounded_nonempty(
            &self.management_client_csr_der_base64,
            16 * 1024,
            "managementClientCsrDerBase64",
        )?;
        let csr = base64::engine::general_purpose::STANDARD
            .decode(&self.management_client_csr_der_base64)
            .map_err(|_| "managementClientCsrDerBase64 is invalid base64")?;
        if csr.is_empty() || csr.len() > 12 * 1024 {
            return Err("management client CSR is empty or oversized".into());
        }
        bounded_nonempty(
            &self.management_client_identity_ref,
            1024,
            "managementClientIdentityRef",
        )?;
        if !self.server_parameters.is_object() {
            return Err("serverParameters must be an object".into());
        }
        let computed = transcript_digest(
            &self.server_parameters,
            &self.management_client_csr_der_base64,
            &self.management_client_identity_ref,
        )?;
        if !constant_time_eq(computed.as_bytes(), self.transcript_digest.as_bytes()) {
            return Err("transcriptDigest does not match serverParameters".into());
        }
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InitializationTranscript<'a> {
    server_parameters: &'a serde_json::Value,
    management_client_csr_der_base64: &'a str,
    management_client_identity_ref: &'a str,
}

pub fn transcript_digest(
    parameters: &serde_json::Value,
    management_client_csr_der_base64: &str,
    management_client_identity_ref: &str,
) -> Result<String, String> {
    let encoded = serde_json::to_vec(&InitializationTranscript {
        server_parameters: parameters,
        management_client_csr_der_base64,
        management_client_identity_ref,
    })
    .map_err(|e| e.to_string())?;
    Ok(format!("sha256:{}", lower_hex(&Sha256::digest(encoded))))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootstrapResult {
    pub schema: String,
    pub status: String,
    pub cluster_identity: String,
    pub node_identity: String,
    pub session_id: String,
    pub committed_incarnation: String,
    pub management_endpoint: String,
    pub management_client_identity_ref: String,
    pub management_ca_certificate_der_base64: String,
    pub management_client_certificate_der_base64: String,
    pub management_mtls_verified: bool,
    pub node_admitted: bool,
    pub activation_consumed: bool,
    pub bootstrap_closed: bool,
}

fn validate_common(
    incarnation: &str,
    endpoint: &str,
    build_identity: &str,
    endpoint_identity: &str,
    expires_at: &str,
    now: OffsetDateTime,
) -> Result<(), String> {
    bounded_nonempty(incarnation, 128, "incarnation")?;
    if !endpoint.starts_with("https://") || endpoint.len() > 2048 {
        return Err("endpoint must be a bounded https URL".into());
    }
    bounded_nonempty(build_identity, 128, "buildIdentity")?;
    validate_endpoint_identity(endpoint_identity)?;
    let expiry = OffsetDateTime::parse(expires_at, &Rfc3339)
        .map_err(|_| "expiresAt must be RFC 3339".to_string())?;
    if expiry <= now {
        return Err("activation handoff has expired".into());
    }
    Ok(())
}

pub fn validate_endpoint_identity(identity: &str) -> Result<[u8; 32], String> {
    let encoded = identity
        .strip_prefix("SHA256:")
        .ok_or("endpointIdentity must start with SHA256:")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "endpointIdentity is not canonical base64")?;
    if base64::engine::general_purpose::STANDARD.encode(&decoded) != encoded {
        return Err("endpointIdentity is not canonical base64".into());
    }
    decoded
        .try_into()
        .map_err(|_| "endpointIdentity must contain a 32-byte SHA-256 digest".into())
}

fn validate_sha256(value: &str, field: &str) -> Result<(), String> {
    let hex = value
        .strip_prefix("sha256:")
        .ok_or_else(|| format!("{field} must start with sha256:"))?;
    if hex.len() != 64
        || !hex
            .as_bytes()
            .iter()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        return Err(format!("{field} must contain lowercase SHA-256 hex"));
    }
    Ok(())
}

fn bounded_nonempty(value: &str, max: usize, field: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(format!(
            "{field} is empty, oversized, or contains control characters"
        ));
    }
    Ok(())
}

fn require_exact(value: &str, expected: &str, field: &str) -> Result<(), String> {
    if value != expected {
        return Err(format!("unsupported {field}"));
    }
    Ok(())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut difference = a.len() ^ b.len();
    let max = a.len().max(b.len());
    for index in 0..max {
        difference |=
            usize::from(a.get(index).copied().unwrap_or(0) ^ b.get(index).copied().unwrap_or(0));
    }
    difference == 0
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn future() -> String {
        "2099-01-01T00:00:00Z".into()
    }

    fn handoff() -> BootstrapHandoff {
        let mut value = BootstrapHandoff {
            schema: HANDOFF_SCHEMA.into(),
            handoff_id: "handoff-1".into(),
            incarnation: "incarnation-1".into(),
            endpoint: "https://203.0.113.10:7443".into(),
            bootstrap_protocol: BOOTSTRAP_PROTOCOL.into(),
            build_identity: "0.1.0+build-99".into(),
            machine_identity: "digitalocean/droplet/12345".into(),
            ssh_trust_mode: "operator-accepted".into(),
            ssh_host_key: "SHA256:host".into(),
            endpoint_identity: format!(
                "SHA256:{}",
                base64::engine::general_purpose::STANDARD.encode([7_u8; 32])
            ),
            expires_at: future(),
            binding_digest: String::new(),
            secret_ref: "secret://macrun/project/key".into(),
        };
        value.binding_digest = value.computed_binding_digest().unwrap();
        value
    }

    #[test]
    fn handoff_binding_detects_redirect() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut value = handoff();
        value.validate(now).unwrap();
        value.endpoint = "https://attacker.invalid:7443".into();
        assert_eq!(
            value.validate(now).unwrap_err(),
            "bindingDigest does not match the handoff"
        );
    }

    #[test]
    fn transcript_detects_changed_parameters() {
        let parameters = serde_json::json!({"cluster_id":"abc"});
        let mut request = InitializeRequest {
            schema: INITIALIZE_SCHEMA.into(),
            session_id: "session-1".into(),
            transcript_digest: transcript_digest(&parameters, "Y3Ny", "secret://identity").unwrap(),
            handoff_binding_digest: format!("sha256:{}", "a".repeat(64)),
            activation_code: "x".repeat(43),
            management_client_csr_der_base64: "Y3Ny".into(),
            management_client_identity_ref: "secret://identity".into(),
            server_parameters: parameters,
        };
        request.validate().unwrap();
        request.server_parameters = serde_json::json!({"cluster_id":"changed"});
        assert_eq!(
            request.validate().unwrap_err(),
            "transcriptDigest does not match serverParameters"
        );
    }

    #[test]
    fn expired_handoff_is_rejected() {
        let value = handoff();
        assert_eq!(
            value
                .validate(OffsetDateTime::from_unix_timestamp(4_102_444_801).unwrap())
                .unwrap_err(),
            "activation handoff has expired"
        );
    }
}
