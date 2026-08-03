# Gump System Design

> Status: working draft 0.1  
> Purpose: maximal end-state architecture for refinement  
> This document describes the intended system, not an implementation sequence.

The developer-facing manifest contract is refined separately in [Gump Application Manifest](MANIFEST.md).

## 1. System thesis

Gump is a deployment placer and workload supervisor for Unix hosts. It accepts an immutable application capsule, records deployment intent in object storage, places workload instances on eligible nodes, supervises those instances, and publishes ready local endpoints through Kismet.

Gump is deliberately not a general infrastructure platform. It owns application delivery and local execution. Kismet owns externally reachable networking, TLS, ingress, and inter-node service transport.

The user-facing system is minimalist. The internal design is explicit about identity, state, concurrency, cryptography, reconciliation, failure, and recovery.

## 2. Architectural axioms

The following are system invariants, not provisional implementation choices.

1. **No durable Gump database.** Gump has no SQLite, replicated SQL store, durable Raft log, or equivalent hidden database.
2. **Object storage is the durable source of truth.** Immutable capsules, signed deployment declarations, and authenticated references to their active generations are the only authoritative durable application state.
3. **Control-plane state is reconstructible.** A completely new control plane can recover desired state by reading and verifying object storage.
4. **Capsule is generic framing.** Capsule neither knows nor interprets applications, deployments, archives, environments, secrets, or encryption. Gump defines its own payload dialect.
5. **Application files may persist.** Nodes unpack application material under a directory owned by the capsule UUID. This material contains no plaintext runtime configuration.
6. **Runtime configuration never reaches durable storage in plaintext.** Environment variables and secrets are treated as one protected category. Plaintext exists only in authorized process memory.
7. **The sealed capsule is the disaster-recovery copy.** After total cluster loss, protected runtime configuration is recovered by downloading and unsealing the original capsule.
8. **The object store is untrusted for confidentiality and integrity.** Reading objects reveals metadata and ciphertext; modification, substitution, truncation, replay, and deletion must be detected or bounded by Gump's cryptographic and concurrency protocols.
9. **Observed state is ephemeral.** Process IDs, ports, health results, restart counters, node load, leases, and live placement are rebuilt from agents and running hosts.
10. **All effects are fenced and idempotent.** Repeated or stale control messages must not create duplicate or regressive effects.
11. **Kismet is the network authority.** Gump publishes and withdraws local ready targets; it does not implement ingress, certificates, overlay networking, or cross-node routing.
12. **A release is immutable.** Mutation creates a new capsule UUID and a new declaration generation.

## 3. Boundaries and terminology

### 3.1 Core objects

- **Cluster**: a security and scheduling domain with one logical control plane and a set of enrolled nodes.
- **Node**: a Unix host running a Gump agent and a local Kismet daemon.
- **Application**: a stable logical workload identity, such as `accounts-service`.
- **Capsule**: a byte container conforming to the Capsule specification. Its payload is opaque to Capsule itself.
- **Release**: immutable application material and sealed runtime configuration identified by a capsule UUID and cryptographic digest.
- **Deployment declaration**: a signed statement that asks the cluster to converge an application to a release and policy at a monotonically ordered generation.
- **Instance**: one supervised realization of a deployment on one node.
- **Placement**: the leader's current assignment of an instance identity to a node.
- **Publication**: a lease-bound registration of a ready instance's loopback endpoint with the local Kismet daemon.
- **Runtime configuration**: all environment variables, credentials, tokens, private keys, and other values that must not persist as plaintext.

### 3.2 Gump owns

- Capsule construction through its payload dialect
- Release upload and verification
- Deployment declarations and generation control
- Node enrollment and authorization
- Placement and reconciliation
- Artifact acquisition and unpacking
- Runtime-configuration unsealing and delivery
- Workload execution, isolation, health, restart, termination, and replacement
- Local stdout/stderr capture and operational events
- Kismet publication lifecycle

### 3.3 Gump does not own

- Domain routing algorithms
- Certificate issuance or storage
- Public ingress
- Cross-node application transport
- Service-mesh policy
- General-purpose persistent volumes
- Application-level database migration
- Build systems, except as optional producers of packaged application material
- A durable log or metrics warehouse

## 4. High-level architecture

```text
Developer / CI
    |
    | build + seal + sign
    v
Local Gump ----------------------------+
    |                                  |
    | immutable capsule upload         | signed declaration
    v                                  v
+---------------------------------------------------------+
| S3-compatible object storage                           |
| capsules, declarations, heads, cluster public metadata |
+---------------------------------------------------------+
                    ^
                    | verify + reconstruct desired state
                    |
             +------+------+
             | Active      |
             | Controller  |
             +------+------+
                    |
       fenced, authenticated commands and observations
           +--------+---------+---------+
           |                  |         |
      +----v----+        +----v----+    ...
      | Agent A |        | Agent B |
      +----+----+        +----+----+
           |                  |
     unpack/supervise    unpack/supervise
           |                  |
      127.0.0.1          127.0.0.1
           |                  |
      +----v----+        +----v----+
      | Kismet  |        | Kismet  |
      +---------+        +---------+
```

The **controller** is a logical role, not a separate product. Any eligible control-plane member may acquire the active controller epoch. Only the active, object-store-fenced epoch may issue new placement decisions.

The **agent** is the host authority. It verifies controller authority, materializes assignments, supervises workloads, reports observations, and owns local Kismet publication leases.

### 4.1 One application, multiple roles

`gump` is one coherent application distributed to developer machines, CI systems, controller members, and workload nodes. A particular process activates only the capabilities required by its role:

- **Local role**: run, inspect, test, package, sign, and deploy an application.
- **Ingress role**: authenticate deployers, validate incoming objects, and commit exact capsule bytes and declarations to object storage.
- **Controller role**: reconstruct desired state, schedule, and reconcile.
- **Agent role**: materialize, execute, observe, and publish workloads.
- **Custodian role**: hold the unsealed cluster capability and authorize protected runtime-material delivery.

Roles may coexist in one process on a small installation or be isolated into separate processes and privilege domains on a larger installation. Their protocols and authority boundaries remain the same. Deployment topology must not change the object model or lifecycle semantics.

### 4.2 Local development model

The local role executes the same application manifest, execution contract, runtime-configuration injection rules, health checks, and lifecycle state machine used by a cluster agent. Local execution does not require an object-store upload or deployment declaration.

The intended interaction is:

```text
gump run          # run the application locally under Gump
gump test         # evaluate its declared checks locally
gump deploy       # capture, seal, stamp, upload, and declare a release
```

Command names other than `gump deploy` remain provisional, but local-to-cluster continuity is an architectural requirement.

Local parity means parity of contract, not a false promise of identical machines. Gump reports differences in operating system, architecture, execution driver, isolation capability, Kismet availability, and injected configuration. Local resource observations may be attached to a deployment as advisory profiling evidence, but a cluster never treats developer-machine measurements as authoritative capacity facts.

## 5. Durable object model

"Stateless" in this design means that no Gump process or host owns authoritative durable control state. It does not mean that the system has no durable information: sealed capsules, declarations, active-generation references, and public trust metadata live in object storage. Unpacked application files are disposable node-local materializations. Host identity credentials, when persisted, identify a machine but do not describe desired or observed application state.

An illustrative key layout is:

```text
clusters/<cluster-id>/public.capsule
clusters/<cluster-id>/controller/head

capsules/<capsule-uuid>.capsule

applications/<application-id>/declarations/<generation>.capsule
applications/<application-id>/head
```

The exact key syntax remains configurable, but its semantics are fixed.

### 5.1 Immutable objects

Capsules and deployment declarations are created with a write-if-absent operation. A key collision is an error even if the bytes appear equal. Objects are never updated in place.

Every immutable object is verified independently of transport security using:

- A canonical byte representation
- A cryptographic content digest
- A signer identity and signature
- A cluster and object-purpose binding
- A format and dialect version

Capsule CRC remains useful for corruption detection but is not an authenticity mechanism.

### 5.2 Head objects

An application head is a small signed reference to its current deployment generation. It is advanced using object-store conditional replacement against the previously observed object version or ETag.

A head contains at least:

- Cluster identity
- Application identity
- Current generation
- Declaration object key and digest
- Previous generation and declaration digest
- Issuer identity
- Issued-at time for audit display, not causal ordering
- Signature

The generation and predecessor link establish application history. Wall-clock timestamps never decide ordering.

Rollback creates a new generation referring to an older release. It does not move the head backward and does not mutate history.

### 5.3 Deletion and retention

Normal deployment operations do not delete immutable objects. Garbage collection is an explicit, authorized reachability operation with a reviewable plan and grace period. A capsule is live if referenced by any retained declaration, rollback window, legal hold, or pin.

Loss or deletion of the object store is outside Gump's availability guarantee. Object versioning, replication, retention, and backup are operator responsibilities, with Gump providing verification and recovery tooling.

### 5.4 Object-storage connector contract

S3 is accessed through a narrow Gump connector rather than leaking vendor APIs into deployment logic. A conforming connector must accurately advertise support for:

- Streaming and multipart upload
- Bounded reads and ranged reads
- Write-if-absent immutable commit
- Compare-and-swap replacement of small head objects
- Stable object-version evidence suitable for fencing
- Server-side copy or another safe staging-to-final promotion
- Integrity metadata and post-write verification
- Listing semantics sufficient for explicit reconstruction and garbage collection
- Optional versioning, retention, legal hold, replication, and server-side encryption

Missing correctness capabilities are not silently emulated with unsafe read-then-write sequences. A connector that cannot provide the required atomicity cannot act as the authoritative store for multi-writer deployment or controller coordination.

## 6. Gump capsule dialect

Gump uses `capsule-lib` for Capsule framing and defines a versioned payload dialect, provisionally named `gump/deployment/1`.

```text
Capsule framing
├── prelude
├── dialect-identifying header
└── opaque payload interpreted by Gump
    ├── public deployment material
    │   ├── application manifest
    │   ├── execution contract
    │   ├── health contract
    │   ├── publication intent
    │   └── compressed application archive
    └── protected runtime material
        ├── algorithm and key-version metadata
        ├── wrapped data-encryption key
        ├── nonce
        └── authenticated ciphertext
```

The precise inner serialization is canonical and independently versioned from the Capsule framing version. Unknown mandatory fields cause rejection. Unknown explicitly optional fields may be retained or ignored according to the dialect rules.

### 6.1 Public material

Public material may be read by anyone able to read the capsule and therefore contains no confidential values. It includes:

- Capsule UUID
- Cluster binding
- Application identity
- Build provenance and source revision, when supplied
- Archive format, lengths, and digests
- Entry point and arguments
- Execution driver and platform requirements
- Resource requests and limits
- Replica and placement policy defaults
- Health/readiness contract
- Restart and termination policy
- Kismet publication intent
- Runtime-configuration names, classifications, and injection targets, but not values

### 6.2 Application archive

The archive is path-safe, deterministic, and compressed. Extraction rejects:

- Absolute paths
- Parent traversal
- Device nodes and unsupported special files
- Ownership or permission escalation
- Links escaping the release root
- Duplicate or ambiguous paths
- Expansion beyond declared file-count and byte limits

The archive digest is authenticated. Nodes unpack to a staging directory, verify it, then atomically publish it as the local release root:

```text
/var/lib/gump/apps/<capsule-uuid>/
```

This directory contains only public application material. It is a reconstructible local cache, not authoritative state.

### 6.3 Protected runtime material

Environment values and secrets share one confidentiality boundary. The entire protected segment is encrypted client-side before upload. Public metadata is bound as authenticated associated data so ciphertext cannot be transplanted across clusters, applications, releases, or manifests.

The segment uses envelope encryption:

1. A fresh data-encryption key is generated for every capsule.
2. A versioned, vetted AEAD suite encrypts the canonical runtime-configuration payload.
3. The data-encryption key is wrapped to the cluster's current seal key.
4. Ciphertext, wrapped key, algorithm identifiers, key version, and nonce are stored in the capsule.
5. Plaintext keys and values are zeroized by the packaging process after use.

Cryptographic algorithms are selected through a separately reviewed cryptographic profile. Algorithm agility is explicit; silent downgrade is forbidden.

### 6.4 Authenticity

Encryption does not establish who authorized a deployment. The completed capsule is signed by an authorized release signer. The signature covers the exact Capsule bytes or a canonical signing transcript that binds the prelude, header, public material, archive digest, and protected segment.

Deployment declarations are separately signed because deployment intent may change without rebuilding a release.

## 7. Seal and unseal model

The cluster has a versioned seal-key hierarchy modeled on established security-barrier and envelope-encryption designs.

### 7.1 Required properties

- Object-store credentials alone cannot decrypt protected runtime material.
- No effective unseal key is embedded in a capsule.
- A newly reconstructed cluster can recover capsules after satisfying the configured unseal policy.
- Seal-key rotation does not require decrypting application archives.
- Compromise of one capsule data-encryption key does not expose another capsule.
- Retired node transport keys cannot decrypt future secret deliveries.
- Key version and cryptographic profile are authenticated against substitution.

### 7.2 Unseal authorities

Gump supports pluggable unseal authorities with equivalent security semantics:

- Threshold recovery shares held by independent operators
- Cloud KMS
- Hardware security module
- A policy requiring multiple independent authorities

The unseal authority recovers or releases the cluster seal capability. Gump never writes the resulting plaintext capability to durable local storage.

The cluster publishes an authenticated seal descriptor. Packaging code encrypts to that descriptor without needing to know whether the private unseal operation is backed by an HSM, cloud KMS, threshold shares, or a combination. Consequently, selecting an HSM changes the cluster's custody implementation and recovery ceremony, not the developer's `gump deploy` workflow.

### 7.3 In-memory secret custody

An unsealed cluster maintains secret custody in a dedicated protected-memory subsystem. Custody is replicated across an eligible quorum of live control-plane members so loss of one process does not force a capsule re-unseal.

Custodians do not broadcast a global plaintext secret table. Each capsule's runtime material is scoped by release and application identity. Delivery to an agent is re-encrypted to that agent's current ephemeral session key after authorization and placement checks.

Protected-memory processes:

- Lock sensitive pages where the operating system permits
- Disable core dumps and debugger attachment
- Exclude secrets from logs, traces, metrics, panic reports, and crash handlers
- Avoid command-line injection
- Use bounded secret lifetimes and explicit zeroization
- Keep cryptographic and general workload code in separate privilege domains
- Treat host-root or live memory compromise as capable of exposing secrets on that host

The last point is an explicit threat-model boundary, not a claim that ordinary process memory is magically immune to a compromised kernel or administrator.

## 8. Identity and trust

### 8.1 Identities

The system distinguishes:

- Cluster identity
- Operator identity
- CI/release-signer identity
- Deployment-authorizer identity
- Controller-member identity
- Node identity
- Workload identity
- Kismet daemon identity

One principal may hold several roles, but authorization evaluates the roles separately.

### 8.2 Enrollment

Nodes begin untrusted. Enrollment requires a short-lived, single-use invitation bound to the intended cluster and an authenticated key-establishment ceremony. Successful enrollment produces a node identity certificate or equivalent signed credential.

Long-lived private node identity material may require host persistence or hardware-backed storage. This is not application state and must be explicitly acknowledged as host identity material. A design claiming absolutely zero durable node identity must instead re-enroll every node after every agent restart.

### 8.3 Transport

All CLI-to-controller, controller-to-agent, custodian-to-agent, and agent-to-Kismet interactions are mutually authenticated where both endpoints have identities. Network protocols provide confidentiality, integrity, replay defense, deadlines, and protocol-version negotiation.

Authorization is default-deny and binds every request to cluster, principal, role, application scope, operation, and controller epoch.

## 9. Controller authority without a durable database

Gump uses a single active controller epoch for placement serialization. It does not use a durable Raft log.

### 9.1 Epoch acquisition

Eligible controller members contend for a short-lived controller record in object storage. Acquisition and renewal use conditional writes. Every successful acquisition creates a strictly newer epoch and a unique fencing token.

The record binds:

- Cluster identity
- Epoch number
- Controller identity
- Unique fencing token
- Lease validity information
- Previous record digest/version
- Signature

Object-store time or another agreed lease authority must be used where local-clock ambiguity would make a lease unsafe.

### 9.2 Fencing

Every mutating controller command contains its epoch and fencing token. Agents validate controller authority before accepting new assignments. An agent that cannot distinguish the current controller fails closed for new mutations while continuing already-running healthy workloads according to policy.

An agent restart cannot rely on a remembered local epoch. Before accepting mutation, it obtains fresh controller authority evidence or validates a bounded proof issued from the current object-store record.

### 9.3 Controller loss

Loss of the controller does not stop healthy workloads. Publications remain owned and renewed by agents. A replacement controller:

1. Acquires a newer fenced epoch.
2. Reconstructs desired state from signed application heads and declarations.
3. Discovers nodes and obtains their observed state.
4. Adopts matching live instances by stable identity.
5. Reconciles divergence idempotently.

There is no replay of a hidden command log. Desired state plus observations are sufficient.

### 9.4 Object-store outage

During object-store unavailability:

- Existing healthy workloads continue.
- Agents continue local supervision and Kismet lease renewal.
- New controller epochs cannot be safely acquired.
- Deployments, rollbacks, and other desired-state mutations stop.
- Cached releases may restart locally only when their authorization and secret leases remain valid under explicit policy.

Gump prefers frozen availability to split-brain mutation.

## 10. Desired state and reconciliation

The active controller continuously computes:

```text
desired declarations
    + eligible nodes and capabilities
    + current observations
    + disruption and placement policy
    = next idempotent actions
```

Each application declaration includes:

- Application and generation identity
- Release capsule UUID and digest
- Desired replica count
- Execution and resource policy overrides
- Placement constraints and preferences
- Update strategy and disruption bounds
- Health/readiness policy
- Restart policy
- Publication policy
- Authorization and signature

### 10.1 Stable identities

Instances have logical identities independent of process IDs. A placement attempt adds an attempt identity. Commands are keyed by application, generation, instance, and attempt so retries can be recognized safely.

### 10.2 Reconciliation rules

- The controller issues intent, never assumes completion from request success.
- Agents report observations, not desired state.
- All create, start, stop, and publish operations are idempotent.
- Newer generations supersede older generations according to the declared update policy.
- Stale controller epochs and stale instance attempts are rejected.
- Unexpected processes are quarantined or stopped according to a declared adoption policy; they are never silently counted as correct.
- Health does not equal readiness. Only ready instances may be published.

## 11. Scheduling

Scheduling is deterministic for a fixed input snapshot and policy version. It considers:

- Node eligibility and health
- Architecture, operating system, kernel, and execution-driver capabilities
- Available CPU, memory, disk, and process capacity
- Required labels, forbidden labels, and affinity
- Failure-domain spreading
- Existing placements and movement cost
- Release locality without allowing cache locality to violate safety
- Update disruption budgets
- Secret-delivery eligibility
- Kismet availability

Hard constraints are evaluated before scored preferences. Unschedulable instances remain explicit desired objects with human-readable reasons.

Capacity claims are reservations within the controller epoch. Agents remain final local admission authorities and may reject assignments if observed host reality has changed.

### 11.1 Resource model

Gump distinguishes three resource concepts:

- **Request**: capacity the scheduler reserves for placement.
- **Limit**: a ceiling the agent enforces when the host and execution policy support enforcement.
- **Observed envelope**: a live statistical model of what a release actually consumes during startup, steady state, bursts, and shutdown.

Requests and limits may be declared by the developer, supplied by policy, inferred conservatively, or refined from observation. The system always records which source produced each value. An inferred value never silently becomes a developer-authored guarantee.

Relevant observations include:

- CPU time, saturation, throttling, and run-queue pressure
- Resident and working-set memory, faults, reclaim, and OOM events
- Process and thread counts
- File-descriptor consumption
- Local disk bytes, operations, and latency
- Startup duration and readiness time
- Restart and crash-loop behavior
- Network throughput where visible without invading Kismet's authority

Observed envelopes are ephemeral cluster knowledge. They are replicated in memory for controller continuity but are not authoritative durable state. After total cluster reconstruction, Gump begins with declared policy and conservative uncertainty until it relearns live behavior. An external telemetry integration may retain historical measurements as advisory input.

### 11.2 Placement under uncertainty

A release with no trustworthy history carries an explicit uncertainty score. The scheduler gives unknown workloads additional headroom, prefers nodes able to absorb a mistake, and learns from a bounded initial population before broad placement when deployment policy permits it.

Placement scoring accounts for both individual reservations and correlated host pressure. A node with nominally free memory may still be a poor target if existing workloads burst together or show reclaim pressure.

Gump does not react to every spike by moving workloads. Rebalancing requires sustained evidence, hysteresis, a cooldown period, disruption-budget permission, and a predicted improvement greater than movement cost. A running process is never teleported: Gump starts a replacement elsewhere, waits for readiness, transfers publication through Kismet, and drains the old instance.

### 11.3 Enforcement is independent of packaging

Containers provide a portable filesystem/process envelope and an opportunity for stronger isolation, but CPU and memory governance come from host mechanisms such as Linux cgroups. A sufficiently privileged Gump agent can place native executables, scripts, and OCI workloads into cgroups and enforce CPU, memory, process, and I/O policy for all three.

Each node advertises one of the following per resource and execution driver:

- **Enforced**: the agent can apply and verify the declared limit.
- **Observed**: the agent can measure consumption but cannot enforce the limit.
- **Unavailable**: the agent can neither enforce nor measure it reliably.

Scheduling policy may forbid a workload from landing where its required controls cannot be enforced. If native execution is intentionally unconstrained, Gump still observes it, reserves headroom, reports pressure, and may replace or relocate it, but it cannot promise protection from host exhaustion.

## 12. Agent and workload execution

### 12.1 Agent responsibilities

Each agent:

- Authenticates the controller and validates fencing
- Advertises node capabilities and capacity
- Downloads and verifies capsules
- Unpacks public application material safely
- Requests only the runtime material needed for authorized local placements
- Creates isolated workload contexts
- Supervises process lifecycle
- Performs health and readiness checks
- Owns local Kismet publication leases
- Streams bounded logs and observations
- Reconciles orphaned local resources
- Garbage-collects unreachable cached releases only after authorization

### 12.2 Execution drivers

Gump exposes a stable internal execution contract with drivers for:

- Native executable
- Script plus declared interpreter
- OCI image or OCI bundle

Drivers must converge on the same lifecycle semantics: prepare, admit, start, observe, signal, terminate, kill, and clean. Driver-specific features cannot weaken common identity, secret, health, or publication rules.

An OCI workload is still stored as application material inside the Gump payload; OCI is an execution-driver input, not Gump's outer transport. Capsule remains the outer framing in every case.

### 12.3 Isolation

On Linux, the maximal design uses dedicated identities, cgroups v2, namespaces, capability reduction, resource limits, and syscall/filesystem policy appropriate to the driver. The exact isolation profile is declared and observable.

Gump does not claim VM-grade isolation for native processes. A workload that requires a stronger boundary must select an execution driver providing one.

### 12.4 Runtime-configuration injection

Gump supports two protected injection forms:

- A child process environment assembled immediately before execution
- Anonymous memory-backed files/descriptors for file-shaped or rotatable values

Values never appear in command arguments or persistent files. Workloads run under distinct operating-system identities so unrelated workloads cannot read one another's process environments through ordinary host interfaces.

Environment injection is immutable for the life of a process. Rotation requiring a new value creates a controlled process replacement unless the application uses a memory-backed dynamic delivery contract.

### 12.5 Ports

The agent allocates a loopback endpoint under an explicit bind contract. The workload must bind the supplied address and port or use a driver-provided socket-activation mechanism. Readiness fails if the expected listener is absent or bound outside policy.

## 13. Deployment lifecycle

### 13.1 Packaging and declaration

1. The local Gump process resolves the application manifest, application files, runtime-configuration sources, and target cluster identity.
2. It computes a release identity and captures version provenance. Time and a human version are annotations; the capsule UUID and cryptographic digest identify the exact release.
3. It constructs the deterministic public payload and application archive.
4. It obtains and verifies the cluster's authenticated seal descriptor.
5. It encrypts runtime configuration into the protected payload.
6. It constructs, stamps, and signs the Gump Capsule.
7. It sends the exact Capsule bytes plus a proposed signed declaration to the server-side Gump ingress role over an authenticated streaming protocol.
8. Ingress authenticates and authorizes the deployer, enforces size and dialect bounds, verifies framing, cluster binding, digest, and signature, and uploads the exact unchanged Capsule using write-if-absent.
9. Ingress verifies the stored object's length and digest without unsealing its protected payload.
10. It commits the immutable declaration and advances the application head using compare-and-swap.

Uploading a capsule alone has no runtime effect. Advancing a valid declaration head creates desired state.

Ingress never expands the application archive and never needs runtime-configuration plaintext. Extraction belongs to the selected workload agents. This keeps upload authority, scheduling authority, execution authority, and secret custody separable even when a small installation runs those roles in one Gump process.

#### 13.1.1 Sealed ingress and promotion

Ingress handles large capsules without writing them to local disk:

1. It authorizes the upload before accepting a body and assigns a bounded, expiring upload identity.
2. It streams exact incoming bytes to a non-authoritative staging object while incrementally enforcing byte limits and computing the signing transcript and digest.
3. It completes structural, dialect, cluster-binding, digest, and signature verification while the object remains quarantined.
4. It promotes the verified bytes to `capsules/<capsule-uuid>.capsule` using write-if-absent semantics and verifies the committed object's evidence.
5. It records the immutable declaration and advances the active head only after the final capsule is durably committed.
6. It removes or expires the staging object. Abandoned uploads are lifecycle-collected and can never become desired state.

The staging object contains the same sealed bytes that the developer sent. It is safe from plaintext disclosure under the same cryptographic assumptions as the final capsule, but it is not executable or referenceable as a release.

At no point does ingress request an unseal operation. At no point does the controller extract the archive. The assigned agent extracts only public application material to disk and requests protected runtime material through the in-memory custody path.

### 13.2 Placement and start

1. The controller observes the new head and verifies its declaration and capsule reference.
2. It calculates placements respecting update and disruption policy.
3. The target agent admits the assignment.
4. The agent downloads the raw capsule and verifies framing, digest, signature, dialect, and policy.
5. It unpacks application material to the UUID release root.
6. It obtains authorized runtime material through the secret-custody protocol.
7. It starts the isolated workload.
8. Liveness and readiness checks begin.
9. Only after readiness does the agent create a Kismet publication lease.
10. The agent reports observations; the controller continues reconciliation until convergence.

### 13.3 Replacement and rollback

Updates create new generation and instance attempts. Old and new generations may coexist only as permitted by update policy. Kismet publication provides the traffic handoff boundary: publish the new ready instance before withdrawing an old one when availability policy requires overlap.

Rollback is a new declaration generation referencing a previous capsule. Its protected runtime material is the material sealed into that capsule unless the new declaration explicitly references an authorized runtime-configuration replacement capsule under a future dialect extension.

## 14. Kismet integration

The agent communicates with its local Kismet daemon over an authenticated Unix-domain protocol.

A publication request contains:

- Cluster, application, release, instance, and attempt identities
- Local loopback target
- Service identity and authorized publication intent
- Health/readiness evidence or reference
- Lease identity and duration
- Gump agent identity
- Protocol version

The publication is lease-bound to the agent and instance. The agent renews it only while the instance remains authorized and ready. On failed readiness, termination, supersession, or loss of authorization, the agent stops renewal or explicitly withdraws the publication.

Gump may carry a user's domain or service publication intent to Kismet, but Kismet remains authoritative for whether and how that intent becomes reachable.

## 15. Health, supervision, and termination

The lifecycle separates:

- **Starting**: process exists but has not met readiness.
- **Ready**: eligible for Kismet publication.
- **Unready**: running but withdrawn from publication.
- **Unhealthy**: failing liveness policy and eligible for restart.
- **Draining**: withdrawn from new traffic while completing work.
- **Stopping**: termination signal sent within grace period.
- **Failed**: attempt ended without satisfying desired state.
- **Stopped**: deliberate terminal state for that attempt.

Restart policy uses bounded exponential backoff, jitter, and a failure budget. Crash loops become explicit observations and cannot consume unbounded node resources or log storage.

Termination order is normally:

1. Mark unready and withdraw publication.
2. Allow configured drain delay.
3. Send the declared graceful signal.
4. Wait the termination grace period.
5. Force termination if necessary.
6. Revoke secret delivery and zeroize agent-held material.
7. Clean ephemeral execution resources.

## 16. Logs, events, and diagnostics

Agents capture stdout and stderr into bounded local ring buffers with backpressure and rotation. Local logs are operational cache, not authoritative durable state. Cluster-wide `gump logs` fans out to live agents.

Optional external sinks may persist logs, metrics, and audit events, but they are integrations rather than Gump's control-state database.

Events use stable reason codes and include cluster, application, generation, release, instance, attempt, node, and controller epoch identities. Secret values and decrypted payloads are structurally excluded before formatting rather than removed by best-effort string redaction.

If durable security audit is required, signed audit records may be emitted to an external append-only sink. Their failure policy is configured independently from application availability.

## 17. Failure and recovery semantics

| Failure | Required behavior |
|---|---|
| Workload exits | Agent applies restart policy; publication is absent until readiness returns. |
| Health fails | Agent withdraws publication and applies declared liveness policy. |
| Agent process exits | Workload fate follows explicit parent-death policy; Kismet lease expires if not renewed. |
| Node disappears | Controller eventually places replacements subject to policy and capacity. |
| Controller disappears | Workloads continue; a newly fenced controller reconstructs and reconciles. |
| Controller partition | No controller lacking current fencing authority may create new effects. |
| Object store unavailable | Existing workloads continue; desired-state mutation and new epochs freeze. |
| Kismet unavailable locally | Workload may run but cannot become publicly ready. |
| One secret custodian fails | Remaining custody quorum continues; replacement member is securely provisioned. |
| All custodians fail | Cluster reseals; capsules remain recoverable after unseal. |
| Entire cluster is lost | New cluster members enroll, unseal, reconstruct declarations, fetch capsules, and converge. |
| Capsule is corrupted/substituted | Verification fails before unpacking or decryption; no workload starts. |
| Head is replayed | Generation/predecessor/fencing validation rejects regression. |
| Object store is deleted | Recovery depends on object-store backup/replication; Gump cannot recreate lost capsules. |

## 18. Full-cluster reconstruction

A complete recovery requires only:

- The configured object-store namespace and access
- Cluster public identity and trust anchors
- Valid replacement member identities or an authorized reconstitution ceremony
- The configured unseal authority
- Compatible Gump and Capsule-dialect implementations

Recovery proceeds as follows:

1. Establish or restore the cluster identity and trust roots.
2. Enroll controller members and agents.
3. Satisfy the unseal policy and establish in-memory secret custody.
4. Acquire a new fenced controller epoch.
5. Enumerate or read application heads.
6. Verify each head, declaration chain as required, and referenced capsule.
7. Reconstruct desired state in memory.
8. Schedule and materialize instances.
9. Unseal protected runtime material from the stored capsules.
10. Publish ready instances through Kismet.

No local database restore occurs.

## 19. Protocol evolution and upgrades

Every persisted dialect and network protocol has an explicit version. Compatibility is capability-negotiated, and the controller will not assign a release to an agent that cannot faithfully execute its required contract.

Mixed-version operation must preserve:

- Signature and canonicalization behavior
- Controller fencing
- Idempotency keys
- Secret-delivery authorization
- Lifecycle state meanings
- Unknown-field rules

Changing cryptographic profiles, canonical serialization, or signing transcripts requires an explicit migration design. Existing capsules remain immutable and readable for their declared retention lifetime.

## 20. Security model

### 20.1 Protected assets

- Runtime-configuration plaintext
- Seal and signing private keys
- Node and controller credentials
- Deployment authority
- Application code integrity
- Desired-state integrity and ordering
- Workload isolation
- Availability within declared failure bounds

### 20.2 Considered attackers

- Passive or active network attacker
- Reader or writer of object storage without signing or unseal authority
- Unauthorized cluster client
- Compromised workload attempting lateral access
- Former or revoked node
- Stale or partitioned controller
- Malicious capsule attempting parser, archive, or execution escape

### 20.3 Explicit limits

- A hostile host kernel or root administrator may inspect local workload and agent memory.
- Object-store deletion can destroy availability unless external retention or replication prevents it.
- A compromised authorized release signer can sign malicious application code within its scope.
- A compromised unseal authority can undermine runtime-configuration confidentiality according to that authority's policy.
- Gump cannot prevent an application from disclosing secrets legitimately delivered to that application.

### 20.4 Mandatory defenses

- Memory-safe parsing boundary where practical
- Strict size, depth, count, decompression, and timeout limits
- Domain-separated signatures and keys
- Authenticated encryption with unique nonces
- Constant-time cryptographic operations through reviewed libraries
- Default-deny authorization
- Mutual authentication
- Replay defense and fencing
- Least-privilege operating-system identities
- Secret-safe diagnostics
- Dependency, provenance, and reproducible-build controls for release tooling

## 21. System invariants suitable for testing

1. No plaintext runtime value appears in a capsule, release directory, log, event, crash dump, or object-store object.
2. No capsule bytes become executable before digest and signature verification succeeds.
3. At most one controller epoch can create accepted new effects at a time.
4. A stale epoch cannot replace, stop, or publish an instance owned by a newer epoch.
5. An instance is published only while authorized and ready.
6. Withdrawal or expiry of readiness eventually removes its Kismet target.
7. Replaying any accepted command is idempotent.
8. Reconstructing from the same verified heads yields the same desired application generations.
9. Rollback never rewrites historical declarations.
10. A node receives plaintext runtime material only for a placement it is currently authorized to run.
11. Capsule or declaration substitution across cluster/application identity fails verification.
12. Object-store outage cannot cause two controllers to make unfenced progress.
13. Local release directories are disposable and reconstructible from their capsules.
14. Total control-plane loss does not require recovery of a Gump database.
15. The ingress role can accept and durably store a deployment without decrypting its protected runtime material.
16. OCI and native packaging choices do not change Capsule's role as the outer deployment framing.
17. Resource enforcement claims reflect verified host capability, not whether a workload happened to arrive as a container.

## 22. Unresolved design questions

These are intentional refinement points, not permission to violate the axioms.

1. What canonical inner representation and streaming layout should `gump/deployment/1` use inside Capsule?
2. Which exact signing and AEAD profiles are mandatory, optional, and forbidden?
3. Is cluster binding performed at capsule construction, or may one sealed capsule be deliberately deployable to several clusters through multiple wrapped keys?
4. What is the canonical application identity: human name, generated ID, public key, or a composition?
5. Are deployment declarations ordinary signed CBOR objects or a separate Capsule dialect?
6. Which S3-compatible semantics are mandatory: strong read-after-write consistency, conditional writes, versioning, object lock, server time, multipart behavior?
7. How is a strictly newer controller epoch allocated without trusting local clocks, and which object-store operations form the proof?
8. How many in-memory secret custodians are required, and is custody full replication or threshold cryptography?
9. What durable host identity is acceptable while preserving the claim that application/control state is stateless?
10. What happens to an already-running workload when its agent cannot reach the controller, object store, or secret custodians for an extended period?
11. Which runtime values are immutable environment entries, and which require memory-backed rotation?
12. What isolation guarantees distinguish native, script, and OCI drivers?
13. What Kismet publication-intent fields belong in the release versus the deployment declaration?
14. How are concurrent deploy, scale, rollback, and policy updates composed into a single new declaration generation?
15. What audit guarantees are mandatory if Gump itself owns no durable event store?
16. What object retention and garbage-collection proof prevents deletion of the only recoverable secret ciphertext?
17. What recovery ceremony distinguishes restoration of the same cluster identity from creation of a new cluster?
18. Which local commands and watch/reload behaviors form the final developer workflow around `gump deploy`?
19. Which manifest properties must be identical locally and in-cluster, and which may have explicit local overrides?
20. How are observed resource envelopes summarized, aged, replicated in memory, and protected from manipulated workload measurements?
21. What conservative request and headroom policy applies to a release with no trustworthy resource history?
22. What exact atomic promotion and fencing capability profile must every object-storage connector satisfy?

## 23. Reference foundations

- [Capsule library documentation](https://docs.rs/crate/capsule-lib/latest) — framing, encodings, parsing, serialization, and CRC behavior.
- [HashiCorp Vault security model](https://developer.hashicorp.com/vault/docs/internals/security) — untrusted storage, security barrier, authenticated transport, and threat boundaries.
- [HashiCorp Vault architecture](https://developer.hashicorp.com/vault/docs/internals/architecture) — seal/unseal hierarchy and recovery concepts.
- [HashiCorp Vault envelope encryption](https://developer.hashicorp.com/vault/docs/secrets/transit/envelope-encryption) — per-object data keys and encrypted data keys.
- [Amazon S3 conditional writes](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html) — write-if-absent and compare-against-version primitives.
