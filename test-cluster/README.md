# Gump three-node test cluster

This directory provisions three disposable Ubuntu 24.04 nodes on DigitalOcean,
installs one identical Gump release artifact on every node, and drives Gump's
public cluster and workload interfaces through live-like acceptance and fault
tests.

It deliberately does not install Nomad, Consul, Vault, Valkey, Caddy, Kismet,
or an application platform. Those would hide the behavior this environment is
meant to test.

See [GUMP_CONVERSION.md](GUMP_CONVERSION.md) for the complete rationale, target
topology, acceptance ladder, and fault matrix. See
[MACRUN_INTEGRATION.md](MACRUN_INTEGRATION.md) for the memory-only local secret
boundary.

## Current status

The live infrastructure and hardened-host scaffold are present. Cluster
formation and workload acceptance targets intentionally fail with a clear
message until the matching `gump server --init`, `--join`, deploy, and status
interfaces land. The harness must never emulate a missing Gump feature with
another platform.

## Prerequisites

- Terraform 1.5 or newer
- Ansible
- a Linux x86-64 Gump release binary
- an uploaded DigitalOcean SSH key
- macrun with an `infra` scope containing `DIGITALOCEAN_TOKEN`
- later, a `cluster` scope for S3/recovery inputs and fixture scopes for
  application values

Create `terraform/terraform.tfvars` from
`terraform/terraform.tfvars.example`. It contains only non-secret settings.
Keep the DigitalOcean token in macrun, never in the tfvars file.

Create the infrastructure scope using macrun's secure prompt:

```sh
macrun set gump-test-cluster infra DIGITALOCEAN_TOKEN
```

The future cluster-formation path will use a separate scope. The currently
implemented S3 connector names can be prepared independently:

```sh
macrun set gump-test-cluster cluster GUMP_S3_ENDPOINT
macrun set gump-test-cluster cluster GUMP_S3_BUCKET
macrun set gump-test-cluster cluster GUMP_S3_ACCESS_KEY
macrun set gump-test-cluster cluster GUMP_S3_SECRET_KEY
macrun set gump-test-cluster cluster GUMP_S3_REGION
```

These commands prompt for values; values do not appear in the shell arguments.
Recovery/unseal key names will be added only when that public Gump interface is
frozen.

## Workflow

```sh
make init
make plan
make infra
make configure-base
make install-gump GUMP_ARTIFACT=/absolute/path/to/linux/gump
make verify
```

`make infra` waits for cloud-init and verifies the non-root operator account on
all three nodes. This matters because DigitalOcean can report a droplet created
before its initialization script has completed.

When the cluster CLI contract lands:

```sh
make form
make smoke
```

`make plan`, `make infra`, and `make destroy` launch Terraform through macrun
2.x using the one adapter in `scripts/macrun-exec.sh`.

## Safety

- `make destroy` requires `CONFIRM_DESTROY=YES`.
- Generated inventory, evidence, Terraform state, plans, and tfvars are ignored.
- Ansible receives no plaintext cluster or application secrets.
- Gump runtime state belongs under `/run/gump`; `/var/lib/gump` is prohibited.
- Swap is disabled on every node.
- Only SSH is publicly reachable; Gump and fixture traffic is private-only.
