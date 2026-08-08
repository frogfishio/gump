# inventory.tf — writes ../ansible/inventory/terraform.ini

# Compute paths in locals (functions allowed here)
locals {
  ansible_inventory_path = abspath("${path.module}/../ansible/inventory/terraform.ini")
  ansible_inventory_dir  = dirname(local.ansible_inventory_path)

  ssh_user        = "manager"
  ssh_key_path    = var.ssh_private_key_path == null ? "" : pathexpand(var.ssh_private_key_path)
  nodes = [for host in digitalocean_droplet.host : {
    host_name  = host.name
    public_ip  = host.ipv4_address
    private_ip = host.ipv4_address_private
  }]
}

# Ensure parent directory exists (idempotent)
resource "null_resource" "ensure_inventory_dir" {
  provisioner "local-exec" {
    command = "mkdir -p ${local.ansible_inventory_dir}"
  }
  triggers = { dir = local.ansible_inventory_dir }
}

# Write the INI inventory
resource "local_file" "ansible_inventory" {
  depends_on = [null_resource.ensure_inventory_dir]
  filename   = local.ansible_inventory_path
  content = templatefile("${path.module}/ansible_inventory.tpl", {
    nodes        = local.nodes
    ssh_user     = local.ssh_user
    ssh_key_path = local.ssh_key_path
  })
}