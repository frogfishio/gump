# Gump

Gump is a zero-footprint workload placer and supervisor for one server or many. Start with one disposable beta server to test the real packaged workload, then join servers to add capacity and replicate cluster memory without changing the application model. Nodes retain only transient application materializations, while S3 holds immutable sealed Capsules. Gump runs independently and is designed to pair exceptionally well with Kismet when Kismet is present.

- [v1 implementation pack](docs/v1/README.md) — normative engineering handoff, formats, protocols, security, tests, and delivery backlog
- [Project seed](SEED.md)
- [System design](docs/SYSTEM_DESIGN.md)
- [Distributed cluster memory](docs/CLUSTER_MEMORY.md)
- [Application manifest](docs/MANIFEST.md)
- [CLI and lifecycle](docs/CLI_LIFECYCLE.md)
- [Telemetry with Ratatouille](docs/TELEMETRY.md)
- [Hiccup workload discovery](docs/v1/HICCUP.md)

## Repository shape

One Cargo workspace (MSRV **1.85**, edition **2024**). Product crate boundaries match
[`docs/v1/README.md`](docs/v1/README.md) §5:

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
  gump-gates/           workspace quality gates (not a runtime dependency)
proto/gump/v1/          source-controlled wire schemas
spec/v1/                schemas, fixtures, vectors, and conformance data
```

Crates communicate through narrow traits and bounded typed channels. Protocol
types do not leak transport-library types. Drivers and connectors cannot mutate
cluster state directly. Dependency direction is enforced by
`cargo test -p gump-gates`. Traceability ledger checks:
`cargo run -p gump-gates --bin check-traceability` (structural) and
`--strict` / `--prove-missing` for release / W04 demonstration.
