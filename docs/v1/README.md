# Gump v1 Implementation Pack

> Status: implementation baseline 1.0.0-draft.1  
> Audience: engineering, security, QA, operations, and integration owners

This directory turns Gump's product design into a buildable v1 contract. It is
not a delivery schedule and it does not reduce the maximal system design to a
temporary architecture. An implementation may deliver vertical slices in any
safe order, but every slice must converge on the contracts fixed here.

## 1. Authority

The documents have this precedence when they disagree:

1. [`DECISIONS.md`](DECISIONS.md) — frozen v1 choices and prohibitions.
2. [`FORMATS.md`](FORMATS.md) — serialized Capsule and manifest contracts.
3. [`PROTOCOL.md`](PROTOCOL.md) — cluster transport, records, RPCs, and state machines.
4. [`RUNTIME.md`](RUNTIME.md) — placement, drivers, supervision, telemetry, and connectors.
5. [`SECURITY.md`](SECURITY.md) — identity, authorization, cryptography, and secret custody.
6. [`CONFORMANCE.md`](CONFORMANCE.md) — required tests and release gates.
7. [`DELIVERY.md`](DELIVERY.md) — work decomposition and dependency order only.
8. The parent design documents — product intent where this pack is silent.

[`RESOLUTION_MAP.md`](RESOLUTION_MAP.md) maps every formerly open parent-design
question to its frozen v1 answer.

Machine-readable handoff artifacts are the
[`gump.toml` JSON Schema](../../spec/v1/gump.schema.json),
[`formats.proto`](../../proto/gump/v1/formats.proto),
[`cluster.proto`](../../proto/gump/v1/cluster.proto),
the [`manifest fixtures`](../../spec/v1/fixtures), and the
[`traceability ledger`](../../spec/v1/traceability.tsv).

Normative words MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY have their RFC 2119
meanings. Examples are non-normative unless explicitly identified as fixtures.

## 2. Product boundary

Gump v1 consists of one Rust binary, `gump`, which can act as developer CLI,
server, control-plane member, workload agent, or a combination of those roles.
It deploys arbitrary finite or continuous workloads from immutable sealed
Capsules, remembers live intent in replicated memory, places and supervises
execution, and emits diskless Ratatouille telemetry.

The four kernel responsibilities are:

1. Capsule construction, verification, storage, and materialization.
2. Distributed in-memory authoritative state.
3. Capability-aware placement and fenced admission.
4. Driver-neutral supervision and cleanup.

Kismet, HSM/KMS providers, object stores, output stores, checkpoint systems,
and publication systems are typed integrations. None is part of the kernel.

## 3. Non-negotiable invariants

- Gump creates no durable database, consensus log, snapshot, secret file, or
  node identity file.
- The only durable Gump-owned deployment object is the exact sealed Capsule in
  an object store.
- Capsule is generic framing. `gump/deployment/1` defines its contents.
- Runtime values are plaintext only inside authorized process memory. They are
  encrypted before leaving the developer Gump process and never extracted by
  ingress.
- Stored Capsules are inert. Only live, authorized intent in cluster memory can
  cause execution.
- A one-server cluster is valid and feature-complete, with zero tolerance for
  loss of its memory.
- Total loss of cluster memory creates an empty cluster. Recovery requires
  explicit Capsule reintroduction.
- No workload kind is assumed. Ports, readiness, continuous life, restart,
  gang coordination, GPU use, and publication are all explicit contracts.
- stdout and stderr are both captured as best-effort Ratatouille telemetry.
  Neither is an audit or application-state channel, and neither becomes a log
  file.
- Kismet is optional. Its absence affects only declarations that explicitly
  require the Kismet publication provider.

## 4. v1 compatibility unit

The compatibility unit is:

```text
manifest schema:       gump/1
Capsule dialect:       gump/deployment/1
wire protocol:         gump.cluster.v1, major 1
record schemas:        gump.record/*/1
driver ABI:            gump.driver/1
telemetry profile:     gump.ratatouille/1
```

Additive wire changes increment the protocol minor version. Any change to
canonical bytes, a signing transcript, associated data, identifier derivation,
state-machine meaning, or a required field needs a new named profile and an
explicit migration design.

## 5. Required repository shape

The implementation is one Cargo workspace with these ownership boundaries:

```text
crates/
  gump-types/           shared bounded types, clock, cancellation, IDs, safe errors
  gump-cli/             command UX and machine output
  gump-manifest/        parse, normalize, validate
  gump-capsule/         dialect, deterministic archive, signing transcript
  gump-crypto/          established primitives and provider traits
  gump-protocol/        protobuf messages, frame limits, golden vectors
  gump-memory/          in-memory Raft storage and typed record state machine
  gump-transport/       authenticated QUIC sessions
  gump-scheduler/       feasibility, reservations, scoring, gang admission
  gump-agent/           materialization, secret delivery, driver supervision
  gump-driver/          stable driver trait and common lifecycle
  gump-telemetry/       Ratatouille capture, relay, subscription
  gump-connectors/      object, identity, publication, output adapters
  gump-server/          role composition and process entry point
proto/gump/v1/          source-controlled wire schemas
spec/v1/                schemas, fixtures, vectors, and conformance data
```

Crates communicate through narrow traits and bounded typed channels. Protocol
types do not leak transport-library types. Drivers and connectors cannot mutate
cluster state directly.

## 6. Definition of implementation-ready

An engineering item is ready only when it names:

- its normative contract and stable input/output types;
- bounds, deadlines, cancellation, and retry behavior;
- authorization point and secret-redaction rules;
- idempotency and fencing behavior for mutations;
- observable reason and error codes;
- unit, property, fault-injection, and conformance evidence.

The v1 release is not complete until every MUST in this pack is either covered
by passing evidence or recorded as an explicit release blocker.
