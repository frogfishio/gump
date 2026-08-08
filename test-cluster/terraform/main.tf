terraform {
  required_version = ">= 1.5.0"
  required_providers {
    digitalocean = {
      source  = "digitalocean/digitalocean"
      version = "~> 2.0"
    }
    local = {
      source  = "hashicorp/local"
      version = "~> 2.5"
    }
    null = {
      source  = "hashicorp/null"
      version = "~> 3.2"
    }
  }
}

provider "digitalocean" {}

resource "digitalocean_droplet" "host" {
  count    = var.cluster_size
  name     = format("frogfish%02d", count.index + 1)
  region   = var.region
  size     = var.instance_type
  image    = "ubuntu-24-04-x64"
  ssh_keys = var.ssh_key_fingerprints

  backups    = false
  ipv6       = false
  monitoring = true

  # Minimal cloud-init: ensure Python is present for Ansible.
  user_data = <<-CLOUD
#!/usr/bin/env bash
set -eux
export DEBIAN_FRONTEND=noninteractive
apt-get update -y
apt-get install -y python3 python3-apt sudo ufw

if ! id -u manager >/dev/null 2>&1; then
  adduser --disabled-password --gecos "" manager
  usermod -aG sudo manager
fi

install -d -m 0700 -o manager -g manager /home/manager/.ssh
if [ -f /root/.ssh/authorized_keys ]; then
  install -m 0600 -o manager -g manager /root/.ssh/authorized_keys /home/manager/.ssh/authorized_keys
fi
cat >/etc/sudoers.d/90-manager <<'EOF'
manager ALL=(ALL) NOPASSWD:ALL
EOF
chmod 0440 /etc/sudoers.d/90-manager

ufw default deny incoming
ufw default allow outgoing
ufw allow OpenSSH
ufw allow 80/tcp
ufw allow 443/tcp
ufw --force enable
CLOUD

  tags = ["frogfish-cluster", format("frogfish%02d", count.index + 1)]
}

output "public_ip" {
  value = digitalocean_droplet.host[0].ipv4_address
}

output "public_ips" {
  value = [for host in digitalocean_droplet.host : host.ipv4_address]
}

output "private_ips" {
  value = [for host in digitalocean_droplet.host : host.ipv4_address_private]
}

output "ssh_example" {
  value = var.ssh_private_key_path == null ? "ssh manager@${digitalocean_droplet.host[0].ipv4_address}" : "ssh -i ${var.ssh_private_key_path} manager@${digitalocean_droplet.host[0].ipv4_address}"
}