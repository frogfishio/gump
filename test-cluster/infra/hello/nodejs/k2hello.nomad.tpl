job "k2hello" {
  datacenters = ["*"]
  type        = "service"

  meta = {
    build_tag = "{{build_tag}}"
    roll_id   = "{{roll_id}}"
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
        to = 4300
      }
    }

    service {
      name     = "k2hello"
      provider = "consul"
      port     = "http"
      tags     = ["build:{{build_tag}}", "roll:{{roll_id}}", "plane:runtime"]

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
      tags     = ["build:{{build_tag}}", "roll:{{roll_id}}", "plane:edge", "domain:{{hello_domain}}"]
      meta {
        domain = "{{hello_domain}}"
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
        image = "{{image_repo}}:{{image_tag}}"
        ports = ["http"]
      }

      template {
        destination = "${NOMAD_SECRETS_DIR}/app.env"
        env         = true
        change_mode = "restart"

        data = <<EOT
{{- with nomadVar "nomad/jobs/k2hello" -}}
HELLO_SESSION_SECRET={{ .HELLO_SESSION_SECRET }}
{{- end -}}
HELLO_LISTEN_HOST=0.0.0.0
HELLO_LISTEN_PORT=4300
HELLO_PUBLIC_ORIGIN=https://{{hello_domain}}
HELLO_LOGIN_ORIGIN=https://{{login_domain}}
HELLO_LOGIN_HANDOFF_BASE_URL=https://{{login_domain}}
HELLO_COOKIE_SECURE=true
EOT
      }

      resources {
        cpu    = 300
        memory = 256
      }

      restart {
        attempts = 3
        delay    = "10s"
        mode     = "fail"
      }
    }
  }
}