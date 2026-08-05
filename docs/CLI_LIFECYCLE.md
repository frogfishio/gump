# Gump CLI and Lifecycle

> Status: working draft 0.1  
> Purpose: make Gump's live-memory and inert-Capsule model obvious through its commands
>  
> v1 command and default-wait decisions are frozen in
> [`v1/DECISIONS.md`](v1/DECISIONS.md).

## 1. Product contract

Gump uses ordinary verbs and reports what each action changes. It never blurts infrastructure internals at a developer, but it never hides durability semantics either.

Every server-side operation distinguishes three layers:

```text
Capsule:   immutable sealed bytes in S3
Intent:    desired workload state in live distributed memory
Execution: observed processes and resources on transient nodes
```

Changing one layer does not silently change another.

## 2. Core workflow

```text
gump run
gump test
gump deploy
gump status
gump explain
gump telemetry
```

`gump run` and `gump test` use the same manifest and runtime contract locally without creating a server-side Capsule or intent unless an explicit sealed verification mode is selected.

`gump deploy` prepares, packages, seals, signs, uploads, creates live intent, and follows the requested convergence condition as one user action.

An existing Capsule can be deployed directly without its source manifest:

```text
gump deploy kismet.capsule --nodes=all
```

`--nodes=all` is continuous coverage: one eligible unit on every current and
future matching node. It is not expanded into a one-time fixed unit count.

## 3. Deployment receipt

Successful deployment prints a stable, machine-readable and human-readable receipt:

```text
Application: accounts-service
Release:     7f4c... / blake3:...

Capsule:     persisted in s3://.../capsules/7f4c....capsule
Intent:      accepted at cluster revision 1842
Execution:   converging — 2/3 units eligible
Durability:  1 memory member; live intent has zero failure tolerance
```

The receipt never says “deployed” while leaving unclear whether that means uploaded, accepted, started, ready, published, or completed.

An S3 upload followed by K/V failure leaves an inert orphan Capsule and reports it. Retrying with the same transaction identity does not create duplicate live intent.

## 4. Observation and explanation

### 4.1 `gump status`

Shows current desired and observed state by logical workload identity. It includes generation, Capsule, execution, units, attempts, placement, declared lifecycle condition, publication state when applicable, Hiccup active/degraded status and safe counts when detected, and the current memory-survival guarantee. It does not print Hiccup `data` or `secretData` by default.

### 4.2 `gump explain`

Answers “why?” using stable reason codes and human explanations:

- Why is this unit on this node?
- Why is it unschedulable?
- Which hard requirement rejected each candidate?
- Why is a gang waiting?
- Why did a unit restart or move?
- Why is a workload ready but unpublished?
- Why is Hiccup undetected, degraded, incomplete, overloaded, or unauthorized?
- Which policy, quota, or priority decision applied?
- Which value came from the manifest, an override, or cluster policy?

Explanation is a product feature, not merely debug logging. It reads current K/V state and bounded transition evidence and admits when older evidence has been compacted.

### 4.3 `gump telemetry`

Subscribes by application, execution, role/rank, unit, attempt, node, or topic. It calls the result live or recent telemetry, never durable logs.

## 5. Live-state verbs

### 5.1 `gump stop <workload>`

Stops running units while retaining the workload declaration, Capsule reference, policy, and terminal observation in live K/V memory. It does not delete the Capsule. A later `gump start` creates a fresh execution under the retained intent.

For finite work, `stop` is not success. `gump cancel` is the explicit terminal operation that prevents further attempts under the current execution.

### 5.2 `gump scale <workload> <units>`

Creates a new live generation or authorized intent revision. It validates workload coordination rules; it cannot arbitrarily resize a non-elastic gang.

### 5.3 `gump cancel <execution>`

Prevents new attempts, withdraws publication if any, terminates units under policy, and records cancellation in live memory. It does not imply that external side effects were rolled back.

### 5.4 `gump forget <workload>`

Removes the workload and its retained live history from distributed K/V memory after stopping or cancelling it. Agents sweep unreferenced materializations. The raw Capsule remains in S3.

The command previews affected live objects and requires explicit authorization. It is idempotent. Forgetting is the normal expression of Gump's zero-footprint model.

## 6. Capsule-store verbs

### 6.1 `gump inventory`

Lists and verifies inert Capsules from the configured S3 namespace. It displays public metadata, signatures, size, creation annotations, and whether the current live cluster references each Capsule.

After total cluster-memory loss, “unreferenced” means only “not in this new cluster.” It never means unused, obsolete, previously completed, or safe to delete.

### 6.2 `gump inspect <capsule-or-workload>`

Shows normalized public Capsule metadata and, when live state exists, the effective declaration and provenance. It never prints protected values.

### 6.3 `gump reintroduce <capsule-uuid>`

Verifies and unseals an existing Capsule, presents its public workload contract, and asks for fresh live intent. It never restores assumed prior unit count, completion, placement, or publication from S3.

Finite work requires an explicit choice:

```text
--new-execution
--resume-from <external-checkpoint-reference>
```

Noninteractive use must provide that decision explicitly.

### 6.4 `gump purge <capsule-uuid>`

Deletes raw recovery material from S3. Purge is separate from stop and forget because it has different consequences and authority.

Before deletion, Gump:

1. Resolves the exact Capsule UUID and object-store namespace.
2. Verifies that the current live K/V state has no reference.
3. Warns that total-memory loss prevents Gump from proving historical unreachability.
4. Displays object retention, versioning, replication, and recovery implications.
5. Produces a reviewable deletion plan.
6. Requires explicit confirmation or a signed noninteractive authorization.

Object lock or retention policy may reject the purge. Gump never weakens those controls.

## 7. Recovery and diagnostics

### 7.1 `gump doctor`

Checks the live cluster without mutating it:

- K/V membership, quorum, revisions, memory budgets, and watches
- Controller fencing
- Node reachability and capability reports
- Secret-custody and seal state without exposing keys
- S3 read/write/verify capability using safe probes
- Capsule integrity and unseal readiness when explicitly selected
- Publication and data connectors
- Hiccup health binding, token injection, keeper reachability, quotas, and safe topic counts
- Scheduler feasibility for a selected manifest or Capsule

Every result distinguishes required failure, degraded guarantee, optional integration absence, and advisory warning.

### 7.2 `gump reintroduce --plan`

Performs Capsule discovery, verification, unseal-authority checks, capability matching, and proposed-intent rendering without starting work or mutating K/V state. This makes full-loss recovery rehearsable.

## 8. Cluster verbs

Provisioning tools normally invoke:

```text
gump server --init <cluster-params>
gump server --join <seed-address> <join-params>
```

Operators inspect and change membership through:

```text
gump cluster status
gump cluster join-plan
gump cluster drain <node>
gump cluster remove <node>
```

`cluster status` reports memory copies, loss tolerance, mutation availability, current revision, memory budgets, controller authority, and secret-custody health. One member is valid and reported as zero failure tolerance.

Joining a server transfers live memory and adds scheduling capacity without changing manifests or rebuilding Capsules.

## 9. Exit and wait semantics

Commands have stable outcome classes suitable for automation:

- Local validation or preparation failure
- Authentication or authorization failure
- Capsule persistence failure
- Live-intent acceptance failure
- Accepted but unschedulable
- Started but lifecycle condition unsatisfied
- Converged to the requested condition
- Lost observation before the requested condition

`gump deploy` accepts an explicit wait condition such as `accepted`, `started`, `eligible`, `published`, or `completed`. The default is derived from the declared workload contract and printed before waiting. A finite non-networked job never waits for readiness or publication it did not declare.

## 10. Interaction rules

- Human output leads with application meaning; machine output uses a versioned schema.
- Every mutation has an idempotency identity.
- Dry-run and plan output contain no protected values.
- Destructive commands resolve exact targets before confirmation.
- Noninteractive commands never prompt unexpectedly.
- Optional integrations are reported as absent, not as cluster failures.
- Compacted explanation history is disclosed rather than reconstructed from guesses.
- CLI interruption does not roll back already accepted live intent.
- Command names map to one durability layer; no verb secretly deletes both K/V state and S3 bytes.

## 11. Testable invariants

1. `stop` and `forget` never delete a Capsule.
2. `purge` never mutates live workload intent.
3. `inventory` never creates desired state.
4. `reintroduce` never infers prior desired state or finite completion.
5. Deployment output distinguishes Capsule persistence, intent acceptance, and execution convergence.
6. Every destructive command previews an exact scope.
7. A finite workload is never made to wait for an undeclared service condition.
8. One-server operation exposes zero failure tolerance without being rejected.
9. No command prints a protected runtime value.

## 12. Design decisions resolved for v1

The frozen answers are indexed in
[`v1/RESOLUTION_MAP.md`](v1/RESOLUTION_MAP.md). These questions remain as the
design history and as candidates for later command-surface revisions.

1. Which concise top-level command names survive user testing?
2. What wait condition should each common workload contract select by default?
3. How long are idempotency results retained in bounded cluster memory?
4. Is `forget` immediate after termination or may policy require a short undo window in live memory?
5. What signed authorization format supports unattended purge safely?
