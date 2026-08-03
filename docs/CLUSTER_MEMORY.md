# Gump Cluster Memory

> Status: working draft 0.1  
> Purpose: define the distributed in-memory K/V contract used by Gump  
> This is an internal coordination subsystem, not a user-facing general-purpose database.
> Concrete v1 records, wire behavior, and OpenRaft selection are normative in
> [`v1/PROTOCOL.md`](v1/PROTOCOL.md) and [`v1/DECISIONS.md`](v1/DECISIONS.md).

## 1. Thesis

The distributed K/V store is the memory of a living Gump cluster. It remembers desired workloads, membership, authority, placement, and execution state while at least one valid memory lineage survives. It creates no database file, write-ahead log, snapshot, or other durable Gump state.

One server is a complete cluster with one memory copy and zero failure tolerance. As servers join, the same live state is transferred and replicated. Losing every copy creates a new empty cluster; Capsules in S3 remain inert until explicitly reintroduced.

## 2. Boundaries

Cluster memory stores small control records:

- Workload declarations, active generations, and current policy
- Controller epochs and fencing tokens
- Member and node identity, capability, heartbeat, and lease state
- Placements, reservations, execution units, attempts, roles, and ranks
- Retry, progress, readiness, publication, cancellation, and completion state
- Idempotency records and bounded recent transition history
- Secret-custody membership and non-secret key identifiers
- Materialization references and eviction eligibility

It never stores:

- Capsule or application bytes
- Plaintext runtime configuration, encryption keys, or secret values
- Ratatouille event streams
- Application data, checkpoints, or outputs
- A durable audit trail
- Arbitrary user K/V entries

The K/V protocol is private to Gump. Exposing it as a general database would enlarge its security boundary, create durability expectations, and invite unrelated state to become coupled to cluster survival.

## 3. Membership and topology

Every Gump process may activate the **memory-member** role. Deployment policy decides which servers do so; workload-only agents need not hold cluster memory.

The first server starts with `--init` and creates a one-member group. Other servers start with `--join <seed-address>`, authenticate the seed, present enrollment authority, receive the current state in memory, and enter membership through a coordinated configuration change. The seed has no permanent status after joining completes.

Membership changes use a joint old/new configuration so two overlapping groups cannot both believe they exclusively own authority. A joining member is non-voting until state transfer and verification complete. A leaving member stops receiving new state before its authority is revoked.

### 3.1 Topology guarantees

Gump reports topology as facts rather than a single “HA” label:

| Memory members | Memory-loss tolerance | Safe mutation availability |
|---:|---:|---|
| 1 | 0 failures | Available while the member lives |
| 2 | 1 lost copy without total memory loss | Normally freezes after either member becomes unavailable |
| 3 | 1 member failure | Continues with a two-member majority |
| N | Depends on surviving copies | Requires the configured majority/quorum |

Two members can preserve memory after one failure even when they cannot safely accept new mutations. Gump displays **memory survival** and **mutation availability** separately.

## 4. Consistency model

Authoritative mutations are serialized through a crash-fault-tolerant consensus protocol operating entirely in memory. The design does not require a particular named algorithm, but it requires equivalent observable semantics:

- A single monotonically increasing live revision
- Linearizable authoritative reads and writes
- Compare-by-revision, compare-by-value, and compare-absent predicates
- Atomic multi-key transactions needed for fencing and placement
- Revisioned watches
- Lease-attached records
- Idempotency keys for retried client operations

Scheduling hints and dashboards may use explicitly marked stale observations. Authority checks, controller fencing, membership, secret delivery, placement admission, and lifecycle mutations may not.

An acknowledged write is held by the required live replication quorum. It is not durable across loss of every memory member.

## 5. Revisions, transactions, and watches

Every successful mutation advances the live revision. A transaction evaluates all comparisons against one revision and applies all writes atomically or none.

At minimum, transactions support:

- Create only if absent
- Replace or delete only at an expected revision
- Compare a controller epoch and fencing token
- Atomically reserve a coordinated placement group
- Atomically advance one workload generation
- Attach or transfer records to a lease
- Record an idempotent result with the mutation it protects

Watches begin after a known revision and return ordered changes. History is bounded. If a watcher falls behind compaction, it receives an explicit `compacted` result, performs a consistent relist, and resumes from the returned revision. No consumer may interpret a disconnected watch as evidence that state is unchanged.

## 6. Leases and time

Leases represent liveness, not durability. They cover controller authority, member heartbeats, placements, reservations, subscriptions, and other state that must disappear after its owner is gone.

Lease expiry is decided by the authoritative memory group using bounded monotonic timing; individual wall clocks never order mutations. Renewal has a deadline and jitter. A partitioned owner cannot extend a lease without quorum and cannot create new effects after its fencing token expires.

Agents may continue already-running workloads during disconnection according to explicit workload policy, but they cannot infer renewed authority from process survival.

## 7. Record classes and bounded memory

Every key belongs to a declared record class with a size limit, count limit, ownership rule, and retention rule.

### 7.1 Authoritative live records

Desired workload state, active generation, membership, controller authority, and current finite-execution terminal state remain until an authorized lifecycle operation replaces or forgets them. These records are never silently evicted.

If the authoritative budget is exhausted, Gump rejects the mutation that would grow it and explains the responsible namespace and record class. It does not sacrifice existing authority to appear available.

### 7.2 Leased records

Heartbeats, placements, reservations, subscriptions, and ephemeral ownership disappear with their leases. Limits exist per cluster, namespace, workload, and owner so one producer cannot exhaust cluster memory.

### 7.3 Bounded history

Transition history, failure samples, idempotency results, resource envelopes, and completed attempt details retain bounded counts or time windows. Compaction preserves current semantic state while discarding superseded explanation detail.

The effective budgets, current use, rejected growth, compaction revision, and oldest retained revision are observable through `gump cluster status` and `gump explain`.

## 8. Placement transactions

An independent unit reservation binds workload generation, execution, unit, attempt, node, resources, controller epoch, and lease in one transaction.

Gang admission uses one logical transaction:

1. Compute the complete candidate group.
2. Compare controller authority, node capability revisions, and available reservations.
3. Reserve every unit or none.
4. Deliver assignments carrying the same placement-group identity and fence.
5. Open the launch barrier only after every required agent confirms admission.
6. Expire the group reservation if the barrier cannot open within its deadline.

Partial placement cannot become useful work unless the workload explicitly declares elastic membership.

## 9. Partitions and recovery

Only a quorum side may accept authoritative mutations. Minority members retain their memory copy but stop controller elections, placement, retry, secret authorization, and membership changes.

When connectivity returns, minority members discard divergent uncommitted work and catch up from the authoritative lineage. Stale controller and placement fences remain invalid.

If a two-member cluster permanently loses one member, the survivor preserves memory but ordinarily cannot prove that the missing member is dead rather than partitioned. Forced recovery therefore requires an operator to fence or destroy the missing member and authorize a new cluster incarnation. The surviving state is transferred into that incarnation, credentials and fencing epochs rotate, and the old member cannot rejoin without fresh enrollment.

If no memory copy survives, there is nothing to recover. `--init` creates a new empty cluster and selected Capsules must be explicitly reintroduced.

## 10. Planned restart and upgrade

State transfer happens over authenticated cluster transport directly into memory. It never stages a snapshot on disk.

For a rolling restart:

1. Join or confirm enough current members to preserve the desired guarantee.
2. Drain one memory member from voting membership.
3. Restart or replace it.
4. Rejoin and transfer current state.
5. Repeat.

A planned restart of the sole member is allowed and intentionally loses live memory. Gump warns with the exact consequence but does not impose a high-availability requirement.

Mixed versions must agree on consensus behavior and all authoritative record schemas before a member may vote. Unknown mandatory fields or lifecycle states prevent admission.

## 11. Security and fault model

Members authenticate mutually and every membership change is authorized. Records have strict schemas, bounds, and writer roles. State transfer is encrypted, integrity-checked, and bound to cluster identity and incarnation.

The initial model tolerates crashes, loss, delay, duplication, reordering, and network partitions. It does not claim Byzantine consensus. A compromised voting member may disclose non-secret control metadata, disrupt availability, or attempt protocol abuse. It cannot read Capsule secrets merely by reading K/V memory because plaintext secrets and seal keys do not enter this subsystem.

Protocol violations, impossible revisions, invalid signatures, or corrupt state-transfer frames quarantine the sender and surface a security event.

## 12. Operator-visible status

Gump always makes the current guarantee inspectable:

```text
Cluster incarnation:       7f4c...
Memory members:            1
Voting members:            1
Memory copies:             1
Memory-loss tolerance:     0 failures
Mutation availability:     available while this member lives
Current revision:          1842
Compaction floor:          1720
Authoritative memory:      412 KiB / 64 MiB
Leased memory:             1.8 MiB / 32 MiB
```

Warnings state consequences without inventing product tiers. One member is valid. Two members preserve an additional memory copy. Three or more can provide majority progress according to topology.

## 13. Testable invariants

1. No K/V operation performs a required disk write.
2. A one-member cluster implements the same transaction and watch semantics as a larger cluster.
3. At most one live controller fence can create accepted effects.
4. No minority partition accepts authoritative mutations.
5. A joining member cannot vote before verified state transfer completes.
6. No record class has an unbounded key, value, count, or history policy.
7. Authoritative live records are never silently evicted under memory pressure.
8. A compacted watcher is forced to relist rather than miss state silently.
9. Loss of every member yields an empty new cluster, never reconstructed desired state.
10. Plaintext runtime configuration and Capsule bytes never enter K/V records or state transfer.

## 14. Design decisions resolved for v1

The frozen answers are indexed in
[`v1/RESOLUTION_MAP.md`](v1/RESOLUTION_MAP.md). These questions remain as the
design history and as candidates for later memory-protocol profiles.

1. Which in-memory consensus implementation and proof strategy satisfy the required semantics?
2. Which servers activate the memory-member role by default as a cluster grows?
3. What default memory budgets and compaction windows fit beta, ordinary, and very large clusters?
4. What exact operator proof is required for forced recovery from a surviving non-quorate member?
5. How are cluster incarnations compared and rejected without durable Gump state?
6. Which record schemas require end-user signatures in addition to authenticated Gump writers?
