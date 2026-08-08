# Infra Handover: Layer 3 Constellation Bootstrap

This document explains what the infra team should do with the layer-3 pack in this repository.

It is written as an operational handoff, not as design notes.

## Goal

Plug the constellation app layer into the existing infrastructure boot flow so a newly created cluster reaches a minimally usable application baseline.

The target baseline is:

1. `k2db-api` deployed and reachable
2. `k2rbac-api` deployed and reachable
3. `k2login` deployed and reachable
4. `k2hello` deployed, reachable, and exported as the reference layer-3 relying app
5. core init completed
6. auth-path acceptance checks passing

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

1. `infra/layer3-ansible/README.md`
2. `infra/layer3-ansible/HANDOVER.md`
3. `infra/layer3-ansible/SECRETS.md`
4. `infra/layer3-ansible/site.yml`
5. `infra/layer3-ansible/group_vars/frogfish.current.yml`
6. `infra/layer3-ansible/group_vars/constellation.example.yml`
7. `infra/layer3-ansible/inventory/hosts.example.yml`

Reference docs:

1. `docs/constellation-layer3-bootstrap.md`
2. `docs/constellation-layer3-vars.example.yml`
3. `docs/constellation-auth-infra.md`
4. `docs/constellation-rbac-introduction.md`
5. `docs/constellation-rbac-one-page.md`
6. `docs/constellation-rbac-faq.md`
7. `docs/constellation-layer3-reference-app.md`

## What The Pack Already Defines

The pack already defines:

1. the required services
2. the install order
3. the variable schema
4. the core-init command contract
5. the readiness and acceptance checks
6. `k2hello` as the exported reference relying app in the layer-3 baseline

The pack does not yet implement environment-specific deployment execution.

That missing part belongs to infra because it depends on the real cluster wiring.

## Required Infra Work

Infra needs to do four things.

### 1. Replace example values

Use the current frogfish baseline first, then replace the remaining environment-specific values.

Replace placeholders in:

1. `infra/layer3-ansible/group_vars/frogfish.current.yml`
2. `infra/layer3-ansible/inventory/hosts.example.yml`

Create environment-specific versions with:

1. real Nomad manager target
2. current frogfish domains and image repos carried forward unless intentionally changing them
3. real image tags
4. real secret references or values
5. real deploy mode choice: `vault` or `nomad-var`

Important:

1. `frogfish.current.yml` is not generic sample data
2. it is the repo's current known frogfish baseline derived from deploy-script defaults and service docs
3. use it as the starting point unless the target environment is deliberately different
4. `k2db-api` bootstrap token is a control-plane unlock secret, not a normal runtime service secret; supply it from Vault or an equivalent infra-owned secret source only for init, recover, and config-set style actions

### How To Fill `frogfish.current.yml`

Use this rule when reading the file.

#### 1. Keep concrete repo-backed values unless intentionally changing them

These are already real baseline values, not placeholders.

Examples:

1. `public_domain`
2. `image_repo`
3. known current `image_tag`, `build_tag`, and `roll_id`
4. `system_db_name: k2_system`
5. `nomad_var_path: nomad/jobs/<job_name>`

Where they come from:

1. deploy-script defaults
2. current service documentation
3. `docs/stage2-cluster-reset-inventory.md`

#### 2. Fill blank secret values from the secret system, not by inventing literals in git

If a field is blank and has a Vault path next to it, the source of truth is the secret backend.

Examples:

1. `k2rbac-api.jwt_secret`
2. `k2rbac-api.k2db_api_key`
3. `k2login.rbac_api_key`
4. `k2mx-api.k2db_api_key`
5. `k2mx-api.bootstrap_token`
6. `k2mx-api.ui_session_secret`

#### 3. Fill blank generated values from environment bootstrap, not from repo defaults

These should be generated or chosen per environment.

Examples:

1. `k2hello.session_secret`
2. `constellation_layer3_core_init.rbac_first_admin.password`

#### 4. Fill blank release metadata from the live release record

If a service has a blank image tag, the repo is telling you it does not currently pin one verified live value.

Example:

1. `k2hello.image_tag`
2. `k2hello.build_tag`
3. `k2hello.roll_id`

Source of truth:

1. live cluster
2. release record
3. image registry

#### 5. Fill unresolved infra-owned paths from the actual cluster binding

Some paths are intentionally blank because the repo does not define one single blessed live value.

Example:

1. `k2db-api.vault_mongo_uri_path`
2. `k2db-api.vault_mongo_uri_cli_path`
3. `k2db-api.bootstrap_token_vault_path`

Source of truth:

1. the actual target cluster's Vault layout
2. infra-owned secret conventions

### 2. Implement service deployment wiring

The service contract is already defined, but infra must connect it to the live cluster.

Implement the actual deploy behavior behind the role using the existing Nomad model.

For each enabled service:

1. render the correct Nomad template from this repo
2. inject secrets through the environment's chosen mechanism
3. submit the job to Nomad
4. wait for service registration and readiness

Do not introduce a second deployment model.

The intended source templates remain the repo's current service job templates.

### 3. Implement core init execution

Infra must choose how to run the already-defined commands.

Any of these are acceptable if done safely and repeatably:

1. `nomad alloc exec`
2. a one-shot admin job
3. a dedicated bootstrap task container
4. another controlled execution wrapper already used by infra

The command contract itself should stay as documented.

Core init must cover:

1. `k2db-api-server init` or `recover`
2. `k2db-api-server config set`
3. `k2rbac-api-server bootstrap-admin --email ...`

Important distinction:

1. `init` and `bootstrap-admin` are one-time bootstrap actions
2. `recover` and `config set` are repeatable reconcile actions

Infra should preserve that distinction in the boot pipeline.

### 4. Enforce acceptance checks

Boot should not report success only because the jobs started.

Infra should fail the boot flow if any of these checks fail:

1. `k2db-api` passes `/ready`
2. `k2rbac-api` passes `/ready`
3. `k2login` passes `/health` and `/ready`
4. `k2hello` passes `/health` and `/ready`
5. wrong-password login fails quickly instead of hanging
6. successful login completes redirect and handoff through `k2hello`

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
10. deploy `k2hello` as the exported reference layer-3 app
11. optionally deploy `k2mx-api`
12. run readiness checks
13. run auth-path acceptance checks
14. mark the cluster application baseline ready

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

1. use `infra/layer3-ansible` as the integration pack
2. replace example vars with real environment values
3. wire the role to render and submit the existing Nomad jobs
4. run the documented core-init commands through the cluster's approved execution path
5. fail boot if readiness or auth-path checks fail