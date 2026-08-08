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
        to = 4200
      }
    }

    service {
      name     = "k2login"
      provider = "consul"
      port     = "http"
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
      name     = "edge"
      provider = "consul"
      port     = "http"
      tags     = ["build:[[build_tag]]", "roll:[[roll_id]]", "plane:edge", "domain:[[login_domain]]"]
      meta {
        domain = "[[login_domain]]"
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
        args = ["serve"]
      }

      template {
        destination = "${NOMAD_SECRETS_DIR}/app.env"
        env         = true
        change_mode = "restart"

        data = <<EOT
K2LOGIN_LISTEN_HOST=0.0.0.0
K2LOGIN_LISTEN_PORT=4200
{{- with service "k2rbac-api" }}
{{- with index . 0 }}
K2LOGIN_RBAC_BASE_URL=http://{{ .Address }}:{{ .Port }}
{{- end }}
{{- end }}
[[rbac_api_key_template]]
K2LOGIN_DEFAULT_CONTEXT=https://[[hello_domain]]/
K2LOGIN_PUBLIC_ORIGIN=https://[[login_domain]]
K2LOGIN_ALLOWED_NEXT_HOSTS=[[login_domain]],[[auth_domain]],[[hello_domain]],[[hello_alt_domain]]
K2LOGIN_SIGNUP_ELIGIBILITY=closed
K2LOGIN_SIGNUP_CREDENTIAL=password-required
K2LOGIN_MAIL_ENABLED=false
K2LOGIN_COOKIE_SECURE=true
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