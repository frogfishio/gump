# macrun boundary for the Gump test cluster

macrun is the macOS-side secret entry and capture boundary. It is never
installed on a cluster node and never becomes a runtime dependency of Gump.

## Values flowing into tools

The straightforward path is `macrun run`: macrun reads an exact scope from
Keychain and launches Terraform or Gump. The child consumes the values directly
from its environment. No wrapper prints, parses, exports, or writes them.

Use three narrow scopes:

- `infra`: DigitalOcean API access for Terraform;
- `cluster`: S3 connector and recovery/bootstrap inputs for Gump formation;
- `fixtures`: application values consumed by `gump deploy` test manifests.

The installed macrun 2.x CLI is isolated in `scripts/macrun-exec.sh`. The
adapter resolves `macrun` from `PATH` (or `MACRUN_BIN`), invokes `macrun run
PROJECT ENVIRONMENT -- COMMAND`, and requires a 2.x version before launching
the child.

## Generated values flowing back to Keychain

Initialization often returns a one-time secret. Redirecting stdout to a file and
importing it later is not acceptable. Registering it as an Ansible variable is
also insufficient: `no_log` suppresses display but does not establish that
facts, callbacks, caches, temporary module data, or third-party plugins never
persisted it.

macrun 2.x already accepts a value through `macrun set PROJECT ENVIRONMENT KEY
--stdin`. A simple controller-side command can therefore consume a generated
value and place it directly into Keychain while `no_log: true` prevents normal
Ansible output.

That is useful today, but the generic Ansible `command` recipe is not yet proof
of a zero-disk path: Ansible may serialize module arguments or stage module data
in its controller temporary directory. The stronger integration should use a
dedicated result channel:

1. macrun preflights Keychain access and overwrite policy;
2. macrun launches the orchestration child with an inherited descriptor or a
   private, authenticated local Unix socket;
3. a controller-side Ansible action/callback sends a framed secret result over
   that channel;
4. macrun validates the expected project, env, key name, operation identity,
   content bounds, and producer identity;
5. macrun commits the value to Keychain atomically;
6. the child receives only an opaque receipt containing no value;
7. buffers are zeroized and the channel is closed on success, cancellation, or
   failure.

Do not scrape ordinary stdout. Logs and secrets inevitably become ambiguous,
and stdout is routinely captured by terminals, CI, callbacks, and evidence
collectors.

### Terraform limitation

macrun cannot retroactively protect a generated value that a Terraform provider
has already written into Terraform state. Marking an output `sensitive` hides
normal display; it does not remove or encrypt the value in state.

Do not model generated plaintext credentials as ordinary Terraform-managed
resource attributes. Either use a provider design that never returns plaintext,
generate the value locally in macrun and supply it to the real initializer, or
run a separate post-infrastructure initialization operation through the
dedicated macrun capture/encrypted-return channel. Terraform should retain only
non-secret identifiers or encrypted envelopes.

## Prefer encrypted return for one-shot initialization

A crash between remote secret generation and local Keychain commit can make a
one-time initialization unrecoverable. Preflight reduces this window but does
not eliminate it.

For generators that support it, macrun should provide an ephemeral recipient
public key. The remote initializer encrypts the generated value immediately and
returns only an authenticated ciphertext envelope through Ansible. macrun then
decrypts it directly into Keychain. The encrypted envelope may be retried or
temporarily retained without exposing plaintext.

When the initialized system accepts caller-generated recovery material, the
stronger and simpler direction is to generate it locally inside macrun and send
it to the initializer through the existing secret-input channel. Gump should
prefer this model for recovery/unseal authority.

## Required capture semantics

- explicit destination project, env, and key;
- create-only by default; overwrite requires an explicit policy;
- atomic all-or-none storage for a generated bundle;
- strict byte and key-count bounds;
- no values in argv, environment inherited by unrelated children, stdout,
  stderr, JSON results, Ansible facts, callback events, or error strings;
- cancellation and timeout propagation in both directions;
- an opaque receipt suitable for Ansible idempotency checks;
- a test mode using canaries and a disposable Keychain namespace;
- clear behavior when generation succeeds but capture/commit fails;
- no shell command tracing around secret-bearing operations.

## Gump-specific use

Gump cluster formation should normally receive recovery authority from macrun,
not generate a reusable plaintext recovery file. If Gump returns a generated
share, emergency token, or bootstrap authority, it must use the dedicated
capture channel or encrypted-return envelope.

Application deployment remains simpler:

```text
macrun run gump-test-cluster fixtures -- gump deploy <fixture>
```

Gump reads only manifest-declared names and immediately seals their values into
the protected Capsule segment. The exact sealed Capsule in S3 is the durable
recovery artifact; macrun is the local source, not the server-side store.
