#!/bin/sh
# SPDX-FileCopyrightText: 2026 Alexander R. Croft
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
part=${1:-patch}
version_file="$repo_root/VERSION"
cargo_file="$repo_root/Cargo.toml"
version=$(tr -d '[:space:]' <"$version_file")

old_ifs=$IFS
IFS=.
set -- $version
IFS=$old_ifs
[ "$#" -eq 3 ] || { echo "VERSION must be numeric major.minor.patch" >&2; exit 1; }
major=$1
minor=$2
patch=$3
for value in "$major" "$minor" "$patch"; do
    case "$value" in ''|*[!0-9]*) echo "VERSION must be numeric major.minor.patch" >&2; exit 1 ;; esac
done

case "$part" in
    patch) patch=$((patch + 1)) ;;
    minor) minor=$((minor + 1)); patch=0 ;;
    major) major=$((major + 1)); minor=0; patch=0 ;;
    *) echo "PART must be patch, minor, or major" >&2; exit 2 ;;
esac
next="$major.$minor.$patch"

version_tmp="$version_file.tmp.$$"
cargo_tmp="$cargo_file.tmp.$$"
trap 'rm -f "$version_tmp" "$cargo_tmp"' EXIT HUP INT TERM
printf '%s\n' "$next" >"$version_tmp"
awk -v new_version="$next" '
    BEGIN { in_workspace_package = 0; changed = 0 }
    /^\[workspace\.package\]$/ { in_workspace_package = 1; print; next }
    /^\[/ && in_workspace_package { in_workspace_package = 0 }
    in_workspace_package && /^version = / {
        print "version = \"" new_version "\""
        changed = 1
        next
    }
    { print }
    END { if (!changed) exit 1 }
' "$cargo_file" >"$cargo_tmp"
mv "$version_tmp" "$version_file"
mv "$cargo_tmp" "$cargo_file"
trap - EXIT HUP INT TERM
printf 'Gump version %s\n' "$next"
