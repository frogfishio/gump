#!/bin/sh
# SPDX-FileCopyrightText: 2026 Alexander R. Croft
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
build_file="$repo_root/BUILD"
build=$(tr -d '[:space:]' <"$build_file")

case "$build" in
    ''|*[!0-9]*)
        echo "BUILD must contain an unsigned integer" >&2
        exit 1
        ;;
esac

next=$((build + 1))
tmp="$build_file.tmp.$$"
trap 'rm -f "$tmp"' EXIT HUP INT TERM
printf '%s\n' "$next" >"$tmp"
mv "$tmp" "$build_file"
trap - EXIT HUP INT TERM
printf 'Gump build %s\n' "$next"
