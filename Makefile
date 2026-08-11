# SPDX-FileCopyrightText: 2026 Alexander R. Croft
# SPDX-License-Identifier: AGPL-3.0-or-later

.PHONY: help dist dist-all dist-native dist-macos-arm64 dist-linux-x86_64 dist-linux-arm64 \
	deb deb-linux-x86_64 deb-linux-arm64 rpm rpm-linux-x86_64 rpm-linux-arm64 \
	bump bump-build check-release-metadata clean-dist

MACOS_ARM64_TARGET := aarch64-apple-darwin
LINUX_X86_64_TARGET := x86_64-unknown-linux-gnu
LINUX_ARM64_TARGET := aarch64-unknown-linux-gnu
DIST_ROOT := $(CURDIR)/dist/bin
PACKAGE_ROOT := $(CURDIR)/dist/packages
VERSION := $(shell tr -d '[:space:]' < "$(CURDIR)/VERSION")
BUILD := $(shell tr -d '[:space:]' < "$(CURDIR)/BUILD")
BUILD_ID := $(if $(GUMP_BUILD),$(GUMP_BUILD),$(BUILD))
DEB_VERSION ?= $(VERSION)+build$(BUILD_ID)
RPM_VERSION ?= $(VERSION)
RPM_RELEASE ?= build$(BUILD_ID)
ZIG_GLOBAL_CACHE_DIR := $(CURDIR)/target/zig-global-cache
ZIG_LOCAL_CACHE_DIR := $(CURDIR)/target/zig-local-cache

help:
	@printf '%s\n' \
	  'Gump build targets' \
	  '' \
	  '  make dist                Build all currently supported raw assets.' \
	  '                           Increments BUILD exactly once before building.' \
	  '  make bump [PART=patch]   Bump VERSION (patch, minor, or major).' \
	  '  make dist-native TARGET=<rust-target>' \
	  '                           Build one target on its native host (used by CI).' \
	  '  make dist-macos-arm64    Build dist/bin/aarch64-apple-darwin/gump.' \
	  '  make dist-linux-x86_64   Build dist/bin/x86_64-unknown-linux-gnu/gump.' \
	  '  make dist-linux-arm64    Build dist/bin/aarch64-unknown-linux-gnu/gump.' \
	  '  make deb TARGET=<rust-target> [DEB_VERSION=<debian-version>]' \
	  '                           Package an existing Linux asset as a .deb.' \
	  '  make deb-linux-x86_64    Package the Linux x86-64 asset as a .deb.' \
	  '  make deb-linux-arm64     Package the Linux ARM64 asset as a .deb.' \
	  '  make rpm TARGET=<rust-target> [RPM_VERSION=<version>] [RPM_RELEASE=<release>]' \
	  '                           Package an existing Linux asset as an .rpm.' \
	  '  make rpm-linux-x86_64    Package the Linux x86-64 asset as an .rpm.' \
	  '  make rpm-linux-arm64     Package the Linux ARM64 asset as an .rpm.' \
	  '  make clean-dist          Remove generated distribution assets.'

dist: bump-build
	$(MAKE) dist-all

dist-all: dist-macos-arm64 dist-linux-x86_64 dist-linux-arm64

bump:
	"$(CURDIR)/scripts/bump-version.sh" "$(PART)"

bump-build:
	"$(CURDIR)/scripts/bump-build.sh"

check-release-metadata:
	"$(CURDIR)/scripts/check-release-metadata.sh"

dist-native:
	@test -n "$(TARGET)" || { echo 'TARGET is required' >&2; exit 2; }
	rustup target add "$(TARGET)"
	cargo build --locked --release --target "$(TARGET)" -p gump-server --bin gump
	install -d "$(DIST_ROOT)/$(TARGET)"
	install -m 0755 "$(CURDIR)/target/$(TARGET)/release/gump" \
	  "$(DIST_ROOT)/$(TARGET)/gump"
	printf '%s\n' "$(VERSION)+build-$(BUILD_ID)" > "$(DIST_ROOT)/$(TARGET)/BUILD_INFO"
	cd "$(DIST_ROOT)/$(TARGET)" && shasum -a 256 gump > SHA256SUMS

dist-macos-arm64:
	$(MAKE) dist-native TARGET=$(MACOS_ARM64_TARGET)

dist-linux-x86_64:
	@command -v zig >/dev/null || { echo 'zig is required for Linux cross-compilation' >&2; exit 2; }
	@command -v cargo-zigbuild >/dev/null || { echo 'cargo-zigbuild is required for Linux cross-compilation' >&2; exit 2; }
	rustup target add $(LINUX_X86_64_TARGET)
	ZIG_GLOBAL_CACHE_DIR="$(ZIG_GLOBAL_CACHE_DIR)" \
	ZIG_LOCAL_CACHE_DIR="$(ZIG_LOCAL_CACHE_DIR)" \
	cargo zigbuild --locked --release --target $(LINUX_X86_64_TARGET) -p gump-server --bin gump
	install -d "$(DIST_ROOT)/$(LINUX_X86_64_TARGET)"
	install -m 0755 "$(CURDIR)/target/$(LINUX_X86_64_TARGET)/release/gump" \
	  "$(DIST_ROOT)/$(LINUX_X86_64_TARGET)/gump"
	printf '%s\n' "$(VERSION)+build-$(BUILD_ID)" > "$(DIST_ROOT)/$(LINUX_X86_64_TARGET)/BUILD_INFO"
	cd "$(DIST_ROOT)/$(LINUX_X86_64_TARGET)" && shasum -a 256 gump > SHA256SUMS

dist-linux-arm64:
	@command -v zig >/dev/null || { echo 'zig is required for Linux cross-compilation' >&2; exit 2; }
	@command -v cargo-zigbuild >/dev/null || { echo 'cargo-zigbuild is required for Linux cross-compilation' >&2; exit 2; }
	rustup target add $(LINUX_ARM64_TARGET)
	ZIG_GLOBAL_CACHE_DIR="$(ZIG_GLOBAL_CACHE_DIR)" \
	ZIG_LOCAL_CACHE_DIR="$(ZIG_LOCAL_CACHE_DIR)" \
	cargo zigbuild --locked --release --target $(LINUX_ARM64_TARGET) -p gump-server --bin gump
	install -d "$(DIST_ROOT)/$(LINUX_ARM64_TARGET)"
	install -m 0755 "$(CURDIR)/target/$(LINUX_ARM64_TARGET)/release/gump" \
	  "$(DIST_ROOT)/$(LINUX_ARM64_TARGET)/gump"
	printf '%s\n' "$(VERSION)+build-$(BUILD_ID)" > "$(DIST_ROOT)/$(LINUX_ARM64_TARGET)/BUILD_INFO"
	cd "$(DIST_ROOT)/$(LINUX_ARM64_TARGET)" && shasum -a 256 gump > SHA256SUMS

deb:
	@test -n "$(TARGET)" || { echo 'TARGET is required' >&2; exit 2; }
	"$(CURDIR)/packaging/debian/build-deb.sh" \
	  --binary "$(DIST_ROOT)/$(TARGET)/gump" \
	  --target "$(TARGET)" \
	  --version "$(DEB_VERSION)" \
	  --output "$(PACKAGE_ROOT)/deb"

deb-linux-x86_64:
	$(MAKE) deb TARGET=$(LINUX_X86_64_TARGET)

deb-linux-arm64:
	$(MAKE) deb TARGET=$(LINUX_ARM64_TARGET)

rpm:
	@test -n "$(TARGET)" || { echo 'TARGET is required' >&2; exit 2; }
	"$(CURDIR)/packaging/rpm/build-rpm.sh" \
	  --binary "$(DIST_ROOT)/$(TARGET)/gump" \
	  --target "$(TARGET)" \
	  --version "$(RPM_VERSION)" \
	  --release "$(RPM_RELEASE)" \
	  --output "$(PACKAGE_ROOT)/rpm"

rpm-linux-x86_64:
	$(MAKE) rpm TARGET=$(LINUX_X86_64_TARGET)

rpm-linux-arm64:
	$(MAKE) rpm TARGET=$(LINUX_ARM64_TARGET)

clean-dist:
	rm -rf "$(CURDIR)/dist"
