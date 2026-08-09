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
  }
}

provider "digitalocean" {}

resource "digitalocean_droplet" "gump" {
  count = var.cluster_size

  name     = format("%s-%02d", var.cluster_name, count.index + 1)
  region   = var.region
  size     = var.instance_type
  image    = "ubuntu-24-04-x64"
  ssh_keys = var.ssh_key_fingerprints

  backups    = false
  ipv6       = false
  monitoring = true

  tags = [var.cluster_tag, format("%s-%02d", var.cluster_name, count.index + 1)]

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
ufw --force enable
CLOUD
}

resource "digitalocean_firewall" "gump" {
  name        = "${var.cluster_name}-firewall"
  droplet_ids = digitalocean_droplet.gump[*].id

  inbound_rule {
    protocol         = "tcp"
    port_range       = "22"
    source_addresses = var.admin_cidrs
  }

  # The cloud edge admits HTTP/S to the test-cluster addresses. The host
  # firewall opens the corresponding forwarded listener only on gump01, so the
  # selected ACME entry remains the sole effective public endpoint.
  inbound_rule {
    protocol         = "tcp"
    port_range       = "80"
    source_addresses = ["0.0.0.0/0"]
  }

  inbound_rule {
    protocol         = "tcp"
    port_range       = "443"
    source_addresses = ["0.0.0.0/0"]
  }

  inbound_rule {
    protocol    = "udp"
    port_range  = tostring(var.gump_cluster_port)
    source_tags = [var.cluster_tag]
  }

  inbound_rule {
    protocol    = "tcp"
    port_range  = var.workload_port_range
    source_tags = [var.cluster_tag]
  }

  inbound_rule {
    protocol    = "udp"
    port_range  = var.workload_port_range
    source_tags = [var.cluster_tag]
  }

  inbound_rule {
    protocol    = "icmp"
    source_tags = [var.cluster_tag]
  }

  outbound_rule {
    protocol              = "tcp"
    port_range            = "all"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }

  outbound_rule {
    protocol              = "udp"
    port_range            = "all"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }

  outbound_rule {
    protocol              = "icmp"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }
}

output "seed_public_ip" {
  value = digitalocean_droplet.gump[0].ipv4_address
}

output "public_ips" {
  value = digitalocean_droplet.gump[*].ipv4_address
}

output "private_ips" {
  value = digitalocean_droplet.gump[*].ipv4_address_private
}
