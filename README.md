# Gump

Gump is a zero-footprint workload placer and supervisor for one server or many. Start with one disposable beta server to test the real packaged workload, then join servers to add capacity and replicate cluster memory without changing the application model. Nodes retain only transient application materializations, while S3 holds immutable sealed Capsules. Gump runs independently and is designed to pair exceptionally well with Kismet when Kismet is present.

Gump is authored by Alexander R. Croft and licensed under
`AGPL-3.0-or-later`. Commercial licensing is available at
[frogfish.io](https://frogfish.io). See [NOTICE](NOTICE) and [LICENSE](LICENSE).

## Feedback and contributions

Bug reports and suggestions are welcome through GitHub Issues. Gump does not
accept external code, documentation, or other contributed material, including
pull requests. See [CONTRIBUTING.md](CONTRIBUTING.md) for the complete policy.

- [v1 implementation pack](docs/v1/README.md) — normative engineering handoff, formats, protocols, security, tests, and delivery backlog
- [Project seed](SEED.md)
- [System design](docs/SYSTEM_DESIGN.md)
- [Distributed cluster memory](docs/CLUSTER_MEMORY.md)
- [Workload-scoped shared K/V](docs/SHARED_KV.md)
- [Application manifest](docs/MANIFEST.md)
- [CLI and lifecycle](docs/CLI_LIFECYCLE.md)
- [Telemetry with Ratatouille](docs/TELEMETRY.md)
- [Hiccup workload discovery](docs/v1/HICCUP.md)

## Repository shape

One Cargo workspace (MSRV **1.85**, edition **2024**). Product crate boundaries match
[`docs/v1/README.md`](docs/v1/README.md) §5:

```text
crates/
  gump-types/           shared bounded types, clock, cancellation, IDs, safe errors
  gump-cli/             command UX and machine output
  gump-manifest/        parse, normalize, validate
  gump-capsule/         dialect, deterministic archive, signing transcript
  gump-crypto/          established primitives and provider traits
  gump-protocol/        protobuf messages, frame limits, golden vectors
  gump-memory/          in-memory Raft storage and typed record state machine
  gump-transport/       authenticated QUIC sessions
  gump-scheduler/       feasibility, reservations, scoring, gang admission
  gump-agent/           materialization, secret delivery, driver supervision
  gump-driver/          stable driver trait and common lifecycle
  gump-telemetry/       Ratatouille capture, relay, subscription
  gump-hiccup/          health upgrade, discovery board, keepers, SDK corpus
  gump-connectors/      object, identity, publication, output adapters
  gump-server/          role composition and process entry point
  gump-gates/           workspace quality gates (not a runtime dependency)
proto/gump/v1/          source-controlled wire schemas
spec/v1/                schemas, fixtures, vectors, and conformance data
```

Crates communicate through narrow traits and bounded typed channels. Protocol
types do not leak transport-library types. Drivers and connectors cannot mutate
cluster state directly. Dependency direction is enforced by
`cargo test -p gump-gates`. Traceability ledger checks:
`cargo run -p gump-gates --bin check-traceability` (structural) and
`--strict` / `--prove-missing` for release / W04 demonstration.

## Distribution assets

Build every currently supported raw executable with:

```sh
make dist
```

`make dist` increments the root `BUILD` counter once, then embeds
`VERSION+build-BUILD` into every target. Use `make bump` for a patch release,
or `make bump PART=minor|major` for an intentional larger version change.
`gump --version` reports the embedded identity and `gump --copyright` reports
the licensing notice. CI uses GitHub's monotonic run number as `GUMP_BUILD`, so
every architecture and package from one workflow has one identity without
creating competing commits from matrix jobs; the Actions run permanently
records that build number.

Build output is isolated from deployment under `dist/bin/<rust-target>/gump`.
The initial target set is `aarch64-apple-darwin`,
`x86_64-unknown-linux-gnu`, and `aarch64-unknown-linux-gnu`; deployment tooling
consumes these files but never compiles them. GitHub Actions builds each target
on a native runner and retains the executable plus `SHA256SUMS` as separate
workflow artifacts. CI uses `make dist-native TARGET=<rust-target>`; local
cross-building remains an optional developer convenience and is not part of
deployment.

Linux assets can be wrapped in an intentionally inert Debian package on a
Debian-family host:

```sh
make deb TARGET=x86_64-unknown-linux-gnu
make deb TARGET=aarch64-unknown-linux-gnu
```

RPM-family packages use the same existing Linux assets:

```sh
make rpm TARGET=x86_64-unknown-linux-gnu
make rpm TARGET=aarch64-unknown-linux-gnu
```

Packages are written beneath `dist/packages/deb/` and `dist/packages/rpm/`.
They install only `/usr/bin/gump` and package documentation: they do not create
an account, configuration, directories, sockets, or services, and they never
start Gump. Captain owns those host-specific effects.

## Install a published release

Published releases are available directly from
[GitHub Releases](https://github.com/frogfishio/gump/releases). Package-manager
repositories contain the current stable release; GitHub retains the historical
release assets.

On Debian or Ubuntu, install the repository key and source once:

```sh
curl -fsSL https://frogfishio.github.io/gump/packages/gump-archive-keyring.gpg \
  | sudo tee /usr/share/keyrings/gump-archive-keyring.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/gump-archive-keyring.gpg] https://frogfishio.github.io/gump/packages/apt stable main" \
  | sudo tee /etc/apt/sources.list.d/gump.list >/dev/null
sudo apt-get update
sudo apt-get install gump
```

On Fedora or another DNF-based system:

```sh
sudo dnf config-manager addrepo \
  --from-repofile=https://frogfishio.github.io/gump/packages/gump.repo
sudo dnf install gump
```

On an Apple Silicon Mac:

```sh
brew install frogfishio/tap/gump
```

Intel macOS is not currently a published target.

Release construction and one-time repository setup are documented in
[`docs/RELEASING.md`](docs/RELEASING.md).
