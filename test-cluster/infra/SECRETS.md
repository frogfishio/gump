# Layer 3 Secrets Guide

This document explains the secrets and sensitive values used by the layer-3 constellation stack in plain language.

It exists because names like `RBAC_K2DB_API_KEY` or `K2MX_BOOTSTRAP_TOKEN` are not self-explanatory if you are trying to deploy the system rather than reverse-engineer it.

## How To Read This

For each value below, the guide tells you:

1. what it is
2. who uses it
3. who should create or source it
4. where it should live
5. whether it belongs in steady-state runtime or only bootstrap flows

## `K2DB_MONGO_URI`

What it is:

1. the Mongo connection string for the `k2db-api` service
2. this is the database connection used by the `k2db` control plane and runtime

Who uses it:

1. `k2db-api`
2. `k2db-api` control-plane commands such as `init`, `recover`, `config set`, and key management

Who should source it:

1. infra, from the real Mongo deployment for the target environment

Where it should live:

1. normally in Vault or an equivalent infra secret system
2. optionally in Nomad vars if the environment uses that mode instead of Vault

Runtime or bootstrap:

1. both
2. it is required for normal `k2db-api` runtime and for control-plane commands

## `RBAC_JWT_SECRET`

What it is:

1. the signing secret used by `k2rbac-api` for JWT issuance and validation
2. this is one of the core auth secrets in the platform

Who uses it:

1. `k2rbac-api`

Who should create or source it:

1. infra or security tooling
2. generated once per environment and then stored securely

Where it should live:

1. Vault path currently documented as `kv/data/k2/rbac-api`, field `jwt_secret`

Runtime or bootstrap:

1. runtime secret

## `RBAC_K2DB_API_KEY`

What it is:

1. the runtime API key that allows `k2rbac-api` to call `k2db-api`
2. this is not a browser credential
3. this is not the bootstrap token

Who uses it:

1. `k2rbac-api`

Who should create or source it:

1. originally created in the `k2db` control plane
2. then stored by infra in Vault or Nomad vars for RBAC runtime use

Where it should live:

1. Vault path currently documented as `kv/data/k2/rbac-api`, field `api_key`

Runtime or bootstrap:

1. runtime secret

## `RBAC_K2MX_API_KEY`

What it is:

1. optional runtime API key that allows `k2rbac-api` to call `k2mx-api`
2. used only if RBAC mail-plane or notification flows are wired in that environment

Who uses it:

1. `k2rbac-api`

Who should create or source it:

1. created in the `k2db` control plane for `k2mx`
2. stored by infra only in environments that wire RBAC to k2mx

Where it should live:

1. Vault path currently documented as `kv/data/k2/rbac-api-mail`, field `api_key`

Runtime or bootstrap:

1. runtime secret
2. optional

## `K2LOGIN_RBAC_API_KEY`

What it is:

1. the runtime API key that allows `k2login` to call `k2rbac-api`
2. this is how the browser-facing login front door talks to the RBAC authority

Who uses it:

1. `k2login`

Who should create or source it:

1. created as a runtime key for the login service
2. stored by infra for `k2login`

Where it should live:

1. Vault path currently documented as `kv/data/k2/login`, field `api_key`

Runtime or bootstrap:

1. runtime secret

## `K2MX_K2DB_API_KEY`

What it is:

1. the runtime API key that allows `k2mx-api` to call `k2db-api`

Who uses it:

1. `k2mx-api`

Who should create or source it:

1. created in the `k2db` control plane for the `k2mx` service
2. stored by infra for the `k2mx` runtime

Where it should live:

1. Vault path currently documented as `kv/data/k2/mx`, field `api_key`

Runtime or bootstrap:

1. runtime secret

## `K2MX_BOOTSTRAP_TOKEN`

What it is:

1. the bootstrap/control token used by `k2mx-api`
2. this is the secret that lets k2mx perform its own bootstrap-protected operations

Who uses it:

1. `k2mx-api`

Who should create or source it:

1. infra or control-plane setup, depending on environment conventions
2. it must be treated as a privileged secret, not a casual app setting

Where it should live:

1. Vault path currently documented as `kv/data/k2/mx`, field `bootstrap_token`

Runtime or bootstrap:

1. runtime secret for `k2mx-api`

## `K2MX_UI_SESSION_SECRET`

What it is:

1. the cookie/session signing secret for the `k2mx` operator UI

Who uses it:

1. `k2mx-api` UI/admin surfaces

Who should create or source it:

1. infra generates it per environment

Where it should live:

1. Vault path currently documented as `kv/data/k2/mx-ui`, field `session_secret`

Runtime or bootstrap:

1. runtime secret

## Important Non-Equivalent Values

These names are easy to confuse.

They are not the same thing.

### `K2DB_BOOTSTRAP_TOKEN` vs `RBAC_K2DB_API_KEY`

`K2DB_BOOTSTRAP_TOKEN`:

1. privileged control-plane unlock token
2. used for `init`, `recover`, `config set`, and key management
3. should not be injected into the normal steady-state `k2db-api` runtime task

`RBAC_K2DB_API_KEY`:

1. normal runtime service-to-service credential
2. used by RBAC to call `k2db-api`

### `K2LOGIN_RBAC_API_KEY` vs browser auth cookies

`K2LOGIN_RBAC_API_KEY`:

1. server-side service credential used by `k2login`
2. never sent to the browser

Browser cookies:

1. belong to the login and relying-app session flows
2. are a different security layer entirely

## Practical Infra Rule

When you see a value in the layer-3 vars file, ask this in order:

1. is this a current repo-backed constant such as a domain or image repo?
2. is this a runtime secret that should already live in Vault?
3. is this a privileged bootstrap secret?
4. is this a generated per-environment session secret?
5. is this release metadata such as an image tag?

If you answer that question first, the field becomes much easier to wire correctly.