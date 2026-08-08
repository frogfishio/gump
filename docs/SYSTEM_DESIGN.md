# Gump System Design

> Product architecture. Concrete v1 implementation choices are normative in
> the [Gump v1 implementation pack](v1/README.md).

> Status: working draft 0.1  
> Purpose: maximal end-state architecture for refinement  
> This document describes the intended system, not an implementation sequence.

The developer-facing manifest contract is refined separately in [Gump Application Manifest](MANIFEST.md).
The distributed K/V contract is refined separately in [Gump Cluster Memory](CLUSTER_MEMORY.md).
The user-facing state and recovery contract is refined separately in [Gump CLI and Lifecycle](CLI_LIFECYCLE.md).
The native observability contract is refined separately in [Gump Telemetry with Ratatouille](TELEMETRY.md).

The native mTLS administration model is refined separately in
[Gump Native Management Plane](MANAGEMENT_APP.md).

## 1. System thesis

Gump is a workload deployment, placement, and supervision system for Unix hosts. It accepts an immutable application capsule, records execution intent in distributed K/V memory, places execution units on eligible nodes, and supervises their declared lifecycle. It does not assume that a workload is a network service, long-running, independently replicated, port-bearing, or CPU-only. When publication is requested, it reconciles eligible endpoints through an available publication provider; Kismet is the first-class integration.

Gump is deliberately not a general infrastructure platform. It owns application delivery and local execution. It does not require Kismet, nor does it absorb Kismet's responsibilities. When the products are used together, Kismet owns externally reachable networking, TLS, ingress, and inter-node service transport.

The user-facing system is minimalist. The internal design is explicit about identity, state, concurrency, cryptography, reconciliation, failure, and recovery.

## 2. Architectural axioms

The following are system invariants, not provisional implementation choices.

1. **No durable Gump database.** Gump has no SQLite, replicated SQL store, durable Raft log, or equivalent hidden database.
2. **The distributed K/V store is Gump's live memory.** Desired state, controller epochs, placements, leases, execution status, and completion knowledge are replicated there and nowhere on node disks.
3. **S3 stores raw sealed capsules, not control state.** A capsule is inert recovery material. Its presence never means that its workload is currently desired or should be restarted.
4. **Capsule is generic framing.** Capsule neither knows nor interprets applications, deployments, archives, environments, secrets, or encryption. Gump defines its own payload dialect.
5. **Node materializations are transient.** Nodes may unpack application material under a directory owned by the capsule UUID, but it is an evictable cache with no authority and no promised lifetime.
6. **Runtime configuration never reaches durable storage in plaintext.** Environment variables and secrets are treated as one protected category. Plaintext exists only in authorized process memory.
7. **The sealed capsule is the disaster-recovery copy.** After total cluster loss, protected runtime configuration is recovered by downloading and unsealing the original capsule.
8. **The object store is untrusted for confidentiality and integrity.** Reading objects reveals metadata and ciphertext; modification, substitution, truncation, replay, and deletion must be detected or bounded by Gump's cryptographic and concurrency protocols.
9. **Observed state is ephemeral.** Process IDs, ports, health results, restart counters, node load, leases, and live placement are rebuilt from agents and running hosts.
10. **All effects are fenced and idempotent.** Repeated or stale control messages must not create duplicate or regressive effects.
11. **Publication is optional and provider-driven.** Gump can run a complete cluster without Kismet. It may publish and withdraw ready targets through a configured provider, with Kismet as the first-class integration; Gump itself does not implement ingress, certificates, overlay networking, or cross-node routing.
12. **A release is immutable.** Mutation creates a new capsule UUID and a new declaration generation.
13. **Workload shape is declared, not inferred.** Ports, probes, unit cardinality, continuous execution, independent restart, completion, coordinated launch, accelerators, and publication are optional contracts.
14. **Total-memory loss requires explicit reintroduction.** If every replica of the distributed K/V store is lost, Gump does not infer desired workloads or execution completion from S3. An authorized actor must explicitly reintroduce a capsule and decide whether work should run again.

## 3. Boundaries and terminology

### 3.1 Core objects

- **Cluster**: a security and scheduling domain with one logical control plane and a set of enrolled nodes.
- **Node**: a Unix host running a Gump agent. It may also run Kismet, but Kismet is not part of the node definition.
- **Application**: immutable packaged release material and its runtime-configuration contract. The developer-facing term remains convenient, but it does not imply a web application.
- **Workload**: a stable logical computation identity and declared lifecycle. It may be a continuous service, worker, finite job, distributed training run, scheduled task, migration, or another executable shape.
- **Capsule**: a byte container conforming to the Capsule specification. Its payload is opaque to Capsule itself.
- **Release**: immutable application material and sealed runtime configuration identified by a capsule UUID and cryptographic digest.
- **Deployment declaration**: a signed statement that asks the cluster to converge an application to a release and policy at a monotonically ordered generation.
- **Execution**: one declared attempt to realize a workload generation, whether continuous or finite.
- **Instance / execution unit**: one supervised member of an execution on one node. Independently replicated services have interchangeable units; coordinated workloads may assign distinct ranks or roles. Existing uses of “instance” in this document mean an execution unit, not necessarily a service replica.
- **Placement**: the controller's current assignment of an execution unit to a node and its reserved capabilities.
- **Publication**: optional, provider-specific registration of an eligible execution unit's declared endpoint. The Kismet integration uses lease-bound registration with a local Kismet daemon.
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
- Optional publication-provider lifecycle, including the first-class Kismet integration

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

### 3.4 Workload-neutral core

Gump's core schedules declared execution units and observes state transitions. It does not choose a workload type and then smuggle in implied behavior. The declaration independently specifies, where relevant:

- Continuous or finite lifetime
- Unit count and whether units are interchangeable or role/rank-specific
- Independent, ordered, or gang admission
- Independent or group-coupled failure and restart behavior
- Success, failure, retry, and completion conditions
- Optional readiness, liveness, progress, and completion checks
- Optional ports, publication, and traffic-drain behavior
- Required resources, devices, topology, locality, and isolation
- External data dependencies, checkpoint expectations, and output destinations

A trivial native process can omit nearly all of these. A distributed training workload may request a coordinated group, accelerators, same-fabric placement, rendezvous information, group failure semantics, and replicated in-memory completion tracking without using ports, HTTP checks, or publication. These are composable contracts, not a closed taxonomy such as `service` versus `job`.

### 3.5 Small kernel and extension boundary

Gump's kernel consists of four mechanisms:

1. **Capsule**: construct, verify, seal, unseal, and materialize an immutable release.
2. **Distributed memory**: remember and fence the living cluster without durable Gump state.
3. **Placement**: match declared execution requirements to current capacity and policy.
4. **Supervision**: start, observe, coordinate, stop, and clean execution units.

Everything else enters through a narrow, typed contract: Capsule storage connector, seal authority, execution driver, resource-capability provider, external-data connector, publication provider, or telemetry sink. These contracts are not a general plugin runtime.

An extension cannot create desired state, participate in controller election, bypass placement fencing, inspect unrelated runtime configuration, weaken Capsule verification, or store hidden Gump control state. Inputs and outputs are bounded and versioned; authority is explicit; timeouts and failure behavior are isolated. A missing optional extension affects only declarations that request it.

## 4. High-level architecture

```text
Developer / CI
    |
    | build + seal + sign
    v
Local Gump
    | \
    |  \ live desired state
    |   +----------------------> distributed K/V memory
    |                                   |
    | raw sealed capsule                | watch / reconcile
    v                                   v
S3-compatible storage             Active Controller
immutable capsules only                 |
    |                             fenced commands
    |                                   |
    +---- capsule fetch -----> Agent A, Agent B, ...
                                      |
                                transient unpack
                                and supervision
                                      |
                            optional declared endpoint
                                      |
                            optional publication provider
```

The **controller** is a logical role, not a separate product. Any eligible control-plane member may acquire the active controller epoch. Only the active, distributed-K/V-fenced epoch may issue new placement decisions.

The **agent** is the host authority. It verifies controller authority, materializes assignments, supervises workloads, reports observations, and reconciles any configured local publication provider. Gump's correctness and workload lifecycle do not depend on such a provider being installed.

### 4.1 One application, multiple roles

`gump` is one coherent application distributed to developer machines, CI systems, controller members, and workload nodes. A particular process activates only the capabilities required by its role:

- **Local role**: run, inspect, test, package, sign, and deploy an application.
- **Ingress role**: authenticate deployers, commit exact capsule bytes to S3, and commit validated live intent to distributed K/V memory.
- **Controller role**: read desired state from distributed K/V memory, schedule, and reconcile.
- **Memory-member role**: hold and replicate a share of the distributed K/V state in process memory.
- **Agent role**: materialize, execute, observe, and publish workloads.
- **Custodian role**: hold the unsealed cluster capability and authorize protected runtime-material delivery.
- **Telemetry-keeper role**: retain a sharded, redundant, bounded recent window of Ratatouille records in memory.

Roles may coexist in one process on a small installation or be isolated into separate processes and privilege domains on a larger installation. Their protocols and authority boundaries remain the same. Deployment topology must not change the object model or lifecycle semantics.

### 4.2 Local development model

The local role executes the same application manifest, execution contract, runtime-configuration injection rules, declared checks, and lifecycle state machine used by a cluster agent. Local execution does not require a capsule-store upload or live deployment declaration.

The intended interaction is:

```text
gump run          # run the application locally under Gump
gump test         # evaluate its declared checks locally
gump deploy       # capture, seal, stamp, upload, and declare a release
```

Command names other than `gump deploy` remain provisional, but local-to-cluster continuity is an architectural requirement.

Local parity means parity of contract, not a false promise of identical machines. Gump reports differences in operating system, architecture, execution driver, isolation capability, publication-provider availability, and injected configuration. Local resource observations may be attached to a deployment as advisory profiling evidence, but a cluster never treats developer-machine measurements as authoritative capacity facts.

### 4.3 Cluster startup

Gump assumes that ordinary infrastructure automation has already created machines, installed the Gump executable, established basic network reachability, and knows the node addresses. Gump does not add a machine-provisioning or automatic-discovery system on top of Terraform, Ansible, cloud-init, systemd, or equivalent tooling.

Startup is seed-based:

```text
terraform / equivalent
    -> creates bare machines and addresses

ansible / equivalent
    -> installs Gump
    -> starts first node with --init and explicit cluster parameters
    -> starts remaining nodes with --join <first-node-address>
```

The first process creates the initial in-memory K/V membership, cluster epoch, and listening rendezvous. Joining processes authenticate the seed, present authorized enrollment evidence, negotiate capabilities, and become ordinary members. Once membership is established, the first node has no permanent special status and may fail, restart, or leave like any other member.

Cluster parameters include advertised and listening addresses, intended cluster identity, intended initial membership, Capsule-store connector configuration, and references to seal or enrollment authority. Plaintext secrets do not appear in process arguments; automation passes secret handles, inherited descriptors, environment references, or HSM/KMS identities as appropriate.

The initial node is a complete one-server Gump cluster and may admit workloads immediately. Gump has no artificial minimum cluster size. With one memory member, loss or restart of that process loses all live cluster memory; S3 Capsules survive but remain inert until explicitly reintroduced. Gump reports this as zero failure tolerance without treating it as a configuration error.

Additional members replicate memory and reduce the chance of total loss. The exact distinction between memory survival and continued mutation availability is explicit: for example, two members can preserve a copy after one failure while a safe quorum policy may freeze new control effects until membership is restored. Three members can tolerate one unavailable member while retaining a majority. Gump reports the guarantees of the actual topology rather than labeling it simply “highly available.”

If the seed fails before another member joins, automation starts initialization again. If every member loses memory, the same procedure forms a new empty cluster under the total-memory-loss semantics in section 5.4.

### 4.4 Deployment continuum

One-server operation is a primary product workflow for beta environments, integration testing, demonstrations, personal deployments, and workloads whose loss is acceptable. It provides the real server-side Gump behavior—Capsule upload, in-memory intent, placement, supervision, secret delivery, resource observation, telemetry, and optional publication—without pretending to provide redundancy.

The progression is continuous:

```text
gump run locally
    -> deploy to one beta server
    -> validate the real packaged workload
    -> join additional servers
    -> gain capacity and replicated cluster memory
```

Joining servers does not create a different cluster type, require a different manifest, rebuild a Capsule, or migrate a database. The existing in-memory state is transferred and replicated to the new members, after which the scheduler may place or rebalance work according to policy. A workload can move from beta to a larger topology through ordinary deployment and membership operations rather than a separate “production” product.

Failure of a disposable one-server environment is allowed to be inexpensive: automation can initialize a fresh empty server and the developer can explicitly deploy or reintroduce the Capsules still wanted. Gump communicates the lack of failure tolerance clearly but does not burden this workflow with high-availability ceremony.

## 5. Memory and storage model

“Zero footprint” means that Gump leaves no authoritative operational state on a node. There is no local database, write-ahead log, controller snapshot, durable telemetry queue, secret file, deployment journal, or completion ledger. A node may contain the Gump executable, externally provisioned identity material, and currently materialized application files; only the application files are Gump-managed disk content, and they are disposable.

### 5.1 Distributed K/V memory

The distributed K/V store is the authoritative memory of a live Gump cluster. It holds small control records such as:

- Desired workload declarations and active generations
- Controller epochs and fencing tokens
- Node membership, capabilities, heartbeats, and leases
- Placements, reservations, execution units, attempts, roles, and ranks
- Health, progress, publication, retry, and finite-completion state
- Secret-custody membership and non-secret key references
- Cache references and garbage-collection eligibility

Records use versioned schemas, authenticated writers, compare-and-swap or transactions where concurrency requires them, watches for reconciliation, and leases or TTLs for liveness. Large application bytes, telemetry streams, and plaintext runtime configuration never enter the K/V store.

The K/V implementation supports a one-member cluster. In that topology, transactions, revisions, watches, and leases retain the same semantics, but replication and failure tolerance are zero. The operator receives a clear durability report, not a prohibition.

With multiple members, K/V state is replicated so loss of fewer than all memory copies need not erase the cluster's memory. Safe mutation availability may require a stronger quorum than mere survival of one copy. Gump reports these separately as **memory copies**, **failures survivable without memory loss**, and **failures survivable while accepting new control mutations**.

Gump does not require or create disk persistence for the K/V store. If all K/V members and their memory are lost, the operational history is gone by design.

### 5.2 Capsule object storage

S3-compatible storage contains exact, raw, sealed Capsule bytes under immutable capsule identities. It does not contain deployment heads, desired-state declarations, placements, controller elections, execution records, or completion receipts.

```text
capsules/<capsule-uuid>.capsule
```

Capsules are committed write-if-absent and verified independently of transport security using their canonical digest, signer identity, signature, cluster/purpose binding, and dialect version. Capsule CRC remains useful for corruption detection but is not an authenticity mechanism.

The S3 connector needs streaming or multipart upload, bounded reads, immutable commit, safe staging-to-final promotion, integrity evidence, and optional retention/replication features. It does not need to provide consensus, leases, compare-and-swap heads, or controller fencing.

Capsule retention is an explicit object-store policy. Gump never concludes that a capsule is safe to delete merely because the live K/V store no longer references it; that absence may reflect intentional dormancy or loss of all cluster memory.

### 5.3 Transient node materialization

After verification, an agent may unpack the public application segment beneath the capsule UUID. The directory is a reconstructible cache, not an installation record. Reference counts and eviction eligibility live in the distributed K/V store; absence, relocation, restart, pressure, or policy may remove the materialization at any time after no live execution uses it.

Gump writes no secret into that tree and no sidecar metadata is required to reconstruct cluster intent. A fresh node begins empty, joins the K/V cluster, receives placement, fetches its capsule, materializes what it needs, and is equivalent to any long-lived node. When a node joins a newly reconstituted empty cluster, any old materialization not referenced by a newly accepted execution is swept as orphaned cache.

### 5.4 Total-memory loss

Raw capsules survive total cluster loss, including their encrypted runtime configuration, but desired state does not. Capsules are deliberately inert: Gump never lists the bucket and assumes every object should run.

Recovery establishes a new live cluster, restores its unseal capability, and then requires an authorized actor or external policy source to explicitly reintroduce selected capsule identities. For finite work, that actor must decide whether to start a new execution, resume from an external checkpoint, or leave the capsule dormant. Gump cannot promise exactly-once behavior across total loss of all of its distributed memory.

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
- Execution-unit cardinality and placement policy defaults
- Optional check and completion contract
- Restart and termination policy
- Optional publication intent and provider-specific parameters
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

### 6.2.1 Distribution at scale

Direct authenticated download from S3 is the baseline and universal fallback. Large clusters may accelerate an authorized Capsule through verified peer-assisted distribution, hierarchical fan-out, prewarming, streaming extraction, and digest-addressed public-segment caching.

Peers are untrusted byte sources. A receiving agent accepts bytes only for a current authorized placement, enforces transfer bounds, reconstructs the exact Capsule stream, and independently verifies Capsule identity, digest, signature, cluster binding, dialect, and archive digest before execution. A peer never supplies plaintext protected material and never becomes an authority merely because it has cached bytes.

Distribution membership and availability remain separate from Kismet. Gump may select nearby sources using node and topology observations, but it does not require an overlay network product. Failure or corruption falls back to another peer or S3 and produces bounded telemetry.

Prewarming creates only transient cache and has no execution effect. Content deduplication is permitted for non-confidential public material when its equality leakage is acceptable; the raw sealed Capsule remains the release identity and disaster-recovery object.

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
- Publication-provider identity, when a provider has its own principal
- Kismet daemon identity, only when the Kismet integration is active

One principal may hold several roles, but authorization evaluates the roles separately.

### 8.2 Enrollment

Nodes begin untrusted. Enrollment requires a short-lived, single-use invitation or external attestation bound to the intended cluster and an authenticated key-establishment ceremony. Successful enrollment produces an in-memory node identity certificate or equivalent short-lived credential and a live membership record in distributed K/V memory.

Gump does not persist a node private key to disk. After an agent restart, the node re-enrolls or obtains identity from an operator-provided system facility such as a machine identity, TPM, HSM, or workload-identity service. Such an external facility may be durable, but it is not storage created or owned by Gump.

### 8.3 Transport

All CLI-to-controller, controller-to-agent, custodian-to-agent, and agent-to-publication-provider interactions are mutually authenticated where both endpoints have identities. Network protocols provide confidentiality, integrity, replay defense, deadlines, and protocol-version negotiation.

Authorization is default-deny and binds every request to cluster, principal, role, application scope, operation, and controller epoch.

## 9. Controller authority in distributed memory

Gump uses a single active controller epoch for placement serialization. It does not use a durable Raft log.

### 9.1 Epoch acquisition

Eligible controller members contend for a short-lived controller lease in the distributed K/V store. Acquisition and renewal use its transactional compare-and-swap and lease primitives. Every successful acquisition creates a strictly newer live epoch and a unique fencing token.

The record binds:

- Cluster identity
- Epoch number
- Controller identity
- Unique fencing token
- Lease validity information
- Previous K/V revision
- Signature

K/V lease time or another agreed lease authority must be used where local-clock ambiguity would make a lease unsafe.

### 9.2 Fencing

Every mutating controller command contains its epoch and fencing token. Agents validate controller authority before accepting new assignments. An agent that cannot distinguish the current controller fails closed for new mutations while continuing already-running authorized workload units according to policy.

An agent restart cannot rely on a remembered local epoch. Before accepting mutation, it obtains fresh controller authority evidence from the distributed K/V store or validates a bounded proof issued from its current lease.

### 9.3 Controller loss

Loss of the controller does not stop authorized workload units. Publications remain owned and renewed by agents. A replacement controller:

1. Acquires a newer fenced epoch.
2. Reads desired state and execution memory from the distributed K/V store.
3. Discovers nodes and obtains their observed state.
4. Adopts matching live instances by stable identity.
5. Reconciles divergence idempotently.

There is no node-local command log. Distributed K/V state plus live observations are sufficient while the cluster's memory survives.

### 9.4 Storage and memory outages

During capsule-store unavailability:

- Existing authorized workload units continue according to their declared lifecycle.
- Agents continue local supervision and renew any active publication leases.
- Controller elections and desired-state changes continue in the distributed K/V store.
- Deployments requiring an uncached capsule, recovery from a capsule, and starts on nodes lacking the materialization wait.
- Cached releases may restart locally when their K/V authorization and secret-delivery policy permit it.

During loss of K/V quorum:

- Existing workload processes follow an explicit disconnected-agent policy.
- No new placements, executions, retries, publications, or desired-state mutations begin.
- Agents fail closed on effects whose current authorization cannot be proven.
- Restoration of quorum resumes from replicated memory; loss of every replica invokes the explicit reintroduction model in section 5.4.

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

Each workload declaration includes:

- Workload, generation, and execution identity
- Release capsule UUID and digest
- Lifetime, unit cardinality, roles, and coordination policy
- Execution and resource policy overrides
- Placement constraints and preferences
- Update, retry, restart, completion, and disruption policy where applicable
- Optional health, readiness, progress, and completion policy
- Optional publication policy
- Authorization and signature

### 10.1 Stable identities

Executions and their units have logical identities independent of process IDs. A placement attempt adds an attempt identity. Commands are keyed by workload, generation, execution, unit, and attempt so retries can be recognized safely. A retry of a failed attempt is distinct from an authorized rerun of an already completed finite execution.

### 10.2 Reconciliation rules

- The controller issues intent, never assumes completion from request success.
- Agents report observations, not desired state.
- All create, start, stop, and publish operations are idempotent.
- Newer generations supersede older generations according to the declared update policy.
- Stale controller epochs and stale instance attempts are rejected.
- Unexpected processes are quarantined or stopped according to a declared adoption policy; they are never silently counted as correct.
- A finite execution marked complete in the distributed K/V store is converged and is not materialized again while that live memory survives.
- Checks have declared meanings. Readiness is required only for contracts that use it; successful exit may be the sole success condition for a finite workload.
- Only endpoints satisfying their publication eligibility contract may be published.

## 11. Scheduling

Scheduling is deterministic for a fixed input snapshot and policy version. It considers:

- Node eligibility and health
- Architecture, operating system, kernel, and execution-driver capabilities
- Available CPU, memory, disk, process, accelerator, device, and fabric capacity
- Required labels, forbidden labels, and affinity
- Failure-domain spreading and hardware/topology locality
- Unit roles, gang-admission requirements, and co-placement or separation constraints
- Existing placements and movement cost
- Release locality without allowing cache locality to violate safety
- Update disruption budgets
- Secret-delivery eligibility
- Availability of any publication provider required by the declaration

Hard constraints are evaluated before scored preferences. Unschedulable units and groups remain explicit desired objects with human-readable reasons.

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
- Network throughput where visible without assuming authority over the configured networking or publication system
- Accelerator allocation, utilization, memory, errors, throttling, and health where supported
- Interconnect capability and pressure where visible
- Workload-declared progress and checkpoint recency without treating either as trusted control authority

Observed envelopes are ephemeral cluster knowledge. They are replicated in memory for controller continuity but are not authoritative durable state. After total K/V-memory loss, each explicitly reintroduced workload begins with its declared policy and conservative uncertainty until Gump relearns live behavior. An external telemetry integration may retain historical measurements as advisory input.

### 11.2 Placement under uncertainty

A release with no trustworthy history carries an explicit uncertainty score. The scheduler gives unknown workloads additional headroom, prefers nodes able to absorb a mistake, and learns from a bounded initial population before broad placement when deployment policy permits it.

Placement scoring accounts for both individual reservations and correlated host pressure. A node with nominally free memory may still be a poor target if existing workloads burst together or show reclaim pressure.

Gump does not react to every spike by moving workloads. Rebalancing requires sustained evidence, hysteresis, a cooldown period, disruption-budget permission, and a predicted improvement greater than movement cost. Movement follows the workload contract: a service may be replaced behind a publication handoff, while a coordinated training group may be non-movable until a checkpoint or may require group-wide restart.

### 11.3 Coordinated and accelerator placement

Resources are typed capabilities, not a fixed CPU-and-memory tuple. Nodes may advertise GPUs or other accelerators by model, memory, partitioning mode, driver/runtime compatibility, health, exclusivity, interconnect fabric, NUMA relationship, and topology domain. The scheduler matches declared requirements without embedding a particular accelerator vendor into the core model.

Gang admission reserves all required units under one fenced placement transaction before any unit is allowed to begin useful work. Partial reservation expires without starting the execution. After admission, agents receive stable role/rank identities and authenticated rendezvous material through Gump's protected in-memory delivery path. Gump coordinates launch and failure policy but does not implement the workload's collective-communication library, training framework, checkpoint format, or dataset layer.

Large datasets, model checkpoints, and produced artifacts are not assumed to fit in the application Capsule. The manifest declares external data and output capabilities; connectors or host integrations materialize them under explicit access, locality, and persistence contracts. Capsule remains the immutable delivery envelope for application material and protected runtime configuration, not a compulsory bulk-data transport.

### 11.4 Shared-cluster governance

Every workload belongs to a namespace or project. A one-user cluster has an implicit owner namespace with permissive defaults; larger installations can add governance without changing the workload model.

Authorization evaluates principal, namespace, operation, workload, Capsule signer, secret scope, telemetry scope, and connector use. Deploy, alter, stop, forget, purge, inspect protected metadata, subscribe to telemetry, manage nodes, change policy, and unseal are separate permissions.

Scheduling governance includes:

- Per-namespace and per-principal resource quotas
- Priority classes with bounded, inspectable meanings
- Fair-share ordering among equally eligible queued work
- Explicit reservations and time bounds
- Preemption only where policy and victim workload permit it
- Gang queue aging and anti-starvation behavior
- Disruption and maintenance budgets
- Node cordon and drain operations
- Per-namespace release-signing and runtime-configuration authority

Hard capability and safety constraints always precede fairness scoring. Priority never bypasses authorization, isolation, secret eligibility, or required hardware compatibility.

Gang requests do not reserve fragments indefinitely while waiting for the rest. The scheduler either identifies an admissible complete group or queues the request. Queue age, fair share, reservation windows, and preemption eligibility are visible through `gump explain` so scarce accelerators do not disappear into unexplained policy.

Preemption is a declared lifecycle event. Gump withdraws publication where applicable, requests checkpoint or graceful termination when supported, enforces a deadline, and records the reason. It does not claim to roll back application side effects.

### 11.5 Distributed-workload connectivity

Placement does not create a network. Nodes advertise existing connectivity capabilities such as reachability domain, interface class, bandwidth tier, latency tier, MTU, RDMA support, accelerator fabric, and relevant driver/runtime compatibility. Declarations express requirements and preferences against those capabilities.

Before opening a coordinated launch barrier, agents may verify declared prerequisites with bounded authenticated probes and report a common rendezvous view. Gump delivers rank, role, peer addresses, rendezvous tokens, and other protected bootstrap material through the in-memory runtime-configuration path.

Gump does not implement the underlying fabric, routes, NCCL, MPI, collective algorithms, or a service mesh. A cluster may provide ordinary networking, specialist AI/HPC fabric, Kismet, or another system. The scheduler's responsibility is to avoid placements that cannot satisfy the declared connectivity contract and to explain why.

#### 11.5.1 Hiccup peer introduction

Hiccup is Gump's optional, health-driven discovery facility. An application can
extend an ordinary HTTP health response to advertise Hiccup support. Gump then
uses authenticated POST exchanges on that endpoint to receive one current
declaration and deliver current matching peer presence.

Gump stamps stable workload/unit identity, exact attempt incarnation, and the
receiver-reachable private IP. `@self` introduces instances of the same
workload without a configured topic; authorized named topics support broader
discovery. Applications may attach public JSON and opaque application-encrypted
data whose keys Gump does not manage as part of Hiccup.

Hiccup is a speed-dating venue, not a relationship participant. It does not
proxy application traffic, replicate application state, establish consensus,
provide complete membership, or persist a registry. After introduction,
applications connect over their private network and run their own protocols.
The normative v1 contract is [`v1/HICCUP.md`](v1/HICCUP.md).

### 11.6 Enforcement is independent of packaging

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
- Performs the checks declared for that workload, if any
- Owns local publication state and leases when a provider is configured
- Streams bounded logs and observations
- Reconciles orphaned local resources
- Garbage-collects unreachable cached releases only after authorization

### 12.2 Execution drivers

Gump exposes a stable internal execution contract with drivers for:

- Native executable
- Script plus declared interpreter
- OCI image or OCI bundle

Drivers must converge on the same primitive lifecycle semantics: prepare, admit, start, observe, signal, terminate, kill, and clean. Higher-level readiness, restart, completion, coordination, and publication behavior is supplied by the workload declaration and is not implied by a driver. Driver-specific features cannot weaken common identity, secret, or authorization rules.

An OCI workload is still stored as application material inside the Gump payload; OCI is an execution-driver input, not Gump's outer transport. Capsule remains the outer framing in every case.

### 12.3 Isolation

On Linux, the maximal design uses dedicated identities, cgroups v2, namespaces, capability reduction, resource limits, and syscall/filesystem policy appropriate to the driver. The exact isolation profile is declared and observable.

Gump does not claim VM-grade isolation for native processes. A workload that requires a stronger boundary must select an execution driver providing one.

### 12.3.1 Zero-footprint execution boundary

“Zero footprint” is a guarantee about Gump-managed control state, not a claim that arbitrary applications perform no writes. Every attempt receives an explicit execution root containing:

- A read-only or policy-controlled release materialization
- A private ephemeral writable area with declared quota
- Bounded temporary and shared-memory areas
- Explicit external data/output mounts, if any
- No ordinary file containing protected runtime configuration

The agent owns the complete process tree through cgroups or the strongest equivalent available mechanism. Descendants cannot escape cleanup merely by reparenting. On termination, replacement, forgotten intent, or orphan reconciliation, the agent kills remaining descendants, detaches external mounts, removes sockets and shared-memory objects, and sweeps the ephemeral writable area.

Core dumps are disabled by default for Gump and secret-bearing workloads. Dumpability, `/proc` visibility, ptrace, environment visibility, swap exposure, and memory locking are controlled or reported according to the execution profile. A node advertises each protection as **enforced**, **observed**, or **unavailable**, and policy may reject placement when a required protection is unavailable.

Application-created log files and temporary data remain application behavior. When written inside the ephemeral execution root they disappear during cleanup; when written to an explicitly declared external mount they follow that external system's contract. Gump does not discover and preserve undeclared files.

Native execution receives honest, capability-specific guarantees. It cannot promise containment against a hostile host kernel, root administrator, or privileges deliberately granted to the workload. Stronger tenant isolation requires an OCI sandbox, VM-backed driver, or another execution driver advertising that boundary.

### 12.4 Runtime-configuration injection

Gump supports two protected injection forms:

- A child process environment assembled immediately before execution
- Anonymous memory-backed files/descriptors for file-shaped or rotatable values

Values never appear in command arguments or persistent files. Workloads run under distinct operating-system identities so unrelated workloads cannot read one another's process environments through ordinary host interfaces.

Environment injection is immutable for the life of a process. Rotation requiring a new value creates a controlled process replacement unless the application uses a memory-backed dynamic delivery contract.

### 12.5 Optional endpoints

When a workload declares an endpoint, the agent allocates it under an explicit bind contract. The workload must bind the supplied address and port or use a driver-provided socket-activation mechanism. Any endpoint-specific check or publication fails if the expected listener is absent or bound outside policy. A workload with no endpoint declaration allocates no port and loses no functionality.

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
10. It commits the desired workload generation into the distributed K/V store using a fenced transaction referencing the verified capsule UUID and digest.

Uploading a capsule alone has no runtime effect. Only accepted live intent in the distributed K/V store creates desired state.

Ingress never expands the application archive and never needs runtime-configuration plaintext. Extraction belongs to the selected workload agents. This keeps upload authority, scheduling authority, execution authority, and secret custody separable even when a small installation runs those roles in one Gump process.

#### 13.1.1 Sealed ingress and promotion

Ingress handles large capsules without writing them to local disk:

1. It authorizes the upload before accepting a body and assigns a bounded, expiring upload identity.
2. It streams exact incoming bytes to a non-authoritative staging object while incrementally enforcing byte limits and computing the signing transcript and digest.
3. It completes structural, dialect, cluster-binding, digest, and signature verification while the object remains quarantined.
4. It promotes the verified bytes to `capsules/<capsule-uuid>.capsule` using write-if-absent semantics and verifies the committed object's evidence.
5. Only after the final capsule is durably committed, it records the signed declaration and active generation in the distributed K/V store.
6. It removes or expires the staging object. Abandoned uploads are lifecycle-collected and can never become desired state.

The staging object contains the same sealed bytes that the developer sent. It is safe from plaintext disclosure under the same cryptographic assumptions as the final capsule, but it is not executable or referenceable as a release.

At no point does ingress request an unseal operation. At no point does the controller extract the archive. The assigned agent extracts only public application material to disk and requests protected runtime material through the in-memory custody path.

### 13.2 Placement and start

1. The controller observes the new K/V generation and verifies its signed declaration and capsule reference.
2. It calculates placements respecting update and disruption policy.
3. The target agent admits the assignment.
4. The agent downloads the raw capsule and verifies framing, digest, signature, dialect, and policy.
5. It unpacks application material to the UUID release root.
6. It obtains authorized runtime material through the secret-custody protocol.
7. For coordinated workloads, all required placements are fenced and admitted before the declared launch barrier opens.
8. It starts the isolated workload unit with its declared role and coordination context.
9. Declared checks begin; a workload without checks is observed through process and driver state.
10. When the publication eligibility contract is satisfied, the agent reconciles requested publication through the configured provider, if any.
11. A finite workload satisfying its success condition is marked complete in the distributed K/V store and is not reconciled again while that cluster memory survives.
12. The agent reports observations; the controller continues reconciliation until convergence.

### 13.3 Replacement and rollback

For continuous workloads using replacement rollout, updates create new generation and unit attempts. Old and new generations may coexist only as permitted by update policy. When a publication provider is configured, it supplies the traffic handoff boundary: publish an eligible new unit before withdrawing an old one when availability policy requires overlap. Finite and coordinated workloads instead follow their declared supersession, cancellation, checkpoint, and restart semantics; rolling service behavior is not applied to them.

Rollback is a new declaration generation referencing a previous capsule. Its protected runtime material is the material sealed into that capsule unless the new declaration explicitly references an authorized runtime-configuration replacement capsule under a future dialect extension.

## 14. Publication providers and Kismet integration

Publication is an optional capability, not part of Gump's control-plane substrate. A workload with no publication intent can fully converge without any publication product installed; this covers training runs, batch jobs, workers, scheduled processes, internal consumers, and applications exposed by separately managed infrastructure.

The publication contract is deliberately narrow: reconcile an authorized ready endpoint into an external reachability system, report its state, and withdraw it. It is not a general extension mechanism and does not allow a provider to control placement, secret custody, process supervision, cluster membership, or desired state.

Kismet is Gump's first-class publication provider. The two products have native identity mapping, lifecycle integration, useful discovery defaults, and excellent diagnostics when colocated, but neither product is required for the other to function. Gump must be independently installable, testable, operable, and recoverable with no Kismet components present.

When the Kismet provider is selected, the agent communicates with its local Kismet daemon over an authenticated Unix-domain protocol.

A publication request contains:

- Cluster, application, release, instance, and attempt identities
- Declared local endpoint
- Service identity and authorized publication intent
- Publication-eligibility evidence or reference
- Lease identity and duration
- Gump agent identity
- Protocol version

The publication is lease-bound to the agent and execution unit. The agent renews it only while the unit remains authorized and satisfies its declared publication-eligibility condition. On lost eligibility, termination, supersession, or loss of authorization, the agent stops renewal or explicitly withdraws the publication.

Gump may carry a user's domain or service publication intent to Kismet, but Kismet remains authoritative for whether and how that intent becomes reachable. Kismet's absence cannot impair packaging, deployment, placement, execution, health evaluation, secret recovery, or telemetry; it affects only declarations that explicitly require Kismet-backed publication.

## 15. Lifecycle, supervision, and termination

Lifecycle state is composable. States appear only when relevant to the declared contract:

- **Admitting**: required resources or coordinated peers are being reserved.
- **Starting**: a unit has begun but has not satisfied its start or eligibility contract.
- **Ready**: the workload has passed its readiness contract, independently of publication.
- **Published**: a ready workload has been confirmed by its configured publication provider.
- **Unready**: running but failing or awaiting readiness; any publication is withdrawn.
- **Progressing**: a finite or long-running workload is reporting optional progress.
- **Succeeded**: the finite execution met its success condition and the distributed K/V quorum accepted its completion state.
- **Unhealthy**: failing liveness policy and eligible for restart.
- **Draining**: withdrawn from new traffic while completing work.
- **Stopping**: termination signal sent within grace period.
- **Failed**: attempt ended without satisfying desired state.
- **Stopped**: deliberate terminal state for that attempt.

Retry and restart policy uses bounded exponential backoff, jitter, and a failure budget when requested. Successful exit of a continuous workload may trigger restart; successful exit of a finite workload may complete it. Independent units may restart separately, while a coordinated group may require all units to terminate and be readmitted together. The declaration decides; Gump does not infer intent from the executable.

For an endpoint-bearing workload with drain semantics, termination order is normally:

1. Mark unready and withdraw publication.
2. Allow configured drain delay.
3. Send the declared graceful signal.
4. Wait the termination grace period.
5. Force termination if necessary.
6. Revoke secret delivery and zeroize agent-held material.
7. Clean ephemeral execution resources.

## 16. Telemetry, events, and diagnostics

Gump uses Ratatouille as its native observability substrate: a best-effort, topic-filtered telemetry firehose with per-topic sequence counters and bounded in-memory relays. Gump does not create conventional local log files or provide a durable historical log database.

Gump roles and instrumented applications emit Ratatouille topics. Agents attach authoritative cluster, application, release, generation, execution, unit, role/rank where declared, attempt, and node identities before forwarding events through authenticated cluster transport. Application-supplied source identity is never trusted for authorization or attribution.

Telemetry loss is permitted and visible. Filters, relay overflows, sink failures, and sequence gaps are counted. Telemetry congestion cannot block process supervision, health decisions, publication lease renewal, secret handling, or reconciliation.

Agents own and continuously drain every child process's stdout and stderr. They segment the streams into bounded `process:stdout` and `process:stderr` Ratatouille records and attach authoritative placement-attempt identity. No conventional log file is written. Direct Ratatouille application topics remain the preferred channel for semantic events.

Agents immediately forward telemetry into a sharded, redundantly held, bounded cluster-memory recent window. Consequently, an application may move across many nodes while a subscription by logical application identity continues across its attempts, and loss of an old node does not erase records already held by surviving telemetry keepers. Unforwarded bytes and records beyond the bounded window may be lost; durable retention remains an external integration.

Optional external sinks may persist selected telemetry, but they are integrations rather than Gump control state. Durable security audit, if required, is a separate signed protocol because Ratatouille's best-effort semantics cannot establish audit completion.

## 17. Failure and recovery semantics

| Failure | Required behavior |
|---|---|
| Workload unit exits | Agent applies the declared continuous, finite, independent, or group-coupled exit policy; no universal restart behavior is assumed. |
| A declared health check fails | Agent applies the declared liveness and any publication-withdrawal policy. |
| Agent process exits | Workload fate follows explicit parent-death policy; any agent-owned publication lease expires if not renewed. |
| Node disappears | Controller eventually places replacements subject to policy and capacity. |
| Sole server or sole K/V member disappears | Live cluster memory is lost; a replacement starts empty and requires explicit Capsule reintroduction. |
| Controller disappears | Workloads continue; a newly fenced controller reconstructs and reconciles. |
| Controller partition | No controller lacking current fencing authority may create new effects. |
| Capsule store unavailable | Existing workloads and K/V coordination continue; starts requiring an unavailable capsule wait. |
| Distributed K/V quorum unavailable | Existing units follow disconnected policy; new control effects freeze until quorum returns. |
| Required publication provider unavailable | Workload may become ready, but remains unpublished; declaration convergence reports the provider failure. |
| Kismet absent and not requested | No effect; Gump operates normally without it. |
| One secret custodian fails | Remaining custody quorum continues; replacement member is securely provisioned. |
| All custodians fail | Cluster reseals; capsules remain recoverable after unseal. |
| Every K/V replica is lost | A new empty cluster forms; no capsule runs until an authorized actor explicitly reintroduces it. |
| Capsule is corrupted/substituted | Verification fails before unpacking or decryption; no workload starts. |
| Stale K/V mutation is replayed | Revision, transaction, lease, and fencing validation reject it. |
| Object store is deleted | Recovery depends on object-store backup/replication; Gump cannot recreate lost capsules. |

## 18. Full-cluster reconstitution

A new empty cluster can be formed with:

- The configured object-store namespace and access
- Cluster public identity and trust anchors
- Valid replacement member identities or an authorized reconstitution ceremony
- The configured unseal authority
- Compatible Gump and Capsule-dialect implementations

Reconstitution proceeds as follows:

1. Establish or restore the cluster identity and trust roots.
2. Enroll controller members and agents.
3. Satisfy the unseal policy and establish in-memory secret custody.
4. Form a new distributed K/V memory and acquire its first fenced controller epoch.
5. Begin with no desired workloads.
6. Accept explicit, authorized reintroduction of selected capsule UUIDs and fresh workload intent.
7. Verify each selected capsule before recreating any desired state.
8. For finite work, require an explicit new-run or external-checkpoint-resume decision.
9. Schedule and materialize the newly introduced executions.
10. Unseal protected runtime material from the selected stored capsules and reconcile optional publication.

No local database restore occurs, and S3 enumeration never substitutes for operator intent. If an organization requires automatic restoration of desired workloads after total K/V-memory loss, that desired-state source is an explicit external integration and is not hidden persistence inside Gump.

## 19. Protocol evolution and upgrades

Every serialized dialect, K/V record schema, and network protocol has an explicit version. Compatibility is capability-negotiated, and the controller will not assign a release to an agent that cannot faithfully execute its required contract.

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
5. An execution unit is published only while authorized and satisfying its declared publication-eligibility condition.
6. Loss or expiry of declared publication eligibility eventually removes any provider-managed target.
7. Replaying any accepted command is idempotent.
8. Reading the same consistent distributed K/V revision yields the same desired workload generations.
9. Rollback never rewrites historical declarations.
10. A node receives plaintext runtime material only for a placement it is currently authorized to run.
11. Capsule or declaration substitution across cluster/application identity fails verification.
12. Loss of distributed K/V quorum cannot cause two controllers to make unfenced progress.
13. Local release directories are disposable and reconstructible from their capsules.
14. Total control-plane loss creates a new empty live state; it does not recover or infer desired workloads from node disks or S3.
15. The ingress role can durably store raw sealed Capsule bytes without decrypting protected runtime material or durably storing desired state.
16. OCI and native packaging choices do not change Capsule's role as the outer deployment framing.
17. Resource enforcement claims reflect verified host capability, not whether a workload happened to arrive as a container.
18. Telemetry loss, backpressure, or sink failure cannot block workload supervision or control-plane convergence.
19. Ratatouille topic sequences are observability hints and never control-plane ordering primitives.
20. Every supervised process has stdout and stderr drained into bounded Ratatouille records without writing conventional log files.
21. Application movement changes attempt and node identity but does not require consumers to discover or reconcile host files.
22. Omitting ports, checks, publication, or continuous restart behavior does not make a workload invalid.
23. No finite execution crosses its launch barrier before its authorization is accepted by the live distributed K/V quorum.
24. Loss of all distributed K/V memory never turns stored Capsules into implicitly desired work.
25. Gang-constrained execution units cannot cross the launch barrier until the entire declared group is admitted under one fenced placement transaction.
26. A one-member cluster is valid and behaviorally complete while reporting zero tolerance for loss of that member's memory.
27. No extension can create desired state, bypass fencing, weaken Capsule verification, or become hidden Gump persistence.
28. Every ended attempt eventually loses its Gump-managed process tree and ephemeral writable root.
29. A manifest cannot grant itself namespace authority, quota, priority, preemption, connector access, or secret scope.
30. Capsule bytes obtained from a peer receive the same complete verification as bytes obtained from S3.
31. Hiccup introductions never become application traffic relay, authoritative membership, durable state, or input to Gump consensus.

## 22. Design questions resolved for v1

These were intentional refinement points, not permission to violate the axioms.
Their v1 answers are indexed in [`v1/RESOLUTION_MAP.md`](v1/RESOLUTION_MAP.md);
the questions remain here to preserve the reasoning and identify future profile
boundaries.

1. What canonical inner representation and streaming layout should `gump/deployment/1` use inside Capsule?
2. Which exact signing and AEAD profiles are mandatory, optional, and forbidden?
3. Is cluster binding performed at capsule construction, or may one sealed capsule be deliberately deployable to several clusters through multiple wrapped keys?
4. What is the canonical application identity: human name, generated ID, public key, or a composition?
5. What signed canonical representation do live deployment declarations use in the distributed K/V store?
6. Which S3-compatible semantics are mandatory for immutable raw-capsule storage, promotion, verification, retention, and recovery?
7. Which concrete in-memory consensus implementation realizes the contract in `CLUSTER_MEMORY.md`?
8. How many in-memory secret custodians are required, and is custody full replication or threshold cryptography?
9. Which external machine-identity, TPM/HSM, and short-lived re-enrollment mechanisms are supported without Gump writing a host key?
10. What happens to an already-running workload when its agent cannot reach the controller, object store, or secret custodians for an extended period?
11. Which runtime values are immutable environment entries, and which require memory-backed rotation?
12. What isolation guarantees distinguish native, script, and OCI drivers?
13. Which publication-intent fields are provider-neutral, which are Kismet-specific, and which belong in the release versus the deployment declaration?
14. How are concurrent deploy, scale, rollback, and policy updates composed into a single new declaration generation?
15. What audit guarantees are mandatory if Gump itself owns no durable event store?
16. What object retention and garbage-collection proof prevents deletion of the only recoverable secret ciphertext?
17. What recovery ceremony distinguishes restoration of the same cluster identity from creation of a new cluster?
18. Which provisional command names and default wait conditions in `CLI_LIFECYCLE.md` survive user testing?
19. Which manifest properties must be identical locally and in-cluster, and which may have explicit local overrides?
20. How are observed resource envelopes summarized, aged, replicated in memory, and protected from manipulated workload measurements?
21. What conservative request and headroom policy applies to a release with no trustworthy resource history?
22. What exact atomic promotion capability profile must every object-storage connector satisfy?
23. Which Ratatouille topics, filters, relay policies, and local transport form Gump's default telemetry profile?
24. Which open implementation decisions in `CLUSTER_MEMORY.md` must be fixed before protocol version 1?

## 23. Reference foundations

- [Capsule library documentation](https://docs.rs/crate/capsule-lib/latest) — framing, encodings, parsing, serialization, and CRC behavior.
- [HashiCorp Vault security model](https://developer.hashicorp.com/vault/docs/internals/security) — untrusted storage, security barrier, authenticated transport, and threat boundaries.
- [HashiCorp Vault architecture](https://developer.hashicorp.com/vault/docs/internals/architecture) — seal/unseal hierarchy and recovery concepts.
- [HashiCorp Vault envelope encryption](https://developer.hashicorp.com/vault/docs/secrets/transit/envelope-encryption) — per-object data keys and encrypted data keys.
- [Amazon S3 conditional writes](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html) — write-if-absent support for immutable capsule identities.
- [Ratatouille crate documentation](https://docs.rs/ratatouille/latest/ratatouille/) — topic filtering, sequence counters, bounded relays, sinks, and best-effort telemetry semantics.
