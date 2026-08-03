# Gump v1 Runtime Contracts

> Status: normative

## 1. Node capability model

Every agent publishes a leased, revisioned capability report. Capability names
and values are facts, never inferred from workload type:

- OS, architecture, kernel, page size, and execution drivers;
- logical CPU, allocatable millicores, memory, and ephemeral storage;
- GPU/accelerator vendor, model, device ID, memory, interconnect, locality, and
  share/isolation capability;
- network interfaces, zones, racks, failure domains, and named labels allowed
  by cluster policy;
- isolation features: namespaces, cgroups, seccomp, user separation, memory
  locking, swap control, `/proc` restriction, ptrace restriction, core-dump
  suppression, and OCI runtime;
- connector reachability and publication-provider availability;
- current reservations and observed host pressure.

Each protection is `ENFORCED`, `OBSERVED`, or `UNAVAILABLE`. Placement requiring
enforcement rejects `OBSERVED` and `UNAVAILABLE`.

## 2. Scheduler pipeline

Placement always executes these stages in order:

1. Validate declaration and authority.
2. Expand independent units or the complete gang.
3. Filter hard capabilities, architecture, isolation, secret-custody reach,
   connector requirements, policy, quota, and topology.
4. Check declared reservations plus conservative headroom.
5. Score feasible candidates by resource fit, observed envelope, correlated
   pressure, spread/affinity requests, data locality hints, movement cost, and
   disruption budget.
6. Commit reservations and fences atomically in cluster memory.
7. Ask agents to admit against current local facts.

Hard filters precede scores. A priority or heuristic cannot bypass a hard
requirement. Every rejection and score component has a stable reason code and
bounded numeric evidence available through `gump explain`.

For a release without trustworthy history, the scheduler reserves declared
requests plus the larger of 20% or cluster-policy minimum headroom. Missing
requests use policy defaults and are shown as assumed, never as observed fact.

## 3. Resource observations

Agents sample process-tree CPU, resident/anonymous/file memory, faults, I/O,
ephemeral bytes/inodes, process count, accelerator utilization/memory, start
latency, readiness latency, and exit behavior when the host can observe them.

Observations are typed, bounded, and aggregated into startup, steady, burst,
and shutdown envelopes using count, exponentially aged mean/variance, p50,
p95, p99, and maximum. Raw samples are not put in consensus. A leader-authorized
aggregator periodically commits a bounded summary identified by release,
driver, capability class, and observation generation.

Untrusted application self-report may enrich diagnostics but cannot reduce a
reservation or establish a hard capability. Rebalancing needs sustained
pressure, a 10-minute default cooldown, disruption permission, and predicted
benefit above movement cost.

## 4. Driver ABI

The internal Rust trait is versioned semantically as `gump.driver/1`:

```text
probe(host) -> DriverCapabilities
prepare(ReleaseRoot, RuntimeSpec, AttemptContext) -> PreparedHandle
admit(PreparedHandle, ResourceGrant, SecretPlan) -> Admission
start(Admission, StartFence, IoEndpoints) -> RunningHandle
observe(RunningHandle) -> ObservationStream
signal(RunningHandle, Signal) -> Outcome
terminate(RunningHandle, Deadline) -> Outcome
kill(RunningHandle) -> Outcome
cleanup(PreparedHandle) -> Outcome
```

Calls are cancellable, deadline-bound, and idempotent by attempt ID and fence.
Handles are process-local opaque capabilities and are never serialized into K/V.
A driver cannot read cluster credentials, mutate desired state, publish an
endpoint, fetch arbitrary Capsules, or retain secret values after cleanup.

`prepare` occurs only after Capsule verification and creates a private attempt
root. `admit` proves current local feasibility without starting useful work.
`start` requires the latest accepted fence and, for gangs, an open barrier.
`cleanup` owns the entire process tree, mounts, namespace, descriptors, and
writable root and must be safe after partial failure.

## 5. Native driver

- Executes only a packaged executable or a cluster-policy-approved host binary.
- Uses no shell and performs no PATH search for a relative application command.
- Runs under a dedicated ephemeral OS identity where available.
- Creates a process group/cgroup before child execution and supervises the full
  descendant tree, not only the initial PID.
- Applies enforceable CPU, memory, process, file-descriptor, and ephemeral
  limits before useful execution.
- Installs secret environment/descriptors only in the final child setup path.
- Defaults core dumps off and child dumpability off.

Native execution without an enforcement capability is allowed only when the
declaration and cluster policy accept that explicit weaker profile.

## 6. Script driver

The script driver is the native driver plus an explicit interpreter capability.
Interpreter path and arguments are argument arrays in the release contract.
The interpreter is resolved from packaged material or a versioned host
capability. `/bin/sh -c` is never implicit.

## 7. OCI driver

OCI is an execution driver input, not Gump's outer package. The Capsule contains
an OCI image layout or bundle as public application material. The driver:

- verifies every OCI blob digest after Capsule verification;
- rejects registry pulls not explicitly declared as an external dependency;
- resolves entrypoint/argument composition exactly as normalized metadata says;
- maps declared resources and isolation to the local OCI runtime;
- reports which controls were actually enforced;
- uses memory-backed secret mounts or inherited descriptors, never layer files;
- owns container and descendant cleanup under the attempt fence.

## 8. Secret injection ABI

Environment injection is limited to required UTF-8, NUL-free values. It is
compatible but weaker: same-identity process inspection may expose it. Policy
can require descriptor injection for `secret` values.

Descriptor injection creates an anonymous memory-backed file, writes the value,
seals it against size/write/grow/shrink changes where supported, rewinds it,
sets the declared inherited descriptor number, and optionally injects a public
environment reference such as `/proc/self/fd/7`. The value never has a pathname
in the release or attempt root. The parent closes its copy immediately after
successful spawn.

Secret delivery is scoped to exact cluster, workload, release, execution,
attempt, node, generation, fence, variable set, and short expiry. A retry after
expiry requires reauthorization. Agents zeroize staging buffers on every exit
path.

## 9. Supervision

The agent starts stdout and stderr drains before the child can produce output.
It then records start, observes readiness/liveness/completion contracts, renews
its placement lease, applies declared retry policy, and reports transitions.

Continuous and finite semantics differ only by declaration:

- finite success may require any unit or all units to exit zero;
- continuous success is not inferred from exit zero and normally restarts under
  its policy;
- independent failure affects one unit unless policy says otherwise;
- gang failure applies the group failure rule;
- maximum attempts, backoff, jitter, reset window, and terminal failure are
  explicit and bounded.

Default retry backoff is exponential from 1 second to 5 minutes with 20% jitter.
No default retry is added where the manifest did not request one.

## 10. Isolation and disconnection

An agent that cannot validate current authority may continue an already-running
attempt for the effective isolation grace, but it may not:

- start or restart an attempt;
- open a gang barrier or change rank;
- receive or redeliver secrets;
- renew external publication;
- create new connector effects;
- accept a lower or equal divergent fence.

`stop_on_isolation` terminates immediately after a short confirmation window;
otherwise v1 defaults to 15 minutes. Reconnection validates generation and
fence before resuming authority. Grace expiry terminates and cleans the attempt.

## 11. Health and completion checks

Checks are optional typed contracts: process, TCP, HTTP, command, file-descriptor
signal, or external provider. They have explicit interval, timeout, thresholds,
initial delay, success definition, and maximum output. Check commands run in the
attempt isolation context without access to undeclared secrets.

Readiness controls eligibility, not process liveness. Liveness can trigger the
declared failure policy. Progress is diagnostic unless a deadline explicitly
makes it terminal. Completion is only evaluated for a declared finite contract.

## 12. Publication provider ABI

```text
probe() -> ProviderCapabilities
reconcile(PublicationIntent, Endpoint, Fence) -> ProviderReceipt
status(ProviderReceipt) -> PublicationStatus
withdraw(ProviderReceipt, Fence) -> Outcome
```

Calls are idempotent, bounded, cancellable, and scoped by authorization. The
provider receives only its endpoint, normalized provider-specific intent, and
publication credential capability. It cannot receive runtime values unrelated
to publication or mutate Gump state directly.

The Kismet adapter maps a Gump eligible unit to Kismet's local authenticated
service publication contract, maintains a bounded lease, and withdraws on loss
of eligibility. Absence is `CAPABILITY_UNAVAILABLE` only when the deployment
requires Kismet; otherwise it is irrelevant.

## 13. Object-store connector ABI

```text
begin_quarantine(cluster, capsule, expected_len) -> Upload
write(Upload, chunk) -> Progress
finish_quarantine(Upload, digest) -> ObjectEvidence
publish_if_absent(Quarantine, FinalKey, digest, len) -> ObjectEvidence
head(FinalKey) -> ObjectEvidence
get(FinalKey, optional_range) -> ByteStream
delete(ExactKey, Preconditions) -> Outcome
abort(Upload) -> Outcome
```

The S3 connector never sees runtime plaintext. It uses exact keys, checksum and
length evidence, TLS, bounded retry, and least-privilege credentials. Promotion
must not overwrite a different object. Inventory is an operator read operation,
not a recovery or reconciliation input.

## 14. Telemetry capture and wire profile

Each attempt has authoritative source fields:

```text
cluster_id, namespace, app_id, workload_id, release_id,
execution_id, unit_id, role, rank, attempt_id, node_id,
process_stream, agent_incarnation, local_sequence
```

Application-supplied Ratatouille source fields are retained under `producer`,
never allowed to replace these fields. Canonical topics are lowercase ASCII,
1–128 bytes, slash-separated. Gump reserves `gump/`; captured streams use
`app/stdout` and `app/stderr`.

Capture is byte-preserving. Each pipe is read in at most 32 KiB chunks. A record
contains raw bytes, stream-local sequence, chunk flags (`BEGIN`, `CONTINUE`,
`END`), optional UTF-8 hint, monotonic receive offset, and diagnostic wall time.
Lines up to 64 KiB are emitted as one record; longer lines and binary streams
are chunked. Reconstruction is possible within received chunks, but delivery,
global ordering, and durability are not promised.

Gump feeds normalized records through Ratatouille's callback sink and bounded
filtering. Remote delivery uses authenticated Gump QUIC `TelemetryBatchV1`, not
Ratatouille's plain HTTP/TCP sinks. Queues drop oldest by default and increment
per-topic and per-reason counters. Supervision never awaits telemetry capacity.

On normal exit the agent drains both pipes to EOF and permits a bounded 250 ms
local relay flush. Node death may lose unreplicated records. Subscribers receive
up to the configured recent window, then live data, with explicit gap markers.

## 15. Telemetry keepers

Keeper selection uses domain-separated BLAKE3 rendezvous hashing over attempt
ID and eligible node IDs. Two distinct failure domains are preferred. Keepers
hold independent bounded RAM rings; they do not participate in Raft and do not
acknowledge durability.

Membership change calculates old/new keeper sets, streams the still-live recent
window when capacity permits, and emits a transfer gap when it cannot. A node
must reserve control-plane memory before accepting keeper work. Telemetry memory
is evicted before any authoritative memory budget is threatened.

## 16. Cleanup and zero footprint

On terminal attempt or revoked placement the agent:

1. withdraws publication;
2. closes new secret delivery and zeroizes held material;
3. sends the declared graceful signal;
4. drains telemetry until EOF or deadline;
5. kills the entire process tree after timeout;
6. unmounts and closes anonymous secret objects;
7. removes the exact attempt root;
8. marks release materialization evictable when no live attempt references it.

Orphan reconciliation only deletes paths beneath the configured Gump state root
whose validated ownership marker matches the local process incarnation and no
current attempt. It never scans the machine to infer live desired state.

