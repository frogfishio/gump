# Gump v1 Open-Question Resolution Map

> Status: traceability aid. Normative answers live in the linked v1 contracts.

The parent architecture intentionally retained questions while the product was
being refined. The implementation pack closes every one for v1 as follows.

## System design §22

| # | v1 resolution |
|---:|---|
| 1 | Fixed segment table, canonical protobuf, and ustar+zstd in [`FORMATS.md`](FORMATS.md). |
| 2 | Mandatory/only v1 suites in [`DECISIONS.md`](DECISIONS.md) D004 and [`SECURITY.md`](SECURITY.md) §5. |
| 3 | Exactly one target cluster per Capsule. |
| 4 | Stable allocated workload UUID plus authorized namespace/name mapping. |
| 5 | Canonical signed `DeploymentDeclarationV1`. |
| 6 | S3 quarantine, exact evidence, immutable publish, and 5 GiB connector envelope. |
| 7 | OpenRaft 0.9 with first-party RAM-only storage. |
| 8 | Full in-memory custody replication: 1, 2, or 3 selected custodians. |
| 9 | External enrollment provider with ephemeral node certificates; no Gump host key. |
| 10 | Bounded 15-minute `continue_existing`, or stricter `stop_on_isolation`. |
| 11 | Values are immutable per Capsule; changing one creates a new Capsule. |
| 12 | Native, script, and OCI guarantees in [`RUNTIME.md`](RUNTIME.md) §§4–7. |
| 13 | Provider-neutral release capability plus effective live declaration; typed adapters. |
| 14 | Compare current generation and atomically commit exactly one next generation. |
| 15 | Optional external signed audit sink; audit-required actions fail closed. |
| 16 | No automatic final-Capsule garbage collection in v1; explicit purge only. |
| 17 | Same cluster uses its cluster ID/recovery authority and a new incarnation; a new identity is a new empty cluster. |
| 18 | Command names and waits frozen in D014. |
| 19 | Local parity and permitted overrides frozen in D014 and conformance §6. |
| 20 | Typed, aged envelope summaries in [`RUNTIME.md`](RUNTIME.md) §3. |
| 21 | Declared requests plus max(20%, policy minimum); policy defaults are labeled assumptions. |
| 22 | Connector `publish_if_absent` contract and exact-object equivalence rule. |
| 23 | Ratatouille version, topics, buffers, chunks, relays, and keepers fixed in D011/runtime. |
| 24 | Consensus, voters, budgets, watches, forced recovery, and record signatures fixed in D006/protocol. |

## Manifest §18

| # | v1 resolution |
|---:|---|
| 1 | `prepare` is first-class. |
| 2 | Allowlist-only; root capture requires an explicit acknowledgement. |
| 3 | Case-sensitive globset vocabulary; excludes are separate, no negation. |
| 4 | Symlinks are rejected. |
| 5 | `gump run` uses the deterministic materialized release tree. |
| 6 | `gump test --sealed` is the sealed local path. |
| 7 | Core sources: environment, hidden prompt, stdin/descriptor, and typed credential connector. |
| 8 | Both encrypt identically; `secret` has stricter display/injection policy. |
| 9 | Anonymous sealed file descriptor with an optional public descriptor reference. |
| 10 | Any value change makes a complete new Capsule. |
| 11 | Effective overrides are authorized, signed live intent and preserve provenance. |
| 12 | Stable workload UUID plus namespace/name. |
| 13 | Provider-neutral capability in release; effective choice and target in live declaration. |
| 14 | VCS revision/dirty state when available; explicit non-VCS provenance otherwise. |
| 15 | Open-root, two-pass metadata/content capture with change detection. |
| 16 | `gump.local.toml` is limited to the `[local]` source/port/watch namespace. |
| 17 | Wait defaults fixed in D014. |
| 18 | One command per unit; cooperating processes use explicit independent/gang units. |
| 19 | Structural denial and a typed cluster-policy scanner hook; no perfect-scan claim. |
| 20 | Release capability/request plus effective deployment default; local filter may be stricter. |
| 21 | Portable core uses named capability facts; typed providers own provider vocabulary. |
| 22 | v1 isolation profiles, attempt-root accounting, and enforced/observed/unavailable reporting are fixed. |
| 23 | Governance requests may appear as requests; all authority remains cluster policy. |

## Telemetry §18

| # | v1 resolution |
|---:|---|
| 1 | Ratatouille Rust 0.1 behavior plus `gump.ratatouille/1` cluster profile. |
| 2 | Callback sink in-process; no local plaintext network hop is required. |
| 3 | Lowercase ASCII slash-separated topics, 1–128 bytes; `gump/` reserved. |
| 4 | Gump control topics and `app/stdout`, `app/stderr` enabled; manifest filters may narrow app topics. |
| 5 | Per-attempt 8 MiB/30 s, node 256 MiB, two keepers where topology permits, drop-oldest. |
| 6 | Effective filters may update live at relay/subscriber; producer-library changes need restart unless its API supports reload. |
| 7 | Native API gets an inherited public bootstrap descriptor; captured stdout/stderr need none. |
| 8 | Gump cluster ingestion is typed protobuf with raw bytes, not NDJSON-only. |
| 9 | 32 KiB reads, 64 KiB records, explicit stream/chunk flags and byte sequences. |
| 10 | New subscribers receive the bounded recent window and explicit gaps. |
| 11 | Only safe IDs/result/reason metadata enters audit; no payload assumption. |
| 12 | Dedicated typed observations are authoritative input; sampled telemetry is human visibility. |
| 13 | Rendezvous hashing prefers distinct failure domains and transfers only bounded live windows. |

## Cluster memory §14

| # | v1 resolution |
|---:|---|
| 1 | OpenRaft 0.9 plus a Gump-owned RAM-only v2 storage adapter and simulator. |
| 2 | First three eligible servers by default; configurable one through seven voters. |
| 3 | 64 MiB authoritative, 32 MiB leased, 32 MiB history; documented compaction rules. |
| 4 | Signed recovery authorization naming and fencing missing members; new incarnation. |
| 5 | Cluster ID plus live incarnation in every session/record; old incarnation is rejected. |
| 6 | User signature for Capsules and declarations; authenticated fenced writers for other typed records. |

## CLI lifecycle §12

| # | v1 resolution |
|---:|---|
| 1 | Existing command names are the v1 surface. |
| 2 | Lifecycle-derived waits are fixed in D014. |
| 3 | 24 hours or 100,000 outcomes, whichever binds first. |
| 4 | `forget` is immediate after required termination; no hidden undo. |
| 5 | Canonical signed noninteractive authorization bound to exact purge plan and object evidence. |

