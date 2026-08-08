variable "region" {
  description = "DigitalOcean region slug"
  type        = string
  default     = "sgp1"
}

variable "cluster_size" {
  description = "Number of nodes in the frogfish cluster"
  type        = number
  default     = 3
}

variable "instance_type" {
  description = "DigitalOcean droplet size slug"
  type        = string
  default     = "s-1vcpu-2gb"
}

variable "ssh_key_fingerprints" {
  description = "DigitalOcean SSH key fingerprints to inject into the droplet"
  type        = list(string)
}

variable "ssh_private_key_path" {
  description = "Optional local SSH private key path override used by Ansible and helper scripts"
  type        = string
  default     = null
  nullable    = true
}