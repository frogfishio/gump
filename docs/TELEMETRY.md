# Gump Telemetry with Ratatouille

> Product telemetry design. Concrete v1 bounds and wire behavior are normative
> in [`v1/RUNTIME.md`](v1/RUNTIME.md) and [`v1/DECISIONS.md`](v1/DECISIONS.md).

> Status: working draft 0.1  
> Native library: [`ratatouille`](https://crates.io/crates/ratatouille)  
> Purpose: define Gump's live, bounded, best-effort observability contract

## 1. Thesis

Gump does not build, own, or emulate a conventional durable logging platform. Its native observability stream is Ratatouille: topic-oriented, filtered, sequenced, bounded, and explicitly best-effort.

This directly eliminates the multi-server file-collection problem. Gump does not write per-instance log files and later ask collectors to discover, tail, rotate, ship, merge, and index them. Applications emit relevant topics into memory near the process; Gump attaches placement identity and fans the live stream toward its current consumers.

```text
application
    | filtered Ratatouille records
    v
bounded node-local memory relay
    | authoritative Gump identity envelope
    v
sharded, redundantly held cluster-memory recent window
    +----> live developer subscription
    +----> operational consumer
    +----> optional external durable sink

No file write -> no tailer -> no file rotation -> no node-log reconciliation
```

Telemetry answers “what is happening now?” It does not become hidden control state and does not create a durable cluster dependency.

Ratatouille currently provides:

- Topic filtering, disabled by default
- Per-topic sequence counters
- Text and NDJSON formatting
- Source identity fields
- Pluggable sinks
- Plain HTTP and TCP sinks
- Bounded in-memory HTTP and TCP relays
- Explicit relay flushing
- Queue overflow policies
- Emission, relay, and sink statistics

Gump builds its identity, routing, security, and lifecycle conventions around those primitives without changing Ratatouille into a database.

The producer filter avoids work for disabled topics. Bounded in-memory relays and explicit overflow behavior prevent telemetry pressure from turning into unbounded memory growth or synchronous disk pressure. Ratatouille still has a runtime cost, but that cost is controlled, visible, and separated from application progress.

## 2. Guarantees and non-guarantees

### 2.1 Guaranteed by the Gump integration

- Telemetry production cannot require durable disk writes.
- Buffers have explicit byte or record bounds.
- Every accepted Gump-native event has a topic and authoritative source envelope.
- Per-topic sequence gaps are observable where Ratatouille supplies sequence counters.
- Drops, filter decisions, relay failures, and sink failures are counted.
- Telemetry congestion does not block workload supervision or control-plane convergence.
- Gump drains every supervised process's stdout and stderr and attempts to emit every bounded segment through Ratatouille.
- Gump's own telemetry constructors cannot accept protected runtime-value types.
- Cross-node transport is authenticated and encrypted by Gump even when the application-facing local sink is plain HTTP or TCP.
- A workload restart creates a new attempt identity, so a restarted per-topic sequence is not confused with continuation of the previous process.

### 2.2 Explicitly not guaranteed

- Durable retention
- Delivery of every event
- Global ordering
- Ordering across topics
- Exactly-once delivery
- Replay after total cluster loss
- Complete history for a disconnected client
- Secret redaction from arbitrary application-generated messages
- Audit-grade evidence

An external sink may provide some of these properties after accepting an event. Those become properties of that integration, not retroactive guarantees from Gump or Ratatouille.

## 3. Event identity

Ratatouille's source identity includes `app`, `where`, and `instance`. Gump maps those fields consistently and adds an authenticated outer envelope when telemetry enters the agent:

```text
Ratatouille record
├── topic
├── per-topic sequence
├── message
└── source hint
    ├── app
    ├── where
    └── instance

Gump transport envelope
├── cluster identity
├── application identity
├── release / capsule UUID
├── deployment generation
├── execution identity
├── logical instance identity
├── unit role / rank, when declared
├── placement-attempt identity
├── placement-transition identity, when applicable
├── node identity
├── execution driver
├── receive time
└── original Ratatouille record
```

Application-supplied source fields are hints. The local agent derives the authoritative outer identity from the supervised process and its placement. A workload cannot impersonate another application merely by formatting a different `SourceIdentity`.

For local execution:

- `app` is the manifest application identity.
- `where` is `local` or another explicit local context.
- `instance` is the local run-attempt identity.

For cluster execution:

- `app` is the application identity.
- `where` identifies the Gump cluster context without disclosing unnecessary infrastructure details.
- `instance` is the stable logical instance plus attempt identity, or a documented compact representation of both.

## 4. Topics instead of levels

Ratatouille is organized around topics, not the conventional fatal/error/warn/info/debug/trace hierarchy. Gump preserves that model.

Gump-owned topics use stable namespaces, provisionally:

```text
gump:lifecycle
gump:health
gump:placement
gump:resource
gump:publication
gump:capsule
gump:security
gump:telemetry
app:*
process:stdout
process:stderr
```

Application topics remain application-defined beneath an allowed namespace. Topic names are bounded and validated before they enter cluster relays.

Severity-like concepts, when useful, are data in a topic convention rather than the global routing primitive. Operators subscribe to relevant topics; they do not turn “the log level” up and down across an undifferentiated stream.

## 5. Filtering

Ratatouille emits no topics by default. Gump retains this safe default and derives effective filters from explicitly authorized inputs:

- Application manifest defaults
- Local developer override
- Deployment declaration override
- Cluster policy
- Temporary live subscription

Gump explicitly enables `process:stdout` and `process:stderr` in its pipe-capture adapter for every supervised process. Application filters do not disable pipe draining or initial bounded emission. Downstream routing and recent-window retention remain bounded and may shed records under pressure.

The effective filter and its provenance are observable without revealing event payloads.

Filtering may occur at several stages:

1. **Producer filter** avoids formatting and sending unwanted topics.
2. **Agent filter** protects local relay capacity and enforces cluster policy.
3. **Subscriber filter** selects a view for one CLI or external sink.

A downstream filter cannot recover an event discarded upstream. Gump must therefore show where filtering occurred when diagnosing a missing topic.

Temporary topic enablement is lease-bound and automatically expires. A disconnected debugging client cannot accidentally leave expensive topics enabled forever.

## 6. Application-to-agent transport

The Gump agent exposes a bounded local Ratatouille ingestion endpoint to each supervised workload. Based on currently available Ratatouille sinks, this may be loopback TCP or HTTP. It is never exposed as public ingress.

At process creation, Gump supplies non-secret telemetry bootstrap information through the execution contract:

- Local relay endpoint
- Effective topic filter
- Output format required by the relay, normally NDJSON
- Application source-identity hints
- Maximum record and batch sizes
- Attempt identity

The application integration constructs its Ratatouille logger and sink from those inputs. If the application ignores them, Gump does not claim native telemetry coverage for that application.

Where workload isolation permits one workload to reach another workload's loopback endpoint, the agent must provide per-placement authorization or isolate network namespaces. The authoritative source envelope always comes from the accepted placement endpoint rather than trusting record contents.

Plain HTTP or TCP is acceptable only within the protected local host boundary defined by the execution profile. Gump never forwards that plain transport across nodes. Remote fan-out uses Gump's mutually authenticated cluster transport.

## 7. Bounded relays and backpressure

Every relay declares:

- Queue capacity
- Maximum record size
- Maximum batch size
- Flush interval and trigger
- Overflow policy
- Retry budget
- Connection and write deadlines

Telemetry must not become a denial-of-service path against the agent or workload. When a bounded queue fills, the selected Ratatouille drop policy applies. Gump surfaces cumulative and interval statistics for emitted, filtered, dropped, failed, retried, and successfully flushed records.

Drop accounting itself must remain bounded. Gump aggregates counters rather than emitting one new event for every dropped event and causing recursive overload.

Control-plane commands, health decisions, secret delivery, publication lease renewal, and process supervision never share an exhaustion-prone telemetry queue.

## 8. Sequence semantics

Ratatouille assigns sequence counters per topic. Gump uses them to detect discontinuities within one logger/run identity.

A gap can mean:

- Producer or relay overflow
- Transport loss
- Subscriber-side filtering
- Client disconnection
- Process restart with an incorrectly reused identity

Producer-side filtering may instead appear as silence because a record rejected before sequence assignment need not create a sequence gap. Filter statistics and effective-filter provenance are therefore inspected separately.

Sequence numbers expose loss; they do not repair it. Gump displays gaps and nearby relay counters rather than presenting an apparently continuous history.

Counters are scoped to workload execution, unit attempt, and topic. They are not global cluster offsets and are never used for control-plane ordering.

## 9. Gump's own telemetry

Every Gump role uses Ratatouille internally:

- Local role: preparation, packaging, sealing, upload, and deployment-follow events
- Ingress role: validation, staging, promotion, and declaration-commit events
- Controller role: leadership, scheduling, reconciliation, and policy events
- Agent role: admission, extraction, supervision, health, resources, and publication events
- Custodian role: sealed/unsealed state and authorization outcomes without key material

Security-sensitive topics report identities, algorithms, key versions, decisions, and reason codes—not secret bytes, wrapped-key contents, authentication tokens, or plaintext-derived fingerprints.

Gump uses typed internal event constructors. Protected runtime values are represented by types that do not implement general-purpose string formatting, reducing accidental interpolation into telemetry.

Panics and third-party library diagnostics are treated as unstructured compatibility output unless deliberately adapted into safe typed topics.

## 10. Application telemetry

Applications may link Ratatouille directly and emit `app:*` topics. Gump standardizes only the transport and identity envelope; it does not dictate an application's entire topic vocabulary.

Applications remain responsible for not emitting their own secrets. Gump cannot reliably redact arbitrary text after the application has serialized it, and retaining plaintext values merely to scan outgoing messages would create another exposure surface.

Cluster policy can:

- Disable application telemetry entirely
- Allow or deny topic patterns
- Bound message and aggregate throughput
- Route selected topics to authorized live subscribers
- Route selected topics to external sinks

Application telemetry is never interpreted as desired state, health, readiness, billing truth, or authorization evidence unless a separate authenticated protocol explicitly defines that use.

## 11. Process output capture

Gump owns the stdout and stderr pipes of every supervised process. It continuously drains both so applications cannot block on full pipes and so diagnostic output is detached from the machine's filesystem.

Each stream is segmented into bounded records and emitted into Ratatouille:

- stdout becomes `process:stdout`.
- stderr becomes `process:stderr`.
- Complete text lines are preserved when they fit within bounds.
- Long lines are split without discarding their position.
- Partial final lines are emitted when the pipe reaches EOF.
- Invalid UTF-8 and binary chunks are safely encoded and marked.
- Every record carries the authoritative application, release, logical instance, attempt, and node envelope.

Capture is mandatory and not controlled by an application manifest switch. “Capture” means Gump reads and attempts bounded Ratatouille emission for every byte; it does not mean lossless retention under every failure. Relay overflow may drop complete segments and exposes the resulting counters and sequence gaps.

No mode writes conventional log files. During local execution Gump may render captured streams to the terminal while simultaneously passing them through the same Ratatouille identity and segmentation path.

Direct application Ratatouille instrumentation remains preferable for semantically important events because it provides topics at the source and avoids inferring record boundaries from byte streams. stderr is useful diagnostic evidence, but applications must not use it as the sole channel for mission-critical operational or business events.

When Gump observes process exit, it drains both pipes to EOF and gives the local relay a small bounded flush opportunity. Rescheduling and supervision do not wait indefinitely for telemetry delivery. A whole-node failure can therefore lose bytes that had not yet reached another cluster member.

## 12. Multi-node reconciliation and live consumption

### 12.1 Movement identity

Gump never treats a node path or Unix log filename as the identity of an application's telemetry. Identity is layered:

```text
application
└── deployment generation
    └── logical execution unit / role / rank
        ├── attempt A on node-1
        └── attempt B on node-7
```

The logical execution unit survives a planned replacement when policy chooses to preserve that identity. The attempt never does. Every process start receives a new attempt identity even when it restarts on the same node. Coordinated workloads also retain execution and role/rank identity across their unit attempts.

A placement transition has its own correlation identity. Controller and agent lifecycle topics describe:

- Why the transition began
- Old and new instance attempts
- Old and new nodes
- Release and deployment generation
- Any declared admission, progress, readiness, publication, completion, checkpoint, and termination milestones
- Whether the transition completed, was superseded, or failed

The transition normally becomes visible as:

```text
attempt A / node-1: gump:lifecycle draining
attempt A / node-1: gump:publication withdrawn
attempt B / node-7: gump:lifecycle starting
attempt B / node-7: gump:health ready
attempt B / node-7: gump:publication active
attempt A / node-1: gump:lifecycle stopped
```

Exact ordering varies with rollout policy. Causal identifiers and lifecycle state establish the relationship; wall-clock order and arrival order do not.

Per-topic Ratatouille sequences remain scoped to one attempt and topic. Attempt B does not continue attempt A's counters. A user interface groups both beneath the logical instance while showing the discontinuity explicitly.

### 12.2 Live fan-out

`gump telemetry` subscribes to live Ratatouille topics from agents; `gump tail` may exist as a concise alias for the stdout/stderr-focused view.

The subscription is expressed against logical selectors such as application, generation, execution, role/rank, unit, attempt, topic, or node—not filenames. A cluster subscription coordinator continually resolves those selectors against current placements and fans in streams from every relevant agent. When a unit moves or a coordinated group restarts, the subscription follows the new attempts without requiring the user to discover hostnames.

Before serving consumers, agents forward telemetry to a sharded set of **telemetry keepers**. Keepers retain a bounded recent window in memory, keyed by logical application/instance and subdivided by attempt and topic. Rendezvous or equivalent stable hashing assigns at least two eligible keepers where cluster size permits, avoiding a single central collector while allowing recent events to survive loss of one node.

Each forwarded record has a deduplication identity derived from execution, unit attempt, topic, and sequence. Multiple keeper copies do not become duplicate user-visible events. Keeper membership changes may reduce or restore redundancy, but never turn telemetry into control state.

The command may receive:

- The bounded recent cluster-memory window
- New events after subscription
- Sequence-gap markers
- Filter and drop statistics
- Lifecycle notices for instances entering or leaving the subscription

It does not promise historical retrieval. User-facing language must say “live telemetry” or “recent bounded relay state,” never imply durable logs.

Local `gump run` renders the same topic model directly. This makes local topic filters and application instrumentation testable before deployment.

Fan-in creates a presentation order, not a global event order. The client may sort a short display window by receive time, but it preserves source attempt, topic sequence, gap markers, and causal transition identity. No system decision depends on the merged display order.

If an old node disappears, records already accepted by a surviving keeper remain available through the bounded window. Only records that never left the failed node, or whose keeper replicas also failed, are lost. The new attempt remains observable under the same logical application selector. Durable history beyond the memory window or total cluster failure requires an external sink.

### 12.3 Applications that write Unix log files

An application-managed log file belongs to the application, not to Gump's telemetry model. Gump does not copy, merge, rotate, or migrate `/var/log/...` files when an instance moves.

Gump does not include a distributed file-tail collection system. A legacy application that insists on file logging must bring an application-owned bridge or external collector, accepting that system's operational and performance costs. The native solutions are direct Ratatouille emission and Gump's automatic stdout/stderr capture.

## 13. External sinks

Ratatouille's sink model permits forwarding to HTTP, TCP, or custom callbacks. Gump may expose connector-driven external sinks for operators who want durable search, analytics, or alerting.

An external sink receives only topics authorized for it. Delivery uses a dedicated bounded relay so a failed external service cannot consume the live debugging relay or interfere with supervision.

Sink credentials are protected runtime configuration for the Gump role using them. They never appear in the manifest's public telemetry section or event stream.

External-sink acknowledgements are telemetry-delivery observations, not control-plane commits.

## 14. Audit is separate

Best-effort telemetry is unsuitable for mandatory security audit. If Gump requires durable, tamper-evident audit evidence, it uses a separate signed audit protocol and an explicitly configured external append-only destination.

The same occurrence may produce both:

- A Ratatouille event for immediate operational visibility
- A signed audit record for durable accountability

Failure of one channel does not silently inherit the semantics of the other. Policy states whether inability to commit a required audit record blocks the associated privileged operation.

## 15. Manifest contract

A provisional application section is:

```toml
[telemetry]
protocol = "ratatouille/0.1"
format = "ndjson"
filter = "app:*,-app:noise"

[telemetry.relay]
capacity = "1MiB"
max_record = "64KiB"
overflow = "drop_oldest"
```

This section contributes release-contract requirements and deployment defaults:

- `protocol` is a release capability requirement.
- Required application-facing format and bootstrap contract are release-bound.
- Default application-topic filters, relay capacity, and overflow policy are deployment intent and may be constrained or overridden by cluster policy.
- Local topic filters may be overridden under `[local.telemetry]` without changing the release.

`process:stdout` and `process:stderr` capture is mandatory and therefore has no manifest enable/disable switch. Cluster policy controls memory-window bounds and external routing, not whether the agent drains the pipes.

Secrets, sink credentials, and external destination URLs do not belong in the public section.

## 16. Failure behavior

| Failure | Required behavior |
|---|---|
| Application relay is unreachable | Telemetry attempt follows bounded Ratatouille sink behavior; workload continues. |
| Agent telemetry relay fills | Complete bounded segments are dropped; counters advance; supervision continues. |
| Subscriber disconnects | Subscription state expires; producer filters revert when their lease expires. |
| External sink is slow | Its dedicated relay drops or retries within bounds; other paths continue. |
| Agent restarts | Local unforwarded backlog is lost; records on surviving keepers remain until bounded expiry. |
| Workload restarts | New attempt identity scopes new topic counters. |
| Entire cluster fails | All in-memory telemetry is lost; no control or secret recovery depends on it. |
| Application emits a secret | Gump may forward it; this is an application security failure, not something reliable redaction can cure. |

## 17. Testable invariants

1. No telemetry path performs a required durable write.
2. Every queue and record has a configured upper bound.
3. Exhausting telemetry capacity cannot block health checks, supervision, publication renewal, secret handling, or reconciliation.
4. Remote telemetry never uses Ratatouille's plain local HTTP/TCP transport without Gump's authenticated encryption boundary.
5. Workload-provided source fields cannot override the authoritative placement identity.
6. Per-topic sequence numbers never participate in control-plane ordering.
7. A process or agent restart cannot masquerade as uninterrupted sequence continuity.
8. Gump-native telemetry APIs cannot format protected runtime-value types.
9. Standard output and error are always drained and segmented into bounded Ratatouille records; they are never left able to block a child indefinitely.
10. stdout and stderr capture never creates a durable local log file.
11. Audit-required operations do not treat best-effort telemetry emission as an audit commit.
12. Moving a logical instance never continues the previous attempt's topic sequence counters.
13. Node names and file paths never serve as application telemetry identity.
14. Gump never copies or merges application-owned Unix log files during placement changes.
15. Loss of one node does not remove stdout/stderr records already accepted by a surviving telemetry keeper.

## 18. Design questions resolved for v1

The frozen answers are indexed in
[`v1/RESOLUTION_MAP.md`](v1/RESOLUTION_MAP.md). These questions remain as the
design history and as candidates for later telemetry profiles.

1. Which exact Ratatouille version and wire compatibility contract does `gump/1` require?
2. Should Gump contribute a Unix-domain sink to Ratatouille, or use its existing local TCP/HTTP sinks with per-placement isolation?
3. What are the canonical Gump topic naming and validation rules?
4. Which topics are enabled by default for Gump itself, given Ratatouille's default-off behavior?
5. What relay and recent-window capacities, retention time, replication factor, and overflow defaults preserve useful diagnostics without wasting cluster memory?
6. Does the effective producer filter support live reconfiguration, or does changing it require an application restart?
7. How should an application receive Ratatouille bootstrap configuration without coupling directly to Gump-specific environment names?
8. Is NDJSON the sole cluster ingestion format, or may text be adapted locally?
9. What exact line/chunk framing preserves all stdout/stderr bytes while keeping individual Ratatouille records bounded?
10. What small recent in-memory backlog, if any, does a new live subscriber receive?
11. Which telemetry fields are safe to include in a signed audit record without conflating the two channels?
12. Do resource observations flow through Ratatouille topics, a dedicated observation protocol, or both for different purposes?
13. How should telemetry-keeper placement avoid correlated failure while remaining balanced as nodes join and leave?

## 19. Reference

- [`ratatouille` crate documentation](https://docs.rs/ratatouille/latest/ratatouille/) — best-effort topic telemetry, filters, sequence counters, formats, sinks, bounded relays, overflow policy, and statistics.
