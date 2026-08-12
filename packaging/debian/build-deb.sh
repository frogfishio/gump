#!/bin/sh
# SPDX-FileCopyrightText: 2026 Alexander R. Croft
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu

usage() {
    echo "usage: $0 --binary PATH --target RUST_TARGET --version VERSION --output DIR" >&2
    exit 2
}

binary=
target=
version=
output=

while [ "$#" -gt 0 ]; do
    case "$1" in
        --binary)
            [ "$#" -ge 2 ] || usage
            binary=$2
            shift 2
            ;;
        --target)
            [ "$#" -ge 2 ] || usage
            target=$2
            shift 2
            ;;
        --version)
            [ "$#" -ge 2 ] || usage
            version=$2
            shift 2
            ;;
        --output)
            [ "$#" -ge 2 ] || usage
            output=$2
            shift 2
            ;;
        *) usage ;;
    esac
done

[ -n "$binary" ] || usage
[ -n "$target" ] || usage
[ -n "$version" ] || usage
[ -n "$output" ] || usage
[ -f "$binary" ] || { echo "Gump binary not found: $binary" >&2; exit 1; }
[ -x "$binary" ] || { echo "Gump binary is not executable: $binary" >&2; exit 1; }
command -v dpkg-deb >/dev/null 2>&1 || {
    echo "dpkg-deb is required to build the Debian package" >&2
    exit 1
}
dpkg --validate-version "$version" >/dev/null

case "$target" in
    x86_64-unknown-linux-gnu) architecture=amd64 ;;
    aarch64-unknown-linux-gnu) architecture=arm64 ;;
    *)
        echo "unsupported Debian package target: $target" >&2
        exit 1
        ;;
esac

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/gump-deb.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT HUP INT TERM

package_root="$work_dir/gump"
install -d "$package_root/DEBIAN"
install -d "$package_root/usr/bin"
install -d "$package_root/usr/share/doc/gump"
install -d "$package_root/usr/share/gump/systemd"
install -m 0755 "$binary" "$package_root/usr/bin/gump"
install -m 0644 "$repo_root/LICENSE" "$package_root/usr/share/doc/gump/LICENSE"
install -m 0644 "$repo_root/NOTICE" "$package_root/usr/share/doc/gump/NOTICE"
install -m 0644 "$script_dir/README.Debian" "$package_root/usr/share/doc/gump/README.Debian"
install -m 0644 "$repo_root/packaging/systemd/gump-bootstrap.service" \
    "$package_root/usr/share/gump/systemd/gump-bootstrap.service"
cat >"$package_root/usr/share/doc/gump/copyright" <<'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: Gump
Source: https://github.com/frogfishio/gump

Files: *
Copyright: 2026 Alexander R. Croft
License: AGPL-3+
 This program is free software: you can redistribute it and/or modify it under
 the terms of the GNU Affero General Public License as published by the Free
 Software Foundation, either version 3 of the License, or (at your option) any
 later version.
 .
 On Debian systems, the complete text of version 3 is available in
 /usr/share/common-licenses/AGPL-3. The upstream LICENSE file is also included.

Comment:
 A commercial license is available from https://frogfish.io.
EOF

installed_size=$(du -sk "$package_root/usr" | awk '{print $1}')
cat >"$package_root/DEBIAN/control" <<EOF
Package: gump
Version: $version
Architecture: $architecture
Maintainer: Frogfish <info@frogfish.io>
Installed-Size: $installed_size
Depends: libc6 (>= 2.35), libgcc-s1
Section: admin
Priority: optional
Homepage: https://github.com/frogfishio/gump
Description: zero-footprint cluster application deployment runtime
 Gump packages, places, supervises, and discovers arbitrary workloads across a
 cluster. This package installs only the Gump executable; Captain owns host
 configuration and service lifecycle.
EOF

find "$package_root/usr" -type f -print0 \
    | sort -z \
    | xargs -0 md5sum \
    | sed "s#  $package_root/#  #" >"$package_root/DEBIAN/md5sums"

if [ -n "${SOURCE_DATE_EPOCH:-}" ]; then
    case "$SOURCE_DATE_EPOCH" in
        *[!0-9]*)
            echo "SOURCE_DATE_EPOCH must be an unsigned integer" >&2
            exit 1
            ;;
    esac
    find "$package_root" -exec touch -d "@$SOURCE_DATE_EPOCH" {} +
fi

mkdir -p "$output"
artifact="$output/gump_${version}_${architecture}.deb"
dpkg-deb --root-owner-group --build "$package_root" "$artifact"
(cd "$output" && sha256sum "$(basename "$artifact")" >"$(basename "$artifact").sha256")
echo "$artifact"
