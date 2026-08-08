# infra-do

Provision a three-node DigitalOcean cluster in Singapore and configure it with Ansible.

Defaults:
- Region: `sgp1`
- OS: Ubuntu 24.04 LTS
- Cluster size: `3`
- Size: `s-1vcpu-2gb`

Prerequisites:
- Terraform installed locally
- Ansible installed locally
- A DigitalOcean API token exported as `DIGITALOCEAN_TOKEN`
- An SSH key already uploaded to DigitalOcean
- The matching private key available locally

Optional local env workflow:
- Put your real `DIGITALOCEAN_TOKEN` in `.envrc`
- Optionally set `VAULT_INIT_FILE` in `.envrc` if you want to override the default local Vault init material path
- If you use `direnv`, run `direnv allow`
- If you do not use `direnv`, load it manually with `source .envrc`

Provisioning:
```sh
cd terraform
terraform init
source ../.envrc  # or export DIGITALOCEAN_TOKEN="your-do-token"
```

Create `terraform/terraform.tfvars` and set:
- `ssh_key_fingerprints` to your uploaded DigitalOcean SSH key fingerprint

Minimal example:
```hcl
cluster_size         = 3
region               = "sgp1"
instance_type        = "s-1vcpu-2gb"
ssh_key_fingerprints = ["your:digitalocean:ssh:key:fingerprint"]
```

If you want to force a non-default SSH key path, you can also set:
- `ssh_private_key_path = "/full/path/to/private/key"`

SSH access is key-based and host access is controlled on the machine with `ufw` rather than a DigitalOcean network firewall.
The droplet creates a `manager` user during first boot, copies the DigitalOcean-injected SSH key to that account, enables passwordless `sudo`, and applies a minimal `ufw` policy before Ansible connects.
Wait for cloud-init to finish before the first `ssh manager@...` or Ansible run; `root` usually becomes reachable first and `manager` follows once first-boot provisioning completes.

Security defaults after Ansible bootstrap:
- SSH keys only; password login disabled
- Administrative SSH access through `manager` with `sudo`
- Root SSH login disabled
- `fail2ban` enabled for SSH
- UFW default deny on inbound traffic, with only SSH, HTTP, and HTTPS allowed on the host
- Three Consul servers clustered over the private network
- Three Nomad servers and clients clustered over the private network
- Three Vault nodes configured for integrated Raft storage over the private network

Apply infrastructure:
```sh
cd terraform
terraform apply
```

Or from repo root:
```sh
make infra
```

This writes the generated Ansible inventory to `ansible/inventory/terraform.ini` for all three nodes.

Configure the host:
```sh
cd ansible
ansible-inventory -i inventory/terraform.ini --graph
ansible frogfish -m ping
ansible-playbook site.yml
```

Or run the full Stage 2 bootstrap plus smoke test from repo root:
```sh
make setup
```

Bolt the current app layer onto the raw cluster with a separate Stage 3 run:
```sh
cd services
cp group_vars/constellation.example.yml group_vars/constellation.yml
ansible-playbook -i ../ansible/inventory/terraform.ini site.yml \
	-e constellation_layer3_target=frogfish01 \
	-e @group_vars/constellation.yml
```

Or run the same Stage 3 flow from repo root:
```sh
make services
```

`make services` creates `services/group_vars/constellation.yml` from the example only if it does not already exist, then runs the Stage 3 playbook against `frogfish01`. Override the control target with `make services SERVICES_TARGET=frogfish02` if needed.

If you want the whole flow driven from the Makefile, the stage sequence is:
```sh
make infra
make setup
make services
```

There is also a convenience end-to-end target:
```sh
make up
```

`make up` runs Terraform, then Stage 2 plus the smoke test, then Stage 3.

Useful helpers:
```sh
./ansible/inventory.sh
./tunnel.sh
./smoke-test.sh
```

Nomad smoke test:
```sh
./smoke-test.sh run
./smoke-test.sh allocs
./smoke-test.sh status
./smoke-test.sh stop
```

Notes:
- Terraform provisions `frogfish01`, `frogfish02`, and `frogfish03`
- The helper scripts use the first node (`frogfish01`) as the bootstrap/UI target via the `public_ip` Terraform output
- The top-level `nomad/` directory is only the substrate smoke test; the real app deploy run lives under `services/`
- Vault listens on each node on port `8200`, but UFW only allows Vault cluster traffic from peer private IPs
- Ansible installs the generic Consul-backed Caddy config, the local ask-helper service for on-demand TLS checks, and the Consul prepared-query template needed for dynamic edge routing
- On the first Ansible run, Vault is initialized automatically from `frogfish01` and the init material is written to a local gitignored file at `.vault-init.json` by default, or to the path in `VAULT_INIT_FILE` if you set it in `.envrc`
- On later Ansible runs, that same local init material is used to unseal sealed Vault nodes automatically
- Use `./tunnel.sh` if you want the Nomad UI on `http://localhost:4646`
- By default the helper scripts use your normal SSH defaults and agent; pass a second argument or set `SSH_KEY_PATH` only if you want to force a specific private key file
- Find the dynamic port from `./smoke-test.sh allocs` and then inspect the allocation on the host if needed
- The smoke test proves the Nomad Docker driver can pull and run a container on the node

Vault operator material:
```sh
cat .vault-init.json
ssh manager@$(cd terraform && terraform output -raw public_ip) 'sudo env VAULT_ADDR=http://127.0.0.1:8200 vault status'
```

If you destroy and recreate the cluster, remove the stale local Vault init file or point `VAULT_INIT_FILE` at a fresh path before rerunning Ansible.

Destroy the droplet:
```sh
cd terraform
terraform destroy
```

Or use the repo-level reset target:
```sh
make clean
```

`make clean` runs `terraform destroy` from `terraform/` and then removes the local Vault init file and generated Ansible inventory. If you override `VAULT_INIT_FILE` in your environment, that value is used.

