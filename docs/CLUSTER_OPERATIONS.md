# Gump Cluster Operations Patterns

> Status: product design registry; replacement is the first detailed candidate
> pattern. A field becomes normative for `gump/1` only when it is frozen into
> the v1 implementation pack, schemas, reason codes, and conformance fixtures.

## 1. Purpose

Cluster operation is a mature engineering domain. Gump should reuse mechanisms
that have survived real failure rather than invent novel names for familiar
control loops. It should also avoid inheriting assumptions merely because they
are conventional in container platforms.

This document is the home for operational patterns that act on already accepted
Gump intent: replacement, draining, scaling, maintenance, rebalancing, member
changes, and related reconciliations. It records:

- the outcome the operator requested;
- the state machine and bounded algorithm Gump uses;
- the declared workload properties on which the algorithm may rely;
- failure, cancellation, fencing, and total-loss behaviour;
- what Gump adopts from established systems;
- what Gump deliberately does not copy.

Delivery order is separate. The maximal product design may contain a complete
operation before every part is implemented.

## 2. Operating principles

Every cluster operation follows these rules:

1. **Desired outcome, not imperative choreography.** The operator commits an
   outcome and policy. A fenced controller reconciles toward it.
2. **No workload mythology.** Gump does not infer HTTP, statelessness,
   containerization, public traffic, stable ports, or service semantics.
3. **Use declared semantics.** Lifetime, coordination, health, completion,
   publication, topology, resources, and supersession determine which
   algorithms are valid.
4. **Every external effect is fenced.** Starts, stops, secret delivery,
   publication, and connector effects validate generation, attempt, and
   controller authority.
5. **Bound everything.** Parallelism, unavailability, surge, retries, grace,
   observation windows, and progress all have explicit ceilings.
6. **Absence of evidence is not success.** Process existence, readiness,
   publication, application analysis, and completion remain distinct facts.
7. **Pause safely when uncertain.** An operation that cannot prove its next
   effect is authorized makes no new effect and reports why.
8. **Rollback is forward history.** It creates a new generation referencing an
   old Capsule. Gump never rewrites history or pretends time moved backward.
9. **Operations are live memory.** Their current state is replicated in cluster
   RAM, not persisted as a hidden database. Total cluster-memory loss leaves a
   new empty cluster and requires explicit Capsule reintroduction.
10. **Optional products stay optional.** Kismet, Ringtail, an analysis service,
    or a publication provider may improve an operation only when the committed
    policy explicitly selects that capability.

## 3. Common operation model

An operation has:

```text
identity         stable operation UUID
subject          workload, execution, unit set, node set, or member set
source           committed state from which the operation began
target           requested committed outcome
policy           normalized bounds and gates with provenance
authority        controller epoch, fence, and expected revisions
phase            current state-machine phase
observations     bounded current evidence and reason codes
deadline         absolute or committed-time progress boundary
```

Submitting the same operation identity and request is idempotent. Reusing that
identity for different bytes is a conflict. A newer semantic request creates a
new generation or operation rather than mutating the meaning of an old one.

Status must distinguish at least:

```text
accepted intent
planned effects
effects in progress
observed condition
converged outcome
paused / failed / cancelled outcome
```

An accepted request is never reported as a completed operation.

## 4. Pattern registry

| Pattern | Purpose | Design status |
|---|---|---|
| Replacement and rollout | Move a workload from one generation to another | Detailed below |
| Planned node drain | Remove useful work before maintenance or loss | Candidate |
| Server membership replacement | Preserve memory quorum while replacing Gump servers | Partly specified in `CLUSTER_MEMORY.md` |
| Scale and coverage change | Change fixed units or reconcile `all_nodes` coverage | Candidate |
| Placement repair | Replace attempts lost through node or process failure | Partly specified |
| Rebalance | Improve placement without turning observation into constant churn | Candidate |
| Capacity acquisition and optimization | Add, reshape, or remove provider capacity through an optional autoscaler | [Concept documented](CAPACITY_AUTOSCALER.md) |
| Capacity pressure and preemption | Admit higher-priority work under declared policy | Candidate |
| Secret/configuration rotation | Replace attempts or use a future dynamic delivery profile | Candidate |
| Coordinated-workload transition | Replace or resize a gang behind an application barrier | Candidate |
| Finite-work supersession | Cancel, resume, checkpoint, or create a new execution explicitly | Candidate |
| Cluster binary upgrade | Replace Gump members without losing memory or wire compatibility | Candidate |
| Full-loss reintroduction | Rebuild live intent explicitly from inert Capsules | Specified in v1 recovery documents |

Patterns are added when assembly or failure testing reveals a repeated operation.
They are not added merely to reproduce another scheduler's object catalogue.

## 5. Pattern 1: workload replacement

### 5.1 Outcome

Replacement converges a workload from source generation `g` to accepted target
generation `g+1` while respecting its declared availability, coordination,
resource, health, publication, and failure policy.

Canary is a phase within replacement. It is not a universal replacement mode.

### 5.2 Applicability

| Declared workload shape | Normally valid modes | Required caution |
|---|---|---|
| Continuous, independent, multiple units | Progressive; blue/green; recreate | Availability and surge bounds |
| Continuous, independent, singleton | Blue/green when coexistence and handoff exist; otherwise recreate | Downtime must be explicit |
| `all_nodes` continuous coverage | Progressive destructive or surge-per-node | Fixed ports, exclusive resources, one-unit-per-node invariant |
| Coordinated/gang | Coordinated barrier or whole-group recreate | No unit-wise rolling inference |
| Finite execution | New execution, cancel-and-replace, or declared resume | Never silently rerun completed work |
| Exclusive device or host resource | Destructive progressive or recreate | Surge may be impossible |

The controller rejects a mode that contradicts the normalized workload
contract. It does not silently substitute a different operation.

### 5.3 Replacement modes

`progressive`
: Start with deterministic canaries, then proceed in bounded waves. Old and new
  attempts coexist only within surge and coordination policy.

`blue_green`
: Admit the complete target set alongside the source set, establish eligibility,
  switch an explicitly declared handoff such as publication, then retire the
  source. This requires enough capacity and a real handoff boundary.

`recreate`
: Stop the affected source attempts before starting their replacements. This is
  valid for fixed-port, singleton, exclusive-device, or explicitly disruptive
  workloads. The resulting unavailability is part of the plan, not a surprise.

`coordinated`
: Prepare the target group and cross an application-declared barrier or hook.
  Gump owns placement and fencing; the workload owns its protocol, state
  transfer, compatibility, quorum, and split-brain prevention.

`new_execution`
: Create a distinct finite execution. Prior success is never erased and a
  running finite execution is not implicitly cancelled.

### 5.4 State machine

```text
ACCEPTED
   |
   v
PREFLIGHT -----> PAUSED
   |
   v
CANARY --------> PAUSED / ABORTING
   |
   v
AWAITING_PROMOTION
   |
   v
PROGRESSING ---> PAUSED / ABORTING
   |
   v
FINALIZING
   |
   v
CONVERGED

ABORTING -> ABORTED
PAUSED   -> CANARY | PROGRESSING | ABORTING
```

`FAILED` is reserved for a terminal outcome selected by policy or an impossible
invariant. A deadline does not imply rollback; it produces the declared pause,
abort, or rollback action.

### 5.5 Algorithm

1. Verify the target Capsule is durably present and trusted before accepting
   target intent.
2. Commit target generation `g+1` and a rollout record comparing source
   generation, current controller fence, and operation identity.
3. Normalize the effective mode, budgets, gates, deadlines, topology cohorts,
   and policy provenance.
4. Preflight target capabilities, resource headroom, named-port feasibility,
   secret custody, driver availability, publication capability, and coordination
   constraints. Preflight is advisory where the world can change, but known
   impossibility fails before disruption.
5. Select canaries deterministically from each required capability or failure
   cohort. Reconciliation after controller loss selects the same set.
6. Create target attempts with new attempt identities and target-generation
   fences. Preserve source attempts as required by availability policy.
7. Wait for each declared promotion gate and continuous healthy interval.
8. If promotion is manual or external, enter `AWAITING_PROMOTION` without
   consuming further availability budget.
9. Reconcile further waves while both unavailability and surge budgets permit.
10. Where publication is declared, make eligible target endpoints available
    before withdrawing their source counterparts when zero interruption is
    required. Without a declared handoff, Gump claims no traffic transition.
11. Fence and stop superseded attempts through their declared termination
    sequence. Hiccup presence and publication leases disappear with the old
    attempt.
12. Converge only when the complete target set satisfies its declared condition,
    no unauthorized source attempt remains, and required publication is current.

### 5.6 Canary and cohort selection

Canaries are selected by stable hashing over operation identity, target
generation, unit identity, and cohort. A cohort is created only from declared
or trusted placement facts that can materially change behaviour, such as:

- architecture or operating system;
- driver and isolation profile;
- accelerator/device class;
- zone or failure domain when policy requires it;
- materially distinct network or publication capability.

"One canary" means one across the operation only when one cohort exists.
Gump must not validate an x86 release on one node and infer that an ARM or GPU
cohort is safe.

### 5.7 Budgets

Replacement budgets are independent constraints:

- `canaries`: initial target attempts or a bounded percentage by cohort;
- `max_unavailable`: maximum desired units not satisfying the source-or-target
  availability condition;
- `max_surge`: maximum temporary units above target cardinality;
- `max_parallel`: maximum replacement effects initiated in one wave;
- `min_healthy_time`: continuous declared health before an attempt can promote;
- `healthy_deadline`: maximum time for one target attempt to become healthy;
- `progress_deadline`: maximum time without operation progress;
- `stop_grace`: per-attempt declared termination bound.

Absolute values are normalized before the rollout begins. Percentage rounding
is explicit in the committed declaration. `max_unavailable = 0` and
`max_surge = 0` is invalid when replacement requires a new process. Surge never
overrides scheduler admission or resource limits.

For `all_nodes`, budgets apply to the eligible-node snapshot used by that wave.
Newly eligible nodes do not accidentally join an unpromoted canary generation;
their treatment is explicit in the rollout policy.

### 5.8 Promotion evidence

Promotion policy is one of:

- `automatic`: declared gates pass for `min_healthy_time`;
- `manual`: an authorized operator promotes the exact operation revision;
- `external`: a typed analysis provider returns a bounded, authenticated result.

Automatic promotion may use only declared authoritative evidence. Process state
does not become readiness. Readiness does not become publication. Ringtail or
other best-effort telemetry may assist humans and external analysis, but cannot
silently become the sole safety gate—especially when the telemetry system is
itself being replaced.

If no meaningful readiness or progress gate exists, automatic canary promotion
must be an explicit policy choice rather than an inferred default.

### 5.9 Failure actions

`pause`
: Start no further replacement effect. Keep healthy source and target attempts
  within budget and expose the blocking evidence. This is the safest general
  default.

`abort`
: Fence and remove target attempts while retaining or restoring source attempts
  that remain authorized by the current rollout record.

`rollback`
: Submit a new generation referencing a prior Capsule. It is not an internal
  mutation of `g+1`. Automatic rollback is valid only when explicitly selected;
  schema migration or application incompatibility may make it more dangerous
  than pausing.

An old Capsule can be retrieved from S3 and unsealed again. No local release
cache or plaintext runtime material is required for rollback correctness.

### 5.10 Generation and identity semantics

- Target acceptance creates exactly one next generation.
- The source generation may remain operationally eligible during rollout; it
  is not the target desired generation.
- Unit identity may survive a planned replacement when policy preserves its
  logical role. Attempt identity never survives a process start.
- Every promotion, pause, resume, and abort compares the rollout revision and
  controller fence.
- A stale controller cannot advance a wave, publish a target, or terminate a
  source.
- Concurrent deploy, scale, rollback, and policy changes either compose through
  an explicit transaction or conflict. Last-writer-wins is forbidden.

### 5.11 Proposed manifest vocabulary

The existing `deploy.rollout` shape should evolve toward the following. Exact
field names remain non-normative until schema freeze:

```toml
[deploy.replacement]
mode = "progressive"
canaries = 1
max_unavailable = 1
max_surge = 0
max_parallel = 1
min_healthy_time = "10s"
healthy_deadline = "5m"
progress_deadline = "10m"
promotion = "automatic"
on_failure = "pause"
```

Deployment flags and cluster policy may override release defaults only through
authorized intent. The committed declaration records every effective value and
its provenance.

### 5.12 Operator contract

`gump plan` should explain before acceptance:

- source and target generations and Capsule digests;
- affected units/nodes and deterministic canaries;
- whether overlap is possible;
- worst-case declared unavailability and temporary resource surge;
- gates, observation windows, promotion mode, and deadlines;
- publication handoff behaviour;
- what pause, abort, and rollback will mean;
- any cohort or capacity that cannot currently be satisfied.

`gump observe` should report the rollout phase, source/target counts, current
budget use, gate evidence, last progress, blocking reason, and safe available
actions. Machine output retains unit, attempt, node, and generation provenance.

### 5.13 Replacement invariants

1. No wave exceeds normalized unavailability, surge, or parallelism bounds.
2. No target attempt starts without target-generation authority and fresh
   secret authorization.
3. No source attempt is stopped merely because target intent was accepted.
4. Canary selection is deterministic across controller replacement.
5. Publication never points to an ineligible attempt.
6. An attempt cannot belong to both source and target generations.
7. A fixed-port or exclusive-resource conflict cannot be hidden as surge.
8. Finite success is never converted into a rerun by continuous rollout logic.
9. Coordinated work never receives independent rolling semantics by inference.
10. Rollback creates a new generation and preserves historical causality.
11. Telemetry loss cannot deadlock replacement or be mistaken for success.
12. Total cluster-memory loss resumes no rollout automatically.

## 6. What Gump borrows—and what it does not

### 6.1 Kubernetes

Useful mechanisms:

- separate `maxUnavailable` and `maxSurge` budgets;
- a minimum-ready interval rather than promoting on one successful sample;
- progress deadlines and explicit stalled conditions;
- controlled per-node replacement for node-covering workloads;
- partitioned or staged rollout as an operator-controlled boundary.

Deliberate departures:

- Gump has no Pod, Deployment, StatefulSet, or DaemonSet ontology. Equivalent
  algorithms arise from declared cardinality, coordination, resources, and
  lifecycle rather than workload-kind objects.
- Readiness is not assumed to mean safe traffic or correct application state.
- A disruption budget is not a detached object whose interaction with every
  controller is ambiguous; the effective availability contract is normalized
  into the operation being executed.
- Rollout history is not a hidden durable control-plane database.
- Percentage defaults and rounding never remain implicit after acceptance.

### 6.2 HashiCorp Nomad

Useful mechanisms:

- canary followed by explicit or automatic promotion;
- bounded parallel replacement;
- `min_healthy_time`, per-attempt health deadline, and overall progress
  deadline as different concepts;
- pause/observe before promotion;
- blue/green as a full-size canary set rather than a separate mystical system;
- system-wide node coverage receiving a destructive canary interpretation when
  only one unit may occupy a node.

Deliberate departures:

- Gump does not treat "task running" as sufficient health unless policy
  explicitly accepts process state.
- Automatic revert is not a harmless universal default.
- Canary count does not imply traffic routing; publication or another declared
  handoff must do that work.
- Update behaviour is not tied to a container/task-group model.

The reference mechanisms are described in the official
[Kubernetes Deployment rolling-update documentation](https://kubernetes.io/docs/tasks/run-application/update-deployment-rolling/),
[Kubernetes DaemonSet update documentation](https://kubernetes.io/docs/tasks/manage-daemon/update-daemon-set/),
and [Nomad update specification](https://developer.hashicorp.com/nomad/docs/job-specification/update).

## 7. Candidate patterns to develop next

### 7.1 Planned drain

Stop new placements, transfer or replace current work within a disruption
budget, remove publication, then declare the node empty. Memory membership
drain is a related but separate quorum operation. A node running both roles must
satisfy both state machines.

### 7.2 Placement repair versus rebalance

Repair restores declared cardinality after failure and may act urgently.
Rebalance improves an already valid placement and needs hysteresis, benefit
thresholds, cooldown, and a disruption budget. Gump must not churn workloads
merely because a marginally better node appeared.

### 7.3 Scaling and `all_nodes`

Scaling changes desired cardinality. It is not replacement, though scale and
replacement may conflict or be transactionally composed. `all_nodes` reacts to
eligibility changes and must define how new nodes interact with an in-progress
rollout.

### 7.4 Capacity pressure and preemption

Preemption needs explicit priority, victim eligibility, disruption accounting,
and a proof that removing victims can admit the target. Observed memory or CPU
consumption alone must not become an unbounded eviction loop.

Acquiring, vertically reshaping, consolidating, and removing the underlying
machine supply is a separate optional-product pattern described in
[`CAPACITY_AUTOSCALER.md`](CAPACITY_AUTOSCALER.md). Gump owns the deficit,
drain, membership, and fence; a Capsule-deployed provider owns the cloud effect.

### 7.5 Coordinated transitions

Gang and distributed workloads need prepare, admit, barrier, abort, and cleanup
semantics across the whole declared group. Application state transfer remains
outside Gump, but Gump must make the handoff identities, endpoints, and fences
available safely.

### 7.6 Gump server upgrades

Workload replacement and Gump-member replacement are different operations.
Member upgrade must preserve wire/schema compatibility, memory quorum, custody,
and controller fencing while members drain, restart, rejoin, and receive state
directly into memory.

## 8. How a pattern becomes part of Gump

A candidate pattern is frozen only after it has:

1. a state machine with typed operations and stable reason codes;
2. normalized bounds, rounding, deadlines, cancellation, and retry rules;
3. authority, authorization, idempotency, and fencing rules;
4. explicit behaviour for one-node, quorum loss, partition, and total loss;
5. workload-shape compatibility and rejection rules;
6. truthful human and machine observation contracts;
7. deterministic simulation and property tests;
8. a disposable three-node live rehearsal including interruption and recovery;
9. traceability into the implementation pack and schema fixtures.

We borrow algorithms only after translating them into Gump's product truths:
arbitrary workloads, sealed Capsules, memory-only control state, explicit
capabilities, and optional integrations.
