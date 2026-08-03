# Gump Application Manifest

> Status: working draft 0.1  
> File name: `gump.toml`  
> Purpose: define the developer contract shared by local execution and deployment

## 1. Role of the manifest

`gump.toml` is the committed, declarative description of a Gump application. It tells Gump:

- Which application files become release material
- How those files are prepared
- How the application is started and stopped
- Which runtime variables must be supplied
- How readiness and health are determined
- Which resources and host capabilities the workload expects
- What placement, rollout, and Kismet publication behavior is desired by default

The manifest describes value requirements and policies; it does not contain protected runtime values. Environment-variable and secret plaintext is resolved from external sources into Gump's memory during `run` or `deploy`.

The same manifest drives local execution and cluster execution. Differences must be explicit and inspectable.

## 2. Manifest invariants

1. The manifest is safe to commit to source control.
2. Plaintext runtime values MUST NOT appear in `gump.toml`.
3. Every runtime value has a declared logical name, source, requirement, and injection contract.
4. All runtime values receive the same encrypted-at-rest treatment; marking a value `secret` additionally controls display, inspection, and application-facing handling.
5. File selection is deterministic for a fixed workspace state and manifest.
6. Paths are relative to the manifest directory unless a field explicitly says otherwise.
7. Packaging cannot read outside the declared workspace roots.
8. Commands are argument arrays, not implicit shell strings.
9. Unknown keys and unsupported required capabilities are errors.
10. Defaults are versioned as part of the manifest schema; upgrades never silently reinterpret an existing manifest.
11. Local overrides cannot alter the release that `gump deploy` creates unless the deploy command explicitly selects them as release inputs.
12. `gump deploy` can show the complete public file list, runtime-variable names, and normalized manifest without exposing protected values.

## 3. Three classes of configuration

Every manifest field belongs to one of three classes.

### 3.1 Release contract

Release-contract fields are stamped into the Capsule. Changing one creates a different release.

Examples:

- Included application files
- Archive properties
- Execution driver
- Entrypoint, arguments, and working directory
- Runtime-variable schema and injection targets
- Port contract
- Health-check definitions
- Required platform capabilities

### 3.2 Deployment intent

Deployment-intent fields become defaults in the signed deployment declaration. They can later be changed by creating a new declaration generation that still references the same Capsule.

Examples:

- Replica count
- Resource request and limit policy
- Placement constraints
- Rollout and disruption policy
- Restart policy
- Kismet publication intent

The final declaration is always inspectable. A default taken from the manifest is indistinguishable in meaning from the same value supplied through an authorized deployment policy, but its provenance is recorded.

### 3.3 Local behavior

Local fields affect developer execution only and never enter a production release unless explicitly promoted into a release-contract field.

Examples:

- Source-tree watch paths
- Local port preference
- Local-only runtime-variable source mappings
- Whether local Kismet integration is used
- Developer convenience commands

## 4. Proposed top-level shape

```toml
schema = "gump/1"

[app]
id = "accounts-service"
description = "Customer account API"

[package]
root = "."
include = ["bin/accounts-server", "assets/**", "migrations/**"]
exclude = ["**/*.tmp"]
format = "tar+zstd"

[prepare]
command = ["cargo", "build", "--release", "--locked"]
outputs = ["target/release/accounts-server"]

[runtime]
driver = "native"
command = ["./bin/accounts-server"]
workdir = "."
stop_signal = "TERM"
stop_timeout = "30s"

[runtime.ports.http]
address = "127.0.0.1"
value = "auto"
inject = "env:PORT"

[runtime.variables.LOG_LEVEL]
source = "env:LOG_LEVEL"
required = true
classification = "internal"
inject = "env"

[runtime.variables.DATABASE_URL]
source = "env:PROD_DATABASE_URL"
required = true
classification = "secret"
inject = "env"

[health.readiness]
type = "http"
port = "http"
path = "/ready"
interval = "5s"
timeout = "2s"

[health.liveness]
type = "http"
port = "http"
path = "/health"
interval = "10s"
timeout = "2s"
failures = 3

[resources]
cpu_request = "250m"
cpu_limit = "2"
memory_request = "256MiB"
memory_limit = "1GiB"

[deploy]
replicas = 3

[deploy.rollout]
strategy = "rolling"
max_unavailable = 0
max_surge = 1

[deploy.placement]
spread = ["node", "zone"]

[publish]
service = "accounts-service"
port = "http"
domain = "accounts.example.com"

[local]
watch = ["src/**", "assets/**"]

[local.ports]
http = 8080

[local.variables.DATABASE_URL]
source = "env:LOCAL_DATABASE_URL"
```

This example demonstrates the shape, not a frozen schema. Every accepted field still requires precise normalization and validation rules.

## 5. Application identity

```toml
[app]
id = "accounts-service"
description = "Customer account API"
```

`app.id` is a stable human-facing identity within a cluster namespace. It is not the release identity and does not change when code or configuration changes.

The normalized application identity is included in:

- Capsule authenticated associated data
- Release signatures
- Deployment declarations
- Instance identities
- Log and event context
- Kismet publication requests

Renaming an application is an explicit migration, not an ordinary deployment.

Whether the ultimate identity is the human name alone or a generated immutable ID plus name remains an architectural decision.

## 6. File selection and preparation

### 6.1 Workspace root

`package.root` defines the only filesystem tree packaging may traverse. It is resolved relative to the directory containing `gump.toml`.

The resolved root MUST NOT:

- Escape through `..`
- Escape through symlinks
- Change during traversal without detection
- Include unsupported files without an explicit policy

### 6.2 Include and exclude rules

`package.include` is an allowlist. At least one include rule is required for deployment unless the selected execution driver defines an equivalent immutable artifact input.

`package.exclude` removes matches from the allowlist. Exclusion never grants access outside `package.root`.

Rules use one documented, platform-independent glob syntax. Matching is performed against normalized slash-separated relative paths. Order, case sensitivity, hidden-file treatment, and negation behavior are fixed by the Gump schema rather than inherited from the host shell.

Gump always excludes its control files, machine-local state, and version-control internals from the application archive. This includes `gump.toml`, `gump.local.toml`, `.gumpignore`, `.gump/`, and `.git/` unless a future schema gives a control file an explicit packaged representation. Potential secret-bearing files such as `.env`, private keys, credentials, and editor backups are denied by default. Including one requires a conspicuous explicit override and still produces a packaging warning or policy failure.

Gump cannot prove that arbitrary source code or binary data contains no embedded credential. It applies structural exclusions and may run pluggable secret scanners, but the security boundary ultimately distinguishes files intentionally declared as public application material from values declared as protected runtime material. Packaging output makes that distinction visible before deployment.

Before sealing, Gump can emit a manifest of selected paths, types, modes, sizes, and digests. It never prints file contents by default.

### 6.3 Filesystem normalization

The package builder establishes deterministic behavior for:

- Lexical path ordering
- File modes and executable bits
- User and group ownership
- Modification times
- Empty directories
- Symbolic and hard links
- Sparse files
- Extended attributes
- Platform-specific metadata

Host ownership and timestamps are not preserved unless the schema explicitly requires them. Device nodes, sockets, FIFOs, and paths that differ only under ambiguous Unicode or case normalization are rejected by default.

### 6.4 Preparation command

Preparation is an optional developer-side command used to produce package inputs:

```toml
[prepare]
command = ["cargo", "build", "--release", "--locked"]
outputs = ["target/release/accounts-server"]
```

Gump executes the command directly without an implicit shell. A shell can be selected explicitly as an interpreter when genuinely required, making shell semantics visible in the manifest.

Preparation output is not trusted merely because the command exits successfully. Declared output paths must exist, must remain inside the workspace, and are hashed during capture. The preparation command and relevant provenance are recorded, but Gump does not claim that arbitrary builds are reproducible.

The relationship between `prepare.outputs` and `package.include` must be made unambiguous before the schema is frozen. The likely rule is that preparation produces files while package rules alone determine what enters the Capsule.

## 7. Runtime contract

### 7.1 Driver

```toml
[runtime]
driver = "native" # native | script | oci
```

The driver selects preparation and execution semantics; it does not change Capsule's role as the outer framing.

- `native` executes a packaged binary.
- `script` executes packaged source through an explicitly declared interpreter.
- `oci` supplies an OCI image or bundle to the node's OCI execution driver.

### 7.2 Command and arguments

```toml
command = ["./bin/accounts-server", "serve"]
workdir = "."
```

The first command entry resolves inside the release root unless the driver explicitly permits a trusted host interpreter. No shell expansion, environment substitution, globbing, pipelines, or command substitution occurs.

If an OCI artifact already declares an entrypoint, the manifest must say whether Gump inherits, replaces, or appends to it. Ambiguous composition is rejected.

### 7.3 Shutdown

```toml
stop_signal = "TERM"
stop_timeout = "30s"
```

The runtime contract defines graceful signal, drain interaction, grace period, and eventual forced termination. Local execution uses the same sequence when interrupted.

## 8. Runtime variables

### 8.1 Value declarations

A variable declaration describes a value without containing its plaintext:

```toml
[runtime.variables.DATABASE_URL]
source = "env:PROD_DATABASE_URL"
required = true
classification = "secret"
encoding = "utf8"
max_bytes = "64KiB"
inject = "env"
```

The table key, `DATABASE_URL`, is the logical name delivered to the application. `source` identifies where the local Gump process obtains the value. Source references are packaging instructions; resolved values are protected runtime material.

Source references are not stamped into the Capsule's public runtime-variable schema. They describe the developer-side lookup environment and are unnecessary to execute the release. Gump may record a non-sensitive source-kind provenance such as `environment` or `prompt`, but not `PROD_DATABASE_URL`, a keychain path, or another machine-specific locator unless explicitly requested for an external audit record.

The public schema for each logical value contains only execution-relevant metadata:

- Logical name
- Required or optional presence
- `utf8` or opaque `bytes` encoding
- Maximum accepted byte length
- Classification
- Injection target

Environment injection requires valid non-NUL text representable by the target Unix process environment. Opaque bytes require memory-backed file or descriptor injection. Missing and present-but-empty are distinct states.

### 8.2 Sources

The initial source model should remain deliberately small:

- `env:NAME` reads the invoking process environment.
- `prompt` reads an interactive hidden prompt.
- `stdin` or an inherited file descriptor supports automation without command-line exposure.
- A credential-source connector may integrate an operating-system keychain or another secret provider.

Source connectors return bytes into protected process memory. Gump does not execute arbitrary source commands by default. Secret values MUST NOT be supplied directly as command-line arguments because shell history and process listings make that unsafe.

Noninteractive deployment fails with a complete list of unresolved required variable names. It never prompts unexpectedly in CI.

### 8.3 Classification

All resolved runtime values are encrypted into the protected Capsule segment. Classification controls secondary behavior:

- `secret`: never displayed; aggressively excluded from diagnostics; eligible for memory-backed file injection.
- `internal`: hidden by default but may expose metadata such as byte length under policy.

A future `public` class should be introduced only if there is a compelling need to place plaintext configuration in public release material. It is absent from the initial contract to preserve the rule that runtime-variable values never persist in plaintext.

### 8.4 Injection

Supported injection targets are:

- `env`: add the value to the child environment under its logical name.
- `file`: expose it through an anonymous memory-backed descriptor and inject a reference understood by the application.

File injection requires an explicit application-visible path or descriptor contract. Gump does not materialize the value as an ordinary release file.

### 8.5 Local source overrides

Local execution often uses different credentials without changing the application-facing variable name:

```toml
[local.variables.DATABASE_URL]
source = "env:LOCAL_DATABASE_URL"
```

Only the source mapping changes. The logical variable name, requirement, classification, and injection method remain those of `runtime.variables.DATABASE_URL`.

Local overrides MUST NOT contain literal values. They may be committed if their source references are harmless. Machine-specific mappings may live in a separate ignored file, provisionally `gump.local.toml`, whose permissible fields are strictly limited to the `[local]` namespace.

### 8.6 Resolution lifecycle

For `gump run`:

1. Resolve required local source mappings into memory.
2. Construct the child injection contract.
3. Start the application.
4. Zeroize Gump-held copies when no longer needed.

For `gump deploy`:

1. Resolve all required deployment source mappings into memory.
2. Validate types and size bounds without logging values.
3. Canonically serialize the runtime-variable map.
4. Encrypt it into the protected Gump payload segment.
5. Zeroize plaintext and data-encryption-key copies after sealing.

A change to any resolved runtime value produces a different protected segment and therefore a new Capsule, even when application files are unchanged.

## 9. Ports and local networking

Named ports decouple health and publication policy from a particular allocated number:

```toml
[runtime.ports.http]
address = "127.0.0.1"
value = "auto"
inject = "env:PORT"
```

The cluster agent allocates an available loopback port and injects it into `PORT`. Local Gump may honor `local.ports.http` for a predictable developer endpoint, failing clearly if it is unavailable unless fallback is explicitly enabled.

Multiple ports use additional named tables such as `[runtime.ports.metrics]`. Publication and health checks refer to port names, never duplicated numeric literals.

## 10. Health and declared tests

Readiness and liveness are separate named checks. Supported check types may include:

- HTTP request
- TCP connection
- Process existence
- Driver-native health signal
- Explicit executable check from the release

Checks define startup grace, interval, timeout, success threshold, and failure threshold. HTTP checks name a runtime port and use loopback directly; they do not traverse public Kismet ingress.

`gump test` starts the local workload using the runtime contract, waits for readiness, executes checks marked as local tests, and shuts down through the declared termination sequence. It is not a replacement for an application's own unit-test framework.

Executable health checks run with a deliberately restricted runtime-variable view. They do not automatically inherit every application secret.

## 11. Resources and observed behavior

```toml
[resources]
cpu_request = "250m"
cpu_limit = "2"
memory_request = "256MiB"
memory_limit = "1GiB"
```

Resource values become deployment defaults. The declaration records whether each value came from the manifest, cluster policy, an authorized override, or Gump's conservative inference.

Local execution observes the same resource dimensions where the host permits. Local observations can be shown before deployment and attached as advisory evidence, but the cluster treats them as untrusted because developer hardware and load differ from production.

Omitting a request does not mean zero. It invokes the cluster's explicit unknown-workload policy. Omitting a limit means unbounded only if cluster policy permits it and the selected node reports that enforcement is optional.

## 12. Placement, rollout, and publication defaults

These sections contribute defaults to deployment intent rather than changing release identity:

```toml
[deploy]
replicas = 3

[deploy.rollout]
strategy = "rolling"
max_unavailable = 0
max_surge = 1

[deploy.placement]
require = ["os=linux", "arch=x86_64"]
spread = ["node", "zone"]

[publish]
service = "accounts-service"
port = "http"
domain = "accounts.example.com"
```

Gump carries publication intent, but Kismet decides whether and how it becomes reachable. A domain in the manifest is not a request for Gump to issue a certificate or implement ingress.

Authorized deployment flags or policy may override these defaults without rebuilding the Capsule. The resulting declaration records the final effective values and their provenance.

## 13. Local execution

`gump run` follows the cluster lifecycle as closely as the local host allows:

1. Parse and normalize the manifest.
2. Resolve local runtime-variable sources.
3. Run preparation if requested by the selected local mode.
4. Materialize or select application files.
5. Allocate ports.
6. Apply available local isolation and resource controls.
7. Start the workload using the selected execution driver.
8. Evaluate readiness and liveness.
9. Stream logs and resource observations.
10. On interruption, drain and terminate using the declared contract.

Local execution does not need to create a Capsule. A `--sealed` or equivalent verification mode may deliberately build the exact Capsule and then execute its public material and in-memory-decrypted runtime material locally, providing a higher-fidelity pre-deployment test.

Watch/reload is a local orchestration loop: changes trigger a fresh prepare/materialize/start attempt. Gump does not inject changed files into a running process unless the execution driver explicitly supports that behavior.

## 14. Deployment transaction

`gump deploy` is one user action and a multi-step transactional protocol:

1. Discover `gump.toml` and select a target cluster.
2. Parse, validate, and normalize the manifest.
3. Resolve the target cluster identity, trust material, and seal descriptor.
4. Run preparation.
5. Freeze the workspace snapshot used for packaging.
6. Resolve and validate runtime values into memory.
7. Construct deterministic public application material.
8. Construct and encrypt protected runtime material.
9. Build, stamp, and sign the Capsule.
10. Construct the proposed deployment declaration from manifest defaults and authorized overrides.
11. Present a non-secret deployment summary according to interaction policy.
12. Stream the exact Capsule and declaration to cluster ingress.
13. Await durable commit of Capsule, declaration, and application head.
14. Follow reconciliation until the deployment reaches its requested success condition or fails with actionable diagnostics.
15. Zeroize local plaintext and key material as soon as their final use completes.

The command's exit status distinguishes at least:

- Local preparation or validation failure
- Packaging or sealing failure
- Authentication or authorization failure
- Durable upload/commit failure
- Accepted but unschedulable deployment
- Started but unhealthy deployment
- Successful convergence

Interruption after durable commit does not roll back the deployment. Re-running the command with the same transaction identity safely resumes observation or retries idempotently.

## 15. Stamping and release identity

Stamping records facts without making wall-clock time authoritative. The Capsule includes or binds:

- Capsule UUID
- Cryptographic content digest
- Gump payload-dialect version
- Normalized release-contract digest
- Application identity
- Target cluster identity or permitted cluster set
- Source revision and dirty-state indicator, when discoverable
- Preparation command identity and selected output digests
- Builder Gump version
- Signer identity
- Creation time as informational metadata

The exact release identity is the Capsule UUID plus verified digest. A human label such as `v1.4.2` is an annotation and may be subject to uniqueness policy, but it cannot substitute for content identity.

Two independently built Capsules with identical public files may still differ because protected runtime values use randomized authenticated encryption. Deduplication must never depend on plaintext secret comparison.

## 16. Inspection and secret safety

Gump should make packaging transparent through commands equivalent to:

```text
gump manifest       # normalized effective manifest with sources, never values
gump files          # exact selected file inventory
gump deploy --plan  # release and declaration summary without committing
gump inspect <id>   # verified public Capsule/declaration metadata
```

Names remain provisional. The required behaviors are not.

Inspection output may reveal:

- Runtime-variable logical names
- Source reference names when local policy allows
- Classifications and injection methods
- Ciphertext size and cryptographic profile
- Public file names, sizes, modes, and digests

Inspection output never reveals resolved values, plaintext-derived hashes, partial values, or value-equality information across releases.

## 17. Validation phases

Validation happens in layers:

1. **Schema validation**: types, required fields, unknown fields, syntax, and version.
2. **Static semantic validation**: path relationships, named references, driver compatibility, and contradictory policy.
3. **Local capability validation**: tools, interpreters, files, source mappings, and local execution support.
4. **Target capability validation**: cluster dialect, seal profile, driver, architecture, isolation, and policy support.
5. **Capture validation**: exact selected paths, types, sizes, and race detection.
6. **Ingress validation**: authorization, bounds, signature, cluster binding, and declaration consistency.
7. **Agent admission**: node-local capacity and executable enforcement capability.

Errors identify the phase, manifest path, violated rule, and safe remediation without printing protected values.

## 18. Open questions

1. Should `prepare` be a first-class manifest section or an explicitly separate build integration?
2. Should package inclusion be an allowlist-only model, or may `include = ["."]` opt into a denylist-based workspace capture?
3. What exact glob and ignore semantics should Gump standardize?
4. Are symlinks ever packaged, and if so, under what containment proof?
5. Does `gump run` execute directly from the workspace by default or from a materialized release tree?
6. Should sealed local execution be a separate command or a `gump run` mode?
7. Which runtime-value source connectors belong in the core application?
8. Do `internal` and `secret` need distinct semantics in the first schema if both are encrypted and hidden?
9. What is the memory-backed file injection ABI for native, script, and OCI drivers?
10. Does changing only protected runtime values always create a complete new Capsule, or may a future runtime-configuration Capsule reference an unchanged application Capsule?
11. Which deployment-intent fields may an authorized command override, and how is override authority scoped?
12. Is a human application name sufficient identity, or does each application receive an immutable generated ID?
13. Should domain publication intent live in a committed manifest, a target-specific declaration input, or both?
14. What provenance is required when the workspace is not a Git repository?
15. How does Gump freeze a consistent workspace snapshot while a build or editor may still be changing files?
16. Which local behavior belongs in committed `gump.toml` versus ignored `gump.local.toml`?
17. What does `gump deploy` wait for by default: durable acceptance, minimum readiness, or complete convergence?
18. How are multiple processes or sidecars represented without turning one application manifest into a general pod specification?
19. Is best-effort secret scanning built into core packaging, delegated to connectors, or only a cluster policy hook?
