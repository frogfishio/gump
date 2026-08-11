#!/bin/sh
# SPDX-FileCopyrightText: 2026 Alexander R. Croft
# SPDX-License-Identifier: AGPL-3.0-or-later
set -eu

[ "$#" -eq 4 ] || {
    echo "usage: $0 VERSION TAG ARCHIVE_SHA256 OUTPUT" >&2
    exit 2
}

version=$1
tag=$2
sha256=$3
output=$4

case "$version" in *[!0-9A-Za-z.+-]*) exit 2 ;; esac
case "$tag" in v[0-9]*) ;; *) exit 2 ;; esac
case "$sha256" in *[!0-9a-f]*|'') exit 2 ;; esac
[ "${#sha256}" -eq 64 ] || exit 2

mkdir -p "$(dirname "$output")"
cat > "$output" <<EOF
class Gump < Formula
  desc "Zero-footprint cluster application deployment runtime"
  homepage "https://github.com/frogfishio/gump"
  url "https://github.com/frogfishio/gump/releases/download/$tag/gump-aarch64-apple-darwin.tar.gz"
  version "$version"
  sha256 "$sha256"
  license "AGPL-3.0-or-later"

  depends_on :macos
  depends_on arch: :arm64

  def install
    bin.install "gump"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/gump --version")
  end
end
EOF
