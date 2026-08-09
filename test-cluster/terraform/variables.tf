variable "cluster_name" {
  description = "Display-name prefix for the disposable Gump cluster"
  type        = string
  default     = "gump-test"
}

variable "cluster_tag" {
  description = "DigitalOcean tag used for private peer firewall rules"
  type        = string
  default     = "gump-test-cluster"
}

variable "region" {
  description = "DigitalOcean region slug"
  type        = string
  default     = "sgp1"
}

variable "cluster_size" {
  description = "This harness deliberately tests exactly three Gump nodes"
  type        = number
  default     = 3

  validation {
    condition     = var.cluster_size == 3
    error_message = "The live-like Gump harness requires exactly three nodes."
  }
}

variable "instance_type" {
  description = "DigitalOcean droplet size slug"
  type        = string
  default     = "s-1vcpu-2gb"
}

variable "ssh_key_fingerprints" {
  description = "DigitalOcean SSH key fingerprints injected into every node"
  type        = list(string)

  validation {
    condition     = length(var.ssh_key_fingerprints) > 0
    error_message = "At least one SSH key fingerprint is required."
  }
}

variable "ssh_private_key_path" {
  description = "Optional local SSH private key override for generated Ansible inventory"
  type        = string
  default     = null
  nullable    = true
}

variable "admin_cidrs" {
  description = "Public CIDRs allowed to reach SSH"
  type        = list(string)

  validation {
    condition     = length(var.admin_cidrs) > 0
    error_message = "At least one explicit administrative CIDR is required."
  }
}

variable "gump_cluster_port" {
  description = "Private UDP port used by Gump QUIC; provisional until the CLI default is frozen"
  type        = number
  default     = 7443

  validation {
    condition     = var.gump_cluster_port >= 1024 && var.gump_cluster_port <= 65535
    error_message = "gump_cluster_port must be between 1024 and 65535."
  }
}

variable "workload_port_range" {
  description = "Private-only unprivileged TCP/UDP range available to arbitrary workloads"
  type        = string
  default     = "1024-65535"
}
