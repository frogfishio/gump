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

Fixture application values come from the narrow macrun `fixtures` scope. No
plaintext `.env` or generated secret file belongs in this directory.
