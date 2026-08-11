#!/bin/sh
# SPDX-FileCopyrightText: 2026 Alexander R. Croft
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
version=$(tr -d '[:space:]' <"$repo_root/VERSION")
build=$(tr -d '[:space:]' <"$repo_root/BUILD")
cargo_version=$(awk '
    /^\[workspace\.package\]$/ { in_workspace_package = 1; next }
    /^\[/ && in_workspace_package { in_workspace_package = 0 }
    in_workspace_package && /^version = / {
        value = $0
        sub(/^version = "/, "", value)
        sub(/".*$/, "", value)
        print value
        exit
    }
' "$repo_root/Cargo.toml")

old_ifs=$IFS
IFS=.
set -- $version
IFS=$old_ifs
[ "$#" -eq 3 ] || { echo "VERSION must contain numeric major.minor.patch" >&2; exit 1; }
for value in "$1" "$2" "$3"; do
    case "$value" in
        ''|*[!0-9]*) echo "VERSION must contain numeric major.minor.patch" >&2; exit 1 ;;
    esac
done
case "$build" in
    ''|*[!0-9]*) echo "BUILD must contain an unsigned integer" >&2; exit 1 ;;
esac
[ "$cargo_version" = "$version" ] || {
    echo "VERSION ($version) differs from workspace.package.version ($cargo_version)" >&2
    exit 1
}
grep -q '^license = "AGPL-3.0-or-later"$' "$repo_root/Cargo.toml" || {
    echo "workspace license must be AGPL-3.0-or-later" >&2
    exit 1
}
grep -q '^authors = \["Alexander R. Croft"\]$' "$repo_root/Cargo.toml" || {
    echo "workspace author must be Alexander R. Croft" >&2
    exit 1
}
grep -q '^Copyright (C) 2026 Alexander R. Croft$' "$repo_root/NOTICE" || {
    echo "NOTICE lacks the required copyright declaration" >&2
    exit 1
}
grep -q 'AGPL-3.0-or-later' "$repo_root/NOTICE" || {
    echo "NOTICE lacks the AGPL-3.0-or-later declaration" >&2
    exit 1
}
grep -q 'https://frogfish.io' "$repo_root/NOTICE" || {
    echo "NOTICE lacks the commercial-license location" >&2
    exit 1
}
printf 'release metadata: %s+build-%s (AGPL-3.0-or-later)\n' "$version" "$build"
