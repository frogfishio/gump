# Gump v1 Security Contract

> Status: normative; independent review required before production claims

## 1. Protected assets

Gump protects runtime values, cluster unseal authority, release-signing keys,
node/session keys, workload authorization, control-plane integrity, Capsule
integrity, and connector credentials. Application files and normalized public
metadata are not confidential unless the application encrypts them separately.

## 2. Explicit threat boundary

The v1 protocol addresses network attackers, malicious clients, unauthorized
cluster members, replay, stale controllers, object substitution, archive
escape, malformed protocol input, accidental disk persistence, and ordinary
crash/partition faults.

It does not claim Byzantine consensus. Host root or hypervisor compromise can
read secrets present on that host. A workload can disclose a secret legitimately
delivered to it. Traffic analysis, CPU side channels, malicious firmware, and
provider compromise require controls outside the v1 boundary.

## 3. Principal and role model

Authenticated principals have stable provider-qualified IDs. Built-in roles
are bundles only; enforcement checks explicit actions:

```text
cluster.initialize       cluster.join          cluster.manage
cluster.unseal           workload.deploy       workload.read
workload.alter           workload.stop         workload.forget
execution.cancel         capsule.inventory     capsule.inspect_public
capsule.inspect_protected_metadata             capsule.reintroduce
capsule.purge            secret.resolve        secret.deliver
telemetry.subscribe      publication.use:<provider>
connector.use:<name>     policy.read            policy.manage
audit.read
```

Authorization input includes principal, cluster/incarnation, namespace,
workload, operation, Capsule signer, secret names/classifications, connector,
publication provider, requested policy, and current revision. The decision
returns allow/deny, decision ID, constraints, expiry, and policy revision.

The manifest cannot grant authority. Server roles do not imply user actions.
A node may perform only actions required by its current role and leases.

## 4. Release signer trust

Clusters maintain an in-memory policy of authorized Ed25519 signer public keys,
namespace scopes, expiry, and optional capability constraints. A Capsule's
embedded public key proves a signature but grants no trust by itself. Ingress
checks the key fingerprint against current policy before object publication and
again before declaration acceptance.

Revocation prevents new declaration generations and reintroduction. Existing
running attempts follow explicit emergency policy; revocation does not silently
rewrite immutable Capsules.

## 5. Cryptographic profile

Required suites are:

| Purpose | Suite |
|---|---|
| release/declaration signature | Ed25519 |
| content identity | BLAKE3-256 with domain separation where derived |
| runtime payload encryption | XChaCha20-Poly1305 |
| DEK sealing | HPKE X25519/HKDF-SHA256/ChaCha20-Poly1305 |
| transport | TLS 1.3, rustls-approved AEAD suites |
| key derivation | HKDF-SHA256 |
| software share reconstruction | Shamir over GF(256), vetted library only |

Randomness comes from the operating system CSPRNG. No nonce, key, share, or
identifier is derived from wall time. No handwritten cryptographic primitive is
permitted. Crypto dependencies are pinned, audited, and covered by known-answer
and cross-library vectors.

## 6. Software unseal ceremony

At initialization, an operator supplies a 32-byte recovery secret or requests
generation and immediate split into `n` shares with threshold `t`. Gump displays
or writes shares only to explicit operator-selected outputs and never retains a
durable copy. Production defaults are 5 shares, threshold 3; one-server beta use
may select 1 of 1 with an explicit loss/compromise warning.

On startup, shares are entered through protected input until the threshold
reconstructs the secret. The cluster unseal private key is derived as:

```text
HKDF-SHA256(
  ikm = recovery_secret,
  salt = cluster_id,
  info = "gump.cluster-unseal-x25519/1\0"
)
```

The derived scalar is clamped by the vetted X25519 implementation. Recovery
secret and reconstruction buffers are immediately zeroized. Custody members
receive unwrap capability through an authenticated in-memory protocol; a global
plaintext secret table is never broadcast.

An HSM/KMS provider replaces reconstruction/derivation with provider-backed
unwrap authorization. Gump stores only provider type and non-secret key ID in
live memory and Capsule envelopes. Provider configuration is delivered at
startup or by external machine identity, not a Gump file.

## 7. Custody and delivery

In one-member mode the sole member is custodian. With three or more eligible
memory members, three custodians are selected across failure domains. v1 uses
full in-memory unwrap capability replication, not threshold decryption; the
recovery ceremony remains threshold-capable. Two members use both.

Custodians decrypt only after validating current declaration, placement,
attempt, agent transport identity, fence, variable scope, and authorization.
They return a short-lived encrypted delivery to the agent certificate's X25519
key. Delivery plaintext is scoped to requested variables and never enters K/V.

Custody transfer uses mutually authenticated sessions, independent ephemeral
keys, transcript binding, replay protection, and explicit zeroization. A new
custodian is not eligible until transfer verification completes. Loss of every
custodian reseals the cluster.

## 8. Memory handling

Secret-bearing buffers use zeroizing containers, avoid cloning and formatting,
and are excluded from serialization traits except the one controlled encryption
path. Gump attempts memory locking and non-dumpable process settings and reports
whether each is enforced. It disables core dumps, redacts panic hooks, and never
passes plaintext in command arguments.

Rust memory safety reduces accidental disclosure but does not guarantee that
every compiler/runtime copy is erased. Product language says “never
intentionally persisted and aggressively zeroized,” not “physically impossible
to recover from compromised RAM.”

## 9. Node identity without Gump persistence

Node enrollment is rooted in an operator token, cloud/workload identity, TPM,
HSM, or another configured external authenticator. The agent generates session
signing and X25519 keys in memory, proves enrollment authority, and receives a
short-lived cluster certificate. Restart repeats the process with a higher node
incarnation allocated by live consensus.

If no external authenticator is configured, the operator must supply a join
token at every restart. This is acceptable for beta/single-server operation and
is reported plainly. Gump never quietly writes a reusable join credential.

## 10. Network and parser controls

- TLS 1.3 only; mutual authentication for cluster traffic.
- Strict SNI/identity/cluster binding and certificate expiry checks.
- Bounded connections, streams, frames, chunks, lists, decompression ratio,
  archive expansion, and error text before allocation.
- Per-principal and per-node rate limits with control-plane reserved capacity.
- Signature verified over original canonical bytes; signed protobuf is never
  decode/re-encoded for verification.
- Capsule CRC is never accepted as security evidence.
- Peer-assisted Capsule bytes are treated exactly as untrusted object-store
  bytes and receive full verification.

## 11. Object-store controls

The S3 credential can write quarantine, publish only under the cluster prefix,
read/head exact Capsule keys, and delete quarantine. Purge is a separate
credential/action where possible. Public access is blocked. TLS verification,
server-side encrypted storage, bucket versioning, object lock, replication, and
provider audit are recommended defense in depth; client-side protected config
encryption remains mandatory.

Promotion checks exact cluster prefix, Capsule UUID, length, digest, and
write-if-absent. A same-key different-object conflict is a security event.
Garbage collection never deletes final Capsules automatically in v1. Only an
authorized explicit `purge` with preconditions can remove one.

## 12. Audit contract

Security-relevant operations create canonical signed `AuditEventV1` records
with event ID, principal, action, target IDs, policy decision ID, cluster
revision, result code, timestamp annotation, and previous-event digest when an
external sink supports chaining. They never include protected values.

An audit sink contract declares whether acceptance is durable. When policy
requires durable audit, Gump sends the event and obtains a signed receipt before
committing the protected mutation. The receipt digest enters the declaration or
operation record. Sink outage fails closed for those actions.

Without a required external sink, the same event may be emitted to Ratatouille
for visibility, but Gump labels it non-durable telemetry.

## 13. Security failure behavior

Signature, cluster-binding, canonicalization, archive, AEAD, replay, equal-
generation divergence, impossible revision, or identity mismatch failures are
fail-closed and not automatically retried. The peer/session may be quarantined.
The safe error exposes object IDs and reason codes, never parser fragments that
might contain secret bytes.

Availability failure must not weaken authorization or encryption. A one-server
cluster may lose everything; it never becomes permitted to persist secrets or
accept stale authority to avoid that consequence.

## 14. Required external review

Before a production security claim, independent reviewers must examine Capsule
transcripts and vectors, software/HSM unseal, custody replication and delivery,
certificate enrollment, authorization coverage, archive extraction, process
isolation, memory/core-dump behavior, S3 promotion, dependency provenance,
fuzzing results, and the limitations stated here.

