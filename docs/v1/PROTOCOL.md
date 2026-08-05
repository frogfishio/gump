# Gump v1 Cluster Protocol

> Status: normative

## 1. Protocol layers

```text
TLS 1.3 authenticated QUIC session
  ├── control streams: varint length + protobuf envelope
  ├── Raft streams: bounded request/response messages
  ├── bulk streams: typed open frame + digest-verified chunks
  └── datagrams: non-authoritative liveness hints only
```

The protobuf package is `gump.cluster.v1`. Protocol major is 1; initial minor
is 0. A major mismatch rejects a session. Minor negotiation is additive and
capability-gated.

## 2. Common envelope

Every non-Raft control message is enclosed by `EnvelopeV1`:

```text
protocol_major       u32 = 1
protocol_minor       u32
message_type         open enum
cluster_id           bytes[16]
cluster_incarnation  bytes[16]
sender_node_id       bytes[16]
sender_incarnation   u64
message_id           bytes[16]
correlation_id       optional bytes[16]
operation_id         optional bytes[16]
sent_unix_ms         i64, diagnostic only
body                 bytes containing the declared protobuf type
```

The receiver authenticates transport identity before allocating `body`, then
checks cluster, incarnation, sender, version, message type, frame bound, role,
and permission. Wall time never orders control state.

Stream frames use unsigned protobuf varint length and MUST reject lengths over
the per-message ceiling before allocating. Default control maximum is 1 MiB;
errors are 16 KiB; Hello is 64 KiB. No unbounded list exists on wire.

## 3. Session establishment

After QUIC mutual authentication, both sides exchange `HelloV1` containing
identity, incarnation, supported major/minor range, role set, capability set,
maximum frame/chunk sizes, Raft member identity if any, and a random connection
nonce. The session is unusable until both Hellos validate.

Node transport certificates bind cluster ID, node ID, node incarnation, roles,
and an ephemeral X25519 delivery public key. The certificate is short-lived and
signed by the live cluster authority. Restart requires re-enrollment; Gump does
not load a node private key from disk.

For duplicate sessions, both nodes retain the session initiated by the
lexicographically smaller `(node_id, connection_nonce)` pair and close the
other. Certificate rotation opens a new connection and drains the old one.

## 4. RPC set

Every mutation carries an operation ID, expected fence where applicable, and
authenticated principal context. Required v1 operations are:

| Service | Operations |
|---|---|
| Formation | `Init`, `JoinChallenge`, `Join`, `PromoteVoter`, `Drain`, `Remove` |
| Memory | `Read`, `List`, `Txn`, `Watch`, `LeaseGrant`, `LeaseRenew`, `LeaseRevoke` |
| Deploy | `BeginUpload`, `CommitUpload`, `AcceptDeclaration`, `Reintroduce`, `PurgePlan` |
| Control | `AcquireController`, `Reconcile`, `Explain`, `Status` |
| Placement | `Offer`, `Admit`, `OpenBarrier`, `Revoke`, `Observe` |
| Secrets | `UnsealStatus`, `AuthorizeRelease`, `Deliver`, `RevokeDelivery` |
| Agent | `Prepare`, `Start`, `Signal`, `Terminate`, `Cleanup`, `InspectAttempt` |
| Telemetry | `PublishBatch`, `Subscribe`, `AckWindow`, `KeeperTransfer` |
| Hiccup | `PublishView`, `FetchView`, `KeeperTransfer`, `RevokeAttempt` |
| Publication | local connector calls only; no provider receives cluster RPC authority |

These are protocol operations, not necessarily one network request each. Raft
replication is private to `gump-memory`; other components use typed commands.

## 5. Standard response and error

Responses carry correlation ID, operation ID, outcome revision if committed,
retry safety, optional bounded retry delay, and either a typed result or
`ErrorV1`. Stable codes are:

```text
INVALID_ARGUMENT       SOURCE_CHANGED          UNAUTHENTICATED
UNAUTHORIZED           WRONG_CLUSTER           WRONG_INCARNATION
INCOMPATIBLE_VERSION   NOT_FOUND               ALREADY_EXISTS
CONFLICT               STALE_REVISION          STALE_GENERATION
STALE_FENCE            NOT_LEADER              QUORUM_UNAVAILABLE
COMPACTED              LEASE_EXPIRED            UNSCHEDULABLE
CAPABILITY_UNAVAILABLE ISOLATION_UNAVAILABLE   CAPSULE_INVALID
SIGNATURE_INVALID      UNSEAL_REQUIRED          SECRET_DELIVERY_DENIED
OBJECT_STORE_FAILED    PUBLICATION_FAILED       TELEMETRY_OVERLOADED
RESOURCE_EXHAUSTED     DEADLINE_EXCEEDED        RETRY_LATER
INTERNAL               HICCUP_INVALID           HICCUP_UNAUTHORIZED
HICCUP_OVERLOADED
```

Errors expose a safe reason code, message capped at 1 KiB, field path, retry
class (`NEVER`, `SAME_OPERATION`, `AFTER_STATE_CHANGE`), and optional details
from a closed typed set. Provider bodies, paths, tokens, runtime values, key
material, and child environments are redacted.

## 6. In-memory Raft state machine

OpenRaft is configured with node ID `u64` derived and collision-checked during
formation. Its log store, vote, membership, state-machine data, deduplication
table, and snapshots use owned RAM buffers only. Implementations MUST test that
server operation performs no file creation or write system call except explicit
transient application materialization and operator-selected output.

Each committed command advances a global `revision: u64`. A command is one of:

```text
Put { key, expected, value, lease? }
Delete { key, expected }
Txn { comparisons[], success_ops[], failure_ops[] }
LeaseGrant / LeaseRenew / LeaseRevoke
Compact { through_revision }
MembershipCommand
```

`expected` is absent, exact revision, exact digest, or any. Multi-key
transactions compare one state revision and commit all success operations at
one new revision. Keys and values are typed at the API; arbitrary external
bytes never reach the state machine.

Linearizable reads use Raft's current-leader/read-index barrier. Stale reads are
explicit and prohibited for authorization, fencing, placement admission,
membership, secret delivery, and lifecycle mutation.

## 7. Record keyspace

Keys are internal byte tuples, rendered below for documentation. Every value is
a versioned protobuf with a maximum size and authorized writer.

| Prefix | Record | Limit / retention | Writer |
|---|---|---|---|
| `/cluster/meta` | identity, incarnation, policy revision | 64 KiB, live | formation/controller |
| `/members/<id>` | member, roles, capabilities, lease | 64 KiB, leased | membership |
| `/authority/controller` | epoch, fence, lease | 8 KiB, leased | election |
| `/names/<ns>/<app>` | human name to workload ID | 4 KiB, live | deployment |
| `/workloads/<id>/desired` | current declaration | 256 KiB, live | deployment |
| `/workloads/<id>/history/<gen>` | declaration digest/reason | last 32 generations | controller |
| `/executions/<id>` | lifecycle and terminal result | 64 KiB, live until forget | controller |
| `/units/<id>` | desired unit identity and role/rank | 32 KiB, live execution | controller |
| `/placements/<id>` | node, resources, fence, lease | 32 KiB, leased | scheduler |
| `/attempts/<id>` | state and bounded observations | 64 KiB, last 16/unit | agent/controller |
| `/barriers/<id>` | gang reservation/admission set | 256 KiB, leased | scheduler |
| `/materializations/<node>/<capsule>` | verified cache hint only | 8 KiB, leased | agent |
| `/publication/<unit>/<provider>` | desired/status/receipt digest | 32 KiB, leased/current | controller/connector |
| `/custody/<node>` | eligibility and public key IDs | 8 KiB, leased | custody |
| `/operations/<principal>/<op>` | result digest and response | 64 KiB, 24h/100k | state machine |
| `/observations/<release>/<class>` | resource envelope summary | 64 KiB, 24h bounded | observer |
| `/reasons/<object>/<rev>` | stable transition evidence | last 64/object | owning component |

Capsule bytes, protected values, DEKs, unseal keys, raw telemetry, application
outputs, and checkpoints are invalid record content. State-machine validation
rejects them by type; callers cannot invent key prefixes.

Hiccup declarations and delivered presence are also absent from
this keyspace. They use separately bounded keeper RAM and the schema in
[`HICCUP.md`](HICCUP.md) and `proto/gump/v1/hiccup.proto`.

Initial budgets are 64 MiB authoritative records, 32 MiB leased records, and
32 MiB bounded history. Budget exhaustion rejects growth; authoritative state
is never evicted. Limits are policy-visible and may be raised without changing
record semantics.

## 8. Watches and leases

Watches start strictly after a supplied revision and return ordered committed
changes. The server retains 10,000 revisions or 10 minutes of watch history,
whichever is smaller. A lagging watcher receives `COMPACTED` plus compaction
floor, performs a linearizable relist, and resumes from that revision.

Default lease values are:

| Purpose | TTL | Renew by |
|---|---:|---:|
| controller authority | 10 s | 3 s |
| member liveness | 15 s | 5 s |
| placement/attempt | 20 s | 6 s |
| gang reservation | 30 s | 10 s |
| telemetry subscription | 30 s | 10 s |

The leader schedules expiry using a monotonic clock and commits revocation.
Failover reconstructs deadlines conservatively from remaining durations,
never extending beyond one original TTL. Lease expiry invalidates authority
through fences even if cleanup notification is delayed.

## 9. Controller fencing

`AcquireController` commits `(epoch + 1, random fence, lease_id)`. Every command
that creates an external effect carries epoch, fence, declaration generation,
and object revision. The receiver performs a current linearizable validation
or uses a signed bounded authorization proof issued under the current lease.

An agent stores its accepted fence only in memory. A higher epoch permanently
fences lower epochs for that process lifetime. Equal epoch with a different
fence is a protocol violation. An expired or unverifiable fence cannot start,
restart, signal, publish, or deliver a secret.

## 10. Workload and execution state machines

Workload states are:

```text
ABSENT -> ACCEPTED -> ACTIVE -> STOPPED -> ACTIVE
                      |   |        |
                      |   +------> FORGOTTEN
                      +----------> FORGOTTEN (after termination)
```

Each accepted semantic mutation creates `generation + 1`. Rollback is a new
generation referring to an old Capsule; history is never rewritten.

Execution states are:

```text
PENDING -> ADMITTING -> RUNNING -> SUCCEEDED
   |           |          |  +--> FAILED
   |           |          +-----> CANCELLING -> CANCELLED
   +-----------+----------------> UNSCHEDULABLE
```

`UNSCHEDULABLE` is non-terminal while intent and deadline permit retry.
`SUCCEEDED`, `FAILED`, and `CANCELLED` are terminal within surviving cluster
memory. Reintroduction after total loss always creates a new execution ID.

## 11. Unit and attempt state machines

```text
unit:    WAITING -> RESERVED -> ASSIGNED -> ACTIVE -> TERMINAL
attempt: CREATED -> PREPARING -> ADMITTED -> STARTING -> RUNNING
                                      |          |          |
                                      +----------+----------+-> STOPPING
                                                               -> EXITED
                                                               -> CLEANED
```

Each transition compares generation, controller fence, placement revision, and
prior state. Repetition with the same operation ID returns the original result.
Skipped forward transitions, resurrection from `CLEANED`, or divergent content
at an equal generation are rejected.

An attempt is not `RUNNING` merely because a PID exists. The agent must own the
process tree, stdout/stderr drains, attempt root, secret-delivery lease, and
resource observation handle.

## 12. Gang admission

The scheduler calculates the complete placement group, then commits one
transaction that compares controller fence, declaration generation, every
node capability revision, and every resource ledger, and creates all
reservations plus the barrier. Agents admit independently but cannot launch
useful work.

Once every required admission is committed, `OpenBarrier` advances the barrier
generation and issues rank/role/rendezvous material under one fence. Any
admission rejection or deadline expiry revokes all reservations. A member loss
after opening applies the declaration's group failure policy; v1 has no elastic
shrink.

## 13. Deployment transaction

`gump deploy` uses one stable operation ID through:

1. Build and locally verify exact Capsule bytes.
2. `BeginUpload` authenticates, authorizes, fixes limits, and opens quarantine.
3. Ingress streams exact bytes to S3 while hashing; it never unseals.
4. `CommitUpload` validates framing, segments, signature, signer permission,
   cluster binding, manifest policy, length, and stored-object evidence.
5. Connector publishes the final immutable object.
6. `AcceptDeclaration` atomically binds/compares the application name,
   allocates or reuses workload ID, advances generation, stores the declaration,
   and stores the idempotent result.
7. Controller reconciliation begins from the committed watch event.

Failure before step 5 leaves only a quarantined object. Failure between 5 and 6
may leave an inert orphan Capsule. Retrying resumes with the same operation ID
and exact digest. Uploading alone never creates intent.

## 14. Membership and total loss

The first server executes `server --init`; subsequent servers execute
`server --join <seed>`. A joiner authenticates cluster identity, transfers an
in-memory snapshot, verifies its digest and committed index, enters as learner,
catches up, and is promoted through joint consensus. The seed has no special
role afterward.

Forced recovery from one surviving member of a lost two-node quorum requires
an operator proof that fences the missing member, creates a fresh cluster
incarnation, rotates live certificates/fences, and imports the survivor's RAM
state. The old incarnation is rejected everywhere.

If no memory member survives, there is no recovery protocol for state. `--init`
creates an empty incarnation. Authorized reintroduction selects specific
Capsules and constructs new declarations and execution choices; bucket listing
never implies desired state.

## 15. Idempotency and retry

Clients retry transport loss with the same operation ID. A mutation response is
cached with principal, request digest, result, and revision. Reuse with a
different request is `CONFLICT`. After expiry, a client must read semantic state
before deciding whether a new operation is safe.

Automatic retry is allowed only for errors marked `SAME_OPERATION`. Redirects
and leader changes preserve the operation ID. Deadlines cancel waiting, not a
mutation already accepted by consensus.

## 16. Hiccup keeper transport

Hiccup agents and keepers use authenticated QUIC streams carrying bounded
`HiccupPublishV1`, `HiccupListenV1`, `HiccupDeliveryV1`, and transfer messages. These
streams have an independent capacity class below health, supervision, secret,
Raft, and authoritative control traffic. Their success never creates a Raft
revision or Gump effect fence.

The sender agent derives stable unit identity, attempt identity, and the
receiver-reachable private IP from current placement state before forwarding
presence. A keeper revalidates the sender session, attempt/fence digest, topic
authorization evidence, health-derived expiry, and bounds. Delivery is scoped
to the receiving attempt's listened topics and authorization.

Keepers retain only each live attempt's latest declaration. Duplicate,
missing, reordered, or rotated introductions are normal protocol outcomes;
there are no cursors, acknowledgements, or durable offsets.
