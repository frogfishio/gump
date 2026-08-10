# Captain and Gump — product context and integration handoff

> Audience: Captain and Gump developers  
> Status: product-context baseline; integration protocol is not yet frozen  
> Purpose: explain what Captain is ultimately for, what Gump owns, and how the
> two products are intended to meet

## 1. Executive summary

Captain is a separate product: a purpose-built language and execution system
for literally programming cloud infrastructure. It spans provider APIs, host
configuration, Gump bootstrap, application topology, continuous scaling,
migration, and recovery.

Gump is a separate product: a workload deployment, placement, supervision,
cluster-memory, fencing, and secret-delivery system for Unix hosts.

Neither product requires the other:

- Captain can create and configure ordinary infrastructure without Gump.
- Gump can run workloads on manually, Terraform-, Ansible-, or otherwise
  provisioned hosts without Captain.
- Together they create the intended golden path: Captain creates the first
  host and installs Gump; Gump then hosts a living Captain continuation which
  grows and continuously operates the fleet.

The shortest accurate description is:

```text
Captain programs the fleet.
Gump runs and remembers the living forest.
```

Captain is not merely a better shell wrapper or a replacement for Ansible.
Its current wrapper-first, deterministic, agentless implementation is the
foundation. Its destination is a living cluster program whose event lowerings
and lifecycles continue running after the initial infrastructure has been
created.

## 2. The root idea: the Captain program is the cluster

One Captain program describes one living cluster and its operating envelope:

```text
cluster Forest: [
  capacity: [
    minimum servers: 3.
    burstable to: 10.
    vertical: [
      memory upTo: 64GiB.
      cpu upTo: 32.
      accelerator classes: [ #None, #Nvidia ].
    ].
  ].

  on event #CapacityDeficit: [ lower using: #ScaleOut ].
  on event #NodeLost: [ lower using: #ReplaceNode ].
  on event #IdleCapacity: [ lower using: #Consolidate ].
].
```

The program is not finished merely because it created three servers. It
continues to define:

- minimum, burst, and maximum capacity;
- permitted vertical shapes and specialist capabilities;
- provider accounts, regions, zones, costs, and quotas;
- baseline host and cluster services;
- event reactions and cooldowns;
- node replacement and scale-down policy;
- durable-volume, stable-address, and migration lifecycles;
- automatic versus approval-required effects.

This is the context behind Captain's dormant `cluster`, `fleet`, `lowering`,
`lifecycle`, and `test` declarations. M1–M4 intentionally prove the language,
artifact, VM, wrapper, planning, approval, and replay substrate first. M5
activation is where those declarations become the living cluster program.

## 3. Product responsibilities

### 3.1 Captain owns

- Source language, compiler, checker, deterministic `.capb` artifact, and VM.
- Modules that wrap binaries, host facilities, and provider APIs as typed
  programmatic interfaces.
- Local zero-to-one orchestration across provider, SSH, and HTTPS frontiers.
- Cloud resource logical identities, provider ownership metadata, proposals,
  and provider-side idempotency.
- Desired capacity envelope, cost policy, and provider-shape knowledge.
- Event lowerings, infrastructure lifecycles, execution graphs, plan review,
  grants, run logs, and deterministic replay.
- Execution of provider effects such as creating a server, volume, address,
  DNS record, snapshot, or replacement resource.
- Conservative reconstruction of its provider view from provider observations
  and ownership tags.

### 3.2 Gump owns

- Capsule verification, sealing dialect, storage, materialization, and
  protected runtime-value delivery.
- Desired application generations and exact Capsule identities.
- Workload scheduling, placement, reservations, resource accounting, and
  supervision.
- Stable workload/unit identity and replaceable attempt identity.
- Current node inventory and observed capabilities.
- Cluster membership, in-memory consensus, controller epochs, leases, and
  fencing.
- Workload replacement, cordon, drain, member removal, and attempt cleanup.
- Authority to decide whether capacity is actually usable by a workload.
- Hiccup discovery stamps and bounded shared-memory coordination.
- Gump-native telemetry capture and Ringtail relay.

### 3.3 Neither may absorb the other

Captain must not grow a second workload scheduler, process supervisor,
membership protocol, secret-custody system, Hiccup implementation, or ingress
system.

Gump must not grow a Captain parser/compiler, AWS or DigitalOcean SDK, package
manager, SSH orchestrator, general lifecycle language, or provider price
catalogue.

The integration is a bounded, versioned control contract between independent
products.

## 4. The complete lifecycle

### 4.1 Zero to one: Captain runs locally

Before Gump exists there is nowhere in the cluster to run a controller.
Captain therefore begins on the operator's workstation, CI runner, or
dedicated orchestration host:

```text
Captain local executor
-> reads provider credential by secret handle
-> creates or adopts one server
-> enters the server over SSH
-> installs a pinned signed Gump DEB/RPM
-> installs the unprivileged account and dormant service
-> generates or receives initial cluster authority locally
-> stores recovery material in Macrun/Captain secret storage
-> streams one-use startup parameters to Gump without remote secret files
-> forms a real one-node Gump cluster
```

The first node is already the real product. One-node operation has zero
failure tolerance but is a legitimate bootstrap, beta, and development mode.

Captain's current "one local orchestrator, agentless transports" decision is
exactly correct for this phase. Managed hosts need SSH and ordinary tools, not
a Captain agent.

### 4.2 The handoff: Captain becomes a Gump workload

Once the first Gump node is alive, the local runner builds and deploys a
Captain continuation Capsule. Conceptually it contains:

```text
Captain continuation Capsule
├── pinned Captain executor/VM
├── exact cluster-program .capb
├── pinned provider and tool .capb dependencies
├── public configuration and interface metadata
└── protected provider credentials and runtime values
```

The `.capb` and the Capsule are different artifacts:

- `.capb` is Captain's deterministic executable program plus inspectable
  manifest.
- Capsule is the signed, sealed deployment envelope through which Gump stores,
  transports, places, and supplies the running Captain workload.

The initial integration should keep Captain out of the Gump kernel. Gump runs
one Captain executor as an ordinary continuous Capsule workload. The
`Platform::Gump` frontier adapter lives on the Captain side and talks to a
bounded Gump control API. Gump does not parse Captain source or interpret
Captain bytecode.

This preserves Captain's one-orchestrator model: Captain is not installed as
an agent on every managed host. One currently authorized Captain attempt is the
living infrastructure orchestrator; Gump may replace or move that attempt like
any other workload.

### 4.3 One to many: the continuation grows the cluster

The in-cluster Captain continuation may now realize the declared minimum:

```text
Captain sees desired minimum = 3 and current usable nodes = 1
-> plans two provider creates
-> provider creates two machines
-> Captain installs the pinned Gump package on each machine
-> each machine receives a short-lived, single-use enrolment grant
-> machines join Gump as non-voting learners/agents
-> Gump validates observed capabilities
-> Gump policy promotes required memory members
-> Gump reports three usable nodes and the desired topology converges
```

"Droplet created" is not success. Captain reports capacity available only
after Gump has admitted the node, verified its capabilities, and made it
eligible for the intended workload.

## 5. Runtime interaction

### 5.1 Gump supplies authoritative cluster observations

Captain needs a versioned, bounded API for observations such as:

- cluster identity and incarnation;
- current controller epoch/fence and mutation availability;
- node identities, leases, roles, and observed capabilities;
- current placement reservations and usable resource envelopes;
- normalized unschedulable/capacity-deficit reasons;
- active drain, replacement, and membership operations;
- relevant workload/unit/attempt identities and lifecycle state;
- bounded watches with explicit compaction/relist behavior.

Captain must not read or write Gump's private distributed K/V protocol
directly. Gump publishes purpose-specific typed operations over an authenticated
control surface.

### 5.2 Captain supplies plans and provider effects

Captain owns provider-specific proposals and effects:

- feasible provider shapes;
- expected price, quota, region, and acquisition time;
- create, mutate, replace, snapshot, address, and destroy calls;
- provider-side idempotency and reconciliation;
- provider resource evidence and ownership tags.

For scale-up, Captain may create capacity within an authorized cluster policy,
but Gump decides whether the resulting node is admitted and usable.

For scale-down or replacement, Captain does not delete a live Gump node merely
because it appears idle. The interaction is:

```text
Captain proposes exact provider resource
-> Gump validates placement, quorum, and disruption policy
-> Gump cordons and drains the node
-> Gump removes memory membership when applicable
-> Gump fences the exact node incarnation
-> Captain receives the bound evidence/grant
-> Captain deletes the exact provider resource
```

The same boundary supports durable-volume migration and stable-address
movement. Captain operates provider resources; Gump supplies workload identity,
placement, health, drain state, and fences.

### 5.3 Events and telemetry

Captain lowerings may react to:

- Gump capacity deficits and node loss;
- provider quota or resource observations;
- application or Ringtail metric evidence;
- operator-authored desired-capacity changes.

Metrics are evidence, never authority. A memory-utilization event may trigger
a Captain plan, but authoritative checks such as current fleet size, cooldown,
quorum, placement feasibility, and disruption state come from Gump/provider
control observations.

Hiccup is discovery only. Captain may use the capability directory to locate a
Ringtail sink, Gump control endpoint, or another advertised integration, but a
Hiccup advertisement never authorizes an infrastructure effect or carries a
secret.

## 6. State, restart, and total loss

Captain and Gump deliberately do not recreate a Terraform state file as the
universe.

### 6.1 While the Gump cluster lives

- Gump's distributed memory retains current membership, placement, controller
  fences, desired workloads, and bounded capacity-operation records.
- Captain may retain bounded non-secret cooldown and operation coordination in
  a dedicated Gump control record or authorized shared-memory pool.
- Provider resources retain durable ownership tags, logical addresses,
  operation identities, and provider IDs.
- Captain run logs and Ringtail telemetry are audit/debugging evidence, not
  current infrastructure authority.
- Provider credentials remain protected Capsule material delivered only to the
  authorized Captain attempt.

If the Captain attempt crashes or moves, Gump starts a replacement. The new
attempt receives the same compiled program and protected credential, relists
Gump and provider observations, reconciles Unknown effects, and resumes. It
does not require a local database.

### 6.2 After total Gump-memory loss

No living cluster means no living Captain continuation. Nothing autonomously
mutates provider infrastructure.

The operator runs Captain locally again. It uses the retained source/artifact,
local recovery authority, and conservative provider discovery to:

1. identify owned external resources;
2. form a new empty Gump incarnation;
3. explicitly reintroduce the desired Capsules;
4. redeploy the Captain continuation; and
5. reconcile without silently adopting or destroying ambiguous resources.

S3 Capsules do not secretly reconstruct Gump desired state, and a provider tag
does not by itself authorize adoption or deletion.

## 7. Secret boundaries

The three major credential classes are different:

### Initial cluster recovery authority

Generated or supplied locally and retained in Macrun/Captain secret storage.
It is streamed to the Gump bootstrap boundary without remote plaintext files or
process arguments. It is not given to the in-cluster autoscaler merely because
the autoscaler can create machines.

### Provider credential

Referenced by name in Captain source and `.capb` manifests. During local
bootstrap it is resolved by the local secret store. For the living continuation
it enters the Capsule protected segment and is delivered to the authorized
Captain attempt through memory/descriptor injection. It never enters `.capb`,
plans, Gump K/V, Hiccup, Ringtail, argv, or provider tags.

### Node enrolment authority

Short-lived, single-use, and bound to cluster/incarnation, capacity operation,
expected provider evidence, node public key/identity, role, and expiry. It may
be delivered through a bounded bootstrap mechanism because it becomes useless
after consumption. It is not a recovery secret or reusable join token.

## 8. Authority composition

Captain's plan/grant model and Gump's controller/fence model are complementary,
not interchangeable.

A future in-cluster infrastructure effect should be bound at least to:

- Captain artifact/program hash;
- Captain plan/operation identity;
- Gump cluster identity and incarnation;
- current Gump controller epoch/fence;
- current Captain workload unit and attempt;
- exact provider profile, action, region, shape/count/cost ceiling;
- exact target resource for mutation/destruction;
- expiry and idempotency identity.

Captain plan approval proves that the operator or automatic Captain policy
accepted a particular graph. Gump fencing proves that the living cluster still
authorizes the cluster-side consequence. Provider credentials should still be
scoped as narrowly as the provider permits; a software grant cannot technically
restrain a malicious process holding an unrestricted cloud-admin token.

## 9. What the first integration should build

Do not begin with full autonomous scale-down, pricing optimization, multi-cloud
placement, or vertical migration. Build one vertical path through the final
architecture.

### 9.1 Local `Platform::Gump` wrapper

Captain should be able to use existing Gump binaries and machine output to:

1. install a pinned signed Gump DEB/RPM over an SSH frontier;
2. install/verify the dormant service contract;
3. stream initial server parameters through the bootstrap socket;
4. initialize and unseal one Gump node;
5. inspect cluster identity/status without parsing human text;
6. build/deploy a supplied Capsule; and
7. verify workload intent acceptance and observed readiness separately.

This is wrapper-first Captain work and does not require Gump to understand
Captain.

### 9.2 Captain continuation Capsule

Package an exact Captain runtime and `.capb` into a normal Gump Capsule with:

- `units = 1` / one active controller attempt;
- continuous lifecycle and restart on failure;
- stop-on-isolation or equivalent short authority-loss behavior;
- provider credential delivered through an inherited descriptor;
- Ringtail telemetry as evidence;
- a health/capability endpoint suitable for discovery, not authority.

The first continuation may implement only a fixed desired minimum and one
DigitalOcean provider wrapper.

### 9.3 Minimal Gump capacity contract

Add only the bounded Gump API necessary to:

- read current usable-node inventory and capability envelopes;
- observe "minimum 3, currently 1" as a capacity deficit;
- request/mint two one-use node enrolment grants;
- observe joining nodes and capability verification;
- report convergence when three nodes are usable.

Gump remains authoritative for membership and capability admission. Captain
remains authoritative for the DigitalOcean create calls and host bootstrap.

### 9.4 Required live acceptance

The end-to-end proof is the original product story:

```text
start with one ordinary server
-> Captain installs and initializes Gump
-> Captain deploys its continuation/autoscaler Capsule
-> continuation creates two more servers
-> installs the same pinned Gump package
-> enrols them with one-use grants
-> Gump reports three verified usable nodes
-> no provider/recovery secret appears on disk, argv, logs, Hiccup, or K/V
```

Then destroy or fence one non-sole node and prove a replacement converges
without duplicate provider resources. Do not grant automatic scale-down until
drain, membership removal, fencing, and exact-resource deletion are proven.

## 10. How current Captain work maps to the destination

The existing Captain implementation is not throwaway work:

| Existing Captain mechanism | Destination role with Gump |
|---|---|
| Deterministic `.capb` | Exact cluster-program identity shipped inside Capsule |
| Inspectable manifest | Capability/secret/effect review before deployment |
| Pure VM + effect seam | Same executor used locally and in continuation workload |
| Root `exec`/`shell`/`http` effects | Host/provider implementation substrate |
| Typed wrappers | APT/RPM, SSH bootstrap, DigitalOcean/AWS, Gump API modules |
| Frontiers | Local, provider, SSH, and authenticated Gump control boundaries |
| Agentless SSH | Zero-to-one and ordinary host configuration |
| Plans and plan hashes | Exact review/approval of infrastructure graph |
| Grants and receipts | Captain-side authority and prerequisite evidence |
| Run logs/replay | Deterministic debugging and audit evidence |
| Dormant cluster/lowering/lifecycle declarations | M5 living cluster activation |

The crucial context is that M4's one local orchestrator is not the final
deployment location for a living cluster program. The Gump-hosted continuation
is the deliberate second mode of the same VM. It should preserve determinism,
artifact identity, effect semantics, replay, and the one-active-orchestrator
model.

## 11. Non-negotiable invariants

1. Captain and Gump remain independently useful products.
2. Gump never needs Captain source or the Captain compiler to run workloads.
3. Captain does not become an agent installed on every managed host.
4. The in-cluster Captain continuation is a replaceable, fenced Gump workload.
5. Captain never mutates Gump's private distributed K/V directly.
6. Gump owns placement, drain, membership, attempt identity, and fencing.
7. Captain owns provider knowledge, proposals, APIs, and provider-side
   reconciliation.
8. Provider creation is not reported as usable capacity until Gump verifies it.
9. Metrics and Hiccup may trigger/discover; neither authorizes effects.
10. `.capb` never contains secret bytes; Capsule protected material supplies
    in-cluster provider credentials.
11. Recovery authority, provider credentials, and node-enrolment grants remain
    distinct.
12. A Captain restart or move does not duplicate an uncertain provider effect.
13. No node is destroyed before Gump drain, membership removal where required,
    and fencing complete.
14. Total Gump-memory loss stops autonomous Captain effects and requires local
    explicit reintroduction.
15. No local Captain or Terraform-style state file becomes the sole source of
    external resource truth.

## 12. Questions the integration design must answer

1. What exact authenticated transport exposes the Gump frontier to the Captain
   continuation: local Unix socket, private mTLS endpoint, or both?
2. Which Gump observations and mutations form `gump.control/1`, and what are
   their bounds, deadlines, revisions, watch, and compaction semantics?
3. How is exactly one active Captain attempt authorized after placement,
   restart, partition, and controller change?
4. Where do bounded capacity-operation records live in Gump memory, and which
   component may write each field?
5. How are Captain plan hashes and grants bound to Gump controller epochs and
   attempt fences?
6. How does a newly provisioned node establish an ephemeral public key and
   consume a one-use enrolment grant without provider user-data becoming a
   long-lived secret store?
7. Which Gump roles should an autoscaled node initially receive, and when may it
   become a voting memory member?
8. How is a Captain continuation Capsule upgraded while an infrastructure
   operation is Unknown or partially complete?
9. What minimum reserved capacity guarantees that the Captain controller can
   run when ordinary workloads have exhausted the cluster?
10. Which provider tags and observations are sufficient to reconstruct owned
    resources after Captain restart or total Gump loss?

These questions refine the bridge. They do not change the product boundary:
Captain programs the fleet; Gump supplies the living cluster substrate that
places, supervises, remembers, and fences it.
