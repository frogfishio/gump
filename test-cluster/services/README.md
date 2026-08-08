# Layer 3 App Deploy

This directory is the owned stage-3 app deploy run.

Stage 2 builds the raw cluster:

1. Terraform creates the machines.
2. top-level `ansible/` lifts the cluster substrate.
3. top-level `nomad/` remains only the smoke-test harness.

Stage 3 lives here and bolts the current application jobs onto that already-healthy cluster in a separate run.

## What Lives Here

1. `site.yml`: stage-3 playbook entrypoint
2. `inventory/hosts.example.yml`: example control-host inventory if you want a dedicated stage-3 inventory
3. `group_vars/constellation.example.yml`: example layer-3 variables with actual deployment fields
4. `nomad/templates/`: canonical app Nomad job templates used by this run
5. `roles/constellation_layer3/`: validation, render, secret push, job submit, optional core-init, and health checks
6. `docs/variable-matrix.md`: service and secret mapping reference

## Execution Model

This run is meant to execute against one control node in the cluster, typically `frogfish01` over SSH as `manager`.

The role now does the concrete work:

1. validates layer-3 inputs
2. renders the existing Nomad job templates in `services/nomad/templates/`
3. pushes secrets into Nomad Variables by default
4. optionally pushes secrets into Vault when explicit Vault paths are provided
5. submits jobs to Nomad
6. waits for running allocations and passing Consul checks
7. optionally runs explicit core-init commands when you enable them

Default deploy mode is `nomad-var` because that is the only concrete secret path model already documented in this repo.
Vault mode is supported only when you provide explicit `vault_kv_put_path` and `vault_kv_read_path` values for each enabled service.

## Current Deployable Services

The real templates currently present in this repo are:

1. `k2db-api`
2. `k2rbac-api`
3. `k2login`
4. `k2hello`
5. `k2mx-api`

## Separate Run Examples

Using the dedicated example inventory:

```sh
cd services
cp inventory/hosts.example.yml inventory/hosts.yml
cp group_vars/constellation.example.yml group_vars/constellation.yml
ansible-playbook -i inventory/hosts.yml site.yml -e @group_vars/constellation.yml
```

Reusing the stage-2 Terraform inventory and targeting the bootstrap node directly:

```sh
cd services
ansible-playbook -i ../ansible/inventory/terraform.ini site.yml \
	-e constellation_layer3_target=frogfish01 \
	-e @group_vars/constellation.yml
```

From repo root, the equivalent shortcut is:

```sh
make services
```

For `k2hello`, set `K2HELLO_DOMAIN` explicitly before deploy.
It must not fall back to `K2LOGIN_DOMAIN`, or the hello edge registration will be published under the login hostname.

## Core Init Safety

Core-init is disabled by default.

You must explicitly opt in before any one-time app bootstrap commands run:

1. set `constellation_layer3_core_init.enabled: true`
2. choose `k2db_action: init` or `recover`
3. set `rbac_bootstrap_admin: true` only for an empty auth store

That keeps the default stage-3 run safe for repeated deploys.