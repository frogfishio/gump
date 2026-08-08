# Layer 3 Constellation Ansible Pack

This directory is the handoff pack for the infra team.

The operational handover document is:

1. `infra/layer3-ansible/HANDOVER.md`

The plain-English secrets guide is:

1. `infra/layer3-ansible/SECRETS.md`

The architecture introduction for new readers is:

1. `docs/constellation-rbac-introduction.md`
2. `docs/constellation-rbac-one-page.md`
3. `docs/constellation-rbac-faq.md`

It is meant to plug into an existing boot pipeline where:

1. Terraform already created the substrate
2. lower-layer Ansible already lifted the platform
3. Nomad, Consul, Vault, Docker, DNS, and edge routing already exist

This pack covers only layer 3:

1. define the app-layer install contract
2. define the deploy order for the core constellation services
3. define the core-init command contract
4. define the acceptance checks that must pass before frontend apps can rely on the base constellation

## What This Pack Contains

1. `site.yml`: entrypoint playbook scaffold
2. `inventory/hosts.example.yml`: minimal example inventory shape
3. `group_vars/frogfish.current.yml`: current frogfish baseline values derived from the repo's deploy defaults and service docs
4. `group_vars/constellation.example.yml`: generic layer-3 schema example
5. `roles/constellation_layer3/`: role scaffold for validation, deploy planning, core init, and acceptance

## What This Pack Does Not Own

1. Terraform substrate creation
2. platform package installation or cluster bootstrap
3. Docker image build and push
4. infra-specific secret storage policy

Those belong to the infra layer outside this repo.

## Integration Intent

The infra team should plug this pack into their system boot flow after the cluster is ready.

Recommended sequence:

1. lower-layer infra boot completes
2. this pack receives environment-specific vars
3. layer-3 app deploy wiring is implemented against the existing Nomad and Vault setup
4. core init runs
5. acceptance checks gate completion

## Handoff Inputs

The canonical contract and service details live in:

1. `docs/constellation-layer3-bootstrap.md`
2. `docs/constellation-layer3-vars.example.yml`

This Ansible pack mirrors those definitions in a form the infra team can wire into their existing automation.

## Current State

This pack is intentionally a scaffold, not a second hidden deployment system.

It already gives the infra team:

1. the current frogfish baseline values for domains and image repos
2. the variable schema
3. the install order
4. the explicit core-init commands
5. the acceptance contract

The remaining infra-side implementation work is to connect these definitions to the real environment-specific Nomad, Vault, and network plumbing.

## Recommended Next Infra Steps

1. replace the example inventory and vars with environment-specific values
2. implement the service deploy execution in the role where marked
3. wire the core-init commands to the preferred runtime execution path
4. make acceptance checks fail the boot flow on any broken auth-path condition