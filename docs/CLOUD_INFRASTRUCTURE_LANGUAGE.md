# Cloud Infrastructure Language — concept baseline

> Status: idea-stage baseline for further refinement. Syntax in this document
> is illustrative. No grammar, type system, provider ABI, execution format, or
> delivery commitment is frozen.

Working name: **Captain**. The name is intentionally whimsical: Captain
programs the fleet and tells Gump how to grow the forest. In the unofficial
Forrest Gump lore, Lieutenant Dan got promoted.

## 1. Thesis

If infrastructure as code is the promise, users should receive code rather
than a growing collection of ill-fitting declarative files.

The proposed language is a purpose-built, infrastructure-centric programming
language for programming cloud systems from their first provider resource
through continuous operation:

```text
cloud account
-> networks, firewalls, addresses, DNS, disks, and machines
-> downloads, packages, files, archives, mounts, and services
-> Gump installation, initialization, joining, and deployment
-> workload topology, scaling, replacement, and migration
-> event-driven reactions and failure recovery
```

Its surface syntax takes inspiration from Zing. It is not Zing, does not embed
Zing, and does not require users to know or install Zing. The language must be
designed specifically around infrastructure identities, observations, effects,
failure, reconciliation, security, and explanation.

This is more than an autoscaler language and more than a richer Gump manifest.
The autoscaler becomes an early program or library written for the language.
Gump becomes one especially capable execution frontier within the wider cloud
programming model.

## 2. The problem with the current tool boundary

A conventional stack separates one operation into unrelated representations:

```text
Terraform creates a server
-> exports an address
-> generates inventory
-> Ansible connects to that address
-> templates files
-> starts a service
-> another deployment tool installs the application
-> monitoring and autoscaling reconstruct the relationships later
```

The value returned by one system becomes loosely typed text consumed by the
next. Resource identity, ownership, secrets, failure state, and lifecycle
relationships are repeatedly flattened and reconstructed.

The language should instead carry typed values through the complete program:

```text
Server -> SSH frontier -> installed Gump node -> Gump cluster -> deployed app
```

A server is not reduced to an inventory line containing an IP address. It
remains a server value with provider identity, addresses, capabilities,
ownership, lifecycle, and receipts.

## 3. Target developer experience

An illustrative program may read as follows:

```text
module Production::Analytics.

use Provider::AWS.
use Host::Debian.
use Platform::Gump.

network := Aws createNetwork region: #Singapore cidr: "10.40.0.0/16".

server := Aws createServer: [
  network: network.
  architecture: #X86_64.
  memoryAtLeast: 8GiB.
].

on server using: #SSH do: [
  apt addRepository: Gump signedRepository.
  apt install: [ "ca-certificates", "gump" ].

  release := download
    url: "https://example.invalid/application.tar.zst"
    sha256: #ExpectedApplicationDigest.

  unpack release into: "/opt/application" atomically: true.
  copy "./application.conf"
    to: "/etc/application/application.conf"
    owner: "application"
    mode: 0640.

  service "application" ensure: #Running.
].

cluster := Gump initializeOn: server
  recoverySecret: (secret #GumpRecovery).

cluster deploy: "./autoscaler.capsule"
  providerCredential: (secret #AwsPrimary).
```

There is no generated inventory, output-variable handoff, shell glue, or
separate configuration language between provisioning and configuration.

The expected tool family is provisionally:

```text
captain check <program>
captain format <program>
captain plan <program>
captain explain <program>
captain run <program>
captain test
captain simulate <program> --event <event>
```

The final product name and command ownership remain open.

### 3.1 The program is the cluster

The root of a Captain program is a living cluster, not a provisioning script
that creates a cluster and exits. One root cluster establishes the program's
identity, authority boundary, desired operating envelope, baseline services,
and event-processing context.

```text
module Production::Forest.

use File.
use Net.
use Apt.
use Systemd.
use Provider::AWS.

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

  provider Aws primary: [
    regions: [ #Singapore ].
    maximumCost: 500USD monthly.
  ].

  on event #CapacityDeficit: [ lower using: #ScaleOut ].
  on event #NodeLost: [ lower using: #ReplaceNode ].
  on event #IdleCapacity: [ lower using: #Consolidate ].
].
```

The cluster form declares at least:

- minimum, burst, and maximum horizontal capacity;
- permitted vertical shapes and capability classes;
- regions, zones, failure domains, and provider frontiers;
- cost, quota, and disruption ceilings;
- baseline node-wide and cluster-wide services;
- replacement, consolidation, and scale-down policy;
- event handlers and lowerings;
- which effects may proceed automatically and which require approval.

The exact surface remains open, but "the program is the cluster" is a baseline
semantic decision. A future multi-cluster composition model must preserve each
cluster's separate identity and authority rather than weakening this root.

### 3.2 Local bootstrap and living handoff

Before the cluster exists, Captain runs locally. It reads local secret handles,
creates or adopts the first provider resources, installs Gump, initializes the
first node, grows to the declared minimum, and verifies the cluster.

Once Gump exists, Captain hands a compiled continuation of the same cluster
program to the Gump frontier in a signed Capsule. The living program then
handles events, scaling, replacement, migration, and reconciliation from
inside the cluster it created.

```text
local Captain runner
-> create first server
-> install and initialize Gump
-> expand to minimum viable cluster
-> package compiled cluster continuation and provider access
-> Gump runs the living cluster program
```

This is a handoff, not two configuration systems. Logical identities and
receipts cross the frontier without being flattened into inventory files.

After total cluster loss, no fictional in-cluster controller remains alive.
The operator runs Captain locally again. It rediscovers conservatively owned
provider resources, creates a new Gump incarnation, reintroduces the compiled
program and Capsules, and hands control back to the live cluster.

## 4. One language, several infrastructure forms

Infrastructure needs more than a single declarative resource form. The
language should make several concerns explicit and composable.

### 4.1 Topology

Topology describes what should normally exist and how its parts relate:

```text
fleet Analytics: [
  provider: AwsFleet.
  instances minimum: 2 maximum: 10.
  requires memoryAtLeast: 16GiB.
  requires acceleratorClass: #Nvidia.
].

volume AnalyticsData: [
  provider: AwsStorage.
  capacity: 500GiB.
  attachment: #Exclusive.
  retention: #Keep.
].

bind AnalyticsData to: Analytics [
  affinity: #Sticky.
  movement: #FullMigration.
].
```

Topology is not sufficient for every lifecycle, but it remains useful for
stable relationships, cardinality, constraints, ownership, and retention.

### 4.2 Procedures

Procedures perform construction and host configuration using ordinary control
flow and infrastructure-native effects:

```text
on servers parallelDo: [ :server |
  packageManager := detectPackageManager on: server.

  (packageManager = #Apt) ifTrue: [
    apt on: server install: [ "gump", "ca-certificates" ].
  ].

  (packageManager = #Rpm) ifTrue: [
    rpm on: server install: [ "gump", "ca-certificates" ].
  ].
].
```

The language is code: it has values, functions, modules, branches, iteration,
collections, errors, and reusable libraries. It is not a static object file
with increasingly elaborate interpolation.

### 4.3 Reactive lowerings

Lowerings transform observed events and authoritative state into proposed
action graphs:

```text
lowering MetricToAction frontier AwsFleet: [
  match event #MemoryStarvation: [
    metric type: "memory_utilization".
    window duration: "3m" aggregate: "average".
  ].

  check: [
    ensure (metric value > 85%).
    ensure (fleet current_size < 10).
    ensure (fleet cooldown_elapsed > "5m").
  ].

  build: [
    emit capacity Analytics increaseBy: 1.
  ].

  ifFail diagnostic #SCALE_OUT_BLOCKED
    "Memory high, but scaling is blocked by capacity or cooldown".
].
```

Metrics are evidence that may trigger planning. They are not infrastructure
authority. Fleet size, placement, membership, fencing, budget, and cooldown
checks use authoritative state supplied to the lowering.

### 4.4 Lifecycles

Lifecycles express operations whose correctness depends on ordered transitions
and failure handling:

```text
lifecycle MigrateDatabase: [
  steps: [
    provision destination.
    quiesce source.
    snapshot sourceVolume.
    restore snapshot to: destinationVolume.
    attach destinationVolume exclusivelyTo: destination.
    start destination.
    verify destination health.
    fence source.
    move publicAddress to: destination.
    retire source.
  ].

  compensate: [
    before source fenced: [ resume source ].
    after source fenced: [ preserve sourceVolume; preserve destinationVolume ].
  ].
].
```

"Sticky" is therefore not a boolean pretending to describe a database. It is
a relationship supported by explicit ownership, fencing, migration, retention,
and recovery behavior.

## 5. Infrastructure-native values and effects

The language needs ordinary programming types plus domain-native types:

```text
Duration, Instant, Bytes, Digest, CIDR, Address, Port, Region, Zone
Money, Rate, Architecture, Capability, ResourceId, LogicalAddress
Artifact, Server, Volume, Snapshot, Network, Service, Cluster
Secret[T], Receipt[T], Observation[T], Effect[T], Diagnostic
```

Infrastructure operations are typed effects rather than arbitrary SDK calls:

```text
provision, observe, download, verify, copy, render, unpack
installPackage, mount, attach, snapshot, migrate
startService, reloadService, publishDns, moveAddress
initializeCluster, joinCluster, deployCapsule
cordon, drain, fence, replace, destroy
```

Effects produce typed receipts. Receipts may be required by later effects:

```text
terminate
  node: ProviderNode
  drainedBy: DrainReceipt
  fencedBy: FenceReceipt
  authorization: DestroyGrant
  -> Receipt[Termination].
```

Code cannot manufacture a drain receipt, fence receipt, or current authority
grant. This permits the type and effect systems to prevent important classes of
unsafe ordering before execution.

## 6. Frontiers

A frontier is an execution and authority boundary. It defines:

- where observations and code evaluation occur;
- which typed effects are available;
- which identities and capabilities are in scope;
- which secret handles may be used;
- what connectivity and trust exist;
- how receipts, cancellation, and diagnostics return.

Examples include:

```text
LocalMachine
AwsAccount
DigitalOceanProject
SshHost
GumpCluster
GumpNode
ApplicationAttempt
```

The program moves through frontiers explicitly:

```text
LocalMachine
-> AwsAccount creates Server
-> Server opens SshHost
-> SshHost installs Gump
-> GumpNode initializes GumpCluster
-> GumpCluster deploys provider and application Capsules
```

Frontiers are also the basis for static effect checking and runtime authority.
A module targeting an AWS fleet cannot silently perform an undeclared DNS,
filesystem, or DigitalOcean effect.

## 7. Files, packages, archives, commands, and services

The language must go all the way to machine configuration. These operations
need native convergence semantics rather than thin aliases for shell commands.

### 7.1 Artifacts

Downloaded and built artifacts are content-addressed typed values:

```text
artifact := download url: releaseUrl digest: expectedDigest.
verified := verify artifact using: releaseKey.
copy verified to: server path: "/run/install/release.tar.zst".
unpack verified into: "/opt/application" atomically: true.
```

The runtime knows the expected digest, transfer status, destination, atomicity,
ownership, and completion receipt.

### 7.2 Files

Native file operations define content, ownership, mode, replacement,
notification, and sensitive-data behavior. They use safe path resolution and
atomic publication where applicable.

### 7.3 Packages

APT, RPM, and other package operations express an intended package result,
repository trust, version policy, and reboot implications. They do not blindly
run an update command on every evaluation.

### 7.4 Services

Service operations understand enablement, current state, restart/reload
notifications, readiness, deadlines, and platform-specific service managers.

### 7.5 Programmatic modules over binaries and APIs

Captain modules give operating-system binaries, native libraries, remote
protocols, and provider APIs stable programmatic faces. Application code uses
the module contract rather than repeatedly constructing command lines and
parsing stdout.

For example:

```text
use Grep.

matches := Grep find: pattern
  beneath: "/etc/application"
  recursive: true
  includeLineNumbers: true.
```

The result is a typed collection rather than an unbounded stdout string:

```text
Match [
  path: Path.
  line: i32.
  column: Optional[i32].
  content: Text.
].
```

The `Grep` implementation may invoke `/usr/bin/grep`, but it owns argument
construction, binary discovery, supported versions, exit-code interpretation,
output parsing, size bounds, cancellation, diagnostics, and sensitive-output
classification.

Stable module protocols may have several adapters:

```text
Net::Firewall -> Ufw | Firewalld | Nftables
Package       -> Apt | Dnf | Rpm
Service       -> Systemd | OpenRC | Launchd
```

The source normally imports the semantic module, such as `Net`, and the host
frontier resolves an adapter while showing the exact choice in `plan` and
`explain`. Code may import a specific backend when its behavior is genuinely
required.

A module does not have to wrap a process. Its implementation may use a native
library, kernel API, remote protocol, cloud API, or another Captain module. The
programmatic contract is the stable boundary.

### 7.6 Arbitrary commands and module growth

An explicit escape hatch remains necessary:

```text
exec "/opt/vendor/install" args: [ "--quiet" ] [
  unless fileExists: "/opt/vendor/.installed".
  timeout: 5m.
  creates: "/opt/vendor/.installed".
  output: #NonSensitive.
].
```

Arbitrary execution is visibly weaker than a native effect. The compiler and
plan must report which idempotency, observation, secrecy, and compensation
properties cannot be proven.

`exec` is also how the module ecosystem grows:

```text
one-off requirement
-> application uses exec
-> repeated requirement becomes a reusable module
-> widely used contract becomes a standard protocol
-> platform differences become adapters behind that protocol
```

A module may initially wrap a binary with `exec`, validate and parse its result,
and expose a typed function such as `Grep find`. Important implementations may
later move to a native library without changing programs that consume the
module contract.

Direct process execution uses an argv vector without shell interpretation by
default. A separate, visibly weaker shell effect is required for pipes,
redirection, expansion, or other interpreter behavior.

## 8. Planning and execution

The provisional pipeline is:

```text
source program
-> parse and type-check
-> secret, effect, authority, and boundedness analysis
-> partial evaluation
-> typed execution graph with deferred branches
-> plan and explanation
-> exact frontier authorization
-> effect execution and typed receipts
-> expansion of deferred branches
-> reconciliation until convergence or explicit failure
```

The system must not pretend that every program has a completely known static
plan. Runtime observations and provider results may decide later branches.

For example:

```text
1. Provision server
2. Observe operating-system family
3. Deferred branch:
   - Debian -> configure signed APT repository
   - RHEL -> configure signed RPM repository
   - otherwise -> diagnostic #UNSUPPORTED_OPERATING_SYSTEM
```

`plan` displays known effects, bounds, deferred decisions, maximum consequences,
and required authorities. It never claims certainty it does not possess.

## 9. Identity, reconciliation, and the state-file problem

Replacing Terraform while retaining an opaque authoritative state file would
miss the opportunity.

Every durable external resource receives a stable logical address:

```text
Production::Network
Production::GumpNodes[0]
Production::Database::Volume
Production::PublicAddress
```

Provider implementations bind logical addresses to external resources using
supported combinations of tags, names, provider idempotency identities, and
ownership metadata. A rerun observes the provider and reconstructs bindings.

An execution receipt or local cache may accelerate reconciliation, but it is
not the universe. Losing it must not make existing infrastructure unknowable.
When a provider cannot support safe discovery, the limitation is explicit and
the required registry mechanism must be bounded and protected. Ambiguous,
cross-incarnation, unknown, or manually modified resources are reported as
drift rather than silently adopted or destroyed.

Programs distinguish:

- desired logical identity;
- current provider identity;
- observed properties;
- last authorized operation;
- current ownership/incarnation;
- convergence and drift state.

## 10. Secrets

Secrets are opaque typed handles, not strings with a sensitive annotation:

```text
credential := secret #DigitalOceanPrimary.
```

Ordinary code cannot print, concatenate, compare, serialize, place in a path,
or interpolate a secret. Authorized effects may consume it through a protected
channel such as Macrun, an inherited descriptor, a Capsule protected segment,
or a provider-specific ephemeral credential exchange.

Secret bytes must not enter:

- source, plans, diagnostics, receipts, or execution graphs;
- process arguments;
- generated inventories or variable files;
- provider tags or user-data except for explicitly bounded one-use grants;
- Hiccup, Ringtail, or ordinary stdout/stderr;
- Gump's distributed K/V store.

The language toolchain should statically reject common secret-flow errors and
fail closed when it cannot preserve the declared channel.

## 11. Provider interfaces and lowering

Provider integrations publish versioned, signed type and effect interfaces.
Portable programs normally target abstract capabilities:

```text
Compute::Provision
Storage::Volume
Network::StableAddress
Dns::Record
Cluster::Node
```

Provider lowerings translate them into specific effects:

```text
AbstractCapacityPlan
-> Gump safety and lifecycle lowering
-> AWS EC2/EBS/Elastic-IP effect graph
```

Provider-specific effects remain available for workloads that genuinely need
them. The source must make that portability decision visible.

The compiler validates provider interface versions and effect shapes. Runtime
authorization independently binds the exact provider profile, operation,
resource identity, limits, controller fence, expiry, and idempotency identity.

## 12. Gump integration

Gump remains a small execution and cluster kernel. The language does not place
AWS, APT, RPM, database, or migration logic inside the Gump server.

The same cluster program crosses two execution contexts during its lifecycle:

1. A local runner performs zero-to-one work: provider construction, SSH host
   configuration, package installation, and initial Gump formation.
2. After formation, the local runner hands the compiled living continuation to
   a Gump frontier. That frontier performs deployment, placement-aware actions,
   scaling, draining, fencing, replacement, and reaction to events.

The root cluster identity, logical resource addresses, program revision, and
operation receipts bind the handoff. Local bootstrap and in-cluster operation
must not become separate sources of desired truth.

The architectural packaging remains open, but compiler, local runner, Gump
frontier adapter, and provider implementations should remain separable. A
cluster should not need a compiler or general source-language runtime in its
trusted kernel merely to run ordinary workloads.

Compiled programs and provider implementations may be distributed through
signed Capsules. Gump validates the bounded representation and current effect
authority; it does not trust arbitrary source text or shell scripts as cluster
control.

## 13. Relationship to the autoscaler

The capacity autoscaler is an early proving program, not a one-off subsystem.
It exercises:

- event matching and metric windows;
- authoritative checks and cooldowns;
- desired capacity and capability vectors;
- provider proposal and effect interfaces;
- machine provisioning and package installation;
- one-use Gump node enrolment;
- capability verification;
- vertical replacement, drain, fencing, and removal;
- cost, count, region, and disruption limits.

The same language mechanisms then support stable addresses, durable volumes,
database migration, DNS, certificates, and complete environment construction.

## 14. Explanation, testing, and simulation

World-class developer experience requires more than pleasant syntax.

`explain` should answer:

- what observation or source location produced an action;
- what alternatives were rejected and why;
- which checks, defaults, and policies applied;
- maximum cost and destructive scope;
- what is known now and what is deferred;
- which secrets and authorities are referenced by name;
- what constitutes successful completion;
- what compensation exists after each failure point.

Testing is part of the language:

```text
test "never exceed fleet ceiling": [
  given fleet currentSize: 10.
  given event #MemoryStarvation value: 94%.

  expect noEffects.
  expect diagnostic #SCALE_OUT_BLOCKED.
].

test "database never has two unfenced writers": [
  explore failuresDuring: #MigrateDatabase.
  invariant activeWriters <= 1.
].
```

The simulator should explore effect failure, timeout, lost response, retry,
partial completion, process restart, node loss, partition, and cancellation.
Infrastructure invariants should be testable before real resources are touched.

## 15. Safety defaults

The language and standard library should default to:

- create before destroy;
- retain durable storage unless deletion is explicit;
- fence the previous exclusive owner before attachment or promotion;
- drain a Gump node before infrastructure removal;
- preserve required memory quorum;
- never automatically remove the sole remaining server;
- use content digests and atomic file publication;
- use exact package and repository trust policies;
- bound retries, concurrency, output, downloads, and execution time;
- reconcile uncertain provider outcomes before retrying;
- give effects deterministic idempotency identities;
- treat destruction as stronger authority than creation;
- report drift instead of silently adopting it;
- keep telemetry as evidence rather than effect authority.

Defaults must remain visible in plans and explanations. Safety by surprise is
not good developer experience.

## 16. Explicitly rejected traps

The language should not become:

- YAML or TOML with a different punctuation style;
- a universal static schema that must predict every future infrastructure
  lifecycle;
- a collection of provider SDK calls with no effect model;
- shell scripts with cloud-administrator credentials;
- an engine whose local mutable state file defines reality;
- a planner that executes hidden provider calls;
- a system that claims a fully known plan for dynamic programs;
- a requirement that every application author implement a controller;
- an excuse to place cloud-specific code and credentials in the Gump kernel.

## 17. Baseline invariants

1. The language covers provider construction, host configuration, Gump
   operation, and continuous lifecycle without inventory-file glue.
2. Values retain typed identity across frontiers.
3. Source programs contain real abstraction and control flow.
4. Infrastructure effects are explicit, typed, bounded, and explainable.
5. Topology, procedures, reactions, and lifecycles compose without being
   collapsed into one giant declarative object model.
6. Secrets are opaque handles and never ordinary program strings.
7. Destructive effects require stronger authority and prerequisite evidence.
8. External resources can be rediscovered without treating a local state file
   as the only source of truth.
9. Native file, package, archive, and service effects are convergent and
   observable.
10. Arbitrary commands are possible but visibly carry weaker guarantees.
11. Dynamic plans expose deferred branches honestly.
12. Programs and workflows are testable under injected failures.
13. Provider implementations remain independent and replaceable.
14. Gump remains useful without the language or an autoscaler deployed.
15. The autoscaler is expressible as a normal program using the same public
    mechanisms available to other infrastructure programs.
16. One Captain program has one living root cluster and explicit authority
    boundaries.
17. Local zero-to-one execution hands the same compiled cluster program to
    Gump for continuous operation.
18. Modules expose stable typed interfaces over binaries, libraries, protocols,
    or APIs; programs do not depend on raw stdout conventions.
19. Host adapter selection is visible and never silently changes semantics.
20. `exec` is both an honest escape hatch and the path by which repeated
    process integrations mature into reusable modules.

## 18. Questions for refinement

1. What is the language's name, file extension, module identity, and package
   model?
2. Which parts of the Zing-inspired surface are retained, changed, or removed?
3. Is the general computation model deliberately bounded, or fully general
   around a bounded effect system?
4. What is the canonical compiled form: bytecode, typed graph, automata, or a
   combination?
5. Which code runs locally, inside a Capsule workload, or in a dedicated Gump
   frontier runtime?
6. How are provider interfaces distributed, pinned, signed, and documented?
7. How are deferred branches bounded before execution?
8. Which effects are compiler primitives and which belong to standard or
   provider libraries?
9. How do reusable lifecycle libraries expose application-specific callbacks
   without weakening fencing?
10. What discovery contract replaces Terraform-style resource state for
    providers with poor tagging or idempotency support?
11. Is a small encrypted receipt registry ever required, and if so, where is
    its authority boundary?
12. How does plan review bind to the exact program, observations, provider
    catalogue, and later authorization?
13. How do long-running lowerings upgrade without duplicating or abandoning
    in-flight operations?
14. What effect and secret information is available to IDE tooling without
    exposing runtime values?
15. Which three complete example systems form the language's initial design
    corpus?
16. How is the local-to-Gump handoff represented, authorized, upgraded, and
    recovered without creating two program owners?
17. Does the first language version enforce exactly one root cluster at compile
    time, or is that a package-level rule?
18. What module interface description is sufficient for autocomplete, effect
    analysis, adapter resolution, version negotiation, and typed result
    decoding?
19. Which binary-backed modules belong in the initial standard library, and
    what is the compatibility policy for host tool versions?

The baseline direction is nevertheless clear: create a real programming
language for cloud infrastructure, spanning construction through continuous
operation, with infrastructure-native effects and Gump as a composable runtime
rather than reducing infrastructure as code to configuration files.
