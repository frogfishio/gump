.PHONY: help dist dist-macos-arm64 dist-linux-x86_64 clean-dist

MACOS_ARM64_TARGET := aarch64-apple-darwin
LINUX_X86_64_TARGET := x86_64-unknown-linux-gnu
DIST_ROOT := $(CURDIR)/dist/bin
ZIG_GLOBAL_CACHE_DIR := $(CURDIR)/target/zig-global-cache
ZIG_LOCAL_CACHE_DIR := $(CURDIR)/target/zig-local-cache

help:
	@printf '%s\n' \
	  'Gump build targets' \
	  '' \
	  '  make dist                Build all currently supported raw assets.' \
	  '  make dist-macos-arm64    Build dist/bin/aarch64-apple-darwin/gump.' \
	  '  make dist-linux-x86_64   Build dist/bin/x86_64-unknown-linux-gnu/gump.' \
	  '  make clean-dist          Remove generated distribution assets.'

dist: dist-macos-arm64 dist-linux-x86_64

dist-macos-arm64:
	rustup target add $(MACOS_ARM64_TARGET)
	cargo build --locked --release --target $(MACOS_ARM64_TARGET) -p gump-server --bin gump
	install -d "$(DIST_ROOT)/$(MACOS_ARM64_TARGET)"
	install -m 0755 "$(CURDIR)/target/$(MACOS_ARM64_TARGET)/release/gump" \
	  "$(DIST_ROOT)/$(MACOS_ARM64_TARGET)/gump"

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

clean-dist:
	rm -rf "$(CURDIR)/dist"
