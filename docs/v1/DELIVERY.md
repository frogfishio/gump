# Gump v1 Engineering Delivery Pack

> Status: normative work decomposition, not architecture

The target design is the complete architecture in this pack. The work packages
below control integration risk and evidence order; they do not define temporary
product architectures. Teams may work in parallel where dependencies allow.

## 1. Repository and quality baseline

| ID | Deliverable | Exit evidence |
|---|---|---|
| W01 | Cargo workspace and crate boundaries from `README.md` | workspace builds on MSRV; dependency-direction test |
| W02 | shared bounded types, clock, cancellation, IDs, safe errors | unit/property tests; no secret `Debug` implementation |
| W03 | protobuf build and golden harness | exact encode/decode fixtures; frame-bound tests |
| W04 | traceability and CI gates | missing requirement demonstrably fails CI |
| W05 | deterministic simulation harness | controlled time/network/crash smoke suite |

W01–W05 unlock all other work. They contain no persistence substitute.

## 2. Format and local-execution workstream

| ID | Deliverable | Depends | Exit evidence |
|---|---|---|---|
| F01 | strict `gump/1` parser and normalized model | W02 | valid/invalid schema corpus |
| F02 | stable glob/capture and prepare virtual tree | F01 | race, escape, sensitive-file tests |
| F03 | deterministic ustar+zstd writer/extractor | F02 | byte goldens, bomb/escape fuzzing |
| F04 | streaming Gump reader/writer over capsule-lib v0001 semantics | W02 | capsule-lib cross-goldens; complete malformed table corpus |
| F05 | crypto transcript, seal, sign, verify | F04 | independent known-answer vectors |
| F06 | local materialization and driver ABI | F01,F03 | native/script contract suite |
| F07 | `gump run` and `gump test` | F05,F06,T01 | local parity acceptance |

## 3. Cluster-memory and transport workstream

| ID | Deliverable | Depends | Exit evidence |
|---|---|---|---|
| C01 | protocol schemas, envelope, version negotiation | W03 | wire goldens and compatibility tests |
| C02 | rustls/Quinn transport abstraction | C01 | mTLS, limits, reconnect, rotation |
| C03 | RAM-only OpenRaft log/state/snapshot adapter | W02,W05 | no-write proof; 1/2/3/5 simulation |
| C04 | typed record state machine and budgets | C03 | transaction/property/budget tests |
| C05 | revisions, watches, compaction, leases | C04 | slow-watch and expiry simulation |
| C06 | membership init/join/drain/remove | C02,C05 | learner transfer and joint-change suite |
| C07 | controller election and fences | C05,C06 | stale-effect replay suite |
| C08 | CLI/server local Unix API | C01,C07 | peer-auth and machine-output goldens |

## 4. Secrets, identity, and authorization workstream

| ID | Deliverable | Depends | Exit evidence |
|---|---|---|---|
| S01 | policy engine/action matrix | W02,C04 | deny-by-default coverage tests |
| S02 | release signer enrollment/trust | S01,F05 | revocation and scope matrix |
| S03 | software unseal/share ceremony | F05 | vectors, zeroization/failure tests |
| S04 | external HSM/KMS unseal provider trait | S03 | fake-provider conformance suite |
| S05 | ephemeral node enrollment/certificates | C02,S01 | restart/no-key-file tests |
| S06 | in-memory custody replication | S03,S05,C05 | reseal/transfer/failure simulation |
| S07 | scoped secret delivery and fd/env injection | S06 | wrong-scope replay and canary scan |
| S08 | external durable audit sink trait | S01,C04 | required-sink fail-closed tests |

## 5. Deployment and object workstream

| ID | Deliverable | Depends | Exit evidence |
|---|---|---|---|
| D01 | object connector contract and fake store | W02 | overwrite/conflict/fault suite |
| D02 | S3 quarantine and immutable publication | D01,F04 | real S3-compatible integration |
| D03 | streamed ingress verification | D02,F05,S02 | peak-memory and corrupt-input suite |
| D04 | declaration normalization/signing/acceptance | D03,C04,S01 | concurrent generation tests |
| D05 | deploy receipt, wait, retry, orphan handling | D04,C08 | workflow acceptance matrix |
| D06 | inventory/inspect/reintroduce | D03,S06 | full-loss recovery rehearsal |
| D07 | explicit purge plan and authorization | D02,S08 | exact-target/retention tests |

## 6. Placement and runtime workstream

| ID | Deliverable | Depends | Exit evidence |
|---|---|---|---|
| R01 | capability reports and resource ledgers | C04 | fake host/capability corpus |
| R02 | hard-filter scheduler and explain reasons | R01,C07 | deterministic candidate matrices |
| R03 | scoring/headroom/resource envelopes | R02 | stability/performance/property tests |
| R04 | atomic independent reservation/admission | R02,C05 | crash and stale-capability tests |
| R05 | gang reservation and launch barrier | R04 | all-or-none 1,024-unit simulation |
| R06 | attempt roots and native supervision | F06,R04,S07 | process-tree/cleanup/isolation suite |
| R07 | script driver | R06 | shared driver conformance |
| R08 | OCI driver | R06 | shared contract and digest/mount suite |
| R09 | checks, retry, finite/continuous completion | R06 | lifecycle state-machine suite |
| R10 | isolation grace and reconnect reconciliation | R09,C07 | partition/fence tests |

## 7. Telemetry and integration workstream

| ID | Deliverable | Depends | Exit evidence |
|---|---|---|---|
| T01 | Ratatouille callback adapter and canonical identity | W02 | upstream contract corpus plus source-forgery tests |
| T02 | stdout/stderr binary-safe capture | T01 | long-line/binary/saturation tests |
| T03 | bounded local ring and subscriber API | T02,C01 | gap/backpressure/window tests |
| T04 | authenticated batch relay and keeper selection | T03,C02 | node-loss/transfer/overflow tests |
| T05 | typed resource observation path | R01,T01 | forged-report and bounded-summary tests |
| I01 | publication provider trait | R09,S01 | fake-provider conformance |
| I02 | optional Kismet adapter | I01 | absent/present/lease/fence suite |
| I03 | output/checkpoint connector capability hooks | S01,R06 | no-hidden-persistence tests |

## 7.1 Hiccup discovery workstream

| ID | Deliverable | Depends | Exit evidence |
|---|---|---|---|
| H01 | health GET declaration, authenticated POST delivery, and bounded JSON codec | R09,C01 | legacy-health and strict-detection corpus |
| H02 | latest-presence replacement and health-derived expiry | W02,W05 | property and deterministic expiry tests |
| H03 | agent identity/IP stamping, token, and topic authorization | R04,S01,H01 | spoofing/scope/fence matrix |
| H04 | keeper selection, replication, transfer, and quotas | H02,H03,C02 | loss/partition/overload simulation |
| H05 | bounded rotating POST delivery and health-independent degradation | H01,H04 | churn, omission, and health-isolation suite |
| H06 | language-neutral corpus and Rust reference SDK | H05 | request/response goldens and adversarial codec tests |

## 8. Integration slices

These slices continuously combine the final components; none authorizes a
throwaway architecture:

1. Local manifest → deterministic Capsule → verified local run.
2. One server → S3 → live declaration → native finite execution → cleanup.
3. One server → continuous process → readiness → Ratatouille subscription.
4. Three memory members → leader loss → fenced continued reconciliation.
5. Three agents → independent spread and rolling replacement.
6. Gang admission → rank delivery → member failure → group policy.
7. Total memory loss → empty init → explicit Capsule reintroduction.
8. Hiccup `@self` discovery → direct peer connection → movement and restart reconciliation.
9. Kismet all-node deployment → Hiccup candidate discovery → authenticated Kismet membership → optional publish/withdraw.
10. OCI and GPU-capability fixtures through the same placement/driver contracts.

Every slice adds evidence to the same contracts and remains in CI.

## 9. Ownership boundaries

- Format team owns exact bytes, not deployment policy.
- Memory team owns committed state semantics, not scheduling choices.
- Scheduler owns plans and reservations, not process control.
- Agent owns local effects only under a current fence.
- Security team owns primitives/provider contracts and reviews every secret path;
  it does not invent a parallel state store.
- Connectors own external side effects and receipts, never desired state.
- CLI owns truthful workflow language, not alternate semantics.

Changes crossing a boundary need reviewers from both owners and new
cross-component evidence.

## 10. First implementation backlog

The first mergeable tickets are W01–W05, F01, F04, C01, T01, and D01. Their
interfaces should be reviewed together before code lands. The first end-to-end
milestone is integration slice 1; the first server milestone is slice 2; the
release candidate requires all ten slices and all gates in `CONFORMANCE.md`.

This order minimizes late protocol discovery. It is not permission to ship a
SQLite-backed, file-backed, service-only, container-only, or Kismet-dependent
interim design.
