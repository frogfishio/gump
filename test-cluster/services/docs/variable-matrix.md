# Ansible Variable Matrix

This document maps the current Rust service deployment into infrastructure-facing variables.

Use it as the bridge between:

- the source Nomad templates in `services/nomad/templates/`
- the edge artifacts in `infra/edge/`
- your Ansible inventory, group vars, Vault secrets, and template rendering

All secret values below are placeholders or names only. Do not commit real values here.

## Shared Cluster Variables

| Variable | Purpose | Notes |
| --- | --- | --- |
| `cluster_nomad_namespace` | Nomad namespace | currently `default` |
| `cluster_nomad_datacenters` | Nomad datacenter list | current templates use `[*]` |
| `cluster_manager_host` | SSH target for operational actions | current app-side deploy scripts use `manager@159.223.37.147` |
| `cluster_consul_http_addr` | Consul HTTP endpoint | current live checks use `127.0.0.1:8500` on manager-side tooling |
| `cluster_consul_dns_addr` | Consul DNS endpoint | generic Caddy edge docs use `127.0.0.1:8600` |
| `cluster_vault_enabled` | Secret delivery mode toggle | app deploy scripts support Nomad vars or Vault |
| `cluster_docker_registry` | Container registry base | current images are published under `docker.io/frogfishio/*` |

## Public Domains

Live edge registrations currently show these public domains:

| Domain variable | Live value | Notes |
| --- | --- | --- |
| `public_domains.k2db` | `k2db.frogfish.io` | published by `k2db-api` |
| `public_domains.auth` | `auth.frogfish.io` | published by `k2rbac-api` edge registration |
| `public_domains.login` | `login.frogfish.io` | published by `k2login` edge registration |
| `public_domains.hello` | `hello.frogfish.io` | currently present in live edge registrations |
| `public_domains.hello_alt` | `replace-me` | optional alternate host; may default to `public_domains.hello` if you do not use a second hostname |

## Service Matrix

### `k2db-api`

| Field | Value |
| --- | --- |
| Job name | `k2db-api` |
| Image repo | `docker.io/frogfishio/k2db-api-server` |
| Runtime service | `k2db-api` |
| Edge service | `edge` |
| Runtime port | `3000` in container, dynamically exposed by Nomad |
| Public domain | `public_domains.k2db` |
| Nomad template | `services/nomad/templates/k2db-api.nomad.tpl` |
| Required runtime env | `K2DB_MONGO_URI` |
| Optional runtime env | `K2DB_SYSTEM_DB_NAME` |
| Nomad var keys | `K2DB_SYSTEM_DB_NAME` |
| Vault/secret inputs | Mongo URI secret source |
| Infra note | Control-plane root; rebuild first |

Suggested Ansible vars:

- `services.k2db.job_name`
- `services.k2db.image_repo`
- `services.k2db.image_tag`
- `services.k2db.domain`
- `services.k2db.system_db_name`
- `services.k2db.mongo_uri_secret_ref`
- `services.k2db.vault_role`

### `k2rbac-api`

| Field | Value |
| --- | --- |
| Job name | `k2rbac-api` |
| Image repo | `docker.io/frogfishio/k2rbac-api-server` |
| Runtime service | `k2rbac-api` |
| Edge service | `edge` |
| Runtime port | `4100` in container, dynamically exposed by Nomad |
| Public domain | `public_domains.auth` |
| Upstream dependency | `k2db-api` |
| Nomad template | `services/nomad/templates/k2rbac-api.nomad.tpl` |
| Required secrets | `RBAC_JWT_SECRET`, `RBAC_K2DB_API_KEY` |
| Optional secrets | `RBAC_K2MX_API_KEY` if mail-plane integration is enabled |
| Nomad var path | `nomad/jobs/k2rbac-api` |
| Nomad var keys | `RBAC_JWT_SECRET`, `RBAC_K2DB_API_KEY` |
| Special template var | `excluded_host` |

Suggested Ansible vars:

- `services.rbac.job_name`
- `services.rbac.image_repo`
- `services.rbac.image_tag`
- `services.rbac.auth_domain`
- `services.rbac.excluded_host`
- `services.rbac.jwt_secret_ref`
- `services.rbac.k2db_api_key_ref`
- `services.rbac.k2mx_api_key_ref`
- `services.rbac.k2db_key_database`
- `services.rbac.vault_role`

### `k2login`

| Field | Value |
| --- | --- |
| Job name | `k2login` |
| Image repo | `docker.io/frogfishio/k2login-server` |
| Runtime service | `k2login` |
| Edge service | `edge` |
| Runtime port | `4200` in container, dynamically exposed by Nomad |
| Public domain | `public_domains.login` |
| Upstream dependency | `k2rbac-api` |
| Nomad template | `services/nomad/templates/k2login.nomad.tpl` |
| Required secret | `K2LOGIN_RBAC_API_KEY` |
| Nomad var path | `nomad/jobs/k2login` |
| Nomad var keys | `K2LOGIN_RBAC_API_KEY` |
| Required non-secret vars | `hello_domain`, `hello_alt_domain`, `auth_domain`, `login_domain` |

Suggested Ansible vars:

- `services.k2login.job_name`
- `services.k2login.image_repo`
- `services.k2login.image_tag`
- `services.k2login.login_domain`
- `services.k2login.auth_domain`
- `services.k2login.hello_domain`
- `services.k2login.hello_alt_domain`
- `services.k2login.rbac_api_key_ref`
- `services.k2login.vault_role`

### `k2hello`

| Field | Value |
| --- | --- |
| Job name | `k2hello` |
| Image repo | `docker.io/frogfishio/k2hello` |
| Runtime service | `k2hello` |
| Edge service | `edge` |
| Runtime port | `4300` in container, dynamically exposed by Nomad |
| Public domain | `public_domains.hello` |
| Upstream dependency | `k2login` |
| Nomad template | `services/nomad/templates/k2hello.nomad.tpl` |
| Required secret | `HELLO_SESSION_SECRET` |
| Nomad var path | `nomad/jobs/k2hello` |
| Nomad var keys | `HELLO_SESSION_SECRET` |
| Required non-secret vars | `hello_domain`, `login_domain` |
| Stage-3 derived local secrets | generated `session_secret` |

Suggested Ansible vars:

- `services.k2hello.job_name`
- `services.k2hello.image_repo`
- `services.k2hello.image_tag`
- `services.k2hello.hello_domain`
- `services.k2hello.login_domain`
- `services.k2hello.session_secret_ref`
- `services.k2hello.vault_role`

### `k2mx-api`

| Field | Value |
| --- | --- |
| Job name | `k2mx-api` |
| Image repo | `docker.io/frogfishio/k2mx-api-server` |
| Runtime services | `k2mx-api`, `k2mx-admin-api`, `k2mx-ui` |
| Edge service | none in current template |
| Runtime ports | fixed host ports `3001`, `3002`, `4181` |
| Upstream dependency | `k2db-api` |
| Nomad template | `services/nomad/templates/k2mx-api.nomad.tpl` |
| Required secrets | `K2MX_K2DB_API_KEY`, `K2MX_BOOTSTRAP_TOKEN`, `K2MX_UI_SESSION_SECRET` |
| Nomad var path | `nomad/jobs/k2mx-api` |
| Nomad var keys | `K2MX_K2DB_API_KEY`, `K2MX_BOOTSTRAP_TOKEN`, `K2MX_UI_SESSION_SECRET` |
| Special infra requirement | `network_mode = host` |
| Special data requirement | active runtime key for `k2mx` with read/write/search/count |
| Stage-3 derived local secrets | `k2mx-api.k2db_api_key`, `k2mx-api.bootstrap_api_key`, generated `bootstrap_token`, generated `ui_session_secret` |

Suggested Ansible vars:

- `services.k2mx.job_name`
- `services.k2mx.image_repo`
- `services.k2mx.image_tag`
- `services.k2mx.runtime_port`
- `services.k2mx.admin_port`
- `services.k2mx.ui_port`
- `services.k2mx.k2db_api_key_ref`
- `services.k2mx.bootstrap_token_ref`
- `services.k2mx.ui_session_secret_ref`
- `services.k2mx.k2db_key_database`
- `services.k2mx.k2db_key_permissions`
- `services.k2mx.bootstrap_api_key`
- `services.k2mx.bootstrap_key_id`
- `services.k2mx.bootstrap_scope`
- `services.k2mx.bootstrap_mailbox_id`
- `services.k2mx.bootstrap_provider_id`
- `services.k2mx.bootstrap_permissions`
- `services.k2mx.vault_role`

## Reverse Proxy Variables

### Generic edge layer

These inputs belong on the infra side, not the app side:

- `edge.generic_caddy_enabled`
- `edge.caddy_admin_email`
- `edge.consul_dns_addr`
- `edge.consul_http_addr`
- `edge.prepared_query_template_path`
- `edge.ask_helper_script_path`
- `edge.ask_helper_unit_name`

### Site-specific Caddy inputs

From the bundled repo assets, infra will likely need vars for:

- `edge.api_ramblerbooks_domain`
- `edge.api_ramblerbooks_upstream_service`
- `edge.mx_hostnames`
- `edge.default_fallback_service`

## Secret Backend Mapping

If infra keeps the current Nomad-var pattern, model these paths:

| Path | Keys |
| --- | --- |
| `nomad/jobs/k2rbac-api` | `RBAC_JWT_SECRET`, `RBAC_K2DB_API_KEY` |
| `nomad/jobs/k2login` | `K2LOGIN_RBAC_API_KEY` |
| `nomad/jobs/k2hello` | `HELLO_SESSION_SECRET` |
| `nomad/jobs/k2mx-api` | `K2MX_BOOTSTRAP_TOKEN`, `K2MX_K2DB_API_KEY`, `K2MX_UI_SESSION_SECRET` |

If infra migrates to Vault-backed rendering, keep the same logical secret names and map them to Vault refs in Ansible.

## Deployment Order For Ansible

1. Provision edge prerequisites and Consul-side generic routing assets.
2. Render secret backend state.
3. Deploy `k2db-api`.
4. Deploy `k2mx-api`.
5. Deploy `k2rbac-api`.
6. Deploy `k2login`.
7. Deploy `k2hello`.
8. Validate public ingress and internal readiness.
