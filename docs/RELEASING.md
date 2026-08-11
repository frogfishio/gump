# Publishing Gump

Gump uses a version tag as the only publication trigger. The tag must exactly
match the root `VERSION` file: `VERSION=0.1.0` is released with `v0.1.0`.

The workflow validates and builds each target once, signs the Linux package
channels, publishes the same files to GitHub Releases, deploys the current APT
and RPM repositories to GitHub Pages, and updates the Frogfish Homebrew tap.

## One-time GitHub configuration

1. In the Gump repository Pages settings, select **GitHub Actions** as the
   publishing source.
2. Create the public repository `frogfishio/homebrew-tap` with a `main` branch.
3. Add a dedicated write-enabled SSH deploy key to that tap and store only its
   private half as the Gump repository secret `HOMEBREW_TAP_DEPLOY_KEY`. The
   key must not be reused for another repository or purpose.
4. Store the Macrun `gump/release` values as repository secrets with the same
   names:
   - `GUMP_PACKAGE_SIGNING_KEY`
   - `GUMP_PACKAGE_SIGNING_PASSPHRASE`

The CI key is a signing subkey. Keep `GUMP_PACKAGE_SIGNING_MASTER_KEY` and its
passphrase outside GitHub, backed up in KeePassXC or equivalent protected
storage. The committed authority fingerprint is in
`packaging/repository/SIGNING_KEY`.

The release workflow needs repository **Actions** permissions set to
**Read and write** so its scoped `GITHUB_TOKEN` can create the GitHub Release.
It also uses GitHub's `pages`, OIDC, and artifact-attestation permissions.

## Release

Run all ordinary CI on `main`, update `VERSION`, and then create and push the
matching tag:

```sh
make bump PART=patch
git add VERSION
git commit -m "release 0.1.1"
git tag -s v0.1.1 -m "Gump 0.1.1"
git push origin main v0.1.1
```

Do not reuse or move a release tag. After the first successful publication,
enable immutable releases in the GitHub repository settings.

## Failure and retry

Publication jobs are ordered after all builds and checks. A failed Pages or tap
update can be rerun from the same workflow without rebuilding a different
binary. GitHub Release uploads are idempotent until immutable releases are
enabled. Once immutability is enabled, fix the publication channel and rerun
only its failed job; never replace published release assets.

GitHub Releases retain every version. GitHub Pages intentionally contains only
the current stable APT and RPM package set, keeping the static repository small
and replaceable.
