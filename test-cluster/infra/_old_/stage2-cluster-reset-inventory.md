# Stage 2 Cluster Reset Inventory

Last verified: 2026-03-22

This document captures the live cluster shape after Stage 1 parity work for the Rust services.
It is intended to be the baseline for Stage 2 extraction, infra cleanup, and clean redeploy.

## Live Jobs

| Job | Version | Deployment | Node | Image tag | Runtime registration |
| --- | --- | --- | --- | --- | --- |
| `k2db-api` | `6` | `d6eabeec` successful | `frogfish03` | `20260322061156` | `104.248.153.136:28771` |
| `k2rbac-api` | `10` | `fccd8274` successful | `frogfish03` | `20260322135229` | `104.248.153.136:22276` |
| `k2login` | `14` | `7ff7b289` successful | `frogfish03` | `20260322143907` | `104.248.153.136:21316` |
| `k2mx-api` | `12` | `be45c4a6` successful | `frogfish02` | `20260322153737` | `165.22.58.15:3001` |

## Live Consul Registrations

Passing registrations observed during Stage 2 extraction:

| Service | Node | Address | Port | Notes |
| --- | --- | --- | --- | --- |
| `k2db-api` | `frogfish03` | `104.248.153.136` | `28771` | runtime plane |
| `k2rbac-api` | `frogfish03` | `104.248.153.136` | `22276` | runtime plane |
| `k2login` | `frogfish03` | `104.248.153.136` | `21316` | runtime plane |
| `k2mx-api` | `frogfish02` | `165.22.58.15` | `3001` | runtime plane |
| `k2mx-admin-api` | `frogfish02` | `165.22.58.15` | `3002` | private admin plane |
| `k2mx-ui` | `frogfish02` | `165.22.58.15` | `4181` | private UI plane |

## Reverse Proxy And Edge Assets

This is a key Stage 2 asset. Public ingress is not only an app concern; it depends on infrastructure-owned edge behavior plus service-side registration metadata.

### Intended model

The intended production model is documented in the Rust `k2db-api` README:

- apps publish public-domain metadata in Consul service registration metadata
- infrastructure-owned Caddy consumes that metadata through a generic edge configuration
- public routing should remain generic and disposable rather than handwritten per app after every rebuild

Reference docs and assets already in the repo:

- `docs/caddy-edge-generic-prepared-query.Caddyfile`
- `docs/consul-edge-query-template.json`
- `docs/caddy-consul-ask.py`
- `docs/caddy-consul-ask.service`
- `ext/api.ramblerbooks.com/Caddyfile`
- `ext/api.ramblerbooks.com/edge-caddy.tpl`
- `ext/MX/Caddyfile`

### Live edge registrations observed

The live cluster currently also contains passing `edge` service registrations with domain tags.

Observed examples:

| Service | Node | Address | Port | Domain metadata |
| --- | --- | --- | --- | --- |
| `edge` | `frogfish02` | `165.22.58.15` | `21464` | `hello.frogfish.io` |
| `edge` | `frogfish03` | `104.248.153.136` | `22276` | `auth.frogfish.io` |

This means Stage 2 must preserve both:

1. the infrastructure-owned Caddy/generic-edge configuration assets
2. the service-side domain metadata and any edge-plane Consul registrations produced by app jobs

### Stage 2 implication

Before any clean reset, extract and preserve:

1. current Caddyfile or Caddy template sources in use by the cluster
2. any Consul prepared queries or Caddy `ask` helper configuration used for generic edge routing
3. all service registrations carrying `plane:edge` and `domain:*` metadata
4. any app job templates that currently emit `edge` registrations as part of their public ingress contract

## Secret Delivery Shape

Current Stage 1 cluster state uses Nomad variables for the dependent Rust services.
This section records the paths and key names only, not the secret values.

### `nomad/jobs/k2rbac-api`

- `RBAC_JWT_SECRET`
- `RBAC_K2DB_API_KEY`

### `nomad/jobs/k2login`

- `K2LOGIN_RBAC_API_KEY`

### `nomad/jobs/k2mx-api`

- `K2MX_BOOTSTRAP_TOKEN`
- `K2MX_K2DB_API_KEY`
- `K2MX_UI_SESSION_SECRET`

## Control Plane Runtime Shape

The live `k2db-api` task environment currently exposes:

- `K2DB_MONGO_URI`
- `K2DB_SYSTEM_DB_NAME=k2_system`

Notably, the steady-state `k2db-api` task does not expose `K2DB_BOOTSTRAP_TOKEN` in its runtime environment.
That remains correct for steady-state operation, but it means any Stage 2 key rotation or control-plane action must deliberately supply the bootstrap token.

## Verified Infra Quirks

### 1. `k2mx` requires host networking

`k2mx-api` is currently the only Stage 1 service that requires:

- `network_mode = "host"`
- static host ports `3001`, `3002`, and `4181`

Reason:

- default container networking caused same-node east-west failures when `k2mx` talked to `k2db-api`
- dynamic host-port registration then broke Nomad and Consul health checks once `k2mx` moved to host networking

The working state is therefore:

- host networking
- fixed host ports
- Consul registrations on `3001`, `3002`, `4181`

### 2. `k2mx` depends on an active `mx_dev` runtime key

The live `k2mx` rollout only became healthy after rotating a stale inactive `K2MX_K2DB_API_KEY`.

The `k2mx` runtime key must be scoped to database `mx_dev` and include:

- `collections.read`
- `collections.write`
- `collections.search`
- `collections.count`

### 3. Live `k2db-api` binary still exposes older flag-based control-plane CLI

The currently deployed `k2db-api` binary still exposes `keys` management through the older flag-capable command shape.
That matters for Stage 2 because live emergency rotation may still need that command form until `k2db-api` itself is redeployed from the latest source.

## Stage 2 Reset Preconditions

Before a destructive cleanup or a fresh redeploy, preserve the following facts:

1. `k2db-api` is the control-plane root and must come up first.
2. `k2rbac-api` depends on `k2db-api` and its own JWT secret.
3. `k2login` depends on `k2rbac-api` and its runtime API key.
4. `k2mx-api` depends on `k2db-api`, its bootstrap token, UI session secret, and an active `mx_dev` runtime key.
5. `k2mx-api` must keep host networking and fixed ports unless the underlying cluster networking model changes.
6. reverse proxy and edge configuration is a first-class asset and must be extracted before destructive cleanup.

## Recommended Stage 2 Order

1. Snapshot current Nomad variable paths and key names.
2. Snapshot current live job versions, image tags, and placements.
3. Snapshot reverse proxy assets:
   - current Caddyfiles/templates
   - any prepared-query config
   - current `edge` registrations and domain metadata
4. Preserve the current active `k2mx` runtime key label and permissions model.
5. Tear down dependent jobs in reverse order:
   - `k2mx-api`
   - `k2login`
   - `k2rbac-api`
   - `k2db-api` only if the control plane itself is being rebuilt
6. Recreate and verify in forward dependency order:
   - `k2db-api`
   - `k2rbac-api`
   - `k2login`
   - `k2mx-api`
7. Restore and validate edge/public ingress.
8. Re-run service validation:
   - Consul passing checks
   - endpoint health/readiness
   - key-dependent flows such as `k2mx` queue search
   - public domain routing through the edge layer

## Post-Reset Validation Checklist

- `k2db-api` passes `/health` and `/ready`
- `k2rbac-api` passes `/health` and `/ready`
- `k2login` passes `/health` and `/ready`
- `k2mx-api`, `k2mx-admin-api`, and `k2mx-ui` all pass Consul health checks
- `k2mx` queue search against `k2db-api` returns `200 OK`
- Nomad variable paths exist with the expected key names
