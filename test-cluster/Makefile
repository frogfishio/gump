.PHONY: help clean destroy infra setup smoke services up terraform stage2 layer3

VAULT_INIT_FILE ?= .vault-init.json
TERRAFORM_DIR := terraform
ANSIBLE_DIR := ansible
ANSIBLE_INVENTORY := ansible/inventory/terraform.ini
SERVICES_DIR := services
SERVICES_TARGET ?= frogfish01
SERVICES_VARS := $(SERVICES_DIR)/group_vars/constellation.yml
SERVICES_VARS_EXAMPLE := $(SERVICES_DIR)/group_vars/constellation.example.yml
SERVICES_ONLY ?=

help:
	@printf '%s\n' \
	  'Targets:' \
	  '  make infra     Run terraform init and terraform apply.' \
	  '  make setup     Run the Stage 2 Ansible bootstrap and smoke test.' \
	  '  make smoke     Run the Nomad smoke test only.' \
	  '  make services  Run the stage-3 app deploy playbook.' \
	  '                  Set SERVICES_ONLY=k2db-api to deploy one service.' \
	  '  make up        Run infra, setup, then services.' \
	  '  make clean    Destroy the cluster and remove local bootstrap artifacts.' \
	  '  make destroy  Alias for make clean.'

infra:
	cd "$(TERRAFORM_DIR)" && terraform init
	cd "$(TERRAFORM_DIR)" && terraform apply

setup:
	cd "$(ANSIBLE_DIR)" && ansible-inventory -i inventory/terraform.ini --graph
	cd "$(ANSIBLE_DIR)" && ansible frogfish -m ping
	cd "$(ANSIBLE_DIR)" && ansible-playbook site.yml
	$(MAKE) smoke

smoke:
	./smoke-test.sh run

clean:
	cd terraform && terraform destroy
	rm -f "$(VAULT_INIT_FILE)" "$(ANSIBLE_INVENTORY)" "$(SERVICES_DIR)/.constellation-secrets.yml"

destroy: clean

services:
	@if [ ! -f "$(SERVICES_VARS)" ]; then cp "$(SERVICES_VARS_EXAMPLE)" "$(SERVICES_VARS)"; fi
	cd "$(SERVICES_DIR)" && ansible-playbook -i ../$(ANSIBLE_INVENTORY) site.yml \
	  -e constellation_layer3_target=$(SERVICES_TARGET) \
	  -e constellation_layer3_only_csv=$(SERVICES_ONLY) \
	  -e @group_vars/constellation.yml

up: infra setup services

terraform: infra

stage2: setup

layer3: services