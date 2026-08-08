# Converting `test-cluster` into a Three-Node Gump Test Cluster

> Status: reviewed conversion blueprint  
> Scope: reusable DigitalOcean substrate, Gump installation, cluster formation,
> live-like acceptance tests, and fault injection

## 1. Conclusion

The copied environment is a good starting point. Its Terraform layer already
models the required substrate: three Ubuntu 24.04 machines in one region, each
with public administrative access, private networking, and generated Ansible
inventory.

The old platform layer is mostly irrelevant to Gump. The correct conversion is
to preserve the small Terraform core and host-hardening work, remove the
HashiCorp/application stack, and add only the machinery needed to install,
form, test, break, and recreate a Gump cluster.

This environment should test Gump, not quietly supply functionality that Gump
is supposed to provide. Consul, Nomad, Vault, Valkey, Caddy, and Kismet must not
be installed as implicit dependencies of the baseline test cluster.

## 2. Safety findings before any apply or destroy

The copied tree contains local or tracked material that may belong to the old
cluster:

- stale Terraform state that still lists three destroyed DigitalOcean droplets;
- ignored `terraform.tfvars` and `.envrc` files;
- ignored Vault initialization material;
- tracked Ansible secret files;
- a tracked deployment-cache archive;
- tracked application and Kismet binaries.

The three copied DigitalOcean resources have been destroyed. The copied local
state still lists those three droplet resource addresses, so it is stale and
must not be treated as the starting state for the new cluster. Archive it only
if historical evidence is useful, then initialize a clean state before the
first reviewed plan. Do not run `terraform destroy` against the stale copy.

Do not print, inspect in automation output, or recommit copied credentials.
Determine out of band whether the tracked secret files contain real values. If
they do, rotate those values and remove the files from the repository and its
history. Merely adding them to `.gitignore` is not remediation.

The new cluster should begin with new state. Local state is acceptable for this
disposable harness if it remains ignored and protected, although a deliberately
configured encrypted remote backend is preferable once multiple operators need
to control the environment.

## 3. Keep, rewrite, and remove

### Keep with small changes

| Existing part | Decision | Required change |
|---|---|---|
| `terraform/main.tf` | Keep | Rename hosts/tags/resources to Gump and tighten public ingress |
| `terraform/variables.tf` | Keep | Rename descriptions; add admin CIDRs and Gump network variables |
| `terraform/inventory.tf` | Keep | Emit `gump` group and stable seed/member metadata |
| `terraform/ansible_inventory.tpl` | Keep | Rename group and include private address plus node ordinal |
| Ubuntu 24.04 droplets | Keep | Do not compile Rust on them; install a prebuilt Linux artifact |
| `manager` SSH user | Keep | Continue key-only administrative access |
| SSH hardening/fail2ban | Keep | Extract into a small `base` Ansible role |
| UFW default deny | Keep | Replace old product ports with private Gump rules |
| `Makefile` | Rewrite | Expose Gump lifecycle and test targets only |

### Remove from the baseline

| Existing part | Reason |
|---|---|
| Nomad | Gump is the workload placer and supervisor being tested |
| Consul | Gump cluster memory and Hiccup must be tested directly |
| Vault | Gump's own in-memory custody/unseal paths must be tested |
| Valkey | Introduces unrelated persistence and state |
| Caddy and edge helpers | Gump is not an ingress proxy; publish providers are optional |
| Docker | Not needed for native baseline; add later only for OCI-driver tests |
| `services/` | Old constellation application deployment layer |
| `infra/` | Old application handover, scripts, fixtures, and edge configuration |
| `nomad/` | Old substrate smoke job |
| `ext/` | Old Darwin binaries are not deployable to Linux droplets |
| old Ansible templates | Specific to Nomad, Consul, Vault, Valkey, and Caddy |
| old secret/deploy-cache files | Product-specific and potentially sensitive |

Kismet should later be tested as a Capsule deployed by Gump with
`--nodes=all`; it must not be installed by Ansible.

## 4. Target directory shape

```text
test-cluster/
  README.md
  Makefile
  .gitignore
  terraform/
    main.tf
    variables.tf
    inventory.tf
    ansible_inventory.tpl
    .terraform.lock.hcl
  ansible/
    ansible.cfg
    site.yml
    inventory/                 # generated and ignored
    group_vars/
      gump.example.yml
    roles/
      base/
        tasks/main.yml
      gump_host/
        tasks/main.yml
        templates/gump.service.j2
  scripts/
    build-artifact.sh
    install.sh
    form.sh
    status.sh
    smoke.sh
    destroy-runtime.sh
    faults/
      kill-node.sh
      partition-node.sh
      heal-node.sh
      stop-majority.sh
      restart-empty.sh
  fixtures/
    finite-native/
    continuous-native/
    noisy-output/
    process-tree/
    hiccup-peer/
  evidence/                    # generated and ignored
```

The scripts are operator/test drivers. They must call public Gump commands and
must not manipulate Gump's internal state to manufacture passing results.

## 5. Terraform target

### Machines

- exactly three droplets by default;
- Ubuntu 24.04 x86-64;
- one region and VPC/private network;
- the existing inexpensive 1-vCPU/2-GiB size is sufficient for initial native
  control-plane testing if builds occur elsewhere;
- monitoring enabled, backups disabled;
- stable inventory ordinals `gump01`, `gump02`, and `gump03`.

Because the old droplets are gone and the new cluster starts from clean state,
Terraform resource addresses may be renamed coherently without a state move.

### Network policy

Public ingress should allow only SSH, restricted to configured administrative
CIDRs when practical. Remove unconditional public HTTP and HTTPS rules.

Private peer ingress should allow:

- the configured Gump QUIC cluster port over UDP from the three private IPs;
- any explicitly configured Gump control stream port if the implementation
  does not share the QUIC listener;
- a small configurable workload-test TCP/UDP range between private nodes;
- ICMP between nodes if retained for diagnostics.

No Raft, custody, Hiccup keeper, telemetry relay, or application-introduction
traffic should traverse public addresses. Outbound HTTPS remains available for
S3-compatible object storage and package/artifact retrieval.

The exact Gump ports are not currently frozen in the executable interface.
They should be Ansible/Terraform variables, not duplicated literals. The
harness must consume the same documented defaults as the product once those
defaults land.

Add a DigitalOcean firewall in addition to UFW so an accidental host-firewall
reset does not expose cluster ports.

## 6. Host configuration target

The baseline Ansible run should do only the following:

1. wait for cloud-init and SSH;
2. install small diagnostic/runtime packages;
3. configure SSH hardening and fail2ban;
4. disable swap and remove any old swap activation;
5. ensure core dumps are disabled at the service and host policy boundary;
6. create an unprivileged `gump` account;
7. install the verified Linux Gump binary atomically;
8. create `/run/gump` as a private runtime directory;
9. install, but do not prematurely start, the Gump service definition;
10. apply private-only Gump firewall rules;
11. assert that prohibited old services and durable Gump-state directories are
    absent.

The cluster must not create `/var/lib/gump`, a persistent Raft directory, a
secret environment file, or a reusable node identity file. The Gump binary and
ordinary operating-system configuration may persist; Gump's cluster state,
node identity, credentials, and application materialization must not.

Use `/run/gump` for sockets, transient materializations, and attempt roots so a
machine reboot removes them naturally. Set permissions to `0700` and a service
umask of `0077`.

The service definition should at minimum set:

- `User=gump` and `Group=gump` unless a tested driver capability requires a
  more privileged helper boundary;
- `RuntimeDirectory=gump` and a private runtime mode;
- `LimitCORE=0`;
- an explicit `LimitMEMLOCK` compatible with the advertised hardening mode;
- bounded file/process limits;
- restart behavior that does not pretend it can recover missing bootstrap or
  custody secrets from disk.

Systemd hardening must be validated against native/script/OCI driver needs. Do
not add restrictions that silently change the arbitrary-workload contract.

## 7. Build and installation flow

Do not install Rust and build separately on the three nodes. Produce one
`x86_64-unknown-linux-gnu` release artifact from the current commit, record its
source revision and checksum, and upload the identical bytes to all nodes.

Installation must:

1. verify the artifact checksum locally and remotely;
2. upload to a temporary filename;
3. set root ownership and executable permissions;
4. atomically replace `/usr/local/bin/gump`;
5. confirm all three nodes report the same build identity;
6. retain enough local evidence to reproduce the tested build.

This artifact path can later be replaced with signed release downloads without
changing cluster formation or testing semantics.

## 8. Cluster formation flow

The orchestration should mirror the intended Terraform → Ansible → Gump model:

1. Terraform creates three bare machines and inventory.
2. Ansible installs the same Gump binary and host policy everywhere.
3. The controller starts `gump01` with `gump server --init` and bootstrap
   parameters supplied through the product's secure input mechanism.
4. The controller waits for the first member to report a one-voter cluster.
5. It obtains fresh join authorization from the first member.
6. It starts `gump02` and `gump03` with `gump server --join`, supplying join
   authorization without writing it to either machine.
7. It waits for learners to transfer state and promote through the real joint
   membership path.
8. It verifies three voters, one-failure memory tolerance, a current controller,
   three agents, custody health, and identical cluster identity/incarnation.

Secrets must not appear in command-line arguments, Terraform state, generated
inventory, Ansible facts, task output, systemd unit files, journald, or remote
temporary files. Prefer a Gump `--params-fd`/stdin contract driven over SSH.
Ansible's ordinary module staging should not be assumed safe for plaintext
bootstrap material merely because `no_log` hides console output.

Static non-secret parameters such as private bind addresses and peer ports may
live in ordinary unit configuration. Join credentials, connector credentials,
unseal material, and custody shares may not.

Node restart and total-cluster restart must be distinct:

- when peers survive, a restarted node enrolls with a new ephemeral identity
  and recovers live memory/custody from the cluster;
- after all nodes lose memory, the controller explicitly initializes a new
  empty cluster and later reintroduces selected Capsules from S3.

The harness must never make full loss look like transparent persistence.

## 9. Object storage

Use a real S3-compatible bucket separate from production. The test principal
should be restricted to a dedicated prefix and the exact operations required by
the connector, including quarantine, conditional immutable publication, range
reads, listing, and explicit purge tests.

Before using a provider, run Gump's conditional-copy capability probe. A
provider that ignores the required precondition must fail the environment
setup rather than weaken immutability.

Bucket versioning and retention choices must be explicit because they affect
purge tests. The bucket is the one intentionally durable part of the Gump test
system; Terraform destruction must not silently delete it unless the operator
selects a separate explicit purge target.

Connector credentials must be delivered through the same memory-only bootstrap
path described above. Do not create a remote `.env`, credentials file, or
systemd `EnvironmentFile` containing them.

## 10. Local secret entry with macrun

macrun is the local secret-entry boundary for the macOS-operated test cluster.
It stores plaintext values in macOS Keychain and injects them only into an
explicit child process. It is not installed on cluster nodes and does not
replace Gump's protected Capsule segment, unseal authority, or in-memory
custody.

Use macrun only in `run`-style flows. The harness must not call any command that
prints resolved values for capture by a shell, JSON parser, Make variable,
Ansible fact, or log.

Recommended scopes are separate so each child receives only the class of
secrets it needs:

| macrun scope | Intended child | Secret classes |
|---|---|---|
| `gump-test/infra` | Terraform | DigitalOcean API token only |
| `gump-test/cluster` | local Gump formation client | S3 connector and recovery/unseal bootstrap values |
| `gump-test/fixtures` | `gump deploy` | application values used by test Capsules |

macrun 2.x uses `macrun run PROJECT ENVIRONMENT -- COMMAND`. Scripts centralize
that syntax in one small launcher rather than embedding it across Terraform,
Make, Ansible, and every test.

### Infrastructure flow

Terraform should be invoked as the macrun child so the DigitalOcean provider
reads `DIGITALOCEAN_TOKEN` directly from its process environment:

```text
macrun run gump-test-cluster infra -- terraform -chdir=test-cluster/terraform plan
macrun run gump-test-cluster infra -- terraform -chdir=test-cluster/terraform apply
```

Sensitive provider values must not be copied into `terraform.tfvars`,
`TF_VAR_*` files, Make variables, or command-line arguments. Non-sensitive
settings such as region, size, SSH fingerprint, and administrative CIDRs may
remain ordinary Terraform inputs.

### Cluster formation flow

Base Ansible configuration requires no cluster secrets and should run without
macrun. Formation should then be launched locally under the dedicated cluster
scope. The local Gump/orchestration client receives S3 and recovery inputs from
macrun and passes them to Gump through the product's secure stdin/inherited-FD
bootstrap contract.

Do not hand plaintext cluster values to Ansible. `no_log` hides output but does
not prove that module staging, facts, remote temporary files, or process
arguments remained secret-free.

### Application deployment flow

The intended developer path is direct:

```text
macrun run gump-test-cluster fixtures -- gump deploy <fixture>
```

Gump reads only the values declared by the manifest, constructs the protected
configuration in its own process, encrypts it into the Capsule, and does not
persist plaintext. The resulting sealed Capsule—not a macrun archive—is the
durable deployment artifact.

Until macrun supports selecting individual keys for a child, keep each fixture
scope minimal. Once key selection is available, deployment wrappers should
pass only the manifest-declared names. Gump should ignore unrelated inherited
environment variables and must never serialize the whole process environment.

### Evidence and error handling

- macrun metadata and scope names may be logged; resolved values may not;
- command tracing (`set -x`) must be disabled in secret-bearing launchers;
- subprocess errors must be redacted before entering Ratatouille or evidence;
- test canaries should prove values do not appear in Terraform state, Ansible
  artifacts, SSH commands, systemd units, journals, S3 public bytes, or Gump
  telemetry;
- CI/CD remains a separate integration because macrun intentionally targets
  local macOS development rather than unattended runners.

## 11. Acceptance ladder

### T0 — Substrate

- exactly three intended nodes exist;
- private addresses are distinct and mutually reachable;
- only approved public ingress is open;
- swap and core dumps are disabled;
- prohibited old services are absent;
- all nodes run the same Gump binary digest.

### T1 — Formation

- initialize one member;
- join the other two as learners and promote them;
- report three memory voters and one-node failure tolerance;
- prove no Gump state files exist outside `/run/gump`.

### T2 — One-node product path on the live substrate

- build a Capsule locally with a canary protected value;
- publish it to real S3-compatible storage;
- accept live intent;
- place and execute a finite native workload;
- capture stdout/stderr through Ratatouille;
- report a truthful receipt and clean all attempt material.

### T3 — Normal three-node behavior

- run a continuous multi-instance workload;
- observe spread and resource accounting;
- stop/restart one application attempt and reconcile it;
- move an instance and retain logical workload identity;
- verify telemetry follows attempts rather than machines.

### T4 — Controller/member faults

- kill the current controller and continue through a new fenced controller;
- restart the old controller and reject its stale effects;
- partition one member and continue on the majority;
- partition the controller into a minority and prove it cannot mutate;
- heal partitions and verify one committed history.

### T5 — Custody and secret faults

- restart an agent and redeliver only to its new authorized attempt;
- replay a prior attempt/fence delivery and reject it;
- lose one custodian while threshold remains and continue;
- lose the custody threshold and enter the specified resealed behavior;
- scan filesystem, process output, telemetry, errors, and S3 public bytes for
  the canary.

### T6 — Total memory loss

- stop all three nodes and clear all `/run/gump` state;
- initialize a replacement cluster and prove it has zero desired workloads;
- list inert Capsules from S3;
- explicitly reintroduce one selected Capsule;
- prove unselected and previously finite work does not start.

### T7 — Hiccup and movement

- deploy two or more Hiccup-capable instances;
- discover current peers through `@self` without seed addresses;
- move/restart a peer and receive its new stamped attempt/address;
- partition/kill keepers and observe bounded degraded discovery;
- prove health and control-plane progress remain independent.

### T8 — Optional breadth

- deploy Kismet as a Capsule with `--nodes=all` and no Ansible-installed Kismet;
- run the OCI driver after Docker/containerd is deliberately enabled;
- run synthetic GPU/gang feasibility without requiring physical GPUs;
- exercise publication, output, checkpoint, HSM/KMS, and purge providers only
  when their respective test profiles are selected.

## 12. Fault-injection requirements

Fault scripts must be reversible, target exact node identities, and print the
expected invariant before acting. They should use SSH plus systemd/network tools
only; they must not edit Raft memory or fabricate observations.

Required operations are:

- terminate or kill a selected Gump process;
- stop a selected node's Gump service;
- isolate one node from Gump private traffic while preserving SSH control;
- isolate a chosen pair/minority;
- add bounded latency, loss, duplication, and reordering;
- heal all injected network state;
- clear only the validated `/run/gump` runtime target while Gump is stopped;
- restart one member or deliberately initialize a new empty cluster;
- capture status, telemetry gaps, process trees, open files, memory, and journal
  metadata without capturing protected payloads.

Every destructive helper must resolve the target from generated inventory,
refuse broad or empty targets, and have a corresponding heal/recovery action.

## 13. Makefile target contract

The converted Makefile should expose a small honest interface:

```text
make plan              # Terraform plan only
make infra             # create/update three droplets
make configure         # base hardening + install binary
make form              # init first member, join the other two
make verify            # T0 + T1
make smoke             # current implemented acceptance ladder
make fault-leader      # controller-loss scenario
make fault-minority    # minority partition scenario
make heal              # remove injected network faults
make restart-empty     # explicit total-memory-loss scenario
make evidence          # collect safe test evidence
make destroy           # destroy compute only after explicit confirmation
```

`make up` may remain as a convenience alias for `infra`, `configure`, `form`,
and `verify`, but it must stop on the first failed gate. There should be no
`services` or Nomad deployment stage.

## 14. Implementation dependencies exposed by this harness

The infrastructure can be simplified immediately, but full formation must wait
for stable product interfaces for:

- `gump server --init` and `--join`;
- non-secret bind/advertise parameters;
- secure bootstrap parameter input over stdin or inherited descriptor;
- join authorization generation and consumption;
- versioned status and cluster membership output;
- clean shutdown and member restart behavior;
- artifact build identity;
- S3 connector bootstrap configuration;
- diagnostic output that reveals guarantees without revealing secrets.

These are not reasons to retain Nomad, Consul, or Vault. The harness should fail
clearly at an unavailable Gump interface until that interface lands.

## 15. Safe conversion order

1. Archive or remove the stale copied Terraform state and initialize clean
   state for the new cluster.
2. Resolve and rotate any real copied secrets, then store replacement values in
   the appropriate macrun scopes.
3. Convert names, tags, firewall, and inventory and review a fresh Terraform
   plan before creating resources.
4. Replace the 991-line Ansible platform playbook with the small base/Gump roles.
5. Remove old product directories and tracked artifacts after reviewing the
   exact deletion list.
6. Add binary build/install verification.
7. Add T0 substrate checks.
8. Verify the macrun 2.x scopes through the one local launcher.
9. Add formation commands as the matching Gump CLI interfaces land.
10. Grow the acceptance ladder from T1 upward; never simulate a missing Gump
    feature with an old platform dependency.

The outcome is intentionally boring infrastructure. The interesting behavior
must come from Gump.
