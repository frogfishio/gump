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
      port "runtime" {
        static = 3001
      }

      port "admin" {
        static = 3002
      }

      port "ui" {
        static = 4181
      }
    }

    service {
      name     = "k2mx-api"
      provider = "consul"
      port     = "runtime"
      tags     = ["build:[[build_tag]]", "roll:[[roll_id]]", "plane:runtime"]

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
      name     = "k2mx-admin-api"
      provider = "consul"
      port     = "admin"
      tags     = ["build:[[build_tag]]", "roll:[[roll_id]]", "plane:admin"]

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
      name     = "k2mx-ui"
      provider = "consul"
      port     = "ui"
      tags     = ["build:[[build_tag]]", "roll:[[roll_id]]", "plane:ui", "access:private"]

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
        force_pull = true
        network_mode = "host"
        ports = ["runtime", "admin", "ui"]
        args = ["serve"]
      }

      template {
        destination = "${NOMAD_SECRETS_DIR}/app.env"
        env         = true
        change_mode = "restart"

        data = <<EOT
K2MX_LISTEN_HOST=0.0.0.0
K2MX_LISTEN_PORT=3001
K2MX_ADMIN_API_ENABLED=true
K2MX_ADMIN_API_HOST=0.0.0.0
K2MX_ADMIN_API_PORT=3002
{{ with service "k2db-api" }}
{{ with index . 0 }}
K2MX_K2DB_BASE_URL=http://{{ .Address }}:{{ .Port }}
{{ end }}
{{ end }}
[[k2db_api_key_template]]
[[bootstrap_token_template]]
K2MX_UI_MODE=ui-local
K2MX_UI_HOST=0.0.0.0
K2MX_UI_PORT=4181
[[ui_session_secret_template]]
K2MX_WORKER_ENABLED=true
EOT
      }

  [[vault_block]]

      resources {
        cpu    = 500
        memory = 250
      }

      restart {
        attempts = 3
        delay    = "10s"
        mode     = "fail"
      }
    }
  }
}