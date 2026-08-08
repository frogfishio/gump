# Infra Handoff Bundle

This directory is the shareable Stage 2 infrastructure snapshot for the current Rust cluster setup.

It now packages the live-verified Stage 2 inventory and the generic edge assets that still belong to the substrate side.
The owned Stage 3 app deploy run has moved to `services/`.

## Contents

### Inventory

- `stage2-cluster-reset-inventory.md`
- `../services/docs/variable-matrix.md`
- `../services/group_vars/all.example.yml`

This is the current live-cluster baseline.
It includes:

- deployed job versions and placements
- passing Consul registrations
- Nomad variable paths and key names
- `k2mx` host-networking requirement
- reverse proxy and edge observations
- reset and redeploy order

The Ansible-specific artifacts provide:

- a service-by-service variable matrix
- a starter `group_vars` structure with placeholder refs for secrets and domains

### Edge Assets

- `edge/caddy-edge-generic-prepared-query.Caddyfile`
- `edge/consul-edge-query-template.json`
- `edge/caddy-consul-ask.py`
- `edge/caddy-consul-ask.service`
- `edge/api.ramblerbooks.com/Caddyfile`
- `edge/api.ramblerbooks.com/edge-caddy.tpl`
- `edge/MX/Caddyfile`

These files capture both:

1. the intended generic Consul-driven Caddy model
2. the repo-owned site-specific Caddy inputs currently present in this repository

### Stage 3 App Deploy

- `../services/site.yml`
- `../services/nomad/templates/k2db-api.nomad.tpl`
- `../services/nomad/templates/k2rbac-api.nomad.tpl`
- `../services/nomad/templates/k2login.nomad.tpl`
- `../services/nomad/templates/k2mx-api.nomad.tpl`

These are now owned directly from the top-level `services/` run.

## Important Live Notes

### Secret material is not bundled here

This folder intentionally includes secret names and paths, not secret values.

Current Nomad variable paths and key names are documented in `stage2-cluster-reset-inventory.md`.
Infra should model those values in Ansible Vault, Vault, Nomad variables, or another secret backend as appropriate.

### `k2mx` is special

`k2mx-api` currently requires:

- host networking
- static host ports `3001`, `3002`, and `4181`

If infra rewrites the Nomad deployment in Ansible, preserve that behavior unless the underlying cluster networking model changes.

### Reverse proxy is a first-class asset

Do not treat edge routing as an afterthought during rebuild.

The live environment currently depends on both:

- infrastructure-owned Caddy or generic edge behavior
- service-side `edge` registrations carrying domain metadata

That means a clean reset must preserve or recreate:

- Caddy configuration
- any Consul prepared-query setup
- any `ask` helper service/unit wiring
- domain-bearing Consul registrations

## Recommended Infra Consumption Order

1. Read `stage2-cluster-reset-inventory.md` first.
2. Review `edge/` and decide which parts become Ansible-managed host config versus generated templates.
3. Review `../services/docs/variable-matrix.md` and `../services/group_vars/all.example.yml` to map repo inputs into the Stage 3 variables.
4. Review `../services/` for the owned app deploy run and `edge/` for the still-infra-owned edge assets.
5. Recreate secret delivery for the documented Nomad variable paths or replace it with Vault-backed rendering.
6. Preserve the live `k2mx` networking and key-scope requirements during rollout.
