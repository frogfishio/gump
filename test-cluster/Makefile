.PHONY: help init plan infra configure-base install-gump configure verify form smoke destroy

ROOT_DIR := $(abspath .)
TERRAFORM_DIR := $(ROOT_DIR)/terraform
ANSIBLE_DIR := $(ROOT_DIR)/ansible
INVENTORY := $(ANSIBLE_DIR)/inventory/terraform.ini
MACRUN := $(ROOT_DIR)/scripts/macrun-exec.sh
GUMP_ARTIFACT ?=
GUMP_ARTIFACT_SHA256 ?=

help:
	@printf '%s\n' \
	  'Gump three-node test cluster' \
	  '' \
	  '  make init       Initialize Terraform providers.' \
	  '  make plan       Plan infrastructure through macrun infra scope.' \
	  '  make infra      Apply infrastructure and wait until all hosts are ready.' \
	  '  make configure-base  Harden hosts without requiring a Gump artifact.' \
	  '  make install-gump    Install GUMP_ARTIFACT after the server contract lands.' \
	  '  make configure       Run configure-base and install-gump together.' \
	  '  make verify     Verify the substrate and installed artifact.' \
	  '  make form       Form Gump cluster (enabled when CLI contract lands).' \
	  '  make smoke      Run current live acceptance ladder.' \
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
	@test -n "$(GUMP_ARTIFACT)" || { echo 'GUMP_ARTIFACT=/absolute/path/to/linux/gump is required' >&2; exit 2; }
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
	"$(MACRUN)" cluster "$(ROOT_DIR)/scripts/form.sh"

smoke:
	"$(ROOT_DIR)/scripts/smoke.sh"

destroy:
	@test "$(CONFIRM_DESTROY)" = YES || { echo 'Refusing: rerun with CONFIRM_DESTROY=YES' >&2; exit 2; }
	"$(MACRUN)" infra terraform -chdir="$(TERRAFORM_DIR)" destroy
