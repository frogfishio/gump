# Project Brief: Gump
The Minimalist App Placer & Forest Supervisor
"Gump manages the forest so your apps can run."
1. Vision & Core Philosophy
Gump is an anti-complexity, CLI-driven application placer and supervisor built to pair directly with Kismet.
Modern orchestrators (Kubernetes, Nomad) have become overly complex platform ecosystems optimized for enterprise monetization. Gump takes the opposite approach: its sole responsibility is local process placement and lifecycle management. It deliberately delegates networking, TLS, cross-node routing, and edge ingress entirely to Kismet.
Guiding Principles
• Developer-First UX: Operating Gump requires no platform engineering background. If you can type gump deploy, you can manage production workloads.
• Radical Isolation of Scope: No CNI plugins, no ingress controllers, and no service mesh logic. Gump places processes on loopback (127.0.0.1); Kismet makes them securely reachable.
• Hermetic App Bundling: Application code, environment vars, and metadata ship together as a single deployable unit.
• Zero Platform Lock-in: Gump acts as a process supervisor. It runs raw binaries, scripts, or container images on standard Unix hosts.
2. Architecture & System Boundary
Gump divides orchestrator responsibilities into a minimal single-leader architecture with lightweight node agents:
[ Developer Laptop ]
        │
        │ gump deploy (CLI)
        ▼
  ┌───────────┐
  │ Gump      │ <--- State Sync (Raft / SQLite)
  │ Leader    │
  └─────┬─────┘
        │
        ├────────────────────────┬────────────────────────┐
        │ Payload & Task         │ Payload & Task         │ Payload & Task
        ▼                        ▼                        ▼
  ┌───────────┐            ┌───────────┐            ┌───────────┐
  │ Node A    │            │ Node B    │            │ Node C    │
  │ ┌───────┐ │            │ ┌───────┐ │            │ ┌───────┐ │
  │ │ Gump  │ │            │ │ Gump  │ │            │ │ Gump  │ │
  │ │ Agent │ │            │ │ Agent │ │            │ │ Agent │ │
  │ └───┬───┘ │            │ └───┬───┘ │            │ └───┬───┘ │
  │     │       │            │     │       │            │     │       │
  │ ┌───▼───┐ │            │ ┌───▼───┐ │            │ ┌───▼───┐ │
  │ │ App   │ │            │ │ App   │ │            │ │ App   │ │
  │ │ Port  │ │            │ │ Port  │ │            │ │ Port  │ │
  │ └───┬───┘ │            │ └───┬───┘ │            │ └───┬───┘ │
  │     │ 127.0.0.1        │     │ 127.0.0.1        │     │ 127.0.0.1
  │ ┌───▼───┐ │            │ ┌───▼───┐ │            │ ┌───▼───┐ │
  │ │Kismet │ │            │ │Kismet │ │            │ │Kismet │ │
  │ │ Daemon│ │            │ │ Daemon│ │            │ │ Daemon│ │
  │ └───────┘ │            │ └───────┘ │            │ └───────┘ │
  └───────────┘            └───────────┘            └───────────┘

Responsibility Matrix
Feature	Gump (The Placer)	Kismet (The Network Layer)
App Placement	Fetches binary/image, spawns process	Ignorant of process creation
Local Binding	Assigns random 127.0.0.1:PORT	Accepts local publication request
Process Control	Supervisors (restart on crash, I/O logs)	Watches local target health
Ingress & TLS	Ignorant of domains/certificates	Issues certs, handles SNI, terminates TLS
Inter-Node Traffic	Ignorant of cluster network	Relays encrypted traffic between nodes
3. Developer Workflow & CLI Specification
The developer interacts exclusively through a clean, subcommand-based CLI.
Primary Commands
# Initialize a minimal config file in current directory
gump init [app-name]

# Package local source/binary, stream to cluster, and execute deployment
gump deploy

# Scale application instances up or down across cluster nodes
gump scale [app-name]=[replicas]

# Roll back to previous deployment release
gump rollback [app-name]

# View cluster-wide workload status and logs
gump status
gump logs [app-name] --follow

4. Configuration Schema (gump.toml)
Developers can include an optional gump.toml in their project root. If absent, Gump infers defaults based on project structure.
name = "accounts-service"
domain = "accounts.example.com"
replicas = 3

[build]
# Supports 'binary', 'docker', or 'script'
runner = "binary"
exec = "./bin/accounts-server"

[env]
LOG_LEVEL = "info"
PORT = "auto" # Gump automatically injects available 127.0.0.1 port

[secrets]
DATABASE_URL = "env:PROD_DB_URL"

[health]
path = "/health"
interval = "5s"

5. Execution Pipeline (gump deploy)
When a user executes gump deploy from their local machine, the system executes a 5-step lifecycle:
1. Bundle: Local CLI packages code/binary, static assets, and gump.toml into a compressed tarball artifact (.gump bundle).
2. Ship: The CLI streams the bundle over TLS to the Gump Cluster Leader via gRPC.
3. Schedule: The Leader selects N target nodes based on node capacity and current replica distribution.
4. Spawn: The local gump-agent on each target node unpacks the payload, allocates an available loopback port (e.g., 127.0.0.1:41029), and starts the child process as a supervised process.
5. Publish to Kismet: The gump-agent registers a lease-bound service publication directly with Kismet's local Unix socket: • Service Name: accounts-service • Target: 127.0.0.1:41029 • Domain: accounts.example.com
6. Key Design Decisions & Technical Trade-Offs
1. Communication with Kismet
Gump uses Kismet's Lease-Bound Unix Socket API. If a gump-agent or the underlying application crashes, the Kismet publication lease expires automatically, instantly dropping the dead node target from public ingress without leaving dangling routes.
2. Process Supervision Strategy
Instead of reinventing process management, gump-agent acts as an embedded process supervisor (similar to supervisord or systemd). It captures stdout/stderr into ring buffers for log aggregation and handles local restart policies.
3. Minimal State Storage
The Gump Leader uses an embedded, zero-dependency database (e.g., sqlite with Raft replication or rqlite) to keep track of current cluster state, active releases, and secret references.
7. Next Steps for Seed Analysis
1. Payload Execution Contract: Define support limits for raw Linux binaries vs. OCI/Docker container images.
2. Secret Distribution: Determine how encrypted secrets travel from local CLI to target node memory during deployment.
3. Leader Election: Evaluate whether Gump should use its own light Raft consensus for state, or hook into Kismet's existing node identity/gossip mechanisms.
What aspect of Gump should we detail next—the Gump-to-Kismet socket protocol handshake or the internal process supervisor logic on the node agent?