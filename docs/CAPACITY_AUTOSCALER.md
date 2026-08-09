# Gump Capacity Autoscaler

> Status: product concept for later refinement. This is not yet a frozen
> `gump/1` manifest, provider ABI, or delivery commitment.

The autoscaler is also an early proving program for the broader purpose-built
cloud infrastructure language described in
[`CLOUD_INFRASTRUCTURE_LANGUAGE.md`](CLOUD_INFRASTRUCTURE_LANGUAGE.md). That
language extends the same ideas across provider construction, machine
configuration, application topology, reaction, and migration.

## 1. Thesis

Gump may deploy an optional autoscaler application from an ordinary sealed
Capsule. The autoscaler observes authoritative capacity deficits, proposes an
allowed infrastructure change, and actuates a cloud or machine provider under
a short-lived Gump authority grant.

Gump remains responsible for workload intent, placement, disruption, draining,
membership, and fencing. The autoscaler remains responsible for provider
catalogue knowledge, credentials, cost modelling, and provider API effects.

```text
Gump scheduler                    capacity provider
      |                                  ^
      | bounded capacity deficit         | create / mutate / destroy
      v                                  |
autoscaler workload ---------------------+
      |
      | proposal / receipt
      v
Gump capacity-operation controller
      |
      +--> join, validate, drain, fence, reconcile
```

This is not "Gump contains DigitalOcean support." It is "an independently
versioned DigitalOcean capacity provider can be deployed on Gump." Equivalent
providers may target another cloud, an on-premises provisioner, or a mixed
fleet. The Capsule is packaging; the application inside it is the autoscaler.

## 2. Primary adoption story: one server to a cluster

Many applications begin on one inexpensive server. The infrastructure problem
arrives later, when the developer needs redundancy, more memory, specialist
hardware, or additional capacity but does not want to become the operator of a
new configuration-management stack.

Gump should support this complete path:

```text
manually create one ordinary server
-> install and initialize Gump over SSH
-> deploy the autoscaler Capsule and provider credential
-> commit a desired capacity outcome
-> create and enrol additional machines
-> promote eligible memory members
-> place node-wide and application workloads
-> converge as a multi-server Gump cluster
```

A provisional developer experience is:

```bash
gump bootstrap ssh root@203.0.113.10 --init
gump deploy autoscaler.capsule
gump capacity set \
  --servers 3 \
  --region sgp1 \
  --memory 8GiB \
  --max-cost "40 USD/month"
```

The autoscaler Capsule supplies the provider implementation and protected
credential. The capacity declaration supplies the mutable desired outcome. A
change from three to five servers changes live capacity intent; it does not
require rebuilding the autoscaler Capsule.

The provider may create machines, private networking, and provider firewall
objects within its authorized profile. Gump remains authoritative for node
enrolment, capability validation, workload placement, and voter promotion. A
request for three servers is not complete merely because three provider
resources exist.

### 2.1 SSH bootstrap command

`gump bootstrap ssh` is the productized zero-to-one path. It installs the same
Gump binary and service contract used by Terraform/Ansible rather than creating
a second kind of cluster.

The command should:

1. Verify or explicitly establish the remote SSH host identity.
2. Inspect operating system and architecture.
3. Select the correct pinned distribution artifact.
4. Verify its release signature and digest.
5. Create the unprivileged `gump` account and constrained directories.
6. Install the binary and dormant bootstrap/service units idempotently.
7. Validate executable-cache and transient-runtime mount properties.
8. Apply an explicitly requested narrow host/provider firewall policy.
9. Start the memory-only bootstrap socket.
10. Stream initial parameters over SSH without arguments or remote secret files.
11. Initialize the cluster or join the named seed.
12. Return safe cluster identity, sealing key, membership, and verification
    evidence.
13. Remove transient bootstrap material and close inherited descriptors.

The intended command family is:

```bash
gump bootstrap ssh root@host --plan
gump bootstrap ssh root@host --init
gump bootstrap ssh root@host --join seed
gump bootstrap ssh root@host --verify
```

It uses the user's existing SSH agent/key and privilege path. Passwords,
provider credentials, recovery secrets, and bootstrap parameters are never
interpolated into the remote command line. The operation is idempotent and
reports every durable host change before applying it.

This is not `curl | sh`. The local trusted Gump client chooses and verifies an
exact artifact, performs bounded preflight, and installs a known service shape.

### 2.2 Reboot and zero-footprint truth

The installed binary, unprivileged account, directories, and dormant service
unit survive reboot. Live Gump state, plaintext secrets, unseal authority, and
bootstrap parameters deliberately do not.

- Before initialization, reboot leaves the dormant bootstrap socket waiting.
- Rebooting the sole initialized server loses its live cluster memory and
  requires explicit initialization and Capsule reintroduction.
- Once a quorum-capable cluster exists, one server can be replaced or rebooted
  while surviving members preserve live state.
- A rebooted diskless-identity node may re-enrol as a new incarnation through a
  short-lived grant from the surviving cluster.
- No recovery secret or reusable join credential is persisted merely to make a
  service appear self-starting.

The CLI must explain this before initializing or rebooting a one-server cluster.
Convenience cannot weaken the defining durability model.

### 2.3 Two entry paths, one cluster contract

Both installation paths remain first-class:

```text
infrastructure-managed:
Terraform -> Ansible -> dormant Gump service -> streamed startup parameters

developer golden path:
SSH -> gump bootstrap -> dormant Gump service -> streamed startup parameters
```

They converge on identical binaries, units, bootstrap framing, server
parameters, membership rules, and conformance checks. Organizations can retain
infrastructure-as-code ownership. A developer whose single server has simply
become insufficient does not have to adopt it as a prerequisite for growth.

The autoscaler is the zero-to-many continuation of the SSH zero-to-one path.
Total cluster-memory loss still returns to zero-to-one; the autoscaler cannot
run when no Gump cluster remains alive.

## 3. Product boundary

### 3.1 Gump owns

- Authoritative desired workloads and executions.
- Normalized resource and capability deficits.
- Current node inventory and trusted capability reports.
- Placement feasibility and reservations.
- Workload replacement and disruption budgets.
- Cordon, drain, membership removal, and node fencing.
- Authorization of exact infrastructure effects.
- Admission of a joined node after capability verification.
- Truthful operation state and reason codes.

### 3.2 The autoscaler owns

- Provider API integration and credential use.
- Provider instance, disk, accelerator, region, price, and quota catalogues.
- Mapping provider shapes to expected Gump capabilities.
- Feasible proposals with cost, acquisition-time, and disruption estimates.
- Idempotent execution of authorized create, mutate, and destroy effects.
- Reconciliation of provider resources it previously created.
- Provider rate limits, eventual consistency, and error translation.

### 3.3 The autoscaler does not own

- Workload placement or eviction decisions.
- Gump memory membership or cluster recovery.
- Direct mutation of Gump's distributed K/V state.
- Permission to delete a machine merely because metrics call it idle.
- Application state transfer, checkpointing, or durability.
- A hidden database of intended infrastructure.

## 4. Capacity is a vector

Autoscaling is not a node counter. Gump expresses missing supply as a bounded
typed vector and the provider proposes shapes capable of satisfying it.

Initial dimensions may include:

```text
cpu quantity and architecture
memory
ephemeral workspace
accelerator count, model, memory, partitioning, and exclusivity
CUDA/driver/runtime compatibility
local ports and networking capability
zone, region, and failure domain
network or collective-fabric capability
isolation and execution-driver support
provider quota and acquisition latency
```

Provider claims are never final truth. A new Gump node reports observed
capabilities after joining, and the scheduler validates the original workload
requirement before placing anything.

Observed consumption may improve proposals, but declared requests, hard
constraints, and actual unschedulable reasons remain distinct. "CPU is busy"
does not prove that another node would help.

## 5. Horizontal and vertical actions

The planner may consider:

- add a node;
- replace a node with a larger or differently shaped node;
- replace several poorly matched nodes with fewer suitable nodes;
- grow a provider resource where mutation is safe and supported;
- introduce a GPU or specialist capability cohort;
- consolidate movable workloads and remove empty nodes;
- downsize an oversized node through safe replacement;
- take no action because the deficit is transient, forbidden, unsatisfiable,
  over budget, or unrelated to capacity.

Provider actions carry a disruption class:

```text
hot mutation
rebooting mutation
create-before-destroy replacement
destroy-before-create replacement
grow-only mutation
unsupported mutation
```

Gump plans the cluster operation from that class. A provider may not label a
disruptive replacement as a harmless in-place resize.

## 6. Scale-up algorithm

1. The scheduler produces a normalized `CapacityDeficit` after placement fails
   for explicit reasons.
2. The autoscaler receives a bounded, non-secret view of that deficit and its
   authorized provider profile.
3. It returns `CapacityProposal`s with expected capabilities, resources, cost,
   acquisition time, disruption, and quota impact.
4. Gump policy selects or rejects a proposal under the current capacity-
   operation revision and controller fence.
5. Gump issues a short-lived, single-use effect grant naming the exact provider
   profile, shape, region, count, tags, and maximum cost.
6. The autoscaler performs an idempotent create and returns provider evidence.
7. The machine starts a minimal Gump bootstrap and presents a short-lived
   enrolment grant or approved provider identity.
8. Gump validates its observed capabilities. A mismatched machine is not used
   merely because it has already been purchased.
9. The scheduler places pending work normally.
10. The operation converges only after usable capacity exists; provider
    creation alone is not success.

## 7. Vertical replacement

Most vertical changes should be treated as node replacement even when a cloud
provider calls them resizing:

1. Create the better-shaped node when quota and policy allow surge.
2. Join it and validate actual capabilities.
3. Cordon the source node.
4. Reconcile movable workloads within their replacement/disruption policy.
5. Drain workload-agent responsibilities.
6. If it is a memory member, complete the separate membership-removal state
   machine while preserving the declared quorum guarantee.
7. Fence the old node and its credentials.
8. Authorize destruction of that exact provider resource.

A replacement receives a new Gump node identity even if a human name, DNS name,
or provider role is reused. Old fences and attempts never become valid on it.

The sole-server case is explicitly disruptive. Replacing it loses live cluster
memory unless another memory member is introduced first. Gump must explain that
consequence and require authorization; the autoscaler cannot disguise it.

## 8. Scale-down algorithm

Scale-down is deliberately two-sided:

1. The autoscaler proposes an exact owned resource as a removal candidate.
2. Gump validates minimum capacity, failure domains, quorum, custody, placement
   feasibility, `all_nodes` overhead, and disruption budgets.
3. Gump simulates moving or replacing every affected workload unit.
4. Sustained idle evidence, cooldown, and hysteresis requirements pass.
5. Gump cordons and drains the node; new placement is prohibited there.
6. Workload replacement, Hiccup/publication withdrawal, and memory-membership
   removal complete where applicable.
7. Gump fences the node and issues an exact, expiring destroy grant.
8. The autoscaler deletes only the provider resource named by the grant.

An autoscaler never independently deletes an active or merely unobserved node.
Failure to drain leaves the provider resource intact and the operation paused.

## 9. The `all_nodes` fixed-point problem

Every eligible node creates one desired unit for every `all_nodes` workload.
Adding a node therefore creates demand as well as supply. A naive autoscaler can
loop forever by adding a node for the node-wide unit that appeared because the
previous node was added.

The planner treats `all_nodes` requirements and Gump's host reserve as a
per-node tax:

```text
candidate usable capacity
    = provider shape
    - Gump/system reserve
    - all_nodes requirements for that candidate
```

Only positive remaining capacity satisfies ordinary pending demand. A shape
that cannot host its own mandatory node-wide tax is infeasible.

During an unpromoted `all_nodes` rollout, replacement policy determines which
generation a newly added node receives. The autoscaler does not decide that.

## 10. Autoscaled-node bootstrap and enrolment

Initial formation remains unchanged:

```text
Terraform creates initial machines
-> Ansible installs Gump
-> the first server forms the cluster
-> the other initial servers join
```

The autoscaler extends a live cluster. It does not replace the ground-zero
ceremony and cannot autonomously recover after total cluster-memory loss.

An autoscaled node may use an approved image or a minimal bootstrap that
downloads and verifies a signed Gump release. Bootstrap may contain seed
addresses and public cluster identity. It must not contain:

- the cluster recovery secret;
- long-lived member or join credentials;
- Capsule plaintext runtime values;
- the autoscaler's provider credentials;
- reusable authority to join future machines.

The preferred enrolment grant is single-use, short-lived, and bound to cluster,
capacity operation, expected provider evidence, node public key, role, and
expiry. Provider metadata and user-data are not assumed to be secret stores.

## 11. Authority and fencing

The autoscaler is privileged but replaceable. Possessing a cloud credential
does not give it current Gump authority.

Provider effects require grants scoped to actions such as:

```text
capacity.observe
capacity.propose
capacity.create:<provider-profile>
capacity.mutate:<owned-resource>
capacity.destroy:<owned-resource>
```

Each grant binds cluster/incarnation, autoscaler attempt, controller epoch,
operation revision, provider profile, exact action/parameters, cost/count
ceiling, expiry, and idempotency identity.

The autoscaler uses `stop_on_isolation` with a short confirmation window. Only
the quorum-authorized side receives new grants. An uncertain provider call is
reconciled idempotently rather than repeated blindly.

Hiccup may advertise autoscaler presence or provider profile. It does not carry
authoritative scaling commands or credentials.

## 12. Credentials and stateless reconciliation

Provider credentials are protected autoscaler runtime material, preferably
delivered through a sealed anonymous memory descriptor. They never enter public
Capsule metadata, command arguments, Hiccup, telemetry, distributed K/V,
cloud-init, or release/attempt files.

Credentials should be restricted to the exact account/project and operations
needed by the provider profile. Rotation creates a controlled autoscaler
replacement until a future dynamic-secret profile exists.

Every created resource carries bounded provider tags equivalent to:

```text
gump-cluster-id
gump-cluster-incarnation
capacity-provider-profile
capacity-operation-id
gump-node-id, once assigned
creation-generation
```

After restart, the autoscaler lists resources in its authorized scope,
reconstructs its external view from tags/evidence, compares it with live Gump
inventory, and reports discrepancies. It needs no durable local database.

Unknown, ambiguously tagged, cross-incarnation, or manually modified resources
are never silently adopted or destroyed. They become explicit drift.

## 13. Cost and disruption policy

A provisional policy shape is:

```toml
[capacity]
provider = "digitalocean"
min_nodes = 3
max_nodes = 20
max_hourly_cost = "50 USD"
allowed_regions = ["sgp1"]
scale_up_cooldown = "30s"
scale_down_idle = "30m"
prefer = "lowest_disruption"

[capacity.disruption]
max_draining_nodes = 1
create_before_destroy = true
allow_single_server_loss = false
```

Exact currency, pricing freshness, quota, and catalogue semantics belong to the
provider contract. Gump policy remains authoritative for hard ceilings.

The planner may optimize financial cost, disruption, time to verified usable
capacity, failure-domain effects, and stranded capacity. No score overrides a
hard capability, security, quorum, or disruption constraint.

## 14. Provider capability

A future signed workload declaration may resemble:

```toml
[provides.capacity]
protocol = "gump.capacity/1"
provider = "digitalocean"
profile = "primary-sgp"
actions = ["create", "replace", "destroy"]
```

The signed declaration claims an implementation capability. Cluster policy
separately authorizes the provider profile, credential, regions, actions, and
budget. Hiccup presence proves only that a current attempt is available. Gump
requires all three before issuing an effect grant.

Multiple providers may coexist, but each operation has one authoritative
selected proposal and idempotency identity.

## 15. Failure behaviour

| Failure | Required behaviour |
|---|---|
| No autoscaler deployed | Gump remains complete; capacity deficits stay visible |
| Provider unavailable/rate-limited | Operation waits or fails within deadline; workloads continue |
| Create accepted but response lost | Reconcile by operation tags/idempotency before retry |
| Node created but cannot join | Do not place work; cleanup requires explicit policy |
| Joined node reports wrong capabilities | Reject placement; preserve workload requirement |
| Autoscaler crashes | Replacement reconstructs provider view from tags/live state |
| Autoscaler loses Gump authority | No new effect; short isolation policy terminates it |
| Cluster loses quorum | No new capacity effects |
| Drain cannot complete | Do not destroy the node |
| Sole server selected for removal | Reject without explicit complete-loss authorization |
| Total Gump memory loss | Autoscaling stops; manual bootstrap creates an empty cluster |
| Cost/quota exhausted | Expose the deficit and exact limiting reason |

Autoscaler telemetry is evidence, never authority for infrastructure effects.
Telemetry congestion or Ringtail failure cannot block fencing or draining.

## 16. Operator experience

Before an effect, `gump plan capacity` should explain:

- the scheduler deficit and affected workloads;
- why existing nodes cannot satisfy it;
- candidate plans and rejected alternatives;
- expected cost, acquisition time, surge, and disruption;
- `all_nodes` and system tax;
- nodes to create, mutate, drain, or destroy;
- quorum and one-node consequences;
- policy provenance and required approval.

Observation distinguishes:

```text
deficit observed -> proposal selected -> effect authorized -> effect submitted
-> resource observed -> node bootstrapping -> node joined
-> capabilities verified -> capacity usable -> workload converged
```

"Droplet created" or equivalent is never reported as "capacity available."

## 17. Testable invariants

1. Gump functions normally without a capacity provider.
2. Provider credentials are visible only inside an authorized autoscaler attempt.
3. Hiccup presence alone cannot authorize a provider effect.
4. Only quorum-authorized capacity operations issue current effect grants.
5. Every provider effect is idempotent or reconciled before retry.
6. No node is destroyed before successful drain and fencing.
7. Memory membership removal is separate from workload-agent drain.
8. A replacement machine always receives a new node identity.
9. Observed capabilities, not provider labels, govern placement.
10. `all_nodes` demand is charged as per-node tax and cannot cause runaway scale.
11. Scale-down uses sustained evidence, cooldown, hysteresis, and simulation.
12. Local disk expansion is never described as durable application storage.
13. The last memory copy is not removed without explicit loss authorization.
14. Total cluster-memory loss causes no autonomous provider mutation.
15. Provider drift is reported, not silently adopted or destroyed.
16. Telemetry loss cannot grant, block, or prove a capacity effect.
17. SSH bootstrap installs only a verified pinned artifact and known service shape.
18. SSH bootstrap writes no plaintext cluster or provider secret to the host.
19. SSH and Terraform/Ansible installations converge on the same runtime contract.
20. Growing one manually provisioned server into three is a required live rehearsal.

## 18. Questions for refinement

1. One generic autoscaler with provider plugins, or one small release per provider?
2. Which capacity-operation records enter the first memory state machine?
3. How are current prices and quotas authenticated and bounded?
4. Which provider identity mechanisms can replace bootstrap tokens safely?
5. How should mixed-provider cost, locality, egress, and failure domains compare?
6. When may a created-but-unjoined resource be cleaned up automatically?
7. Which vertical mutations are trustworthy enough to perform in place?
8. How do attached volumes constrain replacement without making storage Gump's job?
9. Does capacity intent originate at cluster startup, in signed policy, or both?
10. What reserve guarantees the autoscaler can run under cluster exhaustion?
11. How are competing proposals cancelled without leaving resources behind?
12. Which one-node and three-node rehearsals precede destroy authority?
13. Which initial operating systems and privilege configurations does SSH
    bootstrap support, and how does it fail on an unknown host?
14. What release-signature root and artifact-distribution mechanisms does the
    bootstrap client trust?
15. Which non-secret host configuration may persist across reboot without
    accidentally becoming hidden cluster identity?
16. Can provider-native identity safely enrol a rebooted node, or must every
    provider use a one-time Gump grant?

These questions refine the ABI. They do not change the core shape: Gump owns
authoritative demand and safe node lifecycle; an optional Capsule-deployed
provider owns cloud-specific capacity effects.
