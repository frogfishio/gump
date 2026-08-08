job "[[job_name]]" {
  datacenters = ["*"]
  type        = "service"

  meta = {
    build_tag = "[[build_tag]]"
    roll_id   = "[[roll_id]]"
  }

  group "api" {
    count = 1

    update {
      canary            = 1
      auto_promote      = true
      auto_revert       = true
      max_parallel      = 1
      health_check      = "checks"
      min_healthy_time  = "15s"
      healthy_deadline  = "10m"
      progress_deadline = "15m"
    }

    network {
      port "http" {
        to = 3000
      }
    }

    service {
      name     = "k2db-api"
      provider = "consul"
      port     = "http"
      tags     = ["build:[[build_tag]]", "roll:[[roll_id]]", "plane:runtime"]
      meta {
        domain = "[[domain]]"
      }

      check {
        name     = "health"
        type     = "http"
        path     = "/health"
        interval = "10s"
        timeout  = "2s"
      }

      check {
        name     = "ready"
        type     = "http"
        path     = "/ready"
        interval = "10s"
        timeout  = "2s"
      }
    }

    service {
      name     = "edge"
      provider = "consul"
      port     = "http"
      tags     = ["build:[[build_tag]]", "roll:[[roll_id]]", "plane:edge"]
      meta {
        domain = "[[domain]]"
      }

      check {
        name     = "health"
        type     = "http"
        path     = "/health"
        interval = "10s"
        timeout  = "2s"
      }

      check {
        name     = "ready"
        type     = "http"
        path     = "/ready"
        interval = "10s"
        timeout  = "2s"
      }
    }

    task "server" {
      driver = "docker"

      config {
        image = "[[image_repo]]:[[image_tag]]"
        ports = ["http"]
        args = ["serve", "--mongo-uri-env", "K2DB_MONGO_URI", "--system-db-name", "[[system_db_name]]"]
      }

      template {
        destination = "${NOMAD_SECRETS_DIR}/app.env"
        env         = true
        change_mode = "restart"

        data = <<EOT
[[system_db_name_template]]
[[mongo_uri_template]]
EOT
      }

      [[vault_block]]

      resources {
        cpu    = 500
        memory = 120
      }

      restart {
        attempts = 3
        delay    = "10s"
        mode     = "fail"
      }
    }
  }
}