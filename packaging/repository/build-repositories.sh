#!/bin/sh
# SPDX-FileCopyrightText: 2026 Alexander R. Croft
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu

usage() {
    echo "usage: $0 --deb-dir DIR --rpm-dir DIR --output DIR --signing-key FINGERPRINT" >&2
    exit 2
}

deb_dir=
rpm_dir=
output=
signing_key=

while [ "$#" -gt 0 ]; do
    case "$1" in
        --deb-dir) deb_dir=$2; shift 2 ;;
        --rpm-dir) rpm_dir=$2; shift 2 ;;
        --output) output=$2; shift 2 ;;
        --signing-key) signing_key=$2; shift 2 ;;
        *) usage ;;
    esac
done

[ -d "$deb_dir" ] || usage
[ -d "$rpm_dir" ] || usage
[ -n "$output" ] || usage
[ -n "$signing_key" ] || usage
: "${GUMP_PACKAGE_SIGNING_PASSPHRASE:?GUMP_PACKAGE_SIGNING_PASSPHRASE is required}"

for tool in apt-ftparchive createrepo_c dpkg-scanpackages gpg rpm; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "$tool is required to construct the package repositories" >&2
        exit 1
    }
done

[ ! -e "$output" ] || {
    echo "repository output already exists: $output" >&2
    exit 1
}
install -d "$output/apt/pool/main/g/gump"
install -d "$output/rpm/x86_64" "$output/rpm/aarch64"

find "$deb_dir" -type f -name '*.deb' -exec cp {} "$output/apt/pool/main/g/gump/" \;
find "$rpm_dir" -type f -name '*.rpm' | while IFS= read -r package; do
    architecture=$(rpm -qp --qf '%{ARCH}' "$package")
    case "$architecture" in
        x86_64|aarch64) cp "$package" "$output/rpm/$architecture/" ;;
        *) echo "unsupported RPM architecture: $architecture" >&2; exit 1 ;;
    esac
done

for architecture in amd64 arm64; do
    packages_dir="$output/apt/dists/stable/main/binary-$architecture"
    install -d "$packages_dir"
    (
        cd "$output/apt"
        dpkg-scanpackages --arch "$architecture" pool/main/g/gump /dev/null \
            > "dists/stable/main/binary-$architecture/Packages"
        gzip -9n -c "dists/stable/main/binary-$architecture/Packages" \
            > "dists/stable/main/binary-$architecture/Packages.gz"
    )
done

(
    cd "$output/apt"
    apt-ftparchive \
        -o APT::FTPArchive::Release::Origin=Frogfish \
        -o APT::FTPArchive::Release::Label=Gump \
        -o APT::FTPArchive::Release::Suite=stable \
        -o APT::FTPArchive::Release::Codename=stable \
        -o APT::FTPArchive::Release::Architectures='amd64 arm64' \
        -o APT::FTPArchive::Release::Components=main \
        -o APT::FTPArchive::Release::Description='Gump package repository' \
        release dists/stable > dists/stable/Release
    gpg --batch --yes --pinentry-mode loopback \
        --passphrase "$GUMP_PACKAGE_SIGNING_PASSPHRASE" \
        --local-user "$signing_key" --digest-algo SHA256 \
        --clearsign --output dists/stable/InRelease dists/stable/Release
    gpg --batch --yes --pinentry-mode loopback \
        --passphrase "$GUMP_PACKAGE_SIGNING_PASSPHRASE" \
        --local-user "$signing_key" --digest-algo SHA256 \
        --armor --detach-sign --output dists/stable/Release.gpg dists/stable/Release
)

gpg --batch --yes --export "$signing_key" > "$output/gump-archive-keyring.gpg"
gpg --batch --yes --armor --export "$signing_key" > "$output/gump-signing-key.asc"

for architecture in x86_64 aarch64; do
    createrepo_c "$output/rpm/$architecture"
    gpg --batch --yes --pinentry-mode loopback \
        --passphrase "$GUMP_PACKAGE_SIGNING_PASSPHRASE" \
        --local-user "$signing_key" --digest-algo SHA256 \
        --armor --detach-sign \
        --output "$output/rpm/$architecture/repodata/repomd.xml.asc" \
        "$output/rpm/$architecture/repodata/repomd.xml"
    cp "$output/gump-signing-key.asc" "$output/rpm/$architecture/repodata/"
done

cat > "$output/gump.repo" <<'EOF'
[gump]
name=Gump stable packages
baseurl=https://frogfishio.github.io/gump/packages/rpm/$basearch
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=https://frogfishio.github.io/gump/packages/gump-signing-key.asc
EOF

touch "$output/.nojekyll"
