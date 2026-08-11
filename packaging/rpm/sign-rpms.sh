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

find "$rpm_dir" -type f -name '*.rpm' | while IFS= read -r package; do
    rpmsign --addsign \
        --define "_signature gpg" \
        --define "_gpg_name $signing_key" \
        --define "_gpg_path $GNUPGHOME" \
        --define "_gpg_sign_cmd_extra_args --batch --pinentry-mode loopback --passphrase $GUMP_PACKAGE_SIGNING_PASSPHRASE" \
        "$package"
    rpm --checksig "$package" | grep -Eq 'digests signatures OK|digests signatures.*OK'
done

# POSIX pipelines execute the loop in a subshell, so verify independently.
find "$rpm_dir" -type f -name '*.rpm' -print -quit | grep -q . || {
    echo "no RPM packages found in $rpm_dir" >&2
    exit 1
}
