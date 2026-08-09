.PHONY: help init plan infra configure-base install-gump configure verify form unseal smoke fixture-finite deploy-finite accept-finite live-finite fixture-ringtail deploy-ringtail fixture-ringtail-relay-probe deploy-ringtail-relay-probe accept-ringtail live-ringtail destroy

ROOT_DIR := $(abspath .)
TERRAFORM_DIR := $(ROOT_DIR)/terraform
ANSIBLE_DIR := $(ROOT_DIR)/ansible
INVENTORY := $(ANSIBLE_DIR)/inventory/terraform.ini
MACRUN := $(ROOT_DIR)/scripts/macrun-exec.sh
GUMP_ARTIFACT ?= $(abspath ../dist/bin/x86_64-unknown-linux-gnu/gump)
GUMP_LOCAL_BIN ?= $(abspath ../dist/bin/aarch64-apple-darwin/gump)
GUMP_ARTIFACT_SHA256 ?=

help:
	@printf '%s\n' \
	  'Gump three-node test cluster' \
	  '' \
	  '  make init       Initialize Terraform providers.' \
	  '  make plan       Plan infrastructure through macrun infra scope.' \
	  '  make infra      Apply infrastructure and wait until all hosts are ready.' \
	  '  make configure-base  Harden hosts without requiring a Gump artifact.' \
	  '  make install-gump    Install the Linux x86-64 raw asset (override GUMP_ARTIFACT).' \
	  '  make configure       Run configure-base and install-gump together.' \
	  '  make verify     Verify the substrate and installed artifact.' \
	  '  make form       Form Gump cluster (enabled when CLI contract lands).' \
	  '  make unseal     Retry memory-only cluster unseal without reforming.' \
	  '  make smoke      Run current live acceptance ladder.' \
	  '  make fixture-finite  Build a signed Capsule for the live finite fixture.' \
	  '  make deploy-finite   Build and deploy the finite fixture through the live cluster.' \
	  '  make accept-finite   Prove completion converged once across all three nodes.' \
	  '  make live-finite     Deploy then run the finite-workload acceptance check.' \
	  '  make fixture-ringtail  Build a signed all-nodes Ringtail Capsule.' \
	  '  make deploy-ringtail   Build and deploy Ringtail to every Gump node.' \
	  '  make deploy-ringtail-relay-probe  Emit one telemetry record on every node.' \
	  '  make accept-ringtail   Exercise all three live Ringtail collectors.' \
	  '  make live-ringtail     Deploy Ringtail + all-node probe, then accept end to end.' \
	  '  make destroy    Destroy compute; requires CONFIRM_DESTROY=YES.'

init:
	terraform -chdir="$(TERRAFORM_DIR)" init

plan:
	"$(MACRUN)" infra terraform -chdir="$(TERRAFORM_DIR)" plan

infra:
	"$(MACRUN)" infra terraform -chdir="$(TERRAFORM_DIR)" apply
	cd "$(ANSIBLE_DIR)" && ansible gump -m wait_for_connection -a 'timeout=600'
	cd "$(ANSIBLE_DIR)" && ansible gump -m ping

configure-base:
	cd "$(ANSIBLE_DIR)" && ansible-playbook site.yml --tags base

install-gump:
	@test -x "$(GUMP_ARTIFACT)" || { echo 'Missing Linux asset: run make dist at the repository root or override GUMP_ARTIFACT' >&2; exit 2; }
	cd "$(ANSIBLE_DIR)" && ansible-inventory --graph
	cd "$(ANSIBLE_DIR)" && ansible gump -m ping
	cd "$(ANSIBLE_DIR)" && ansible-playbook site.yml \
	  --tags gump \
	  -e "gump_artifact_path=$(abspath $(GUMP_ARTIFACT))" \
	  $(if $(GUMP_ARTIFACT_SHA256),-e "gump_artifact_sha256=$(GUMP_ARTIFACT_SHA256)",)

configure: configure-base install-gump

verify:
	"$(ROOT_DIR)/scripts/verify-substrate.sh"

form:
	@test -x "$(GUMP_LOCAL_BIN)" || { echo 'Missing local asset: run make dist at the repository root or override GUMP_LOCAL_BIN' >&2; exit 2; }
	"$(MACRUN)" cluster env GUMP_BIN="$(GUMP_LOCAL_BIN)" "$(ROOT_DIR)/scripts/form.sh"

unseal:
	"$(MACRUN)" cluster "$(ROOT_DIR)/scripts/unseal.sh"

smoke:
	"$(ROOT_DIR)/scripts/smoke.sh"

fixture-finite:
	"$(MACRUN)" fixtures env GUMP_BIN="$(GUMP_LOCAL_BIN)" "$(ROOT_DIR)/scripts/build-finite-fixture.sh"

deploy-finite: fixture-finite
	"$(ROOT_DIR)/scripts/deploy-finite-fixture.sh"

accept-finite:
	"$(ROOT_DIR)/scripts/accept-finite.sh"

live-finite: deploy-finite accept-finite

fixture-ringtail:
	"$(MACRUN)" fixtures env GUMP_BIN="$(GUMP_LOCAL_BIN)" "$(ROOT_DIR)/scripts/build-ringtail-fixture.sh"

deploy-ringtail: fixture-ringtail
	"$(ROOT_DIR)/scripts/deploy-ringtail-fixture.sh"

accept-ringtail:
	"$(ROOT_DIR)/scripts/accept-ringtail.sh"

fixture-ringtail-relay-probe:
	"$(MACRUN)" fixtures env GUMP_BIN="$(GUMP_LOCAL_BIN)" "$(ROOT_DIR)/scripts/build-ringtail-relay-probe.sh"

deploy-ringtail-relay-probe: fixture-ringtail-relay-probe
	"$(ROOT_DIR)/scripts/deploy-ringtail-relay-probe.sh"

live-ringtail:
	$(MAKE) deploy-ringtail
	$(MAKE) deploy-ringtail-relay-probe
	$(MAKE) accept-ringtail

destroy:
	@test "$(CONFIRM_DESTROY)" = YES || { echo 'Refusing: rerun with CONFIRM_DESTROY=YES' >&2; exit 2; }
	"$(MACRUN)" infra terraform -chdir="$(TERRAFORM_DIR)" destroy
