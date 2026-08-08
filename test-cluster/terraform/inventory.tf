locals {
  ansible_inventory_path = abspath("${path.module}/../ansible/inventory/terraform.ini")
  ssh_key_path = (
    var.ssh_private_key_path == null ? "" : pathexpand(var.ssh_private_key_path)
  )
  nodes = [for index, host in digitalocean_droplet.gump : {
    ordinal    = index + 1
    host_name  = format("gump%02d", index + 1)
    public_ip  = host.ipv4_address
    private_ip = host.ipv4_address_private
  }]
}

resource "local_file" "ansible_inventory" {
  filename        = local.ansible_inventory_path
  file_permission = "0600"
  content = templatefile("${path.module}/ansible_inventory.tpl", {
    nodes        = local.nodes
    ssh_user     = "manager"
    ssh_key_path = local.ssh_key_path
  })
}
