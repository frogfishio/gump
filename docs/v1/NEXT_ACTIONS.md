# Gump v1 — Implementation Actions

> Status: Kanban-ready actions derived from the implementation review  
> Purpose: finish the specified product before entering productionisation

This backlog does not change or reduce the v1 architecture. It converts the
remaining implementation and integration gaps into bounded delivery actions.
The existing normative documents remain authoritative.

## 1. Rules for using this backlog

Every card created from this document must:

- link the applicable v1 contract section and delivery IDs;
- name its owner and dependencies;
- include bounded failure, cancellation, authorization, and redaction behavior;
- add automated evidence before being marked complete;
- update `spec/v1/traceability.tsv` only when the named evidence exists and passes;
- preserve the zero-durable-state and arbitrary-workload invariants.

Passing unit tests for an isolated component is not sufficient when the card's
acceptance criteria require an integrated path.

## 2. Immediate correctness and project-truth actions

### GUMP-N001 — Contain runtime paths beneath the verified release

**Priority:** P0  
**Maps to:** F06, R06, INV-002, INV-014

Reject absolute paths, parent traversal, symlink traversal, and race-based
escape for runtime command, script, workdir, attempt root, and cleanup targets.
Use descriptor-relative/beneath-root operations where the operating system
supports them and a fail-closed equivalent elsewhere.

**Acceptance evidence:**

- native and script commands cannot escape through `..` or symlinks;
- workdir cannot resolve outside the release;
- cleanup cannot remove anything outside its owned attempt root;
- adversarial race tests cover command, workdir, and cleanup;
- the normal native/script driver suites continue to pass.

### GUMP-N002 — Make Capsule creation genuinely streaming

**Priority:** P0  
**Maps to:** F03, F04, D03, CONFORMANCE performance gates

Replace the segment `Vec<u8>` accumulator and full-Capsule assembly with a
bounded-memory writer. Stream the application archive from its spill file,
compute segment metadata and signing material without retaining full payloads,
and write the final Capsule without multiple complete copies in memory.

**Acceptance evidence:**

- a multi-GiB synthetic archive can be packaged without memory proportional to
  archive or Capsule size;
- peak packaging memory is measured and stays within the published envelope;
- output remains byte-compatible with capsule-lib and checked-in goldens;
- corrupt and short-read cases leave no apparently complete Capsule.

### GUMP-N003 — Restore a clean, truthful CI baseline

**Priority:** P0  
**Maps to:** W04, CONFORMANCE section 9

Fix the current formatting failure. Keep the structural traceability check for
ordinary development, add a mandatory strict gate to release/tag builds, and
ensure every `missing` or `blocked` ledger row has an owned open Kanban card.
Remove the possibility of an empty Kanban while the release ledger is open.

**Acceptance evidence:**

- format, Clippy, workspace tests, and the MinIO integration job pass;
- release workflow fails while any v1 invariant is `missing` or `blocked`;
- a CI check fails if a non-implemented invariant has no ticket reference;
- `--prove-missing` remains only a gate self-test, not a completion signal.

## 3. One-server product actions

These cards produce the first actual server product and integration slices 2
and 3. One server is feature-complete but truthfully reports zero failure
tolerance.

### GUMP-N004 — Compose the product into the `gump` binary

**Priority:** P0  
**Maps to:** C08 and implementation-pack product boundary
**Depends on:** GUMP-N003

Add `gump server --init ...` and role selection to the main binary. Replace the
single-accept demonstration with a cancellable, concurrent service runtime.
Compose explicit interfaces for cluster memory, transport, connectors,
scheduler, agent, telemetry, and secret custody rather than constructing a
parallel state path.

**Acceptance evidence:**

- one `gump` artifact acts as CLI, server, memory member, and agent;
- server accepts repeated and concurrent authenticated local requests;
- startup and graceful shutdown cleanly stop all owned tasks;
- the standalone demonstration server is removed or made an internal test tool.

### GUMP-N005 — Run a real one-member OpenRaft cluster

**Priority:** P0  
**Maps to:** C03–C07, INV-004–INV-007
**Depends on:** GUMP-N004

Instantiate OpenRaft with the RAM log/state/snapshot adapters and the typed
cluster state machine. Controller authority, revisions, watches, leases,
idempotency, and desired state must flow through committed Raft entries.

**Acceptance evidence:**

- `gump server --init` forms a one-voter cluster;
- mutations and reads use the live Raft node rather than direct model calls;
- restart begins empty and does not infer intent from S3 or node files;
- status reports one memory member and zero failure tolerance;
- no-write observation proves Gump creates no durable cluster state.

### GUMP-N006 — Implement the cluster-backed local API and CLI client

**Priority:** P0  
**Maps to:** C08, CLI lifecycle contract
**Depends on:** GUMP-N004, GUMP-N005

Extend the authenticated local protocol with versioned operations required by
deploy, observation, lifecycle, recovery, and cluster administration. Make CLI
commands clients of this API; do not duplicate server semantics in the CLI.

**Acceptance evidence:**

- bounded frames, deadlines, cancellation, peer authorization, and stable error
  codes apply to every operation;
- CLI reconnection and interruption behavior matches the lifecycle contract;
- incompatible protocol versions fail clearly;
- machine-output goldens cover every initial operation and error class.

### GUMP-N007 — Implement real protected configuration packaging

**Priority:** P0  
**Maps to:** F05, S02–S04, INV-001, INV-002
**Depends on:** GUMP-N002

Resolve declared runtime values only in the developer process, validate the
public variable schema, encrypt the protected configuration segment, wrap its
DEK for configured recovery authority, and sign the complete release.

**Acceptance evidence:**

- real manifest variables and secrets round-trip through a Capsule;
- public Capsule bytes expose names/contracts but no protected values;
- unset, malformed, cancelled, and provider-error cases fail before upload;
- seeded canaries are absent from public bytes, errors, telemetry, and temporary
  filesystem artifacts;
- tampering at every Capsule layer prevents intent acceptance and execution.

### GUMP-N008 — Implement one-node unseal and in-memory custody

**Priority:** P0  
**Maps to:** S03–S06
**Depends on:** GUMP-N005, GUMP-N007

Connect software 1-of-1 recovery and the HSM/KMS provider trait to server
startup and Capsule activation. Plaintext custody material must exist only in
hardened memory and be explicitly zeroized on replacement, reseal, shutdown,
and error paths.

**Acceptance evidence:**

- software and fake external providers pass the same activation contract;
- unavailable or unauthorized providers fail closed;
- restart reseals the cluster and requires authority to activate new work;
- core, swap, proc, inherited-descriptor, error, and telemetry canary checks pass
  for the advertised isolation profile.

### GUMP-N009 — Deliver secrets only to the authorized current attempt

**Priority:** P0  
**Maps to:** S07, R06, INV-013
**Depends on:** GUMP-N008

Implement scoped env and file-descriptor injection after placement admission.
Bind delivery to cluster, workload, release, unit, attempt, node, controller
epoch, placement fence, and declared variable name/form.

**Acceptance evidence:**

- authorized applications receive exactly their declared values;
- wrong node/release/unit/attempt/fence/scope replays are rejected;
- protected values never enter release roots, attempt roots, K/V, S3, telemetry,
  status, explanation, or errors;
- delivery material is zeroized and descriptors are closed after launch/failure.

### GUMP-N010 — Wire the one-server deploy transaction

**Priority:** P0  
**Maps to:** D01–D05, integration slice 2
**Depends on:** GUMP-N005–GUMP-N009

Connect packaging, verification, immutable S3 publication, authorized live
intent acceptance, committed idempotency, observation, and orphan reporting.
Remove the process-local deploy receipt/idempotency cache from authoritative
runtime use.

**Acceptance evidence:**

- `gump deploy` performs upload → intent → execution as one truthful workflow;
- the same operation ID replays its committed result after lost replies;
- different content under the same operation ID conflicts;
- upload followed by intent failure reports an inert orphan;
- directly uploaded Capsules never execute;
- a real MinIO/S3-compatible integration exercises the complete path.

### GUMP-N011 — Implement minimum arbitrary-workload placement

**Priority:** P0  
**Maps to:** R01–R04
**Depends on:** GUMP-N005

Implement node capability reports, resource ledgers, hard filtering, stable
explain reasons, scoring, reservation, and admission. Do not assume a web
service, port, container, CPU-only workload, or continuous lifecycle.

**Acceptance evidence:**

- finite native, continuous native, script, GPU-requesting, and portless
  fixtures receive correct feasibility results;
- reservations are committed atomically before launch;
- stale capability and stale-fence admissions fail;
- unschedulable output explains every rejected hard requirement;
- resource/accounting structures are bounded.

### GUMP-N012 — Implement the agent reconciliation and supervision loop

**Priority:** P0  
**Maps to:** R06, R09, R10, INV-014
**Depends on:** GUMP-N009–GUMP-N011

Turn `gump-agent` into a fenced effect executor. Reconcile accepted placements,
materialize only fully verified Capsules, create owned attempt roots, invoke the
driver ABI, capture output, report observation, and clean up complete process
trees and writable state.

**Acceptance evidence:**

- finite native execution reaches terminal success and cleans up;
- continuous execution remains supervised until intent changes;
- daemonizing, descendant, crash, cancellation, and forced-kill fixtures leave
  no owned process or writable root;
- stale effects cannot start, stop, publish, refresh, or report attempts;
- controller isolation follows the declared grace policy.

### GUMP-N013 — Implement lifecycle checks and restart/completion policy

**Priority:** P0  
**Maps to:** R09, integration slice 3
**Depends on:** GUMP-N012

Implement startup, liveness, readiness, completion, retry, backoff, stop signal,
timeout, finite/continuous, and gang-member lifecycle semantics exactly as
declared. Health is optional and must not imply a workload type.

**Acceptance evidence:**

- fixtures cover workloads with no checks, HTTP checks, command checks, finite
  completion, continuous restart, and permanent failure;
- checking cannot block reconciliation indefinitely;
- readiness and publication are never inferred when undeclared;
- retry and terminal reasons remain bounded and explainable.

### GUMP-N014 — Connect Ratatouille telemetry end to end

**Priority:** P0  
**Maps to:** T01–T05, integration slice 3, INV-009
**Depends on:** GUMP-N006, GUMP-N012

Route stdout, stderr, supervisor events, and typed resource observations through
bounded local rings and authenticated relay. Implement `gump telemetry` with
recent-window replay, live subscription, gaps, filtering, and safe identity.

**Acceptance evidence:**

- child output pressure never blocks the child or control plane;
- binary, huge-line, high-rate, subscriber-lag, keeper-loss, and source-forgery
  suites pass;
- telemetry remains memory-only and reports drops/gaps honestly;
- protected data and Hiccup payload content are not emitted by Gump itself.

### GUMP-N015 — Complete developer-facing deploy and observation commands

**Priority:** P0  
**Maps to:** D05, C08, CLI workflow acceptance
**Depends on:** GUMP-N010–GUMP-N014

Implement `gump deploy`, `status`, `explain`, and `telemetry`, including stable
human and machine output. A deployment receipt must distinguish persistence,
intent acceptance, scheduling, start, readiness, publication, completion, and
loss of observation.

**Acceptance evidence:**

- wait conditions and defaults match the declared workload contract;
- one-node durability is visible in all successful mutation receipts;
- interruption does not falsely imply rollback;
- explanations read committed/observed state and disclose compaction;
- no command prints protected values.

### GUMP-N016 — Implement explicit full-loss recovery

**Priority:** P0  
**Maps to:** D06, INV-003, INV-004, INV-018, integration slice 7
**Depends on:** GUMP-N007–GUMP-N015

Implement `inventory`, `inspect`, `reintroduce`, and `reintroduce --plan` against
the object store. Recovery must start from an empty cluster and create fresh
intent only for explicitly selected Capsules.

**Acceptance evidence:**

- total cluster-memory loss followed by restart shows zero desired work;
- inventory lists verified inert Capsules without activating them;
- finite work requires explicit new-execution or external-checkpoint resume;
- recovery rehearsal activates only the selected Capsule;
- corrupt, unauthorized, or undecryptable Capsules remain inert.

### GUMP-N017 — Implement one-node Hiccup discovery

**Priority:** P0 for developer preview  
**Maps to:** H01–H03, H05–H06, INV-019–INV-024
**Depends on:** GUMP-N009, GUMP-N013

Create the `gump-hiccup` crate and implement exact health-response detection,
authenticated POST delivery, bounded codecs, latest-presence replacement,
health-derived expiry, Gump-stamped identity/IP, attempt tokens, topic policy,
and a Rust reference SDK corpus.

**Acceptance evidence:**

- legacy health behavior is unchanged without exact opt-in;
- two instances using `@self` receive current stamped introductions;
- wrong tokens, topics, identities, addresses, attempts, and fences receive no
  discovery view;
- Hiccup degradation never alters health or workload lifecycle;
- Gump sends no application traffic after introduction.

## 4. Developer-preview gate

Developer preview is reached only when GUMP-N001 through GUMP-N017 pass together
and the corresponding conformance evidence is recorded. The required user story
is:

1. initialize one disposable server;
2. package real files and protected values;
3. run `gump deploy` against real S3-compatible storage;
4. accept live intent through one-member RAM Raft;
5. place, unseal, inject, execute, observe, and clean up a native workload;
6. observe it through status/explain/Ratatouille;
7. discover another instance through one-node Hiccup;
8. lose the server, restart empty, and explicitly reintroduce the Capsule.

This gate begins product usability. It is not the v1 release candidate.

## 5. Multi-server and full-v1 actions

### GUMP-N018 — Implement ephemeral node enrollment and join

**Priority:** P1  
**Maps to:** C02, C06, S05
**Depends on:** developer-preview gate

Implement `gump server --join <seed>` with replay-resistant enrollment,
ephemeral certificates, learner transfer, verified snapshot installation, and
joint promotion. No node identity or key material may be written to disk.

**Acceptance evidence:** join, crash-during-transfer, replay, certificate
rotation, remove-and-return, and rolling-replacement suites pass.

### GUMP-N019 — Run live multi-member Raft and fenced reconciliation

**Priority:** P1  
**Maps to:** C03–C07, R10, integration slices 4–5
**Depends on:** GUMP-N018

Connect Raft networking over authenticated QUIC, controller election, watches,
agent sessions, isolation grace, and reconciliation across leader/member loss.

**Acceptance evidence:** the complete one-, two-, three-, and five-member fault
matrix passes, including minority freeze, stale-effect rejection, catch-up, and
rolling replacement.

### GUMP-N020 — Replicate and transfer secret custody

**Priority:** P1  
**Maps to:** S06–S07
**Depends on:** GUMP-N018, GUMP-N019

Replicate custody material only among current authorized memory custodians and
transfer it safely during membership change. Define and implement reseal behavior
when the custody threshold is lost.

**Acceptance evidence:** threshold, transfer, custodian loss, replay, scope, and
member-removal simulations pass without durable secret artifacts.

### GUMP-N021 — Complete scheduler breadth

**Priority:** P1  
**Maps to:** R03–R05, R01/T05, INV-008, INV-028
**Depends on:** GUMP-N019

Complete headroom/scoring, enforced resource envelopes, typed observation,
gang reservation/barrier, GPU and arbitrary capability matching, stable spread,
and continuous `all_nodes` coverage.

**Acceptance evidence:** deterministic candidate/property suites, 1,024-unit
gang simulation, synthetic GPU fixtures, node join/drain/capability changes, and
the scheduler performance envelope pass.

### GUMP-N022 — Implement the OCI driver

**Priority:** P1  
**Maps to:** R08, INV-012
**Depends on:** GUMP-N012, GUMP-N021

Implement OCI through the same driver ABI, authority, fencing, secret, telemetry,
resource, process-tree, and cleanup semantics as native and script execution.

**Acceptance evidence:** the shared driver contract plus digest, mount,
namespace, limit, secret, and cleanup adversarial suites pass.

### GUMP-N023 — Complete distributed Hiccup

**Priority:** P1  
**Maps to:** H04–H05, INV-025–INV-027, integration slice 8
**Depends on:** GUMP-N017, GUMP-N019

Implement deterministic keeper selection, bounded replication, transfer,
quotas, rotating delivery, omission semantics, and rebuild from health refresh.

**Acceptance evidence:** keeper loss, partition, churn, overload, movement,
restart, expiry, and 10,000-entry topic envelopes pass without affecting Raft or
health progress.

### GUMP-N024 — Implement lifecycle, cluster, and purge commands

**Priority:** P1  
**Maps to:** C06, D07, CLI lifecycle contract
**Depends on:** GUMP-N016, GUMP-N019

Implement start, stop, scale, cancel, forget, purge, doctor, cluster status,
join-plan, drain, and remove with exact durability-layer semantics and stable
machine output.

**Acceptance evidence:** every destructive action resolves and previews its
exact scope; stop/forget never purge; purge never mutates intent; membership
commands preserve quorum safety; noninteractive behavior never prompts.

### GUMP-N025 — Implement optional provider integrations

**Priority:** P1  
**Maps to:** I01–I03, INV-011, integration slice 9
**Depends on:** GUMP-N013, GUMP-N019, GUMP-N023

Implement publication, output, and checkpoint provider traits plus the optional
Kismet adapter. Providers receive fenced effects and receipts but never become
authoritative desired-state stores.

**Acceptance evidence:** absent/present/degraded/provider-loss suites pass;
non-Kismet workloads behave identically without Kismet; Kismet deployed with
`--nodes=all` forms through Hiccup without a seed list.

## 6. Release-candidate evidence actions

### GUMP-N026 — Close every conformance invariant

**Priority:** P1  
**Maps to:** INV-001–INV-028
**Depends on:** GUMP-N018–GUMP-N025

Implement the exact invariant tests, link their stable evidence paths in the
traceability ledger, and remove every `missing` and `blocked` status. Do not mark
an invariant implemented based only on a narrower unit test.

### GUMP-N027 — Complete parser fuzzing and adversarial security suites

**Priority:** P1  
**Maps to:** CONFORMANCE sections 2 and 5
**Depends on:** GUMP-N026

Add fuzz targets and retained corpora for every required parser and implement
the full secret, filesystem-race, process-isolation, transport-replay, resource
exhaustion, provider-error, Hiccup, and object-conflict matrices.

### GUMP-N028 — Establish and meet performance envelopes

**Priority:** P1  
**Maps to:** CONFORMANCE section 7
**Depends on:** GUMP-N021, GUMP-N023, GUMP-N027

Create reproducible benchmarks on a published reference host and enforce the
memory, latency, scheduler, Capsule, archive, telemetry, and Hiccup gates.
Bounds may be calibrated transparently; unbounded behavior may not be waived.

### GUMP-N029 — Run release soak and independent security review

**Priority:** P1  
**Maps to:** CONFORMANCE section 8
**Depends on:** GUMP-N026–GUMP-N028

Run the 24-hour mixed-workload soak, rolling-replacement rehearsal, full-loss
recovery rehearsal, and an independent security review. Critical/high findings
must be resolved before release-candidate approval.

## 7. Productionisation actions

Productionisation begins only after the developer-preview product gate passes.
Release-candidate productionisation additionally includes:

### GUMP-N030 — Build reproducible release artifacts and provenance

Produce supported-platform binaries, checksums, signatures, SBOMs, source and
compiler provenance, license/dependency policy evidence, and a documented
reproducible release procedure.

### GUMP-N031 — Package safe service operation

Provide service definitions and installation guidance with private runtime
directories, least privilege, memory/core/swap policy, file-descriptor limits,
network requirements, S3 permissions, HSM/KMS configuration, and clean removal.

### GUMP-N032 — Publish operator and incident runbooks

Document init/join/drain/remove, quorum loss, reseal, S3 failure, provider
failure, stuck placement, telemetry loss, full-memory loss, reintroduction,
upgrade/rollback, and evidence collection without relying on local log files.

### GUMP-N033 — Define compatibility and upgrade policy

Test protocol negotiation, Capsule compatibility, rolling replacement, CLI/API
machine-output stability, unsupported-version behavior, and rollback boundaries.

## 8. Recommended first board population

Create and assign these cards immediately:

- **Parallel safety track:** GUMP-N001, GUMP-N002, GUMP-N003
- **Server composition track:** GUMP-N004, then GUMP-N005 and GUMP-N006
- **Protected configuration track:** GUMP-N007, then GUMP-N008 and GUMP-N009
- **First integrated product path:** GUMP-N010 through GUMP-N016
- **Developer-preview discovery:** GUMP-N017

Do not start new feature expansion while these cards are open unless
implementation exposes a contract defect that genuinely requires a design
decision. The current v1 specification already supplies the required product
scope.
