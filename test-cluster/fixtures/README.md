# Live test fixtures

The first live fixture is `finite-hello`: a signed, cluster-sealed script
workload carrying one encrypted runtime value. Build it against the currently
running cluster with `make fixture-finite`; generated Capsules go to ignored
`evidence/` storage.

Additional fixtures will be added as their corresponding product paths land:

- finite native execution;
- continuous native execution;
- binary and saturated stdout/stderr;
- descendant/process-tree cleanup;
- protected-value canary scanning;
- Hiccup `@self` discovery and movement;
- Kismet deployed as an `all_nodes` Capsule;
- OCI and synthetic GPU/gang profiles.

`fixtures/ringtail/gump.toml` is the first real-product fixture: a continuous
native collector deployed with `coverage = "all_nodes"`. Gump generates its
producer and Hiccup credentials independently for every attempt and passes them
through inherited memory descriptors. Neither credential enters the Capsule or
the `fixtures` Keychain scope.

`fixtures/ringtail-relay-probe/gump.toml` is a finite `all_nodes` workload. Each
node emits one stdout record so acceptance can prove node-local delivery into
all three independent Ringtail collectors without learning their bearer tokens.

Fixture application values come from the narrow macrun `fixtures` scope. No
plaintext `.env` or generated secret file belongs in this directory.
