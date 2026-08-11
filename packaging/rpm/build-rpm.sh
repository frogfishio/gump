#!/bin/sh
set -eu

usage() {
    echo "usage: $0 --binary PATH --target RUST_TARGET --version VERSION --release RELEASE --output DIR" >&2
    exit 2
}

binary=
target=
version=
release=
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
        --release)
            [ "$#" -ge 2 ] || usage
            release=$2
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
[ -n "$release" ] || usage
[ -n "$output" ] || usage
[ -f "$binary" ] || { echo "Gump binary not found: $binary" >&2; exit 1; }
[ -x "$binary" ] || { echo "Gump binary is not executable: $binary" >&2; exit 1; }
command -v rpmbuild >/dev/null 2>&1 || {
    echo "rpmbuild is required to build the RPM package" >&2
    exit 1
}
command -v rpm >/dev/null 2>&1 || {
    echo "rpm is required to build the RPM package" >&2
    exit 1
}

case "$version" in
    *[!A-Za-z0-9._+~]*) echo "invalid RPM version: $version" >&2; exit 1 ;;
esac
case "$release" in
    *[!A-Za-z0-9._+~]*) echo "invalid RPM release: $release" >&2; exit 1 ;;
esac

case "$target" in
    x86_64-unknown-linux-gnu) architecture=x86_64 ;;
    aarch64-unknown-linux-gnu) architecture=aarch64 ;;
    *)
        echo "unsupported RPM package target: $target" >&2
        exit 1
        ;;
esac

host_arch=$(rpm --eval '%{_arch}')
[ "$host_arch" = "$architecture" ] || {
    echo "RPM target $architecture must be packaged on a native $architecture host (found $host_arch)" >&2
    exit 1
}

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/gump-rpm.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT HUP INT TERM

top_dir="$work_dir/rpmbuild"
install -d "$top_dir/BUILD" "$top_dir/BUILDROOT" "$top_dir/RPMS"
install -d "$top_dir/SOURCES" "$top_dir/SPECS" "$top_dir/SRPMS"
install -m 0755 "$binary" "$top_dir/SOURCES/gump"
install -m 0644 "$repo_root/LICENSE" "$top_dir/SOURCES/LICENSE"
install -m 0644 "$script_dir/README.rpm" "$top_dir/SOURCES/README.rpm"

cat >"$top_dir/SPECS/gump.spec" <<EOF
%global __os_install_post %{nil}

Name:           gump
Version:        $version
Release:        $release
Summary:        Zero-footprint cluster application deployment runtime
License:        AGPL-3.0-only
URL:            https://github.com/frogfishio/gump
BuildArch:      $architecture
Source0:        gump
Source1:        LICENSE
Source2:        README.rpm

%description
Gump packages, places, supervises, and discovers arbitrary workloads across a
cluster. This package installs only the Gump executable; Captain owns host
configuration and service lifecycle.

%prep

%build

%install
install -Dpm 0755 %{SOURCE0} %{buildroot}%{_bindir}/gump
install -Dpm 0644 %{SOURCE1} %{buildroot}%{_licensedir}/gump/LICENSE
install -Dpm 0644 %{SOURCE2} %{buildroot}%{_docdir}/gump/README.rpm

%files
%{_bindir}/gump
%license %{_licensedir}/gump/LICENSE
%doc %{_docdir}/gump/README.rpm
EOF

if [ -n "${SOURCE_DATE_EPOCH:-}" ]; then
    case "$SOURCE_DATE_EPOCH" in
        *[!0-9]*)
            echo "SOURCE_DATE_EPOCH must be an unsigned integer" >&2
            exit 1
            ;;
    esac
    find "$top_dir/SOURCES" "$top_dir/SPECS" -exec touch -d "@$SOURCE_DATE_EPOCH" {} +
fi

rpmbuild -bb \
    --define "_topdir $top_dir" \
    --define "use_source_date_epoch_as_buildtime 1" \
    --define "clamp_mtime_to_source_date_epoch 1" \
    "$top_dir/SPECS/gump.spec"

built_rpm=$(find "$top_dir/RPMS/$architecture" -type f -name 'gump-*.rpm' -print -quit)
[ -n "$built_rpm" ] || { echo "rpmbuild produced no Gump package" >&2; exit 1; }

mkdir -p "$output"
artifact="$output/$(basename "$built_rpm")"
install -m 0644 "$built_rpm" "$artifact"
(cd "$output" && sha256sum "$(basename "$artifact")" >"$(basename "$artifact").sha256")
echo "$artifact"
