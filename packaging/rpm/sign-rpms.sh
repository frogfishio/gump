#!/bin/sh
# SPDX-FileCopyrightText: 2026 Alexander R. Croft
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu

[ "$#" -eq 2 ] || {
    echo "usage: $0 SIGNING_KEY_FINGERPRINT RPM_DIRECTORY" >&2
    exit 2
}

signing_key=$1
rpm_dir=$2
: "${GUMP_PACKAGE_SIGNING_PASSPHRASE:?GUMP_PACKAGE_SIGNING_PASSPHRASE is required}"

[ -d "$rpm_dir" ] || exit 2
command -v rpmsign >/dev/null 2>&1 || {
    echo "rpmsign is required" >&2
    exit 1
}
command -v rpmkeys >/dev/null 2>&1 || {
    echo "rpmkeys is required" >&2
    exit 1
}

umask 077
signing_dir=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/gump-rpm-signing.XXXXXX")
passphrase_file=$signing_dir/passphrase
public_key_file=$signing_dir/public-key.asc
rpm_db=$signing_dir/rpmdb
cleanup() {
    rm -rf -- "$signing_dir"
}
trap cleanup EXIT HUP INT TERM
printf '%s' "$GUMP_PACKAGE_SIGNING_PASSPHRASE" > "$passphrase_file"
mkdir -m 0700 "$rpm_db"
gpg --batch --armor --export "$signing_key" > "$public_key_file"
rpm --dbpath "$rpm_db" --initdb
rpmkeys --dbpath "$rpm_db" --import "$public_key_file"

# RPM streams the package payload to GPG on stdin. Keep the passphrase on a
# separate, owner-only temporary file so arbitrary passphrase bytes do
# not enter RPM's macro parser or the process arguments.
export GPG_TTY=/dev/null

find "$rpm_dir" -type f -name '*.rpm' | while IFS= read -r package; do
    rpmsign --addsign \
        --define "_signature gpg" \
        --define "_gpg_name $signing_key" \
        --define "_gpg_path $GNUPGHOME" \
        --define "_gpg_sign_cmd_extra_args --batch --pinentry-mode loopback --passphrase-file $passphrase_file" \
        "$package"
    rpm --dbpath "$rpm_db" --checksig "$package" \
        | grep -Eq 'digests signatures OK|digests signatures.*OK'
done

# POSIX pipelines execute the loop in a subshell, so verify independently.
find "$rpm_dir" -type f -name '*.rpm' -print -quit | grep -q . || {
    echo "no RPM packages found in $rpm_dir" >&2
    exit 1
}
