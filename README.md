# Gump

Gump is a zero-footprint workload placer and supervisor for one server or many. Start with one disposable beta server to test the real packaged workload, then join servers to add capacity and replicate cluster memory without changing the application model. Nodes retain only transient application materializations, while S3 holds immutable sealed Capsules. Gump runs independently and is designed to pair exceptionally well with Kismet when Kismet is present.

- [v1 implementation pack](docs/v1/README.md) — normative engineering handoff, formats, protocols, security, tests, and delivery backlog
- [Project seed](SEED.md)
- [System design](docs/SYSTEM_DESIGN.md)
- [Distributed cluster memory](docs/CLUSTER_MEMORY.md)
- [Application manifest](docs/MANIFEST.md)
- [CLI and lifecycle](docs/CLI_LIFECYCLE.md)
- [Telemetry with Ratatouille](docs/TELEMETRY.md)
