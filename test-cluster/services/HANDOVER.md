# Layer 3 Runbook

This document describes the intended stage-3 boot sequence now that the app deploy pack lives directly in `services/`.

## Goal

Plug the constellation app layer into the existing infrastructure boot flow so a newly created cluster reaches a minimally usable application baseline.

The target baseline is:

1. `k2db-api` deployed and reachable
2. `k2rbac-api` deployed and reachable
3. `k2login` deployed and reachable
4. optional `k2mx-api` deployed when enabled
5. optional core init completed when explicitly enabled
6. passing service health checks on every enabled layer-3 job

At that point, frontend apps can rely on the base constellation model.

## Scope Boundary

This pack is layer 3 only.

Infra should assume the following already exist before this pack runs:

1. Terraform substrate
2. lower-layer Ansible platform lift
3. Nomad cluster
4. Consul cluster
5. Vault or an equivalent secret-delivery path
6. Docker runtime on Nomad clients
7. DNS and edge routing

This pack does not build those pieces.

## Files To Use

Start here:

1. `services/README.md`
2. `services/HANDOVER.md`
3. `services/site.yml`
4. `services/group_vars/constellation.example.yml`
5. `services/inventory/hosts.example.yml`

Reference docs:

1. `docs/constellation-layer3-bootstrap.md`
2. `docs/constellation-layer3-vars.example.yml`
3. `docs/constellation-auth-infra.md`

## What The Pack Already Defines

The pack already defines:

1. the required services
2. the install order
3. the variable schema
4. the core-init command contract
5. the readiness and acceptance checks

The pack now implements the default environment-specific deployment execution for the currently owned Nomad jobs.

What still depends on your environment is only:

1. the concrete inventory target
2. the real domains, image tags, and secret values
3. whether you want Nomad Variables or Vault for secret delivery
4. whether you want the optional core-init commands enabled

## Required Infra Work

The operator needs to do four things.

### 1. Replace example values

Use the example vars file as the schema, not as production config.

Replace placeholders in:

1. `services/group_vars/constellation.example.yml`
2. `services/inventory/hosts.example.yml`

Create environment-specific versions with:

1. real Nomad manager target
2. real domains
3. real image tags
4. real secret references or values
5. real deploy mode choice: `vault` or `nomad-var`

### 2. Implement service deployment wiring

The service contract is already defined, but infra must connect it to the live cluster.

The role already performs the actual deploy behavior behind the existing Nomad model.

For each enabled service it now:

1. renders the correct Nomad template from this repo
2. injects secrets through the selected backend
3. submits the job to Nomad
4. waits for running allocations and passing Consul checks

Do not introduce a second deployment model.

### 3. Implement core init execution

Core-init remains operator controlled because these commands can be destructive or one-time.

The current implementation uses `nomad alloc exec` against the running service allocations.

The optional core-init covers:

1. `k2db-api-server init` or `recover`
2. `k2db-api-server config set`
3. `k2rbac-api-server bootstrap-admin --email ...`

Important distinction:

1. `init` and `bootstrap-admin` are one-time bootstrap actions
2. `recover` and `config set` are repeatable reconcile actions

Keep that distinction in the run sequence.

### 4. Enforce acceptance checks

The run should not report success only because the jobs started.

The run fails if any enabled service does not achieve passing Consul health checks.

At the moment the implemented gate is:

1. `k2db-api` passes its registered health checks
2. `k2rbac-api` passes its registered health checks
3. `k2login` passes its registered health checks
4. `k2hello` passes its registered health checks when enabled
5. `k2mx-api` passes its registered health checks when enabled

Browser-path auth checks are still a separate higher-level test concern even though this repo now contains a `k2hello` Nomad job definition.

## Recommended Boot Sequence

Use this order.

1. confirm lower-layer infra is healthy
2. load layer-3 environment-specific vars
3. validate the layer-3 input contract
4. deploy `k2db-api`
5. run `k2db-api` init or recover as appropriate
6. reconcile `k2db-api` runtime config
7. deploy `k2rbac-api`
8. if the auth store is empty, bootstrap the first RBAC admin
9. deploy `k2login`
10. optionally deploy `k2mx-api`
12. run readiness checks
13. mark the cluster application baseline ready

## Service Ownership Notes

Keep these boundaries intact.

1. `k2login` is the browser-facing auth boundary
2. `k2rbac-api` is not the direct browser integration point
3. published Docker Hub images are the deployable artifacts for layer 3
4. service templates in this repo remain the canonical Nomad job source

## What Infra Should Not Change During Integration

Do not change these architectural assumptions while integrating the pack:

1. do not move browser auth directly to RBAC
2. do not require local image builds during cluster boot
3. do not replace the repo's Nomad templates with an unrelated parallel job model
4. do not mark boot successful without auth-path verification

## Suggested Infra Deliverables

When infra finishes integration, the expected deliverables are:

1. environment-specific inventory and vars files
2. real task implementations behind the layer-3 role
3. a boot-pipeline step that runs this pack after platform lift
4. failure-gated readiness and auth-path acceptance checks

## Short Version

If infra only reads one page, it should be this:

1. use `services/` as the separate app deploy run
2. replace example vars with real environment values
3. let the role render and submit the existing Nomad jobs
4. enable core-init only when you really intend to run bootstrap actions
5. fail the run on any broken service health