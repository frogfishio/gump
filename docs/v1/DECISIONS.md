# Gump v1 Frozen Decisions

> Status: normative

This file closes the design questions that would otherwise cause incompatible
implementations. Reconsidering one requires an ADR that identifies affected
formats, migration, security impact, and replacement conformance vectors.

## D001 — implementation foundation

- Language: Rust, edition 2024, MSRV 1.85.
- Async runtime: Tokio 1.
- CLI: clap 4.
- Errors: typed internal errors; stable external codes defined by the protocol.
- Serialization: Protocol Buffers through prost 0.14 for wire and authoritative
  records; TOML is only a developer input; JSON is only operator output.
- Hash: BLAKE3 for Gump content identities and domain-separated fingerprints.
- Transport: QUIC through Quinn 0.11 with TLS 1.3 through rustls 0.23.
- Consensus: OpenRaft 0.9 with a Gump-owned, RAM-only v2 storage adapter.
- Every dependency above is behind a Gump-owned interface. Crate versions are
  implementation pins, never wire-format identifiers.

These choices align with already-proven local Kismet spikes but create no code,
runtime, deployment, or operational dependency on Kismet.

## D002 — identifiers and canonical names

- Cluster, incarnation, node, Capsule, workload, execution, unit, attempt,
  placement-group, operation, message, and lease IDs are UUIDv7 values encoded
  as 16 bytes on wire and lowercase hyphenated strings for humans.
- `app.id` and `app.namespace` are normalized human labels, not authority.
- A workload ID is allocated on first accepted deployment and remains stable
  across releases and generations. The tuple `(cluster_id, namespace, app.id)`
  maps to exactly one live workload ID while retained.
- A release is identified by `(capsule_id, capsule_digest)`. Human versions are
  annotations.
- Names use lowercase ASCII according to this expression:

  ```text
  [a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?
  ```

  Namespace and application names are each at most 63 bytes.

## D003 — Capsule framing and inner representation

- Base Capsule format is exactly Capsule v0001 from `capsule-lib` 1.0.
- Gump uses Capsule encoding `C`. The header is a deterministic CBOR map and the
  payload is one definite-length CBOR byte string whose contents are Gump's
  binary segment layout. This obeys Capsule's rule that one encoding applies to
  both blocks without Base64 expansion of large application artifacts.
- The dialect header entry is `"dialect": "gump/deployment/1"`.
- The Capsule payload is the binary segment table in `FORMATS.md`, not a tarball
  with secrets hidden among files.
- Public metadata and protected configuration are canonical protobuf bytes.
- Application material is deterministic POSIX ustar followed by Zstandard.
- Capsule's CRC detects accidental damage only. Gump's BLAKE3 digest, Ed25519
  signature, and AEAD authentication provide security.
- `gump-capsule` uses `capsule-lib` as the reference implementation and shares
  its goldens. Because v1 Capsules may be large, the implementation must add or
  wrap a streaming Capsule v0001 reader/writer; it may not buffer a full Capsule
  merely to use the current byte-slice API.

## D004 — signatures, encryption, and cluster binding

- Release authorization uses Ed25519 signatures.
- Protected configuration uses a fresh random 256-bit DEK and
  XChaCha20-Poly1305 with a fresh 192-bit nonce.
- The DEK is sealed to the target cluster with HPKE base mode using X25519,
  HKDF-SHA256, and ChaCha20-Poly1305.
- One v1 Capsule is bound to exactly one cluster. Multi-cluster key slots are
  deliberately absent; deploying to a second cluster produces a new Capsule.
- The public signing transcript and AEAD associated data are exact byte
  constructions in `FORMATS.md`. No Rust-struct serialization is signed.
- HSM/KMS integration provides or protects cluster unseal authority. It does
  not change the Capsule cipher suite or wire representation.

## D005 — cluster identity and unseal

- `--init` receives cluster configuration and an unseal-provider selection.
- Software unseal uses a 32-byte cluster recovery secret supplied at startup or
  reconstructed from operator-held Shamir shares. Gump never writes the secret
  or shares.
- A cluster X25519 unseal keypair is deterministically derived from the recovery
  secret and cluster ID using HKDF-SHA256 and held only in locked memory. The
  public key is safe to distribute to developer Gump clients.
- HSM/KMS providers instead return the same logical unwrap capability through a
  provider trait; provider credentials and durability remain external to Gump.
- Loss of all custodians reseals the cluster but does not destroy Capsules.

## D006 — distributed memory

- Gump v1 implements a single Raft group for all authoritative control records.
  This is an internal implementation boundary, not a promise that future scale
  is one group.
- Raft log, vote, membership, snapshots, deduplication results, and application
  state exist only in RAM. No fallback persistence is permitted.
- One node commits with itself. Two nodes require both for new commits. Three
  nodes tolerate one unavailable voter.
- OpenRaft snapshots are memory-to-memory state-transfer artifacts and MUST NOT
  be written to a file or object store.
- The state machine exposes typed transactions, leases, revisions, watches, and
  bounded histories. It is not a public general-purpose K/V API.
- The first three eligible servers become memory voters by default. Operators
  may select one through seven voters; other servers remain full workload agents.
  A joiner is a non-voting learner only while transferring state.
- Forced recovery from a non-quorate survivor requires a signed recovery
  authorization naming every fenced missing member and rotates the cluster
  incarnation, controller epoch, and all node certificates.
- End-user signatures are required for release Capsules and deployment
  declarations. Other records rely on authenticated writer role, Raft commit,
  generation, and fence unless their own profile says otherwise.

## D007 — transport and trust

- Cluster traffic uses mutually authenticated QUIC. Datagrams are limited to
  discovery/liveness hints; every authoritative operation uses a stream.
- Initial control-frame maximum is 1 MiB. Bulk data uses 1 MiB chunks with
  per-chunk length bounds and a final BLAKE3 digest.
- Client-to-ingress deployment uses HTTPS with TLS 1.3 and a streamed body. The
  authentication provider is external; the authorization decision is Gump's.
- Local CLI-to-daemon traffic uses a Unix-domain socket with peer credentials.
- Valid transport identity is necessary but never sufficient authorization.

## D008 — object storage

- v1 ships an S3 connector implementing immutable `put_if_absent`, ranged get,
  head, delete, and abortable quarantine upload.
- Canonical final key:
  `clusters/<cluster-id>/capsules/<capsule-id>.capsule`.
- Final publication MUST be write-if-absent. A pre-existing object is accepted
  only if length and BLAKE3 digest exactly match.
- Quarantine objects are sealed Capsule bytes and are non-authoritative. They
  are deleted after successful promotion or by bounded age cleanup.
- v1 supports Capsules up to 5 GiB through conditional single-object PUT. The
  protocol and segment table are 64-bit and do not impose this delivery limit;
  a connector with atomic immutable multipart promotion may raise it later.
- Bucket versioning and retention are strongly recommended but are operator
  policy, not hidden Gump state.

## D009 — manifest v1 scope

- File capture is allowlist-only. `include=["."]` is permitted only with the
  standard deny set and an explicit `allow_workspace_root=true` acknowledgement.
- Glob syntax follows `globset`: `/` separator, `*`, `?`, `**`, and character
  classes; no negation. Excludes are a separate list. Matching is case-sensitive.
- Symlinks, hard links, sparse files, devices, sockets, FIFOs, xattrs, and ACLs
  are rejected. Regular files and explicit directories are supported.
- `prepare` is first-class. Its outputs enter a virtual package tree; an
  implicit shell is never used.
- `gump run` uses the same materialized deterministic release tree as deploy.
- v1 supports native, script, and OCI drivers through the same driver ABI.
- v1 supports independent and gang coordination. Ordered coordination and
  elastic gangs are reserved for a later schema.
- One unit contains one primary command in v1. Cooperating processes are
  represented as separate independent or gang units, not an implicit pod.
- `internal` and `secret` share encryption; their display policy differs.
- Secret injection supports environment and inherited anonymous file
  descriptors. No plaintext path is created.
- Any runtime-value change creates a complete new Capsule. In-place secret
  rotation and configuration-only Capsules are future dialect extensions.
- The core packager provides structural sensitive-file denial and a typed
  secret-scanner policy hook; it does not claim perfect content detection.

## D010 — runtime survival policy

- The default on loss of control-plane authority is `continue_existing` for a
  bounded 15-minute isolation grace period. No restart, replacement, secret
  redelivery, publication renewal, or new useful work may begin without a valid
  fence.
- A manifest may request `stop_on_isolation` or a shorter grace. Cluster policy
  may only make the behavior stricter.
- After grace expiry, the agent terminates the workload process tree and cleans
  the attempt root.
- Gang workloads default to `stop_group_on_member_loss` and never admit a
  partial group unless a future elastic-group contract explicitly permits it.

## D011 — telemetry

- Gump integrates Ratatouille Rust 0.1 through its callback `Sink`; it does not
  use Ratatouille's plaintext HTTP/TCP transport across trust boundaries.
- Cluster telemetry profile is `gump.ratatouille/1` and uses bounded protobuf
  frames carrying the original stream bytes and authoritative Gump identity.
- stdout and stderr are distinct topics, not distinct reliability classes.
- Records are at most 64 KiB. Longer input is chunked without requiring UTF-8.
- Each agent retains 8 MiB or 30 seconds of recent records per attempt,
  whichever binds first; node total is 256 MiB. Overflow drops oldest records.
- Two telemetry keepers are selected by rendezvous hashing when at least three
  nodes exist; otherwise all surviving nodes are eligible. Replication remains
  best effort and never consumes consensus or blocks supervision.
- Resource observations use a dedicated typed observation protocol; a sampled
  Ratatouille view may also be emitted for humans.

## D012 — publication and external data

- Publication intent lives in the release contract as provider-neutral endpoint
  capability plus deployment defaults. The effective choice is in live intent.
- The v1 provider interface is `reconcile`, `status`, and `withdraw` with an
  opaque provider receipt. Providers cannot write desired state.
- Kismet is the first optional provider and uses Kismet's local authenticated
  publication interface. Gump remains fully operational without it.
- Outputs and checkpoints go through explicit application or typed connector
  contracts. They never become Gump-owned durable state.

## D013 — audit honesty

- Ratatouille is not an audit log.
- Gump emits signed audit events to an optional external audit sink. If cluster
  policy marks an operation `audit_required`, the mutation fails unless the
  sink returns a durable receipt before commit.
- Without such a sink Gump provides authenticated live state and bounded
  explanations only, and MUST NOT claim durable auditability.

## D014 — command surface

The v1 commands are the names in `CLI_LIFECYCLE.md`. Default deploy waits are:

- continuous with publication: `published`;
- continuous without publication: `eligible` if readiness exists, else `started`;
- finite: `completed`;
- gang: barrier opened, then the lifecycle-derived condition above.

Idempotency outcomes are retained for 24 hours or 100,000 operations, whichever
binds first. `forget` has no hidden undo window. `purge` requires a signed
authorization in noninteractive mode.

`gump run` uses a materialized unsealed release. `gump test --sealed` performs
the complete Capsule build/verify/unseal path locally before running it. Local
overrides may change only source mappings, local port choices, watch behavior,
and optional local publication; they cannot change release semantics.

## D015 — explicitly prohibited

- SQLite, redb, RocksDB, sled, local JSON state, Raft WAL files, and filesystem
  recovery scans.
- Reconstructing intent from S3 inventory or transient node materializations.
- Plaintext secrets in Capsule public metadata, K/V records, environment dumps,
  telemetry, errors, process arguments, or ordinary files.
- Standard log-file collection as a Gump subsystem.
- Assuming a workload is an HTTP service, stateless, containerized, restartable,
  independently placeable, or safe to move.
- Treating Kismet, an HSM, S3, OCI, GPUs, or three servers as mandatory.
